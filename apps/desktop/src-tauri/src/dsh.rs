use crate::broker::{BrokerClient, BrokerRequest};
use crate::config::AppConfig;
use serde::{Deserialize, Serialize};
use std::fs;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, Url};
use windows::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
use windows::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstanceRecord {
    pub pid: u32,
    pub port: u16,
    #[serde(alias = "url", alias = "launch_url")]
    pub launch_url: String,
    #[serde(default)]
    pub started_at: serde_json::Value,
    #[serde(default)]
    pub plugin_version: String,
}

#[derive(Debug, Clone)]
pub enum DiscoveryOutcome {
    Connected { port: u16, launch_url: String },
    MissingPlugin { port: u16 },
    Unavailable { reason: String },
}

#[derive(Debug, Default)]
pub struct DshConnectionState {
    pub active_port: Option<u16>,
    pub active_origin: Option<String>,
    pub desired_initial_mode: String,
    pub status_label: String,
}

pub fn is_pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let mut code = 0u32;
        let ok = GetExitCodeProcess(handle, &mut code).is_ok();
        let _ = CloseHandle(handle);
        ok && code == STILL_ACTIVE.0 as u32
    }
}

pub fn is_port_listening(port: u16, timeout_ms: u64) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(timeout_ms)).is_ok()
}

pub fn select_best_instance(
    mut candidates: Vec<InstanceRecord>,
    preferred_port: u16,
) -> Option<InstanceRecord> {
    if candidates.is_empty() {
        return None;
    }
    candidates.sort_by_key(|c| (c.port != preferred_port, c.port));
    candidates.into_iter().next()
}

pub fn scan_live_instances(dsh_home: &Path, preferred_port: u16) -> Option<InstanceRecord> {
    let dirs_to_scan = [
        dsh_home.join("anything-to-prompt").join("instances"),
        dsh_home.join("deepseek-bar").join("instances"),
    ];
    let mut valid = Vec::new();

    for dir in dirs_to_scan {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let Ok(raw) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(rec) = serde_json::from_str::<InstanceRecord>(&raw) else {
                continue;
            };
            if rec.port == 0 || rec.launch_url.trim().is_empty() {
                continue;
            }
            if is_pid_alive(rec.pid)
                && is_port_listening(rec.port, 300)
                && !valid
                    .iter()
                    .any(|existing: &InstanceRecord| existing.port == rec.port)
            {
                valid.push(rec);
            }
        }
    }

    select_best_instance(valid, preferred_port)
}

fn read_fallback_url(config: &AppConfig, dsh_home: &Path) -> Option<(u16, String)> {
    if let Some(custom) = config.dsh_url.as_ref() {
        let trimmed = custom.trim();
        if let Ok(parsed) = Url::parse(trimmed) {
            if let Some(port) = parsed.port_or_known_default() {
                if is_port_listening(port, 300) {
                    return Some((port, trimmed.to_string()));
                }
            }
        }
    }

    let last_web_file = dsh_home.join("LAST-WEB-URL.txt");
    if let Ok(raw) = fs::read_to_string(last_web_file) {
        if let Some(url_str) = extract_url_from_last_web_txt(&raw) {
            if let Ok(parsed) = Url::parse(&url_str) {
                if let Some(port) = parsed.port_or_known_default() {
                    if is_port_listening(port, 300) {
                        return Some((port, url_str));
                    }
                }
            }
        }
    }

    None
}

pub fn extract_url_from_last_web_txt(raw: &str) -> Option<String> {
    for token in raw.split_whitespace() {
        let candidate = token.strip_prefix("URL=").unwrap_or(token).trim();
        if candidate.starts_with("http://127.0.0.1:") || candidate.starts_with("http://localhost:")
        {
            return Some(candidate.to_string());
        }
    }
    None
}

fn resolve_dsh_cmd_path() -> Option<PathBuf> {
    if let Some(appdata) = dirs::data_dir() {
        let npm_cmd = appdata.join("npm").join("dsh.cmd");
        if npm_cmd.exists() {
            return Some(npm_cmd);
        }
    }
    Some(PathBuf::from("dsh.cmd"))
}

