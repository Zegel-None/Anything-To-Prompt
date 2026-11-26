use crate::broker::{BrokerClient, BrokerRequest};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use windows::core::{BOOL, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    CreateProcessW, OpenProcess, QueryFullProcessImageNameW, CREATE_BREAKAWAY_FROM_JOB,
    CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, DETACHED_PROCESS, PROCESS_INFORMATION,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, STARTF_USESHOWWINDOW, STARTUPINFOW,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId,
    IsIconic, IsWindowVisible, SetForegroundWindow, SetWindowPos, ShowWindow, HWND_NOTOPMOST,
    HWND_TOPMOST, SWP_NOMOVE, SWP_NOSIZE, SW_RESTORE, SW_SHOW, SW_SHOWMINNOACTIVE,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HarnessKind {
    #[default]
    Dsh,
    Antigravity,
    Codex,
    ClaudeCode,
}

impl HarnessKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            HarnessKind::Dsh => "dsh",
            HarnessKind::Antigravity => "antigravity",
            HarnessKind::Codex => "codex",
            HarnessKind::ClaudeCode => "claude_code",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "antigravity" | "agy" => HarnessKind::Antigravity,
            "codex" => HarnessKind::Codex,
            "claude_code" | "claude-code" | "claudecode" | "claude" => HarnessKind::ClaudeCode,
            _ => HarnessKind::Dsh,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessAvailability {
    pub id: String,
    pub label: String,
    pub subtitle: String,
    pub available: bool,
    pub running: bool,
    pub exe_path: Option<String>,
    pub sync_note: String,
    pub model_label: String,
    pub effort_label: String,
    pub access_label: String,
    #[serde(default)]
    pub provider_kind: String, // "native" | "acp" | "universal"
    #[serde(default)]
    pub accent: String,
    #[serde(default)]
    pub badge: String,
    #[serde(default)]
    pub custom: bool,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalConversation {
    pub id: String,
    pub title: String,
    pub updated_at: i64,
    pub source: String,
    pub cwd: String,
    pub rollout_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalWorkspaceGroup {
    pub workspace_id: String,
    pub label: String,
    pub path: String,
    pub convs: Vec<ExternalConversation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalHarnessCatalog {
    pub harness: String,
    pub available: bool,
    pub exe_path: Option<String>,
    pub groups: Vec<ExternalWorkspaceGroup>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalChatMessage {
    pub id: String,
    pub role: String,
    pub text: String,
    pub streaming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnAttachment {
    pub kind: String, // "image" | "file" | "folder"
    pub path: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessOptionItem {
    pub id: String,
    pub label: String,
    pub desc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessComposerOptions {
    pub harness: String,
    pub models: Vec<HarnessOptionItem>,
    pub efforts: Vec<HarnessOptionItem>,
    pub modes: Vec<HarnessOptionItem>,
    pub plugins: Vec<HarnessOptionItem>,
    pub actions: Vec<HarnessOptionItem>,
    pub default_model: String,
    pub default_effort: String,
    pub default_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessStreamEvent {
    pub turn_id: String,
    pub harness: String,
    pub kind: String, // "session" | "delta" | "replace" | "tool" | "completed" | "error"
    pub session_id: Option<String>,
    pub text: Option<String>,
    pub tool_name: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessSessionStats {
    pub harness: String,
    pub session_id: String,
    pub model: String,
    pub turns: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub total_tokens: u64,
    pub summary_markdown: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessSessionState {
    pub harness: String,
    pub session_id: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub mode: Option<String>,
    pub plan_mode: Option<bool>,
    pub read_only_in_desktop: bool,
    pub source_note: String,
}

static ACTIVE_TURN_PIDS: OnceLock<Mutex<HashMap<String, u32>>> = OnceLock::new();
static ACTIVE_SIDECAR_CANCELS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();

fn active_turn_pids() -> &'static Mutex<HashMap<String, u32>> {
    ACTIVE_TURN_PIDS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn active_sidecar_cancels() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    ACTIVE_SIDECAR_CANCELS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn register_turn_pid(turn_id: &str, pid: u32) {
    if let Ok(mut map) = active_turn_pids().lock() {
        map.insert(turn_id.to_string(), pid);
    }
}

pub fn unregister_turn_pid(turn_id: &str) {
    if let Ok(mut map) = active_turn_pids().lock() {
        map.remove(turn_id);
    }
    if let Ok(mut cmap) = active_sidecar_cancels().lock() {
        cmap.remove(turn_id);
    }
}

fn register_sidecar_cancel(turn_id: &str) -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    if let Ok(mut cmap) = active_sidecar_cancels().lock() {
        cmap.insert(turn_id.to_string(), flag.clone());
    }
    flag
}

pub fn kill_turn_pid_in_medium(turn_id: &str) -> bool {
    let mut cancelled = false;
    if let Ok(mut cmap) = active_sidecar_cancels().lock() {
        if let Some(flag) = cmap.remove(turn_id) {
            flag.store(true, Ordering::Release);
            cancelled = true;
        }
    }
    let pid = active_turn_pids()
        .lock()
        .ok()
        .and_then(|mut m| m.remove(turn_id));
    if let Some(pid) = pid {
        let _ = Command::new("taskkill.exe")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW.0)
            .status();
        cancelled = true;
    }
    cancelled
}

pub fn resolve_agy_exe() -> Option<PathBuf> {
    if let Some(local_app) = dirs::data_local_dir() {
        let candidate = local_app.join("agy").join("bin").join("agy.exe");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    if let Some(home) = dirs::home_dir() {
        let candidate = home.join(".local").join("bin").join("agy.exe");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    find_in_path("agy.exe")
}

pub fn resolve_codex_exe() -> Option<PathBuf> {
    // 1. Official Codex Desktop MSIX bundled codex.exe under Program Files\WindowsApps
    if let Ok(entries) = fs::read_dir(r"C:\Program Files\WindowsApps") {
        let mut matches: Vec<PathBuf> = entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with("OpenAI.Codex_") {
                    let exe = e.path().join("app").join("resources").join("codex.exe");
                    if exe.is_file() {
                        return Some(exe);
                    }
                }
                None
            })
            .collect();
        matches.sort();
        if let Some(latest) = matches.pop() {
            return Some(latest);
        }
    }
    // 2. npm global or PATH fallback
    if let Some(appdata) = dirs::data_dir() {
        let npm_exe = appdata.join("npm").join("codex.cmd");
        if npm_exe.is_file() {
            return Some(npm_exe);
        }
    }
    find_in_path("codex.exe").or_else(|| find_in_path("codex.cmd"))
}

pub fn resolve_claude_exe() -> Option<PathBuf> {
    if let Some(home) = dirs::home_dir() {
        for candidate in [
            home.join(".local").join("bin").join("claude.exe"),
            home.join(".claude").join("local").join("claude.exe"),
        ] {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    if let Some(appdata) = dirs::data_dir() {
        let npm_exe = appdata.join("npm").join("claude.cmd");
        if npm_exe.is_file() {
            return Some(npm_exe);
        }
    }
    find_in_path("claude.exe").or_else(|| find_in_path("claude.cmd"))
}

fn format_claude_model_label(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "Claude Opus".to_string();
    }
    let clean = trimmed.split('[').next().unwrap_or(trimmed).trim();
    let lower = clean.to_ascii_lowercase();
    if lower == "opus" {
        return "Claude Opus".to_string();
    }
    if lower == "sonnet" {
        return "Claude Sonnet".to_string();
    }
    if lower == "haiku" {
        return "Claude Haiku".to_string();
    }
    let parts: Vec<&str> = clean.split('-').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return "Claude".to_string();
    }
    let mut out: Vec<String> = Vec::new();
    for (idx, part) in parts.iter().enumerate() {
        // Drop 8-digit release date suffixes like 20250514
        if part.len() == 8 && part.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if idx == 0 && part.eq_ignore_ascii_case("claude") {
            out.push("Claude".to_string());
            continue;
        }
        if part.chars().all(|c| c.is_ascii_digit() || c == '.') {
            if let Some(last) = out.last_mut() {
                if last.chars().all(|c| c.is_ascii_digit() || c == '.') {
                    last.push('.');
                    last.push_str(part);
                    continue;
                }
            }
            out.push((*part).to_string());
        } else {
            let mut chars = part.chars();
            if let Some(first) = chars.next() {
                out.push(format!("{}{}", first.to_ascii_uppercase(), chars.as_str()));
            }
        }
    }
    let joined = out.join(" ");
    if joined.is_empty() {
        "Claude".to_string()
    } else {
        joined
    }
}

fn read_claude_code_config_display() -> (String, String, String) {
    let mut model_raw = String::new();
    let mut effort_raw = String::new();
    let mut perm_raw = String::new();

    if let Some(home) = dirs::home_dir() {
        let claude_dir = home.join(".claude");
        for file_name in ["settings.local.json", "settings.json"] {
            let path = claude_dir.join(file_name);
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            if model_raw.is_empty() {
                if let Some(m) = v.get("model").and_then(|s| s.as_str()) {
                    model_raw = m.trim().to_string();
                }
            }
            if effort_raw.is_empty() {
                if let Some(e) = v
                    .get("effortLevel")
                    .or_else(|| v.get("effort"))
                    .and_then(|s| s.as_str())
                {
                    effort_raw = e.trim().to_string();
                }
            }
            if perm_raw.is_empty() {
                if let Some(p) = v
                    .get("permissions")
                    .and_then(|p| p.get("defaultMode"))
                    .and_then(|s| s.as_str())
                {
                    perm_raw = p.trim().to_string();
                }
            }
        }
    }

    let model_label = format_claude_model_label(&model_raw);
    let effort_label = match effort_raw.to_ascii_lowercase().as_str() {
        "max" => "Max".to_string(),
        "xhigh" => "极高".to_string(),
        "high" => "高".to_string(),
        "medium" => "中".to_string(),
        "low" => "低".to_string(),
        other if !other.is_empty() => other.to_string(),
        _ => "高".to_string(),
    };
    let access_label = match perm_raw.as_str() {
        "acceptEdits" => "自动编辑".to_string(),
        "plan" => "仅规划".to_string(),
        "dontAsk" => "免询问".to_string(),
        "bypassPermissions" => "完全访问".to_string(),
        "manual" => "手动确认".to_string(),
        _ => "自动".to_string(),
    };

    (model_label, effort_label, access_label)
}

pub(crate) fn find_in_path(binary: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(binary);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn read_codex_config_display() -> (String, String, String) {
    let mut model_raw = String::new();
    let mut effort_raw = String::new();
    let mut sandbox_raw = String::new();

    if let Some(home) = dirs::home_dir() {
        let cfg = home.join(".codex").join("config.toml");
        if let Ok(text) = fs::read_to_string(cfg) {
            for line in text.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with('[') {
                    // Only inspect top-level keys before section tables
                    break;
                }
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                let Some((k, v)) = trimmed.split_once('=') else {
                    continue;
                };
                let key = k.trim();
                let val = v.trim().trim_matches('"').trim_matches('\'').to_string();
                match key {
                    "model" if model_raw.is_empty() => model_raw = val,
                    "model_reasoning_effort" if effort_raw.is_empty() => effort_raw = val,
                    "sandbox_mode" if sandbox_raw.is_empty() => sandbox_raw = val,
                    _ => {}
                }
            }
        }
    }

    let model_label = if model_raw.is_empty() {
        "Codex".to_string()
    } else {
        let parts: Vec<String> = model_raw
            .split('-')
            .enumerate()
            .map(|(i, p)| {
                if i == 0 && p.eq_ignore_ascii_case("gpt") {
                    "GPT".to_string()
                } else if p.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                    p.to_string()
                } else {
                    let mut chars = p.chars();
                    match chars.next() {
                        Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                        None => String::new(),
                    }
                }
            })
            .collect();
        if parts.len() >= 2 && parts[0] == "GPT" {
            format!("{}-{} {}", parts[0], parts[1], parts[2..].join(" "))
                .trim()
                .to_string()
        } else {
            parts.join(" ")
        }
    };

    let effort_label = match effort_raw.to_ascii_lowercase().as_str() {
        "ultra" | "xhigh" => "极高".to_string(),
        "high" => "高".to_string(),
        "medium" => "中".to_string(),
        "low" => "低".to_string(),
        other if !other.is_empty() => other.to_string(),
        _ => "高".to_string(),
    };

    let access_label = match sandbox_raw.to_ascii_lowercase().as_str() {
        "danger-full-access" => "完全访问".to_string(),
        "workspace-write" => "工作区读写".to_string(),
        "read-only" => "只读模式".to_string(),
        _ => "完全访问".to_string(),
    };

    (model_label, effort_label, access_label)
}

fn to_wide_null(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

pub fn collect_pids_for_exe(exe_lower: &str) -> HashSet<u32> {
    let mut pids = HashSet::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return pids;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snap, &mut entry).is_ok() {
            loop {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..len]).to_ascii_lowercase();
                if name == exe_lower {
                    pids.insert(entry.th32ProcessID);
                }
                if Process32NextW(snap, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    pids
}

pub fn is_process_name_running(exe_lower: &str) -> bool {
    !collect_pids_for_exe(exe_lower).is_empty()
}

pub fn resolve_antigravity_desktop_exe() -> Option<PathBuf> {
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let p = PathBuf::from(local)
            .join("Programs")
            .join("antigravity")
            .join("Antigravity.exe");
        if p.exists() {
            return Some(p);
        }
    }
    if let Ok(pf) = std::env::var("ProgramFiles") {
        let p = PathBuf::from(pf).join("Antigravity").join("Antigravity.exe");
        if p.exists() {
            return Some(p);
        }
    }
    None
}

fn query_exe_path_for_pid(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = vec![0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(handle);
        if ok && len > 0 {
            Some(String::from_utf16_lossy(&buf[..len as usize]))
        } else {
            None
        }
    }
}

fn window_title_text(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if len > 0 {
        String::from_utf16_lossy(&buf[..len as usize])
    } else {
        String::new()
    }
}

fn find_main_window_for_pids(pids: HashSet<u32>, required_title_lower: Option<&str>) -> Option<HWND> {
    if pids.is_empty() {
        return None;
    }
    struct SearchCtx {
        pids: HashSet<u32>,
        required_title_lower: Option<String>,
        best_hwnd: Option<HWND>,
        best_score: i64,
    }
    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = &mut *(lparam.0 as *mut SearchCtx);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if !ctx.pids.contains(&pid) {
            return BOOL(1);
        }
        let iconic = IsIconic(hwnd).as_bool();
        let visible = IsWindowVisible(hwnd).as_bool();
        if !visible && !iconic {
            return BOOL(1);
        }
        if let Some(ref req) = ctx.required_title_lower {
            let title = window_title_text(hwnd).trim().to_ascii_lowercase();
            if title != *req && !title.starts_with(req) {
                return BOOL(1);
            }
        }
        let mut rc = RECT::default();
        let _ = GetWindowRect(hwnd, &mut rc);
        let w = (rc.right - rc.left) as i64;
        let h = (rc.bottom - rc.top) as i64;
        let score = if iconic {
            500_000
        } else if w >= 240 && h >= 160 {
            w * h
        } else {
            return BOOL(1);
        };
        if score > ctx.best_score {
            ctx.best_score = score;
            ctx.best_hwnd = Some(hwnd);
        }
        BOOL(1)
    }
    let mut ctx = SearchCtx {
        pids,
        required_title_lower: required_title_lower.map(|s| s.to_ascii_lowercase()),
        best_hwnd: None,
        best_score: 0,
    };
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut ctx as *mut SearchCtx as isize));
    }
    ctx.best_hwnd
}

fn find_main_window_for_exe(exe_lower: &str) -> Option<HWND> {
    find_main_window_for_pids(collect_pids_for_exe(exe_lower), None)
}

/// Finds the real Codex Desktop GUI window (`WindowsApps\OpenAI.Codex_*\app\ChatGPT.exe`,
/// or `codex.exe` if a future build renames it), distinguishing it from standalone ChatGPT Desktop.
fn find_codex_desktop_window() -> Option<HWND> {
    let mut codex_pids = collect_pids_for_exe("codex.exe");
    let mut fallback_chatgpt_pids = HashSet::new();
    for pid in collect_pids_for_exe("chatgpt.exe") {
        match query_exe_path_for_pid(pid) {
            Some(path) if path.to_ascii_lowercase().contains("openai.codex") => {
                codex_pids.insert(pid);
            }
            None => {
                fallback_chatgpt_pids.insert(pid);
            }
            _ => {}
        }
    }
    find_main_window_for_pids(codex_pids, None)
        .or_else(|| find_main_window_for_pids(fallback_chatgpt_pids, Some("codex")))
}

fn find_harness_desktop_window(kind: HarnessKind) -> Option<HWND> {
    match kind {
        HarnessKind::Antigravity => find_main_window_for_exe("antigravity.exe"),
        HarnessKind::Codex => find_codex_desktop_window(),
        HarnessKind::ClaudeCode => find_main_window_for_exe("claude.exe"),
        HarnessKind::Dsh => None,
    }
}

/// Launches a desktop Harness GUI silently in the background (minimized without activating),
/// completely decoupled from the caller's Job Object.
fn launch_desktop_app_silently(exe: &Path, exe_lower_for_guard: &'static str) -> bool {
    let exe_str = exe.to_string_lossy().to_string();
    let exe_w = to_wide_null(&exe_str);
    let cmdline = format!("\"{exe_str}\"");
    let si = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        dwFlags: STARTF_USESHOWWINDOW,
        wShowWindow: SW_SHOWMINNOACTIVE.0 as u16,
        ..Default::default()
    };
    let mut cmd_w = to_wide_null(&cmdline);
    let mut pi = PROCESS_INFORMATION::default();
    let launched = unsafe {
        CreateProcessW(
            PCWSTR(exe_w.as_ptr()),
            Some(PWSTR(cmd_w.as_mut_ptr())),
            None,
            None,
            false,
            CREATE_BREAKAWAY_FROM_JOB | DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP,
            None,
            PCWSTR::null(),
            &si,
            &mut pi,
        )
        .is_ok()
    } || {
        let mut cmd_w2 = to_wide_null(&cmdline);
        unsafe {
            CreateProcessW(
                PCWSTR(exe_w.as_ptr()),
                Some(PWSTR(cmd_w2.as_mut_ptr())),
                None,
                None,
                false,
                DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP,
                None,
                PCWSTR::null(),
                &si,
                &mut pi,
            )
            .is_ok()
        }
    };
    if launched {
        unsafe {
            let _ = CloseHandle(pi.hThread);
            let _ = CloseHandle(pi.hProcess);
        }
        // Guard for 4.5s in case Electron ignores STARTF_USESHOWWINDOW and tries to pop up on first paint
        std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_millis(4500);
            let mut minimized_once = false;
            while Instant::now() < deadline {
                if let Some(hwnd) = find_main_window_for_exe(exe_lower_for_guard) {
                    unsafe {
                        if !IsIconic(hwnd).as_bool() {
                            let _ = ShowWindow(hwnd, SW_SHOWMINNOACTIVE);
                            minimized_once = true;
                        }
                    }
                    if minimized_once {
                        std::thread::sleep(Duration::from_millis(350));
                        // Re-check once more in case splash transitioned to main window
                        if let Some(hwnd2) = find_main_window_for_exe(exe_lower_for_guard) {
                            unsafe {
                                if !IsIconic(hwnd2).as_bool() {
                                    let _ = ShowWindow(hwnd2, SW_SHOWMINNOACTIVE);
                                }
                            }
                        }
                        break;
                    }
                }
                std::thread::sleep(Duration::from_millis(140));
            }
        });
    }
    launched
}

fn title_matches_target(candidate_raw: &str, target_raw: &str) -> u8 {
    let cand = candidate_raw.trim();
    let target = target_raw.trim();
    if cand.is_empty() || target.is_empty() {
        return 0;
    }
    fn strip_dots(s: &str) -> &str {
        s.trim_end_matches(['…', '.', ' ']).trim()
    }
    let c_clean = strip_dots(cand);
    let t_clean = strip_dots(target);
    if c_clean.is_empty() || t_clean.is_empty() {
        return 0;
    }
    if c_clean.eq_ignore_ascii_case(t_clean) {
        return 3;
    }
    let c_low = c_clean.to_lowercase();
    let t_low = t_clean.to_lowercase();
    let c_chars = c_low.chars().count();
    let t_chars = t_low.chars().count();
    if c_chars >= 4 && t_chars >= 4 && (t_low.starts_with(&c_low) || c_low.starts_with(&t_low)) {
        return 2;
    }
    if t_chars >= 5 && c_low.contains(&t_low) {
        return 1;
    }
    0
}

fn select_conversation_in_window_uia(hwnd: HWND, target_title: &str) -> bool {
    let target = target_title.trim();
    if target.is_empty() {
        return false;
    }
    let Ok(auto) = uiautomation::UIAutomation::new() else {
        return false;
    };
    let Ok(walker) = auto.get_control_view_walker() else {
        return false;
    };
    let Ok(root) = auto.element_from_handle(uiautomation::types::Handle::from(hwnd.0 as isize))
    else {
        return false;
    };

    let mut win_rc = RECT::default();
    unsafe {
        let _ = GetWindowRect(hwnd, &mut win_rc);
    }
    let sidebar_right = win_rc.left + 500;
    let titlebar_bottom = win_rc.top + 52;

    // Two-tier queue: scan left-sidebar subtrees FIRST so huge right-pane chat transcripts
    // never exhaust the UIA traversal budget before reaching the conversation list.
    let mut primary_queue: std::collections::VecDeque<(uiautomation::UIElement, usize)> =
        std::collections::VecDeque::new();
    let mut secondary_queue: std::collections::VecDeque<(uiautomation::UIElement, usize)> =
        std::collections::VecDeque::new();
    primary_queue.push_back((root, 0));
    let mut visited = 0usize;
    let mut best: Option<(u8, bool, String, uiautomation::UIElement)> = None;

    while let Some((el, depth)) = primary_queue.pop_front().or_else(|| {
        // Only fall back to right-pane elements if sidebar didn't find anything
        if best.as_ref().map(|(_, s, _, _)| *s).unwrap_or(false) {
            None
        } else {
            secondary_queue.pop_front()
        }
    }) {
        visited += 1;
        if visited > 5500 {
            break;
        }
        let el_rect = el.get_bounding_rectangle().ok();
        if let Ok(name) = el.get_name() {
            let score = title_matches_target(&name, target);
            if score > 0 {
                let rect_info = el_rect.as_ref().map(|r| {
                    let w = r.get_right() - r.get_left();
                    let h = r.get_bottom() - r.get_top();
                    let below_titlebar = r.get_top() > titlebar_bottom;
                    let valid_box = w > 12 && h > 10 && h < 120;
                    let in_sidebar = valid_box && below_titlebar && r.get_left() <= sidebar_right;
                    (valid_box && below_titlebar, in_sidebar)
                });
                if let Some((true, in_sidebar)) = rect_info {
                    let replace = match &best {
                        None => true,
                        Some((bs, b_side, _, _)) => {
                            (in_sidebar && !*b_side) || (in_sidebar == *b_side && score > *bs)
                        }
                    };
                    if replace {
                        best = Some((score, in_sidebar, name, el.clone()));
                        if score == 3 && in_sidebar {
                            break;
                        }
                    }
                }
            }
        }
        if depth < 24 {
            if let Ok(mut child) = walker.get_first_child(&el) {
                let mut sibs = 0usize;
                loop {
                    sibs += 1;
                    if sibs > 220 {
                        break;
                    }
                    let is_strictly_right_pane = depth >= 2
                        && child
                            .get_bounding_rectangle()
                            .ok()
                            .map(|r| {
                                let w = r.get_right() - r.get_left();
                                let h = r.get_bottom() - r.get_top();
                                w > 20 && h > 20 && r.get_left() > sidebar_right
                            })
                            .unwrap_or(false);
                    if is_strictly_right_pane {
                        secondary_queue.push_back((child.clone(), depth + 1));
                    } else {
                        primary_queue.push_back((child.clone(), depth + 1));
                    }
                    match walker.get_next_sibling(&child) {
                        Ok(next) => child = next,
                        Err(_) => break,
                    }
                }
            }
        }
    }

    let Some((score, in_sidebar, matched_name, chosen)) = best else {
        log::info!(
            "select_conversation_in_window_uia: no match for {:?} (visited={visited})",
            target
        );
        return false;
    };
    log::info!(
        "select_conversation_in_window_uia: matched {:?} (score={score}, in_sidebar={in_sidebar}, visited={visited})",
        matched_name
    );

    // Check chosen element and up to 3 ancestor levels for Invoke / SelectionItem patterns
    let mut cur = chosen.clone();
    for _ in 0..4 {
        if let Ok(inv) = cur.get_pattern::<uiautomation::patterns::UIInvokePattern>() {
            if inv.invoke().is_ok() {
                return true;
            }
        }
        if let Ok(sel) = cur.get_pattern::<uiautomation::patterns::UISelectionItemPattern>() {
            if sel.select().is_ok() {
                return true;
            }
        }
        match walker.get_parent(&cur) {
            Ok(parent) => cur = parent,
            Err(_) => break,
        }
    }
    chosen.click().is_ok()
}

pub fn resolve_claude_desktop_exe() -> Option<PathBuf> {
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        for rel in [
            r"AnthropicClaude\Claude.exe",
            r"Programs\claude-desktop\Claude.exe",
            r"Programs\Claude\Claude.exe",
        ] {
            let p = PathBuf::from(&local).join(rel);
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}

fn find_msix_app_launch_uri(prefix_lower: &str) -> Option<String> {
    let local = std::env::var("LOCALAPPDATA").ok()?;
    let packages_dir = PathBuf::from(local).join("Packages");
    let entries = fs::read_dir(packages_dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.to_ascii_lowercase().starts_with(prefix_lower) {
            return Some(format!(r"shell:AppsFolder\{name}!App"));
        }
    }
    None
}

fn launch_uri_or_exe_foreground(target: &str) {
    let verb_w = to_wide_null("open");
    let target_w = to_wide_null(target);
    unsafe {
        let _ = ShellExecuteW(
            None,
            PCWSTR(verb_w.as_ptr()),
            PCWSTR(target_w.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOW,
        );
    }
}

fn bring_hwnd_to_front_force(target_hwnd: HWND) {
    unsafe {
        if IsIconic(target_hwnd).as_bool() {
            let _ = ShowWindow(target_hwnd, SW_RESTORE);
        } else {
            let _ = ShowWindow(target_hwnd, SW_SHOW);
        }
        crate::window::focus_hwnd_without_sendinput(target_hwnd);
        let _ = BringWindowToTop(target_hwnd);
        // Temporary TOPMOST -> NOTOPMOST pulse guarantees the desktop app rises to the very front
        let _ = SetWindowPos(
            target_hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE,
        );
        let _ = SetWindowPos(
            target_hwnd,
            Some(HWND_NOTOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE,
        );
        let _ = SetForegroundWindow(target_hwnd);
    }
}

/// Builds an authenticated DSH WebUI URL preserving `?token=...` across DSH's 302 auth redirect
/// and passing the target session via URL fragment (`#atp_session=<sid>`) so DSH's SPA opens that
/// exact conversation without hitting a 404 route.
pub fn build_dsh_focus_url(launch_url: &str, session_id: Option<&str>) -> String {
    let trimmed_url = launch_url.trim();
    let sid = session_id.map(|s| s.trim()).filter(|s| !s.is_empty());
    let Ok(mut parsed) = url::Url::parse(trimmed_url) else {
        return match sid {
            Some(id) => format!("{trimmed_url}#atp_session={id}"),
            None => trimmed_url.to_string(),
        };
    };
    let kept_pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(k, _)| k != "dsh_bar_clone" && k != "dsb_mode")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if kept_pairs.is_empty() {
        parsed.set_query(None);
    } else {
        let mut qp = parsed.query_pairs_mut();
        qp.clear();
        for (k, v) in kept_pairs {
            qp.append_pair(&k, &v);
        }
    }
    if let Some(id) = sid {
        let encoded: String = url::form_urlencoded::byte_serialize(id.as_bytes())
            .collect::<String>()
            .replace('+', "%20");
        parsed.set_fragment(Some(&format!("atp_session={encoded}")));
    } else {
        parsed.set_fragment(None);
    }
    parsed.to_string()
}

pub fn focus_harness_desktop(
    app: AppHandle,
    kind: HarnessKind,
    session_id: Option<String>,
    title: Option<String>,
    workspace_path: Option<String>,
    config: crate::config::AppConfig,
    broker: Arc<BrokerClient>,
    dsh_conn: Arc<Mutex<crate::dsh::DshConnectionState>>,
) -> Result<String, String> {
    if kind == HarnessKind::Dsh {
        std::thread::spawn(move || {
            let _ = crate::plugin_sync::ensure_dsh_plugin_synced();
            let outcome = crate::dsh::discover_or_start_dsh(&config, &broker, true);
            match outcome {
                crate::dsh::DiscoveryOutcome::Connected { port, launch_url } => {
                    let needs_bar_nav = dsh_conn
                        .lock()
                        .map(|st| st.active_port != Some(port))
                        .unwrap_or(false);
                    if needs_bar_nav {
                        crate::dsh::apply_discovery_to_bar(
                            &app,
                            &dsh_conn,
                            crate::dsh::DiscoveryOutcome::Connected {
                                port,
                                launch_url: launch_url.clone(),
                            },
                        );
                    }
                    let url = build_dsh_focus_url(&launch_url, session_id.as_deref());
                    log::info!(
                        "focus_harness_desktop(dsh): opening authenticated WebUI on port {port} (session={:?})",
                        session_id
                    );
                    launch_uri_or_exe_foreground(&url);
                }
                crate::dsh::DiscoveryOutcome::MissingPlugin { port } => {
                    let url = build_dsh_focus_url(
                        &format!("http://127.0.0.1:{port}/"),
                        session_id.as_deref(),
                    );
                    log::warn!(
                        "focus_harness_desktop(dsh): plugin missing on port {port}, opening fallback WebUI"
                    );
                    launch_uri_or_exe_foreground(&url);
                }
                crate::dsh::DiscoveryOutcome::Unavailable { reason } => {
                    log::warn!("focus_harness_desktop(dsh): DSH unavailable: {reason}");
                }
            }
        });
        return Ok("正在启动并打开 DeepSeek Harness 对话".to_string());
    }

    let kind_label = kind.as_str();
    let resolved_title = title
        .as_deref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            let sid = session_id.as_deref()?.trim();
            if sid.is_empty() {
                return None;
            }
            match kind {
                HarnessKind::Antigravity => {
                    lookup_antigravity_conversation_meta(sid).map(|(_, _, _, t)| t)
                }
                HarnessKind::Codex => lookup_codex_thread_name(sid),
                _ => None,
            }
        });

    std::thread::spawn(move || {
        let mut freshly_launched = false;
        let mut hwnd = find_harness_desktop_window(kind);

        if hwnd.is_none() {
            match kind {
                HarnessKind::Antigravity => {
                    if let Some(exe) = resolve_antigravity_desktop_exe() {
                        launch_uri_or_exe_foreground(&exe.to_string_lossy());
                        freshly_launched = true;
                    }
                }
                HarnessKind::Codex => {
                    if let Some(apps_uri) = find_msix_app_launch_uri("openai.codex_") {
                        let verb_w = to_wide_null("open");
                        let exp_w = to_wide_null("explorer.exe");
                        let arg_w = to_wide_null(&apps_uri);
                        unsafe {
                            let _ = ShellExecuteW(
                                None,
                                PCWSTR(verb_w.as_ptr()),
                                PCWSTR(exp_w.as_ptr()),
                                PCWSTR(arg_w.as_ptr()),
                                PCWSTR::null(),
                                SW_SHOW,
                            );
                        }
                        freshly_launched = true;
                    }
                }
                HarnessKind::ClaudeCode => {
                    if let Some(exe) = resolve_claude_desktop_exe() {
                        launch_uri_or_exe_foreground(&exe.to_string_lossy());
                        freshly_launched = true;
                    } else if let Some(apps_uri) = find_msix_app_launch_uri("claude_") {
                        let verb_w = to_wide_null("open");
                        let exp_w = to_wide_null("explorer.exe");
                        let arg_w = to_wide_null(&apps_uri);
                        unsafe {
                            let _ = ShellExecuteW(
                                None,
                                PCWSTR(verb_w.as_ptr()),
                                PCWSTR(exp_w.as_ptr()),
                                PCWSTR(arg_w.as_ptr()),
                                PCWSTR::null(),
                                SW_SHOW,
                            );
                        }
                        freshly_launched = true;
                    } else if let Some(claude_cli) = resolve_claude_exe() {
                        // Fallback when only Claude Code CLI is installed: open an interactive terminal resuming that session
                        let sid = session_id.as_deref().map(|s| s.trim()).unwrap_or("");
                        let cli_cmd = if !sid.is_empty() && !sid.ends_with(":new") {
                            format!("\"{}\" --resume \"{}\"", claude_cli.to_string_lossy(), sid)
                        } else {
                            format!("\"{}\"", claude_cli.to_string_lossy())
                        };
                        let mut cmd = Command::new("cmd.exe");
                        cmd.args(["/K", &cli_cmd]);
                        if let Some(cwd) = workspace_path
                            .as_deref()
                            .map(clean_windows_path)
                            .filter(|p| !p.is_empty() && Path::new(p).exists())
                        {
                            cmd.current_dir(cwd);
                        }
                        let _ = cmd.spawn();
                        return;
                    }
                }
                HarnessKind::Dsh => unreachable!(),
            }

            if freshly_launched {
                let deadline = Instant::now() + Duration::from_secs(10);
                while Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(250));
                    if let Some(found) = find_harness_desktop_window(kind) {
                        hwnd = Some(found);
                        break;
                    }
                }
            }
        }

        let Some(target_hwnd) = hwnd else {
            log::warn!("focus_harness_desktop: no window found for {kind_label}");
            return;
        };

        bring_hwnd_to_front_force(target_hwnd);

        if let Some(ref conv_title) = resolved_title {
            std::thread::sleep(Duration::from_millis(if freshly_launched { 1600 } else { 220 }));
            let ok = select_conversation_in_window_uia(target_hwnd, conv_title);
            log::info!(
                "focus_harness_desktop({kind_label}): foregrounded window (title={:?}), select_conversation({:?})={ok}",
                window_title_text(target_hwnd),
                conv_title
            );
        }
    });

    Ok(format!("正在切换到 {}", kind.as_str()))
}

