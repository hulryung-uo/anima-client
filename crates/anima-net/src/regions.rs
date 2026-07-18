//! Guard-zone rectangles: town "guard zone" boundaries, parsed from a local
//! copy of the ServUO/ClassicUO-style region file (`Data/Regions.xml`).
//!
//! UO's guard-zone boundary is server-side data with no packet equivalent —
//! the client never learns a region's rectangle from the wire. `Regions.xml`
//! (ServUO `Server/Region.cs::Load`) is the ground truth: a `<ServerRegions>`
//! root holds one `<Facet name="Felucca|Trammel|Ilshenar|Malas|Tokuno|TerMur">`
//! per map, each containing a tree of `<region type="..">` elements that can
//! nest arbitrarily deep and hold zero or more self-closing `<rect x= y=
//! width= height= [zmin=] />` children. A region is a *guarded* (guard-zone)
//! region if its `type` is one of [`GUARDED_REGION_TYPES`] — i.e. it
//! transitively extends ServUO's `GuardedRegion` (`Scripts/Server`'s class
//! hierarchy) — **and** it doesn't carry a direct `<guards disabled="true"
//! />` child, which is ServUO's own opt-out (`GuardedRegion.Disabled` makes
//! `CheckGuardCandidate`/`CallGuards` a no-op; e.g. Buccaneer's Den — the
//! lawless "no guards" pirate town — and several Ilshenar towns use exactly
//! this to be a `TownRegion` with no actual guards). Every other region type
//! (dungeon, field, moongate trigger, …) is never guarded, even though it may
//! sit right next to (or inside) a guarded one.
//!
//! `anima-core` stays zero-dep, and this is server-local data anyway, so this
//! is a small hand-rolled scanner rather than pulling in an XML crate: the
//! file is simple and well-formed, so a stack of open `Facet`/`region`
//! frames tracked across open/close tags is enough — see [`parse`]'s doc
//! comment for why each frame's rects are *buffered* rather than emitted
//! immediately. As a bonus (and to keep the parser testable without the real
//! multi-thousand line file) a `region` element may also carry its own
//! `map=".."` override, which takes precedence over its enclosing
//! `Facet`/region — real `Regions.xml` never uses this (facet is always set
//! once, on the `Facet` wrapper), but nothing stops a region from doing so
//! and it costs us nothing to honor it.

use std::path::PathBuf;

/// Region `type=".."` values that ServUO's `Regions.xml` treats as guarded —
/// i.e. the class transitively extends `GuardedRegion` in ServUO's
/// `Scripts/Server` hierarchy (`TownRegion : GuardedRegion`, `NewMaginciaRegion
/// : TownRegion`, `BlackthornCastle`/`CusteauPerronHouseRegion`
/// /`TokunoDocksRegion` : `GuardedRegion`). This list is specific to the
/// ServUO checkout it was derived from (`grep -R "class .* : \(GuardedRegion
/// \|TownRegion\)" Scripts/Server`) — a future ServUO version that adds
/// another `GuardedRegion` subclass needs a new entry here; this is a known,
/// intentional extension point rather than a closed set.
const GUARDED_REGION_TYPES: &[&str] = &[
    "TownRegion",
    "GuardedRegion",
    "BlackthornCastle",
    "CusteauPerronHouseRegion",
    "NewMaginciaRegion",
    "TokunoDocksRegion",
];

/// Whether a region's own `type=".."` attribute (if any) names a guarded
/// type. A region with no `type` at all is never guarded.
fn is_guarded_type(type_attr: Option<&str>) -> bool {
    match type_attr {
        Some(t) => GUARDED_REGION_TYPES.contains(&t),
        None => false,
    }
}