pub fn discover_or_start_dsh(
    config: &AppConfig,
    broker: &BrokerClient,
    allow_auto_start: bool,
) -> DiscoveryOutcome {
    let dsh_home = AppConfig::dsh_home_dir();

    if let Some(inst) = scan_live_instances(&dsh_home, config.preferred_port) {
        return DiscoveryOutcome::Connected {
            port: inst.port,
            launch_url: inst.launch_url,
        };
    }

    // If user explicitly configured a custom dshUrl, honor it when reachable
    if let Some(custom) = config.dsh_url.as_ref() {
        let trimmed = custom.trim();
        if let Ok(parsed) = Url::parse(trimmed) {
            if let Some(port) = parsed.port_or_known_default() {
                if is_port_listening(port, 300) {
                    return DiscoveryOutcome::Connected {
                        port,
                        launch_url: trimmed.to_string(),
                    };
                }
            }
        }
    }

    // Preferred port is listening WITHOUT @local/deepseek-bar discovery JSON ->
    // never load unstyled raw DSH page (which would expose its top header bar in 56px window).
    if is_port_listening(config.preferred_port, 300) {
        return DiscoveryOutcome::MissingPlugin {
            port: config.preferred_port,
        };
    }

    if let Some((port, url)) = read_fallback_url(config, &dsh_home) {
        return DiscoveryOutcome::Connected {
            port,
            launch_url: url,
        };
    }

    if !allow_auto_start {
        return DiscoveryOutcome::Unavailable {
            reason: "未检测到运行中的 DeepSeek Harness".to_string(),
        };
    }

    // Spawn `dsh.cmd web --no-open` at medium integrity via broker
    if let Some(dsh_cmd) = resolve_dsh_cmd_path() {
        log::info!(
            "No live DSH found; spawning {:?} web --no-open via medium broker",
            dsh_cmd
        );
        let resp = broker.call(BrokerRequest::Spawn {
            exe: "cmd.exe".to_string(),
            args: vec![
                "/C".to_string(),
                dsh_cmd.to_string_lossy().to_string(),
                "web".to_string(),
                "--no-open".to_string(),
            ],
            cwd: dirs::home_dir().map(|p| p.to_string_lossy().to_string()),
            hidden: true,
        });
        if !resp.ok {
            log::warn!("Failed to spawn DSH via broker: {:?}", resp.error);
        }

        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(600));
            if let Some(inst) = scan_live_instances(&dsh_home, config.preferred_port) {
                return DiscoveryOutcome::Connected {
                    port: inst.port,
                    launch_url: inst.launch_url,
                };
            }
        }

        if is_port_listening(config.preferred_port, 300) {
            return DiscoveryOutcome::MissingPlugin {
                port: config.preferred_port,
            };
        }
    }

    DiscoveryOutcome::Unavailable {
        reason: "无法自动启动或连接 DeepSeek Harness".to_string(),
    }
}

pub fn build_bar_entry_url(launch_url: &str, initial_mode: &str) -> Result<Url, url::ParseError> {
    let mut parsed = Url::parse(launch_url)?;
    // If launch_url already has ?token=..., keep it intact so DSH sets its session cookie first;
    // on_page_load will append ?dsh_bar_clone=1 after the 302 redirect to `/`.
    let has_token = parsed.query_pairs().any(|(k, _)| k == "token");
    if !has_token {
        parsed.query_pairs_mut().append_pair("dsh_bar_clone", "1");
        if initial_mode == "lip" {
            parsed.query_pairs_mut().append_pair("dsb_mode", "lip");
        }
    }
    Ok(parsed)
}

pub fn build_post_auth_bar_url(current: &Url, desired_mode: &str) -> Option<Url> {
    let has_clone = current
        .query_pairs()
        .any(|(k, v)| k == "dsh_bar_clone" && v == "1");
    let has_token = current.query_pairs().any(|(k, _)| k == "token");
    if has_clone || has_token {
        return None;
    }
    let mut next = current.clone();
    next.query_pairs_mut().append_pair("dsh_bar_clone", "1");
    if desired_mode == "lip" {
        next.query_pairs_mut().append_pair("dsb_mode", "lip");
    }
    Some(next)
}

