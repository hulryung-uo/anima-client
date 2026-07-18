//! Parser for UO's gump *layout* command grammar — the `{ button 10 30 4005
//! 4007 0 2 0 }{ text 20 20 0 0 }…` mini-language servers send in 0xB0
//! DisplayGump / 0xDD DisplayGumpPacked (see [`crate::world::Gump`]). This is
//! byte/grammar work over already-decoded protocol data, not rendering, so it
//! lives here rather than being duplicated by every renderer: [`parse`] turns
//! the raw string into typed [`GumpElement`]s a brain can consume directly
//! (see [`crate::agent::GumpView`]) instead of re-implementing this grammar.
//!
//! The core has no Cliloc table (that lives in the sibling `anima-assets`
//! crate), so a cliloc-driven element ([`GumpElement::Html`] with
//! [`HtmlText::Cliloc`]) carries the raw id/args unresolved — a driver with
//! Cliloc access (`anima-net`) resolves it for display.
//!
//! Ported from `anima-net`'s original `scene::parse_gump_layout`, which now
//! only does JSON shaping + cliloc resolution over this module's output (see
//! `anima_net::scene::gump_elements_to_json`).

/// One parsed element of a gump layout (see [`parse`]). Positions/sizes are
/// gump-local pixels, straight off the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GumpElement {
    /// A resizable background panel (`resizepic`).
    Background {
        x: i64,
        y: i64,
        w: i64,
        h: i64,
        page: i64,
    },
    /// Decorative art (`gumppic`); `graphic` is the gump art id.
    Image {
        x: i64,
        y: i64,
        graphic: i64,
        page: i64,
    },
    /// A clickable button (`button`). `graphic` is the up-state gump art id;
    /// `pageflag` 0 = local page-jump (switch to `param`, nothing sent to the
    /// server — ClassicUO `ButtonAction.SwitchPage`); 1 = reply button
    /// (clicking sends `GumpResponse` with `reply_id` — `ButtonAction.Activate`).
    Button {
        x: i64,
        y: i64,
        graphic: i64,
        reply_id: i64,
        pageflag: i64,
        param: i64,
        page: i64,
    },
    /// A line of plain text (`text`/`croppedtext`), already resolved from the
    /// gump's own local text table (an index into the packet's string list —
    /// distinct from a Cliloc table). `w` is the wrap width for `croppedtext`,
    /// `None` for an unbounded `text`.
    Text {
        x: i64,
        y: i64,
        w: Option<i64>,
        s: String,
        page: i64,
    },
    /// An HTML block: `htmlgump` (already resolved from the gump's own local
    /// text table) or `xmfhtmlgump`/`xmfhtmlgumpcolor`/`xmfhtmltok`
    /// (cliloc-driven — see [`HtmlText::Cliloc`]). Either way the string still
    /// carries its raw UO gump-HTML tags/entities (`<CENTER>`, `<BASEFONT
    /// COLOR=#rrggbb>`, `&amp;`, …) unresolved — same as [`GumpElement::Text`]
    /// — because interpreting them is a *display* concern (alignment, bold,
    /// color) that belongs to the renderer, not this protocol-data parser. See
    /// `web/main.js`'s `renderGumpHtml`.
    Html {
        x: i64,
        y: i64,
        w: i64,
        h: i64,
        text: HtmlText,
        page: i64,
    },
    /// A checkbox (`checkbox`); `on` is the initial checked state (0/1).
    Check {
        x: i64,
        y: i64,
        id: i64,
        on: i64,
        page: i64,
    },
    /// A radio button (`radio`); `on` is the initial selected state (0/1).
    Radio {
        x: i64,
        y: i64,
        id: i64,
        on: i64,
        page: i64,
    },
    /// A text entry field (`textentry`), pre-filled with `s` from the gump's
    /// local text table.
    Entry {
        x: i64,
        y: i64,
        w: i64,
        id: i64,
        s: String,
        page: i64,
    },
}

