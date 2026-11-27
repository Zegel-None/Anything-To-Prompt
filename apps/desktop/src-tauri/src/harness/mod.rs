mod catalog;
mod executors;
mod focus;
mod transcript;

pub use catalog::*;
pub use executors::*;
pub use focus::*;
pub use transcript::*;

use crate::broker::{BrokerClient, BrokerRequest};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Write};
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
    BringWindowToTop, EnumWindows, GetWindowRect, GetWindowThreadProcessId,
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
