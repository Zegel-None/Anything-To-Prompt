use std::env;
use std::fs;
use std::path::{Path, PathBuf};

struct DiscoveredHarness {
    id: String,
    order: i32,
    manifest_json: String,
    ui_js: String,
}

fn extract_json_string(src: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let pos = src.find(&needle)?;
    let rest = &src[pos + needle.len()..];
    let colon = rest.find(':')?;
    let after_colon = rest[colon + 1..].trim_start();
    if !after_colon.starts_with('"') {
        return None;
    }
    let mut out = String::new();
    let mut escaped = false;
    for ch in after_colon[1..].chars() {
        if escaped {
            out.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            return Some(out);
        }
        out.push(ch);
    }
    None
}

fn extract_json_i32(src: &str, key: &str) -> Option<i32> {
    let needle = format!("\"{key}\"");
    let pos = src.find(&needle)?;
    let rest = &src[pos + needle.len()..];
    let colon = rest.find(':')?;
    let after_colon = rest[colon + 1..].trim_start();
    let digits: String = after_colon
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '-')
        .collect();
    digits.parse::<i32>().ok()
}

fn discover_harnesses(repo_root: &Path) -> Vec<DiscoveredHarness> {
    let harnesses_dir = repo_root.join("harnesses");
    println!("cargo:rerun-if-changed={}", harnesses_dir.display());

    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(&harnesses_dir) else {
        return found;
    };

    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Some(folder_name) = dir.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if folder_name.starts_with('_') || folder_name.starts_with('.') {
            continue;
        }
        let manifest_path = dir.join("harness.json");
        if !manifest_path.is_file() {
            continue;
        }
        println!("cargo:rerun-if-changed={}", manifest_path.display());
        let manifest_json = fs::read_to_string(&manifest_path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", manifest_path.display()));
        let id = extract_json_string(&manifest_json, "id")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| folder_name.to_string());
        let order = extract_json_i32(&manifest_json, "order").unwrap_or(500);

        let ui_path = dir.join("ui.js");
        let ui_js = if ui_path.is_file() {
            println!("cargo:rerun-if-changed={}", ui_path.display());
            fs::read_to_string(&ui_path)
                .unwrap_or_else(|e| panic!("failed to read {}: {e}", ui_path.display()))
        } else {
            "(function () { return {}; })".to_string()
        };

        found.push(DiscoveredHarness {
            id,
            order,
            manifest_json,
            ui_js,
        });
    }

    found.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id)));
    found
}

fn generate_harness_artifacts(repo_root: &Path, out_dir: &Path) {
    let harnesses = discover_harnesses(repo_root);

    // 1. Assemble generated_client.js with auto-discovered harness modules injected cleanly
    let client_path = repo_root
        .join("packages")
        .join("anything-to-prompt")
        .join("lib")
        .join("client.js");
    println!("cargo:rerun-if-changed={}", client_path.display());
    let client_template = fs::read_to_string(&client_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", client_path.display()));

    let mut js_entries = String::from("[\n");
    for h in &harnesses {
        let trimmed_ui = h.ui_js.trim().trim_end_matches(';');
        js_entries.push_str("      {\n");
        js_entries.push_str("        manifest: ");
        js_entries.push_str(h.manifest_json.trim());
        js_entries.push_str(",\n");
        js_entries.push_str("        initUi: (\n");
        js_entries.push_str(trimmed_ui);
        js_entries.push_str("\n        ),\n");
        js_entries.push_str("      },\n");
    }
    js_entries.push_str("    ]");

    const MARKER: &str = "/*__ATP_HARNESS_MODULES__*/ []";
    assert!(
        client_template.contains(MARKER),
        "client.js is missing harness module injection marker: {MARKER}"
    );
    let generated_client = client_template.replacen(MARKER, &js_entries, 1);
    fs::write(out_dir.join("generated_client.js"), generated_client)
        .expect("failed to write generated_client.js");

    // 2. Assemble generated_harness_registry.rs for Rust-side compile-time discovery & validation
    let mut rs = String::from(
        "#[derive(Debug, Clone, Copy)]\npub struct BuiltinHarnessManifest {\n    pub id: &'static str,\n    pub order: i32,\n    pub manifest_json: &'static str,\n}\n\npub const BUILTIN_HARNESS_MANIFESTS: &[BuiltinHarnessManifest] = &[\n",
    );
    for h in &harnesses {
        rs.push_str("    BuiltinHarnessManifest {\n");
        rs.push_str(&format!("        id: {:?},\n", h.id));
        rs.push_str(&format!("        order: {},\n", h.order));
        rs.push_str(&format!("        manifest_json: {:?},\n", h.manifest_json));
        rs.push_str("    },\n");
    }
    rs.push_str("];\n");
    fs::write(out_dir.join("generated_harness_registry.rs"), rs)
        .expect("failed to write generated_harness_registry.rs");
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let repo_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .expect("failed to resolve repo root from CARGO_MANIFEST_DIR");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    generate_harness_artifacts(repo_root, &out_dir);

    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "bar_report",
            "bar_reconnect",
            "bar_drag_begin",
            "bar_drag_step",
            "bar_drag_end",
            "bar_drag_reset",
            "overlay_compose_done",
            "harness_get_active",
            "harness_set_active",
            "harness_list_available",
            "harness_list_options",
            "harness_load_catalog",
            "harness_load_messages",
            "harness_send_message",
            "harness_cancel_turn",
            "harness_focus_desktop",
            "harness_session_stats",
            "harness_session_state",
            "app_get_settings",
            "app_update_settings",
            "app_settings_action",
            "app_ensure_default_workspace",
            "app_pick_folder",
            "app_pick_files",
            "app_read_clipboard_files",
            "app_describe_paths",
            "app_save_clipboard_image",
            "quick_search_query",
            "quick_search_status",
            "quick_search_build_capsule",
            "quick_search_open_path",
            "quick_search_refresh",
        ]),
    ))
    .expect("failed to run tauri-build");
}
