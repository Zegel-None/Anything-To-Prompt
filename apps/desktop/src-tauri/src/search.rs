use crate::broker::{BrokerClient, BrokerRequest};
use crate::config::AppConfig;
use crate::elevation;
use crate::everything_ipc;
use crate::harness;
use crate::ntfs_usn::{self, NtfsVolumeState};
use crate::pick::fs_meta;
use ignore::WalkBuilder;
use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config as NucleoConfig, Matcher, Utf32Str};
use pinyin::ToPinyin;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Instant, UNIX_EPOCH};

const MAX_TIER1_ENTRIES: usize = 60_000;
const MAX_TOTAL_ENTRIES: usize = 360_000;

const SKIP_DIR_NAMES: &[&str] = &[
    "node_modules",
    ".git",
    ".hg",
    ".svn",
    "target",
    "dist",
    "build",
    "out",
    "__pycache__",
    ".venv",
    "venv",
    ".next",
    ".nuxt",
    ".cache",
    ".pytest_cache",
    ".ruff_cache",
    ".mypy_cache",
    ".turbo",
    ".parcel-cache",
    ".idea",
    ".vs",
    "$recycle.bin",
    "system volume information",
    "appdata",
    "windows",
    "programdata",
    "program files",
    "program files (x86)",
    ".dsh",
    ".gemini",
    ".codex",
    ".cargo",
    ".rustup",
    "_backups",
    "_sandbox",
];

