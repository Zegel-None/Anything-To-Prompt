fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "bar_report",
            "bar_reconnect",
            "overlay_compose_done",
        ]),
    ))
    .expect("failed to run tauri-build");
}
