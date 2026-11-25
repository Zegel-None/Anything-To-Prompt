use crate::broker::{BrokerClient, BrokerRequest};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Manager};
use windows::Win32::System::Threading::CREATE_NO_WINDOW;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HarnessKind {
    #[default]
    Dsh,
    Antigravity,
    Codex,
}

impl HarnessKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            HarnessKind::Dsh => "dsh",
            HarnessKind::Antigravity => "antigravity",
            HarnessKind::Codex => "codex",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "antigravity" | "agy" => HarnessKind::Antigravity,
            "codex" => HarnessKind::Codex,
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
    pub exe_path: Option<String>,
    pub sync_note: String,
    pub model_label: String,
    pub effort_label: String,
    pub access_label: String,
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
pub struct HarnessStreamEvent {
    pub turn_id: String,
    pub harness: String,
    pub kind: String, // "session" | "delta" | "replace" | "tool" | "completed" | "error"
    pub session_id: Option<String>,
    pub text: Option<String>,
    pub tool_name: Option<String>,
    pub error: Option<String>,
}

static ACTIVE_TURN_PIDS: OnceLock<Mutex<HashMap<String, u32>>> = OnceLock::new();

fn active_turn_pids() -> &'static Mutex<HashMap<String, u32>> {
    ACTIVE_TURN_PIDS.get_or_init(|| Mutex::new(HashMap::new()))
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
}

