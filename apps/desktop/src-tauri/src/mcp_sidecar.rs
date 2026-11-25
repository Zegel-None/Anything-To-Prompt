//! Lightweight native stdio MCP sidecar (`anything-to-prompt.exe --mcp-sidecar [--harness <id>]`).
//!
//! Loaded automatically by Antigravity (`mcp_config.json`), Claude Code (`.mcp.json`), and Codex (`.mcp.json`):
//! 1. Immediately ensures the single-instance Anything-to-Prompt desktop bar is running (passing `--ensure-running`
//!    so an already-running bar never steals focus or pops up).
//! 2. Serves MCP JSON-RPC 2.0 over stdio (both NDJSON lines and `Content-Length:` framing) exposing
//!    `anything_to_prompt_get_settings` and `anything_to_prompt_update_settings` so users can inspect or adjust
//!    plugin settings directly from any Harness without needing a Windows tray icon.

use crate::config::{AppConfig, OutputTarget};
use crate::harness_sync::{
    apply_external_sync_plugins, query_external_sync_status, send_control_to_pipe,
};
use serde_json::{json, Value};
use std::ffi::OsStr;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::windows::ffi::OsStrExt;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Threading::{
    CreateProcessW, CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, DETACHED_PROCESS,
    PROCESS_INFORMATION, STARTUPINFOW,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FramingMode {
    LineJson,
    ContentLength,
}

fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Spawns the single-instance Anything-to-Prompt desktop bar synchronously via Win32 `CreateProcessW`
/// with `bInheritHandles = false` and `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW`.
/// Guaranteed to launch in ~3ms without inheriting MCP stdio pipes (zero hang, zero COM race).
fn spawn_bar_companion_if_needed(harness: Option<&str>) {
    // If the bar's sync pipe is already reachable, it is already running — no need to spawn!
    if send_control_to_pipe("ping", harness).is_ok() {
        return;
    }
    let exe = std::env::current_exe().unwrap_or_else(|_| AppConfig::runtime_exe_path());
    let exe_str = exe.to_string_lossy().to_string();
    let cmdline = match harness.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        Some(h) => format!("\"{exe_str}\" --ensure-running --launched-by \"{h}\""),
        None => format!("\"{exe_str}\" --ensure-running"),
    };
    let exe_w = to_wide(&exe_str);
    let mut cmd_w = to_wide(&cmdline);
    let si = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut pi = PROCESS_INFORMATION::default();
    let ok = unsafe {
        CreateProcessW(
            PCWSTR(exe_w.as_ptr()),
            Some(PWSTR(cmd_w.as_mut_ptr())),
            None,
            None,
            false, // CRITICAL: never inherit parent MCP stdin/stdout pipes!
            DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW,
            None,
            PCWSTR::null(),
            &si,
            &mut pi,
        )
        .is_ok()
    };
    if ok {
        unsafe {
            let _ = CloseHandle(pi.hThread);
            let _ = CloseHandle(pi.hProcess);
        }
    }
}

fn read_mcp_message(reader: &mut BufReader<io::StdinLock<'_>>) -> Option<(Value, FramingMode)> {
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).ok()?;
        if n == 0 {
            return None;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Check for LSP-style `Content-Length: N` header
        if let Some(rest) = trimmed
            .strip_prefix("Content-Length:")
            .or_else(|| trimmed.strip_prefix("content-length:"))
        {
            let content_len: usize = rest.trim().parse().ok()?;
            // Consume remaining header lines until empty line
            loop {
                line.clear();
                let hn = reader.read_line(&mut line).ok()?;
                if hn == 0 || line.trim().is_empty() {
                    break;
                }
            }
            let mut body = vec![0u8; content_len];
            reader.read_exact(&mut body).ok()?;
            let val = serde_json::from_slice::<Value>(&body).ok()?;
            return Some((val, FramingMode::ContentLength));
        }

        // Standard NDJSON line (used by modern MCP stdio clients)
        if let Ok(val) = serde_json::from_str::<Value>(trimmed) {
            return Some((val, FramingMode::LineJson));
        }
    }
}

fn write_mcp_message(
    writer: &mut impl Write,
    msg: &Value,
    mode: FramingMode,
) -> io::Result<()> {
    let payload = serde_json::to_string(msg)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    match mode {
        FramingMode::ContentLength => {
            write!(
                writer,
                "Content-Length: {}\r\n\r\n{}",
                payload.len(),
                payload
            )?;
        }
        FramingMode::LineJson => {
            writeln!(writer, "{payload}")?;
        }
    }
    writer.flush()
}