fn read_codex_enabled_plugins() -> Vec<HarnessOptionItem> {
    let mut out = Vec::new();
    let Some(home) = dirs::home_dir() else {
        return out;
    };
    let path = home.join(".codex").join("config.toml");
    let Ok(raw) = fs::read_to_string(&path) else {
        return out;
    };
    let mut cur_plugin: Option<(String, String)> = None; // (full_id, short_id)
    let mut seen = HashSet::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            cur_plugin = None;
            if let Some(rest) = trimmed.strip_prefix("[plugins.\"") {
                if let Some((raw_id, _)) = rest.split_once('"') {
                    let full_id = raw_id.trim();
                    let short = full_id.split('@').next().unwrap_or(full_id).trim();
                    if !short.is_empty() && !short.contains("anything-to-prompt") {
                        cur_plugin = Some((full_id.to_string(), short.to_string()));
                    }
                }
            }
            continue;
        }
        if let Some((ref full_id, ref short_id)) = cur_plugin {
            if let Some((k, v)) = trimmed.split_once('=') {
                if k.trim() == "enabled" && v.trim() == "true" && seen.insert(short_id.clone()) {
                    let (label, desc) = match short_id.as_str() {
                        "documents" => ("Documents", "Create and edit documents"),
                        "pdf" => ("PDF", "Read, create, and verify PDFs"),
                        "spreadsheets" => ("Spreadsheets", "Create and edit spreadsheets"),
                        "presentations" => ("Presentations", "Create and edit presentations"),
                        "browser" => ("Browser", "Browse and automate web pages"),
                        "visualize" => ("Visualize", "Generate charts and diagrams"),
                        "template-creator" => ("Templates", "Create reusable templates"),
                        "computer-use" | "unified-computer-use" => ("Computer Use", "Control desktop apps"),
                        "codex-app-tools" => ("App Tools", "Codex desktop integration tools"),
                        _ => (short_id.as_str(), "Codex enabled plugin"),
                    };
                    out.push(HarnessOptionItem {
                        id: full_id.clone(),
                        label: label.to_string(),
                        desc: desc.to_string(),
                    });
                }
            }
        }
    }
    out
}

/// Maps an Antigravity model family + chosen reasoning effort onto the exact model ID
/// advertised by `agy models` (e.g. `gemini-4-neon-high` + `low` -> `gemini-4-neon-low`).
pub fn resolve_antigravity_model_for_effort(model_id: &str, effort: Option<&str>) -> String {
    let m = model_id.trim();
    if m.is_empty() {
        return String::new();
    }
    let Some(eff_raw) = effort.map(|s| s.trim().to_ascii_lowercase()) else {
        return m.to_string();
    };
    if eff_raw.is_empty() {
        return m.to_string();
    }
    // Fixed-tier / orchestrator models keep their canonical IDs
    if m.starts_with("gemini-pro-orchestrator") || m.starts_with("gpt-oss-") {
        return m.to_string();
    }
    let base = m
        .strip_suffix("-xhigh")
        .or_else(|| m.strip_suffix("-high"))
        .or_else(|| m.strip_suffix("-medium"))
        .or_else(|| m.strip_suffix("-low"))
        .unwrap_or(m);
    let supports_xhigh = base == "gemini-4-neon" || base == "gemini-4-hydrogen";
    let supports_tier_suffix = matches!(
        base,
        "gemini-4-neon"
            | "gemini-4-carbon"
            | "gemini-4-argon"
            | "gemini-4-hydrogen"
            | "gemini-3.8-flash"
            | "claude-opus-5-5"
            | "claude-sonnet-5-5"
    );
    if !supports_tier_suffix {
        return m.to_string();
    }
    let mapped_suffix = match eff_raw.as_str() {
        "low" => "low",
        "medium" => "medium",
        "high" => "high",
        "xhigh" | "max" => {
            if supports_xhigh {
                "xhigh"
            } else {
                "high"
            }
        }
        _ => return m.to_string(),
    };
    format!("{base}-{mapped_suffix}")
}

pub fn load_harness_composer_options(kind: HarnessKind) -> HarnessComposerOptions {
    match kind {
        HarnessKind::Antigravity => HarnessComposerOptions {
            harness: "antigravity".to_string(),
            models: vec![
                HarnessOptionItem { id: "gemini-4-neon-high".into(), label: "Gemini 4 Neon".into(), desc: "High|New|info".into() },
                HarnessOptionItem { id: "gemini-4-carbon-medium".into(), label: "Gemini 4 Carbon".into(), desc: "Medium|New|info".into() },
                HarnessOptionItem { id: "gemini-4-argon-medium".into(), label: "Gemini 4 Argon".into(), desc: "Medium|Leaving Soon|info".into() },
                HarnessOptionItem { id: "gemini-4-hydrogen-medium".into(), label: "Gemini 4 Hydrogen".into(), desc: "Medium|New|info".into() },
                HarnessOptionItem { id: "gemini-pro-orchestrator-neon".into(), label: "Orchestrator".into(), desc: "Claude Opus 5.5 High|New|info".into() },
                HarnessOptionItem { id: "gemini-3.8-flash-medium".into(), label: "Gemini 3.8 Flash".into(), desc: "Medium|Leaving Soon|info".into() },
                HarnessOptionItem { id: "claude-opus-5-5-medium".into(), label: "Claude Opus 5.5".into(), desc: "Medium|New".into() },
                HarnessOptionItem { id: "claude-sonnet-5-5-medium".into(), label: "Claude Sonnet 5.5".into(), desc: "Medium|New".into() },
                HarnessOptionItem { id: "gpt-oss-120b-medium".into(), label: "GPT-OSS 120B (Medium)".into(), desc: "|Notice|info".into() },
            ],
            efforts: vec![
                HarnessOptionItem { id: "low".into(), label: "Low".into(), desc: "Fast low-latency".into() },
                HarnessOptionItem { id: "medium".into(), label: "Medium".into(), desc: "Balanced reasoning".into() },
                HarnessOptionItem { id: "high".into(), label: "High".into(), desc: "Deep complex reasoning".into() },
                HarnessOptionItem { id: "xhigh".into(), label: "XHigh".into(), desc: "Extended thinking budget".into() },
                HarnessOptionItem { id: "max".into(), label: "Max".into(), desc: "Maximum thinking budget".into() },
            ],
            modes: vec![
                HarnessOptionItem { id: "turbo".into(), label: "Turbo".into(), desc: "--dangerously-skip-permissions · 无限制执行".into() },
                HarnessOptionItem { id: "auto".into(), label: "Auto".into(), desc: "--mode auto · 自动评估风险执行".into() },
                HarnessOptionItem { id: "default".into(), label: "Default".into(), desc: "--mode accept-edits · 敏感操作前询问".into() },
                HarnessOptionItem { id: "request-review".into(), label: "Request Review".into(), desc: "--sandbox --mode plan · 沙箱与规划审批".into() },
            ],
            plugins: Vec::new(),
            actions: vec![
                HarnessOptionItem { id: "/plan".into(), label: "/plan".into(), desc: "先制定分步执行计划".into() },
                HarnessOptionItem { id: "/goal".into(), label: "/goal".into(), desc: "开启长周期目标模式".into() },
                HarnessOptionItem { id: "/browser".into(), label: "/browser".into(), desc: "使用内置浏览器查验网页".into() },
                HarnessOptionItem { id: "/compact".into(), label: "/compact".into(), desc: "压缩上下文释放窗口".into() },
                HarnessOptionItem { id: "/boost".into(), label: "/boost".into(), desc: "深度多视角思考与验证".into() },
                HarnessOptionItem { id: "/grill-me".into(), label: "/grill-me".into(), desc: "通过追问澄清设计决策".into() },
                HarnessOptionItem { id: "/learn".into(), label: "/learn".into(), desc: "沉淀规则到长期记忆".into() },
            ],
            default_model: "gemini-4-neon-high".to_string(),
            default_effort: "high".to_string(),
            default_mode: "turbo".to_string(),
        },
        HarnessKind::Codex => {
            let (_cfg_model_label, cfg_effort_label, _cfg_access) = read_codex_config_display();
            let models = vec![
                HarnessOptionItem { id: "default".into(), label: "默认".into(), desc: "default".into() },
                HarnessOptionItem { id: "gpt-6.2-sol".into(), label: "GPT-6.2 Sol".into(), desc: "".into() },
                HarnessOptionItem { id: "gpt-6.2-astra".into(), label: "GPT-6.2 Astra".into(), desc: "".into() },
                HarnessOptionItem { id: "gpt-6.1-terra".into(), label: "GPT-6.1 Terra".into(), desc: "".into() },
                HarnessOptionItem { id: "gpt-6.1-sol".into(), label: "GPT-6.1 Sol".into(), desc: "".into() },
                HarnessOptionItem { id: "gpt-6-astra".into(), label: "GPT-6 Astra".into(), desc: "".into() },
                HarnessOptionItem { id: "gpt-6-sol".into(), label: "GPT-6 Sol".into(), desc: "".into() },
                HarnessOptionItem { id: "gpt-6-luna".into(), label: "GPT-6 Luna".into(), desc: "".into() },
                HarnessOptionItem { id: "gpt-5.6-sol".into(), label: "GPT-5.6 Sol".into(), desc: "".into() },
                HarnessOptionItem { id: "gpt-5.6-terra".into(), label: "GPT-5.6 Terra".into(), desc: "".into() },
            ];
            let default_effort = match cfg_effort_label.as_str() {
                "极低" => "minimal",
                "低" => "low",
                "中" => "medium",
                "极高" => "xhigh",
                _ => "high",
            }
            .to_string();
            HarnessComposerOptions {
                harness: "codex".to_string(),
                models,
                efforts: vec![
                    HarnessOptionItem { id: "minimal".into(), label: "极低".into(), desc: "极速轻量推理".into() },
                    HarnessOptionItem { id: "low".into(), label: "低".into(), desc: "快速轻量推理".into() },
                    HarnessOptionItem { id: "medium".into(), label: "中".into(), desc: "均衡工程推理".into() },
                    HarnessOptionItem { id: "high".into(), label: "高".into(), desc: "深度工程推理".into() },
                    HarnessOptionItem { id: "xhigh".into(), label: "极高".into(), desc: "拉满思考预算".into() },
                ],
                modes: vec![
                    HarnessOptionItem { id: "danger-full-access".into(), label: "完全访问权限".into(), desc: "approvalPolicy=never · dangerFullAccess".into() },
                    HarnessOptionItem { id: "help-approve".into(), label: "帮我批准".into(), desc: "approvalsReviewer=auto_review · workspaceWrite".into() },
                    HarnessOptionItem { id: "request-approval".into(), label: "请求批准".into(), desc: "approvalPolicy=on-request · workspaceWrite".into() },
                    HarnessOptionItem { id: "read-only".into(), label: "只读模式".into(), desc: "sandboxPolicy=readOnly · 仅分析不修改".into() },
                ],
                plugins: read_codex_enabled_plugins(),
                actions: vec![
                    HarnessOptionItem { id: "/plan".into(), label: "计划模式".into(), desc: "开启只读计划模式".into() },
                    HarnessOptionItem { id: "/goal".into(), label: "目标".into(), desc: "设置要持续追求的目标".into() },
                ],
                default_model: "default".to_string(),
                default_effort,
                default_mode: "danger-full-access".to_string(),
            }
        }
        HarnessKind::ClaudeCode => {
            let (_m_label, e_label, _) = read_claude_code_config_display();
            let default_effort = match e_label.as_str() {
                "低" => "low",
                "中" => "medium",
                "极高" => "xhigh",
                "Max" => "max",
                _ => "high",
            }
            .to_string();
            HarnessComposerOptions {
                harness: "claude_code".to_string(),
                models: vec![
                    HarnessOptionItem { id: "claude-opus-5-5".into(), label: "Opus 5.5".into(), desc: "Default · Best for everyday, complex tasks".into() },
                    HarnessOptionItem { id: "claude-fable-5-2".into(), label: "Fable 5.2".into(), desc: "Most capable for hardest, longest-running tasks".into() },
                    HarnessOptionItem { id: "claude-sonnet-5-5".into(), label: "Sonnet 5.5".into(), desc: "Efficient for routine tasks".into() },
                    HarnessOptionItem { id: "claude-haiku-5-5".into(), label: "Haiku 5.5".into(), desc: "Fastest for quick answers".into() },
                ],
                efforts: vec![
                    HarnessOptionItem { id: "low".into(), label: "Low".into(), desc: "快速响应".into() },
                    HarnessOptionItem { id: "medium".into(), label: "Medium".into(), desc: "常规推理".into() },
                    HarnessOptionItem { id: "high".into(), label: "High".into(), desc: "深度推理".into() },
                    HarnessOptionItem { id: "xhigh".into(), label: "XHigh".into(), desc: "超高深度推理".into() },
                    HarnessOptionItem { id: "max".into(), label: "Max".into(), desc: "最大思考深度".into() },
                ],
                modes: vec![
                    HarnessOptionItem { id: "auto".into(), label: "Auto".into(), desc: "--permission-mode auto · 智能自动执行并拦截高危".into() },
                    HarnessOptionItem { id: "acceptEdits".into(), label: "Accept Edits".into(), desc: "--permission-mode acceptEdits · 自动写入文件".into() },
                    HarnessOptionItem { id: "plan".into(), label: "Plan".into(), desc: "--permission-mode plan · 只读分析与规划".into() },
                    HarnessOptionItem { id: "manual".into(), label: "Manual".into(), desc: "--permission-mode manual · 每步手动审批".into() },
                    HarnessOptionItem { id: "dontAsk".into(), label: "Don't Ask".into(), desc: "--permission-mode dontAsk · 仅允许预授权工具".into() },
                    HarnessOptionItem { id: "bypassPermissions".into(), label: "Bypass All".into(), desc: "--permission-mode bypassPermissions · 跳过全部权限检查".into() },
                ],
                plugins: Vec::new(),
                actions: vec![
                    HarnessOptionItem { id: "/plan".into(), label: "/plan".into(), desc: "切换到只读规划模式".into() },
                    HarnessOptionItem { id: "/compact".into(), label: "/compact".into(), desc: "立即压缩当前会话上下文".into() },
                    HarnessOptionItem { id: "/review".into(), label: "/review".into(), desc: "审查当前代码变更".into() },
                    HarnessOptionItem { id: "/cost".into(), label: "/cost".into(), desc: "查看当前会话真实 Token 用量".into() },
                    HarnessOptionItem { id: "/clear".into(), label: "/clear".into(), desc: "清空会话并开启新对话".into() },
                ],
                default_model: "claude-opus-5-5".to_string(),
                default_effort,
                default_mode: "auto".to_string(),
            }
        }
        HarnessKind::Dsh => HarnessComposerOptions {
            harness: "dsh".to_string(),
            models: Vec::new(),
            efforts: Vec::new(),
            modes: Vec::new(),
            plugins: Vec::new(),
            actions: Vec::new(),
            default_model: String::new(),
            default_effort: String::new(),
            default_mode: String::new(),
        },
    }
}