/// The text of a [`GumpElement::Html`] block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HtmlText {
    /// Already-resolved text (an `htmlgump`'s local string), tags/entities
    /// still raw — see [`GumpElement::Html`]'s doc.
    Literal(String),
    /// A cliloc reference the core cannot resolve on its own (no Cliloc table
    /// here — see `anima_assets::Cliloc`). `id` is the cliloc id; `args`, when
    /// present, are the tab-separated substitution args for `~N~` placeholders
    /// (`xmfhtmltok`'s `@arg@arg@…`, converted to tabs) — resolve via
    /// `Cliloc::format(id, args)`. `None` (`xmfhtmlgump`/`xmfhtmlgumpcolor`,
    /// which carry no args) means resolve via the plain `Cliloc::get(id)`
    /// instead — `format` on an empty-args template would silently eat any
    /// literal `~N~` text those variants are allowed to contain verbatim.
    Cliloc { id: u32, args: Option<String> },
}

/// A parsed gump layout: its elements plus the computed window size (see
/// [`parse`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GumpLayout {
    pub elements: Vec<GumpElement>,
    pub width: i64,
    pub height: i64,
}

/// Parse a UO gump `layout` command string (as carried by
/// [`crate::world::Gump::layout`]) into typed elements plus the computed window
/// `(width, height)`. Tokenizes into `{ cmd args… }` groups; supports the
/// common subset (resizepic/gumppic backgrounds, button, page, text/
/// croppedtext, htmlgump/xmfhtml*, checkbox, radio, textentry) — unknown
/// commands are ignored. `text` is the gump's own local string table
/// ([`crate::world::Gump::text`]), referenced by index from
/// `text`/`croppedtext`/`htmlgump` commands.
pub fn parse(layout: &str, text: &[String]) -> GumpLayout {
    let mut elements: Vec<GumpElement> = Vec::new();
    let mut page = 0i64;
    let mut win_w = 0i64; // from resizepic (x + w)
    let mut win_h = 0i64;
    let mut max_x = 0i64; // fallback extent from element positions
    let mut max_y = 0i64;

    let mut rest = layout;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else { break };
        let group = &after[..end];
        rest = &after[end + 1..];

        let toks: Vec<&str> = group.split_whitespace().collect();
        let Some(cmd) = toks.first() else { continue };
        let cmd = cmd.to_ascii_lowercase();
        let num = |i: usize| -> i64 { toks.get(i).and_then(|t| t.parse().ok()).unwrap_or(0) };
        let get_text = |i: usize| -> String {
            toks.get(i)
                .and_then(|t| t.parse::<usize>().ok())
                .and_then(|n| text.get(n))
                .cloned()
                .unwrap_or_default()
        };

        match cmd.as_str() {
            "page" => page = num(1),
            // resizepic x y gumpId w h — the sizing background panel.
            "resizepic" => {
                let (x, y, w, h) = (num(1), num(2), num(4), num(5));
                win_w = win_w.max(x + w);
                win_h = win_h.max(y + h);
                elements.push(GumpElement::Background { x, y, w, h, page });
            }
            // gumppic x y gumpId [hue=…] — decorative art.
            "gumppic" => {
                elements.push(GumpElement::Image {
                    x: num(1),
                    y: num(2),
                    graphic: num(3),
                    page,
                });
            }
            // button x y up down pageflag param reply-id
            "button" => {
                let (x, y, up, flag, param, id) = (num(1), num(2), num(3), num(5), num(6), num(7));
                elements.push(GumpElement::Button {
                    x,
                    y,
                    graphic: up,
                    reply_id: id,
                    pageflag: flag,
                    param,
                    page,
                });
                max_x = max_x.max(x + 32);
                max_y = max_y.max(y + 24);
            }
            // text x y hue textId
            "text" => {
                let (x, y) = (num(1), num(2));
                elements.push(GumpElement::Text {
                    x,
                    y,
                    w: None,
                    s: get_text(4),
                    page,
                });
                max_x = max_x.max(x + 120);
                max_y = max_y.max(y + 20);
            }
            // croppedtext x y w h hue textId
            "croppedtext" => {
                let (x, y, w) = (num(1), num(2), num(3));
                elements.push(GumpElement::Text {
                    x,
                    y,
                    w: Some(w),
                    s: get_text(6),
                    page,
                });
                max_x = max_x.max(x + w);
                max_y = max_y.max(y + num(4).max(20));
            }
            // htmlgump x y w h textId background scrollbar — the raw local
            // string is kept as-is (tags/entities un-stripped, un-decoded);
            // see `GumpElement::Html`'s doc for why.
            "htmlgump" => {
                let (x, y, w, h) = (num(1), num(2), num(3), num(4));
                elements.push(GumpElement::Html {
                    x,
                    y,
                    w,
                    h,
                    text: HtmlText::Literal(get_text(5)),
                    page,
                });
                max_x = max_x.max(x + w);
                max_y = max_y.max(y + h.max(20));
            }
            // xmfhtmlgump/xmfhtmlgumpcolor put the cliloc id at index 5; xmfhtmltok
            // puts it at index 8 (after background/scrollbar/color) followed by
            // `@arg@…`. Left unresolved here — see `HtmlText::Cliloc`.
            s if s.starts_with("xmfhtml") => {
                let (x, y, w, h) = (num(1), num(2), num(3), num(4));
                let cid = (if s == "xmfhtmltok" { num(8) } else { num(5) }) as u32;
                let args = if s == "xmfhtmltok" {
                    let raw = toks.get(9..).map(|a| a.join(" ")).unwrap_or_default();
                    Some(raw.trim_matches('@').replace('@', "\t")) // cliloc.format wants tabs
                } else {
                    None
                };
                elements.push(GumpElement::Html {
                    x,
                    y,
                    w,
                    h,
                    text: HtmlText::Cliloc { id: cid, args },
                    page,
                });
                max_x = max_x.max(x + w);
                max_y = max_y.max(y + h.max(20));
            }
            // checkbox x y up down state id
            "checkbox" => {
                let (x, y, state, id) = (num(1), num(2), num(5), num(6));
                elements.push(GumpElement::Check {
                    x,
                    y,
                    id,
                    on: state,
                    page,
                });
                max_x = max_x.max(x + 24);
                max_y = max_y.max(y + 24);
            }
            // radio x y up down state id
            "radio" => {
                let (x, y, state, id) = (num(1), num(2), num(5), num(6));
                elements.push(GumpElement::Radio {
                    x,
                    y,
                    id,
                    on: state,
                    page,
                });
                max_x = max_x.max(x + 24);
                max_y = max_y.max(y + 24);
            }
            // textentry x y w h hue id textId
            "textentry" => {
                let (x, y, w, h, id) = (num(1), num(2), num(3), num(4), num(6));
                let s = get_text(7);
                elements.push(GumpElement::Entry {
                    x,
                    y,
                    w,
                    id,
                    s,
                    page,
                });
                max_x = max_x.max(x + w);
                max_y = max_y.max(y + h.max(20));
            }
            _ => {}
        }
    }

    // Window size: prefer the resizepic extent; otherwise the element bounds.
    // Clamp to a sane minimum so a degenerate gump is still draggable/closable.
    let width = win_w.max(max_x + 16).max(80);
    let height = win_h.max(max_y + 16).max(48);
    GumpLayout {
        elements,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_commands() {
        let layout = "{ resizepic 0 0 5054 200 120 }{ button 20 90 247 248 1 0 7 }\
                      { text 20 20 0 0 }{ checkbox 20 50 210 211 1 3 }\
                      { textentry 20 65 120 18 0 4 1 }";
        let text = vec!["Accept the quest?".to_string(), "Name".to_string()];
        let layout = parse(layout, &text);
        // Width comes straight from the resizepic; height grows to fit elements
        // that extend below it (the button at y=90 + padding).
        assert_eq!(layout.width, 200);
        assert!(layout.height >= 120, "h={}", layout.height);

        assert_eq!(layout.elements.len(), 5);
        assert!(matches!(layout.elements[0], GumpElement::Background { .. }));
        match &layout.elements[1] {
            // pageflag 1 (reply) — this is what makes the button send a
            // GumpResponse instead of jumping pages locally.
            GumpElement::Button {
                reply_id, pageflag, ..
            } => {
                assert_eq!(*reply_id, 7);
                assert_eq!(*pageflag, 1);
            }
            other => panic!("expected Button, got {other:?}"),
        }
        match &layout.elements[2] {
            GumpElement::Text { s, .. } => assert_eq!(s, "Accept the quest?"),
            other => panic!("expected Text, got {other:?}"),
        }
        match &layout.elements[3] {
            GumpElement::Check { id, on, .. } => assert_eq!((*id, *on), (3, 1)),
            other => panic!("expected Check, got {other:?}"),
        }
        match &layout.elements[4] {
            GumpElement::Entry { id, s, .. } => {
                assert_eq!(*id, 4);
                assert_eq!(s, "Name");
            }
            other => panic!("expected Entry, got {other:?}"),
        }
    }

    #[test]
    fn tracks_pages_and_button_pageflag() {
        // Elements before the first "page" token are page 0 (always visible,
        // e.g. the background + a "next"/"prev" nav button that must show no
        // matter which page is active). "page 1" then "page 2" bracket the two
        // navigable sections; the pageflag-0 button on page 1 jumps to page 2
        // locally (no server round-trip), while the pageflag-1 button on page 2
        // is a real reply button.
        let layout = "{ resizepic 0 0 5054 200 200 }\
                      { page 1 }{ text 10 10 0 0 }\
                      { button 10 30 4005 4007 0 2 0 }\
                      { page 2 }{ text 10 10 0 1 }\
                      { button 10 30 247 248 1 0 99 }";
        let text = vec!["Page one".to_string(), "Page two".to_string()];
        let layout = parse(layout, &text);

        // bg(page0), text(page1), button(page1, pageflag0→page2), text(page2), button(page2, pageflag1, id99)
        let pages: Vec<i64> = layout
            .elements
            .iter()
            .map(|e| match e {
                GumpElement::Background { page, .. }
                | GumpElement::Text { page, .. }
                | GumpElement::Button { page, .. } => *page,
                other => panic!("unexpected element {other:?}"),
            })
            .collect();
        assert_eq!(pages, [0, 1, 1, 2, 2]);

        match &layout.elements[2] {
            GumpElement::Button {
                pageflag, param, ..
            } => {
                assert_eq!(*pageflag, 0);
                assert_eq!(*param, 2); // switches to page 2, contacts no server
            }
            other => panic!("expected Button, got {other:?}"),
        }
        match &layout.elements[4] {
            GumpElement::Button {
                pageflag, reply_id, ..
            } => {
                assert_eq!(*pageflag, 1);
                assert_eq!(*reply_id, 99); // reply id sent to the server on click
            }
            other => panic!("expected Button, got {other:?}"),
        }
    }

    #[test]
    fn preserves_html_tags_and_carries_cliloc_refs() {
        // Tags/entities are NOT interpreted here — that's the renderer's job
        // (see `GumpElement::Html`'s doc) — so the local string round-trips
        // byte-for-byte, same as a plain `text`/`croppedtext` element would.
        let layout = "{ htmlgump 5 5 180 40 0 0 0 }{ xmfhtmlgump 5 50 180 20 1015313 0 0 }";
        let text = vec!["<basefont color=#fff>Hello <b>world</b>".to_string()];
        let layout = parse(layout, &text);
        match &layout.elements[0] {
            GumpElement::Html {
                text: HtmlText::Literal(s),
                ..
            } => {
                assert_eq!(s, "<basefont color=#fff>Hello <b>world</b>")
            }
            other => panic!("expected literal Html, got {other:?}"),
        }
        match &layout.elements[1] {
            GumpElement::Html {
                text: HtmlText::Cliloc { id, args },
                ..
            } => {
                assert_eq!(*id, 1015313); // cliloc reference, unresolved (no table here)
                assert_eq!(*args, None);
            }
            other => panic!("expected cliloc Html, got {other:?}"),
        }
    }
}
