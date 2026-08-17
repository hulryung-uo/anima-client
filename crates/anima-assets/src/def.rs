//! Shared `.def` alias-table parser (`index {a, b, c} hue`).
//!
//! ClassicUO `DefReader` used by `art.def`, `gump.def`, and `TexTerr.def`:
//! when the named index has no art, try each group member in order and take
//! the first that exists, plus the line's hue. First line for an index wins.

use std::collections::HashMap;

/// `index → (alias ids in order, hue)`. Hue is 0 when the line omits it.
pub fn parse_alias_def(text: &str) -> HashMap<u32, (Vec<u32>, u16)> {
    let mut map = HashMap::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let (Some(open), Some(close)) = (line.find('{'), line.find('}')) else {
            continue;
        };
        if close < open {
            continue;
        }
        let Some(index) = parse_def_int(line[..open].trim()) else {
            continue;
        };
        if index < 0 {
            continue;
        }
        let group: Vec<u32> = line[open + 1..close]
            .split(',')
            .filter_map(|t| parse_def_int(t.trim()))
            .filter(|&n| n >= 0)
            .map(|n| n as u32)
            .collect();
        if group.is_empty() {
            continue;
        }
        let hue = parse_def_int(line[close + 1..].trim())
            .unwrap_or(0)
            .clamp(0, 0xFFFF) as u16;
        map.entry(index as u32).or_insert((group, hue));
    }
    map
}

/// `group {replace}` used by `Anim1.def` / `Anim2.def`.
/// `replace == 0xFF` (or a negative, which ClassicUO casts to byte 0xFF) means
/// "use the walk group". `group == 0xFFFF` is skipped.
pub fn parse_group_replace(text: &str) -> Vec<(u16, u8)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let (Some(open), Some(close)) = (line.find('{'), line.find('}')) else {
            continue;
        };
        if close < open {
            continue;
        }
        let Some(group) = parse_def_int(line[..open].trim()) else {
            continue;
        };
        if !(0..=0xFFFE).contains(&group) {
            continue;
        }
        let Some(replace) = line[open + 1..close]
            .split(',')
            .next()
            .and_then(|t| parse_def_int(t.trim()))
        else {
            continue;
        };
        // ClassicUO stores `(byte)replace`; −1 becomes 0xFF = "use walk".
        let replace = if replace < 0 {
            0xFF
        } else {
            (replace as u16 & 0xFF) as u8
        };
        out.push((group as u16, replace));
    }
    out
}

fn parse_def_int(t: &str) -> Option<i64> {
    let t = t.trim();
    if t.is_empty() {
        return None;
    }
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        i64::from_str_radix(hex, 16).ok()
    } else {
        t.parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_alias_def_takes_first_line_and_group() {
        let map = parse_alias_def(
            "\
# comment
2500\t{3, 4}\t1645
2500\t{9}\t1
50759 {50468} 0
",
        );
        assert_eq!(map.get(&2500), Some(&(vec![3, 4], 1645)));
        assert_eq!(map.get(&50759), Some(&(vec![50468], 0)));
    }

    #[test]
    fn parse_group_replace_maps_minus_one_to_ff() {
        let rows = parse_group_replace(
            "\
13 {5} 0
35 {-1} 0
0xFFFF {1} 0
",
        );
        assert_eq!(rows, vec![(13, 5), (35, 0xFF)]);
    }
}