pub fn list_harness_availability(
    dsh_connected: bool,
    custom_acp: &[crate::config::CustomAcpHarness],
    target_window_label: Option<&str>,
) -> Vec<HarnessAvailability> {
    let agy = resolve_agy_exe();
    let codex = resolve_codex_exe();
    let claude = resolve_claude_exe();
    let agy_running = find_main_window_for_exe("antigravity.exe").is_some()
        || is_process_name_running("antigravity.exe");
    let codex_running = find_codex_desktop_window().is_some();
    let claude_running = find_main_window_for_exe("claude.exe").is_some();
    let (codex_model, codex_effort, codex_access) = read_codex_config_display();
    let (claude_model, claude_effort, claude_access) = read_claude_code_config_display();
    let mut list = vec![
        HarnessAvailability {
            id: "dsh".to_string(),
            label: "DeepSeek Harness".to_string(),
            subtitle: if dsh_connected {
                "本地 WebUI 实时双向孪生".to_string()
            } else {
                "未连接本地 DSH 服务".to_string()
            },
            available: dsh_connected,
            running: dsh_connected,
            exe_path: None,
            sync_note: "毫秒级双向同步".to_string(),
            model_label: String::new(),
            effort_label: String::new(),
            access_label: String::new(),
            provider_kind: "native".to_string(),
            accent: "#4d6bfe".to_string(),
            badge: "DS".to_string(),
            custom: false,
            command: String::new(),
            args: Vec::new(),
        },
        HarnessAvailability {
            id: "antigravity".to_string(),
            label: "Antigravity".to_string(),
            subtitle: "CLI Project 原生同步桌面端".to_string(),
            available: agy.is_some() || resolve_antigravity_desktop_exe().is_some(),
            running: agy_running,
            exe_path: agy.map(|p| p.to_string_lossy().to_string()),
            sync_note: "自动同步至桌面端 CLI Project".to_string(),
            model_label: "Gemini 4 Neon".to_string(),
            effort_label: "High".to_string(),
            access_label: "Turbo".to_string(),
            provider_kind: "native".to_string(),
            accent: "#4285f4".to_string(),
            badge: "AG".to_string(),
            custom: false,
            command: String::new(),
            args: Vec::new(),
        },
        HarnessAvailability {
            id: "codex".to_string(),
            label: "Codex".to_string(),
            subtitle: "app-server 同库共享历史".to_string(),
            available: codex.is_some(),
            running: codex_running,
            exe_path: codex.map(|p| p.to_string_lossy().to_string()),
            sync_note: "写入 ~/.codex 历史（桌面端刷新可见）".to_string(),
            model_label: codex_model,
            effort_label: codex_effort,
            access_label: codex_access,
            provider_kind: "native".to_string(),
            accent: "#10a37f".to_string(),
            badge: "CX".to_string(),
            custom: false,
            command: String::new(),
            args: Vec::new(),
        },
        HarnessAvailability {
            id: "claude_code".to_string(),
            label: "Claude Code".to_string(),
            subtitle: "CLI 原生流式 & 同库共享历史".to_string(),
            available: claude.is_some(),
            running: claude_running,
            exe_path: claude.map(|p| p.to_string_lossy().to_string()),
            sync_note: "与 ~/.claude/projects 共享会话历史".to_string(),
            model_label: claude_model,
            effort_label: claude_effort,
            access_label: claude_access,
            provider_kind: "native".to_string(),
            accent: "#d97757".to_string(),
            badge: "CC".to_string(),
            custom: false,
            command: String::new(),
            args: Vec::new(),
        },
    ];

    // Append auto-discovered native ACP agents & user-configured custom ACP harnesses
    list.extend(crate::acp::list_acp_availability(custom_acp));

    // Append Universal Focused-Window Drop-In fallback (works with ANY closed app or webpage)
    let tgt_desc = target_window_label
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| format!("目标窗口：{s}"))
        .unwrap_or_else(|| "一键投递到任意聊天窗 / 网页输入框".to_string());
    list.push(HarnessAvailability {
        id: "universal".to_string(),
        label: "任意窗口投递".to_string(),
        subtitle: tgt_desc,
        available: true,
        running: true,
        exe_path: None,
        sync_note: "零侵入 · 粘贴后自动还原原剪贴板".to_string(),
        model_label: "万能投递".to_string(),
        effort_label: String::new(),
        access_label: "仅粘贴".to_string(),
        provider_kind: "universal".to_string(),
        accent: "#06b6d4".to_string(),
        badge: "投".to_string(),
        custom: false,
        command: String::new(),
        args: Vec::new(),
    });

    list
}

pub(crate) fn clean_windows_path(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix(r"\\?\") {
        return rest.to_string();
    }
    if let Some(rest) = trimmed.strip_prefix("file:///") {
        let decoded = rest.replace("%20", " ").replace('/', r"\");
        return decoded;
    }
    trimmed.to_string()
}

/// Returns (and creates if missing) the unified default workspace directory
/// (`~/Documents/AnythingToPrompt`, falling back to `~/AnythingToPrompt`) so brand-new users
/// with zero existing workspaces can start chatting in any installed Harness immediately.
pub fn ensure_default_workspace_dir() -> PathBuf {
    let base = dirs::document_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("AnythingToPrompt");
    let _ = fs::create_dir_all(&dir);
    dir
}

/// Resolves an explicit workspace directory if it exists on disk, otherwise falls back to
/// `ensure_default_workspace_dir()`.
pub fn resolve_effective_workspace_dir(cwd: Option<&str>) -> Option<String> {
    if let Some(dir) = cwd
        .map(clean_windows_path)
        .filter(|p| !p.trim().is_empty() && Path::new(p).is_dir())
    {
        return Some(dir);
    }
    let fallback = ensure_default_workspace_dir();
    if fallback.is_dir() {
        Some(fallback.to_string_lossy().to_string())
    } else {
        dirs::home_dir().map(|h| h.to_string_lossy().to_string())
    }
}

pub(crate) fn basename_of_path(path: &str) -> String {
    let cleaned = clean_windows_path(path);
    let trimmed = cleaned.trim_end_matches(['\\', '/']);
    if trimmed.is_empty() {
        return "默认工作区".to_string();
    }
    Path::new(trimmed)
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(trimmed)
        .to_string()
}

pub(crate) fn sanitize_title(raw: &str, fallback: &str) -> String {
    let first_line = raw
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("<USER_REQUEST>"))
        .unwrap_or(raw.trim());
    let cleaned = first_line
        .replace("<USER_REQUEST>", "")
        .replace("</USER_REQUEST>", "")
        .replace("\\*", "")
        .trim()
        .to_string();
    if cleaned.is_empty() {
        return fallback.to_string();
    }
    let mut chars = cleaned.chars();
    let truncated: String = chars.by_ref().take(36).collect();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

fn open_sqlite_readonly(path: &Path) -> Option<Connection> {
    if !path.is_file() {
        return None;
    }
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()
}

fn parse_first_workspace_uri(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if let Ok(arr) = serde_json::from_str::<Vec<String>>(trimmed) {
        if let Some(first) = arr.first() {
            return clean_windows_path(first);
        }
    }
    clean_windows_path(trimmed)
}

fn is_antigravity_archived(home: &Path, conv_id: &str) -> bool {
    let pbtxt = home
        .join(".gemini")
        .join("antigravity")
        .join("annotations")
        .join(format!("{conv_id}.pbtxt"));
    let Ok(raw) = fs::read_to_string(&pbtxt) else {
        return false;
    };
    raw.contains("archived:true") || raw.contains("archived: true")
}

pub fn load_antigravity_catalog() -> ExternalHarnessCatalog {
    let exe = resolve_agy_exe();
    let Some(home) = dirs::home_dir() else {
        return ExternalHarnessCatalog {
            harness: "antigravity".to_string(),
            available: exe.is_some(),
            exe_path: exe.map(|p| p.to_string_lossy().to_string()),
            groups: Vec::new(),
            error: Some("无法定位用户目录".to_string()),
        };
    };

    let gemini_dir = home.join(".gemini");
    // Use Antigravity Desktop's authoritative DB only so archived/deleted CLI probes never pollute the list
    let db_path = gemini_dir
        .join("antigravity")
        .join("conversation_summaries.db");

    // Map project_id -> (label, path, Vec<ExternalConversation>)
    // Do NOT unconditionally inject a phantom "CLI Project" — only include projects that
    // actually have active (non-killed, non-archived) conversations in Antigravity's DB,
    // so deleted workspaces vanish immediately on refresh.
    let mut by_project: HashMap<String, (String, String, Vec<ExternalConversation>)> =
        HashMap::new();
    let mut seen_conv_ids: HashMap<String, usize> = HashMap::new();

    if let Some(conn) = open_sqlite_readonly(&db_path) {
        if let Ok(mut stmt) = conn.prepare(
            "SELECT conversation_id, COALESCE(title, ''), COALESCE(preview, ''), \
             COALESCE(CAST(strftime('%s', substr(last_modified_time, 1, 19)) AS INTEGER) * 1000, \
                      CAST(last_modified_time AS INTEGER), 0), \
             COALESCE(workspace_uris, ''), COALESCE(source, ''), \
             COALESCE(project_id, ''), COALESCE(parent_conversation_id, '') \
             FROM conversation_summaries \
             WHERE COALESCE(killed, 0) = 0 \
             ORDER BY last_modified_time DESC LIMIT 120",
        ) {
            if let Ok(rows) = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            }) {
                for row in rows.flatten() {
                    let (conv_id, title, preview, updated_at, ws_uris, source, project_id, parent_id) =
                        row;
                    if conv_id.trim().is_empty() || !parent_id.trim().is_empty() {
                        continue;
                    }
                    if seen_conv_ids.contains_key(&conv_id) {
                        continue;
                    }
                    seen_conv_ids.insert(conv_id.clone(), 1);

                    // Match Antigravity Desktop sidebar: exclude archived conversations (stored in annotations/*.pbtxt)
                    if is_antigravity_archived(&home, &conv_id) {
                        continue;
                    }

                    // Exclude headless CLI one-shot/probe runs (empty title + CLI source) hidden by Antigravity Desktop
                    if source.eq_ignore_ascii_case("CORTEX_TRAJECTORY_SOURCE_CLI")
                        && title.trim().is_empty()
                    {
                        continue;
                    }

                    let ws_path = parse_first_workspace_uri(&ws_uris);
                    let normalized_pid = if project_id.trim().is_empty() {
                        "default-cli-project".to_string()
                    } else {
                        project_id.trim().to_string()
                    };

                    let label = if normalized_pid == "default-cli-project" {
                        "CLI Project".to_string()
                    } else if normalized_pid == "outside-of-project" {
                        "自由对话".to_string()
                    } else if !ws_path.is_empty() {
                        basename_of_path(&ws_path)
                    } else {
                        format!("项目 {}", &normalized_pid[..normalized_pid.len().min(8)])
                    };

                    let display_title = if !title.trim().is_empty() {
                        sanitize_title(&title, "新对话")
                    } else if !preview.trim().is_empty() {
                        sanitize_title(&preview, "新对话")
                    } else {
                        format!("对话 {}", &conv_id[..conv_id.len().min(8)])
                    };

                    let updated_ms = if updated_at > 0 && updated_at < 10_000_000_000 {
                        updated_at * 1000
                    } else {
                        updated_at
                    };

                    let entry = by_project
                        .entry(normalized_pid.clone())
                        .or_insert_with(|| (label.clone(), ws_path.clone(), Vec::new()));
                    if entry.1.is_empty() && !ws_path.is_empty() {
                        entry.1 = ws_path.clone();
                        if normalized_pid != "default-cli-project"
                            && normalized_pid != "outside-of-project"
                        {
                            entry.0 = basename_of_path(&ws_path);
                        }
                    }
                    entry.2.push(ExternalConversation {
                        id: conv_id,
                        title: display_title,
                        updated_at: updated_ms,
                        source,
                        cwd: ws_path,
                        rollout_path: None,
                    });
                }
            }
        }
    }

    let mut groups: Vec<ExternalWorkspaceGroup> = by_project
        .into_iter()
        .map(|(workspace_id, (label, path, mut convs))| {
            convs.sort_by_key(|c| std::cmp::Reverse(c.updated_at));
            ExternalWorkspaceGroup {
                workspace_id,
                label,
                path,
                convs,
            }
        })
        .collect();

    // Put real Desktop projects on top (newest first) and CLI Project at the bottom — matching Antigravity Desktop
    groups.sort_by(|a, b| {
        let a_cli = a.workspace_id == "default-cli-project";
        let b_cli = b.workspace_id == "default-cli-project";
        a_cli.cmp(&b_cli).then_with(|| {
            let a_ts = a.convs.first().map(|c| c.updated_at).unwrap_or(0);
            let b_ts = b.convs.first().map(|c| c.updated_at).unwrap_or(0);
            b_ts.cmp(&a_ts)
        })
    });

    ExternalHarnessCatalog {
        harness: "antigravity".to_string(),
        available: exe.is_some(),
        exe_path: exe.map(|p| p.to_string_lossy().to_string()),
        groups,
        error: None,
    }
}

/// Read-only lookup of an Antigravity conversation's authoritative metadata
/// `(workspace_id, workspace_path, workspace_label, title)` from `conversation_summaries.db`.
/// Prevents real-time hook / JSONL tail updates from overwriting the real conversation title
/// (e.g. "DeepSeek Bar UI Bug Fix") with a raw user message snippet.
pub fn lookup_antigravity_conversation_meta(
    conv_id: &str,
) -> Option<(String, String, String, String)> {
    let cid = conv_id.trim();
    if cid.is_empty() {
        return None;
    }
    let home = dirs::home_dir()?;
    let gemini_dir = home.join(".gemini");
    let dbs = [
        gemini_dir
            .join("antigravity")
            .join("conversation_summaries.db"),
        gemini_dir
            .join("antigravity-cli")
            .join("conversation_summaries.db"),
    ];

    for db_path in dbs {
        let Some(conn) = open_sqlite_readonly(&db_path) else {
            continue;
        };
        let row: rusqlite::Result<(String, String, String, String)> = conn.query_row(
            "SELECT COALESCE(title, ''), COALESCE(preview, ''), \
             COALESCE(workspace_uris, ''), COALESCE(project_id, '') \
             FROM conversation_summaries WHERE conversation_id = ?1 LIMIT 1",
            [cid],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        );
        if let Ok((title, preview, ws_uris, project_id)) = row {
            let ws_path = parse_first_workspace_uri(&ws_uris);
            let normalized_pid = if project_id.trim().is_empty() {
                "default-cli-project".to_string()
            } else {
                project_id.trim().to_string()
            };
            let label = if normalized_pid == "default-cli-project" {
                "CLI Project (桌面端同步)".to_string()
            } else if normalized_pid == "outside-of-project" {
                "自由对话".to_string()
            } else if !ws_path.is_empty() {
                basename_of_path(&ws_path)
            } else {
                format!("项目 {}", &normalized_pid[..normalized_pid.len().min(8)])
            };
            let display_title = if !title.trim().is_empty() {
                sanitize_title(&title, "新对话")
            } else if !preview.trim().is_empty() {
                sanitize_title(&preview, "新对话")
            } else {
                String::new()
            };
            if !display_title.is_empty() || !ws_path.is_empty() {
                return Some((normalized_pid, ws_path, label, display_title));
            }
        }
    }
    None
}

/// Returns the most recently active Antigravity Desktop project_id (excluding CLI & free chats),
/// used to pre-populate the official Plugin Sidecar's default `projectId` if unset.
pub fn latest_antigravity_desktop_project_id() -> Option<String> {
    let home = dirs::home_dir()?;
    let db_path = home
        .join(".gemini")
        .join("antigravity")
        .join("conversation_summaries.db");
    let conn = open_sqlite_readonly(&db_path)?;
    conn.query_row(
        "SELECT project_id FROM conversation_summaries \
         WHERE COALESCE(killed, 0) = 0 \
           AND COALESCE(parent_conversation_id, '') = '' \
           AND COALESCE(project_id, '') != '' \
           AND project_id != 'default-cli-project' \
           AND project_id != 'outside-of-project' \
         ORDER BY last_modified_time DESC LIMIT 1",
        [],
        |r| r.get::<_, String>(0),
    )
    .ok()
    .filter(|s| !s.trim().is_empty())
}

fn is_antigravity_cli_conversation(session_id: &str) -> bool {
    let cid = session_id.trim();
    if cid.is_empty() {
        return false;
    }
    let Some(home) = dirs::home_dir() else {
        return false;
    };
    if home
        .join(".gemini")
        .join("antigravity-cli")
        .join("brain")
        .join(cid)
        .is_dir()
    {
        return true;
    }
    let db_path = home
        .join(".gemini")
        .join("antigravity")
        .join("conversation_summaries.db");
    if let Some(conn) = open_sqlite_readonly(&db_path) {
        if let Ok((src, pid)) = conn.query_row(
            "SELECT COALESCE(source, ''), COALESCE(project_id, '') \
             FROM conversation_summaries WHERE conversation_id = ?1 LIMIT 1",
            [cid],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        ) {
            if src.to_ascii_uppercase().contains("CLI") || pid == "default-cli-project" {
                return true;
            }
        }
    }
    false
}

fn load_codex_session_index_names(home: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let idx_path = home.join(".codex").join("session_index.jsonl");
    let Ok(raw) = fs::read_to_string(&idx_path) else {
        return map;
    };
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        let id = v.get("id").and_then(|s| s.as_str()).unwrap_or("").trim();
        let name = v
            .get("thread_name")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .trim();
        if !id.is_empty() && !name.is_empty() {
            map.insert(id.to_string(), name.to_string());
        }
    }
    map
}

/// Read-only authoritative title lookup for a Codex thread (`threads.name` -> `session_index.jsonl`).
pub fn lookup_codex_thread_name(thread_id: &str) -> Option<String> {
    let tid = thread_id.trim();
    if tid.is_empty() {
        return None;
    }
    let home = dirs::home_dir()?;
    let db_path = home.join(".codex").join("state_5.sqlite");
    if let Some(conn) = open_sqlite_readonly(&db_path) {
        if let Ok(name) = conn.query_row(
            "SELECT COALESCE(name, '') FROM threads WHERE id = ?1 LIMIT 1",
            [tid],
            |r| r.get::<_, String>(0),
        ) {
            if !name.trim().is_empty() {
                return Some(sanitize_title(&name, "Codex 会话"));
            }
        }
    }
    let idx = load_codex_session_index_names(&home);
    idx.get(tid).map(|n| sanitize_title(n, "Codex 会话"))
}