pub fn kill_turn_pid_in_medium(turn_id: &str) -> bool {
    let pid = active_turn_pids()
        .lock()
        .ok()
        .and_then(|mut m| m.remove(turn_id));
    let Some(pid) = pid else {
        return false;
    };
    let _ = Command::new("taskkill.exe")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .creation_flags(CREATE_NO_WINDOW.0)
        .status();
    true
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

fn find_in_path(binary: &str) -> Option<PathBuf> {
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

pub fn list_harness_availability(dsh_connected: bool) -> Vec<HarnessAvailability> {
    let agy = resolve_agy_exe();
    let codex = resolve_codex_exe();
    let (codex_model, codex_effort, codex_access) = read_codex_config_display();
    vec![
        HarnessAvailability {
            id: "dsh".to_string(),
            label: "DeepSeek Harness".to_string(),
            subtitle: if dsh_connected {
                "本地 WebUI 实时双向孪生".to_string()
            } else {
                "未连接本地 DSH 服务".to_string()
            },
            available: dsh_connected,
            exe_path: None,
            sync_note: "毫秒级双向同步".to_string(),
            model_label: String::new(),
            effort_label: String::new(),
            access_label: String::new(),
        },
        HarnessAvailability {
            id: "antigravity".to_string(),
            label: "Antigravity".to_string(),
            subtitle: "CLI Project 原生同步桌面端".to_string(),
            available: agy.is_some(),
            exe_path: agy.map(|p| p.to_string_lossy().to_string()),
            sync_note: "自动同步至桌面端 CLI Project".to_string(),
            model_label: "Gemini 4 Neon".to_string(),
            effort_label: "High".to_string(),
            access_label: "极速".to_string(),
        },
        HarnessAvailability {
            id: "codex".to_string(),
            label: "Codex".to_string(),
            subtitle: "app-server 同库共享历史".to_string(),
            available: codex.is_some(),
            exe_path: codex.map(|p| p.to_string_lossy().to_string()),
            sync_note: "写入 ~/.codex 历史（桌面端刷新可见）".to_string(),
            model_label: codex_model,
            effort_label: codex_effort,
            access_label: codex_access,
        },
    ]
}

fn clean_windows_path(raw: &str) -> String {
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

fn basename_of_path(path: &str) -> String {
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

fn sanitize_title(raw: &str, fallback: &str) -> String {
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
    let dbs = [
        gemini_dir
            .join("antigravity")
            .join("conversation_summaries.db"),
        gemini_dir
            .join("antigravity-cli")
            .join("conversation_summaries.db"),
    ];

    // Map project_id -> (label, path, Vec<ExternalConversation>)
    let mut by_project: HashMap<String, (String, String, Vec<ExternalConversation>)> =
        HashMap::new();
    let mut seen_conv_ids: HashMap<String, usize> = HashMap::new();

    // Always provide default CLI Project at the top so user can start a synced CLI session anytime
    by_project.insert(
        "default-cli-project".to_string(),
        ("CLI Project (桌面端同步)".to_string(), String::new(), Vec::new()),
    );

    for db_path in dbs {
        let Some(conn) = open_sqlite_readonly(&db_path) else {
            continue;
        };
        let Ok(mut stmt) = conn.prepare(
            "SELECT conversation_id, COALESCE(title, ''), COALESCE(preview, ''), \
             COALESCE(CAST(strftime('%s', substr(last_modified_time, 1, 19)) AS INTEGER) * 1000, \
                      CAST(last_modified_time AS INTEGER), 0), \
             COALESCE(workspace_uris, ''), COALESCE(source, ''), \
             COALESCE(project_id, ''), COALESCE(parent_conversation_id, '') \
             FROM conversation_summaries \
             WHERE COALESCE(killed, 0) = 0 \
             ORDER BY last_modified_time DESC LIMIT 120",
        ) else {
            continue;
        };

        let rows = stmt.query_map([], |row| {
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
        });

        let Ok(rows) = rows else {
            continue;
        };

        for row in rows.flatten() {
            let (conv_id, title, preview, updated_at, ws_uris, source, project_id, parent_id) = row;
            if conv_id.trim().is_empty() || !parent_id.trim().is_empty() {
                continue;
            }
            if seen_conv_ids.contains_key(&conv_id) {
                continue;
            }
            seen_conv_ids.insert(conv_id.clone(), 1);

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
                if normalized_pid != "default-cli-project" && normalized_pid != "outside-of-project"
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

    groups.sort_by(|a, b| {
        let a_cli = a.workspace_id == "default-cli-project";
        let b_cli = b.workspace_id == "default-cli-project";
        b_cli.cmp(&a_cli).then_with(|| {
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

    let mut by_cwd: BTreeMap<String, Vec<ExternalConversation>> = BTreeMap::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT id, COALESCE(source, ''), COALESCE(cwd, ''), COALESCE(title, ''), \
         COALESCE(first_user_message, ''), COALESCE(updated_at_ms, updated_at * 1000, 0), \
         COALESCE(rollout_path, '') \
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
                row.get::<_, i64>(5)?,
                row.get::<_, String>(6)?,
            ))
        }) {
            for row in rows.flatten() {
                let (id, source, raw_cwd, title, first_msg, updated_ms, rollout) = row;
                let cwd = clean_windows_path(&raw_cwd);
                let key = if cwd.is_empty() {
                    home.to_string_lossy().to_string()
                } else {
                    cwd.clone()
                };
                let display_title = if !title.trim().is_empty() {
                    sanitize_title(&title, "Codex 会话")
                } else if !first_msg.trim().is_empty() {
                    sanitize_title(&first_msg, "Codex 会话")
                } else {
                    format!("会话 {}", &id[..id.len().min(8)])
                };
                by_cwd.entry(key).or_default().push(ExternalConversation {
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
                });
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
        harness: "codex".to_string(),
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

fn emit_stream_event(writer: &mut impl Write, evt: &HarnessStreamEvent) {
    if let Ok(line) = serde_json::to_string(evt) {
        let _ = writeln!(writer, "{line}");
        let _ = writer.flush();
    }
}

#[derive(Debug, Clone)]
pub struct MediumTurnParams {
    pub turn_id: String,
    pub harness: String,
    pub exe: String,
    pub cwd: Option<String>,
    pub project_id: Option<String>,
    pub session_id: Option<String>,
    pub prompt: String,
    pub out_pipe: String,
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

    match HarnessKind::parse(&params.harness) {
        HarnessKind::Antigravity => {
            run_antigravity_turn_in_medium(
                &params.turn_id,
                &params.exe,
                params.cwd.as_deref(),
                params.project_id.as_deref(),
                params.session_id.as_deref(),
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

fn run_antigravity_turn_in_medium(
    turn_id: &str,
    exe: &str,
    cwd: Option<&str>,
    project_id: Option<&str>,
    session_id: Option<&str>,
    prompt: &str,
    pipe_writer: &mut impl Write,
) {
    let mut cmd = Command::new(exe);
    cmd.args(["--print", "--output-format", "stream-json"]);
    if let Some(sid) = session_id.filter(|s| !s.trim().is_empty()) {
        cmd.args(["--conversation", sid.trim()]);
    } else if let Some(pid) =
        project_id.filter(|s| !s.trim().is_empty() && *s != "outside-of-project")
    {
        if pid != "default-cli-project" {
            cmd.args(["--project", pid.trim()]);
        }
    }
    if let Some(dir) = cwd.filter(|s| !s.trim().is_empty()) {
        let cleaned = clean_windows_path(dir);
        if Path::new(&cleaned).is_dir() {
            cmd.current_dir(&cleaned);
        }
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

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(prompt.as_bytes());
        let _ = stdin.flush();
        drop(stdin);
    }

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
                let status = v
                    .get("status")
                    .and_then(|s| s.as_str())
                    .unwrap_or("SUCCESS");
                if status.eq_ignore_ascii_case("ERROR") || status.eq_ignore_ascii_case("FAILURE") {
                    let err_msg = v
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
                    let final_text = v
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

fn run_codex_app_server_turn_in_medium(
    turn_id: &str,
    exe: &str,
    cwd: Option<&str>,
    session_id: Option<&str>,
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

    let cleaned_cwd = cwd
        .map(clean_windows_path)
        .filter(|p| !p.trim().is_empty() && Path::new(p).is_dir())
        .or_else(|| dirs::home_dir().map(|h| h.to_string_lossy().to_string()));

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
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": 2,
                        "method": "thread/start",
                        "params": {
                            "cwd": cleaned_cwd.clone().unwrap_or_default()
                        }
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
                let turn_req = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 3,
                    "method": "turn/start",
                    "params": {
                        "threadId": tid,
                        "input": [{ "type": "text", "text": prompt }]
                    }
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

#[derive(Debug, Clone)]
pub struct ExternalTurnRequest {
    pub harness: HarnessKind,
    pub turn_id: String,
    pub cwd: Option<String>,
    pub project_id: Option<String>,
    pub session_id: Option<String>,
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
        HarnessKind::Dsh => return Err("DSH 请使用原生 WebView 发送".to_string()),
    };

    let (pipe_short, server_pipe) = broker
        .create_oneshot_read_pipe(&turn_id)
        .map_err(|e| format!("创建流式管道失败: {e}"))?;

    let req = BrokerRequest::StartHarnessTurn {
        turn_id: turn_id.clone(),
        harness: harness.as_str().to_string(),
        exe: exe.to_string_lossy().to_string(),
        cwd: req_in.cwd,
        project_id: req_in.project_id,
        session_id: req_in.session_id,
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
    fn readonly_catalog_loaders_succeed_without_mutating_local_dbs() {
        let avail = list_harness_availability(true);
        assert_eq!(avail.len(), 3);
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
    }
}
