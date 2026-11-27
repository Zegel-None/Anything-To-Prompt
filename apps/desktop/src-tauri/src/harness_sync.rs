use crate::harness::{
    basename_of_path, clean_windows_path, parse_antigravity_transcript_lines,
    parse_claude_code_transcript_lines, parse_codex_rollout_lines, sanitize_title,
    ExternalChatMessage,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::FromRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL, INVALID_HANDLE_VALUE};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{
    PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
};
use windows::Win32::Storage::FileSystem::{
    FlushFileBuffers, PIPE_ACCESS_DUPLEX, PIPE_ACCESS_INBOUND,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows::Win32::System::Threading::CREATE_NO_WINDOW;

const SYNC_PIPE_SHORT: &str = "atp-harness-sync-v1";
const ANTIGRAVITY_SIDECAR_PIPE_SHORT: &str = "atp-antigravity-sidecar-v1";
const ANTIGRAVITY_SIDECAR_ID: &str = "anything-to-prompt-sync/desktop-bridge";
const PLUGIN_DIR_NAME: &str = "anything-to-prompt-sync";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravitySidecarRequest {
    pub action: String, // "ping" | "send_message" | "new_conversation"
    pub conversation_id: Option<String>,
    pub prompt: Option<String>,
    pub title: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravitySidecarResponse {
    pub ok: bool,
    pub conversation_id: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookSyncNotice {
    pub harness: String,
    pub event: String,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub transcript_path: Option<String>,
    pub last_user_input: Option<String>,
    pub last_assistant_message: Option<String>,
    pub running: Option<bool>,
    pub timestamp_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessSyncPayload {
    pub harness: String,
    pub workspace_id: String,
    pub workspace_path: String,
    pub workspace_label: String,
    pub session_id: String,
    pub title: String,
    #[serde(default)]
    pub title_authoritative: bool,
    pub messages: Vec<ExternalChatMessage>,
    pub running: bool,
    pub source: String, // "hook" | "watch"
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalSyncPluginStatus {
    pub enabled: bool,
    pub dsh_plugin: bool,
    pub antigravity_plugin: bool,
    pub claude_code_plugin: bool,
    pub codex_plugin: bool,
    pub codex_live_watch: bool,
    pub runtime_exe_path: String,
    pub pipe_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipeControlCommand {
    pub control: String,
    pub arg: Option<String>,
}

pub fn send_control_to_pipe(action: &str, arg: Option<&str>) -> std::io::Result<()> {
    let cmd = PipeControlCommand {
        control: action.to_string(),
        arg: arg.map(|s| s.to_string()),
    };
    let encoded = serde_json::to_string(&cmd)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let mut file = OpenOptions::new()
        .write(true)
        .custom_flags(0)
        .open(sync_pipe_full_path())?;
    writeln!(file, "{encoded}")?;
    file.flush()
}

fn sync_pipe_full_path() -> String {
    format!(r"\\.\pipe\{SYNC_PIPE_SHORT}")
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

use crate::win32_util::{current_user_sid_string, to_wide};

/// Fast-path CLI subcommand executed by Antigravity / Claude Code / Codex hooks:
/// `anything-to-prompt.exe --hook-event <antigravity|claude_code|codex> [optional_json]`
/// Reads hook JSON, forwards a compact `HookSyncNotice` to the running bar's Named Pipe (<5ms),
/// prints `{}` to stdout (strictly compliant with synchronous hook contracts), and exits 0.
pub fn run_hook_event_cli(harness_raw: &str, inline_json: Option<&str>) {
    let harness = match harness_raw.trim().to_ascii_lowercase().as_str() {
        "antigravity" | "agy" => "antigravity",
        "claude_code" | "claude-code" | "claudecode" | "claude" => "claude_code",
        "codex" => "codex",
        _ => {
            println!("{{}}");
            return;
        }
    };

    let raw_input = if let Some(arg) = inline_json.filter(|s| !s.trim().is_empty()) {
        arg.to_string()
    } else {
        let mut buf = String::new();
        let _ = std::io::stdin().read_to_string(&mut buf);
        buf
    };

    if let Some(notice) = parse_hook_payload(harness, &raw_input) {
        let _ = send_notice_to_pipe(&sync_pipe_full_path(), &notice);
    }

    // Always return empty JSON object so synchronous hooks (Antigravity / Claude Code) never block or error.
    println!("{{}}");
}

pub fn parse_hook_payload(harness: &str, raw_json: &str) -> Option<HookSyncNotice> {
    let trimmed = raw_json.trim();
    if trimmed.is_empty() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(trimmed).ok()?;

    let event = v
        .get("hookEventName")
        .or_else(|| v.get("hook_event_name"))
        .or_else(|| v.get("type"))
        .and_then(|s| s.as_str())
        .unwrap_or("event")
        .to_string();

    let session_id = v
        .get("conversationId")
        .or_else(|| v.get("session_id"))
        .or_else(|| v.get("sessionId"))
        .or_else(|| v.get("thread-id"))
        .or_else(|| v.get("thread_id"))
        .and_then(|s| s.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let cwd = v
        .get("cwd")
        .and_then(|s| s.as_str())
        .map(clean_windows_path)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            v.get("workspacePaths")
                .and_then(|a| a.as_array())
                .and_then(|arr| arr.first())
                .and_then(|s| s.as_str())
                .map(clean_windows_path)
                .filter(|s| !s.is_empty())
        });

    let transcript_path = v
        .get("transcriptPath")
        .or_else(|| v.get("transcript_path"))
        .and_then(|s| s.as_str())
        .map(clean_windows_path)
        .filter(|s| !s.is_empty());

    let last_user_input = v
        .get("lastUserInput")
        .or_else(|| v.get("prompt"))
        .and_then(|s| s.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let last_assistant_message = v
        .get("last_assistant_message")
        .or_else(|| v.get("last-assistant-message"))
        .and_then(|s| s.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let ev_lower = event.to_ascii_lowercase();
    let running = if ev_lower.contains("stop") || ev_lower.contains("complete") || ev_lower.contains("end") {
        Some(false)
    } else if ev_lower.contains("preinvocation")
        || ev_lower.contains("userpromptsubmit")
        || ev_lower.contains("pretooluse")
    {
        Some(true)
    } else {
        None
    };

    Some(HookSyncNotice {
        harness: harness.to_string(),
        event,
        session_id,
        cwd,
        transcript_path,
        last_user_input,
        last_assistant_message,
        running,
        timestamp_ms: now_ms(),
    })
}

fn send_notice_to_pipe(pipe_path: &str, notice: &HookSyncNotice) -> std::io::Result<()> {
    let encoded = serde_json::to_string(notice)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let mut file = OpenOptions::new()
        .write(true)
        .custom_flags(0)
        .open(pipe_path)?;
    writeln!(file, "{encoded}")?;
    file.flush()
}

fn write_if_changed(path: &Path, content: &str) -> std::io::Result<bool> {
    if let Ok(existing) = fs::read_to_string(path) {
        if existing == content {
            return Ok(false);
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(true)
}

pub fn build_antigravity_hooks_json(exe_path: &Path) -> String {
    let cmd = format!("\"{}\" --hook-event antigravity", exe_path.to_string_lossy());
    let hook_entry = serde_json::json!({
        "type": "command",
        "command": cmd,
        "timeout": 3
    });
    let root = serde_json::json!({
        "anything-to-prompt-sync": {
            "SessionStart": [hook_entry],
            "PreInvocation": [hook_entry],
            "PostInvocation": [hook_entry],
            "Stop": [hook_entry]
        }
    });
    serde_json::to_string_pretty(&root).unwrap_or_default()
}

pub fn build_claude_code_hooks_json(exe_path: &Path) -> String {
    let slash_exe = exe_path.to_string_lossy().replace('\\', "/");
    let cmd = format!("\"{slash_exe}\" --hook-event claude_code");
    let group = serde_json::json!([
        {
            "hooks": [
                {
                    "type": "command",
                    "command": cmd,
                    "timeout": 5
                }
            ]
        }
    ]);
    let root = serde_json::json!({
        "description": "Anything-to-Prompt real-time workspace and conversation sync hooks",
        "hooks": {
            "SessionStart": group,
            "UserPromptSubmit": group,
            "Stop": group
        }
    });
    serde_json::to_string_pretty(&root).unwrap_or_default()
}

pub fn build_antigravity_mcp_json(exe_path: &Path) -> String {
    let root = serde_json::json!({
        "mcpServers": {
            "anything-to-prompt": {
                "command": exe_path.to_string_lossy(),
                "args": ["--mcp-sidecar", "--harness", "antigravity"]
            }
        }
    });
    serde_json::to_string_pretty(&root).unwrap_or_default()
}

pub fn build_claude_code_mcp_json(exe_path: &Path) -> String {
    let slash_exe = exe_path.to_string_lossy().replace('\\', "/");
    let root = serde_json::json!({
        "mcpServers": {
            "anything-to-prompt": {
                "command": slash_exe,
                "args": ["--mcp-sidecar", "--harness", "claude_code"]
            }
        }
    });
    serde_json::to_string_pretty(&root).unwrap_or_default()
}

pub fn build_codex_mcp_json(exe_path: &Path) -> String {
    let root = serde_json::json!({
        "mcpServers": {
            "anything-to-prompt": {
                "command": exe_path.to_string_lossy(),
                "args": ["--mcp-sidecar", "--harness", "codex"]
            }
        }
    });
    serde_json::to_string_pretty(&root).unwrap_or_default()
}

fn antigravity_plugin_dir(home: &Path) -> PathBuf {
    home.join(".gemini")
        .join("config")
        .join("plugins")
        .join(PLUGIN_DIR_NAME)
}

fn claude_code_plugin_dir(home: &Path) -> PathBuf {
    home.join(".claude").join("skills").join(PLUGIN_DIR_NAME)
}

fn codex_plugin_dir(home: &Path) -> PathBuf {
    home.join(".codex").join("plugins").join(PLUGIN_DIR_NAME)
}

/// Atomically replaces `dst` with `src` even when `dst` is a currently running Windows executable
/// (e.g., locked by `--mcp-sidecar` or `--antigravity-sidecar`): NTFS allows renaming a running `.exe`
/// on the same volume, freeing the canonical path immediately for the new binary.
fn atomic_sync_exe(src: &Path, dst: &Path) -> std::io::Result<()> {
    if let Some(parent) = dst.parent() {
        let _ = fs::create_dir_all(parent);
        // Best-effort cleanup of stale `.old.*` binaries whose sidecar processes have exited
        if let Ok(entries) = fs::read_dir(parent) {
            for ent in entries.flatten() {
                let name = ent.file_name().to_string_lossy().to_ascii_lowercase();
                if name.starts_with("anything-to-prompt.exe.old") {
                    let _ = fs::remove_file(ent.path());
                }
            }
        }
    }
    if fs::copy(src, dst).is_ok() {
        return Ok(());
    }
    if dst.exists() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let backup = dst.with_file_name(format!(
            "anything-to-prompt.exe.old.{}.{}",
            std::process::id(),
            stamp
        ));
        fs::rename(dst, &backup)?;
        if let Err(copy_err) = fs::copy(src, dst) {
            let _ = fs::rename(&backup, dst);
            return Err(copy_err);
        }
        let _ = fs::remove_file(&backup);
        return Ok(());
    }
    fs::copy(src, dst).map(|_| ())
}

/// Mirrors the running executable into `%LOCALAPPDATA%\AnythingToPrompt\bin\anything-to-prompt.exe`
/// so all 4 Harness plugins reference one stable, permanent dependency binary even if the user
/// moves or deletes their downloaded portable EXE.
pub fn ensure_runtime_exe_installed() -> PathBuf {
    let canonical = crate::config::AppConfig::runtime_exe_path();
    let Ok(cur) = std::env::current_exe() else {
        return canonical;
    };
    let cur_name = cur
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    // Never overwrite the runtime bin with a cargo test harness binary
    if !cur_name.eq_ignore_ascii_case("anything-to-prompt.exe") {
        return if canonical.is_file() { canonical } else { cur };
    }
    if cur
        .to_string_lossy()
        .eq_ignore_ascii_case(&canonical.to_string_lossy())
    {
        return canonical;
    }
    let needs_copy = match (fs::metadata(&cur), fs::metadata(&canonical)) {
        (Ok(src), Ok(dst)) => {
            src.len() != dst.len()
                || src.modified().ok() != dst.modified().ok()
        }
        _ => true,
    };
    if needs_copy {
        match atomic_sync_exe(&cur, &canonical) {
            Ok(()) => {
                log::info!("Synced stable runtime dependency exe -> {}", canonical.display());
                return canonical;
            }
            Err(e) => {
                log::warn!(
                    "Failed to sync runtime dependency exe {} -> {}: {e}",
                    cur.display(),
                    canonical.display()
                );
            }
        }
    }
    if canonical.is_file() {
        canonical
    } else {
        cur
    }
}

/// Installs/updates or cleanly removes the lightweight native companion/sync plugins for:
/// - Antigravity (`~/.gemini/config/plugins/anything-to-prompt-sync`: `plugin.json` + `hooks.json` + `mcp_config.json`)
/// - Claude Code (`~/.claude/skills/anything-to-prompt-sync`: `.claude-plugin/plugin.json` + `hooks/hooks.json` + `.mcp.json`)
/// - Codex (`~/.codex/plugins/anything-to-prompt-sync`: `.codex-plugin/plugin.json` + `.mcp.json`, plus zero-invasion JSONL live tail)
///
/// Never mutates third-party databases or `config.toml`.
pub fn apply_external_sync_plugins(enable: bool) -> ExternalSyncPluginStatus {
    let dsh_ok = crate::config::AppConfig::dsh_home_dir()
        .join("profiles")
        .join("web")
        .join("local-plugins")
        .join("anything-to-prompt")
        .join("package.json")
        .is_file();

    let Some(home) = dirs::home_dir() else {
        return ExternalSyncPluginStatus {
            enabled: enable,
            dsh_plugin: dsh_ok,
            antigravity_plugin: false,
            claude_code_plugin: false,
            codex_plugin: false,
            codex_live_watch: false,
            runtime_exe_path: crate::config::AppConfig::runtime_exe_path()
                .to_string_lossy()
                .to_string(),
            pipe_name: sync_pipe_full_path(),
        };
    };

    let exe = ensure_runtime_exe_installed();
    let exe_display = exe.to_string_lossy().to_string();
    let ag_dir = antigravity_plugin_dir(&home);
    let cc_dir = claude_code_plugin_dir(&home);
    let cx_dir = codex_plugin_dir(&home);
    let has_codex = home.join(".codex").is_dir();

    if !enable {
        enable_antigravity_sidecar_in_config(&home, false);
        for dir in [&ag_dir, &cc_dir, &cx_dir] {
            if dir.exists() {
                let _ = fs::remove_dir_all(dir);
            }
        }
        return ExternalSyncPluginStatus {
            enabled: false,
            dsh_plugin: dsh_ok,
            antigravity_plugin: false,
            claude_code_plugin: false,
            codex_plugin: false,
            codex_live_watch: false,
            runtime_exe_path: exe_display,
            pipe_name: sync_pipe_full_path(),
        };
    }

    // 1. Antigravity native plugin (`plugin.json` + root `hooks.json` + `mcp_config.json` + `sidecars/desktop-bridge/sidecar.json`)
    let mut ag_ok = false;
    if home.join(".gemini").is_dir() {
        let manifest = serde_json::to_string_pretty(&serde_json::json!({
            "name": PLUGIN_DIR_NAME,
            "description": "Anything-to-Prompt companion bar launcher, settings bridge, silent Desktop messaging sidecar, and real-time workspace transcript sync."
        }))
        .unwrap_or_default();
        let hooks = build_antigravity_hooks_json(&exe);
        let mcp = build_antigravity_mcp_json(&exe);
        let sidecar_cfg = serde_json::to_string_pretty(&serde_json::json!({
            "display_name": "Anything-to-Prompt Desktop Bridge",
            "description": "Silently delivers prompts from Anything-to-Prompt into existing Antigravity Desktop conversations via official agentapi.",
            "command": exe.to_string_lossy(),
            "args": ["--antigravity-sidecar"],
            "restart_policy": "always"
        }))
        .unwrap_or_default();
        let w1 = write_if_changed(&ag_dir.join("plugin.json"), &manifest);
        let w2 = write_if_changed(&ag_dir.join("hooks.json"), &hooks);
        let w3 = write_if_changed(&ag_dir.join("mcp_config.json"), &mcp);
        let w4 = write_if_changed(
            &ag_dir
                .join("sidecars")
                .join("desktop-bridge")
                .join("sidecar.json"),
            &sidecar_cfg,
        );
        enable_antigravity_sidecar_in_config(&home, true);
        ag_ok = w1.is_ok() && w2.is_ok() && w3.is_ok() && w4.is_ok();
        if ag_ok {
            log::info!("Synced Antigravity companion plugin + sidecar at {}", ag_dir.display());
        }
    }

    // 2. Claude Code native skills-dir plugin (`.claude-plugin/plugin.json` + `hooks/hooks.json` + `.mcp.json`)
    let mut cc_ok = false;
    if home.join(".claude").is_dir() {
        let manifest = serde_json::to_string_pretty(&serde_json::json!({
            "name": PLUGIN_DIR_NAME,
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Anything-to-Prompt companion bar launcher, settings bridge, and real-time workspace transcript sync.",
            "author": {
                "name": "Anything-to-Prompt"
            }
        }))
        .unwrap_or_default();
        let hooks = build_claude_code_hooks_json(&exe);
        let mcp = build_claude_code_mcp_json(&exe);
        let w1 = write_if_changed(&cc_dir.join(".claude-plugin").join("plugin.json"), &manifest);
        let w2 = write_if_changed(&cc_dir.join("hooks").join("hooks.json"), &hooks);
        let w3 = write_if_changed(&cc_dir.join(".mcp.json"), &mcp);
        cc_ok = w1.is_ok() && w2.is_ok() && w3.is_ok();
        if cc_ok {
            log::info!("Synced Claude Code companion plugin at {}", cc_dir.display());
        }
    }

    // 3. Codex native plugin bundle (`.codex-plugin/plugin.json` + `.mcp.json`) + zero-invasion JSONL live watch
    let mut cx_ok = false;
    if has_codex {
        let manifest = serde_json::to_string_pretty(&serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "name": PLUGIN_DIR_NAME,
            "mcpServers": "./.mcp.json",
            "description": "Anything-to-Prompt companion bar launcher and settings bridge for Codex.",
            "author": "Anything-to-Prompt",
            "license": "MIT"
        }))
        .unwrap_or_default();
        let mcp = build_codex_mcp_json(&exe);
        let w1 = write_if_changed(&cx_dir.join(".codex-plugin").join("plugin.json"), &manifest);
        let w2 = write_if_changed(&cx_dir.join(".mcp.json"), &mcp);
        cx_ok = w1.is_ok() && w2.is_ok();
        if cx_ok {
            log::info!("Synced Codex companion plugin at {}", cx_dir.display());
        }
    }

    ExternalSyncPluginStatus {
        enabled: true,
        dsh_plugin: dsh_ok,
        antigravity_plugin: ag_ok,
        claude_code_plugin: cc_ok,
        codex_plugin: cx_ok,
        codex_live_watch: has_codex,
        runtime_exe_path: exe_display,
        pipe_name: sync_pipe_full_path(),
    }
}

pub fn query_external_sync_status(enabled: bool) -> ExternalSyncPluginStatus {
    let dsh_ok = crate::config::AppConfig::dsh_home_dir()
        .join("profiles")
        .join("web")
        .join("local-plugins")
        .join("anything-to-prompt")
        .join("package.json")
        .is_file();
    let runtime_exe = crate::config::AppConfig::runtime_exe_path()
        .to_string_lossy()
        .to_string();
    let Some(home) = dirs::home_dir() else {
        return ExternalSyncPluginStatus {
            enabled,
            dsh_plugin: dsh_ok,
            antigravity_plugin: false,
            claude_code_plugin: false,
            codex_plugin: false,
            codex_live_watch: false,
            runtime_exe_path: runtime_exe,
            pipe_name: sync_pipe_full_path(),
        };
    };
    ExternalSyncPluginStatus {
        enabled,
        dsh_plugin: dsh_ok,
        antigravity_plugin: antigravity_plugin_dir(&home).join("hooks.json").is_file(),
        claude_code_plugin: claude_code_plugin_dir(&home)
            .join("hooks")
            .join("hooks.json")
            .is_file(),
        codex_plugin: codex_plugin_dir(&home)
            .join(".codex-plugin")
            .join("plugin.json")
            .is_file(),
        codex_live_watch: enabled && home.join(".codex").is_dir(),
        runtime_exe_path: runtime_exe,
        pipe_name: sync_pipe_full_path(),
    }
}

pub fn push_harness_sync_to_bar(app: &AppHandle, payload: &HarnessSyncPayload) {
    if let Some(bar_win) = app.get_webview_window("bar") {
        if let Ok(json) = serde_json::to_string(payload) {
            let js = format!(
                "(function(){{ if (typeof window.__DSB_HARNESS_SYNC__ === 'function') {{ window.__DSB_HARNESS_SYNC__({json}); }} }})();"
            );
            let _ = bar_win.eval(&js);
        }
    }
}

fn file_sig(path: &Path) -> Option<(i64, u64)> {
    let meta = fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    Some((mtime, meta.len()))
}

fn title_from_messages(msgs: &[ExternalChatMessage], fallback: &str) -> String {
    if let Some(first_user) = msgs.iter().find(|m| m.role == "user" && !m.text.trim().is_empty()) {
        return sanitize_title(&first_user.text, fallback);
    }
    fallback.to_string()
}

fn build_sync_payload_from_hook(notice: &HookSyncNotice) -> Option<HarnessSyncPayload> {
    let harness = notice.harness.as_str();
    let session_id = notice.session_id.clone().unwrap_or_default();
    if session_id.is_empty() && notice.transcript_path.is_none() {
        return None;
    }

    let mut messages: Vec<ExternalChatMessage> = Vec::new();
    if let Some(ref tp) = notice.transcript_path {
        if let Ok(raw) = fs::read_to_string(tp) {
            messages = match harness {
                "antigravity" => parse_antigravity_transcript_lines(&raw),
                "claude_code" => parse_claude_code_transcript_lines(&raw),
                "codex" => parse_codex_rollout_lines(&raw),
                _ => Vec::new(),
            };
        }
    }
    if messages.is_empty() && !session_id.is_empty() {
        let kind = crate::harness::HarnessKind::parse(harness);
        messages = crate::harness::load_external_messages(kind, &session_id);
    }

    // Append in-flight user/assistant text from hook payload if async JSONL flush hasn't caught up yet
    if let Some(ref user_txt) = notice.last_user_input {
        let already_last = messages
            .last()
            .map(|m| m.role == "user" && m.text.trim() == user_txt.trim())
            .unwrap_or(false);
        if !already_last && notice.event.to_ascii_lowercase().contains("prompt") {
            messages.push(ExternalChatMessage {
                id: format!("hook-u-{}", notice.timestamp_ms),
                role: "user".to_string(),
                text: user_txt.clone(),
                streaming: false,
            });
        }
    }
    if let Some(ref asst_txt) = notice.last_assistant_message {
        let already_last = messages
            .last()
            .map(|m| m.role == "assistant" && m.text.trim() == asst_txt.trim())
            .unwrap_or(false);
        if !already_last {
            messages.push(ExternalChatMessage {
                id: format!("hook-a-{}", notice.timestamp_ms),
                role: "assistant".to_string(),
                text: asst_txt.clone(),
                streaming: false,
            });
        }
    }

    let mut ws_path = notice.cwd.clone().unwrap_or_default();
    let mut ws_label = if ws_path.is_empty() {
        match harness {
            "antigravity" => "CLI Project".to_string(),
            "claude_code" => "Claude Code".to_string(),
            _ => "Codex".to_string(),
        }
    } else {
        basename_of_path(&ws_path)
    };
    let mut ws_id = if harness == "antigravity" && ws_path.is_empty() {
        "default-cli-project".to_string()
    } else {
        ws_path.clone()
    };

    let fallback_title = format!("{ws_label} · 实时会话");
    let mut title = title_from_messages(&messages, &fallback_title);
    let mut title_authoritative = false;

    if harness == "antigravity" && !session_id.is_empty() {
        if let Some((db_ws_id, db_ws_path, db_ws_label, db_title)) =
            crate::harness::lookup_antigravity_conversation_meta(&session_id)
        {
            if !db_ws_id.is_empty() {
                ws_id = db_ws_id;
            }
            if !db_ws_path.is_empty() {
                ws_path = db_ws_path;
            }
            if !db_ws_label.is_empty() {
                ws_label = db_ws_label;
            }
            if !db_title.is_empty() {
                title = db_title;
                title_authoritative = true;
            }
        }
    }

    Some(HarnessSyncPayload {
        harness: harness.to_string(),
        workspace_id: ws_id,
        workspace_path: ws_path,
        workspace_label: ws_label,
        session_id,
        title,
        title_authoritative,
        messages,
        running: notice.running.unwrap_or(false),
        source: "hook".to_string(),
        updated_at_ms: notice.timestamp_ms,
    })
}

fn create_inbound_sync_pipe(pipe_full: &str) -> Option<HANDLE> {
    let pipe_w = to_wide(pipe_full);
    let sid = current_user_sid_string();
    let sddl = sid.map(|s| format!("D:(A;;GA;;;{s})"));
    let sddl_w = sddl.as_deref().map(to_wide);
    let mut sd = PSECURITY_DESCRIPTOR::default();
    let mut sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: false.into(),
    };
    let sa_ptr = if let Some(ref sw) = sddl_w {
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sw.as_ptr()),
                SDDL_REVISION_1,
                &mut sd,
                None,
            )
            .is_ok()
        };
        if ok {
            sa.lpSecurityDescriptor = sd.0;
            Some(&sa as *const SECURITY_ATTRIBUTES)
        } else {
            None
        }
    } else {
        None
    };

    let handle_res = unsafe {
        CreateNamedPipeW(
            PCWSTR(pipe_w.as_ptr()),
            PIPE_ACCESS_INBOUND,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            16,
            0,
            65536,
            0,
            sa_ptr,
        )
    };
    if !sd.0.is_null() {
        unsafe {
            let _ = LocalFree(Some(HLOCAL(sd.0)));
        }
    }

    if handle_res != INVALID_HANDLE_VALUE && !handle_res.0.is_null() {
        Some(handle_res)
    } else {
        None
    }
}

fn find_newest_codex_rollout(home: &Path) -> Option<(PathBuf, i64, u64)> {
    let sessions_root = home.join(".codex").join("sessions");
    let mut newest_year: Option<PathBuf> = None;
    let mut best_y = String::new();
    for entry in fs::read_dir(&sessions_root).ok()?.flatten() {
        if entry.path().is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name >= best_y {
                best_y = name;
                newest_year = Some(entry.path());
            }
        }
    }
    let year_dir = newest_year?;
    let mut newest_month: Option<PathBuf> = None;
    let mut best_m = String::new();
    for entry in fs::read_dir(&year_dir).ok()?.flatten() {
        if entry.path().is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name >= best_m {
                best_m = name;
                newest_month = Some(entry.path());
            }
        }
    }
    let month_dir = newest_month?;
    let mut day_dirs: Vec<(String, PathBuf)> = fs::read_dir(&month_dir)
        .ok()?
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| (e.file_name().to_string_lossy().to_string(), e.path()))
        .collect();
    day_dirs.sort_by(|a, b| b.0.cmp(&a.0));

    // Check up to the 2 most recent day folders so midnight rollovers never miss
    let mut best: Option<(PathBuf, i64, u64)> = None;
    for (_, day_dir) in day_dirs.into_iter().take(2) {
        let Ok(files) = fs::read_dir(&day_dir) else {
            continue;
        };
        for f in files.flatten() {
            let p = f.path();
            if p.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            if let Some((mtime, len)) = file_sig(&p) {
                if len > 0 && best.as_ref().map(|b| mtime > b.1).unwrap_or(true) {
                    best = Some((p, mtime, len));
                }
            }
        }
    }
    best
}