/// One guarded (guard-zone) rectangle in world coordinates, tagged with the
/// facet (map) it applies to. `facet` matches `World::map_index`/the scene's
/// `"facet"` field: 0=Felucca, 1=Trammel, 2=Ilshenar, 3=Malas, 4=Tokuno,
/// 5=TerMur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuardRect {
    pub facet: u8,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Map name (a `Facet name=".."` or a region's own `map=".."`) → facet index.
/// Unrecognized/absent names default to Felucca (0), matching ServUO (a
/// region with no explicit map inherits its parent's, and the outermost
/// default is Felucca).
fn facet_from_name(name: &str) -> u8 {
    match name {
        "Felucca" => 0,
        "Trammel" => 1,
        "Ilshenar" => 2,
        "Malas" => 3,
        "Tokuno" => 4,
        "TerMur" => 5,
        _ => 0,
    }
}

/// Resolve the region file to load: `$ANIMA_REGIONS` if set, else
/// `$HOME/dev/uo/servuo/Data/Regions.xml`. Never fails — the caller decides
/// what an unreadable/missing path means (graceful: no overlay).
pub fn resolve_path() -> PathBuf {
    if let Ok(p) = std::env::var("ANIMA_REGIONS") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join("dev/uo/servuo/Data/Regions.xml")
}

/// One currently-open `Facet`/`region` element's context. `rects` buffers
/// this element's *own, directly-nested* `<rect>` children — see [`parse`]'s
/// doc comment for why a rect can't be emitted at the time it's seen.
struct Frame {
    facet: u8,
    /// Whether this region's own `type=".."` names a guarded type. (A
    /// `Facet` frame is never itself a guarded region, so this is always
    /// `false` for those.)
    guarded: bool,
    /// Whether a `<guards disabled="true" />` was seen directly inside this
    /// element (not inside a nested child region — each region tracks its
    /// own).
    disabled: bool,
    rects: Vec<GuardRect>,
}

/// Parse every guarded rectangle out of a ServUO-style `Regions.xml`
/// document — every region whose `type` is in [`GUARDED_REGION_TYPES`],
/// *except* one that carries a direct `<guards disabled="true" />` child
/// (ServUO's own "no guards here" opt-out; see the module doc comment).
/// Rects belonging to any other region type (dungeons, fields, moongate
/// triggers, …) are skipped. Malformed or unrecognized input degrades
/// gracefully to fewer/no rects rather than panicking — this reads a file we
/// don't control the contents of.
///
/// Rects are *buffered per-region* rather than emitted the moment a `<rect>`
/// is seen, because `<guards disabled="true" />` can appear anywhere inside
/// a region's body — including *after* its `<rect>` children (ServUO's real
/// `Regions.xml` does exactly this, e.g. Buccaneer's Den) — so whether a
/// given region's rects are kept can only be decided once its closing
/// `</region>` is reached. The disabled flag (like the guarded-type check)
/// is scoped to the one region element it's a direct child of: a disabled
/// parent doesn't disable a guarded child nested inside it, and a disabled
/// child doesn't undisable a guarded parent — each `Frame` tracks its own.
pub fn parse(xml: &str) -> Vec<GuardRect> {
    let mut out = Vec::new();
    // Stack of every currently-open `Facet`/`region` element. A synthetic
    // root frame absorbs anything outside the outermost element (never
    // guarded, so its "rects" — if any stray in — are simply dropped).
    let mut stack: Vec<Frame> = vec![Frame {
        facet: 0,
        guarded: false,
        disabled: false,
        rects: Vec::new(),
    }];

    let mut i = 0usize;
    while let Some(rel) = xml[i..].find('<') {
        i += rel;
        if xml[i..].starts_with("<!--") {
            match xml[i..].find("-->") {
                Some(end) => i += end + 3,
                None => break,
            }
            continue;
        }
        let Some(close_rel) = xml[i..].find('>') else {
            break;
        };
        let tag = &xml[i + 1..i + close_rel];
        i += close_rel + 1;

        if let Some(name) = tag.strip_prefix('/') {
            if matches!(name.trim(), "Facet" | "region") && stack.len() > 1 {
                let frame = stack.pop().expect("len > 1 checked above");
                // Commit this region's own rects only if it ended up both
                // guarded and not disabled; otherwise they're dropped here
                // and never bubble up to the parent frame.
                if frame.guarded && !frame.disabled {
                    out.extend(frame.rects);
                }
            }
            continue;
        }

        let self_closing = tag.trim_end().ends_with('/');
        let body = if self_closing {
            tag.trim_end().trim_end_matches('/')
        } else {
            tag
        };
        let tag_name = body.split_whitespace().next().unwrap_or("");

        match tag_name {
            "Facet" if !self_closing => {
                let facet = read_str_attr(body, "name")
                    .map(|n| facet_from_name(&n))
                    .unwrap_or(0);
                stack.push(Frame {
                    facet,
                    guarded: false,
                    disabled: false,
                    rects: Vec::new(),
                });
            }
            "region" if !self_closing => {
                let guarded = is_guarded_type(read_str_attr(body, "type").as_deref());
                let parent_facet = stack.last().map(|f| f.facet).unwrap_or(0);
                let facet = read_str_attr(body, "map")
                    .map(|m| facet_from_name(&m))
                    .unwrap_or(parent_facet);
                stack.push(Frame {
                    facet,
                    guarded,
                    disabled: false,
                    rects: Vec::new(),
                });
            }
            "guards" => {
                if read_str_attr(body, "disabled").as_deref() == Some("true") {
                    if let Some(top) = stack.last_mut() {
                        top.disabled = true;
                    }
                }
            }
            "rect" => {
                let (x, y, w, h) = (
                    read_int_attr(body, "x"),
                    read_int_attr(body, "y"),
                    read_int_attr(body, "width"),
                    read_int_attr(body, "height"),
                );
                if let (Some(x), Some(y), Some(w), Some(h)) = (x, y, w, h) {
                    if let Some(top) = stack.last_mut() {
                        let facet = top.facet;
                        top.rects.push(GuardRect { facet, x, y, w, h });
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Read a `name="value"` attribute out of a tag's inner text (everything
/// between `<` and `>`/`/>`, e.g. `region type="TownRegion" name="Britain"`).
/// Requires a word boundary before `name` so e.g. reading `"map"` doesn't
/// false-match inside a longer attribute that happens to end in "map".
fn read_str_attr(tag_body: &str, attr: &str) -> Option<String> {
    let bytes = tag_body.as_bytes();
    let mut from = 0usize;
    while let Some(rel) = tag_body[from..].find(attr) {
        let start = from + rel;
        let boundary_ok = start == 0 || bytes[start - 1].is_ascii_whitespace();
        let after = start + attr.len();
        if boundary_ok && tag_body[after..].starts_with("=\"") {
            let vstart = after + 2;
            if let Some(end_rel) = tag_body[vstart..].find('"') {
                return Some(tag_body[vstart..vstart + end_rel].to_string());
            }
        }
        from = start + attr.len();
        if from >= tag_body.len() {
            break;
        }
    }
    None
}

fn read_int_attr(tag_body: &str, attr: &str) -> Option<i32> {
    read_str_attr(tag_body, attr)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic snippet covering: a `TownRegion` with an explicit
    /// `map="Trammel"` override, a nested `GuardedRegion` child that has no
    /// `map` of its own (so it must inherit Trammel from its parent), a
    /// self-closing `rect`, and a sibling non-town region whose rects must be
    /// dropped entirely.
    const SAMPLE: &str = r#"
        <ServerRegions>
          <Facet name="Felucca">
            <region type="TownRegion" name="Outer Town" map="Trammel">
              <rect x="100" y="200" width="30" height="40" />
              <region type="GuardedRegion" name="Inner Ward">
                <rect x="110" y="210" width="10" height="10" />
              </region>
            </region>
            <region type="DungeonRegion" name="Some Dungeon">
              <rect x="900" y="900" width="5" height="5" />
            </region>
          </Facet>
        </ServerRegions>
    "#;

    #[test]
    fn nested_town_region_inherits_facet_and_skips_non_guarded_siblings() {
        let rects = parse(SAMPLE);
        assert_eq!(
            rects.len(),
            2,
            "expected only the TownRegion + its guarded child, got {rects:?}"
        );
        assert!(rects.contains(&GuardRect {
            facet: 1,
            x: 100,
            y: 200,
            w: 30,
            h: 40
        }));
        assert!(rects.contains(&GuardRect {
            facet: 1,
            x: 110,
            y: 210,
            w: 10,
            h: 10
        }));
        assert!(
            !rects.iter().any(|r| r.x == 900),
            "DungeonRegion's rect must be ignored"
        );
    }

    /// Mirrors real `Regions.xml`'s Buccaneer's Den: a guarded-type region
    /// (`TownRegion`) whose `<guards disabled="true" />` comes *after* its
    /// `<rect>`s, so a naive "emit at rect-time" parser would wrongly keep
    /// them. Also covers scoping in both directions: the disabled parent's
    /// nested, unrelated `GuardedRegion` child (no disable of its own) must
    /// still be kept, and a sibling disabled `GuardedRegion` nested inside a
    /// *non*-disabled guarded parent must still be dropped — the flag never
    /// crosses a region boundary either way.
    #[test]
    fn guards_disabled_drops_only_its_own_regions_rects() {
        let xml = r#"
            <region type="TownRegion" name="Buccaneer's Den">
              <rect x="2612" y="2057" width="164" height="210" />
              <rect x="2604" y="2065" width="8" height="189" />
              <guards disabled="true" />
              <region type="GuardedRegion" name="Unrelated Nested Ward">
                <rect x="2700" y="2100" width="10" height="10" />
              </region>
            </region>
            <region type="GuardedRegion" name="Normal Town">
              <rect x="10" y="20" width="30" height="40" />
              <region type="GuardedRegion" name="Also Disabled Nested">
                <rect x="50" y="60" width="7" height="8" />
                <guards disabled="true" />
              </region>
            </region>
        "#;
        let rects = parse(xml);
        assert!(
            !rects.iter().any(|r| r.x == 2612 || r.x == 2604),
            "Buccaneer's Den's own rects must be dropped (guards disabled): {rects:?}"
        );
        assert!(
            rects.contains(&GuardRect { facet: 0, x: 2700, y: 2100, w: 10, h: 10 }),
            "the disabled parent's guarded child (itself not disabled) must still be kept: {rects:?}"
        );
        assert!(
            rects.contains(&GuardRect {
                facet: 0,
                x: 10,
                y: 20,
                w: 30,
                h: 40
            }),
            "Normal Town's own rect must be kept: {rects:?}"
        );
        assert!(
            !rects.iter().any(|r| r.x == 50),
            "Also Disabled Nested's rect must be dropped even though its parent isn't disabled: {rects:?}"
        );
        assert_eq!(
            rects.len(),
            2,
            "expected exactly Normal Town + Unrelated Nested Ward, got {rects:?}"
        );
    }

    /// ServUO's guarded-region set isn't just the literal `TownRegion`/
    /// `GuardedRegion` type names — several named subclasses (New Magincia,
    /// Tokuno Docks, Blackthorn Castle, the Custeau Perron house region) also
    /// extend `GuardedRegion` and must count as guarded too.
    #[test]
    fn guarded_subclass_types_are_recognized() {
        for ty in [
            "NewMaginciaRegion",
            "TokunoDocksRegion",
            "BlackthornCastle",
            "CusteauPerronHouseRegion",
        ] {
            let xml = format!(
                r#"<region type="{ty}" name="X"><rect x="1" y="2" width="3" height="4" /></region>"#
            );
            let rects = parse(&xml);
            assert_eq!(
                rects,
                vec![GuardRect {
                    facet: 0,
                    x: 1,
                    y: 2,
                    w: 3,
                    h: 4
                }],
                "type={ty} should be recognized as a guarded subclass"
            );
        }
    }

    #[test]
    fn facet_defaults_to_felucca_with_no_wrapping_facet_or_map() {
        let xml = r#"<region type="GuardedRegion" name="Bare"><rect x="1" y="2" width="3" height="4" /></region>"#;
        let rects = parse(xml);
        assert_eq!(
            rects,
            vec![GuardRect {
                facet: 0,
                x: 1,
                y: 2,
                w: 3,
                h: 4
            }]
        );
    }

    #[test]
    fn plain_region_type_is_not_guarded() {
        let xml = r#"<region type="Region" name="Wheatfield"><rect x="1" y="2" width="3" height="4" /></region>"#;
        assert!(parse(xml).is_empty());
    }

    #[test]
    fn region_with_no_type_attribute_is_not_guarded() {
        let xml =
            r#"<region name="Britain Mine 1"><rect x="1" y="2" width="3" height="4" /></region>"#;
        assert!(parse(xml).is_empty());
    }

    #[test]
    fn facet_names_map_to_the_expected_indices() {
        assert_eq!(facet_from_name("Felucca"), 0);
        assert_eq!(facet_from_name("Trammel"), 1);
        assert_eq!(facet_from_name("Ilshenar"), 2);
        assert_eq!(facet_from_name("Malas"), 3);
        assert_eq!(facet_from_name("Tokuno"), 4);
        assert_eq!(facet_from_name("TerMur"), 5);
        assert_eq!(facet_from_name("Nonsense"), 0);
    }

    #[test]
    fn comments_are_skipped_without_confusing_the_stack() {
        let xml = r#"
            <region type="TownRegion" name="Commented">
              <!-- a rect-shaped comment: <rect x="1" y="2" width="3" height="4" /> -->
              <rect x="5" y="6" width="7" height="8" />
            </region>
        "#;
        assert_eq!(
            parse(xml),
            vec![GuardRect {
                facet: 0,
                x: 5,
                y: 6,
                w: 7,
                h: 8
            }]
        );
    }

    /// Real-file smoke test: parses the actual ServUO `Regions.xml` (not
    /// checked into this repo) and checks it produced a plausible number of
    /// rects plus specific, hand-verified ones: Britain (Felucca) is present;
    /// Buccaneer's Den — the lawless pirate town, `<guards disabled="true"
    /// />` in both Felucca and Trammel — is absent; New Magincia (guarded
    /// subclass `NewMaginciaRegion`, not literally `TownRegion`/
    /// `GuardedRegion`) is present in both facets. `#[ignore]`d because it
    /// depends on a sibling checkout on disk; run explicitly with
    /// `cargo test -p anima-net -- --ignored --nocapture`.
    #[test]
    #[ignore = "reads the real ServUO Data/Regions.xml from a sibling checkout"]
    fn parses_the_real_regions_xml() {
        let path = resolve_path();
        let xml = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("couldn't read {}: {e}", path.display()));
        let rects = parse(&xml);
        println!(
            "parsed {} guarded rects from {}",
            rects.len(),
            path.display()
        );
        assert!(
            rects.len() > 100,
            "expected more than 100 guarded rects, got {}",
            rects.len()
        );

        let britain = GuardRect {
            facet: 0,
            x: 1416,
            y: 1498,
            w: 324,
            h: 279,
        };
        assert!(
            rects.contains(&britain),
            "expected Britain's guard-zone rect {britain:?} in the parsed set"
        );

        // Buccaneer's Den (`type="TownRegion"`, `<guards disabled="true" />`
        // after its rects) must be dropped in both facets it's defined in.
        let bucs_den_fel = GuardRect {
            facet: 0,
            x: 2612,
            y: 2057,
            w: 164,
            h: 210,
        };
        let bucs_den_tram = GuardRect {
            facet: 1,
            x: 2612,
            y: 2057,
            w: 164,
            h: 210,
        };
        assert!(
            !rects.contains(&bucs_den_fel),
            "Buccaneer's Den (Felucca) must be dropped: guards disabled"
        );
        assert!(
            !rects.contains(&bucs_den_tram),
            "Buccaneer's Den (Trammel) must be dropped: guards disabled"
        );

        // New Magincia (`type="NewMaginciaRegion"`, a guarded subclass, not
        // disabled) must be present in both facets it's defined in.
        let magincia_fel = GuardRect {
            facet: 0,
            x: 3632,
            y: 2032,
            w: 50,
            h: 70,
        };
        let magincia_tram = GuardRect {
            facet: 1,
            x: 3632,
            y: 2032,
            w: 50,
            h: 70,
        };
        assert!(
            rects.contains(&magincia_fel),
            "New Magincia (Felucca) must be present: guarded subclass"
        );
        assert!(
            rects.contains(&magincia_tram),
            "New Magincia (Trammel) must be present: guarded subclass"
        );
    }
}