fn normalize_win_path_key(p: &str) -> String {
    clean_windows_path(p)
        .trim()
        .trim_end_matches(['\\', '/'])
        .replace('/', "\\")
        .to_ascii_lowercase()
}

pub fn load_codex_catalog() -> ExternalHarnessCatalog {
    let exe = resolve_codex_exe();
    let Some(home) = dirs::home_dir() else {
        return ExternalHarnessCatalog {
            harness: "codex".to_string(),
            available: exe.is_some(),
            exe_path: exe.map(|p| p.to_string_lossy().to_string()),
            groups: Vec::new(),
            error: Some("无法定位用户目录".to_string()),
        };
    };

    let db_path = home.join(".codex").join("state_5.sqlite");
    let Some(conn) = open_sqlite_readonly(&db_path) else {
        return ExternalHarnessCatalog {
            harness: "codex".to_string(),
            available: exe.is_some(),
            exe_path: exe.map(|p| p.to_string_lossy().to_string()),
            groups: vec![ExternalWorkspaceGroup {
                workspace_id: home.to_string_lossy().to_string(),
                label: basename_of_path(&home.to_string_lossy()),
                path: home.to_string_lossy().to_string(),
                convs: Vec::new(),
            }],
            error: None,
        };
    };

    // 1. Read Codex Desktop's `.codex-global-state.json` (read-only) for exact Projects / Assignments / Projectless IDs
    struct CodexProjectDef {
        id: String,
        name: String,
        root_paths: Vec<String>,
        root_keys: Vec<String>,
    }
    let mut local_projects: Vec<CodexProjectDef> = Vec::new();
    let mut thread_proj_assign: HashMap<String, String> = HashMap::new();
    let mut projectless_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    let gs_path = home.join(".codex").join(".codex-global-state.json");
    if let Ok(raw_gs) = fs::read_to_string(&gs_path) {
        if let Ok(gs) = serde_json::from_str::<serde_json::Value>(&raw_gs) {
            let order: Vec<String> = gs
                .get("project-order")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();

            if let Some(proj_obj) = gs.get("local-projects").and_then(|v| v.as_object()) {
                let mut seen_pids = std::collections::HashSet::new();
                let mut ordered_keys: Vec<String> = Vec::new();
                for pid in order {
                    if proj_obj.contains_key(&pid) && seen_pids.insert(pid.clone()) {
                        ordered_keys.push(pid);
                    }
                }
                for pid in proj_obj.keys() {
                    if seen_pids.insert(pid.clone()) {
                        ordered_keys.push(pid.clone());
                    }
                }
                for pid in ordered_keys {
                    if let Some(entry) = proj_obj.get(&pid).and_then(|v| v.as_object()) {
                        let name = entry
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .trim()
                            .to_string();
                        let roots: Vec<String> = entry
                            .get("rootPaths")
                            .and_then(|v| v.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|x| x.as_str().map(clean_windows_path))
                                    .filter(|s| !s.is_empty())
                                    .collect()
                            })
                            .unwrap_or_default();
                        let label = if !name.is_empty() {
                            name
                        } else if let Some(first) = roots.first() {
                            basename_of_path(first)
                        } else {
                            continue;
                        };
                        let root_keys: Vec<String> =
                            roots.iter().map(|r| normalize_win_path_key(r)).collect();
                        local_projects.push(CodexProjectDef {
                            id: pid,
                            name: label,
                            root_paths: roots,
                            root_keys,
                        });
                    }
                }
            }

            if let Some(assigns) = gs
                .get("thread-project-assignments")
                .and_then(|v| v.as_object())
            {
                for (tid, val) in assigns {
                    if let Some(pid) = val.get("projectId").and_then(|x| x.as_str()) {
                        if !pid.trim().is_empty() {
                            thread_proj_assign.insert(tid.clone(), pid.trim().to_string());
                        }
                    }
                }
            }

            if let Some(pless) = gs.get("projectless-thread-ids").and_then(|v| v.as_array()) {
                for v in pless {
                    if let Some(tid) = v.as_str() {
                        projectless_ids.insert(tid.to_string());
                    }
                }
            }
        }
    }

    let index_names = load_codex_session_index_names(&home);
    let mut all_threads: Vec<(ExternalConversation, String)> = Vec::new(); // (conv, db_project_id)

    if let Ok(mut stmt) = conn.prepare(
        "SELECT id, COALESCE(source, ''), COALESCE(cwd, ''), COALESCE(name, ''), \
         COALESCE(title, ''), COALESCE(first_user_message, ''), \
         COALESCE(updated_at_ms, updated_at * 1000, 0), COALESCE(rollout_path, ''), \
         COALESCE(tokens_used, 0), COALESCE(project_id, '') \
         FROM threads \
         WHERE COALESCE(archived, 0) = 0 \
           AND source NOT LIKE '{%' \
           AND source != 'exec' \
         ORDER BY COALESCE(updated_at_ms, updated_at * 1000, 0) DESC LIMIT 120",
    ) {
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, String>(9)?,
            ))
        }) {
            for row in rows.flatten() {
                let (id, source, raw_cwd, name, title, first_msg, updated_ms, rollout, tokens_used, db_pid) =
                    row;
                // Skip empty / unstarted ghost threads (0 tokens & no user content) hidden by Codex Desktop
                if tokens_used == 0 && title.trim().is_empty() && first_msg.trim().is_empty() {
                    continue;
                }
                let cwd = clean_windows_path(&raw_cwd);
                let display_title = if !name.trim().is_empty() {
                    sanitize_title(&name, "Codex 会话")
                } else if let Some(idx_name) = index_names.get(&id) {
                    sanitize_title(idx_name, "Codex 会话")
                } else if !title.trim().is_empty() {
                    sanitize_title(&title, "Codex 会话")
                } else if !first_msg.trim().is_empty() {
                    sanitize_title(&first_msg, "Codex 会话")
                } else {
                    format!("会话 {}", &id[..id.len().min(8)])
                };
                all_threads.push((
                    ExternalConversation {
                        id,
                        title: display_title,
                        updated_at: updated_ms,
                        source,
                        cwd,
                        rollout_path: if rollout.trim().is_empty() {
                            None
                        } else {
                            Some(clean_windows_path(&rollout))
                        },
                    },
                    db_pid,
                ));
            }
        }
    }

    let mut groups: Vec<ExternalWorkspaceGroup> = Vec::new();
    if !local_projects.is_empty() {
        // Match Codex Desktop 1:1: show exact `local-projects` first, then `最近` (all recent threads)
        for proj in &local_projects {
            let mut convs: Vec<ExternalConversation> = Vec::new();
            for (conv, db_pid) in &all_threads {
                if projectless_ids.contains(&conv.id) {
                    continue;
                }
                let assigned = thread_proj_assign
                    .get(&conv.id)
                    .map(|p| p == &proj.id)
                    .unwrap_or(false)
                    || db_pid == &proj.id;
                let cwd_key = normalize_win_path_key(&conv.cwd);
                let path_match = !cwd_key.is_empty()
                    && proj
                        .root_keys
                        .iter()
                        .any(|rk| cwd_key == *rk || cwd_key.starts_with(&format!("{rk}\\")));
                if assigned || path_match {
                    convs.push(conv.clone());
                }
            }
            convs.sort_by_key(|c| std::cmp::Reverse(c.updated_at));
            let primary_path = proj.root_paths.first().cloned().unwrap_or_default();
            groups.push(ExternalWorkspaceGroup {
                workspace_id: if primary_path.is_empty() {
                    proj.id.clone()
                } else {
                    primary_path.clone()
                },
                label: proj.name.clone(),
                path: primary_path,
                convs,
            });
        }

        if !all_threads.is_empty() {
            let mut recent_convs: Vec<ExternalConversation> =
                all_threads.into_iter().map(|(c, _)| c).collect();
            recent_convs.sort_by_key(|c| std::cmp::Reverse(c.updated_at));
            groups.push(ExternalWorkspaceGroup {
                workspace_id: "__codex_recent__".to_string(),
                label: "最近".to_string(),
                path: String::new(),
                convs: recent_convs,
            });
        }
    } else {
        // Fallback for CLI-only Codex installations without Codex Desktop global state
        let mut by_cwd: BTreeMap<String, Vec<ExternalConversation>> = BTreeMap::new();
        for (conv, _) in all_threads {
            let key = if conv.cwd.is_empty() {
                home.to_string_lossy().to_string()
            } else {
                conv.cwd.clone()
            };
            by_cwd.entry(key).or_default().push(conv);
        }
        groups = by_cwd
            .into_iter()
            .map(|(path, mut convs)| {
                convs.sort_by_key(|c| std::cmp::Reverse(c.updated_at));
                ExternalWorkspaceGroup {
                    workspace_id: path.clone(),
                    label: basename_of_path(&path),
                    path,
                    convs,
                }
            })
            .collect();
        groups.sort_by(|a, b| {
            let a_ts = a.convs.first().map(|c| c.updated_at).unwrap_or(0);
            let b_ts = b.convs.first().map(|c| c.updated_at).unwrap_or(0);
            b_ts.cmp(&a_ts)
        });
    }

    if groups.is_empty() {
        let default_dir = home.to_string_lossy().to_string();
        groups.push(ExternalWorkspaceGroup {
            workspace_id: default_dir.clone(),
            label: basename_of_path(&default_dir),
            path: default_dir,
            convs: Vec::new(),
        });
    }

    ExternalHarnessCatalog {
        harness: "codex".to_string(),
        available: exe.is_some(),
        exe_path: exe.map(|p| p.to_string_lossy().to_string()),
        groups,
        error: None,
    }
}

fn extract_claude_content_text(content: &serde_json::Value) -> String {
    if let Some(s) = content.as_str() {
        return s.trim().to_string();
    }
    let Some(arr) = content.as_array() else {
        return String::new();
    };
    let mut parts: Vec<String> = Vec::new();
    for item in arr {
        if item.get("type").and_then(|t| t.as_str()) != Some("text") {
            continue;
        }
        if let Some(txt) = item.get("text").and_then(|t| t.as_str()) {
            let trimmed = txt.trim();
            if !trimmed.is_empty() {
                parts.push(trimmed.to_string());
            }
        }
    }
    parts.join("\n")
}

fn is_claude_internal_user_text(text: &str) -> bool {
    let t = text.trim();
    t.is_empty()
        || t.starts_with("<local-command-")
        || t.starts_with("<command-name>")
        || t.starts_with("<command-message>")
        || t.starts_with("<system-reminder>")
}

fn file_mtime_ms(path: &Path) -> i64 {
    fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn decode_claude_project_dir_fallback(dir_name: &str) -> String {
    // e.g. "D--AI-Ollama-Claude" -> "D:\AI-Ollama-Claude" (fallback only if jsonl lacks cwd)
    let bytes = dir_name.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b'-'
        && bytes[2] == b'-'
    {
        let drive = (bytes[0] as char).to_ascii_uppercase();
        let rest = &dir_name[3..];
        return format!(r"{drive}:\{rest}");
    }
    dir_name.to_string()
}

fn inspect_claude_session_file(path: &Path, fallback_cwd: &str) -> Option<ExternalConversation> {
    let stem = path.file_stem()?.to_string_lossy().to_string();
    if stem.is_empty() || stem.starts_with("agent-") {
        return None;
    }

    let file = fs::File::open(path).ok()?;
    let reader = BufReader::new(file);

    let mut session_id = stem.clone();
    let mut cwd = String::new();
    let mut custom_title = String::new();
    let mut summary_title = String::new();
    let mut first_user_text = String::new();

    // Read up to 240 lines to extract metadata quickly without blocking on giant transcripts
    for (idx, line) in reader.lines().map_while(Result::ok).enumerate() {
        if idx >= 240 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        if v.get("isSidechain").and_then(|b| b.as_bool()) == Some(true) {
            continue;
        }
        if cwd.is_empty() {
            if let Some(c) = v.get("cwd").and_then(|s| s.as_str()) {
                if !c.trim().is_empty() {
                    cwd = clean_windows_path(c);
                }
            }
        }
        if let Some(sid) = v.get("sessionId").and_then(|s| s.as_str()) {
            if !sid.trim().is_empty() {
                session_id = sid.trim().to_string();
            }
        }
        let kind = v.get("type").and_then(|s| s.as_str()).unwrap_or("");
        match kind {
            "custom-title" => {
                if let Some(t) = v.get("customTitle").and_then(|s| s.as_str()) {
                    if !t.trim().is_empty() {
                        custom_title = t.trim().to_string();
                    }
                }
            }
            "summary" => {
                if summary_title.is_empty() {
                    if let Some(s) = v.get("summary").and_then(|s| s.as_str()) {
                        if !s.trim().is_empty() {
                            summary_title = s.trim().to_string();
                        }
                    }
                }
            }
            "user" if first_user_text.is_empty() => {
                if v.get("isMeta").and_then(|b| b.as_bool()) == Some(true) {
                    continue;
                }
                if let Some(content) = v.get("message").and_then(|m| m.get("content")) {
                    let txt = extract_claude_content_text(content);
                    if !is_claude_internal_user_text(&txt) {
                        first_user_text = txt;
                    }
                }
            }
            _ => {}
        }
    }

    if first_user_text.is_empty() && custom_title.is_empty() && summary_title.is_empty() {
        return None;
    }

    let final_cwd = if cwd.is_empty() {
        fallback_cwd.to_string()
    } else {
        cwd
    };
    let display_title = if !custom_title.is_empty() {
        sanitize_title(&custom_title, "Claude 会话")
    } else if !summary_title.is_empty() {
        sanitize_title(&summary_title, "Claude 会话")
    } else {
        sanitize_title(&first_user_text, "Claude 会话")
    };

    Some(ExternalConversation {
        id: session_id,
        title: display_title,
        updated_at: file_mtime_ms(path),
        source: "claude_code".to_string(),
        cwd: final_cwd,
        rollout_path: Some(path.to_string_lossy().to_string()),
    })
}

pub fn load_claude_code_catalog() -> ExternalHarnessCatalog {
    let exe = resolve_claude_exe();
    let Some(home) = dirs::home_dir() else {
        return ExternalHarnessCatalog {
            harness: "claude_code".to_string(),
            available: exe.is_some(),
            exe_path: exe.map(|p| p.to_string_lossy().to_string()),
            groups: Vec::new(),
            error: Some("无法定位用户目录".to_string()),
        };
    };

    let projects_root = home.join(".claude").join("projects");
    let mut by_cwd: BTreeMap<String, Vec<ExternalConversation>> = BTreeMap::new();

    if let Ok(proj_dirs) = fs::read_dir(&projects_root) {
        let mut session_files: Vec<(i64, PathBuf, String)> = Vec::new();
        for proj in proj_dirs.flatten() {
            let proj_path = proj.path();
            if !proj_path.is_dir() {
                continue;
            }
            let dir_name = proj.file_name().to_string_lossy().to_string();
            let dir_lower = dir_name.to_ascii_lowercase();
            // Skip ephemeral OS temp scratch directories (e.g. AppData\Local\Temp\...)
            if dir_lower.contains("appdata-local-temp-") || dir_lower.contains("windows-temp-") {
                continue;
            }
            let fallback_cwd = decode_claude_project_dir_fallback(&dir_name);
            let Ok(files) = fs::read_dir(&proj_path) else {
                continue;
            };
            for f in files.flatten() {
                let fp = f.path();
                if !fp.is_file() {
                    continue;
                }
                if fp.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                    continue;
                }
                session_files.push((file_mtime_ms(&fp), fp, fallback_cwd.clone()));
            }
        }

        // Only inspect the most recent 160 sessions for snappy popup open
        session_files.sort_by_key(|(ts, _, _)| std::cmp::Reverse(*ts));
        session_files.truncate(160);

        for (_, fp, fallback_cwd) in session_files {
            if let Some(conv) = inspect_claude_session_file(&fp, &fallback_cwd) {
                let key = if conv.cwd.is_empty() {
                    home.to_string_lossy().to_string()
                } else {
                    conv.cwd.clone()
                };
                by_cwd.entry(key).or_default().push(conv);
            }
        }
    }

    let mut groups: Vec<ExternalWorkspaceGroup> = by_cwd
        .into_iter()
        .map(|(path, mut convs)| {
            convs.sort_by_key(|c| std::cmp::Reverse(c.updated_at));
            ExternalWorkspaceGroup {
                workspace_id: path.clone(),
                label: basename_of_path(&path),
                path,
                convs,
            }
        })
        .collect();

    groups.sort_by(|a, b| {
        let a_ts = a.convs.first().map(|c| c.updated_at).unwrap_or(0);
        let b_ts = b.convs.first().map(|c| c.updated_at).unwrap_or(0);
        b_ts.cmp(&a_ts)
    });

    if groups.is_empty() {
        let default_dir = home.to_string_lossy().to_string();
        groups.push(ExternalWorkspaceGroup {
            workspace_id: default_dir.clone(),
            label: basename_of_path(&default_dir),
            path: default_dir,
            convs: Vec::new(),
        });
    }

    ExternalHarnessCatalog {
        harness: "claude_code".to_string(),
        available: exe.is_some(),
        exe_path: exe.map(|p| p.to_string_lossy().to_string()),
        groups,
        error: None,
    }
}

pub fn load_external_messages(harness: HarnessKind, session_id: &str) -> Vec<ExternalChatMessage> {
    if session_id.trim().is_empty() {
        return Vec::new();
    }
    match harness {
        HarnessKind::Dsh => Vec::new(),
        HarnessKind::Antigravity => load_antigravity_transcript(session_id),
        HarnessKind::Codex => load_codex_transcript(session_id),
        HarnessKind::ClaudeCode => load_claude_code_transcript(session_id),
    }
}

fn strip_antigravity_user_wrappers(raw: &str) -> String {
    let mut text = raw.trim().to_string();
    if let (Some(start), Some(end)) = (text.find("<USER_REQUEST>"), text.find("</USER_REQUEST>")) {
        let inner = &text[start + "<USER_REQUEST>".len()..end];
        text = inner.trim().to_string();
    }
    text
}

pub fn parse_antigravity_transcript_lines(raw: &str) -> Vec<ExternalChatMessage> {
    let mut out = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        let step_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let content = v
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .trim();
        if content.is_empty() {
            continue;
        }
        match step_type {
            "USER_INPUT" => {
                let cleaned = strip_antigravity_user_wrappers(content);
                if !cleaned.is_empty() {
                    out.push(ExternalChatMessage {
                        id: format!("ag-u-{idx}"),
                        role: "user".to_string(),
                        text: cleaned,
                        streaming: false,
                    });
                }
            }
            "SYSTEM_MESSAGE" => {
                // Surface silent messages delivered by Anything-to-Prompt via official agentapi
                if content.contains("Anything-to-Prompt") || content.contains("[Message]") {
                    if let Some(idx_content) = content.find("content=") {
                        let body = content[idx_content + "content=".len()..]
                            .trim()
                            .trim_end_matches("</SYSTEM_MESSAGE>")
                            .trim();
                        if !body.is_empty()
                            && !body.starts_with("[BROWSER_HOST_FACTS")
                            && !body.starts_with("Task id ")
                        {
                            out.push(ExternalChatMessage {
                                id: format!("ag-su-{idx}"),
                                role: "user".to_string(),
                                text: body.to_string(),
                                streaming: false,
                            });
                        }
                    }
                }
            }
            "PLANNER_RESPONSE" => {
                out.push(ExternalChatMessage {
                    id: format!("ag-a-{idx}"),
                    role: "assistant".to_string(),
                    text: content.to_string(),
                    streaming: false,
                });
            }
            _ => {}
        }
    }
    if out.len() > 40 {
        out.drain(0..out.len() - 40);
    }
    out
}

fn load_antigravity_transcript(session_id: &str) -> Vec<ExternalChatMessage> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let candidates = [
        home.join(".gemini")
            .join("antigravity")
            .join("brain")
            .join(session_id)
            .join(".system_generated")
            .join("logs")
            .join("transcript.jsonl"),
        home.join(".gemini")
            .join("antigravity-cli")
            .join("brain")
            .join(session_id)
            .join(".system_generated")
            .join("logs")
            .join("transcript.jsonl"),
    ];
    for path in candidates {
        if let Ok(raw) = fs::read_to_string(&path) {
            return parse_antigravity_transcript_lines(&raw);
        }
    }
    Vec::new()
}

pub fn parse_codex_rollout_lines(raw: &str) -> Vec<ExternalChatMessage> {
    let mut out = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("response_item") {
            continue;
        }
        let Some(payload) = v.get("payload") else {
            continue;
        };
        if payload.get("type").and_then(|t| t.as_str()) != Some("message") {
            continue;
        }
        let role = payload.get("role").and_then(|r| r.as_str()).unwrap_or("");
        if role != "user" && role != "assistant" {
            continue;
        }
        let mut text_parts = Vec::new();
        if let Some(arr) = payload.get("content").and_then(|c| c.as_array()) {
            for item in arr {
                if let Some(t) = item.get("text").and_then(|s| s.as_str()) {
                    if !t.trim().is_empty() {
                        text_parts.push(t.trim());
                    }
                }
            }
        }
        let text = text_parts.join("\n").trim().to_string();
        if text.is_empty() {
            continue;
        }
        let id = payload
            .get("id")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("cx-{role}-{idx}"));
        out.push(ExternalChatMessage {
            id,
            role: role.to_string(),
            text,
            streaming: false,
        });
    }
    if out.len() > 40 {
        out.drain(0..out.len() - 40);
    }
    out
}

fn load_codex_transcript(session_id: &str) -> Vec<ExternalChatMessage> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let db_path = home.join(".codex").join("state_5.sqlite");
    let Some(conn) = open_sqlite_readonly(&db_path) else {
        return Vec::new();
    };
    let rollout_path: Option<String> = conn
        .query_row(
            "SELECT rollout_path FROM threads WHERE id = ?1 LIMIT 1",
            [session_id],
            |row| row.get(0),
        )
        .ok();
    let Some(path_str) = rollout_path else {
        return Vec::new();
    };
    let cleaned = clean_windows_path(&path_str);
    if let Ok(raw) = fs::read_to_string(&cleaned) {
        return parse_codex_rollout_lines(&raw);
    }
    Vec::new()
}