fn parse_codex_rollout_meta(raw: &str) -> (String, String) {
    for line in raw.lines().take(20) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) == Some("session_meta") {
            if let Some(p) = v.get("payload") {
                let id = p
                    .get("id")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let cwd = p
                    .get("cwd")
                    .and_then(|s| s.as_str())
                    .map(clean_windows_path)
                    .unwrap_or_default();
                if !id.is_empty() {
                    return (id, cwd);
                }
            }
        }
    }
    (String::new(), String::new())
}

fn find_newest_claude_jsonl(home: &Path) -> Option<(PathBuf, i64, u64)> {
    let projects_root = home.join(".claude").join("projects");
    let mut best: Option<(PathBuf, i64, u64)> = None;
    for proj in fs::read_dir(&projects_root).ok()?.flatten() {
        let pp = proj.path();
        if !pp.is_dir() {
            continue;
        }
        let Ok(files) = fs::read_dir(&pp) else {
            continue;
        };
        for f in files.flatten() {
            let fp = f.path();
            if fp.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let stem = fp.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if stem.starts_with("agent-") {
                continue;
            }
            if let Some((mtime, len)) = file_sig(&fp) {
                if len > 0 && best.as_ref().map(|b| mtime > b.1).unwrap_or(true) {
                    best = Some((fp, mtime, len));
                }
            }
        }
    }
    best
}

