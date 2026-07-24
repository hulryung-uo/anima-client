//! Locating the Ultima Online client-data directory (the folder holding
//! `tiledata.mul` / `anim.mul` / the art `.uop`s).
//!
//! Both entry points — the `play` bin (CLI) and `anima-desktop` (GUI) — need to
//! find this folder without the user hand-typing a path on every launch. This
//! module holds the *shared, side-effect-light* pieces: validating a candidate
//! ([`looks_like_uo_data`]) and searching known install locations
//! ([`detect_uo_dir`], including a best-effort ClassicUO `settings.json` read).
//! Persistence and *asking* the user differ per frontend (a dotfile + stdin for
//! `play`; a Tauri store + native folder picker for the desktop), so those stay
//! in each bin.

use std::path::{Path, PathBuf};

/// True if `dir` looks like a UO client-data directory. `tiledata.mul` and
/// `anim.mul` ship in every classic client (legacy + UOP); the art files cover
/// the odd repacked layout. Cheap `is_file` probes — no directory scan.
pub fn looks_like_uo_data(dir: &Path) -> bool {
    dir.join("tiledata.mul").is_file()
        || dir.join("anim.mul").is_file()
        || dir.join("artLegacyMUL.uop").is_file()
        || dir.join("art.mul").is_file()
}

/// Search known install locations and return the first that
/// [`looks_like_uo_data`]. Order of preference: a UO install already configured
/// in ClassicUO (read from its `settings.json`), then common dev/macOS/Windows
/// paths, then a couple relative to the current directory (running from a repo
/// checkout). Returns `None` if nothing plausible is found — the caller then
/// asks the user.
pub fn detect_uo_dir() -> Option<PathBuf> {
    uo_dir_from_classicuo()
        .into_iter()
        .chain(candidate_dirs())
        .find(|d| looks_like_uo_data(d))
}

/// Well-known places a UO client is commonly installed, in priority order.
fn candidate_dirs() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut dirs: Vec<PathBuf> = [
        // Dev layout used across this workspace (see CLAUDE.md / play bin default).
        format!("{home}/dev/uo/uo-resource"),
        // macOS installs.
        "/Applications/Ultima Online Classic".to_string(),
        "/Applications/Ultima Online".to_string(),
        format!("{home}/Ultima Online Classic"),
        format!("{home}/Library/Application Support/Ultima Online Classic"),
        format!("{home}/Documents/Ultima Online Classic"),
        format!("{home}/uo"),
        format!("{home}/UO"),
        "/opt/uo".to_string(),
        // Windows install roots (harmless on macOS — they just won't exist).
        "C:/Program Files (x86)/Electronic Arts/Ultima Online Classic".to_string(),
        "C:/Program Files/Electronic Arts/Ultima Online Classic".to_string(),
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect();
    // Relative to the current directory, for `cargo run` from a repo checkout.
    dirs.push(PathBuf::from("../uo-resource"));
    dirs.push(PathBuf::from("uo-resource"));
    dirs
}

/// Best-effort: a machine already running ClassicUO records its UO path in
/// `settings.json` (`"ultimaonlinedirectory": "…"`). We string-scan for that key
/// at ClassicUO's usual config locations — no JSON dependency (the file is flat
/// and the value is a plain path string). The returned path is *unvalidated*;
/// [`detect_uo_dir`] filters it through [`looks_like_uo_data`].
fn uo_dir_from_classicuo() -> Option<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    let spots = [
        format!("{home}/.local/share/ClassicUO/settings.json"),
        format!("{home}/Library/Application Support/ClassicUO/settings.json"),
        format!("{home}/ClassicUO/settings.json"),
        format!("{home}/Documents/ClassicUO/settings.json"),
    ];
    for spot in spots {
        if let Ok(text) = std::fs::read_to_string(&spot) {
            if let Some(dir) = parse_json_string_field(&text, "ultimaonlinedirectory") {
                return Some(dir);
            }
        }
    }
    None
}

/// Extract a JSON string field's value by key, unescaping `\\` and `\/`. Good
/// enough for ClassicUO's flat settings file (paths never contain a `"`); not a
/// general JSON parser.
fn parse_json_string_field(json: &str, key: &str) -> Option<PathBuf> {
    let needle = format!("\"{key}\"");
    let after_key = &json[json.find(&needle)? + needle.len()..];
    let after_colon = &after_key[after_key.find(':')? + 1..];
    let open = after_colon.find('"')? + 1;
    let rest = &after_colon[open..];
    let close = rest.find('"')?;
    let unescaped = rest[..close].replace("\\\\", "\\").replace("\\/", "/");
    (!unescaped.is_empty()).then(|| PathBuf::from(unescaped))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_unix_and_windows_classicuo_paths() {
        let unix = r#"{ "username": "x", "ultimaonlinedirectory": "/home/u/uo", "port": 2593 }"#;
        assert_eq!(
            parse_json_string_field(unix, "ultimaonlinedirectory"),
            Some(PathBuf::from("/home/u/uo"))
        );
        // ClassicUO writes Windows paths with escaped backslashes.
        let win = r#"{"ultimaonlinedirectory":"C:\\Program Files\\UO","x":1}"#;
        assert_eq!(
            parse_json_string_field(win, "ultimaonlinedirectory"),
            Some(PathBuf::from(r"C:\Program Files\UO"))
        );
    }

    #[test]
    fn missing_or_empty_field_is_none() {
        assert_eq!(parse_json_string_field(r#"{"a":"b"}"#, "ultimaonlinedirectory"), None);
        assert_eq!(parse_json_string_field(r#"{"ultimaonlinedirectory":""}"#, "ultimaonlinedirectory"), None);
    }
}