pub fn parse_claude_code_transcript_lines(raw: &str) -> Vec<ExternalChatMessage> {
    let mut out: Vec<ExternalChatMessage> = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        if v.get("isSidechain").and_then(|b| b.as_bool()) == Some(true)
            || v.get("isMeta").and_then(|b| b.as_bool()) == Some(true)
            || v.get("isApiErrorMessage").and_then(|b| b.as_bool()) == Some(true)
        {
            continue;
        }
        let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if kind != "user" && kind != "assistant" {
            continue;
        }
        let Some(msg) = v.get("message") else {
            continue;
        };
        let Some(content) = msg.get("content") else {
            continue;
        };
        let text = extract_claude_content_text(content);
        if text.is_empty() {
            continue;
        }
        if kind == "user" && is_claude_internal_user_text(&text) {
            continue;
        }
        // Merge consecutive assistant text blocks from the same assistant message id
        if kind == "assistant" {
            if let Some(last) = out.last_mut() {
                if last.role == "assistant" {
                    if let Some(msg_id) = msg.get("id").and_then(|s| s.as_str()) {
                        if !msg_id.is_empty() && last.id == format!("cc-{msg_id}") {
                            if !last.text.ends_with(&text) {
                                last.text.push_str("\n\n");
                                last.text.push_str(&text);
                            }
                            continue;
                        }
                    }
                }
            }
        }
        let id = msg
            .get("id")
            .and_then(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| format!("cc-{s}"))
            .or_else(|| {
                v.get("uuid")
                    .and_then(|s| s.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| format!("cc-{s}"))
            })
            .unwrap_or_else(|| format!("cc-{kind}-{idx}"));
        out.push(ExternalChatMessage {
            id,
            role: kind.to_string(),
            text,
            streaming: false,
        });
    }
    if out.len() > 40 {
        out.drain(0..out.len() - 40);
    }
    out
}

fn load_claude_code_transcript(session_id: &str) -> Vec<ExternalChatMessage> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let projects_root = home.join(".claude").join("projects");
    let target_name = format!("{}.jsonl", session_id.trim());
    let Ok(proj_dirs) = fs::read_dir(&projects_root) else {
        return Vec::new();
    };
    for proj in proj_dirs.flatten() {
        let candidate = proj.path().join(&target_name);
        if candidate.is_file() {
            if let Ok(raw) = fs::read_to_string(&candidate) {
                return parse_claude_code_transcript_lines(&raw);
            }
        }
    }
    Vec::new()
}

fn emit_stream_event(writer: &mut impl Write, evt: &HarnessStreamEvent) {
    if let Ok(line) = serde_json::to_string(evt) {
        let _ = writeln!(writer, "{line}");
        let _ = writer.flush();
    }
}

pub fn parse_claude_session_stats_lines(session_id: &str, raw: &str) -> HarnessSessionStats {
    let mut model = String::new();
    let mut turns = 0usize;
    let mut input_tokens = 0u64;
    let mut output_tokens = 0u64;
    let mut cache_read_tokens = 0u64;
    let mut cache_creation_tokens = 0u64;
    let mut seen_msg_ids = HashSet::new();

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
            continue;
        }
        let Some(msg) = v.get("message") else {
            continue;
        };
        if let Some(m) = msg.get("model").and_then(|s| s.as_str()) {
            if !m.trim().is_empty() {
                model = m.trim().to_string();
            }
        }
        if let Some(mid) = msg.get("id").and_then(|s| s.as_str()) {
            if !mid.is_empty() && !seen_msg_ids.insert(mid.to_string()) {
                continue;
            }
        }
        turns += 1;
        if let Some(usage) = msg.get("usage") {
            input_tokens += usage.get("input_tokens").and_then(|n| n.as_u64()).unwrap_or(0);
            output_tokens += usage.get("output_tokens").and_then(|n| n.as_u64()).unwrap_or(0);
            cache_read_tokens += usage
                .get("cache_read_input_tokens")
                .and_then(|n| n.as_u64())
                .unwrap_or(0);
            cache_creation_tokens += usage
                .get("cache_creation_input_tokens")
                .and_then(|n| n.as_u64())
                .unwrap_or(0);
        }
    }

    let total_tokens = input_tokens + output_tokens + cache_read_tokens + cache_creation_tokens;
    let model_disp = if model.is_empty() {
        "Claude Code".to_string()
    } else {
        format_claude_model_label(&model)
    };
    let summary_markdown = format!(
        "### 📊 Claude Code 会话用量（`/cost`）\n\n\
        - **模型**：`{model_disp}`\n\
        - **对话轮次**：`{turns}` 轮\n\
        - **输入 / 输出 Tokens**：`{input_tokens}` in / `{output_tokens}` out\n\
        - **缓存读取 / 写入**：`{cache_read_tokens}` read / `{cache_creation_tokens}` write\n\
        - **合计 Tokens**：**`{total_tokens}`**"
    );

    HarnessSessionStats {
        harness: "claude_code".to_string(),
        session_id: session_id.to_string(),
        model: model_disp,
        turns,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_creation_tokens,
        total_tokens,
        summary_markdown,
    }
}

pub fn load_session_stats(kind: HarnessKind, session_id: &str) -> HarnessSessionStats {
    let sid = session_id.trim();
    let empty = |h: &str, msg: &str| HarnessSessionStats {
        harness: h.to_string(),
        session_id: sid.to_string(),
        model: String::new(),
        turns: 0,
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        cache_creation_tokens: 0,
        total_tokens: 0,
        summary_markdown: msg.to_string(),
    };
    if sid.is_empty() {
        return empty(
            kind.as_str(),
            "当前为全新对话，尚未产生会话 Token 消耗记录。",
        );
    }
    let Some(home) = dirs::home_dir() else {
        return empty(kind.as_str(), "无法定位本地会话存储目录。");
    };
    match kind {
        HarnessKind::ClaudeCode => {
            let projects_root = home.join(".claude").join("projects");
            let target_name = format!("{sid}.jsonl");
            if let Ok(proj_dirs) = fs::read_dir(&projects_root) {
                for proj in proj_dirs.flatten() {
                    let candidate = proj.path().join(&target_name);
                    if candidate.is_file() {
                        if let Ok(raw) = fs::read_to_string(&candidate) {
                            return parse_claude_session_stats_lines(sid, &raw);
                        }
                    }
                }
            }
            empty("claude_code", "未找到该 Claude Code 会话的本地 JSONL 用量日志。")
        }
        HarnessKind::Codex => {
            let db_path = home.join(".codex").join("state_5.sqlite");
            if let Some(conn) = open_sqlite_readonly(&db_path) {
                if let Ok((tokens, provider)) = conn.query_row(
                    "SELECT COALESCE(tokens_used, 0), COALESCE(model_provider, 'openai') FROM threads WHERE id = ?1 LIMIT 1",
                    [sid],
                    |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
                ) {
                    let total = tokens.max(0) as u64;
                    let turns = load_codex_transcript(sid)
                        .into_iter()
                        .filter(|m| m.role == "assistant")
                        .count();
                    return HarnessSessionStats {
                        harness: "codex".to_string(),
                        session_id: sid.to_string(),
                        model: provider.clone(),
                        turns,
                        input_tokens: 0,
                        output_tokens: 0,
                        cache_read_tokens: 0,
                        cache_creation_tokens: 0,
                        total_tokens: total,
                        summary_markdown: format!(
                            "### 📊 Codex 会话用量\n\n- **Provider**：`{provider}`\n- **回复轮次**：`{turns}` 轮\n- **累计 Tokens**：**`{total}`**"
                        ),
                    };
                }
            }
            empty("codex", "未找到该 Codex 会话的本地统计记录。")
        }
        HarnessKind::Antigravity => {
            let msgs = load_antigravity_transcript(sid);
            let turns = msgs.iter().filter(|m| m.role == "assistant").count();
            HarnessSessionStats {
                harness: "antigravity".to_string(),
                session_id: sid.to_string(),
                model: "Antigravity".to_string(),
                turns,
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                total_tokens: 0,
                summary_markdown: format!(
                    "### 📊 Antigravity 会话统计\n\n- **会话 ID**：`{sid}`\n- **回复轮次**：`{turns}` 轮"
                ),
            }
        }
        HarnessKind::Dsh => empty("dsh", "DeepSeek Harness 请直接在原对话界面查看统计。"),
    }
}

fn read_file_tail_utf8(path: &Path, max_bytes: usize) -> Option<String> {
    use std::io::{Read as _, Seek as _, SeekFrom};
    let mut file = fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let take = (len as usize).min(max_bytes);
    if len > take as u64 {
        file.seek(SeekFrom::End(-(take as i64))).ok()?;
    }
    let mut buf = Vec::with_capacity(take);
    file.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf).into_owned();
    if len > take as u64 {
        if let Some(pos) = text.find('\n') {
            return Some(text[pos + 1..].to_string());
        }
    }
    Some(text)
}

pub fn parse_antigravity_model_and_effort(raw_model: &str) -> (Option<String>, Option<String>) {
    let trimmed = raw_model.trim();
    if trimmed.is_empty() {
        return (None, None);
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("gemini-pro-orchestrator") {
        return (Some(trimmed.to_string()), None);
    }
    if lower.starts_with("gpt-oss-") {
        let effort = if lower.ends_with("-low") {
            Some("low".to_string())
        } else if lower.ends_with("-high") {
            Some("high".to_string())
        } else {
            Some("medium".to_string())
        };
        return (Some(trimmed.to_string()), effort);
    }

    let (base, effort) = if let Some(b) = lower.strip_suffix("-xhigh") {
        (b, Some("xhigh".to_string()))
    } else if let Some(b) = lower.strip_suffix("-high") {
        (b, Some("high".to_string()))
    } else if let Some(b) = lower.strip_suffix("-medium") {
        (b, Some("medium".to_string()))
    } else if let Some(b) = lower.strip_suffix("-low") {
        (b, Some("low".to_string()))
    } else {
        (lower.as_str(), None)
    };

    // Map family base to canonical option ID used in harnesses/antigravity/ui.js
    let canonical = match base {
        "gemini-4-neon" => "gemini-4-neon-high".to_string(),
        "gemini-4-carbon" => "gemini-4-carbon-medium".to_string(),
        "gemini-4-argon" => "gemini-4-argon-medium".to_string(),
        "gemini-4-hydrogen" => "gemini-4-hydrogen-medium".to_string(),
        "gemini-3.8-flash" => "gemini-3.8-flash-medium".to_string(),
        "claude-opus-5-5" => "claude-opus-5-5-medium".to_string(),
        "claude-sonnet-5-5" => "claude-sonnet-5-5-medium".to_string(),
        _ => trimmed.to_string(),
    };
    (Some(canonical), effort)
}

pub fn parse_antigravity_permission_preset(pbtxt: &str) -> Option<String> {
    let pos = pbtxt.find("permission_preset:")?;
    let rest = pbtxt[pos + "permission_preset:".len()..].trim_start();
    let token: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    let upper = token.to_ascii_uppercase();
    if upper.is_empty() {
        return None;
    }
    if upper.contains("TURBO") {
        Some("turbo".to_string())
    } else if upper.contains("AUTO") {
        Some("auto".to_string())
    } else if upper.contains("REVIEW") || upper.contains("PLAN") {
        Some("request-review".to_string())
    } else if upper.contains("DEFAULT") || upper.contains("ASK") {
        Some("default".to_string())
    } else {
        None
    }
}

pub fn parse_antigravity_latest_model_from_transcript(raw_tail: &str) -> Option<String> {
    for line in raw_tail.lines().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() || !trimmed.contains("\"model\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        if let Some(m) = v.get("model").and_then(|s| s.as_str()) {
            let clean = m.trim();
            if !clean.is_empty() && clean != "<synthetic>" {
                return Some(clean.to_string());
            }
        }
    }
    None
}

fn map_codex_permission_mode(
    approval: Option<&str>,
    sandbox: Option<&str>,
    reviewer: Option<&str>,
) -> Option<String> {
    let sb = sandbox.unwrap_or("").to_ascii_lowercase();
    let ap = approval.unwrap_or("").trim().to_ascii_lowercase();
    let rv = reviewer.unwrap_or("").trim().to_ascii_lowercase();
    if sb.contains("read-only") || sb.contains("readonly") {
        return Some("read-only".to_string());
    }
    if ap == "never"
        || sb.contains("danger-full-access")
        || sb.contains("dangerfullaccess")
        || sb.contains("\"disabled\"")
        || sb == "disabled"
    {
        return Some("danger-full-access".to_string());
    }
    if rv == "auto_review" || rv == "auto-review" {
        return Some("help-approve".to_string());
    }
    if ap == "on-request" || sb.contains("workspace-write") || sb.contains("workspacewrite") {
        return Some("request-approval".to_string());
    }
    None
}

pub fn parse_codex_session_state_from_turn_context(
    raw_tail: &str,
    db_model: Option<&str>,
    db_effort: Option<&str>,
    db_approval: Option<&str>,
    db_sandbox: Option<&str>,
) -> (Option<String>, Option<String>, Option<String>, Option<bool>) {
    let mut model = db_model
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let mut effort = db_effort
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty());
    let mut approval = db_approval
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let mut sandbox = db_sandbox
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let mut reviewer: Option<String> = None;
    let mut plan_mode: Option<bool> = None;

    for line in raw_tail.lines().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() || !trimmed.contains("\"turn_context\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("turn_context") {
            continue;
        }
        let Some(p) = v.get("payload") else {
            continue;
        };
        if let Some(m) = p.get("model").and_then(|s| s.as_str()).filter(|s| !s.trim().is_empty()) {
            model = Some(m.trim().to_string());
        }
        if let Some(e) = p
            .get("effort")
            .or_else(|| p.pointer("/collaboration_mode/settings/reasoning_effort"))
            .and_then(|s| s.as_str())
            .filter(|s| !s.trim().is_empty())
        {
            effort = Some(e.trim().to_ascii_lowercase());
        }
        if let Some(ap) = p.get("approval_policy").and_then(|s| s.as_str()).filter(|s| !s.trim().is_empty()) {
            approval = Some(ap.trim().to_string());
        }
        if let Some(rv) = p.get("approvals_reviewer").and_then(|s| s.as_str()).filter(|s| !s.trim().is_empty()) {
            reviewer = Some(rv.trim().to_string());
        }
        if let Some(sb_val) = p.get("sandbox_policy") {
            if let Some(s) = sb_val.as_str() {
                sandbox = Some(s.to_string());
            } else if let Some(t) = sb_val.get("type").and_then(|x| x.as_str()) {
                sandbox = Some(t.to_string());
            } else {
                sandbox = Some(sb_val.to_string());
            }
        }
        if let Some(cm) = p.pointer("/collaboration_mode/mode").and_then(|s| s.as_str()) {
            plan_mode = Some(cm.eq_ignore_ascii_case("plan"));
        }
        break;
    }

    let mode = map_codex_permission_mode(approval.as_deref(), sandbox.as_deref(), reviewer.as_deref());
    (model, effort, mode, plan_mode)
}

fn normalize_claude_model_id(raw: &str) -> Option<String> {
    let clean = raw.split('[').next().unwrap_or(raw).trim();
    if clean.is_empty() || clean == "<synthetic>" {
        return None;
    }
    let parts: Vec<&str> = clean.split('-').filter(|p| !p.is_empty()).collect();
    if parts.len() >= 2 {
        if let Some(last) = parts.last() {
            if last.len() == 8 && last.chars().all(|c| c.is_ascii_digit()) {
                return Some(parts[..parts.len() - 1].join("-"));
            }
        }
    }
    Some(clean.to_string())
}

pub fn parse_claude_session_state_from_jsonl(raw_tail: &str) -> (Option<String>, Option<String>) {
    let mut model: Option<String> = None;
    let mut mode: Option<String> = None;
    for line in raw_tail.lines().rev() {
        if model.is_some() && mode.is_some() {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        if v.get("isSidechain").and_then(|b| b.as_bool()) == Some(true) {
            continue;
        }
        let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if mode.is_none() && kind == "user" {
            if let Some(pm) = v.get("permissionMode").and_then(|s| s.as_str()) {
                let mapped = match pm.trim() {
                    "default" | "manual" => Some("manual"),
                    "auto" => Some("auto"),
                    "acceptEdits" => Some("acceptEdits"),
                    "plan" => Some("plan"),
                    "dontAsk" => Some("dontAsk"),
                    "bypassPermissions" => Some("bypassPermissions"),
                    _ => None,
                };
                if let Some(m) = mapped {
                    mode = Some(m.to_string());
                }
            }
        }
        if model.is_none() && kind == "assistant" {
            if let Some(raw_m) = v.pointer("/message/model").and_then(|s| s.as_str()) {
                model = normalize_claude_model_id(raw_m);
            }
        }
    }
    (model, mode)
}

/// Strictly read-only per-conversation state lookup for the bound external Harness session.
/// Never writes to any Harness config, SQLite DB, or client state.
pub fn load_session_state(kind: HarnessKind, session_id: &str) -> HarnessSessionState {
    let sid = session_id.trim();
    let mut out = HarnessSessionState {
        harness: kind.as_str().to_string(),
        session_id: sid.to_string(),
        ..Default::default()
    };
    if sid.is_empty() || kind == HarnessKind::Dsh {
        return out;
    }
    let Some(home) = dirs::home_dir() else {
        return out;
    };

    match kind {
        HarnessKind::Codex => {
            let db_path = home.join(".codex").join("state_5.sqlite");
            let mut db_model = None;
            let mut db_effort = None;
            let mut db_approval = None;
            let mut db_sandbox = None;
            let mut rollout_path = None;
            if let Some(conn) = open_sqlite_readonly(&db_path) {
                if let Ok((m, e, ap, sb, rp)) = conn.query_row(
                    "SELECT COALESCE(model, ''), COALESCE(reasoning_effort, ''), \
                     COALESCE(approval_mode, ''), COALESCE(sandbox_policy, ''), COALESCE(rollout_path, '') \
                     FROM threads WHERE id = ?1 LIMIT 1",
                    [sid],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, String>(3)?,
                            r.get::<_, String>(4)?,
                        ))
                    },
                ) {
                    if !m.trim().is_empty() {
                        db_model = Some(m);
                    }
                    if !e.trim().is_empty() {
                        db_effort = Some(e);
                    }
                    if !ap.trim().is_empty() {
                        db_approval = Some(ap);
                    }
                    if !sb.trim().is_empty() {
                        db_sandbox = Some(sb);
                    }
                    if !rp.trim().is_empty() {
                        rollout_path = Some(clean_windows_path(&rp));
                    }
                }
            }
            let raw_tail = rollout_path
                .as_deref()
                .and_then(|p| read_file_tail_utf8(Path::new(p), 131_072))
                .unwrap_or_default();
            let (model, effort, mode, plan_mode) = parse_codex_session_state_from_turn_context(
                &raw_tail,
                db_model.as_deref(),
                db_effort.as_deref(),
                db_approval.as_deref(),
                db_sandbox.as_deref(),
            );
            out.model = model;
            out.effort = effort;
            out.mode = mode;
            out.plan_mode = plan_mode;
            if out.model.is_some() || out.effort.is_some() || out.mode.is_some() {
                out.source_note = "已同步 Codex 当前对话（发消息时生效并同步回对话）".to_string();
            }
        }
        HarnessKind::ClaudeCode => {
            let projects_root = home.join(".claude").join("projects");
            let target_name = format!("{sid}.jsonl");
            if let Ok(proj_dirs) = fs::read_dir(&projects_root) {
                for proj in proj_dirs.flatten() {
                    let candidate = proj.path().join(&target_name);
                    if candidate.is_file() {
                        if let Some(raw_tail) = read_file_tail_utf8(&candidate, 131_072) {
                            let (model, mode) = parse_claude_session_state_from_jsonl(&raw_tail);
                            out.model = model;
                            out.mode = mode;
                        }
                        break;
                    }
                }
            }
            let claude_dir = home.join(".claude");
            for file_name in ["settings.local.json", "settings.json"] {
                let Ok(text) = fs::read_to_string(claude_dir.join(file_name)) else {
                    continue;
                };
                let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
                    continue;
                };
                if out.effort.is_none() {
                    if let Some(e) = v
                        .get("effortLevel")
                        .or_else(|| v.get("effort"))
                        .and_then(|s| s.as_str())
                        .filter(|s| !s.trim().is_empty())
                    {
                        out.effort = Some(e.trim().to_ascii_lowercase());
                    }
                }
                if out.model.is_none() {
                    if let Some(m) = v.get("model").and_then(|s| s.as_str()) {
                        out.model = normalize_claude_model_id(m);
                    }
                }
                if out.mode.is_none() {
                    if let Some(p) = v.pointer("/permissions/defaultMode").and_then(|s| s.as_str()) {
                        out.mode = match p.trim() {
                            "default" | "manual" => Some("manual".to_string()),
                            "auto" | "acceptEdits" | "plan" | "dontAsk" | "bypassPermissions" => {
                                Some(p.trim().to_string())
                            }
                            _ => None,
                        };
                    }
                }
            }
            if out.model.is_some() || out.effort.is_some() || out.mode.is_some() {
                out.source_note = "已同步 Claude Code 当前对话（发消息时生效并同步回对话）".to_string();
            }
        }
        HarnessKind::Antigravity => {
            let is_cli = is_antigravity_cli_conversation(sid);
            out.read_only_in_desktop = !is_cli;
            let anno_path = home
                .join(".gemini")
                .join("antigravity")
                .join("annotations")
                .join(format!("{sid}.pbtxt"));
            if let Ok(pbtxt) = fs::read_to_string(&anno_path) {
                out.mode = parse_antigravity_permission_preset(&pbtxt);
            }
            let transcript_candidates = [
                home.join(".gemini")
                    .join("antigravity")
                    .join("brain")
                    .join(sid)
                    .join(".system_generated")
                    .join("logs")
                    .join("transcript.jsonl"),
                home.join(".gemini")
                    .join("antigravity-cli")
                    .join("brain")
                    .join(sid)
                    .join(".system_generated")
                    .join("logs")
                    .join("transcript.jsonl"),
            ];
            for tp in transcript_candidates {
                if let Some(raw_tail) = read_file_tail_utf8(&tp, 131_072) {
                    if let Some(raw_model) = parse_antigravity_latest_model_from_transcript(&raw_tail) {
                        let (canon_model, eff) = parse_antigravity_model_and_effort(&raw_model);
                        out.model = canon_model;
                        out.effort = eff;
                        break;
                    }
                }
            }
            out.source_note = if out.read_only_in_desktop {
                "当前绑定 Antigravity 桌面端对话：模型与权限以桌面端为准（新建或 CLI 对话可在此修改）".to_string()
            } else {
                "已同步 Antigravity 当前对话设置".to_string()
            };
        }
        HarnessKind::Dsh => {}
    }
    out
}