const SKIP_FILE_NAMES: &[&str] = &["desktop.ini", "thumbs.db", ".ds_store"];
const SKIP_EXTS: &[&str] = &["tmp", "bak", "log", "pdb", "obj", "o", "d", "rmeta", "rlib", "pyc"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchEntryKind {
    App,
    Dir,
    File,
}

#[derive(Debug, Clone)]
pub struct SearchEntry {
    pub kind: SearchEntryKind,
    pub name: String,
    pub name_lower: String,
    pub pinyin_full: String,
    pub pinyin_initials: String,
    pub path: String,
    pub path_lower: String,
    pub parent_display: String,
    pub ext: String,
    pub modified_secs: i64,
    pub root_priority: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub kind: SearchEntryKind,
    pub name: String,
    pub path: String,
    pub parent_display: String,
    pub ext: String,
    pub score: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchIndexStatus {
    pub ready: bool,
    pub indexing: bool,
    pub entry_count: usize,
    pub last_build_ms: u64,
    pub engine_mode: String, // "everything_ipc" | "ntfs_hybrid" | "high_value"
    pub drives: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickSearchCapsule {
    pub summary_label: String,
    pub summary_title: String,
    pub summary_detail: String,
    pub summary_body: String,
    pub text: String,
}

pub struct SearchEngine {
    entries: RwLock<Vec<SearchEntry>>,
    frecency: Mutex<HashMap<String, u32>>,
    status: Mutex<SearchIndexStatus>,
    extra_roots: Mutex<Vec<String>>,
    ntfs_volumes: Mutex<Vec<NtfsVolumeState>>,
}

impl Default for SearchEngine {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl SearchEngine {
    pub fn new(extra_roots: Vec<String>) -> Self {
        let frecency = load_frecency_map();
        Self {
            entries: RwLock::new(Vec::new()),
            frecency: Mutex::new(frecency),
            status: Mutex::new(SearchIndexStatus {
                ready: false,
                indexing: false,
                entry_count: 0,
                last_build_ms: 0,
                engine_mode: "high_value".to_string(),
                drives: Vec::new(),
            }),
            extra_roots: Mutex::new(extra_roots),
            ntfs_volumes: Mutex::new(Vec::new()),
        }
    }

    pub fn spawn_background_indexer(self: &Arc<Self>) {
        let engine = Arc::clone(self);
        let _ = std::thread::Builder::new()
            .name("atp-quick-search-indexer".to_string())
            .spawn(move || {
                // Phase 1 (<0.5s): Tier 1 high-value curated roots + Start Menu apps live immediately
                let seen_t1 = engine.rebuild_tier1_index();

                // Phase 2 (~1-2.5s): Everything IPC check OR native NTFS USN/MFT full-drive merge
                if everything_ipc::is_everything_running() {
                    if let Ok(mut st) = engine.status.lock() {
                        st.engine_mode = "everything_ipc".to_string();
                    }
                    log::info!("[SEARCH] Detected live Everything IPC; enabled zero-RAM Everything hybrid bridge");
                } else if elevation::is_elevated() {
                    engine.merge_tier2_ntfs_volumes(seen_t1);
                }

                // Phase 3: Live incremental USN journal watcher (every 2s) + Tier 1 refresh every 5 min
                let mut tick = 0u64;
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    tick += 1;

                    let has_ntfs = engine
                        .ntfs_volumes
                        .lock()
                        .map(|v| !v.is_empty())
                        .unwrap_or(false);
                    if has_ntfs {
                        engine.apply_ntfs_incremental_updates();
                    }

                    // Every 5 minutes (150 * 2s), quietly refresh Tier 1 curated roots
                    if tick.is_multiple_of(150) && !has_ntfs {
                        let _ = engine.rebuild_tier1_index();
                    }
                }
            });
    }

    pub fn trigger_refresh_async(self: &Arc<Self>) {
        let engine = Arc::clone(self);
        let _ = std::thread::Builder::new()
            .name("atp-quick-search-refresh".to_string())
            .spawn(move || {
                let seen_t1 = engine.rebuild_tier1_index();
                if everything_ipc::is_everything_running() {
                    if let Ok(mut st) = engine.status.lock() {
                        st.engine_mode = "everything_ipc".to_string();
                    }
                } else if elevation::is_elevated() {
                    engine.merge_tier2_ntfs_volumes(seen_t1);
                }
            });
    }

    pub fn status(&self) -> SearchIndexStatus {
        self.status
            .lock()
            .map(|s| s.clone())
            .unwrap_or(SearchIndexStatus {
                ready: false,
                indexing: false,
                entry_count: 0,
                last_build_ms: 0,
                engine_mode: "high_value".to_string(),
                drives: Vec::new(),
            })
    }

    pub fn rebuild_tier1_index(&self) -> HashSet<String> {
        {
            let Ok(mut st) = self.status.lock() else {
                return HashSet::new();
            };
            if st.indexing {
                return HashSet::new();
            }
            st.indexing = true;
        }

        let started = Instant::now();
        let extra = self
            .extra_roots
            .lock()
            .map(|r| r.clone())
            .unwrap_or_default();

        let mut built = Vec::with_capacity(8192);
        let mut seen_paths = HashSet::<String>::with_capacity(8192);

        // 1. Start Menu Installed Apps (.lnk / .url / .appref-ms) — Highest Priority
        for app_dir in collect_start_menu_dirs() {
            index_start_menu_apps(&app_dir, &mut built, &mut seen_paths);
        }

        // 2. Curated High-Value Roots (Desktop / Documents / Downloads / AI Workspaces / Custom Roots)
        let roots = collect_curated_roots(&extra);
        for (root_path, max_depth, priority) in roots {
            if built.len() >= MAX_TIER1_ENTRIES {
                break;
            }
            index_directory_with_ignore(
                &root_path,
                max_depth,
                priority,
                &mut built,
                &mut seen_paths,
            );
        }

        let count = built.len();
        let elapsed_ms = started.elapsed().as_millis() as u64;

        if let Ok(mut guard) = self.entries.write() {
            *guard = built;
        }
        if let Ok(mut st) = self.status.lock() {
            st.ready = true;
            st.indexing = false;
            st.entry_count = count;
            st.last_build_ms = elapsed_ms;
            if st.engine_mode.is_empty() {
                st.engine_mode = "high_value".to_string();
            }
        }
        log::info!(
            "[SEARCH] Tier 1 high-value index ready: {} entries in {}ms",
            count,
            elapsed_ms
        );
        seen_paths
    }

    fn merge_tier2_ntfs_volumes(&self, mut seen_paths: HashSet<String>) {
        let started = Instant::now();
        let drives = ntfs_usn::list_fixed_ntfs_drives();
        if drives.is_empty() {
            return;
        }

        let remaining_budget = MAX_TOTAL_ENTRIES.saturating_sub(seen_paths.len());
        let per_drive_cap = (remaining_budget / drives.len().max(1)).max(40_000);
        let mut merged_extra: Vec<SearchEntry> = Vec::with_capacity(65_536);
        let mut vol_states: Vec<NtfsVolumeState> = Vec::new();
        let mut indexed_drive_labels: Vec<String> = Vec::new();

        // Scan all NTFS volumes in parallel (e.g. C: and D: concurrently)
        type DriveScanResult = (char, Option<(Vec<ntfs_usn::NtfsItem>, NtfsVolumeState)>);
        let scanned: Vec<DriveScanResult> = std::thread::scope(|s| {
                let handles: Vec<_> = drives
                    .iter()
                    .copied()
                    .map(|drive| s.spawn(move || (drive, ntfs_usn::scan_ntfs_volume(drive, per_drive_cap))))
                    .collect();
                handles
                    .into_iter()
                    .filter_map(|h| h.join().ok())
                    .collect()
            });

        for (drive, maybe_res) in scanned {
            if seen_paths.len() + merged_extra.len() >= MAX_TOTAL_ENTRIES {
                break;
            }
            if let Some((items, vol_state)) = maybe_res {
                indexed_drive_labels.push(format!("{drive}:"));
                vol_states.push(vol_state);
                for item in items {
                    if seen_paths.len() + merged_extra.len() >= MAX_TOTAL_ENTRIES {
                        break;
                    }
                    let key = item.path.to_string_lossy().to_ascii_lowercase();
                    if !seen_paths.insert(key) {
                        continue;
                    }
                    let kind = if item.is_dir {
                        SearchEntryKind::Dir
                    } else {
                        SearchEntryKind::File
                    };
                    merged_extra.push(make_search_entry(
                        kind,
                        item.name,
                        &item.path,
                        0,
                        item.priority,
                    ));
                }
            }
        }

        if vol_states.is_empty() {
            return;
        }

        let added_count = merged_extra.len();
        let total_count = if let Ok(mut guard) = self.entries.write() {
            guard.extend(merged_extra);
            guard.len()
        } else {
            added_count
        };
        if let Ok(mut vguard) = self.ntfs_volumes.lock() {
            *vguard = vol_states;
        }
        let elapsed_ms = started.elapsed().as_millis() as u64;
        if let Ok(mut st) = self.status.lock() {
            st.entry_count = total_count;
            st.last_build_ms = elapsed_ms;
            st.engine_mode = "ntfs_hybrid".to_string();
            st.drives = indexed_drive_labels.clone();
        }
        log::info!(
            "[SEARCH] Tier 2 NTFS full-drive merged: +{} entries (total={}) across {:?} in {}ms",
            added_count,
            total_count,
            indexed_drive_labels,
            elapsed_ms
        );
    }

    fn apply_ntfs_incremental_updates(&self) {
        let mut all_added = Vec::new();
        let mut all_removed = HashSet::new();

        if let Ok(mut vols) = self.ntfs_volumes.lock() {
            for vol in vols.iter_mut() {
                let delta = ntfs_usn::poll_ntfs_volume_changes(vol);
                all_added.extend(delta.added);
                for r in delta.removed_lower {
                    all_removed.insert(r);
                }
            }
        }

        if all_added.is_empty() && all_removed.is_empty() {
            return;
        }

        let now_secs = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        if let Ok(mut guard) = self.entries.write() {
            if !all_removed.is_empty() {
                guard.retain(|e| !all_removed.contains(&e.path_lower));
            }
            if !all_added.is_empty() {
                let existing: HashSet<String> =
                    guard.iter().map(|e| e.path_lower.clone()).collect();
                for item in all_added {
                    if guard.len() >= MAX_TOTAL_ENTRIES {
                        break;
                    }
                    let key = item.path.to_string_lossy().to_ascii_lowercase();
                    if existing.contains(&key) {
                        continue;
                    }
                    let kind = if item.is_dir {
                        SearchEntryKind::Dir
                    } else {
                        SearchEntryKind::File
                    };
                    guard.push(make_search_entry(
                        kind,
                        item.name,
                        &item.path,
                        now_secs,
                        item.priority,
                    ));
                }
            }
            let total = guard.len();
            if let Ok(mut st) = self.status.lock() {
                st.entry_count = total;
            }
        }
    }

    pub fn record_use(&self, path: &str) {
        let key = path.trim().to_ascii_lowercase();
        if key.is_empty() {
            return;
        }
        if let Ok(mut map) = self.frecency.lock() {
            let entry = map.entry(key).or_insert(0);
            *entry = entry.saturating_add(1).min(500);
            save_frecency_map(&map);
        }
    }

    pub fn query(&self, raw_query: &str, limit: usize) -> Vec<SearchHit> {
        let q = raw_query.trim();
        if q.is_empty() || q.contains('\n') || q.contains('\r') || q.chars().count() > 80 {
            return Vec::new();
        }
        let max_n = limit.clamp(1, 20);
        let frec = self
            .frecency
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default();

        let mut hits = if let Ok(entries) = self.entries.read() {
            if entries.is_empty() {
                Vec::new()
            } else {
                score_entries_against_query(&entries, &frec, q, max_n)
            }
        } else {
            Vec::new()
        };

        // Optional Layer 0: if voidtools Everything is running, enrich with live Everything IPC hits
        if q.chars().count() >= 2 && hits.len() < max_n {
            if let Some(ev_hits) = everything_ipc::query_everything_ipc(q, max_n as u32) {
                let mut seen: HashSet<String> =
                    hits.iter().map(|h| h.path.to_ascii_lowercase()).collect();
                for eh in ev_hits {
                    if hits.len() >= max_n {
                        break;
                    }
                    let path_str = eh.path.to_string_lossy().to_string();
                    let lower = path_str.to_ascii_lowercase();
                    if !seen.insert(lower.clone()) {
                        continue;
                    }
                    let kind = if eh.is_dir {
                        SearchEntryKind::Dir
                    } else {
                        SearchEntryKind::File
                    };
                    let ext = if eh.is_dir {
                        String::new()
                    } else {
                        eh.path
                            .extension()
                            .and_then(|s| s.to_str())
                            .map(|s| s.to_ascii_lowercase())
                            .unwrap_or_default()
                    };
                    let freq = frec.get(&lower).copied().unwrap_or(0) as i64 * 160;
                    hits.push(SearchHit {
                        kind,
                        name: eh.name,
                        parent_display: compact_parent_display(&eh.path),
                        path: path_str,
                        ext,
                        score: 900 + freq,
                    });
                }
            }
        }

        hits
    }

    #[cfg(test)]
    pub fn from_entries_for_test(entries: Vec<SearchEntry>) -> Self {
        let count = entries.len();
        Self {
            entries: RwLock::new(entries),
            frecency: Mutex::new(HashMap::new()),
            status: Mutex::new(SearchIndexStatus {
                ready: true,
                indexing: false,
                entry_count: count,
                last_build_ms: 0,
                engine_mode: "high_value".to_string(),
                drives: Vec::new(),
            }),
            extra_roots: Mutex::new(Vec::new()),
            ntfs_volumes: Mutex::new(Vec::new()),
        }
    }
}

pub fn build_pinyin_fields(text: &str) -> (String, String) {
    if text.is_ascii() {
        return (String::new(), String::new());
    }
    let mut full = String::with_capacity(text.len() * 2);
    let mut initials = String::with_capacity(text.len());
    for ch in text.chars() {
        if let Some(py) = ch.to_pinyin() {
            let plain = py.plain();
            full.push_str(plain);
            if let Some(first) = plain.chars().next() {
                initials.push(first);
            }
        } else if ch.is_ascii_alphanumeric() {
            let lower = ch.to_ascii_lowercase();
            full.push(lower);
            initials.push(lower);
        }
    }
    (full, initials)
}

pub fn make_search_entry(
    kind: SearchEntryKind,
    display_name: String,
    full_path: &Path,
    modified_secs: i64,
    root_priority: i32,
) -> SearchEntry {
    let path_str = full_path.to_string_lossy().to_string();
    let name_lower = display_name.to_lowercase();
    let (pinyin_full, pinyin_initials) = build_pinyin_fields(&display_name);
    let parent_display = compact_parent_display(full_path);
    let ext = if kind == SearchEntryKind::File {
        full_path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default()
    } else {
        String::new()
    };

    SearchEntry {
        kind,
        name: display_name,
        name_lower,
        pinyin_full,
        pinyin_initials,
        path_lower: path_str.to_ascii_lowercase(),
        path: path_str,
        parent_display,
        ext,
        modified_secs,
        root_priority,
    }
}

fn compact_parent_display(path: &Path) -> String {
    let Some(parent) = path.parent() else {
        return String::new();
    };
    let s = parent.to_string_lossy().to_string();
    if let Some(home) = dirs::home_dir() {
        let home_s = home.to_string_lossy().to_string();
        if let Some(rest) = s.strip_prefix(&home_s) {
            if rest.is_empty() {
                return "~".to_string();
            }
            return format!("~{rest}");
        }
    }
    s
}

fn collect_start_menu_dirs() -> Vec<PathBuf> {
    let mut dirs_out = Vec::new();
    if let Some(roaming) = dirs::data_dir() {
        let user_sm = roaming
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs");
        if user_sm.is_dir() {
            dirs_out.push(user_sm);
        }
    }
    let common_sm = PathBuf::from(r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs");
    if common_sm.is_dir() {
        dirs_out.push(common_sm);
    }
    dirs_out
}

fn index_start_menu_apps(
    dir: &Path,
    out: &mut Vec<SearchEntry>,
    seen: &mut HashSet<String>,
) {
    let walker = WalkBuilder::new(dir)
        .hidden(false)
        .git_ignore(false)
        .max_depth(Some(4))
        .build();

    for dent in walker.flatten() {
        let path = dent.path();
        if !path.is_file() {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();
        if !matches!(ext.as_str(), "lnk" | "appref-ms" | "url") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .trim();
        if stem.is_empty() {
            continue;
        }
        let lower_stem = stem.to_ascii_lowercase();
        if lower_stem.contains("uninstall")
            || lower_stem.contains("卸载")
            || lower_stem.contains("readme")
            || lower_stem.contains("help")
        {
            continue;
        }
        let dedupe_key = format!("app:{}", lower_stem);
        if !seen.insert(dedupe_key) {
            continue;
        }
        let mtime = dent
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        out.push(make_search_entry(
            SearchEntryKind::App,
            stem.to_string(),
            path,
            mtime,
            240,
        ));
    }
}

fn collect_curated_roots(extra_roots: &[String]) -> Vec<(PathBuf, usize, i32)> {
    let mut raw: Vec<(PathBuf, usize, i32)> = Vec::new();

    if let Some(desktop) = dirs::desktop_dir() {
        if desktop.is_dir() {
            raw.push((desktop, 4, 150));
        }
    }
    if let Some(downloads) = dirs::download_dir() {
        if downloads.is_dir() {
            raw.push((downloads, 3, 120));
        }
    }
    if let Some(docs) = dirs::document_dir() {
        if docs.is_dir() {
            raw.push((docs, 4, 110));
        }
    }

    // Auto-discover active AI workspaces from Antigravity & Codex (read-only!)
    for g in harness::load_antigravity_catalog().groups {
        let p = PathBuf::from(&g.path);
        if !g.path.is_empty() && p.is_dir() {
            raw.push((p, 5, 130));
        }
    }
    for g in harness::load_codex_catalog().groups {
        let p = PathBuf::from(&g.path);
        if !g.path.is_empty() && p.is_dir() {
            raw.push((p, 5, 130));
        }
    }

    // Current working directory's workspace if applicable
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.is_dir() {
            raw.push((cwd, 4, 125));
        }
    }

    // User-configured custom searchRoots from config.json
    for custom in extra_roots {
        let p = PathBuf::from(custom.trim());
        if !custom.trim().is_empty() && p.is_dir() {
            raw.push((p, 5, 140));
        }
    }

    // Deduplicate roots (avoid re-walking exact same canonical/lowercased path)
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    for (p, depth, prio) in raw {
        let key = p
            .to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .to_ascii_lowercase();
        if key.len() <= 3 {
            // Never accidentally walk entire C:\ root via ignore walker
            continue;
        }
        if seen.insert(key) {
            deduped.push((p, depth, prio));
        }
    }
    deduped
}

fn should_skip_dir_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SKIP_DIR_NAMES.contains(&lower.as_str())
}

fn index_directory_with_ignore(
    root: &Path,
    max_depth: usize,
    priority: i32,
    out: &mut Vec<SearchEntry>,
    seen: &mut HashSet<String>,
) {
    let root_owned = root.to_path_buf();
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(true)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(true)
        .max_depth(Some(max_depth))
        .filter_entry(move |e| {
            if e.path() == root_owned {
                return true;
            }
            let fname = e.file_name().to_string_lossy();
            if e.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                return !should_skip_dir_name(&fname);
            }
            let lower = fname.to_ascii_lowercase();
            if SKIP_FILE_NAMES.contains(&lower.as_str()) {
                return false;
            }
            if let Some(ext) = e.path().extension().and_then(|s| s.to_str()) {
                if SKIP_EXTS.contains(&ext.to_ascii_lowercase().as_str()) {
                    return false;
                }
            }
            true
        });

    for dent in builder.build().flatten() {
        if out.len() >= MAX_TIER1_ENTRIES {
            break;
        }
        let path = dent.path();
        if path == root {
            continue;
        }
        let key = path.to_string_lossy().to_ascii_lowercase();
        if !seen.insert(key) {
            continue;
        }
        let fname = dent.file_name().to_string_lossy().trim().to_string();
        if fname.is_empty() {
            continue;
        }
        let is_dir = dent.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
        let mtime = dent
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let kind = if is_dir {
            SearchEntryKind::Dir
        } else {
            SearchEntryKind::File
        };
        out.push(make_search_entry(kind, fname, path, mtime, priority));
    }
}

pub fn score_entries_against_query(
    entries: &[SearchEntry],
    frecency: &HashMap<String, u32>,
    raw_query: &str,
    limit: usize,
) -> Vec<SearchHit> {
    let q_lower = raw_query.trim().to_lowercase();
    if q_lower.is_empty() {
        return Vec::new();
    }

    let tokens: Vec<&str> = q_lower.split_whitespace().collect();
    if tokens.is_empty() {
        return Vec::new();
    }

    let now_secs = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let single_char = q_lower.chars().count() == 1;
    let is_ascii_query = q_lower.is_ascii();
    let has_path_sep = q_lower.contains('\\') || q_lower.contains('/');

    let mut matcher = Matcher::new(NucleoConfig::DEFAULT.match_paths());
    let nucleo_pattern = Pattern::new(
        raw_query.trim(),
        CaseMatching::Ignore,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );
    let mut utf32_buf = Vec::with_capacity(128);

    let mut scored: Vec<(i64, usize)> = Vec::new();

    for (idx, entry) in entries.iter().enumerate() {
        let mut match_score: Option<i64> = None;

        if tokens.len() == 1 && !has_path_sep {
            let tok = tokens[0];
            if entry.name_lower == tok {
                match_score = Some(2600);
            } else if entry.name_lower.starts_with(tok) {
                match_score = Some(2100 - (entry.name_lower.len() as i64).min(120));
            } else if !single_char && entry.name_lower.contains(tok) {
                match_score = Some(1550 - (entry.name_lower.len() as i64).min(120));
            } else if is_ascii_query && !entry.pinyin_initials.is_empty() {
                if entry.pinyin_initials.starts_with(tok) {
                    match_score = Some(2050 - (entry.pinyin_initials.len() as i64).min(80));
                } else if !single_char && entry.pinyin_full.starts_with(tok) {
                    match_score = Some(1820 - (entry.pinyin_full.len() as i64).min(120));
                } else if !single_char && entry.pinyin_initials.contains(tok) {
                    match_score = Some(1380);
                } else if !single_char && entry.pinyin_full.contains(tok) {
                    match_score = Some(1260);
                }
            }

            // Use Helix nucleo fuzzy matcher for >= 2-char queries when literal/pinyin didn't match
            if match_score.is_none() && !single_char {
                utf32_buf.clear();
                let hay = Utf32Str::new(&entry.name, &mut utf32_buf);
                if let Some(ns) = nucleo_pattern.score(hay, &mut matcher) {
                    if ns >= 48 {
                        match_score = Some(700 + (ns as i64).min(900));
                    }
                }
            }
        } else {
            // Multi-token or path-qualified query: ALL tokens must match name, pinyin, or path
            let mut all_ok = true;
            let mut total_tok_score = 0i64;
            for tok in &tokens {
                if entry.name_lower.starts_with(tok) {
                    total_tok_score += 900;
                } else if entry.name_lower.contains(tok) {
                    total_tok_score += 650;
                } else if !entry.pinyin_initials.is_empty()
                    && (entry.pinyin_initials.contains(tok) || entry.pinyin_full.contains(tok))
                {
                    total_tok_score += 600;
                } else if entry.path_lower.contains(tok) {
                    total_tok_score += 360;
                } else {
                    all_ok = false;
                    break;
                }
            }
            if all_ok {
                match_score = Some(total_tok_score);
            }
        }

        let Some(base) = match_score else {
            continue;
        };

        let kind_boost = match entry.kind {
            SearchEntryKind::App => 240,
            SearchEntryKind::Dir => 45,
            SearchEntryKind::File => 0,
        };
        let age_secs = now_secs.saturating_sub(entry.modified_secs);
        let recency_boost = if entry.modified_secs <= 0 {
            0
        } else if age_secs < 86_400 {
            110
        } else if age_secs < 86_400 * 7 {
            55
        } else if age_secs < 86_400 * 30 {
            20
        } else {
            0
        };
        let freq_boost = frecency
            .get(&entry.path_lower)
            .map(|&c| (c as i64).min(50) * 160)
            .unwrap_or(0);

        let hidden_dir_penalty = if !q_lower.starts_with('.') && entry.path_lower.contains("\\.") {
            -180
        } else {
            0
        };

        let total = base
            + kind_boost
            + recency_boost
            + freq_boost
            + entry.root_priority as i64
            + hidden_dir_penalty;
        scored.push((total, idx));
    }

    scored.sort_by_key(|&(score, idx)| (std::cmp::Reverse(score), entries[idx].name.len()));
    scored
        .into_iter()
        .take(limit)
        .map(|(score, idx)| {
            let e = &entries[idx];
            SearchHit {
                kind: e.kind,
                name: e.name.clone(),
                path: e.path.clone(),
                parent_display: e.parent_display.clone(),
                ext: e.ext.clone(),
                score,
            }
        })
        .collect()
}

pub fn build_quick_search_capsule(
    pick_num: usize,
    path_str: &str,
    display_name: &str,
    kind: SearchEntryKind,
) -> QuickSearchCapsule {
    let badge = crate::pick::circled_badge(pick_num);
    let path = Path::new(path_str);
    let name = if display_name.trim().is_empty() {
        path.file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| path_str.to_string())
    } else {
        display_name.trim().to_string()
    };

    let kind_tag = match kind {
        SearchEntryKind::App => "app",
        SearchEntryKind::Dir => "folder",
        SearchEntryKind::File => "file",
    };

    let mut lines = vec![
        format!("[DSB-PICK #{pick_num} | kind=quick_search_{kind_tag}]"),
        format!("path: {path_str}"),
        format!("name: {name}"),
    ];

    if let Some(brief) = fs_meta::inspect_path_metadata(path, 150) {
        if brief.is_dir {
            lines.push(format!(
                "folder_summary: files={} dirs={} size_bytes={}",
                brief.file_count, brief.dir_count, brief.size_bytes
            ));
            if !brief.children_preview.is_empty() {
                lines.push(format!(
                    "children_preview: {}",
                    brief.children_preview.join(", ")
                ));
            }
        } else {
            lines.push(format!("size_bytes: {}", brief.size_bytes));
        }
        if let Some(branch) = brief.git_branch {
            lines.push(format!("git_branch: {branch}"));
        }
    }

    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    if matches!(ext.as_str(), "sldprt" | "sldasm" | "slddrw") {
        lines.push(format!(
            "[SW-TARGET] doc=\"{path_str}\" kind=DOCUMENT | 可调用 Zegel-SWE MCP [solidworks] 打开或读取特征树"
        ));
    } else if matches!(
        ext.as_str(),
        "md" | "txt" | "json" | "yaml" | "yml" | "toml" | "rs" | "py" | "ts" | "tsx" | "js" | "jsx" | "ps1" | "csv" | "ini" | "cfg"
    ) {
        if let Ok(meta) = fs::metadata(path) {
            if meta.len() <= 64 * 1024 {
                if let Ok(content) = fs::read_to_string(path) {
                    let preview: Vec<&str> = content.lines().take(16).collect();
                    if !preview.is_empty() {
                        let snippet: String = preview.join("\n").chars().take(600).collect();
                        lines.push(format!("file_preview:\n{snippet}"));
                    }
                }
            }
        }
    }

    lines.push("[/DSB-PICK]".to_string());

    let parent_short = compact_parent_display(path);
    QuickSearchCapsule {
        summary_label: badge.clone(),
        summary_title: format!("{badge} {name}"),
        summary_detail: if parent_short.is_empty() {
            kind_tag.to_string()
        } else {
            parent_short
        },
        summary_body: path_str.to_string(),
        text: lines.join("\n"),
    }
}

/// Launches a matched file, folder, or shortcut strictly at normal (medium) integrity via Broker.
pub fn open_search_path_medium(broker: &BrokerClient, path_str: &str) -> Result<(), String> {
    let trimmed = path_str.trim();
    if trimmed.is_empty() {
        return Err("路径为空".to_string());
    }
    let path = Path::new(trimmed);
    let resp = if path.is_dir() {
        broker.call(BrokerRequest::Spawn {
            exe: "explorer.exe".to_string(),
            args: vec![trimmed.to_string()],
            cwd: None,
            hidden: false,
        })
    } else {
        broker.call(BrokerRequest::Spawn {
            exe: "cmd.exe".to_string(),
            args: vec![
                "/C".to_string(),
                "start".to_string(),
                "".to_string(),
                trimmed.to_string(),
            ],
            cwd: path.parent().map(|p| p.to_string_lossy().to_string()),
            hidden: true,
        })
    };
    if resp.ok {
        Ok(())
    } else {
        Err(resp.error.unwrap_or_else(|| "打开路径失败".to_string()))
    }
}

fn frecency_path() -> PathBuf {
    AppConfig::config_dir().join("search_frecency.json")
}

fn load_frecency_map() -> HashMap<String, u32> {
    let path = frecency_path();
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<HashMap<String, u32>>(&s).ok())
        .unwrap_or_default()
}

