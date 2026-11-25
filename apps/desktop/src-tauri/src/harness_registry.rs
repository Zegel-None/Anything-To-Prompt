use serde::Deserialize;

include!(concat!(env!("OUT_DIR"), "/generated_harness_registry.rs"));

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedHarnessManifest {
    pub id: String,
    #[serde(default)]
    pub order: i32,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub label: String,
    pub full_label: String,
    #[serde(default)]
    pub badge: String,
    #[serde(default)]
    pub sync_hint: String,
    #[serde(default)]
    pub use_native_composer: bool,
}

pub fn load_builtin_manifests() -> Vec<ParsedHarnessManifest> {
    BUILTIN_HARNESS_MANIFESTS
        .iter()
        .filter_map(|entry| serde_json::from_str::<ParsedHarnessManifest>(entry.manifest_json).ok())
        .collect()
}

pub fn resolve_registered_harness_id(raw: &str) -> Option<String> {
    let needle = raw.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return None;
    }
    for m in load_builtin_manifests() {
        if m.id.eq_ignore_ascii_case(&needle)
            || m.aliases.iter().any(|a| a.eq_ignore_ascii_case(&needle))
        {
            return Some(m.id);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_discovers_all_builtin_harness_folders_in_order() {
        let manifests = load_builtin_manifests();
        assert_eq!(
            manifests.len(),
            BUILTIN_HARNESS_MANIFESTS.len(),
            "every harness.json in harnesses/<id>/ must parse cleanly"
        );
        let ids: Vec<&str> = manifests.iter().map(|m| m.id.as_str()).collect();
        assert!(ids.contains(&"dsh"));
        assert!(ids.contains(&"antigravity"));
        assert!(ids.contains(&"codex"));
        assert!(ids.contains(&"claude_code"));
        assert!(
            !ids.iter().any(|id| id.starts_with('_') || *id == "my_harness"),
            "_template folder must be skipped during auto-discovery"
        );
        // Verify deterministic ascending order
        for pair in manifests.windows(2) {
            assert!(pair[0].order <= pair[1].order);
        }
        assert_eq!(
            resolve_registered_harness_id("agy").as_deref(),
            Some("antigravity")
        );
        assert_eq!(
            resolve_registered_harness_id("claude-code").as_deref(),
            Some("claude_code")
        );
    }

    #[test]
    fn generated_client_js_embeds_all_discovered_harnesses() {
        let generated = include_str!(concat!(env!("OUT_DIR"), "/generated_client.js"));
        assert!(
            !generated.contains("/*__ATP_HARNESS_MODULES__*/ []"),
            "marker must be replaced with auto-discovered modules"
        );
        for id in ["dsh", "antigravity", "codex", "claude_code"] {
            assert!(
                generated.contains(&format!("\"id\": \"{id}\"")),
                "generated_client.js should embed harness manifest for {id}"
            );
        }
    }
}