#[derive(Debug, Clone)]
pub struct MediumTurnParams {
    pub turn_id: String,
    pub harness: String,
    pub exe: String,
    pub acp_args: Vec<String>,
    pub cwd: Option<String>,
    pub project_id: Option<String>,
    pub session_id: Option<String>,
    pub permission_mode: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub sandbox_mode: Option<String>,
    pub execution_mode: Option<String>,
    pub fast_speed: Option<bool>,
    pub plan_mode: Option<bool>,
    pub sandbox_enabled: Option<bool>,
    pub chrome_enabled: Option<bool>,
    pub enabled_plugins: Vec<String>,
    pub attachments: Vec<TurnAttachment>,
    pub prompt: String,
    pub out_pipe: String,
}

pub(crate) fn build_prompt_with_file_attachments(
    prompt: &str,
    attachments: &[TurnAttachment],
    include_images_as_paths: bool,
) -> String {
    let mut lines = Vec::new();
    for att in attachments {
        if att.path.trim().is_empty() {
            continue;
        }
        if att.kind == "image" && !include_images_as_paths {
            continue;
        }
        let label = match att.kind.as_str() {
            "image" => "附加图片",
            "folder" => "附加文件夹",
            _ => "附加文件",
        };
        lines.push(format!("[{label}: {}]", att.path.trim()));
    }
    if lines.is_empty() {
        prompt.to_string()
    } else if prompt.trim().is_empty() {
        lines.join("\n")
    } else {
        format!("{}\n\n{}", lines.join("\n"), prompt)
    }
}

/// Runs strictly inside the medium-integrity broker process (or directly when app is already medium).
pub fn execute_harness_turn_in_medium(params: MediumTurnParams) {
    let full_pipe = if params.out_pipe.starts_with(r"\\.\pipe\") {
        params.out_pipe.clone()
    } else {
        format!(r"\\.\pipe\{}", params.out_pipe)
    };

    let Ok(mut pipe_writer) = fs::OpenOptions::new().write(true).open(&full_pipe) else {
        log::warn!("Medium harness worker failed to open stream pipe {full_pipe}");
        return;
    };

    if crate::acp::is_acp_harness_id(&params.harness) {
        let mode_choice = params
            .execution_mode
            .as_deref()
            .or(params.permission_mode.as_deref())
            .or(params.sandbox_mode.as_deref());
        crate::acp::run_acp_turn_in_medium(
            &params.turn_id,
            &params.harness,
            &params.exe,
            &params.acp_args,
            params.cwd.as_deref(),
            params.session_id.as_deref(),
            params.model.as_deref(),
            params.effort.as_deref(),
            mode_choice,
            &params.attachments,
            &params.prompt,
            &mut pipe_writer,
        );
        unregister_turn_pid(&params.turn_id);
        return;
    }

    match HarnessKind::parse(&params.harness) {
        HarnessKind::Antigravity => {
            run_antigravity_turn_in_medium(
                &params.turn_id,
                &params.exe,
                params.cwd.as_deref(),
                params.project_id.as_deref(),
                params.session_id.as_deref(),
                params.model.as_deref(),
                params.effort.as_deref(),
                params.execution_mode.as_deref(),
                params.plan_mode,
                params.sandbox_enabled,
                &params.attachments,
                &params.prompt,
                &mut pipe_writer,
            );
        }
        HarnessKind::Codex => {
            run_codex_app_server_turn_in_medium(
                &params.turn_id,
                &params.exe,
                params.cwd.as_deref(),
                params.session_id.as_deref(),
                params.model.as_deref(),
                params.effort.as_deref(),
                params.sandbox_mode.as_deref(),
                params.fast_speed,
                params.plan_mode,
                &params.enabled_plugins,
                &params.attachments,
                &params.prompt,
                &mut pipe_writer,
            );
        }
        HarnessKind::ClaudeCode => {
            run_claude_code_turn_in_medium(
                &params.turn_id,
                &params.exe,
                params.cwd.as_deref(),
                params.session_id.as_deref(),
                params.permission_mode.as_deref(),
                params.model.as_deref(),
                params.effort.as_deref(),
                params.plan_mode,
                params.chrome_enabled,
                &params.attachments,
                &params.prompt,
                &mut pipe_writer,
            );
        }
        HarnessKind::Dsh => {
            emit_stream_event(
                &mut pipe_writer,
                &HarnessStreamEvent {
                    turn_id: params.turn_id.clone(),
                    harness: "dsh".to_string(),
                    kind: "error".to_string(),
                    session_id: None,
                    text: None,
                    tool_name: None,
                    error: Some("DSH 使用原生 WebView 发送通道".to_string()),
                },
            );
        }
    }
    unregister_turn_pid(&params.turn_id);
}

fn try_run_antigravity_via_desktop_sidecar(
    turn_id: &str,
    session_id: Option<&str>,
    model: Option<&str>,
    prompt: &str,
    pipe_writer: &mut impl Write,
) -> bool {
    let cancel_flag = register_sidecar_cancel(turn_id);
    let target_sid = session_id.map(|s| s.trim()).filter(|s| !s.is_empty());
    let home = dirs::home_dir();
    let baseline_assistant_count = match (&home, target_sid) {
        (Some(h), Some(sid)) => {
            let tp = h
                .join(".gemini")
                .join("antigravity")
                .join("brain")
                .join(sid)
                .join(".system_generated")
                .join("logs")
                .join("transcript.jsonl");
            fs::read_to_string(&tp)
                .ok()
                .map(|raw| {
                    parse_antigravity_transcript_lines(&raw)
                        .into_iter()
                        .filter(|m| m.role == "assistant")
                        .count()
                })
                .unwrap_or(0)
        }
        _ => 0,
    };

    let req = if let Some(sid) = target_sid {
        crate::harness_sync::AntigravitySidecarRequest {
            action: "send_message".to_string(),
            conversation_id: Some(sid.to_string()),
            prompt: Some(prompt.to_string()),
            title: Some("Anything-to-Prompt".to_string()),
            model: None,
        }
    } else {
        crate::harness_sync::AntigravitySidecarRequest {
            action: "new_conversation".to_string(),
            conversation_id: None,
            prompt: Some(prompt.to_string()),
            title: None,
            model: model.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
        }
    };

    let resp = match crate::harness_sync::call_antigravity_sidecar(&req) {
        Ok(r) => r,
        Err(e) if e.starts_with("SIDE_OFFLINE:") => {
            // Antigravity Desktop is closed or sidecar is still booting: silently launch it in the
            // background (minimized without activating!) and wait for the sidecar pipe to connect.
            let mut woken: Option<crate::harness_sync::AntigravitySidecarResponse> = None;
            if let Some(exe) = resolve_antigravity_desktop_exe() {
                emit_stream_event(
                    pipe_writer,
                    &HarnessStreamEvent {
                        turn_id: turn_id.to_string(),
                        harness: "antigravity".to_string(),
                        kind: "tool".to_string(),
                        session_id: target_sid.map(|s| s.to_string()),
                        text: None,
                        tool_name: Some("正在后台静默启动 Antigravity…".to_string()),
                        error: None,
                    },
                );
                if !is_process_name_running("antigravity.exe") {
                    let _ = launch_desktop_app_silently(&exe, "antigravity.exe");
                }
                let deadline = Instant::now() + Duration::from_secs(25);
                while Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(600));
                    match crate::harness_sync::call_antigravity_sidecar(&req) {
                        Ok(r) => {
                            woken = Some(r);
                            break;
                        }
                        Err(err2) if err2.starts_with("SIDE_OFFLINE:") => continue,
                        Err(err2) => {
                            emit_stream_event(
                                pipe_writer,
                                &HarnessStreamEvent {
                                    turn_id: turn_id.to_string(),
                                    harness: "antigravity".to_string(),
                                    kind: "error".to_string(),
                                    session_id: target_sid.map(|s| s.to_string()),
                                    text: None,
                                    tool_name: None,
                                    error: Some(format!("Antigravity Sidecar 通信失败: {err2}")),
                                },
                            );
                            return true;
                        }
                    }
                }
            }
            if let Some(r) = woken {
                r
            } else {
                emit_stream_event(
                    pipe_writer,
                    &HarnessStreamEvent {
                        turn_id: turn_id.to_string(),
                        harness: "antigravity".to_string(),
                        kind: "error".to_string(),
                        session_id: target_sid.map(|s| s.to_string()),
                        text: None,
                        tool_name: None,
                        error: Some(
                            "Antigravity 后台启动超时，Sidecar 尚未就绪：可点击抽屉右上角按钮拉起 Antigravity 桌面端后重试。"
                                .to_string(),
                        ),
                    },
                );
                return true;
            }
        }
        Err(e) => {
            emit_stream_event(
                pipe_writer,
                &HarnessStreamEvent {
                    turn_id: turn_id.to_string(),
                    harness: "antigravity".to_string(),
                    kind: "error".to_string(),
                    session_id: target_sid.map(|s| s.to_string()),
                    text: None,
                    tool_name: None,
                    error: Some(format!("Antigravity Sidecar 通信失败: {e}")),
                },
            );
            return true;
        }
    };

    if !resp.ok {
        emit_stream_event(
            pipe_writer,
            &HarnessStreamEvent {
                turn_id: turn_id.to_string(),
                harness: "antigravity".to_string(),
                kind: "error".to_string(),
                session_id: resp.conversation_id.or_else(|| target_sid.map(|s| s.to_string())),
                text: None,
                tool_name: None,
                error: Some(
                    resp.error
                        .unwrap_or_else(|| "Antigravity 桌面端投递失败".to_string()),
                ),
            },
        );
        return true;
    }

    let resolved_sid = resp
        .conversation_id
        .or_else(|| target_sid.map(|s| s.to_string()));
    if let Some(ref sid) = resolved_sid {
        emit_stream_event(
            pipe_writer,
            &HarnessStreamEvent {
                turn_id: turn_id.to_string(),
                harness: "antigravity".to_string(),
                kind: "session".to_string(),
                session_id: Some(sid.clone()),
                text: None,
                tool_name: None,
                error: None,
            },
        );
    }
    emit_stream_event(
        pipe_writer,
        &HarnessStreamEvent {
            turn_id: turn_id.to_string(),
            harness: "antigravity".to_string(),
            kind: "tool".to_string(),
            session_id: resolved_sid.clone(),
            text: None,
            tool_name: Some("已静默投递至 Antigravity 桌面端，等待回复".to_string()),
            error: None,
        },
    );

    // Tail transcript.jsonl silently in the background until Antigravity writes a new PLANNER_RESPONSE
    if let (Some(h), Some(ref sid)) = (home, &resolved_sid) {
        let tp = h
            .join(".gemini")
            .join("antigravity")
            .join("brain")
            .join(sid)
            .join(".system_generated")
            .join("logs")
            .join("transcript.jsonl");
        let start = std::time::Instant::now();
        let mut last_reply = String::new();
        let mut last_change_at: Option<std::time::Instant> = None;

        while start.elapsed() < std::time::Duration::from_secs(180) {
            if cancel_flag.load(Ordering::Acquire) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(350));
            if let Ok(raw) = fs::read_to_string(&tp) {
                let assts: Vec<ExternalChatMessage> = parse_antigravity_transcript_lines(&raw)
                    .into_iter()
                    .filter(|m| m.role == "assistant")
                    .collect();
                if assts.len() > baseline_assistant_count {
                    if let Some(newest) = assts.last() {
                        if newest.text != last_reply && !newest.text.trim().is_empty() {
                            last_reply = newest.text.clone();
                            last_change_at = Some(std::time::Instant::now());
                            emit_stream_event(
                                pipe_writer,
                                &HarnessStreamEvent {
                                    turn_id: turn_id.to_string(),
                                    harness: "antigravity".to_string(),
                                    kind: "replace".to_string(),
                                    session_id: Some(sid.clone()),
                                    text: Some(last_reply.clone()),
                                    tool_name: None,
                                    error: None,
                                },
                            );
                        }
                    }
                }
            }
            // Once a reply has arrived and remained stable for 1.5s, complete the turn cleanly
            if let Some(changed) = last_change_at {
                if changed.elapsed() >= std::time::Duration::from_millis(1500) {
                    emit_stream_event(
                        pipe_writer,
                        &HarnessStreamEvent {
                            turn_id: turn_id.to_string(),
                            harness: "antigravity".to_string(),
                            kind: "completed".to_string(),
                            session_id: Some(sid.clone()),
                            text: Some(last_reply),
                            tool_name: None,
                            error: None,
                        },
                    );
                    return true;
                }
            }
        }
    }

    emit_stream_event(
        pipe_writer,
        &HarnessStreamEvent {
            turn_id: turn_id.to_string(),
            harness: "antigravity".to_string(),
            kind: "completed".to_string(),
            session_id: resolved_sid,
            text: None,
            tool_name: None,
            error: None,
        },
    );
    true
}

fn run_antigravity_turn_in_medium(
    turn_id: &str,
    exe: &str,
    cwd: Option<&str>,
    project_id: Option<&str>,
    session_id: Option<&str>,
    model: Option<&str>,
    effort: Option<&str>,
    execution_mode: Option<&str>,
    plan_mode: Option<bool>,
    sandbox_enabled: Option<bool>,
    attachments: &[TurnAttachment],
    prompt: &str,
    pipe_writer: &mut impl Write,
) {
    let full_prompt = build_prompt_with_file_attachments(prompt, attachments, true);
    let resolved_model = model
        .map(|m| resolve_antigravity_model_for_effort(m, effort))
        .filter(|s| !s.is_empty());
    // Route existing Antigravity Desktop conversations (and Desktop projects) silently
    // through the official Plugin Sidecar (`agentapi`) — zero CLI, zero foreground stealing!
    let is_cli_target = match (session_id, project_id) {
        (Some(sid), _) if !sid.trim().is_empty() => is_antigravity_cli_conversation(sid),
        (_, Some("default-cli-project")) => true,
        _ => false,
    };
    if !is_cli_target {
        if try_run_antigravity_via_desktop_sidecar(
            turn_id,
            session_id,
            resolved_model.as_deref(),
            &full_prompt,
            pipe_writer,
        ) {
            return;
        }
    }

    let mut cmd = Command::new(exe);
    cmd.args(["--output-format", "stream-json"]);
    if let Some(ref m) = resolved_model {
        cmd.args(["--model", m]);
    }
    if let Some(e) = effort.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        cmd.args(["--effort", e]);
    }
    let mut added_sandbox = false;
    if plan_mode == Some(true) {
        cmd.args(["--mode", "plan"]);
    } else if let Some(mode) = execution_mode.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        match mode {
            "turbo" => {
                cmd.arg("--dangerously-skip-permissions");
                cmd.args(["--mode", "auto"]);
            }
            "auto" => {
                cmd.args(["--mode", "auto"]);
            }
            "default" | "accept-edits" => {
                cmd.args(["--mode", "accept-edits"]);
            }
            "request-review" => {
                cmd.arg("--sandbox");
                added_sandbox = true;
                cmd.args(["--mode", "plan"]);
            }
            "plan" => {
                cmd.args(["--mode", "plan"]);
            }
            _ => {}
        }
    }
    if sandbox_enabled == Some(true) && !added_sandbox {
        cmd.arg("--sandbox");
    }
    for att in attachments {
        if att.kind == "folder" && !att.path.trim().is_empty() && Path::new(att.path.trim()).is_dir() {
            cmd.args(["--add-dir", att.path.trim()]);
        }
    }
    if let Some(sid) = session_id.filter(|s| !s.trim().is_empty()) {
        cmd.args(["--conversation", sid.trim()]);
    } else if let Some(pid) =
        project_id.filter(|s| !s.trim().is_empty() && *s != "outside-of-project")
    {
        if pid != "default-cli-project" {
            cmd.args(["--project", pid.trim()]);
        }
    }
    cmd.arg(format!("--print={full_prompt}"));
    if let Some(dir) = resolve_effective_workspace_dir(cwd) {
        cmd.current_dir(&dir);
    }
    cmd.creation_flags(CREATE_NO_WINDOW.0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            emit_stream_event(
                pipe_writer,
                &HarnessStreamEvent {
                    turn_id: turn_id.to_string(),
                    harness: "antigravity".to_string(),
                    kind: "error".to_string(),
                    session_id: None,
                    text: None,
                    tool_name: None,
                    error: Some(format!("启动 agy.exe 失败: {e}")),
                },
            );
            return;
        }
    };

    register_turn_pid(turn_id, child.id());

    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        return;
    };

    let reader = BufReader::new(stdout);
    let mut resolved_session_id = session_id.map(|s| s.to_string());
    let mut emitted_any_delta = false;
    let mut completed = false;

    for line in reader.lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        let event_name = v
            .get("event")
            .or_else(|| v.get("type"))
            .and_then(|s| s.as_str())
            .unwrap_or("");

        match event_name {
            "init" => {
                if let Some(cid) = v.get("conversation_id").and_then(|s| s.as_str()) {
                    if !cid.trim().is_empty() {
                        resolved_session_id = Some(cid.to_string());
                        emit_stream_event(
                            pipe_writer,
                            &HarnessStreamEvent {
                                turn_id: turn_id.to_string(),
                                harness: "antigravity".to_string(),
                                kind: "session".to_string(),
                                session_id: Some(cid.to_string()),
                                text: None,
                                tool_name: None,
                                error: None,
                            },
                        );
                    }
                }
            }
            "step_update" => {
                let step_type = v.get("step_type").and_then(|s| s.as_str()).unwrap_or("");
                if let Some(delta) = v.get("text_delta").and_then(|s| s.as_str()) {
                    if !delta.is_empty() {
                        emitted_any_delta = true;
                        emit_stream_event(
                            pipe_writer,
                            &HarnessStreamEvent {
                                turn_id: turn_id.to_string(),
                                harness: "antigravity".to_string(),
                                kind: "delta".to_string(),
                                session_id: resolved_session_id.clone(),
                                text: Some(delta.to_string()),
                                tool_name: None,
                                error: None,
                            },
                        );
                    }
                } else if step_type == "tool" {
                    if let Some(tool) = v.get("tool_name").and_then(|s| s.as_str()) {
                        emit_stream_event(
                            pipe_writer,
                            &HarnessStreamEvent {
                                turn_id: turn_id.to_string(),
                                harness: "antigravity".to_string(),
                                kind: "tool".to_string(),
                                session_id: resolved_session_id.clone(),
                                text: None,
                                tool_name: Some(tool.to_string()),
                                error: None,
                            },
                        );
                    }
                }
            }
            "result" => {
                completed = true;
                let res_obj = v.get("result").unwrap_or(&v);
                let status = res_obj
                    .get("status")
                    .and_then(|s| s.as_str())
                    .unwrap_or("SUCCESS");
                if status.eq_ignore_ascii_case("ERROR") || status.eq_ignore_ascii_case("FAILURE") {
                    let err_msg = res_obj
                        .get("error")
                        .and_then(|s| s.as_str())
                        .unwrap_or("Antigravity 执行失败")
                        .to_string();
                    emit_stream_event(
                        pipe_writer,
                        &HarnessStreamEvent {
                            turn_id: turn_id.to_string(),
                            harness: "antigravity".to_string(),
                            kind: "error".to_string(),
                            session_id: resolved_session_id.clone(),
                            text: None,
                            tool_name: None,
                            error: Some(err_msg),
                        },
                    );
                } else {
                    let final_text = res_obj
                        .get("response")
                        .and_then(|s| s.as_str())
                        .map(|s| s.to_string());
                    if !emitted_any_delta {
                        if let Some(ref text) = final_text {
                            if !text.is_empty() {
                                emit_stream_event(
                                    pipe_writer,
                                    &HarnessStreamEvent {
                                        turn_id: turn_id.to_string(),
                                        harness: "antigravity".to_string(),
                                        kind: "replace".to_string(),
                                        session_id: resolved_session_id.clone(),
                                        text: Some(text.clone()),
                                        tool_name: None,
                                        error: None,
                                    },
                                );
                            }
                        }
                    }
                    emit_stream_event(
                        pipe_writer,
                        &HarnessStreamEvent {
                            turn_id: turn_id.to_string(),
                            harness: "antigravity".to_string(),
                            kind: "completed".to_string(),
                            session_id: resolved_session_id.clone(),
                            text: final_text,
                            tool_name: None,
                            error: None,
                        },
                    );
                }
            }
            _ => {}
        }
    }

    let status = child.wait();
    if !completed {
        let mut stderr_text = String::new();
        if let Some(mut se) = child.stderr.take() {
            let _ = std::io::Read::read_to_string(&mut se, &mut stderr_text);
        }
        let ok = status.map(|s| s.success()).unwrap_or(false);
        if ok {
            emit_stream_event(
                pipe_writer,
                &HarnessStreamEvent {
                    turn_id: turn_id.to_string(),
                    harness: "antigravity".to_string(),
                    kind: "completed".to_string(),
                    session_id: resolved_session_id,
                    text: None,
                    tool_name: None,
                    error: None,
                },
            );
        } else {
            let err = if stderr_text.trim().is_empty() {
                "Antigravity 进程异常退出".to_string()
            } else {
                stderr_text.trim().chars().take(280).collect()
            };
            emit_stream_event(
                pipe_writer,
                &HarnessStreamEvent {
                    turn_id: turn_id.to_string(),
                    harness: "antigravity".to_string(),
                    kind: "error".to_string(),
                    session_id: resolved_session_id,
                    text: None,
                    tool_name: None,
                    error: Some(err),
                },
            );
        }
    }
}

fn send_jsonrpc_line(stdin: &mut impl Write, val: &serde_json::Value) -> std::io::Result<()> {
    let line = serde_json::to_string(val)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    writeln!(stdin, "{line}")?;
    stdin.flush()
}