pub fn apply_discovery_to_bar(
    app: &AppHandle,
    conn_state: &Arc<Mutex<DshConnectionState>>,
    outcome: DiscoveryOutcome,
) {
    let Some(bar_win) = app.get_webview_window("bar") else {
        return;
    };

    match outcome {
        DiscoveryOutcome::Connected { port, launch_url } => {
            let mut desired_mode = "summon".to_string();
            if let Ok(mut st) = conn_state.lock() {
                if !st.desired_initial_mode.is_empty() {
                    desired_mode = st.desired_initial_mode.clone();
                }
                st.active_port = Some(port);
                st.active_origin = Some(format!("http://127.0.0.1:{port}"));
                st.status_label = format!("DSH 已连接 (端口 {port})");
            }
            if let Ok(target) = build_bar_entry_url(&launch_url, &desired_mode) {
                log::info!("Navigating bar window to DSH on port {port}");
                let _ = bar_win.navigate(target);
            }
        }

        DiscoveryOutcome::MissingPlugin { port } => {
            if let Ok(mut st) = conn_state.lock() {
                st.active_port = None;
                st.active_origin = None;
                st.status_label = format!("端口 {port} 需重启 DSH 以加载插件");
            }
            let msg = format!(
                "已自动安装 Anything-to-Prompt 插件，请重启一次 DSH (端口 {port}) 后点击重试"
            );
            log::warn!("{msg}");
            if let Ok(mut fallback) = Url::parse("http://tauri.localhost/index.html") {
                fallback
                    .query_pairs_mut()
                    .append_pair("warn", "1")
                    .append_pair("msg", &msg);
                let _ = bar_win.navigate(fallback);
            }
        }

        DiscoveryOutcome::Unavailable { reason } => {
            if let Ok(mut st) = conn_state.lock() {
                st.active_port = None;
                st.active_origin = None;
                st.status_label = reason.clone();
            }
            log::warn!("DSH unavailable: {reason}");
            if let Ok(mut fallback) = Url::parse("http://tauri.localhost/index.html") {
                fallback
                    .query_pairs_mut()
                    .append_pair("warn", "1")
                    .append_pair("msg", &reason);
                let _ = bar_win.navigate(fallback);
            }
        }
    }

    crate::tray::refresh_tray_status(app);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_configured_port_3080_else_lowest_port() {
        let real_json = r#"{
          "port": 3080,
          "url": "http://127.0.0.1:3080/?token=abc",
          "pid": 37672,
          "profile": "web",
          "startedAt": "2026-11-24T12:01:26.108Z",
          "pluginVersion": "2.0.0"
        }"#;
        let parsed_real: InstanceRecord = serde_json::from_str(real_json).unwrap();
        assert_eq!(parsed_real.port, 3080);
        assert_eq!(parsed_real.launch_url, "http://127.0.0.1:3080/?token=abc");

        let list = vec![
            InstanceRecord {
                pid: 10,
                port: 3091,
                launch_url: "http://127.0.0.1:3091/?token=b".into(),
                started_at: serde_json::json!(2),
                plugin_version: "2.0.0".into(),
            },
            parsed_real,
        ];
        let picked = select_best_instance(list, 3080).unwrap();
        assert_eq!(picked.port, 3080);

        let no_default = vec![
            InstanceRecord {
                pid: 10,
                port: 3095,
                launch_url: "http://127.0.0.1:3095/".into(),
                started_at: serde_json::json!("2026-11-24T12:00:00Z"),
                plugin_version: "2.0.0".into(),
            },
            InstanceRecord {
                pid: 12,
                port: 3082,
                launch_url: "http://127.0.0.1:3082/".into(),
                started_at: serde_json::json!("2026-11-24T12:01:00Z"),
                plugin_version: "2.0.0".into(),
            },
        ];
        let picked2 = select_best_instance(no_default, 3080).unwrap();
        assert_eq!(picked2.port, 3082);

        let kv_line = "RESTART_TIME=2026-11-24 17:00:16 NEW_PID=37672 URL=http://127.0.0.1:3080/?token=xyz HTTP=200";
        assert_eq!(
            extract_url_from_last_web_txt(kv_line).as_deref(),
            Some("http://127.0.0.1:3080/?token=xyz")
        );
    }

    #[test]
    fn post_auth_url_adds_bar_clone_only_after_token_redirect() {
        let with_token = Url::parse("http://127.0.0.1:3080/?token=secret123").unwrap();
        assert!(build_post_auth_bar_url(&with_token, "summon").is_none());

        let redirected = Url::parse("http://127.0.0.1:3080/").unwrap();
        let bar_url = build_post_auth_bar_url(&redirected, "lip").unwrap();
        assert_eq!(
            bar_url.as_str(),
            "http://127.0.0.1:3080/?dsh_bar_clone=1&dsb_mode=lip"
        );

        assert!(build_post_auth_bar_url(&bar_url, "lip").is_none());
    }
}
