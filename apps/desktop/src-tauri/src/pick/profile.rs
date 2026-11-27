use crate::config::AppConfig;
use crate::pick::adapters::solidworks;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

pub const PROFILE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiaPolicy {
    Preferred,
    WatchdogOnly,
    Forbidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroundingLayer {
    Uia,
    MsaaHitMap,
    StaticVisualHitMap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileSource {
    Builtin,
    Learned,
    Imported,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LearnedRegionRule {
    pub class_name: String,
    pub layout_kind: String, // "horizontal_ribbon" | "vertical_icon_strip" | "tree_or_menu_rows" | "icon_card_grid" | "auto"
    #[serde(default)]
    pub is_popup_menu: bool,
    #[serde(default)]
    pub discovered_count: usize,
    /// Scheme B strategy memory: remembers that this `(exe, container_class)` benefits from
    /// auto-waking OmniParser v2 in the Stealth Scout Halo (ZERO hardcoded coordinates!).
    #[serde(default)]
    pub auto_wake_omniparser: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUiProfile {
    pub schema_version: u32,
    pub exe_name: String,
    pub display_name: String,
    pub source: ProfileSource,
    pub uia_policy: UiaPolicy,
    pub preferred_layers: Vec<GroundingLayer>,
    pub ignore_background: bool,
    #[serde(default)]
    pub status_bar_classes: Vec<String>,
    #[serde(default)]
    pub toolbar_classes: Vec<String>,
    #[serde(default)]
    pub ignore_canvas_classes: Vec<String>,
    #[serde(default)]
    pub learned_regions: Vec<LearnedRegionRule>,
    #[serde(default)]
    pub semantic_adapter: Option<String>,
    #[serde(default = "default_uia_timeout_ms")]
    pub uia_timeout_ms: u64,
    #[serde(default)]
    pub learned_at_unix: i64,
    #[serde(default)]
    pub notes: String,
}

fn default_uia_timeout_ms() -> u64 {
    120
}

pub fn normalize_exe_key(raw: &str) -> String {
    let trimmed = raw.trim().trim_matches('"');
    let base = trimmed
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(trimmed)
        .trim()
        .to_ascii_lowercase();
    if base.is_empty() {
        return String::new();
    }
    if base.ends_with(".exe") {
        base
    } else {
        format!("{base}.exe")
    }
}

pub fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Hard safety invariants that no learned or imported profile may ever violate.
pub fn enforce_hard_invariants(profile: &mut AppUiProfile) {
    profile.schema_version = PROFILE_SCHEMA_VERSION;
    profile.exe_name = normalize_exe_key(&profile.exe_name);
    profile.uia_timeout_ms = profile.uia_timeout_ms.clamp(40, 800);

    // Hard rule: NEVER touch UIAutomation on SLDWORKS.exe.
    if solidworks::is_solidworks_exe(&profile.exe_name) {
        profile.uia_policy = UiaPolicy::Forbidden;
        profile.preferred_layers.retain(|l| *l != GroundingLayer::Uia);
        if profile.preferred_layers.is_empty() {
            profile.preferred_layers = vec![
                GroundingLayer::MsaaHitMap,
                GroundingLayer::StaticVisualHitMap,
            ];
        }
        profile.ignore_background = true;
        profile.semantic_adapter = Some("solidworks".to_string());
    }

    if profile.uia_policy == UiaPolicy::Forbidden {
        profile.preferred_layers.retain(|l| *l != GroundingLayer::Uia);
    }
    if profile.preferred_layers.is_empty() {
        profile.preferred_layers = vec![
            GroundingLayer::MsaaHitMap,
            GroundingLayer::StaticVisualHitMap,
        ];
    }
}

pub fn default_unknown_profile(exe_name: &str) -> AppUiProfile {
    let key = normalize_exe_key(exe_name);
    let display = key
        .strip_suffix(".exe")
        .unwrap_or(&key)
        .to_string();
    let mut p = AppUiProfile {
        schema_version: PROFILE_SCHEMA_VERSION,
        exe_name: key,
        display_name: if display.is_empty() {
            "Unknown App".to_string()
        } else {
            display
        },
        source: ProfileSource::Builtin,
        uia_policy: UiaPolicy::WatchdogOnly,
        preferred_layers: vec![
            GroundingLayer::Uia,
            GroundingLayer::MsaaHitMap,
            GroundingLayer::StaticVisualHitMap,
        ],
        ignore_background: false,
        status_bar_classes: vec![
            "msctls_statusbar32".to_string(),
            "afx:statusbar".to_string(),
            "statusbar".to_string(),
        ],
        toolbar_classes: vec![
            "afx:toolbar".to_string(),
            "toolbarwindow32".to_string(),
            "afxcontrolbar".to_string(),
            "bcgptoolbar".to_string(),
        ],
        ignore_canvas_classes: Vec::new(),
        learned_regions: Vec::new(),
        semantic_adapter: None,
        uia_timeout_ms: 120,
        learned_at_unix: 0,
        notes: "Default fallback profile".to_string(),
    };
    enforce_hard_invariants(&mut p);
    p
}

fn builtin_entry(
    exe: &str,
    display: &str,
    uia_policy: UiaPolicy,
    layers: &[GroundingLayer],
    ignore_bg: bool,
    semantic: Option<&str>,
    toolbars: &[&str],
    canvases: &[&str],
    notes: &str,
) -> AppUiProfile {
    let mut p = AppUiProfile {
        schema_version: PROFILE_SCHEMA_VERSION,
        exe_name: normalize_exe_key(exe),
        display_name: display.to_string(),
        source: ProfileSource::Builtin,
        uia_policy,
        preferred_layers: layers.to_vec(),
        ignore_background: ignore_bg,
        status_bar_classes: vec![
            "msctls_statusbar32".to_string(),
            "afx:statusbar".to_string(),
            "statusbar".to_string(),
        ],
        toolbar_classes: toolbars.iter().map(|s| s.to_ascii_lowercase()).collect(),
        ignore_canvas_classes: canvases.iter().map(|s| s.to_ascii_lowercase()).collect(),
        learned_regions: Vec::new(),
        semantic_adapter: semantic.map(|s| s.to_string()),
        uia_timeout_ms: 120,
        learned_at_unix: 0,
        notes: notes.to_string(),
    };
    enforce_hard_invariants(&mut p);
    p
}

pub fn builtin_app_profiles() -> Vec<AppUiProfile> {
    use GroundingLayer::*;
    use UiaPolicy::*;
    vec![
        builtin_entry(
            "sldworks.exe",
            "SOLIDWORKS",
            Forbidden,
            &[StaticVisualHitMap, MsaaHitMap],
            true,
            Some("solidworks"),
            &["afx:toolbar", "afxcontrolbar", "toolbarwindow32", "bcgp"],
            &["afxframeorview", "mdiclient", "opengl", "swgraphics"],
            "Zero-UIA CAD profile: Static Visual Hit-Map v2 + MSAA for CommandManager + COM L4 for 3D entities.",
        ),
        builtin_entry(
            "chrome.exe",
            "Google Chrome",
            Preferred,
            &[Uia, StaticVisualHitMap],
            false,
            Some("browser"),
            &[],
            &[],
            "Chromium accessibility tree fast path.",
        ),
        builtin_entry(
            "msedge.exe",
            "Microsoft Edge",
            Preferred,
            &[Uia, StaticVisualHitMap],
            false,
            Some("browser"),
            &[],
            &[],
            "Chromium accessibility tree fast path.",
        ),
        builtin_entry(
            "code.exe",
            "VS Code",
            Preferred,
            &[Uia, StaticVisualHitMap],
            false,
            Some("ide"),
            &[],
            &[],
            "Electron IDE accessibility fast path.",
        ),
        builtin_entry(
            "antigravity.exe",
            "Antigravity",
            Preferred,
            &[Uia, StaticVisualHitMap],
            false,
            Some("ide"),
            &[],
            &[],
            "Antigravity desktop accessibility fast path.",
        ),
        builtin_entry(
            "cursor.exe",
            "Cursor",
            Preferred,
            &[Uia, StaticVisualHitMap],
            false,
            Some("ide"),
            &[],
            &[],
            "Electron IDE accessibility fast path.",
        ),
        builtin_entry(
            "excel.exe",
            "Microsoft Excel",
            Preferred,
            &[Uia, MsaaHitMap, StaticVisualHitMap],
            false,
            Some("office"),
            &["netuihwnd"],
            &["excel7"],
            "Office Ribbon via UIA + worksheet grid via Office semantic adapter.",
        ),
        builtin_entry(
            "winword.exe",
            "Microsoft Word",
            Preferred,
            &[Uia, MsaaHitMap, StaticVisualHitMap],
            false,
            Some("office"),
            &["netuihwnd"],
            &["_wwg"],
            "Office Ribbon via UIA + document canvas via Office semantic adapter.",
        ),
        builtin_entry(
            "powerpnt.exe",
            "Microsoft PowerPoint",
            Preferred,
            &[Uia, MsaaHitMap, StaticVisualHitMap],
            false,
            Some("office"),
            &["netuihwnd"],
            &["mdiclient"],
            "Office Ribbon via UIA + slide canvas via Office semantic adapter.",
        ),
        builtin_entry(
            "wps.exe",
            "WPS Office",
            Preferred,
            &[Uia, MsaaHitMap, StaticVisualHitMap],
            false,
            Some("office"),
            &[],
            &[],
            "WPS Ribbon + canvas semantic adapter.",
        ),
        builtin_entry(
            "et.exe",
            "WPS Spreadsheets",
            Preferred,
            &[Uia, MsaaHitMap, StaticVisualHitMap],
            false,
            Some("office"),
            &[],
            &[],
            "WPS Spreadsheets semantic adapter.",
        ),
        builtin_entry(
            "explorer.exe",
            "File Explorer",
            Preferred,
            &[Uia, MsaaHitMap],
            false,
            Some("explorer"),
            &[],
            &[],
            "Windows Explorer shell + file metadata enrichment.",
        ),
        builtin_entry(
            "windowsterminal.exe",
            "Windows Terminal",
            Preferred,
            &[Uia],
            false,
            Some("terminal"),
            &[],
            &[],
            "Terminal console UIA text surface.",
        ),
        builtin_entry(
            "pwsh.exe",
            "PowerShell",
            Preferred,
            &[Uia],
            false,
            Some("terminal"),
            &[],
            &[],
            "Terminal console UIA text surface.",
        ),
        builtin_entry(
            "cmd.exe",
            "Command Prompt",
            Preferred,
            &[Uia],
            false,
            Some("terminal"),
            &[],
            &[],
            "Terminal console UIA text surface.",
        ),
        builtin_entry(
            "acad.exe",
            "AutoCAD",
            WatchdogOnly,
            &[MsaaHitMap, StaticVisualHitMap, Uia],
            true,
            None,
            &["afx:toolbar", "afxcontrolbar", "toolbarwindow32"],
            &["afxmdiframe", "afxframeorview"],
            "CAD canvas background ignored; toolbar buttons via Hit-Map.",
        ),
        builtin_entry(
            "blender.exe",
            "Blender",
            Forbidden,
            &[StaticVisualHitMap],
            false,
            None,
            &["ghost_windowclass"],
            &["ghost_windowclass"],
            "Pure OpenGL/Vulkan owner-drawn UI; static visual Hit-Map.",
        ),
        builtin_entry(
            "photoshop.exe",
            "Adobe Photoshop",
            WatchdogOnly,
            &[StaticVisualHitMap, Uia, MsaaHitMap],
            false,
            None,
            &["owl.dock", "owl.tabpane", "owl.controlbar"],
            &["psviewc"],
            "Owner-drawn creative toolbar via Hit-Map.",
        ),
        builtin_entry(
            "mstsc.exe",
            "Remote Desktop",
            Forbidden,
            &[StaticVisualHitMap],
            false,
            None,
            &["ihwindowclass", "opwindowclass"],
            &[],
            "Remote bitmap stream; pure static visual Hit-Map.",
        ),
        builtin_entry(
            "todesk.exe",
            "ToDesk",
            Forbidden,
            &[StaticVisualHitMap],
            false,
            None,
            &[],
            &[],
            "Remote desktop bitmap stream; pure static visual Hit-Map.",
        ),
        builtin_entry(
            "sunloginclient.exe",
            "Sunlogin",
            Forbidden,
            &[StaticVisualHitMap],
            false,
            None,
            &[],
            &[],
            "Remote desktop bitmap stream; pure static visual Hit-Map.",
        ),
    ]
}

pub fn profiles_dir() -> PathBuf {
    AppConfig::local_data_dir().join("ui-profiles")
}

fn profile_file_path(exe_key: &str) -> PathBuf {
    let stem = exe_key
        .strip_suffix(".exe")
        .unwrap_or(exe_key)
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    profiles_dir().join(format!("{stem}.atp-profile.json"))
}

struct ProfileStore {
    builtins: HashMap<String, AppUiProfile>,
    overrides: HashMap<String, AppUiProfile>,
    loaded_disk: bool,
}

static PROFILE_STORE: OnceLock<Mutex<ProfileStore>> = OnceLock::new();

fn store() -> &'static Mutex<ProfileStore> {
    PROFILE_STORE.get_or_init(|| {
        let mut builtins = HashMap::new();
        for p in builtin_app_profiles() {
            builtins.insert(p.exe_name.clone(), p);
        }
        Mutex::new(ProfileStore {
            builtins,
            overrides: HashMap::new(),
            loaded_disk: false,
        })
    })
}

fn ensure_disk_loaded(st: &mut ProfileStore) {
    if st.loaded_disk {
        return;
    }
    st.loaded_disk = true;
    let dir = profiles_dir();
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !name.ends_with(".atp-profile.json") {
            continue;
        }
        if let Ok(raw) = fs::read_to_string(&path) {
            if let Ok(mut prof) = serde_json::from_str::<AppUiProfile>(&raw) {
                enforce_hard_invariants(&mut prof);
                if !prof.exe_name.is_empty() {
                    st.overrides.insert(prof.exe_name.clone(), prof);
                }
            }
        }
    }
}

/// Returns `true` if the exe already has either a builtin factory profile or a learned/imported disk profile.
pub fn is_known_profile(exe_name: &str) -> bool {
    let key = normalize_exe_key(exe_name);
    if key.is_empty() {
        return true;
    }
    let Ok(mut st) = store().lock() else {
        return false;
    };
    ensure_disk_loaded(&mut st);
    st.overrides.contains_key(&key) || st.builtins.contains_key(&key)
}

/// Returns the effective profile for `exe_name` (`PluginOverlay > Imported/Learned > Builtin > DefaultUnknown`),
/// always re-enforcing hard safety invariants (e.g. `SLDWORKS.exe` zero-UIA).
pub fn get_effective_profile(exe_name: &str) -> AppUiProfile {
    let key = normalize_exe_key(exe_name);
    if key.is_empty() {
        return default_unknown_profile("");
    }
    let mut prof = if let Ok(mut st) = store().lock() {
        ensure_disk_loaded(&mut st);
        if let Some(p) = st.overrides.get(&key) {
            p.clone()
        } else if let Some(p) = st.builtins.get(&key) {
            p.clone()
        } else {
            default_unknown_profile(&key)
        }
    } else {
        default_unknown_profile(&key)
    };
    crate::pick::adapter_plugins::apply_plugin_ui_profile_overlay(&key, &mut prof);
    enforce_hard_invariants(&mut prof);
    prof
}

pub fn save_learned_profile(mut profile: AppUiProfile) -> Result<AppUiProfile> {
    enforce_hard_invariants(&mut profile);
    if profile.exe_name.is_empty() {
        return Err(anyhow!("profile exe_name cannot be empty"));
    }
    if profile.learned_at_unix == 0 {
        profile.learned_at_unix = now_unix_secs();
    }
    let dir = profiles_dir();
    fs::create_dir_all(&dir)?;
    let path = profile_file_path(&profile.exe_name);
    let json = serde_json::to_string_pretty(&profile)?;
    fs::write(&path, json)?;

    if let Ok(mut st) = store().lock() {
        ensure_disk_loaded(&mut st);
        st.overrides.insert(profile.exe_name.clone(), profile.clone());
    }
    Ok(profile)
}

pub fn reset_learned_profile(exe_name: &str) -> Result<bool> {
    let key = normalize_exe_key(exe_name);
    if key.is_empty() {
        return Ok(false);
    }
    let path = profile_file_path(&key);
    let existed = path.exists();
    if existed {
        let _ = fs::remove_file(&path);
    }
    if let Ok(mut st) = store().lock() {
        ensure_disk_loaded(&mut st);
        st.overrides.remove(&key);
    }
    Ok(existed)
}

pub fn list_all_profiles() -> Vec<AppUiProfile> {
    let Ok(mut st) = store().lock() else {
        return builtin_app_profiles();
    };
    ensure_disk_loaded(&mut st);
    let mut merged: HashMap<String, AppUiProfile> = st.builtins.clone();
    for (k, v) in &st.overrides {
        merged.insert(k.clone(), v.clone());
    }
    let mut out: Vec<AppUiProfile> = merged.into_values().collect();
    out.sort_by(|a, b| {
        let rank = |s: ProfileSource| match s {
            ProfileSource::Imported => 0,
            ProfileSource::Learned => 1,
            ProfileSource::Builtin => 2,
        };
        rank(a.source)
            .cmp(&rank(b.source))
            .then_with(|| a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase()))
    });
    out
}

pub fn export_profile_json(exe_name: &str) -> Result<String> {
    let prof = get_effective_profile(exe_name);
    Ok(serde_json::to_string_pretty(&prof)?)
}

pub fn import_profile_json(raw_json: &str) -> Result<AppUiProfile> {
    let mut prof: AppUiProfile = serde_json::from_str(raw_json)
        .map_err(|e| anyhow!("软件适配卡 JSON 格式无效: {e}"))?;
    prof.source = ProfileSource::Imported;
    prof.learned_at_unix = now_unix_secs();
    save_learned_profile(prof)
}

pub fn upsert_learned_region(exe_name: &str, rule: LearnedRegionRule) -> Result<AppUiProfile> {
    let mut prof = get_effective_profile(exe_name);
    let cls_norm = rule.class_name.to_ascii_lowercase();
    if let Some(existing) = prof
        .learned_regions
        .iter_mut()
        .find(|r| r.class_name.eq_ignore_ascii_case(&cls_norm))
    {
        if !rule.layout_kind.eq_ignore_ascii_case("auto") || existing.layout_kind.is_empty() {
            existing.layout_kind = rule.layout_kind;
        }
        existing.is_popup_menu = existing.is_popup_menu || rule.is_popup_menu;
        existing.discovered_count = existing.discovered_count.max(rule.discovered_count);
        existing.auto_wake_omniparser = existing.auto_wake_omniparser || rule.auto_wake_omniparser;
    } else {
        let mut r = rule;
        r.class_name = cls_norm;
        prof.learned_regions.push(r);
    }
    if prof.source == ProfileSource::Builtin {
        prof.source = ProfileSource::Learned;
    }
    prof.learned_at_unix = now_unix_secs();
    save_learned_profile(prof)
}

/// Scheme B lookup: returns `true` when this `(exe_name, container_class)` previously proved
/// that waking OmniParser v2 discovers significantly more controls than static segmentation.
pub fn should_auto_wake_omniparser(exe_name: &str, class_name: &str) -> bool {
    let cls_norm = class_name.trim().to_ascii_lowercase();
    if exe_name.trim().is_empty() || cls_norm.is_empty() {
        return false;
    }
    let prof = get_effective_profile(exe_name);
    prof.learned_regions
        .iter()
        .any(|r| r.auto_wake_omniparser && r.class_name.eq_ignore_ascii_case(&cls_norm))
}

/// Scheme B persistence: remembers strategy preference (`auto_wake_omniparser = true`) for this
/// `(exe_name, container_class)` — never stores pixel coordinates.
pub fn record_auto_wake_omniparser(
    exe_name: &str,
    class_name: &str,
    discovered_count: usize,
) -> Result<AppUiProfile> {
    upsert_learned_region(
        exe_name,
        LearnedRegionRule {
            class_name: class_name.to_ascii_lowercase(),
            layout_kind: "auto".to_string(),
            is_popup_menu: false,
            discovered_count,
            auto_wake_omniparser: true,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solidworks_hard_invariant_always_forbids_uia() {
        let mut p = AppUiProfile {
            schema_version: 1,
            exe_name: "SLDWORKS.EXE".to_string(),
            display_name: "SW".to_string(),
            source: ProfileSource::Imported,
            uia_policy: UiaPolicy::Preferred,
            preferred_layers: vec![GroundingLayer::Uia],
            ignore_background: false,
            status_bar_classes: vec![],
            toolbar_classes: vec![],
            ignore_canvas_classes: vec![],
            learned_regions: vec![],
            semantic_adapter: None,
            uia_timeout_ms: 5000,
            learned_at_unix: 0,
            notes: String::new(),
        };
        enforce_hard_invariants(&mut p);
        assert_eq!(p.exe_name, "sldworks.exe");
        assert_eq!(p.uia_policy, UiaPolicy::Forbidden);
        assert!(!p.preferred_layers.contains(&GroundingLayer::Uia));
        assert!(p.ignore_background);
        assert_eq!(p.semantic_adapter.as_deref(), Some("solidworks"));
        assert!(p.uia_timeout_ms <= 800);
    }

    #[test]
    fn builtin_catalog_covers_core_apps_and_normalizes_keys() {
        assert_eq!(normalize_exe_key(r"C:\Program Files\SOLIDWORKS\SLDWORKS.exe"), "sldworks.exe");
        assert_eq!(normalize_exe_key("Chrome"), "chrome.exe");

        let sw = get_effective_profile("SLDWORKS.exe");
        assert_eq!(sw.uia_policy, UiaPolicy::Forbidden);
        assert!(sw.ignore_background);

        let rdp = get_effective_profile("mstsc.exe");
        assert_eq!(rdp.uia_policy, UiaPolicy::Forbidden);
        assert_eq!(rdp.preferred_layers, vec![GroundingLayer::StaticVisualHitMap]);

        let chrome = get_effective_profile("chrome.exe");
        assert_eq!(chrome.uia_policy, UiaPolicy::Preferred);
    }

    #[test]
    fn profile_json_roundtrip_preserves_fields() {
        let orig = get_effective_profile("acad.exe");
        let json = serde_json::to_string_pretty(&orig).expect("serialize");
        let mut back: AppUiProfile = serde_json::from_str(&json).expect("deserialize");
        enforce_hard_invariants(&mut back);
        assert_eq!(back.exe_name, "acad.exe");
        assert_eq!(back.uia_policy, UiaPolicy::WatchdogOnly);
        assert!(back.ignore_background);
    }

    #[test]
    fn learned_region_auto_wake_defaults_false_and_roundtrips() {
        let legacy_json = r#"{"className":"afx:toolbar","layoutKind":"horizontal_ribbon","isPopupMenu":false,"discoveredCount":12}"#;
        let parsed: LearnedRegionRule = serde_json::from_str(legacy_json).expect("legacy parse");
        assert!(!parsed.auto_wake_omniparser);

        let awakened = LearnedRegionRule {
            class_name: "afx:toolbar".to_string(),
            layout_kind: "auto".to_string(),
            is_popup_menu: false,
            discovered_count: 72,
            auto_wake_omniparser: true,
        };
        let encoded = serde_json::to_string(&awakened).expect("encode");
        let back: LearnedRegionRule = serde_json::from_str(&encoded).expect("decode");
        assert!(back.auto_wake_omniparser);
        assert_eq!(back.discovered_count, 72);
    }

    #[test]
    fn scheme_b_auto_wake_preserves_existing_non_auto_layout_kind() {
        let mut existing = LearnedRegionRule {
            class_name: "afx:toolbar".to_string(),
            layout_kind: "horizontal_ribbon".to_string(),
            is_popup_menu: false,
            discovered_count: 30,
            auto_wake_omniparser: false,
        };
        let incoming = LearnedRegionRule {
            class_name: "afx:toolbar".to_string(),
            layout_kind: "auto".to_string(),
            is_popup_menu: false,
            discovered_count: 72,
            auto_wake_omniparser: true,
        };
        if !incoming.layout_kind.eq_ignore_ascii_case("auto") || existing.layout_kind.is_empty() {
            existing.layout_kind = incoming.layout_kind;
        }
        existing.discovered_count = existing.discovered_count.max(incoming.discovered_count);
        existing.auto_wake_omniparser = existing.auto_wake_omniparser || incoming.auto_wake_omniparser;
        assert_eq!(existing.layout_kind, "horizontal_ribbon");
        assert!(existing.auto_wake_omniparser);
        assert_eq!(existing.discovered_count, 72);
    }
}