pub fn build_codex_turn_start_params(
    tid: &str,
    model: Option<&str>,
    effort: Option<&str>,
    sandbox_mode: Option<&str>,
    fast_speed: Option<bool>,
    plan_mode: Option<bool>,
    enabled_plugins: &[String],
    attachments: &[TurnAttachment],
    prompt: &str,
) -> serde_json::Map<String, serde_json::Value> {
    let mut input_items: Vec<serde_json::Value> = Vec::new();
    for att in attachments {
        if att.kind == "image" && !att.path.trim().is_empty() {
            input_items.push(serde_json::json!({
                "type": "localImage",
                "path": att.path.trim()
            }));
        }
    }
    for raw_plg in enabled_plugins {
        let plg_id = raw_plg.trim();
        if plg_id.is_empty() {
            continue;
        }
        let short_name = plg_id.split('@').next().unwrap_or(plg_id).trim();
        let uri = if plg_id.starts_with("plugin://") {
            plg_id.to_string()
        } else {
            format!("plugin://{plg_id}")
        };
        input_items.push(serde_json::json!({
            "type": "mention",
            "name": short_name,
            "path": uri
        }));
    }
    let mut final_text = build_prompt_with_file_attachments(prompt, attachments, false);
    if plan_mode == Some(true) && !final_text.trim_start().starts_with("/plan") {
        final_text = format!("[Plan Mode: 只读分析与制定分步执行计划，不要直接修改代码或执行写入操作]\n\n{final_text}");
    }
    if !final_text.trim().is_empty() || input_items.is_empty() {
        input_items.push(serde_json::json!({
            "type": "text",
            "text": final_text
        }));
    }
    let mut turn_params = serde_json::Map::new();
    turn_params.insert("threadId".to_string(), serde_json::json!(tid));
    turn_params.insert("input".to_string(), serde_json::Value::Array(input_items));
    if let Some(m) = model.map(|s| s.trim()).filter(|s| !s.is_empty() && *s != "default") {
        turn_params.insert("model".to_string(), serde_json::json!(m));
    }
    if let Some(e) = effort.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        let mapped_effort = if e == "max" { "xhigh" } else { e };
        turn_params.insert("effort".to_string(), serde_json::json!(mapped_effort));
    }
    if let Some(fast) = fast_speed {
        turn_params.insert(
            "serviceTier".to_string(),
            serde_json::json!(if fast { "fast" } else { "default" }),
        );
    }
    if plan_mode == Some(true) {
        turn_params.insert("approvalPolicy".to_string(), serde_json::json!("on-request"));
        turn_params.insert("sandboxPolicy".to_string(), serde_json::json!({ "type": "readOnly" }));
    } else if let Some(sb) = sandbox_mode.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        match sb {
            "danger-full-access" => {
                turn_params.insert("approvalPolicy".to_string(), serde_json::json!("never"));
                turn_params.insert("sandboxPolicy".to_string(), serde_json::json!({ "type": "dangerFullAccess" }));
            }
            "help-approve" => {
                // Valid Codex AskForApproval + ApprovalsReviewer per TurnStartParams.json schema
                turn_params.insert("approvalPolicy".to_string(), serde_json::json!("on-request"));
                turn_params.insert("approvalsReviewer".to_string(), serde_json::json!("auto_review"));
                turn_params.insert("sandboxPolicy".to_string(), serde_json::json!({ "type": "workspaceWrite" }));
            }
            "request-approval" | "workspace-write" => {
                turn_params.insert("approvalPolicy".to_string(), serde_json::json!("on-request"));
                turn_params.insert("approvalsReviewer".to_string(), serde_json::json!("user"));
                turn_params.insert("sandboxPolicy".to_string(), serde_json::json!({ "type": "workspaceWrite" }));
            }
            "read-only" => {
                turn_params.insert("approvalPolicy".to_string(), serde_json::json!("on-request"));
                turn_params.insert("sandboxPolicy".to_string(), serde_json::json!({ "type": "readOnly" }));
            }
            _ => {}
        }
    }
    turn_params
}

fn run_codex_app_server_turn_in_medium(
    turn_id: &str,
    exe: &str,
    cwd: Option<&str>,
    session_id: Option<&str>,
    model: Option<&str>,
    effort: Option<&str>,
    sandbox_mode: Option<&str>,
    fast_speed: Option<bool>,
    plan_mode: Option<bool>,
    enabled_plugins: &[String],
    attachments: &[TurnAttachment],
    prompt: &str,
    pipe_writer: &mut impl Write,
) {
    let mut cmd = if exe.to_ascii_lowercase().ends_with(".cmd") {
        let mut c = Command::new("cmd.exe");
        c.args(["/C", exe, "app-server"]);
        c
    } else {
        let mut c = Command::new(exe);
        c.arg("app-server");
        c
    };

    let cleaned_cwd = resolve_effective_workspace_dir(cwd);

    if let Some(ref dir) = cleaned_cwd {
        cmd.current_dir(dir);
    }

    cmd.creation_flags(CREATE_NO_WINDOW.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            emit_stream_event(
                pipe_writer,
                &HarnessStreamEvent {
                    turn_id: turn_id.to_string(),
                    harness: "codex".to_string(),
                    kind: "error".to_string(),
                    session_id: None,
                    text: None,
                    tool_name: None,
                    error: Some(format!("启动 codex app-server 失败: {e}")),
                },
            );
            return;
        }
    };

    register_turn_pid(turn_id, child.id());

    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = child.kill();
        return;
    };

    // Step 1: initialize
    let init_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "clientInfo": {
                "name": "anything-to-prompt",
                "title": "Anything-to-Prompt",
                "version": "0.1.0"
            }
        }
    });
    if let Err(e) = send_jsonrpc_line(&mut stdin, &init_req) {
        emit_stream_event(
            pipe_writer,
            &HarnessStreamEvent {
                turn_id: turn_id.to_string(),
                harness: "codex".to_string(),
                kind: "error".to_string(),
                session_id: None,
                text: None,
                tool_name: None,
                error: Some(format!("Codex initialize 写入失败: {e}")),
            },
        );
        let _ = child.kill();
        return;
    }

    let reader = BufReader::new(stdout);
    let mut stage = 1u8; // 1=wait init, 2=wait thread/start|resume, 3=streaming turn
    let mut resolved_thread_id = session_id
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string());
    let mut emitted_delta = false;
    let mut finished = false;

    for line in reader.lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };

        // Handle JSON-RPC responses by id
        if let Some(id_num) = v.get("id").and_then(|i| i.as_i64()) {
            if let Some(err) = v.get("error") {
                let msg = err
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("Codex app-server 返回错误")
                    .to_string();
                emit_stream_event(
                    pipe_writer,
                    &HarnessStreamEvent {
                        turn_id: turn_id.to_string(),
                        harness: "codex".to_string(),
                        kind: "error".to_string(),
                        session_id: resolved_thread_id.clone(),
                        text: None,
                        tool_name: None,
                        error: Some(msg),
                    },
                );
                finished = true;
                break;
            }

            if id_num == 1 && stage == 1 {
                // Send initialized notification + thread/resume or thread/start
                let _ = send_jsonrpc_line(
                    &mut stdin,
                    &serde_json::json!({ "jsonrpc": "2.0", "method": "initialized" }),
                );
                let thread_req = if let Some(ref tid) = resolved_thread_id {
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": 2,
                        "method": "thread/resume",
                        "params": { "threadId": tid }
                    })
                } else {
                    let mut start_params = serde_json::Map::new();
                    start_params.insert("cwd".to_string(), serde_json::json!(cleaned_cwd.clone().unwrap_or_default()));
                    if let Some(fast) = fast_speed {
                        start_params.insert(
                            "serviceTier".to_string(),
                            serde_json::json!(if fast { "fast" } else { "default" }),
                        );
                    }
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": 2,
                        "method": "thread/start",
                        "params": serde_json::Value::Object(start_params)
                    })
                };
                if let Err(e) = send_jsonrpc_line(&mut stdin, &thread_req) {
                    emit_stream_event(
                        pipe_writer,
                        &HarnessStreamEvent {
                            turn_id: turn_id.to_string(),
                            harness: "codex".to_string(),
                            kind: "error".to_string(),
                            session_id: None,
                            text: None,
                            tool_name: None,
                            error: Some(format!("Codex 创建/恢复会话失败: {e}")),
                        },
                    );
                    finished = true;
                    break;
                }
                stage = 2;
                continue;
            }

            if id_num == 2 && stage == 2 {
                if let Some(tid) = v
                    .get("result")
                    .and_then(|r| r.get("thread"))
                    .and_then(|t| t.get("id"))
                    .and_then(|s| s.as_str())
                {
                    resolved_thread_id = Some(tid.to_string());
                    emit_stream_event(
                        pipe_writer,
                        &HarnessStreamEvent {
                            turn_id: turn_id.to_string(),
                            harness: "codex".to_string(),
                            kind: "session".to_string(),
                            session_id: Some(tid.to_string()),
                            text: None,
                            tool_name: None,
                            error: None,
                        },
                    );
                }
                let Some(ref tid) = resolved_thread_id else {
                    emit_stream_event(
                        pipe_writer,
                        &HarnessStreamEvent {
                            turn_id: turn_id.to_string(),
                            harness: "codex".to_string(),
                            kind: "error".to_string(),
                            session_id: None,
                            text: None,
                            tool_name: None,
                            error: Some("Codex 未返回有效 threadId".to_string()),
                        },
                    );
                    finished = true;
                    break;
                };
                let turn_params = build_codex_turn_start_params(
                    tid,
                    model,
                    effort,
                    sandbox_mode,
                    fast_speed,
                    plan_mode,
                    enabled_plugins,
                    attachments,
                    prompt,
                );
                let turn_req = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 3,
                    "method": "turn/start",
                    "params": serde_json::Value::Object(turn_params)
                });
                if let Err(e) = send_jsonrpc_line(&mut stdin, &turn_req) {
                    emit_stream_event(
                        pipe_writer,
                        &HarnessStreamEvent {
                            turn_id: turn_id.to_string(),
                            harness: "codex".to_string(),
                            kind: "error".to_string(),
                            session_id: resolved_thread_id.clone(),
                            text: None,
                            tool_name: None,
                            error: Some(format!("Codex 发送消息失败: {e}")),
                        },
                    );
                    finished = true;
                    break;
                }
                stage = 3;
                continue;
            }

            // Server-initiated request (e.g., requestApproval) — answer politely so we never deadlock
            if let Some(method) = v.get("method").and_then(|m| m.as_str()) {
                if method.contains("requestApproval") {
                    let reply = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": v.get("id").cloned().unwrap_or(serde_json::json!(0)),
                        "result": { "decision": "decline" }
                    });
                    let _ = send_jsonrpc_line(&mut stdin, &reply);
                    emit_stream_event(
                        pipe_writer,
                        &HarnessStreamEvent {
                            turn_id: turn_id.to_string(),
                            harness: "codex".to_string(),
                            kind: "tool".to_string(),
                            session_id: resolved_thread_id.clone(),
                            text: None,
                            tool_name: Some("需在 Codex 桌面端确认敏感操作".to_string()),
                            error: None,
                        },
                    );
                }
            }
            continue;
        }

        // Handle JSON-RPC notifications
        let Some(method) = v.get("method").and_then(|m| m.as_str()) else {
            continue;
        };
        let params = v.get("params");

        match method {
            "item/agentMessage/delta" => {
                if let Some(delta) = params.and_then(|p| p.get("delta")).and_then(|d| d.as_str()) {
                    if !delta.is_empty() {
                        emitted_delta = true;
                        emit_stream_event(
                            pipe_writer,
                            &HarnessStreamEvent {
                                turn_id: turn_id.to_string(),
                                harness: "codex".to_string(),
                                kind: "delta".to_string(),
                                session_id: resolved_thread_id.clone(),
                                text: Some(delta.to_string()),
                                tool_name: None,
                                error: None,
                            },
                        );
                    }
                }
            }
            "item/started" => {
                if let Some(item_type) = params
                    .and_then(|p| p.get("item"))
                    .and_then(|i| i.get("type"))
                    .and_then(|t| t.as_str())
                {
                    if item_type != "agentMessage" && item_type != "userMessage" {
                        emit_stream_event(
                            pipe_writer,
                            &HarnessStreamEvent {
                                turn_id: turn_id.to_string(),
                                harness: "codex".to_string(),
                                kind: "tool".to_string(),
                                session_id: resolved_thread_id.clone(),
                                text: None,
                                tool_name: Some(item_type.to_string()),
                                error: None,
                            },
                        );
                    }
                }
            }
            "item/completed" => {
                if !emitted_delta {
                    if let Some(item) = params.and_then(|p| p.get("item")) {
                        let ty = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        if ty.eq_ignore_ascii_case("agentMessage") {
                            if let Some(txt) = item.get("text").and_then(|t| t.as_str()) {
                                if !txt.is_empty() {
                                    emitted_delta = true;
                                    emit_stream_event(
                                        pipe_writer,
                                        &HarnessStreamEvent {
                                            turn_id: turn_id.to_string(),
                                            harness: "codex".to_string(),
                                            kind: "replace".to_string(),
                                            session_id: resolved_thread_id.clone(),
                                            text: Some(txt.to_string()),
                                            tool_name: None,
                                            error: None,
                                        },
                                    );
                                }
                            }
                        }
                    }
                }
            }
            "turn/completed" => {
                let turn_err = params
                    .and_then(|p| p.get("turn"))
                    .and_then(|t| t.get("error"))
                    .and_then(|e| {
                        e.get("message")
                            .and_then(|m| m.as_str())
                            .or_else(|| e.as_str())
                    })
                    .map(|s| s.to_string());
                if let Some(err_msg) = turn_err {
                    emit_stream_event(
                        pipe_writer,
                        &HarnessStreamEvent {
                            turn_id: turn_id.to_string(),
                            harness: "codex".to_string(),
                            kind: "error".to_string(),
                            session_id: resolved_thread_id.clone(),
                            text: None,
                            tool_name: None,
                            error: Some(err_msg),
                        },
                    );
                } else {
                    emit_stream_event(
                        pipe_writer,
                        &HarnessStreamEvent {
                            turn_id: turn_id.to_string(),
                            harness: "codex".to_string(),
                            kind: "completed".to_string(),
                            session_id: resolved_thread_id.clone(),
                            text: None,
                            tool_name: None,
                            error: None,
                        },
                    );
                }
                finished = true;
                break;
            }
            _ => {}
        }
    }

    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();

    if !finished {
        emit_stream_event(
            pipe_writer,
            &HarnessStreamEvent {
                turn_id: turn_id.to_string(),
                harness: "codex".to_string(),
                kind: "error".to_string(),
                session_id: resolved_thread_id,
                text: None,
                tool_name: None,
                error: Some("Codex app-server 连接提前结束".to_string()),
            },
        );
    }
}

fn sanitize_claude_permission_mode(raw: Option<&str>) -> &'static str {
    match raw.map(|s| s.trim()).unwrap_or("auto") {
        "acceptEdits" => "acceptEdits",
        "plan" => "plan",
        "dontAsk" => "dontAsk",
        "bypassPermissions" => "bypassPermissions",
        "manual" => "manual",
        _ => "auto",
    }
}

#[derive(Default)]
struct ClaudeStreamParseState {
    session_id: Option<String>,
    emitted_any_delta: bool,
    emitted_text_in_current_msg: bool,
    completed: bool,
}