fn save_frecency_map(map: &HashMap<String, u32>) {
    let path = frecency_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(raw) = serde_json::to_string_pretty(map) {
        let _ = fs::write(path, raw);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_pinyin_full_and_initials_for_chinese_filenames() {
        let (full, initials) = build_pinyin_fields("支架底座.SLDPRT");
        assert!(full.starts_with("zhijiadizuo"));
        assert!(initials.starts_with("zjdz"));

        let (ascii_full, ascii_init) = build_pinyin_fields("config.json");
        assert!(ascii_full.is_empty());
        assert!(ascii_init.is_empty());
    }

    #[test]
    fn matches_from_first_char_supports_pinyin_and_returns_empty_on_ai_sentence() {
        let entries = vec![
            make_search_entry(
                SearchEntryKind::App,
                "微信".to_string(),
                Path::new(r"C:\ProgramData\Start Menu\微信.lnk"),
                0,
                240,
            ),
            make_search_entry(
                SearchEntryKind::File,
                "支架.SLDPRT".to_string(),
                Path::new(r"D:\CAD\支架.SLDPRT"),
                0,
                150,
            ),
            make_search_entry(
                SearchEntryKind::File,
                "报销发票.xlsx".to_string(),
                Path::new(r"C:\Users\Zegel\Desktop\报销发票.xlsx"),
                0,
                150,
            ),
        ];
        let engine = SearchEngine::from_entries_for_test(entries);

        // Single-character prefix match
        let h1 = engine.query("支", 8);
        assert_eq!(h1.len(), 1);
        assert_eq!(h1[0].name, "支架.SLDPRT");

        // Chinese Pinyin initials match (`wx` -> 微信, `bxfp` -> 报销发票.xlsx)
        let h_wx = engine.query("wx", 8);
        assert!(!h_wx.is_empty());
        assert_eq!(h_wx[0].name, "微信");

        let h_fp = engine.query("bxfp", 8);
        assert!(!h_fp.is_empty());
        assert_eq!(h_fp[0].name, "报销发票.xlsx");

        // Normal AI sentence has 0 file matches -> returns empty so Listary bar hides!
        let h_ai = engine.query("帮我写一段关于流体力学的总结", 8);
        assert!(h_ai.is_empty());
    }

    #[test]
    fn builds_solidworks_and_general_blue_capsule_payloads() {
        let cap = build_quick_search_capsule(
            1,
            r"D:\CAD\支架.SLDPRT",
            "支架.SLDPRT",
            SearchEntryKind::File,
        );
        assert_eq!(cap.summary_label, "①");
        assert_eq!(cap.summary_title, "① 支架.SLDPRT");
        assert!(cap.text.contains("[DSB-PICK #1 | kind=quick_search_file]"));
        assert!(cap.text.contains("[SW-TARGET]"));
    }

    #[test]
    #[ignore = "live host smoke & latency benchmark"]
    fn live_full_stack_smoke_and_benchmark() {
        // 1. Harness availability & real read-only catalog sync
        let avail = crate::harness::list_harness_availability(true);
        assert_eq!(avail.len(), 3);
        let ag = avail.iter().find(|a| a.id == "antigravity").unwrap();
        let cx = avail.iter().find(|a| a.id == "codex").unwrap();
        eprintln!(
            "[LIVE] Antigravity avail={} exe={:?} model={} access={}",
            ag.available, ag.exe_path, ag.model_label, ag.access_label
        );
        eprintln!(
            "[LIVE] Codex avail={} exe={:?} model={} effort={} access={}",
            cx.available, cx.exe_path, cx.model_label, cx.effort_label, cx.access_label
        );
        assert!(ag.available, "agy.exe should be detected");
        assert!(cx.available, "codex.exe should be detected");

        let ag_cat = crate::harness::load_antigravity_catalog();
        let cx_cat = crate::harness::load_codex_catalog();
        eprintln!(
            "[LIVE] Antigravity catalog groups={} err={:?}",
            ag_cat.groups.len(),
            ag_cat.error
        );
        eprintln!(
            "[LIVE] Codex catalog groups={} err={:?}",
            cx_cat.groups.len(),
            cx_cat.error
        );
        assert!(ag_cat.error.is_none());
        assert!(cx_cat.error.is_none());
        assert!(!ag_cat.groups.is_empty());
        assert!(!cx_cat.groups.is_empty());

        // 2. SolidWorks COM non-crash smoke check
        let broker = crate::broker::BrokerClient::new();
        let sw_t0 = Instant::now();
        let sw_res = crate::pick::adapters::solidworks::pick_solidworks_feature_com(
            &broker, 500, 500, None,
        );
        eprintln!(
            "[LIVE] SolidWorks COM smoke returned is_some={} in {}ms",
            sw_res.is_some(),
            sw_t0.elapsed().as_millis()
        );

        // 3. Full Tier 1 + Tier 2 NTFS build & query latency benchmark
        let engine = SearchEngine::new(Vec::new());
        let seen = engine.rebuild_tier1_index();
        engine.merge_tier2_ntfs_volumes(seen);
        let st = engine.status();
        eprintln!(
            "[LIVE] Hybrid engine ready: mode={} entries={} drives={:?}",
            st.engine_mode, st.entry_count, st.drives
        );
        assert!(st.entry_count > 50_000);

        for q in [
            "wx",
            "anything-to-prompt",
            "sldprt",
            "帮我写一段关于流体力学的详细技术总结",
        ] {
            let t0 = Instant::now();
            let hits = engine.query(q, 8);
            let us = t0.elapsed().as_micros();
            eprintln!(
                "[LIVE] query={:?} -> {} hits in {:.2}ms (top={:?})",
                q,
                hits.len(),
                us as f64 / 1000.0,
                hits.first().map(|h| (&h.name, &h.parent_display))
            );
            assert!(
                us < 80_000,
                "Query {q:?} took too long: {:.2}ms",
                us as f64 / 1000.0
            );
        }
        assert!(
            engine
                .query("帮我写一段关于流体力学的详细技术总结", 8)
                .is_empty(),
            "AI sentence must yield 0 matches"
        );
    }
}
