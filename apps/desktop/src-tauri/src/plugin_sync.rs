use crate::config::AppConfig;
use serde_json::{Map, Value};
use std::fs;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;
use windows::Win32::System::Threading::CREATE_NO_WINDOW;

const PKG_NAME: &str = "@local/anything-to-prompt";
const LEGACY_PKG_NAMES: &[&str] = &["@local/dsh-card-dock", "@local/deepseek-bar"];

const EMBEDDED_FILES: &[(&str, &str)] = &[
    (
        "package.json",
        include_str!("../../../../packages/anything-to-prompt/package.json"),
    ),
    (
        "cordis.patch.yml",
        include_str!("../../../../packages/anything-to-prompt/cordis.patch.yml"),
    ),
    (
        "icon.svg",
        include_str!("../../../../packages/anything-to-prompt/icon.svg"),
    ),
    (
        "LICENSE",
        include_str!("../../../../packages/anything-to-prompt/LICENSE"),
    ),
    (
        "lib/index.js",
        include_str!("../../../../packages/anything-to-prompt/lib/index.js"),
    ),
    (
        "lib/client.js",
        include_str!(concat!(env!("OUT_DIR"), "/generated_client.js")),
    ),
    (
        "locale/zh.json",
        include_str!("../../../../packages/anything-to-prompt/locale/zh.json"),
    ),
    (
        "locale/en.json",
        include_str!("../../../../packages/anything-to-prompt/locale/en.json"),
    ),
    (
        "docs/DETAIL.zh.md",
        include_str!("../../../../packages/anything-to-prompt/docs/DETAIL.zh.md"),
    ),
    (
        "docs/preview.svg",
        include_str!("../../../../packages/anything-to-prompt/docs/preview.svg"),
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginSyncResult {
    NoDshProfile,
    UpToDate,
    InstalledOrUpdated,
}

/// Ensures Anything-to-Prompt's single built-in DSH companion plugin (`@local/anything-to-prompt`)
/// is deployed into `<DSH_HOME>/profiles/web/local-plugins/anything-to-prompt` and registered in `package.json`,
/// while cleaning up legacy `@local/dsh-card-dock` / `@local/deepseek-bar` manifest entries.
pub fn ensure_dsh_plugin_synced() -> PluginSyncResult {
    let dsh_home = AppConfig::dsh_home_dir();
    let profile_dir = dsh_home.join("profiles").join("web");
    let manifest_path = profile_dir.join("package.json");
    if !manifest_path.exists() {
        return PluginSyncResult::NoDshProfile;
    }

    let plugin_dir = profile_dir.join("local-plugins").join("anything-to-prompt");
    let mut changed = false;

    for (rel, content) in EMBEDDED_FILES {
        let dest = plugin_dir.join(rel);
        let needs_write = match fs::read_to_string(&dest) {
            Ok(existing) => existing != *content,
            Err(_) => true,
        };
        if needs_write {
            if let Some(parent) = dest.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if fs::write(&dest, content).is_ok() {
                changed = true;
            }
        }
    }

    // Ensure node_modules/@local/anything-to-prompt links to local-plugins/anything-to-prompt
    let scope_dir = profile_dir.join("node_modules").join("@local");
    let _ = fs::create_dir_all(&scope_dir);
    let link_path = scope_dir.join("anything-to-prompt");
    if !link_path.join("package.json").exists() {
        let _ = remove_path_if_exists(&link_path);
        let status = Command::new("cmd.exe")
            .args([
                "/C",
                "mklink",
                "/J",
                &link_path.to_string_lossy(),
                &plugin_dir.to_string_lossy(),
            ])
            .creation_flags(CREATE_NO_WINDOW.0)
            .status();
        if status.map(|s| s.success()).unwrap_or(false) {
            changed = true;
        }
    }

    // Clean up legacy @local/dsh-card-dock junction if present
    let legacy_dock_link = scope_dir.join("dsh-card-dock");
    if legacy_dock_link.exists() {
        let _ = remove_path_if_exists(&legacy_dock_link);
        changed = true;
    }

    // If an already-running DSH instance is still serving the pre-rename @local/deepseek-bar client
    // (before its next restart), keep that client.js mirrored so zero downtime occurs on upgrade.
    let legacy_bar_client = profile_dir
        .join("local-plugins")
        .join("deepseek-bar")
        .join("lib")
        .join("client.js");
    let legacy_instances_dir = dsh_home.join("deepseek-bar").join("instances");
    if legacy_bar_client.exists() && legacy_instances_dir.exists() {
        if let Some((_, client_src)) = EMBEDDED_FILES.iter().find(|(k, _)| *k == "lib/client.js") {
            let compat_src = client_src.replace(
                "id: '@local/anything-to-prompt'",
                "id: '@local/deepseek-bar'",
            );
            let needs_mirror = fs::read_to_string(&legacy_bar_client)
                .map(|s| s != compat_src)
                .unwrap_or(true);
            if needs_mirror {
                let _ = fs::write(&legacy_bar_client, compat_src);
            }
        }
    } else {
        let legacy_bar_link = scope_dir.join("deepseek-bar");
        if legacy_bar_link.exists() {
            let _ = remove_path_if_exists(&legacy_bar_link);
            changed = true;
        }
    }

    // Update profiles/web/package.json cleanly
    if let Ok(raw) = fs::read_to_string(&manifest_path) {
        if let Some(updated) = sync_profile_manifest_json(&raw) {
            if fs::write(&manifest_path, updated).is_ok() {
                changed = true;
            }
        }
    }

    if changed {
        log::info!(
            "Synced built-in @local/anything-to-prompt plugin to {}",
            plugin_dir.display()
        );
        PluginSyncResult::InstalledOrUpdated
    } else {
        PluginSyncResult::UpToDate
    }
}

fn remove_path_if_exists(path: &Path) -> std::io::Result<()> {
    if !path.exists() && fs::symlink_metadata(path).is_err() {
        return Ok(());
    }
    if fs::remove_dir(path).is_ok() {
        return Ok(());
    }
    fs::remove_dir_all(path)
}

/// Pure manifest transformer: ensures `@local/anything-to-prompt` is in `dependencies` and `dsh.profile.bundles`,
/// removes any legacy `@local/dsh-card-dock` / `@local/deepseek-bar` entry, and returns `Some(pretty_json)` only when modified.
pub fn sync_profile_manifest_json(raw: &str) -> Option<String> {
    let clean = raw.trim_start_matches('\u{feff}');
    let mut root: Value = serde_json::from_str(clean).ok()?;
    let obj = root.as_object_mut()?;

    let mut dirty = false;

    // 1. dependencies
    let deps = obj
        .entry("dependencies")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()?;
    for legacy in LEGACY_PKG_NAMES {
        if deps.remove(*legacy).is_some() {
            dirty = true;
        }
    }
    let expected_dep = "link:local-plugins\\anything-to-prompt";
    if deps.get(PKG_NAME).and_then(|v| v.as_str()) != Some(expected_dep) {
        deps.insert(
            PKG_NAME.to_string(),
            Value::String(expected_dep.to_string()),
        );
        dirty = true;
    }

    // 2. dsh.profile.bundles
    let dsh = obj
        .entry("dsh")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()?;
    let profile = dsh
        .entry("profile")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()?;
    let bundles = profile
        .entry("bundles")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()?;

    let prev_len = bundles.len();
    bundles.retain(|v| {
        v.as_str()
            .map(|s| !LEGACY_PKG_NAMES.contains(&s))
            .unwrap_or(true)
    });
    if bundles.len() != prev_len {
        dirty = true;
    }
    if !bundles.iter().any(|v| v.as_str() == Some(PKG_NAME)) {
        bundles.push(Value::String(PKG_NAME.to_string()));
        dirty = true;
    }

    if !dirty {
        return None;
    }
    serde_json::to_string_pretty(&root)
        .ok()
        .map(|s| format!("{s}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_manifest_replaces_legacy_plugins_with_anything_to_prompt_and_is_idempotent() {
        let input = r#"{
  "name": "dsh-profile-web",
  "dependencies": {
    "@local/dsh-card-dock": "link:../old",
    "@local/deepseek-bar": "link:local-plugins\\deepseek-bar",
    "dsh-context": "0.84.0"
  },
  "dsh": {
    "profile": {
      "bundles": [
        "@deepseek-ai/dsh-base",
        "@local/dsh-card-dock",
        "@local/deepseek-bar"
      ]
    }
  }
}"#;
        let out = sync_profile_manifest_json(input).expect("should update manifest");
        assert!(!out.contains("dsh-card-dock"));
        assert!(!out.contains("deepseek-bar"));
        assert!(out.contains(
            "\"@local/anything-to-prompt\": \"link:local-plugins\\\\anything-to-prompt\""
        ));
        assert!(out.contains("\"@local/anything-to-prompt\""));

        // Second pass must be a clean no-op
        assert!(sync_profile_manifest_json(&out).is_none());
    }
}