fn parse_claude_jsonl_meta(raw: &str, fallback_stem: &str) -> (String, String) {
    let mut sid = fallback_stem.to_string();
    let mut cwd = String::new();
    for line in raw.lines().take(60) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            continue;
        };
        if let Some(s) = v.get("sessionId").and_then(|x| x.as_str()) {
            if !s.trim().is_empty() {
                sid = s.trim().to_string();
            }
        }
        if cwd.is_empty() {
            if let Some(c) = v.get("cwd").and_then(|x| x.as_str()) {
                if !c.trim().is_empty() {
                    cwd = clean_windows_path(c);
                }
            }
        }
        if !cwd.is_empty() && !sid.is_empty() {
            break;
        }
    }
    (sid, cwd)
}

fn find_newest_antigravity_transcript(home: &Path) -> Option<(PathBuf, String, i64, u64)> {
    let mut best: Option<(PathBuf, String, i64, u64)> = None;
    for sub in ["antigravity", "antigravity-cli"] {
        let brain = home.join(".gemini").join(sub).join("brain");
        let Ok(entries) = fs::read_dir(&brain) else {
            continue;
        };
        for ent in entries.flatten() {
            let dir = ent.path();
            if !dir.is_dir() {
                continue;
            }
            let cid = ent.file_name().to_string_lossy().to_string();
            let tp = dir
                .join(".system_generated")
                .join("logs")
                .join("transcript.jsonl");
            if let Some((mtime, len)) = file_sig(&tp) {
                if len > 0 && best.as_ref().map(|b| mtime > b.2).unwrap_or(true) {
                    best = Some((tp, cid, mtime, len));
                }
            }
        }
    }
    best
}