pub fn handle_mcp_request(req: &Value) -> Option<Value> {
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let params = req.get("params");

    // Notifications carry no `id` and require no reply
    if id.is_none() {
        return None;
    }
    let id = id.unwrap();

    let result = match method {
        "initialize" => {
            let client_ver = params
                .and_then(|p| p.get("protocolVersion"))
                .and_then(|v| v.as_str())
                .unwrap_or("2024-11-05");
            json!({
                "protocolVersion": client_ver,
                "capabilities": {
                    "tools": { "listChanged": false }
                },
                "serverInfo": {
                    "name": "anything-to-prompt",
                    "version": env!("CARGO_PKG_VERSION")
                }
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({
            "tools": [
                {
                    "name": "anything_to_prompt_get_settings",
                    "description": "Read current Anything-to-Prompt floating bar settings and multi-harness live sync plugin status.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {},
                        "additionalProperties": false
                    }
                },
                {
                    "name": "anything_to_prompt_update_settings",
                    "description": "Update Anything-to-Prompt floating bar settings (outputTarget, startupBreathPulse, hideFromScreenCapture, showTrayIcon, syncExternalPlugins, preferAdmin) or invoke an action (open_settings, preview_breath, open_captures, resync_plugins).",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "outputTarget": { "type": "string", "enum": ["dsh", "dsh_and_clipboard", "clipboard_only"] },
                            "startupBreathPulse": { "type": "boolean" },
                            "hideFromScreenCapture": { "type": "boolean" },
                            "showTrayIcon": { "type": "boolean" },
                            "syncExternalPlugins": { "type": "boolean" },
                            "preferAdmin": { "type": "boolean" },
                            "action": { "type": "string", "enum": ["open_settings", "preview_breath", "open_captures", "resync_plugins"] }
                        }
                    }
                }
            ]
        }),
        "tools/call" => {
            let name = params
                .and_then(|p| p.get("name"))
                .and_then(|s| s.as_str())
                .unwrap_or("");
            let args = params.and_then(|p| p.get("arguments"));
            execute_mcp_tool(name, args)
        }
        _ => {
            return Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32601,
                    "message": format!("Method not found: {method}")
                }
            }));
        }
    };

    Some(json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    }))
}

fn execute_mcp_tool(name: &str, args: Option<&Value>) -> Value {
    match name {
        "anything_to_prompt_get_settings" => {
            let cfg = AppConfig::load_or_create();
            let sync = query_external_sync_status(cfg.sync_external_plugins);
            let payload = json!({
                "version": env!("CARGO_PKG_VERSION"),
                "config": cfg,
                "syncStatus": sync,
                "runtimeExePath": AppConfig::runtime_exe_path().to_string_lossy(),
            });
            json!({
                "content": [
                    {
                        "type": "text",
                        "text": serde_json::to_string_pretty(&payload).unwrap_or_default()
                    }
                ]
            })
        }
        "anything_to_prompt_update_settings" => {
            let mut cfg = AppConfig::load_or_create();
            let mut changed = false;
            if let Some(a) = args {
                if let Some(t) = a.get("outputTarget").and_then(|s| s.as_str()) {
                    cfg.output_target = match t {
                        "dsh_and_clipboard" => OutputTarget::DshAndClipboard,
                        "clipboard_only" => OutputTarget::ClipboardOnly,
                        _ => OutputTarget::Dsh,
                    };
                    changed = true;
                }
                if let Some(b) = a.get("startupBreathPulse").and_then(|v| v.as_bool()) {
                    cfg.startup_breath_pulse = b;
                    changed = true;
                }
                if let Some(b) = a.get("hideFromScreenCapture").and_then(|v| v.as_bool()) {
                    cfg.hide_from_screen_capture = b;
                    changed = true;
                }
                if let Some(b) = a.get("showTrayIcon").and_then(|v| v.as_bool()) {
                    cfg.show_tray_icon = b;
                    changed = true;
                }
                if let Some(b) = a.get("syncExternalPlugins").and_then(|v| v.as_bool()) {
                    cfg.sync_external_plugins = b;
                    let _ = apply_external_sync_plugins(b);
                    changed = true;
                }
                if let Some(b) = a.get("preferAdmin").and_then(|v| v.as_bool()) {
                    cfg.prefer_admin = b;
                    changed = true;
                }
                if changed {
                    let _ = cfg.save();
                    let _ = send_control_to_pipe("reload_config", None);
                }
                if let Some(action) = a.get("action").and_then(|s| s.as_str()) {
                    if action == "resync_plugins" {
                        let _ = apply_external_sync_plugins(cfg.sync_external_plugins);
                    }
                    let _ = send_control_to_pipe(action, None);
                }
            }
            let payload = json!({
                "ok": true,
                "changed": changed,
                "config": cfg
            });
            json!({
                "content": [
                    {
                        "type": "text",
                        "text": serde_json::to_string_pretty(&payload).unwrap_or_default()
                    }
                ]
            })
        }
        other => json!({
            "isError": true,
            "content": [
                {
                    "type": "text",
                    "text": format!("Unknown tool: {other}")
                }
            ]
        }),
    }
}

pub fn run_mcp_sidecar(harness: Option<&str>) {
    spawn_bar_companion_if_needed(harness);

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = stdout.lock();

    while let Some((req, mode)) = read_mcp_message(&mut reader) {
        if let Some(resp) = handle_mcp_request(&req) {
            if write_mcp_message(&mut writer, &resp, mode).is_err() {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_mcp_initialize_and_tools_list() {
        let init_req = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "protocolVersion": "2024-11-05" }
        });
        let init_resp = handle_mcp_request(&init_req).expect("init reply");
        assert_eq!(init_resp["id"], 1);
        assert_eq!(init_resp["result"]["serverInfo"]["name"], "anything-to-prompt");

        let list_req = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
        let list_resp = handle_mcp_request(&list_req).expect("tools list");
        let tools = list_resp["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0]["name"], "anything_to_prompt_get_settings");
        assert_eq!(tools[1]["name"], "anything_to_prompt_update_settings");

        // Notifications without id must not produce responses
        let notif = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert!(handle_mcp_request(&notif).is_none());
    }
}
