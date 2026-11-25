fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "bar_report",
            "bar_reconnect",
            "overlay_compose_done",
            "harness_get_active",
            "harness_set_active",
            "harness_list_available",
            "harness_load_catalog",
            "harness_load_messages",
            "harness_send_message",
            "harness_cancel_turn",
            "harness_focus_desktop",
            "quick_search_query",
            "quick_search_status",
            "quick_search_build_capsule",
            "quick_search_open_path",
            "quick_search_refresh",
        ]),
    ))
    .expect("failed to run tauri-build");
}