/// Starts two lightweight daemon threads inside Anything-to-Prompt:
/// 1. Named Pipe receiver (`\\.\pipe\atp-harness-sync-v1`) for instant (<10ms) hook push from Antigravity / Claude Code plugins.
/// 2. Zero-invasion read-only JSONL tail watcher (650ms cadence) across Codex / Claude Code / Antigravity so live desktop & terminal conversations stream into the bar automatically.
pub fn start_live_sync_services(app: AppHandle, enabled_flag: Arc<AtomicBool>) {
    // Thread 1: Named Pipe hook event listener
    let app_pipe = app.clone();
    let flag_pipe = enabled_flag.clone();
    let _ = std::thread::Builder::new()
        .name("atp-sync-pipe".to_string())
        .spawn(move || {
            let full_pipe = sync_pipe_full_path();
            loop {
                let Some(handle) = create_inbound_sync_pipe(&full_pipe) else {
                    std::thread::sleep(Duration::from_millis(800));
                    continue;
                };
                let connected = unsafe {
                    match ConnectNamedPipe(handle, None) {
                        Ok(()) => true,
                        Err(e) => {
                            let win_err = windows::core::Error::from_thread();
                            win_err.code().0 as u32 == 0x80070217 || e.code().0 as u32 == 0x80070217
                        }
                    }
                };
                if !connected {
                    unsafe {
                        let _ = CloseHandle(handle);
                    }
                    continue;
                }
                let mut file = unsafe { std::fs::File::from_raw_handle(handle.0) };
                {
                    let reader = BufReader::new(&mut file);
                    for line in reader.lines().map_while(Result::ok) {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        if let Ok(ctrl) = serde_json::from_str::<PipeControlCommand>(trimmed) {
                            crate::handle_pipe_control_command(
                                &app_pipe,
                                &ctrl.control,
                                ctrl.arg.as_deref(),
                            );
                            continue;
                        }
                        if !flag_pipe.load(Ordering::Acquire) {
                            continue;
                        }
                        if let Ok(notice) = serde_json::from_str::<HookSyncNotice>(trimmed) {
                            if let Some(payload) = build_sync_payload_from_hook(&notice) {
                                push_harness_sync_to_bar(&app_pipe, &payload);
                            }
                        }
                    }
                }
                unsafe {
                    // Disconnect while `file` still owns `handle` (prevents Use-After-Close!).
                    let _ = DisconnectNamedPipe(handle);
                }
                drop(file);
            }
        });

    // Thread 2: Zero-invasion read-only live JSONL watcher (Codex + Claude Code + Antigravity)
    let app_watch = app;
    let flag_watch = enabled_flag;
    let _ = std::thread::Builder::new()
        .name("atp-sync-tail".to_string())
        .spawn(move || {
            let Some(home) = dirs::home_dir() else {
                return;
            };
            // Seed initial baselines so existing historical files do not fire spurious sync events on boot
            let mut seen: HashMap<String, (PathBuf, i64, u64)> = HashMap::new();
            if let Some((p, mt, ln)) = find_newest_codex_rollout(&home) {
                seen.insert("codex".to_string(), (p, mt, ln));
            }
            if let Some((p, mt, ln)) = find_newest_claude_jsonl(&home) {
                seen.insert("claude_code".to_string(), (p, mt, ln));
            }
            if let Some((p, _, mt, ln)) = find_newest_antigravity_transcript(&home) {
                seen.insert("antigravity".to_string(), (p, mt, ln));
            }

            loop {
                std::thread::sleep(Duration::from_millis(650));
                if !flag_watch.load(Ordering::Acquire) {
                    continue;
                }

                // 1. Codex live rollout watcher
                if let Some((p, mt, ln)) = find_newest_codex_rollout(&home) {
                    let changed = seen
                        .get("codex")
                        .map(|(op, omt, oln)| *op != p || mt > *omt || ln != *oln)
                        .unwrap_or(true);
                    if changed {
                        seen.insert("codex".to_string(), (p.clone(), mt, ln));
                        if let Ok(raw) = fs::read_to_string(&p) {
                            let (sid, cwd) = parse_codex_rollout_meta(&raw);
                            let msgs = parse_codex_rollout_lines(&raw);
                            if !sid.is_empty() && !msgs.is_empty() {
                                let label = if cwd.is_empty() {
                                    "Codex".to_string()
                                } else {
                                    basename_of_path(&cwd)
                                };
                                let db_name = crate::harness::lookup_codex_thread_name(&sid);
                                let title_authoritative = db_name.is_some();
                                let title = db_name
                                    .unwrap_or_else(|| title_from_messages(&msgs, &label));
                                let payload = HarnessSyncPayload {
                                    harness: "codex".to_string(),
                                    workspace_id: cwd.clone(),
                                    workspace_path: cwd,
                                    workspace_label: label.clone(),
                                    session_id: sid,
                                    title,
                                    title_authoritative,
                                    messages: msgs,
                                    running: false,
                                    source: "watch".to_string(),
                                    updated_at_ms: mt,
                                };
                                push_harness_sync_to_bar(&app_watch, &payload);
                            }
                        }
                    }
                }

                // 2. Claude Code live JSONL watcher
                if let Some((p, mt, ln)) = find_newest_claude_jsonl(&home) {
                    let changed = seen
                        .get("claude_code")
                        .map(|(op, omt, oln)| *op != p || mt > *omt || ln != *oln)
                        .unwrap_or(true);
                    if changed {
                        seen.insert("claude_code".to_string(), (p.clone(), mt, ln));
                        if let Ok(raw) = fs::read_to_string(&p) {
                            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                            let (sid, cwd) = parse_claude_jsonl_meta(&raw, stem);
                            let msgs = parse_claude_code_transcript_lines(&raw);
                            if !sid.is_empty() && !msgs.is_empty() {
                                let label = if cwd.is_empty() {
                                    "Claude Code".to_string()
                                } else {
                                    basename_of_path(&cwd)
                                };
                                let payload = HarnessSyncPayload {
                                    harness: "claude_code".to_string(),
                                    workspace_id: cwd.clone(),
                                    workspace_path: cwd,
                                    workspace_label: label.clone(),
                                    session_id: sid,
                                    title: title_from_messages(&msgs, &label),
                                    title_authoritative: false,
                                    messages: msgs,
                                    running: false,
                                    source: "watch".to_string(),
                                    updated_at_ms: mt,
                                };
                                push_harness_sync_to_bar(&app_watch, &payload);
                            }
                        }
                    }
                }

                // 3. Antigravity live transcript watcher
                if let Some((p, cid, mt, ln)) = find_newest_antigravity_transcript(&home) {
                    let changed = seen
                        .get("antigravity")
                        .map(|(op, omt, oln)| *op != p || mt > *omt || ln != *oln)
                        .unwrap_or(true);
                    if changed {
                        seen.insert("antigravity".to_string(), (p.clone(), mt, ln));
                        if let Ok(raw) = fs::read_to_string(&p) {
                            let msgs = parse_antigravity_transcript_lines(&raw);
                            if !cid.is_empty() && !msgs.is_empty() {
                                let (ws_id, ws_path, ws_label, title, title_authoritative) =
                                    if let Some((db_ws_id, db_ws_path, db_ws_label, db_title)) =
                                        crate::harness::lookup_antigravity_conversation_meta(&cid)
                                    {
                                        let has_title = !db_title.is_empty();
                                        (
                                            db_ws_id,
                                            db_ws_path,
                                            db_ws_label,
                                            if has_title {
                                                db_title
                                            } else {
                                                title_from_messages(&msgs, "Antigravity 实时对话")
                                            },
                                            has_title,
                                        )
                                    } else {
                                        (
                                            "default-cli-project".to_string(),
                                            String::new(),
                                            "Antigravity".to_string(),
                                            title_from_messages(&msgs, "Antigravity 实时对话"),
                                            false,
                                        )
                                    };
                                let payload = HarnessSyncPayload {
                                    harness: "antigravity".to_string(),
                                    workspace_id: ws_id,
                                    workspace_path: ws_path,
                                    workspace_label: ws_label,
                                    session_id: cid,
                                    title,
                                    title_authoritative,
                                    messages: msgs,
                                    running: false,
                                    source: "watch".to_string(),
                                    updated_at_ms: mt,
                                };
                                push_harness_sync_to_bar(&app_watch, &payload);
                            }
                        }
                    }
                }
            }
        });
}

