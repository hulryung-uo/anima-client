//! Read-only installation checks for the desktop setup screen. Kept beside
//! directory discovery so the native shell does not duplicate asset formats.
use anima_assets::{Anim, Art, Gumps, MapData};
use serde::Serialize;
use std::fs::File;
use std::io;
use std::path::Path;

#[derive(Clone, Serialize)]
pub struct FileCheck {
    pub name: &'static str,
    pub required: bool,
    pub ready: bool,
    pub detail: String,
}
#[derive(Clone, Serialize)]
pub struct DataReport {
    pub path: String,
    pub ready: bool,
    pub checks: Vec<FileCheck>,
}

fn readable(dir: &Path, names: &[&str], minimum: u64) -> io::Result<()> {
    for name in names {
        let file = File::open(dir.join(name))
            .map_err(|e| io::Error::new(e.kind(), format!("{name}: {e}")))?;
        if !file.metadata()?.is_file() || file.metadata()?.len() < minimum {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{name} is empty or incomplete."),
            ));
        }
    }
    Ok(())
}

/// Open the same essential readers the game uses, without changing any file.
/// This is a startup check, not an exhaustive integrity scan of every asset.
pub fn inspect(dir: &Path) -> DataReport {
    let check = |name, expected: &str, result: io::Result<()>| FileCheck {
        name,
        required: true,
        ready: result.is_ok(),
        detail: result.map_or_else(
            |e| {
                let help = match e.kind() {
                    io::ErrorKind::NotFound => "Required files are missing.",
                    io::ErrorKind::PermissionDenied => {
                        "Cannot read these files. Check the folder's permissions."
                    }
                    io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof => {
                        "Files are incomplete or incompatible. Check your shard's client package."
                    }
                    _ => "Could not open these files. Check your shard's client package.",
                };
                format!("{expected} — {help}")
            },
            |_| expected.into(),
        ),
    };
    let mut checks = vec![check(
        "Tile definitions",
        "tiledata.mul",
        // 512 pre-HS land groups and at least one static group.
        readable(dir, &["tiledata.mul"], 512 * 836 + 1188),
    )];
    checks.push(check(
        "World and buildings",
        "map0LegacyMUL.uop, staidx0.mul, statics0.mul",
        readable(dir, &["staidx0.mul"], 12)
            .and_then(|_| readable(dir, &["statics0.mul"], 0))
            .and_then(|_| MapData::open(dir).map(|_| ())),
    ));
    checks.push(check(
        "World artwork",
        "artLegacyMUL.uop or art.mul + artidx.mul",
        if dir.join("artLegacyMUL.uop").is_file() {
            readable(dir, &["artLegacyMUL.uop"], 28)
        } else {
            readable(dir, &["art.mul", "artidx.mul"], 12)
        }
        .and_then(|_| Art::open(dir).map(|_| ())),
    ));
    checks.push(check(
        "Character animations",
        "anim.mul + anim.idx (expansion animations are optional)",
        readable(dir, &["anim.mul", "anim.idx"], 12).and_then(|_| Anim::open(dir).map(|_| ())),
    ));
    checks.push(check(
        "Interface artwork",
        "gumpartLegacyMUL.uop or gumpart.mul + gumpidx.mul",
        if dir.join("gumpartLegacyMUL.uop").is_file() {
            readable(dir, &["gumpartLegacyMUL.uop"], 28)
        } else {
            let mul = if dir.join("gumpart.mul").is_file() {
                "gumpart.mul"
            } else {
                "Gumpart.mul"
            };
            let idx = if dir.join("gumpidx.mul").is_file() {
                "gumpidx.mul"
            } else {
                "Gumpidx.mul"
            };
            readable(dir, &[mul, idx], 12)
        }
        .and_then(|_| Gumps::open(dir).map(|_| ())),
    ));
    for (name, files, detail) in [
        (
            "Sound",
            &["soundLegacyMUL.uop", "sound.mul"][..],
            "Optional sound effects",
        ),
        (
            "Text and names",
            &["Cliloc.enu"][..],
            "Optional English localized names",
        ),
    ] {
        let ready = files.iter().any(|name| readable(dir, &[*name], 1).is_ok());
        checks.push(FileCheck {
            name,
            required: false,
            ready,
            detail: detail.into(),
        });
    }
    DataReport {
        path: dir.to_string_lossy().into_owned(),
        ready: checks.iter().filter(|c| c.required).all(|c| c.ready),
        checks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absent_or_partial_installation_never_claims_readiness() {
        let dir = std::env::temp_dir().join(format!("anima-files-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("tiledata.mul"), [0; 4]).unwrap();
        let report = inspect(&dir);
        assert!(!report.ready);
        assert_eq!(
            report
                .checks
                .iter()
                .filter(|c| c.required && !c.ready)
                .count(),
            5
        );
        assert!(report.checks[0].detail.contains("incomplete"));
        assert!(report.checks[1].detail.contains("map0LegacyMUL.uop"));
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    #[ignore = "requires ANIMA_TEST_UO_DIR with real UO client data"]
    fn installed_game_files_pass_the_same_readers_used_by_play() {
        let dir = std::env::var_os("ANIMA_TEST_UO_DIR").expect("ANIMA_TEST_UO_DIR");
        let report = inspect(Path::new(&dir));
        for item in &report.checks {
            assert!(
                !item.required || item.ready,
                "{}: {}",
                item.name,
                item.detail
            );
        }
        assert!(report.ready);
    }
}