fn parse_claude_stream_json_line(
    turn_id: &str,
    line: &str,
    state: &mut ClaudeStreamParseState,
) -> Vec<HarnessStreamEvent> {
    let mut events = Vec::new();
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return events;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return events;
    };

    if let Some(sid) = v.get("session_id").and_then(|s| s.as_str()) {
        let clean_sid = sid.trim();
        if !clean_sid.is_empty() && state.session_id.as_deref() != Some(clean_sid) {
            state.session_id = Some(clean_sid.to_string());
            events.push(HarnessStreamEvent {
                turn_id: turn_id.to_string(),
                harness: "claude_code".to_string(),
                kind: "session".to_string(),
                session_id: state.session_id.clone(),
                text: None,
                tool_name: None,
                error: None,
            });
        }
    }

    let event_type = v.get("type").and_then(|s| s.as_str()).unwrap_or("");
    match event_type {
        "stream_event" => {
            let Some(ev) = v.get("event") else {
                return events;
            };
            let ev_type = ev.get("type").and_then(|s| s.as_str()).unwrap_or("");
            match ev_type {
                "message_start" => {
                    if state.emitted_text_in_current_msg {
                        // Separate multi-step assistant turns cleanly
                        events.push(HarnessStreamEvent {
                            turn_id: turn_id.to_string(),
                            harness: "claude_code".to_string(),
                            kind: "delta".to_string(),
                            session_id: state.session_id.clone(),
                            text: Some("\n\n".to_string()),
                            tool_name: None,
                            error: None,
                        });
                    }
                    state.emitted_text_in_current_msg = false;
                }
                "content_block_start" => {
                    if let Some(cb) = ev.get("content_block") {
                        if cb.get("type").and_then(|s| s.as_str()) == Some("tool_use") {
                            if let Some(name) = cb.get("name").and_then(|s| s.as_str()) {
                                events.push(HarnessStreamEvent {
                                    turn_id: turn_id.to_string(),
                                    harness: "claude_code".to_string(),
                                    kind: "tool".to_string(),
                                    session_id: state.session_id.clone(),
                                    text: None,
                                    tool_name: Some(name.to_string()),
                                    error: None,
                                });
                            }
                        }
                    }
                }
                "content_block_delta" => {
                    if let Some(delta) = ev.get("delta") {
                        if delta.get("type").and_then(|s| s.as_str()) == Some("text_delta") {
                            if let Some(txt) = delta.get("text").and_then(|s| s.as_str()) {
                                if !txt.is_empty() {
                                    state.emitted_any_delta = true;
                                    state.emitted_text_in_current_msg = true;
                                    events.push(HarnessStreamEvent {
                                        turn_id: turn_id.to_string(),
                                        harness: "claude_code".to_string(),
                                        kind: "delta".to_string(),
                                        session_id: state.session_id.clone(),
                                        text: Some(txt.to_string()),
                                        tool_name: None,
                                        error: None,
                                    });
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        "assistant" => {
            if v.get("is_api_error_message").and_then(|b| b.as_bool()) == Some(true) {
                return events;
            }
            if let Some(arr) = v
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_array())
            {
                for item in arr {
                    if item.get("type").and_then(|s| s.as_str()) == Some("tool_use") {
                        if let Some(name) = item.get("name").and_then(|s| s.as_str()) {
                            events.push(HarnessStreamEvent {
                                turn_id: turn_id.to_string(),
                                harness: "claude_code".to_string(),
                                kind: "tool".to_string(),
                                session_id: state.session_id.clone(),
                                text: None,
                                tool_name: Some(name.to_string()),
                                error: None,
                            });
                        }
                    }
                }
                if !state.emitted_any_delta {
                    if let Some(content) = v.get("message").and_then(|m| m.get("content")) {
                        let txt = extract_claude_content_text(content);
                        if !txt.is_empty() {
                            state.emitted_any_delta = true;
                            state.emitted_text_in_current_msg = true;
                            events.push(HarnessStreamEvent {
                                turn_id: turn_id.to_string(),
                                harness: "claude_code".to_string(),
                                kind: "replace".to_string(),
                                session_id: state.session_id.clone(),
                                text: Some(txt),
                                tool_name: None,
                                error: None,
                            });
                        }
                    }
                }
            }
        }
        "result" => {
            state.completed = true;
            let is_err = v.get("is_error").and_then(|b| b.as_bool()).unwrap_or(false)
                || v.get("subtype")
                    .and_then(|s| s.as_str())
                    .map(|s| s.starts_with("error"))
                    .unwrap_or(false);
            let result_text = v
                .get("result")
                .and_then(|s| s.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            if is_err {
                let err_msg = result_text.unwrap_or_else(|| "Claude Code 执行失败".to_string());
                events.push(HarnessStreamEvent {
                    turn_id: turn_id.to_string(),
                    harness: "claude_code".to_string(),
                    kind: "error".to_string(),
                    session_id: state.session_id.clone(),
                    text: None,
                    tool_name: None,
                    error: Some(err_msg),
                });
            } else {
                if !state.emitted_any_delta {
                    if let Some(ref txt) = result_text {
                        events.push(HarnessStreamEvent {
                            turn_id: turn_id.to_string(),
                            harness: "claude_code".to_string(),
                            kind: "replace".to_string(),
                            session_id: state.session_id.clone(),
                            text: Some(txt.clone()),
                            tool_name: None,
                            error: None,
                        });
                    }
                }
                events.push(HarnessStreamEvent {
                    turn_id: turn_id.to_string(),
                    harness: "claude_code".to_string(),
                    kind: "completed".to_string(),
                    session_id: state.session_id.clone(),
                    text: result_text,
                    tool_name: None,
                    error: None,
                });
            }
        }
        _ => {}
    }
    events
}

fn run_claude_code_turn_in_medium(
    turn_id: &str,
    exe: &str,
    cwd: Option<&str>,
    session_id: Option<&str>,
    permission_mode: Option<&str>,
    model: Option<&str>,
    effort: Option<&str>,
    plan_mode: Option<bool>,
    chrome_enabled: Option<bool>,
    attachments: &[TurnAttachment],
    prompt: &str,
    pipe_writer: &mut impl Write,
) {
    let perm = if plan_mode == Some(true) {
        "plan"
    } else {
        sanitize_claude_permission_mode(permission_mode)
    };
    let full_prompt = build_prompt_with_file_attachments(prompt, attachments, true);
    let mut cmd = if exe.to_ascii_lowercase().ends_with(".cmd") {
        let mut c = Command::new("cmd.exe");
        c.args(["/C", exe]);
        c
    } else {
        Command::new(exe)
    };
    cmd.args([
        "-p",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--permission-mode",
        perm,
    ]);
    if chrome_enabled == Some(true) {
        cmd.arg("--chrome");
    }
    if let Some(m) = model.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        cmd.args(["--model", m]);
    }
    if let Some(e) = effort.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        cmd.args(["--effort", e]);
    }
    for att in attachments {
        if att.kind == "folder" && !att.path.trim().is_empty() && Path::new(att.path.trim()).is_dir() {
            cmd.args(["--add-dir", att.path.trim()]);
        }
    }
    if let Some(sid) = session_id.filter(|s| !s.trim().is_empty()) {
        cmd.args(["--resume", sid.trim()]);
    }

    let cleaned_cwd = resolve_effective_workspace_dir(cwd);
    if let Some(ref dir) = cleaned_cwd {
        cmd.current_dir(dir);
    }

    cmd.creation_flags(CREATE_NO_WINDOW.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            emit_stream_event(
                pipe_writer,
                &HarnessStreamEvent {
                    turn_id: turn_id.to_string(),
                    harness: "claude_code".to_string(),
                    kind: "error".to_string(),
                    session_id: None,
                    text: None,
                    tool_name: None,
                    error: Some(format!("启动 claude.exe 失败: {e}")),
                },
            );
            return;
        }
    };

    register_turn_pid(turn_id, child.id());

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(full_prompt.as_bytes());
        let _ = stdin.flush();
        drop(stdin);
    }

    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        return;
    };

    let reader = BufReader::new(stdout);
    let mut state = ClaudeStreamParseState {
        session_id: session_id
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_string()),
        ..Default::default()
    };

    for line in reader.lines().map_while(Result::ok) {
        let evts = parse_claude_stream_json_line(turn_id, &line, &mut state);
        for evt in evts {
            emit_stream_event(pipe_writer, &evt);
        }
    }

    let status = child.wait();
    if !state.completed {
        let mut stderr_text = String::new();
        if let Some(mut se) = child.stderr.take() {
            let _ = std::io::Read::read_to_string(&mut se, &mut stderr_text);
        }
        let ok = status.map(|s| s.success()).unwrap_or(false);
        if ok {
            emit_stream_event(
                pipe_writer,
                &HarnessStreamEvent {
                    turn_id: turn_id.to_string(),
                    harness: "claude_code".to_string(),
                    kind: "completed".to_string(),
                    session_id: state.session_id,
                    text: None,
                    tool_name: None,
                    error: None,
                },
            );
        } else {
            let err = if stderr_text.trim().is_empty() {
                "Claude Code 进程异常退出".to_string()
            } else {
                stderr_text.trim().chars().take(280).collect()
            };
            emit_stream_event(
                pipe_writer,
                &HarnessStreamEvent {
                    turn_id: turn_id.to_string(),
                    harness: "claude_code".to_string(),
                    kind: "error".to_string(),
                    session_id: state.session_id,
                    text: None,
                    tool_name: None,
                    error: Some(err),
                },
            );
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExternalTurnRequest {
    pub harness: HarnessKind,
    pub turn_id: String,
    pub cwd: Option<String>,
    pub project_id: Option<String>,
    pub session_id: Option<String>,
    pub permission_mode: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub sandbox_mode: Option<String>,
    pub execution_mode: Option<String>,
    pub fast_speed: Option<bool>,
    pub plan_mode: Option<bool>,
    pub sandbox_enabled: Option<bool>,
    pub chrome_enabled: Option<bool>,
    pub enabled_plugins: Vec<String>,
    pub attachments: Vec<TurnAttachment>,
    pub prompt: String,
}

/// Dispatches an external harness turn via the medium-integrity broker and streams NDJSON events
/// straight into the Tauri `bar` WebView via `window.__DSB_HARNESS_EVENT__(evt)`.
pub fn start_external_harness_turn(
    app: AppHandle,
    broker: std::sync::Arc<BrokerClient>,
    req_in: ExternalTurnRequest,
) -> Result<(), String> {
    let harness = req_in.harness;
    let turn_id = req_in.turn_id;
    let exe = match harness {
        HarnessKind::Antigravity => {
            resolve_agy_exe().ok_or_else(|| "未检测到本机 Antigravity CLI (agy.exe)".to_string())?
        }
        HarnessKind::Codex => {
            resolve_codex_exe().ok_or_else(|| "未检测到本机 Codex (codex.exe)".to_string())?
        }
        HarnessKind::ClaudeCode => {
            resolve_claude_exe().ok_or_else(|| "未检测到本机 Claude Code CLI (claude.exe)".to_string())?
        }
        HarnessKind::Dsh => return Err("DSH 请使用原生 WebView 发送".to_string()),
    };

    let (pipe_short, server_pipe) = broker
        .create_oneshot_read_pipe(&turn_id)
        .map_err(|e| format!("创建流式管道失败: {e}"))?;

    let req = BrokerRequest::StartHarnessTurn {
        turn_id: turn_id.clone(),
        harness: harness.as_str().to_string(),
        exe: exe.to_string_lossy().to_string(),
        acp_args: Vec::new(),
        cwd: req_in.cwd,
        project_id: req_in.project_id,
        session_id: req_in.session_id,
        permission_mode: req_in.permission_mode,
        model: req_in.model,
        effort: req_in.effort,
        sandbox_mode: req_in.sandbox_mode,
        execution_mode: req_in.execution_mode,
        fast_speed: req_in.fast_speed,
        plan_mode: req_in.plan_mode,
        sandbox_enabled: req_in.sandbox_enabled,
        chrome_enabled: req_in.chrome_enabled,
        enabled_plugins: req_in.enabled_plugins,
        attachments: req_in.attachments,
        prompt: req_in.prompt,
        out_pipe: pipe_short,
    };

    let resp = broker.call(req);
    if !resp.ok {
        return Err(resp
            .error
            .unwrap_or_else(|| "启动普通权限 Harness 子进程失败".to_string()));
    }

    std::thread::Builder::new()
        .name(format!("harness-stream-{}", harness.as_str()))
        .spawn(move || {
            let Ok(reader) = server_pipe.accept_and_read() else {
                let err_evt = HarnessStreamEvent {
                    turn_id: turn_id.clone(),
                    harness: harness.as_str().to_string(),
                    kind: "error".to_string(),
                    session_id: None,
                    text: None,
                    tool_name: None,
                    error: Some("流式管道未能连接".to_string()),
                };
                push_harness_event_to_bar(&app, &err_evt);
                return;
            };
            for line in reader.lines().map_while(Result::ok) {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Ok(evt) = serde_json::from_str::<HarnessStreamEvent>(trimmed) {
                    push_harness_event_to_bar(&app, &evt);
                }
            }
        })
        .map_err(|e| format!("启动流式读取线程失败: {e}"))?;

    Ok(())
}

pub fn start_acp_harness_turn(
    app: AppHandle,
    broker: std::sync::Arc<BrokerClient>,
    harness_id: String,
    spec: crate::acp::AcpAgentSpec,
    turn_id: String,
    cwd: Option<String>,
    session_id: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    mode: Option<String>,
    attachments: Vec<TurnAttachment>,
    prompt: String,
) -> Result<(), String> {
    if !spec.available {
        return Err(format!("未找到 ACP 命令：{}", spec.command));
    }
    let (pipe_short, server_pipe) = broker
        .create_oneshot_read_pipe(&turn_id)
        .map_err(|e| format!("创建流式管道失败: {e}"))?;

    let req = BrokerRequest::StartHarnessTurn {
        turn_id: turn_id.clone(),
        harness: harness_id.clone(),
        exe: spec.command,
        acp_args: spec.args,
        cwd: cwd.or(spec.default_cwd),
        project_id: None,
        session_id,
        permission_mode: mode.clone(),
        model,
        effort,
        sandbox_mode: None,
        execution_mode: mode,
        fast_speed: None,
        plan_mode: None,
        sandbox_enabled: None,
        chrome_enabled: None,
        enabled_plugins: Vec::new(),
        attachments,
        prompt,
        out_pipe: pipe_short,
    };

    let resp = broker.call(req);
    if !resp.ok {
        return Err(resp
            .error
            .unwrap_or_else(|| "启动 ACP Agent 进程失败".to_string()));
    }

    std::thread::Builder::new()
        .name(format!("acp-stream-{}", crate::acp::normalize_acp_slug(&harness_id)))
        .spawn(move || {
            let Ok(reader) = server_pipe.accept_and_read() else {
                let err_evt = HarnessStreamEvent {
                    turn_id: turn_id.clone(),
                    harness: harness_id.clone(),
                    kind: "error".to_string(),
                    session_id: None,
                    text: None,
                    tool_name: None,
                    error: Some("ACP 流式管道未能连接".to_string()),
                };
                push_harness_event_to_bar(&app, &err_evt);
                return;
            };
            for line in reader.lines().map_while(Result::ok) {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Ok(evt) = serde_json::from_str::<HarnessStreamEvent>(trimmed) {
                    push_harness_event_to_bar(&app, &evt);
                }
            }
        })
        .map_err(|e| format!("启动 ACP 流式读取线程失败: {e}"))?;

    Ok(())
}

pub fn push_harness_event_to_bar(app: &AppHandle, evt: &HarnessStreamEvent) {
    if let Some(bar_win) = app.get_webview_window("bar") {
        if let Ok(json) = serde_json::to_string(evt) {
            let js = format!(
                "(function(){{ if (typeof window.__DSB_HARNESS_EVENT__ === 'function') {{ window.__DSB_HARNESS_EVENT__({json}); }} }})();"
            );
            let _ = bar_win.eval(&js);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_antigravity_transcript_and_strips_request_tags() {
        let sample = concat!(
            r#"{"step_index":0,"type":"USER_INPUT","content":"<USER_REQUEST>\n帮我检查一下登录按钮\n</USER_REQUEST>\n<ADDITIONAL_METADATA>now</ADDITIONAL_METADATA>"}"#,
            "\n",
            r#"{"step_index":1,"type":"PLANNER_RESPONSE","content":""}"#,
            "\n",
            r#"{"step_index":2,"type":"PLANNER_RESPONSE","content":"已经定位到登录按钮组件。"}"#,
            "\n"
        );
        let msgs = parse_antigravity_transcript_lines(sample);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].text, "帮我检查一下登录按钮");
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[1].text, "已经定位到登录按钮组件。");
    }

    #[test]
    fn parses_codex_rollout_messages_only() {
        let sample = concat!(
            r#"{"type":"response_item","payload":{"type":"message","id":"m1","role":"developer","content":[{"type":"input_text","text":"sys"}]}}"#,
            "\n",
            r#"{"type":"response_item","payload":{"type":"message","id":"m2","role":"user","content":[{"type":"input_text","text":"修复构建报错"}]}}"#,
            "\n",
            r#"{"type":"response_item","payload":{"type":"message","id":"m3","role":"assistant","content":[{"type":"output_text","text":"已修复 Cargo.toml 依赖。"}]}}"#,
            "\n"
        );
        let msgs = parse_codex_rollout_lines(sample);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].text, "修复构建报错");
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[1].text, "已修复 Cargo.toml 依赖。");
    }

    #[test]
    fn cleans_windows_verbatim_paths_and_extracts_basename() {
        assert_eq!(
            clean_windows_path(r"\\?\D:\Agent-Studio\Deepseek-Harness\DSH-ZegelUI"),
            r"D:\Agent-Studio\Deepseek-Harness\DSH-ZegelUI"
        );
        assert_eq!(
            basename_of_path(r"\\?\D:\Agent-Studio\Deepseek-Harness\DSH-ZegelUI"),
            "DSH-ZegelUI"
        );
    }

    #[test]
    fn parses_claude_code_transcript_and_skips_thinking_and_sidechain() {
        let sample = concat!(
            r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"<local-command-caveat>ignored</local-command-caveat>"}}"#,
            "\n",
            r#"{"type":"user","uuid":"u1","isSidechain":false,"message":{"role":"user","content":"检查一下 harness.rs 的接口"}}"#,
            "\n",
            r#"{"type":"assistant","uuid":"a1","isSidechain":false,"message":{"id":"msg_1","role":"assistant","content":[{"type":"thinking","thinking":"internal"}]}}"#,
            "\n",
            r#"{"type":"assistant","uuid":"a2","isSidechain":false,"message":{"id":"msg_1","role":"assistant","content":[{"type":"text","text":"已完成接口检查，所有类型匹配。"}]}}"#,
            "\n",
            r#"{"type":"assistant","uuid":"a3","isSidechain":true,"message":{"id":"msg_2","role":"assistant","content":[{"type":"text","text":"subagent noise"}]}}"#,
            "\n"
        );
        let msgs = parse_claude_code_transcript_lines(sample);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].text, "检查一下 harness.rs 的接口");
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[1].text, "已完成接口检查，所有类型匹配。");
    }

    #[test]
    fn parses_claude_code_stream_json_deltas_tools_and_errors() {
        let mut st = ClaudeStreamParseState::default();
        let e1 = parse_claude_stream_json_line(
            "t1",
            r#"{"type":"system","subtype":"init","session_id":"sess-cc-1","model":"claude-opus-5[1m]"}"#,
            &mut st,
        );
        assert_eq!(e1.len(), 1);
        assert_eq!(e1[0].kind, "session");
        assert_eq!(e1[0].session_id.as_deref(), Some("sess-cc-1"));

        let e2 = parse_claude_stream_json_line(
            "t1",
            r#"{"type":"stream_event","event":{"type":"content_block_start","content_block":{"type":"tool_use","name":"Read"}}}"#,
            &mut st,
        );
        assert_eq!(e2.len(), 1);
        assert_eq!(e2[0].kind, "tool");
        assert_eq!(e2[0].tool_name.as_deref(), Some("Read"));

        let e3 = parse_claude_stream_json_line(
            "t1",
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"你好，"}}}"#,
            &mut st,
        );
        assert_eq!(e3.len(), 1);
        assert_eq!(e3[0].kind, "delta");
        assert_eq!(e3[0].text.as_deref(), Some("你好，"));

        let e4 = parse_claude_stream_json_line(
            "t1",
            r#"{"type":"result","is_error":false,"result":"你好，世界","session_id":"sess-cc-1"}"#,
            &mut st,
        );
        assert_eq!(e4.len(), 1);
        assert_eq!(e4[0].kind, "completed");

        // Error case (e.g. unauthenticated)
        let mut st_err = ClaudeStreamParseState::default();
        let e_err = parse_claude_stream_json_line(
            "t2",
            r#"{"type":"result","is_error":true,"result":"Not logged in · Please run /login","session_id":"sess-err"}"#,
            &mut st_err,
        );
        assert!(e_err.iter().any(|e| e.kind == "error" && e.error.as_deref() == Some("Not logged in · Please run /login")));
    }

    #[test]
    fn readonly_catalog_loaders_succeed_without_mutating_local_dbs() {
        let avail = list_harness_availability(true, &[], Some("Chrome · ChatGPT"));
        assert!(avail.len() >= 5);
        assert!(avail.iter().any(|a| a.id == "claude_code"));
        assert!(avail.iter().any(|a| a.id == "universal" && a.subtitle.contains("Chrome")));
        let ag = load_antigravity_catalog();
        assert_eq!(ag.harness, "antigravity");
        assert!(ag.error.is_none());
        if let Some(first_conv) = ag.groups.iter().flat_map(|g| g.convs.iter()).next() {
            let msgs = load_external_messages(HarnessKind::Antigravity, &first_conv.id);
            eprintln!(
                "[LIVE-VERIFY] Antigravity: {} groups, first_conv={}, msgs={}",
                ag.groups.len(),
                first_conv.title,
                msgs.len()
            );
        }

        let cx = load_codex_catalog();
        assert_eq!(cx.harness, "codex");
        assert!(cx.error.is_none());
        for g in &cx.groups {
            for c in &g.convs {
                assert_ne!(c.source, "exec", "exec threads must stay filtered out");
                assert!(!c.source.starts_with('{'), "subagents must stay filtered out");
            }
        }
        if let Some(first_conv) = cx.groups.iter().flat_map(|g| g.convs.iter()).next() {
            let msgs = load_external_messages(HarnessKind::Codex, &first_conv.id);
            eprintln!(
                "[LIVE-VERIFY] Codex: {} groups, first_conv={}, msgs={}",
                cx.groups.len(),
                first_conv.title,
                msgs.len()
            );
        }

        let cc = load_claude_code_catalog();
        assert_eq!(cc.harness, "claude_code");
        assert!(cc.error.is_none());
        if let Some(first_conv) = cc.groups.iter().flat_map(|g| g.convs.iter()).next() {
            let msgs = load_external_messages(HarnessKind::ClaudeCode, &first_conv.id);
            eprintln!(
                "[LIVE-VERIFY] ClaudeCode: {} groups, first_conv={}, msgs={}",
                cc.groups.len(),
                first_conv.title,
                msgs.len()
            );
        }
    }

    #[test]
    fn builds_authenticated_dsh_focus_url_with_session_fragment() {
        let with_sid = build_dsh_focus_url(
            "http://127.0.0.1:3080/?token=secret-tok&dsh_bar_clone=1&dsb_mode=lip",
            Some("session-abc-123"),
        );
        assert_eq!(
            with_sid,
            "http://127.0.0.1:3080/?token=secret-tok#atp_session=session-abc-123"
        );
        let without_sid = build_dsh_focus_url("http://127.0.0.1:3080/?token=secret-tok", Some("  "));
        assert_eq!(without_sid, "http://127.0.0.1:3080/?token=secret-tok");
    }

    #[test]
    fn resolves_antigravity_model_variants_and_codex_turn_params_and_claude_stats() {
        // 1. Antigravity model + effort resolution against live `agy models` IDs
        assert_eq!(
            resolve_antigravity_model_for_effort("gemini-4-neon-high", Some("low")),
            "gemini-4-neon-low"
        );
        assert_eq!(
            resolve_antigravity_model_for_effort("gemini-4-neon-high", Some("xhigh")),
            "gemini-4-neon-xhigh"
        );
        assert_eq!(
            resolve_antigravity_model_for_effort("claude-opus-5-5-medium", Some("max")),
            "claude-opus-5-5-high"
        );
        assert_eq!(
            resolve_antigravity_model_for_effort("gemini-pro-orchestrator-neon", Some("low")),
            "gemini-pro-orchestrator-neon"
        );

        // 2. Codex TurnStartParams: valid AskForApproval + auto_review + serviceTier fast + plugin mention
        let cx_params = build_codex_turn_start_params(
            "thread-1",
            Some("gpt-6-luna"),
            Some("xhigh"),
            Some("help-approve"),
            Some(true),
            Some(false),
            &["documents@openai-curated".to_string()],
            &[],
            "整理文档",
        );
        assert_eq!(cx_params["approvalPolicy"], serde_json::json!("on-request"));
        assert_eq!(cx_params["approvalsReviewer"], serde_json::json!("auto_review"));
        assert_eq!(cx_params["serviceTier"], serde_json::json!("fast"));
        let inputs = cx_params["input"].as_array().unwrap();
        assert_eq!(inputs[0]["type"], serde_json::json!("mention"));
        assert_eq!(inputs[0]["name"], serde_json::json!("documents"));
        assert_eq!(inputs[0]["path"], serde_json::json!("plugin://documents@openai-curated"));

        // 3. Codex Plan Mode enforces readOnly sandbox
        let cx_plan = build_codex_turn_start_params(
            "thread-1",
            None,
            None,
            Some("danger-full-access"),
            Some(false),
            Some(true),
            &[],
            &[],
            "设计架构",
        );
        assert_eq!(cx_plan["sandboxPolicy"]["type"], serde_json::json!("readOnly"));
        assert_eq!(cx_plan["serviceTier"], serde_json::json!("default"));

        // 4. Claude Code session stats parser
        let cc_jsonl = concat!(
            r#"{"type":"assistant","message":{"id":"m1","model":"claude-opus-5-5","usage":{"input_tokens":1200,"output_tokens":340,"cache_read_input_tokens":5000,"cache_creation_input_tokens":200}}}"#,
            "\n",
            r#"{"type":"assistant","message":{"id":"m1","model":"claude-opus-5-5","usage":{"input_tokens":1200,"output_tokens":340,"cache_read_input_tokens":5000,"cache_creation_input_tokens":200}}}"#,
            "\n",
            r#"{"type":"assistant","message":{"id":"m2","model":"claude-opus-5-5","usage":{"input_tokens":800,"output_tokens":160,"cache_read_input_tokens":6000,"cache_creation_input_tokens":0}}}"#,
            "\n"
        );
        let stats = parse_claude_session_stats_lines("sess-1", cc_jsonl);
        assert_eq!(stats.turns, 2);
        assert_eq!(stats.input_tokens, 2000);
        assert_eq!(stats.output_tokens, 500);
        assert_eq!(stats.cache_read_tokens, 11000);
        assert_eq!(stats.total_tokens, 13700);
        assert!(stats.summary_markdown.contains("13700"));
    }

    #[test]
    fn resolve_effective_workspace_dir_falls_back_to_default_workspace() {
        let fallback = resolve_effective_workspace_dir(None).expect("default workspace must resolve");
        assert!(Path::new(&fallback).is_dir());
        assert!(fallback.ends_with("AnythingToPrompt"));

        let missing = resolve_effective_workspace_dir(Some(r"Z:\definitely_nonexistent_atp_dir_99999"))
            .expect("missing dir should fall back to default workspace");
        assert_eq!(missing, fallback);

        let tmp = std::env::temp_dir();
        let tmp_str = tmp.to_string_lossy().to_string();
        let explicit = resolve_effective_workspace_dir(Some(&tmp_str)).expect("existing dir should be kept");
        assert_eq!(explicit, tmp_str);
    }

    #[test]
    fn parses_per_session_model_effort_and_permission_across_harnesses() {
        // 1. Antigravity model/effort + permission_preset parser
        assert_eq!(
            parse_antigravity_model_and_effort("claude-opus-5-5-high"),
            (Some("claude-opus-5-5-medium".to_string()), Some("high".to_string()))
        );
        assert_eq!(
            parse_antigravity_model_and_effort("gemini-4-neon-xhigh"),
            (Some("gemini-4-neon-high".to_string()), Some("xhigh".to_string()))
        );
        assert_eq!(
            parse_antigravity_permission_preset(
                r#"title:"Debug" permission_preset:AGENT_PERMISSION_PRESET_TURBO"#
            ),
            Some("turbo".to_string())
        );
        assert_eq!(
            parse_antigravity_permission_preset("permission_preset:AGENT_PERMISSION_PRESET_AUTO"),
            Some("auto".to_string())
        );
        let ag_tail = concat!(
            r#"{"type":"USER_INPUT","content":"hi"}"#,
            "\n",
            r#"{"type":"PLANNER_RESPONSE","model":"gemini-4-carbon-high","content":"ok"}"#,
            "\n"
        );
        assert_eq!(
            parse_antigravity_latest_model_from_transcript(ag_tail),
            Some("gemini-4-carbon-high".to_string())
        );

        // 2. Codex turn_context + SQLite fallback parser
        let cx_tail = concat!(
            r#"{"type":"session_meta","payload":{"id":"t1"}}"#,
            "\n",
            r#"{"type":"turn_context","payload":{"model":"gpt-6.1-terra","effort":"xhigh","approval_policy":"on-request","approvals_reviewer":"auto_review","sandbox_policy":{"type":"workspace-write"},"collaboration_mode":{"mode":"plan"}}}"#,
            "\n"
        );
        let (cx_m, cx_e, cx_mode, cx_plan) =
            parse_codex_session_state_from_turn_context(cx_tail, Some("gpt-6-luna"), Some("high"), Some("never"), None);
        assert_eq!(cx_m.as_deref(), Some("gpt-6.1-terra"));
        assert_eq!(cx_e.as_deref(), Some("xhigh"));
        assert_eq!(cx_mode.as_deref(), Some("help-approve"));
        assert_eq!(cx_plan, Some(true));

        // 3. Claude Code JSONL parser (ignores <synthetic> and sidechain)
        let cc_tail = concat!(
            r#"{"type":"user","permissionMode":"acceptEdits","message":{"content":"fix"}}"#,
            "\n",
            r#"{"type":"assistant","message":{"model":"claude-sonnet-5-5-20260514","content":[]}}"#,
            "\n",
            r#"{"type":"assistant","isSidechain":true,"message":{"model":"claude-haiku-5-5","content":[]}}"#,
            "\n",
            r#"{"type":"assistant","message":{"model":"<synthetic>","content":[]}}"#,
            "\n"
        );
        let (cc_m, cc_mode) = parse_claude_session_state_from_jsonl(cc_tail);
        assert_eq!(cc_m.as_deref(), Some("claude-sonnet-5-5"));
        assert_eq!(cc_mode.as_deref(), Some("acceptEdits"));
    }
}