pub fn antigravity_sidecar_pipe_full_path() -> String {
    format!(r"\\.\pipe\{ANTIGRAVITY_SIDECAR_PIPE_SHORT}")
}

fn enable_antigravity_sidecar_in_config(home: &Path, enable: bool) {
    let cfg_path = home.join(".gemini").join("config").join("config.json");
    let mut root: serde_json::Value = if let Ok(raw) = fs::read_to_string(&cfg_path) {
        serde_json::from_str(&raw).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        if !enable {
            return;
        }
        serde_json::json!({})
    };
    let Some(obj) = root.as_object_mut() else {
        return;
    };
    let sidecars = obj
        .entry("sidecars")
        .or_insert_with(|| serde_json::json!({}));
    let Some(sc_map) = sidecars.as_object_mut() else {
        return;
    };
    if !enable {
        if let Some(entry) = sc_map.get_mut(ANTIGRAVITY_SIDECAR_ID).and_then(|v| v.as_object_mut()) {
            entry.insert("enabled".to_string(), serde_json::Value::Bool(false));
        }
    } else {
        let entry = sc_map
            .entry(ANTIGRAVITY_SIDECAR_ID)
            .or_insert_with(|| serde_json::json!({}));
        if let Some(em) = entry.as_object_mut() {
            em.insert("enabled".to_string(), serde_json::Value::Bool(true));
            let has_pid = em
                .get("projectId")
                .and_then(|v| v.as_str())
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            if !has_pid {
                if let Some(pid) = crate::harness::latest_antigravity_desktop_project_id() {
                    em.insert("projectId".to_string(), serde_json::Value::String(pid));
                }
            }
        }
    }
    if let Ok(pretty) = serde_json::to_string_pretty(&root) {
        let _ = write_if_changed(&cfg_path, &pretty);
    }
}

