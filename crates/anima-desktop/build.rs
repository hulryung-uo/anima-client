fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "setup_status",
            "setup_check",
            "setup_choose",
            "setup_detect",
            "setup_recover",
            "setup_apply",
            "setup_close",
            "setup_restart",
        ]),
    ))
    .expect("build desktop capabilities");
}
