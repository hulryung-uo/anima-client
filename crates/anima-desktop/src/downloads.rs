//! Only the renderer's local settings/profile exports may use the native downloader.
use std::path::Path;
use tauri::Url;

pub fn from_renderer(url: &Url, current: &Url, port: u16) -> bool {
    let origin = format!("http://127.0.0.1:{port}");
    current.origin().ascii_serialization() == origin
        && current.path() == "/"
        && url.as_str().starts_with(&format!("blob:{origin}/"))
}
pub fn settings_file(destination: &Path, downloads: &Path) -> bool {
    if destination.parent() != Some(downloads) {
        return false;
    }
    let Some(name) = destination.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let Some(stem) = name.strip_suffix(".json") else {
        return false;
    };
    ["anima-settings", "anima-settings-recovery", "anima-worlds"]
        .iter()
        .any(|base| {
            stem == *base
                || stem
                    .strip_prefix(&format!("{base} ("))
                    .and_then(|s| s.strip_suffix(')'))
                    .is_some_and(|n| !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exports_are_bound_to_the_active_loopback_origin() {
        let current = "http://127.0.0.1:8190/".parse().unwrap();
        let blob = "blob:http://127.0.0.1:8190/fixture".parse().unwrap();
        assert!(from_renderer(&blob, &current, 8190));
        assert!(!from_renderer(&blob, &current, 8191));
        assert!(!from_renderer(
            &blob,
            &"https://example.com/".parse().unwrap(),
            8190
        ));
        assert!(!from_renderer(&current, &current, 8190));
    }
    #[test]
    fn exports_stay_in_downloads_and_allow_unique_json_names() {
        let folder = Path::new("downloads");
        for name in [
            "anima-settings.json",
            "anima-settings (2).json",
            "anima-settings-recovery.json",
            "anima-settings-recovery (3).json",
            "anima-worlds.json",
            "anima-worlds (2).json",
        ] {
            assert!(settings_file(&folder.join(name), folder));
        }
        for name in ["../anima-settings.json", "other.json", "anima-settings.exe"] {
            assert!(!settings_file(&folder.join(name), folder));
        }
    }
}