/// Resolves the direct executable and leading subcommand for Antigravity's official `agentapi`.
/// On Windows, `agentapi.bat` wraps `language_server.exe agentapi %*`. Calling the `.exe` directly
/// avoids `cmd.exe` batch mangling of newlines, `%`, `&`, and quotes in user prompts.
fn resolve_agentapi_command() -> Option<(PathBuf, Vec<String>)> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            for name in ["agentapi.exe", "agentapi.bat", "agentapi.cmd", "agentapi"] {
                let p = dir.join(name);
                if p.is_file() {
                    candidates.push(p);
                }
            }
        }
    }
    if let Some(home) = dirs::home_dir() {
        for name in ["agentapi.exe", "agentapi.bat", "agentapi.cmd"] {
            let p = home.join(".gemini").join("antigravity").join("bin").join(name);
            if p.is_file() {
                candidates.push(p);
            }
        }
    }
    for cand in candidates {
        let lower = cand.to_string_lossy().to_ascii_lowercase();
        if lower.ends_with(".exe") {
            return Some((cand, Vec::new()));
        }
        if lower.ends_with(".bat") || lower.ends_with(".cmd") {
            if let Ok(raw) = fs::read_to_string(&cand) {
                for line in raw.lines() {
                    let t = line.trim();
                    if t.is_empty() || t.to_ascii_lowercase().starts_with("@echo") {
                        continue;
                    }
                    if let Some(rest) = t.strip_prefix('"') {
                        if let Some(end_q) = rest.find('"') {
                            let exe_str = &rest[..end_q];
                            let exe_path = PathBuf::from(exe_str);
                            if exe_path.is_file() {
                                let after = rest[end_q + 1..].trim();
                                let leading: Vec<String> = after
                                    .split_whitespace()
                                    .take_while(|tok| !tok.contains('%'))
                                    .map(|s| s.to_string())
                                    .collect();
                                return Some((exe_path, leading));
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// Guards against Windows's 32,767-char `CreateProcessW` command-line limit when passing
/// prompts containing many `[SW-TARGET v2]` JSON capsules to CLI tools like `agentapi`.
/// If `prompt` exceeds 24,000 UTF-16 code units, writes the full UTF-8 prompt to a temp file
/// and returns a compact reference prompt pointing to that file.
pub fn bound_cli_prompt_arg(prompt: &str) -> String {
    const MAX_CLI_UTF16_CHARS: usize = 24_000;
    let utf16_len = prompt.encode_utf16().count();
    if utf16_len <= MAX_CLI_UTF16_CHARS {
        return prompt.to_string();
    }
    let dir = std::env::temp_dir().join("atp-prompts");
    let _ = fs::create_dir_all(&dir);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = dir.join(format!("prompt-{}-{ts}.md", std::process::id()));
    if fs::write(&path, prompt).is_ok() {
        let preview: String = prompt.chars().take(1200).collect();
        return format!(
            "[提示：本次消息总长 {utf16_len} 字符，超过 Windows 命令行安全上限，完整正文与所有引用胶囊已写入文件，请先读取该文件再作答]\n完整内容路径: {}\n\n--- 前 1200 字预览 ---\n{}",
            path.display(),
            preview
        );
    }
    prompt.chars().take(16_000).collect()
}

fn run_command_with_timeout(
    mut cmd: Command,
    timeout: Duration,
) -> std::io::Result<std::process::Output> {
    use std::io::Read;
    let mut child = cmd
        .creation_flags(CREATE_NO_WINDOW.0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(ref mut r) = out_pipe {
            let _ = r.read_to_end(&mut buf);
        }
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(ref mut r) = err_pipe {
            let _ = r.read_to_end(&mut buf);
        }
        buf
    });
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        if let Some(st) = child.try_wait()? {
            break st;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = out_thread.join();
            let _ = err_thread.join();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("agentapi 执行超时（>{}s）", timeout.as_secs()),
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let stdout = out_thread.join().unwrap_or_default();
    let stderr = err_thread.join().unwrap_or_default();
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

fn execute_agentapi_request(req: &AntigravitySidecarRequest) -> AntigravitySidecarResponse {
    if req.action == "ping" {
        return AntigravitySidecarResponse {
            ok: true,
            conversation_id: None,
            error: None,
        };
    }
    let Some((exe, leading)) = resolve_agentapi_command() else {
        return AntigravitySidecarResponse {
            ok: false,
            conversation_id: None,
            error: Some("未找到官方 agentapi 可执行文件".to_string()),
        };
    };

    let mut cmd = Command::new(&exe);
    for arg in &leading {
        cmd.arg(arg);
    }
    match req.action.as_str() {
        "send_message" => {
            let cid = req.conversation_id.as_deref().unwrap_or("").trim().replace('\0', "");
            let prompt = bound_cli_prompt_arg(&req.prompt.as_deref().unwrap_or("").replace('\0', ""));
            if cid.is_empty() || prompt.trim().is_empty() {
                return AntigravitySidecarResponse {
                    ok: false,
                    conversation_id: None,
                    error: Some("缺少 conversationId 或消息内容".to_string()),
                };
            }
            let title = req
                .title
                .as_deref()
                .map(|s| s.replace('\0', ""))
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "Anything-to-Prompt".to_string());
            cmd.arg("send-message")
                .arg(format!("--title={title}"))
                .arg("--")
                .arg(cid)
                .arg(prompt);
        }
        "new_conversation" => {
            let prompt = bound_cli_prompt_arg(&req.prompt.as_deref().unwrap_or("").replace('\0', ""));
            if prompt.trim().is_empty() {
                return AntigravitySidecarResponse {
                    ok: false,
                    conversation_id: None,
                    error: Some("消息内容为空".to_string()),
                };
            }
            cmd.arg("new-conversation");
            if let Some(model) = req
                .model
                .as_deref()
                .map(|s| s.trim().replace('\0', ""))
                .filter(|s| !s.is_empty())
            {
                cmd.arg(format!("--model={model}"));
            }
            if let Some(title) = req
                .title
                .as_deref()
                .map(|s| s.replace('\0', ""))
                .filter(|s| !s.trim().is_empty())
            {
                cmd.arg(format!("--title={title}"));
            }
            cmd.arg("--").arg(prompt);
        }
        other => {
            return AntigravitySidecarResponse {
                ok: false,
                conversation_id: None,
                error: Some(format!("不支持的 sidecar action: {other}")),
            };
        }
    }

    match run_command_with_timeout(cmd, Duration::from_secs(45)) {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            let parsed_json = serde_json::from_str::<serde_json::Value>(&stdout).ok();
            let json_err = parsed_json
                .as_ref()
                .and_then(|v| v.get("error"))
                .and_then(|e| e.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.to_string());
            if out.status.success() && json_err.is_none() {
                let cid = parsed_json
                    .as_ref()
                    .and_then(|v| {
                        v.pointer("/response/sendMessage/recipientId")
                            .or_else(|| v.pointer("/response/newConversation/conversationId"))
                            .or_else(|| v.pointer("/response/conversationId"))
                            .and_then(|x| x.as_str())
                    })
                    .map(|s| s.to_string())
                    .or_else(|| req.conversation_id.clone());
                AntigravitySidecarResponse {
                    ok: true,
                    conversation_id: cid,
                    error: None,
                }
            } else {
                let err_msg = json_err
                    .or_else(|| (!stderr.is_empty()).then_some(stderr))
                    .or_else(|| (!stdout.is_empty()).then_some(stdout))
                    .unwrap_or_else(|| format!("agentapi 退出码 {:?}", out.status.code()));
                AntigravitySidecarResponse {
                    ok: false,
                    conversation_id: req.conversation_id.clone(),
                    error: Some(err_msg),
                }
            }
        }
        Err(e) => AntigravitySidecarResponse {
            ok: false,
            conversation_id: req.conversation_id.clone(),
            error: Some(format!("调用 agentapi 失败: {e}")),
        },
    }
}

fn create_duplex_user_pipe(pipe_full: &str) -> Option<HANDLE> {
    let pipe_w = to_wide(pipe_full);
    let sid = current_user_sid_string();
    let sddl = sid.map(|s| format!("D:(A;;GA;;;{s})"));
    let sddl_w = sddl.as_deref().map(to_wide);
    let mut sd = PSECURITY_DESCRIPTOR::default();
    let mut sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: false.into(),
    };
    let sa_ptr = if let Some(ref sw) = sddl_w {
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sw.as_ptr()),
                SDDL_REVISION_1,
                &mut sd,
                None,
            )
            .is_ok()
        };
        if ok {
            sa.lpSecurityDescriptor = sd.0;
            Some(&sa as *const SECURITY_ATTRIBUTES)
        } else {
            None
        }
    } else {
        None
    };

    let handle_res = unsafe {
        CreateNamedPipeW(
            PCWSTR(pipe_w.as_ptr()),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            8,
            65536,
            65536,
            0,
            sa_ptr,
        )
    };
    if !sd.0.is_null() {
        unsafe {
            let _ = LocalFree(Some(HLOCAL(sd.0)));
        }
    }

    if handle_res != INVALID_HANDLE_VALUE && !handle_res.0.is_null() {
        Some(handle_res)
    } else {
        None
    }
}

/// Runs as an official Antigravity Plugin Sidecar (`anything-to-prompt.exe --antigravity-sidecar`).
/// Antigravity Desktop launches this process in the background with official `agentapi` on PATH,
/// enabling 100% silent background prompt delivery into existing Antigravity Desktop conversations.
pub fn run_antigravity_sidecar_cli() {
    crate::mcp_sidecar::spawn_bar_companion_if_needed(Some("antigravity"));
    let full_pipe = antigravity_sidecar_pipe_full_path();
    loop {
        let Some(handle) = create_duplex_user_pipe(&full_pipe) else {
            std::thread::sleep(Duration::from_millis(800));
            continue;
        };
        let connected = unsafe {
            match ConnectNamedPipe(handle, None) {
                Ok(()) => true,
                Err(e) => {
                    let win_err = windows::core::Error::from_thread();
                    win_err.code().0 as u32 == 0x80070217 || e.code().0 as u32 == 0x80070217
                }
            }
        };
        if !connected {
            unsafe {
                let _ = CloseHandle(handle);
            }
            continue;
        }
        // Spawn a worker per accepted connection so the accept loop immediately creates the next
        // listening pipe instance (prevents concurrent calls or rapid retries from hitting SIDE_OFFLINE).
        let raw_handle = handle.0 as isize;
        let _ = std::thread::Builder::new()
            .name("atp-ag-sidecar-req".to_string())
            .spawn(move || {
                let h = HANDLE(raw_handle as *mut std::ffi::c_void);
                let mut file = unsafe { std::fs::File::from_raw_handle(h.0) };
                let mut line = String::new();
                {
                    let mut reader = BufReader::new(&mut file);
                    let _ = reader.read_line(&mut line);
                }
                let resp = match serde_json::from_str::<AntigravitySidecarRequest>(line.trim()) {
                    Ok(req) => execute_agentapi_request(&req),
                    Err(e) => AntigravitySidecarResponse {
                        ok: false,
                        conversation_id: None,
                        error: Some(format!("无效请求: {e}")),
                    },
                };
                if let Ok(resp_json) = serde_json::to_string(&resp) {
                    let _ = file.write_all(resp_json.as_bytes());
                    let _ = file.write_all(b"\n");
                    let _ = file.flush();
                }
                unsafe {
                    // CRITICAL on Win32 Named Pipes: wait until client reads all response bytes
                    // before DisconnectNamedPipe, and keep `file` alive until after disconnect!
                    let _ = FlushFileBuffers(h);
                    let _ = DisconnectNamedPipe(h);
                }
                drop(file);
            });
    }
}

/// Calls the running Antigravity Plugin Sidecar over the per-user Named Pipe.
pub fn call_antigravity_sidecar(
    req: &AntigravitySidecarRequest,
) -> Result<AntigravitySidecarResponse, String> {
    let pipe_path = antigravity_sidecar_pipe_full_path();
    let payload = serde_json::to_string(req).map_err(|e| format!("序列化失败: {e}"))?;
    let mut last_err = String::new();
    for _ in 0..3 {
        match OpenOptions::new().read(true).write(true).open(&pipe_path) {
            Ok(mut file) => {
                file.write_all(payload.as_bytes())
                    .and_then(|_| file.write_all(b"\n"))
                    .and_then(|_| file.flush())
                    .map_err(|e| format!("写入 Sidecar 管道失败: {e}"))?;
                let mut reader = BufReader::new(file);
                let mut resp_line = String::new();
                reader
                    .read_line(&mut resp_line)
                    .map_err(|e| format!("读取 Sidecar 响应失败: {e}"))?;
                return serde_json::from_str::<AntigravitySidecarResponse>(resp_line.trim())
                    .map_err(|e| format!("解析 Sidecar 响应失败: {e}"));
            }
            Err(e) => {
                last_err = e.to_string();
                std::thread::sleep(Duration::from_millis(60));
            }
        }
    }
    Err(format!("SIDE_OFFLINE:{last_err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_antigravity_and_claude_and_codex_hook_payloads() {
        let ag_raw = r#"{
            "conversationId": "ag-conv-123",
            "workspacePaths": ["D:\\Work\\Demo"],
            "transcriptPath": "C:\\Users\\Me\\.gemini\\antigravity\\brain\\ag-conv-123\\transcript.jsonl",
            "lastUserInput": "优化编译速度",
            "hookEventName": "PreInvocation",
            "cwd": "D:\\Work\\Demo"
        }"#;
        let ag = parse_hook_payload("antigravity", ag_raw).unwrap();
        assert_eq!(ag.harness, "antigravity");
        assert_eq!(ag.session_id.as_deref(), Some("ag-conv-123"));
        assert_eq!(ag.cwd.as_deref(), Some(r"D:\Work\Demo"));
        assert_eq!(ag.running, Some(true));

        let cc_raw = r#"{
            "session_id": "cc-sess-999",
            "cwd": "D:\\AI\\Ollama Claude",
            "transcript_path": "C:\\Users\\Me\\.claude\\projects\\p\\cc-sess-999.jsonl",
            "hook_event_name": "Stop",
            "last_assistant_message": "已完成重构"
        }"#;
        let cc = parse_hook_payload("claude_code", cc_raw).unwrap();
        assert_eq!(cc.harness, "claude_code");
        assert_eq!(cc.session_id.as_deref(), Some("cc-sess-999"));
        assert_eq!(cc.running, Some(false));
        assert_eq!(cc.last_assistant_message.as_deref(), Some("已完成重构"));

        let cx_raw = r#"{
            "type": "agent-turn-complete",
            "thread-id": "cx-thread-7",
            "cwd": "D:\\Repo",
            "last-assistant-message": "All tests pass."
        }"#;
        let cx = parse_hook_payload("codex", cx_raw).unwrap();
        assert_eq!(cx.harness, "codex");
        assert_eq!(cx.session_id.as_deref(), Some("cx-thread-7"));
        assert_eq!(cx.running, Some(false));
    }

    #[test]
    fn generates_valid_antigravity_and_claude_hooks_json() {
        let exe = Path::new(r"C:\Tools\anything-to-prompt.exe");
        let ag_json: serde_json::Value =
            serde_json::from_str(&build_antigravity_hooks_json(exe)).unwrap();
        assert!(ag_json["anything-to-prompt-sync"]["SessionStart"].is_array());
        assert!(ag_json["anything-to-prompt-sync"]["Stop"].is_array());

        let cc_json: serde_json::Value =
            serde_json::from_str(&build_claude_code_hooks_json(exe)).unwrap();
        assert!(cc_json["hooks"]["SessionStart"].is_array());
        assert!(cc_json["hooks"]["UserPromptSubmit"].is_array());
        assert!(cc_json["hooks"]["Stop"].is_array());
        let cmd = cc_json["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(cmd.contains("C:/Tools/anything-to-prompt.exe"));
        assert!(cmd.contains("--hook-event claude_code"));
    }

    #[test]
    fn pipe_roundtrip_delivers_hook_notice_locally() {
        let pipe_name = format!(r"\\.\pipe\atp-sync-test-{}", std::process::id());
        let pipe_for_srv = pipe_name.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let srv = std::thread::spawn(move || {
            let h = create_inbound_sync_pipe(&pipe_for_srv).expect("create pipe");
            let _ = tx.send(());
            unsafe {
                let _ = ConnectNamedPipe(h, None);
            }
            let file = unsafe { std::fs::File::from_raw_handle(h.0) };
            let mut line = String::new();
            let _ = BufReader::new(file).read_line(&mut line);
            line
        });

        rx.recv().unwrap();
        std::thread::sleep(Duration::from_millis(30));
        let notice = HookSyncNotice {
            harness: "claude_code".to_string(),
            event: "Stop".to_string(),
            session_id: Some("sess-pipe-1".to_string()),
            cwd: Some(r"D:\Work".to_string()),
            transcript_path: None,
            last_user_input: None,
            last_assistant_message: Some("Done".to_string()),
            running: Some(false),
            timestamp_ms: 123456,
        };
        send_notice_to_pipe(&pipe_name, &notice).expect("send notice");
        let received = srv.join().unwrap();
        let parsed: HookSyncNotice = serde_json::from_str(received.trim()).unwrap();
        assert_eq!(parsed.session_id.as_deref(), Some("sess-pipe-1"));
        assert_eq!(parsed.last_assistant_message.as_deref(), Some("Done"));
    }

    #[test]
    fn duplex_pipe_flush_prevents_error_233_across_rapid_roundtrips() {
        let pipe = format!(
            r"\\.\pipe\atp-duplex-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let pipe_srv = pipe.clone();
        let srv = std::thread::spawn(move || {
            for i in 0..8 {
                let h = create_duplex_user_pipe(&pipe_srv).expect("create duplex pipe");
                let connected = unsafe {
                    match ConnectNamedPipe(h, None) {
                        Ok(()) => true,
                        Err(e) => e.code().0 as u32 == 0x80070217,
                    }
                };
                assert!(connected);
                let mut f = unsafe { std::fs::File::from_raw_handle(h.0) };
                let mut req_line = String::new();
                {
                    let mut r = BufReader::new(&mut f);
                    let _ = r.read_line(&mut req_line);
                }
                let resp = format!("{{\"ok\":true,\"conversationId\":\"c-{i}\"}}\n");
                f.write_all(resp.as_bytes()).expect("write resp");
                unsafe {
                    let _ = FlushFileBuffers(h);
                    let _ = DisconnectNamedPipe(h);
                }
                drop(f);
            }
        });

        for i in 0..8 {
            let mut resp_line = String::new();
            let mut ok = false;
            for _ in 0..25 {
                if let Ok(mut client) = OpenOptions::new().read(true).write(true).open(&pipe) {
                    client.write_all(b"{\"action\":\"send_message\"}\n").unwrap();
                    let mut r = BufReader::new(client);
                    r.read_line(&mut resp_line).expect("must never hit os error 233");
                    ok = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(15));
            }
            assert!(ok, "client failed to connect on iteration {i}");
            assert!(resp_line.contains(&format!("c-{i}")));
        }

        let _ = srv.join();
    }

    #[test]
    fn atomic_sync_exe_replaces_existing_file_and_cleans_stale_backups() {
        let dir = std::env::temp_dir().join(format!("atp-atomic-exe-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let src = dir.join("new.exe");
        let dst = dir.join("anything-to-prompt.exe");
        let stale = dir.join("anything-to-prompt.exe.old.12345");
        fs::write(&src, b"new-binary-payload").unwrap();
        fs::write(&dst, b"old-binary").unwrap();
        fs::write(&stale, b"stale-old").unwrap();

        atomic_sync_exe(&src, &dst).expect("atomic_sync_exe must succeed");
        assert_eq!(fs::read(&dst).unwrap(), b"new-binary-payload");
        assert!(!stale.exists(), "stale .old backup should be cleaned up");

        let _ = fs::remove_dir_all(&dir);
    }
}
