use crate::config::{AppConfig, CustomAcpHarness};
use crate::harness::{
    basename_of_path, build_prompt_with_file_attachments, clean_windows_path,
    ensure_default_workspace_dir, find_in_path, register_turn_pid, resolve_effective_workspace_dir,
    sanitize_title, ExternalChatMessage, ExternalConversation, ExternalHarnessCatalog,
    ExternalWorkspaceGroup, HarnessAvailability, HarnessComposerOptions, HarnessOptionItem,
    HarnessSessionState, HarnessStreamEvent, TurnAttachment,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use windows::Win32::System::Threading::CREATE_NO_WINDOW;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpAgentSpec {
    pub id: String,       // always prefixed with "acp:", e.g. "acp:opencode"
    pub slug: String,     // e.g. "opencode"
    pub label: String,    // e.g. "OpenCode"
    pub subtitle: String, // e.g. "ACP 标准协议 · opencode acp"
    pub command: String,  // executable path or command name
    pub args: Vec<String>,
    pub accent: String,
    pub badge: String,
    pub custom: bool,
    pub available: bool,
    pub default_cwd: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct AcpProbedCapabilities {
    pub agent_name: String,
    pub agent_version: String,
    pub supports_load_session: bool,
    pub supports_list_sessions: bool,
    pub supports_image: bool,
    pub models: Vec<HarnessOptionItem>,
    pub efforts: Vec<HarnessOptionItem>,
    pub modes: Vec<HarnessOptionItem>,
    pub model_config_id: Option<String>,
    pub effort_config_id: Option<String>,
    pub mode_config_id: Option<String>,
    pub default_model: String,
    pub default_effort: String,
    pub default_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct AcpStoredSession {
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub updated_at: i64,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub messages: Vec<ExternalChatMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct AcpSessionStore {
    pub sessions: Vec<AcpStoredSession>,
}

struct BuiltinAcpCandidate {
    slug: &'static str,
    label: &'static str,
    badge: &'static str,
    accent: &'static str,
    commands: &'static [&'static str],
    args: &'static [&'static str],
    description: &'static str,
}

/// Well-known native ACP agents from the official Agent Client Protocol Registry
/// (https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json).
/// Auto-discovered purely from local PATH/npm without requiring any network call.
const BUILTIN_ACP_CANDIDATES: &[BuiltinAcpCandidate] = &[
    BuiltinAcpCandidate {
        slug: "opencode",
        label: "OpenCode",
        badge: "OC",
        accent: "#10b981",
        commands: &["opencode.cmd", "opencode.exe", "opencode"],
        args: &["acp"],
        description: "ACP 原生协议 · 开源多模型编码助手",
    },
    BuiltinAcpCandidate {
        slug: "gemini",
        label: "Gemini CLI",
        badge: "GM",
        accent: "#4285f4",
        commands: &["gemini.cmd", "gemini.exe", "gemini"],
        args: &["--acp"],
        description: "ACP 原生协议 · Google Gemini CLI",
    },
    BuiltinAcpCandidate {
        slug: "cursor",
        label: "Cursor Agent",
        badge: "CR",
        accent: "#8b5cf6",
        commands: &["cursor-agent.cmd", "cursor-agent.exe", "cursor-agent"],
        args: &["acp"],
        description: "ACP 原生协议 · Cursor CLI Agent",
    },
    BuiltinAcpCandidate {
        slug: "goose",
        label: "Goose",
        badge: "GS",
        accent: "#f59e0b",
        commands: &["goose.exe", "goose.cmd", "goose"],
        args: &["acp"],
        description: "ACP 原生协议 · Block Goose Agent",
    },
    BuiltinAcpCandidate {
        slug: "copilot",
        label: "Copilot CLI",
        badge: "CP",
        accent: "#0ea5e9",
        commands: &["copilot.cmd", "copilot.exe", "copilot"],
        args: &["--acp", "--stdio"],
        description: "ACP 原生协议 · GitHub Copilot CLI",
    },
    BuiltinAcpCandidate {
        slug: "cline",
        label: "Cline",
        badge: "CL",
        accent: "#ec4899",
        commands: &["cline.cmd", "cline.exe", "cline"],
        args: &["--acp"],
        description: "ACP 原生协议 · Cline Autonomous Agent",
    },
    BuiltinAcpCandidate {
        slug: "amp",
        label: "Amp",
        badge: "AM",
        accent: "#f97316",
        commands: &["amp.cmd", "amp.exe", "amp"],
        args: &["--acp"],
        description: "ACP 原生协议 · Sourcegraph Amp",
    },
];

static ACP_CAPS_CACHE: OnceLock<Mutex<HashMap<String, (Instant, AcpProbedCapabilities)>>> =
    OnceLock::new();

fn caps_cache() -> &'static Mutex<HashMap<String, (Instant, AcpProbedCapabilities)>> {
    ACP_CAPS_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn is_acp_harness_id(raw: &str) -> bool {
    raw.trim().to_ascii_lowercase().starts_with("acp:")
}

pub fn normalize_acp_slug(raw: &str) -> String {
    let trimmed = raw
        .trim()
        .strip_prefix("acp:")
        .unwrap_or(raw.trim())
        .to_ascii_lowercase();
    let mut out = String::new();
    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else if ch.is_whitespace() {
            out.push('-');
        }
    }
    if out.is_empty() {
        "custom".to_string()
    } else {
        out
    }
}

fn resolve_command_path(cmd: &str) -> Option<PathBuf> {
    let trimmed = cmd.trim().trim_matches('"');
    if trimmed.is_empty() {
        return None;
    }
    let direct = PathBuf::from(trimmed);
    if direct.is_file() {
        return Some(direct);
    }
    if let Some(found) = find_in_path(trimmed) {
        return Some(found);
    }
    if !trimmed.contains('.') {
        for ext in [".exe", ".cmd", ".bat"] {
            let with_ext = format!("{trimmed}{ext}");
            if let Some(found) = find_in_path(&with_ext) {
                return Some(found);
            }
        }
    }
    if let Some(appdata) = dirs::data_dir() {
        let npm_dir = appdata.join("npm");
        for cand in [
            npm_dir.join(trimmed),
            npm_dir.join(format!("{trimmed}.cmd")),
            npm_dir.join(format!("{trimmed}.exe")),
        ] {
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    if let Some(home) = dirs::home_dir() {
        let local_bin = home.join(".local").join("bin");
        for cand in [
            local_bin.join(trimmed),
            local_bin.join(format!("{trimmed}.exe")),
            local_bin.join(format!("{trimmed}.cmd")),
        ] {
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

/// Discovers locally installed native ACP agents + all user-configured custom ACP harnesses.
pub fn discover_acp_agents(custom_list: &[CustomAcpHarness]) -> Vec<AcpAgentSpec> {
    let mut out: Vec<AcpAgentSpec> = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();

    // 1. User-defined custom ACP harnesses (always visible, even if path needs fixing)
    for item in custom_list {
        let slug = normalize_acp_slug(&item.id);
        let full_id = format!("acp:{slug}");
        if !seen_ids.insert(full_id.clone()) {
            continue;
        }
        let resolved = resolve_command_path(&item.command);
        let available = resolved.is_some();
        let command = resolved
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| item.command.trim().to_string());
        let label = if item.label.trim().is_empty() {
            slug.clone()
        } else {
            item.label.trim().to_string()
        };
        let badge: String = label
            .chars()
            .filter(|c| c.is_alphanumeric())
            .take(2)
            .collect::<String>()
            .to_ascii_uppercase();
        let accent = if item.accent.trim().starts_with('#') {
            item.accent.trim().to_string()
        } else {
            "#38bdf8".to_string()
        };
        let args_disp = if item.args.is_empty() {
            String::new()
        } else {
            format!(" {}", item.args.join(" "))
        };
        out.push(AcpAgentSpec {
            id: full_id,
            slug,
            label,
            subtitle: format!("自定义 ACP · {}{}", item.command.trim(), args_disp),
            command,
            args: item.args.clone(),
            accent,
            badge: if badge.is_empty() { "AC".to_string() } else { badge },
            custom: true,
            available,
            default_cwd: item.default_cwd.clone(),
        });
    }

    // 2. Auto-discovered locally installed native ACP CLIs (only include if actually installed!)
    for cand in BUILTIN_ACP_CANDIDATES {
        let full_id = format!("acp:{}", cand.slug);
        if seen_ids.contains(&full_id) {
            continue;
        }
        let mut resolved = None;
        for bin in cand.commands {
            if let Some(p) = resolve_command_path(bin) {
                resolved = Some(p);
                break;
            }
        }
        if let Some(exe_path) = resolved {
            seen_ids.insert(full_id.clone());
            out.push(AcpAgentSpec {
                id: full_id,
                slug: cand.slug.to_string(),
                label: cand.label.to_string(),
                subtitle: cand.description.to_string(),
                command: exe_path.to_string_lossy().to_string(),
                args: cand.args.iter().map(|s| s.to_string()).collect(),
                accent: cand.accent.to_string(),
                badge: cand.badge.to_string(),
                custom: false,
                available: true,
                default_cwd: None,
            });
        }
    }

    out
}

pub fn resolve_acp_agent_spec(harness_id: &str, custom_list: &[CustomAcpHarness]) -> Option<AcpAgentSpec> {
    let target = harness_id.trim().to_ascii_lowercase();
    discover_acp_agents(custom_list)
        .into_iter()
        .find(|s| s.id == target)
}

pub fn list_acp_availability(custom_list: &[CustomAcpHarness]) -> Vec<HarnessAvailability> {
    discover_acp_agents(custom_list)
        .into_iter()
        .map(|spec| HarnessAvailability {
            id: spec.id,
            label: spec.label,
            subtitle: spec.subtitle,
            available: spec.available,
            running: false,
            exe_path: if spec.available {
                Some(spec.command.clone())
            } else {
                None
            },
            sync_note: "ACP 标准协议实时握手 · 动态模型与权限".to_string(),
            model_label: "动态模型".to_string(),
            effort_label: String::new(),
            access_label: "ACP".to_string(),
            provider_kind: "acp".to_string(),
            accent: spec.accent,
            badge: spec.badge,
            custom: spec.custom,
            command: spec.command,
            args: spec.args,
        })
        .collect()
}

fn acp_store_path(harness_id: &str) -> PathBuf {
    let slug = normalize_acp_slug(harness_id);
    AppConfig::local_data_dir()
        .join("acp_sessions")
        .join(format!("{slug}.json"))
}

fn load_acp_store(harness_id: &str) -> AcpSessionStore {
    let path = acp_store_path(harness_id);
    if let Ok(raw) = fs::read_to_string(&path) {
        if let Ok(store) = serde_json::from_str::<AcpSessionStore>(&raw) {
            return store;
        }
    }
    AcpSessionStore::default()
}

fn save_acp_store(harness_id: &str, store: &AcpSessionStore) {
    let path = acp_store_path(harness_id);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(pretty) = serde_json::to_string_pretty(store) {
        let _ = fs::write(path, pretty);
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn record_acp_turn_history(
    harness_id: &str,
    session_id: &str,
    cwd: &str,
    model: Option<&str>,
    effort: Option<&str>,
    mode: Option<&str>,
    user_text: &str,
    assistant_text: &str,
) {
    let sid = session_id.trim();
    if sid.is_empty() {
        return;
    }
    let mut store = load_acp_store(harness_id);
    let ts = now_ms();
    let cleaned_cwd = clean_windows_path(cwd);
    if let Some(existing) = store.sessions.iter_mut().find(|s| s.id == sid) {
        existing.updated_at = ts;
        if !cleaned_cwd.is_empty() {
            existing.cwd = cleaned_cwd;
        }
        if let Some(m) = model.filter(|s| !s.trim().is_empty()) {
            existing.model = Some(m.trim().to_string());
        }
        if let Some(e) = effort.filter(|s| !s.trim().is_empty()) {
            existing.effort = Some(e.trim().to_string());
        }
        if let Some(md) = mode.filter(|s| !s.trim().is_empty()) {
            existing.mode = Some(md.trim().to_string());
        }
        if !user_text.trim().is_empty() {
            existing.messages.push(ExternalChatMessage {
                id: format!("acp-u-{ts}"),
                role: "user".to_string(),
                text: user_text.trim().to_string(),
                streaming: false,
            });
        }
        if !assistant_text.trim().is_empty() {
            existing.messages.push(ExternalChatMessage {
                id: format!("acp-a-{ts}"),
                role: "assistant".to_string(),
                text: assistant_text.trim().to_string(),
                streaming: false,
            });
        }
        if existing.messages.len() > 60 {
            let drain = existing.messages.len() - 60;
            existing.messages.drain(0..drain);
        }
    } else {
        let mut msgs = Vec::new();
        if !user_text.trim().is_empty() {
            msgs.push(ExternalChatMessage {
                id: format!("acp-u-{ts}"),
                role: "user".to_string(),
                text: user_text.trim().to_string(),
                streaming: false,
            });
        }
        if !assistant_text.trim().is_empty() {
            msgs.push(ExternalChatMessage {
                id: format!("acp-a-{ts}"),
                role: "assistant".to_string(),
                text: assistant_text.trim().to_string(),
                streaming: false,
            });
        }
        store.sessions.push(AcpStoredSession {
            id: sid.to_string(),
            title: sanitize_title(user_text, "ACP 对话"),
            cwd: cleaned_cwd,
            updated_at: ts,
            model: model.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
            effort: effort.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
            mode: mode.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
            messages: msgs,
        });
    }
    store.sessions.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
    store.sessions.truncate(80);
    save_acp_store(harness_id, &store);
}

pub fn load_acp_catalog(harness_id: &str, custom_list: &[CustomAcpHarness]) -> ExternalHarnessCatalog {
    let spec = resolve_acp_agent_spec(harness_id, custom_list);
    let available = spec.as_ref().map(|s| s.available).unwrap_or(false);
    let exe_path = spec.as_ref().map(|s| s.command.clone());
    let default_dir = spec
        .as_ref()
        .and_then(|s| s.default_cwd.as_deref())
        .map(clean_windows_path)
        .filter(|p| !p.is_empty() && Path::new(p).is_dir())
        .unwrap_or_else(|| ensure_default_workspace_dir().to_string_lossy().to_string());

    let store = load_acp_store(harness_id);
    let mut by_cwd: std::collections::BTreeMap<String, Vec<ExternalConversation>> =
        std::collections::BTreeMap::new();
    for s in store.sessions {
        let cwd = if s.cwd.trim().is_empty() {
            default_dir.clone()
        } else {
            s.cwd.clone()
        };
        by_cwd.entry(cwd.clone()).or_default().push(ExternalConversation {
            id: s.id,
            title: s.title,
            updated_at: s.updated_at,
            source: "acp".to_string(),
            cwd,
            rollout_path: None,
        });
    }
    if by_cwd.is_empty() {
        by_cwd.insert(default_dir, Vec::new());
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
        let ats = a.convs.first().map(|c| c.updated_at).unwrap_or(0);
        let bts = b.convs.first().map(|c| c.updated_at).unwrap_or(0);
        bts.cmp(&ats)
    });

    ExternalHarnessCatalog {
        harness: harness_id.to_string(),
        available,
        exe_path,
        groups,
        error: if available {
            None
        } else {
            Some("未找到该 ACP 命令，请在设置中检查命令路径".to_string())
        },
    }
}

pub fn load_acp_messages(harness_id: &str, session_id: &str) -> Vec<ExternalChatMessage> {
    let sid = session_id.trim();
    if sid.is_empty() {
        return Vec::new();
    }
    load_acp_store(harness_id)
        .sessions
        .into_iter()
        .find(|s| s.id == sid)
        .map(|s| s.messages)
        .unwrap_or_default()
}

pub fn load_acp_session_state(harness_id: &str, session_id: &str) -> HarnessSessionState {
    let sid = session_id.trim();
    let mut out = HarnessSessionState {
        harness: harness_id.to_string(),
        session_id: sid.to_string(),
        read_only_in_desktop: false,
        ..Default::default()
    };
    if let Some(found) = load_acp_store(harness_id)
        .sessions
        .into_iter()
        .find(|s| s.id == sid)
    {
        out.model = found.model;
        out.effort = found.effort;
        out.mode = found.mode;
        out.source_note = "ACP 标准协议会话（修改模型/模式将实时通过 ACP 协议生效）".to_string();
    }
    out
}

fn spawn_acp_command(exe: &str, args: &[String], cwd: Option<&str>) -> std::io::Result<std::process::Child> {
    let lower = exe.to_ascii_lowercase();
    let mut cmd = if lower.ends_with(".cmd") || lower.ends_with(".bat") {
        let mut c = Command::new("cmd.exe");
        c.arg("/C").arg(exe);
        for a in args {
            c.arg(a);
        }
        c
    } else if lower.ends_with(".ps1") {
        let mut c = Command::new("powershell.exe");
        c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", exe]);
        for a in args {
            c.arg(a);
        }
        c
    } else {
        let mut c = Command::new(exe);
        for a in args {
            c.arg(a);
        }
        c
    };
    if let Some(dir) = resolve_effective_workspace_dir(cwd) {
        cmd.current_dir(dir);
    }
    cmd.creation_flags(CREATE_NO_WINDOW.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}

fn send_jsonrpc_line(stdin: &mut impl Write, val: &serde_json::Value) -> std::io::Result<()> {
    let line = serde_json::to_string(val)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    writeln!(stdin, "{line}")?;
    stdin.flush()
}

/// Parses standard ACP v1 `session/new` / `session/load` result `configOptions`, `modes`, and `models`.
pub fn parse_acp_session_options_from_result(result: &serde_json::Value) -> AcpProbedCapabilities {
    let mut caps = AcpProbedCapabilities::default();

    // 1. Standard ACP v1 `configOptions` array (used by OpenCode, Gemini CLI, Zed adapters, etc.)
    if let Some(arr) = result.get("configOptions").and_then(|v| v.as_array()) {
        for opt in arr {
            let id = opt.get("id").and_then(|s| s.as_str()).unwrap_or("").trim();
            let category = opt
                .get("category")
                .and_then(|s| s.as_str())
                .unwrap_or(id)
                .trim()
                .to_ascii_lowercase();
            let cur_val = opt
                .get("currentValue")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .trim()
                .to_string();

            let mut parsed_items: Vec<HarnessOptionItem> = Vec::new();
            if let Some(opts) = opt.get("options").and_then(|v| v.as_array()) {
                for entry in opts {
                    if let Some(group_opts) = entry.get("options").and_then(|v| v.as_array()) {
                        for sub in group_opts {
                            if let Some(val) = sub.get("value").and_then(|s| s.as_str()) {
                                let name = sub
                                    .get("name")
                                    .and_then(|s| s.as_str())
                                    .unwrap_or(val)
                                    .to_string();
                                let desc = sub
                                    .get("description")
                                    .and_then(|s| s.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                parsed_items.push(HarnessOptionItem {
                                    id: val.to_string(),
                                    label: name,
                                    desc,
                                });
                            }
                        }
                    } else if let Some(val) = entry.get("value").and_then(|s| s.as_str()) {
                        let name = entry
                            .get("name")
                            .and_then(|s| s.as_str())
                            .unwrap_or(val)
                            .to_string();
                        let desc = entry
                            .get("description")
                            .and_then(|s| s.as_str())
                            .unwrap_or("")
                            .to_string();
                        parsed_items.push(HarnessOptionItem {
                            id: val.to_string(),
                            label: name,
                            desc,
                        });
                    }
                }
            }

            if parsed_items.is_empty() {
                continue;
            }
            if category == "model" || id.eq_ignore_ascii_case("model") {
                caps.models = parsed_items;
                caps.model_config_id = Some(id.to_string());
                caps.default_model = cur_val;
            } else if category == "mode" || id.eq_ignore_ascii_case("mode") {
                caps.modes = parsed_items;
                caps.mode_config_id = Some(id.to_string());
                caps.default_mode = cur_val;
            } else if category.contains("thought")
                || category.contains("effort")
                || category.contains("reasoning")
                || id.to_ascii_lowercase().contains("effort")
            {
                caps.efforts = parsed_items;
                caps.effort_config_id = Some(id.to_string());
                caps.default_effort = cur_val;
            }
        }
    }

    // 2. Fallback: ACP legacy/draft `modes` object (`{ currentModeId, availableModes: [{ id, name, description }] }`)
    if caps.modes.is_empty() {
        if let Some(modes_obj) = result.get("modes") {
            if let Some(cur) = modes_obj.get("currentModeId").and_then(|s| s.as_str()) {
                caps.default_mode = cur.trim().to_string();
            }
            if let Some(avail) = modes_obj.get("availableModes").and_then(|v| v.as_array()) {
                for m in avail {
                    if let Some(mid) = m.get("id").and_then(|s| s.as_str()) {
                        let name = m.get("name").and_then(|s| s.as_str()).unwrap_or(mid);
                        let desc = m.get("description").and_then(|s| s.as_str()).unwrap_or("");
                        caps.modes.push(HarnessOptionItem {
                            id: mid.to_string(),
                            label: name.to_string(),
                            desc: desc.to_string(),
                        });
                    }
                }
            }
        }
    }

    // 3. Fallback: ACP legacy/draft `models` object (`{ currentModelId, availableModels: [{ modelId, name, description }] }`)
    if caps.models.is_empty() {
        if let Some(models_obj) = result.get("models") {
            if let Some(cur) = models_obj.get("currentModelId").and_then(|s| s.as_str()) {
                caps.default_model = cur.trim().to_string();
            }
            if let Some(avail) = models_obj.get("availableModels").and_then(|v| v.as_array()) {
                for m in avail {
                    if let Some(mid) = m
                        .get("modelId")
                        .or_else(|| m.get("id"))
                        .and_then(|s| s.as_str())
                    {
                        let name = m.get("name").and_then(|s| s.as_str()).unwrap_or(mid);
                        let desc = m.get("description").and_then(|s| s.as_str()).unwrap_or("");
                        caps.models.push(HarnessOptionItem {
                            id: mid.to_string(),
                            label: name.to_string(),
                            desc: desc.to_string(),
                        });
                    }
                }
            }
        }
    }

    if caps.default_model.is_empty() {
        if let Some(first) = caps.models.first() {
            caps.default_model = first.id.clone();
        }
    }
    if caps.default_mode.is_empty() {
        if let Some(first) = caps.modes.first() {
            caps.default_mode = first.id.clone();
        }
    }
    if caps.default_effort.is_empty() {
        if let Some(first) = caps.efforts.first() {
            caps.default_effort = first.id.clone();
        }
    }
    caps
}

/// Probes an ACP agent via `initialize` + `session/new` (with 5-min in-memory cache and 4.5s timeout)
/// to obtain its live, authoritative models, modes, and reasoning effort options.
pub fn probe_acp_options(
    harness_id: &str,
    custom_list: &[CustomAcpHarness],
) -> HarnessComposerOptions {
    let default_modes = vec![
        HarnessOptionItem {
            id: "auto".into(),
            label: "Auto (ACP)".into(),
            desc: "自动批准常规权限请求".into(),
        },
        HarnessOptionItem {
            id: "read-only".into(),
            label: "只读保护".into(),
            desc: "自动拒绝写入/命令权限请求".into(),
        },
    ];
    let fallback = HarnessComposerOptions {
        harness: harness_id.to_string(),
        models: vec![HarnessOptionItem {
            id: "default".into(),
            label: "默认模型 (ACP)".into(),
            desc: "由 Agent 自行选用默认模型".into(),
        }],
        efforts: Vec::new(),
        modes: default_modes.clone(),
        plugins: Vec::new(),
        actions: vec![
            HarnessOptionItem {
                id: "/plan".into(),
                label: "/plan".into(),
                desc: "以规划模式分析任务".into(),
            },
            HarnessOptionItem {
                id: "/compact".into(),
                label: "/compact".into(),
                desc: "压缩上下文".into(),
            },
        ],
        default_model: "default".to_string(),
        default_effort: String::new(),
        default_mode: "auto".to_string(),
    };

    if let Ok(cache) = caps_cache().lock() {
        if let Some((ts, caps)) = cache.get(harness_id) {
            if ts.elapsed() < Duration::from_secs(300) {
                return HarnessComposerOptions {
                    harness: harness_id.to_string(),
                    models: if caps.models.is_empty() {
                        fallback.models.clone()
                    } else {
                        caps.models.clone()
                    },
                    efforts: caps.efforts.clone(),
                    modes: if caps.modes.is_empty() {
                        default_modes
                    } else {
                        caps.modes.clone()
                    },
                    plugins: Vec::new(),
                    actions: fallback.actions,
                    default_model: if caps.default_model.is_empty() {
                        fallback.default_model
                    } else {
                        caps.default_model.clone()
                    },
                    default_effort: caps.default_effort.clone(),
                    default_mode: if caps.default_mode.is_empty() {
                        fallback.default_mode
                    } else {
                        caps.default_mode.clone()
                    },
                };
            }
        }
    }

    let Some(spec) = resolve_acp_agent_spec(harness_id, custom_list) else {
        return fallback;
    };
    if !spec.available {
        return fallback;
    }

    let cwd = spec
        .default_cwd
        .clone()
        .or_else(|| resolve_effective_workspace_dir(None))
        .unwrap_or_else(|| ensure_default_workspace_dir().to_string_lossy().to_string());

    let Ok(mut child) = spawn_acp_command(&spec.command, &spec.args, Some(&cwd)) else {
        return fallback;
    };
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = child.kill();
        return fallback;
    };

    let init_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": 1,
            "clientCapabilities": {
                "fs": { "readTextFile": false, "writeTextFile": false },
                "terminal": false
            },
            "clientInfo": {
                "name": "anything-to-prompt",
                "title": "Anything-to-Prompt",
                "version": env!("CARGO_PKG_VERSION")
            }
        }
    });
    if send_jsonrpc_line(&mut stdin, &init_req).is_err() {
        let _ = child.kill();
        return fallback;
    }

    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    let mut probed = AcpProbedCapabilities::default();
    let deadline = Instant::now() + Duration::from_millis(4500);
    let mut stage = 1u8;

    while Instant::now() < deadline {
        let remain = deadline.saturating_duration_since(Instant::now());
        let Ok(line) = rx.recv_timeout(remain.min(Duration::from_millis(500))) else {
            break;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        let id_num = v.get("id").and_then(|i| i.as_i64());
        if id_num == Some(1) && stage == 1 {
            if let Some(res) = v.get("result") {
                probed.agent_name = res
                    .pointer("/agentInfo/name")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string();
                probed.agent_version = res
                    .pointer("/agentInfo/version")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string();
                probed.supports_load_session = res
                    .pointer("/agentCapabilities/loadSession")
                    .and_then(|b| b.as_bool())
                    .unwrap_or(false);
                probed.supports_image = res
                    .pointer("/agentCapabilities/promptCapabilities/image")
                    .and_then(|b| b.as_bool())
                    .unwrap_or(false);
                probed.supports_list_sessions = res
                    .pointer("/agentCapabilities/sessionCapabilities/list")
                    .is_some();
            }
            let sess_req = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "session/new",
                "params": {
                    "cwd": cwd,
                    "mcpServers": []
                }
            });
            let _ = send_jsonrpc_line(&mut stdin, &sess_req);
            stage = 2;
        } else if id_num == Some(2) && stage == 2 {
            if let Some(res) = v.get("result") {
                let parsed = parse_acp_session_options_from_result(res);
                probed.models = parsed.models;
                probed.efforts = parsed.efforts;
                probed.modes = parsed.modes;
                probed.model_config_id = parsed.model_config_id;
                probed.effort_config_id = parsed.effort_config_id;
                probed.mode_config_id = parsed.mode_config_id;
                probed.default_model = parsed.default_model;
                probed.default_effort = parsed.default_effort;
                probed.default_mode = parsed.default_mode;
            }
            break;
        }
    }

    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();

    if let Ok(mut cache) = caps_cache().lock() {
        cache.insert(harness_id.to_string(), (Instant::now(), probed.clone()));
    }

    HarnessComposerOptions {
        harness: harness_id.to_string(),
        models: if probed.models.is_empty() {
            fallback.models
        } else {
            probed.models
        },
        efforts: probed.efforts,
        modes: if probed.modes.is_empty() {
            default_modes
        } else {
            probed.modes
        },
        plugins: Vec::new(),
        actions: fallback.actions,
        default_model: if probed.default_model.is_empty() {
            fallback.default_model
        } else {
            probed.default_model
        },
        default_effort: probed.default_effort,
        default_mode: if probed.default_mode.is_empty() {
            fallback.default_mode
        } else {
            probed.default_mode
        },
    }
}

fn emit_stream_event(writer: &mut impl Write, evt: &HarnessStreamEvent) {
    if let Ok(line) = serde_json::to_string(evt) {
        let _ = writeln!(writer, "{line}");
        let _ = writer.flush();
    }
}

/// Runs a complete real ACP v1 turn (`initialize` -> `session/resume` / `session/load` / `session/new` ->
/// `session/set_config_option` / `session/set_model` / `session/set_mode` -> `session/prompt`),
/// streaming deltas & tools over the named pipe.
/// Returns `true` if the session started/streamed cleanly, or `false` if pre-prompt handshake failed.
pub fn run_acp_turn_in_medium(
    turn_id: &str,
    harness_id: &str,
    exe: &str,
    acp_args: &[String],
    cwd: Option<&str>,
    session_id: Option<&str>,
    model: Option<&str>,
    effort: Option<&str>,
    mode: Option<&str>,
    attachments: &[TurnAttachment],
    prompt: &str,
    pipe_writer: &mut impl Write,
    emit_pre_prompt_errors: bool,
) -> bool {
    let effective_cwd = resolve_effective_workspace_dir(cwd)
        .unwrap_or_else(|| ensure_default_workspace_dir().to_string_lossy().to_string());

    let mut child = match spawn_acp_command(exe, acp_args, Some(&effective_cwd)) {
        Ok(c) => c,
        Err(e) => {
            if emit_pre_prompt_errors {
                emit_stream_event(
                    pipe_writer,
                    &HarnessStreamEvent {
                        turn_id: turn_id.to_string(),
                        harness: harness_id.to_string(),
                        kind: "error".to_string(),
                        session_id: session_id.map(|s| s.to_string()),
                        text: None,
                        tool_name: None,
                        error: Some(format!("启动 ACP Agent 失败 ({exe}): {e}")),
                    },
                );
            }
            return false;
        }
    };
    register_turn_pid(turn_id, child.id());

    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = child.kill();
        return false;
    };

    let init_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": 1,
            "clientCapabilities": {
                "fs": { "readTextFile": false, "writeTextFile": false },
                "terminal": false
            },
            "clientInfo": {
                "name": "anything-to-prompt",
                "title": "Anything-to-Prompt",
                "version": env!("CARGO_PKG_VERSION")
            }
        }
    });
    if let Err(e) = send_jsonrpc_line(&mut stdin, &init_req) {
        if emit_pre_prompt_errors {
            emit_stream_event(
                pipe_writer,
                &HarnessStreamEvent {
                    turn_id: turn_id.to_string(),
                    harness: harness_id.to_string(),
                    kind: "error".to_string(),
                    session_id: None,
                    text: None,
                    tool_name: None,
                    error: Some(format!("ACP initialize 失败: {e}")),
                },
            );
        }
        let _ = child.kill();
        return false;
    }

    let reader = BufReader::new(stdout);
    let mut stage = 1u8; // 1=init, 2=session/resume|load|new, 3=prompting
    let mut supports_resume_session = false;
    let mut supports_load_session = false;
    let mut supports_image = false;
    let mut resolved_sid = session_id
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let mut tried_resume_or_load = false;
    let mut accumulated_text = String::new();
    let mut finished = false;
    let read_only_mode = mode
        .map(|m| {
            let l = m.to_ascii_lowercase();
            l.contains("read") || l == "plan" || l == "request-review"
        })
        .unwrap_or(false);
    let mut next_req_id = 10i64;
    let mut prompt_req_id = 0i64;

    let full_prompt_text = build_prompt_with_file_attachments(prompt, attachments, false);

    for line in reader.lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };

        // Handle server-initiated requests (e.g. session/request_permission)
        if v.get("method").is_some() && v.get("id").is_some() {
            let method = v.get("method").and_then(|m| m.as_str()).unwrap_or("");
            let req_id = v.get("id").cloned().unwrap_or(serde_json::json!(0));
            if method == "session/request_permission" {
                let options = v
                    .pointer("/params/options")
                    .and_then(|o| o.as_array())
                    .cloned()
                    .unwrap_or_default();
                let chosen = if read_only_mode {
                    options
                        .iter()
                        .find(|o| {
                            o.get("kind")
                                .and_then(|k| k.as_str())
                                .map(|k| k.contains("reject"))
                                .unwrap_or(false)
                        })
                        .or_else(|| options.last())
                } else {
                    options
                        .iter()
                        .find(|o| {
                            o.get("kind")
                                .and_then(|k| k.as_str())
                                .map(|k| k.contains("allow"))
                                .unwrap_or(false)
                        })
                        .or_else(|| options.first())
                };
                let outcome = if let Some(opt_id) = chosen.and_then(|o| o.get("optionId")).and_then(|s| s.as_str()) {
                    serde_json::json!({ "outcome": "selected", "optionId": opt_id })
                } else {
                    serde_json::json!({ "outcome": "cancelled" })
                };
                let reply = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": { "outcome": outcome }
                });
                let _ = send_jsonrpc_line(&mut stdin, &reply);
            } else {
                let reply = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "error": { "code": -32601, "message": format!("Method not supported: {method}") }
                });
                let _ = send_jsonrpc_line(&mut stdin, &reply);
            }
            continue;
        }

        // Handle JSON-RPC responses
        if let Some(id_num) = v.get("id").and_then(|i| i.as_i64()) {
            if let Some(err) = v.get("error") {
                // If session/resume or session/load failed (e.g. expired session), gracefully fall back to session/new
                if id_num == 2 && stage == 2 && tried_resume_or_load {
                    tried_resume_or_load = false;
                    resolved_sid = None;
                    let new_req = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": 3,
                        "method": "session/new",
                        "params": { "cwd": effective_cwd, "mcpServers": [] }
                    });
                    let _ = send_jsonrpc_line(&mut stdin, &new_req);
                    continue;
                }
                // Non-fatal config option set error: ignore and proceed with prompt
                if id_num >= 10 && id_num != prompt_req_id {
                    continue;
                }
                let msg = err
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("ACP Agent 返回错误")
                    .to_string();
                if stage >= 3 || emit_pre_prompt_errors {
                    emit_stream_event(
                        pipe_writer,
                        &HarnessStreamEvent {
                            turn_id: turn_id.to_string(),
                            harness: harness_id.to_string(),
                            kind: "error".to_string(),
                            session_id: resolved_sid.clone(),
                            text: None,
                            tool_name: None,
                            error: Some(msg),
                        },
                    );
                }
                finished = stage >= 3;
                break;
            }

            if id_num == 1 && stage == 1 {
                if let Some(res) = v.get("result") {
                    supports_resume_session = res
                        .pointer("/agentCapabilities/sessionCapabilities/resume")
                        .is_some()
                        || res
                            .pointer("/agentCapabilities/session_capabilities/resume")
                            .is_some();
                    supports_load_session = res
                        .pointer("/agentCapabilities/loadSession")
                        .and_then(|b| b.as_bool())
                        .unwrap_or(false);
                    supports_image = res
                        .pointer("/agentCapabilities/promptCapabilities/image")
                        .and_then(|b| b.as_bool())
                        .unwrap_or(false);
                }
                let sess_req = if supports_resume_session && resolved_sid.is_some() {
                    tried_resume_or_load = true;
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": 2,
                        "method": "session/resume",
                        "params": {
                            "sessionId": resolved_sid.clone().unwrap_or_default(),
                            "cwd": effective_cwd,
                            "mcpServers": []
                        }
                    })
                } else if supports_load_session && resolved_sid.is_some() {
                    tried_resume_or_load = true;
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": 2,
                        "method": "session/load",
                        "params": {
                            "sessionId": resolved_sid.clone().unwrap_or_default(),
                            "cwd": effective_cwd,
                            "mcpServers": []
                        }
                    })
                } else {
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": 2,
                        "method": "session/new",
                        "params": {
                            "cwd": effective_cwd,
                            "mcpServers": []
                        }
                    })
                };
                if send_jsonrpc_line(&mut stdin, &sess_req).is_err() {
                    break;
                }
                stage = 2;
                continue;
            }

            if (id_num == 2 || id_num == 3) && stage == 2 {
                let res = v.get("result").cloned().unwrap_or(serde_json::Value::Null);
                if let Some(sid) = res
                    .get("sessionId")
                    .and_then(|s| s.as_str())
                    .filter(|s| !s.trim().is_empty())
                {
                    resolved_sid = Some(sid.trim().to_string());
                }
                let Some(ref sid) = resolved_sid else {
                    if emit_pre_prompt_errors {
                        emit_stream_event(
                            pipe_writer,
                            &HarnessStreamEvent {
                                turn_id: turn_id.to_string(),
                                harness: harness_id.to_string(),
                                kind: "error".to_string(),
                                session_id: None,
                                text: None,
                                tool_name: None,
                                error: Some("ACP Agent 未返回有效 sessionId".to_string()),
                            },
                        );
                    }
                    break;
                };
                emit_stream_event(
                    pipe_writer,
                    &HarnessStreamEvent {
                        turn_id: turn_id.to_string(),
                        harness: harness_id.to_string(),
                        kind: "session".to_string(),
                        session_id: Some(sid.clone()),
                        text: None,
                        tool_name: None,
                        error: None,
                    },
                );

                // Apply dynamic ACP config options (model / mode / effort) live on the session!
                let parsed_caps = parse_acp_session_options_from_result(&res);
                if let Some(m) = model.map(|s| s.trim()).filter(|s| !s.is_empty() && *s != "default") {
                    let has_legacy_models = res.pointer("/models/availableModels").is_some();
                    if has_legacy_models {
                        next_req_id += 1;
                        let _ = send_jsonrpc_line(
                            &mut stdin,
                            &serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": next_req_id,
                                "method": "session/set_model",
                                "params": { "sessionId": sid, "modelId": m }
                            }),
                        );
                    }
                    if let Some(ref cfg_id) = parsed_caps.model_config_id {
                        let base_m = m
                            .strip_suffix("-xhigh")
                            .or_else(|| m.strip_suffix("-high"))
                            .or_else(|| m.strip_suffix("-medium"))
                            .or_else(|| m.strip_suffix("-low"))
                            .unwrap_or(m);
                        let matched_val = if parsed_caps.models.iter().any(|x| x.id == m) {
                            m
                        } else if parsed_caps.models.iter().any(|x| x.id == base_m) {
                            base_m
                        } else {
                            m
                        };
                        next_req_id += 1;
                        let _ = send_jsonrpc_line(
                            &mut stdin,
                            &serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": next_req_id,
                                "method": "session/set_config_option",
                                "params": { "sessionId": sid, "configId": cfg_id, "value": matched_val }
                            }),
                        );
                    }
                }
                if let Some(md) = mode.map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    let mapped_md = match md {
                        "turbo" => "auto",
                        "request-review" => "plan",
                        other => other,
                    };
                    if let Some(ref cfg_id) = parsed_caps.mode_config_id {
                        if parsed_caps.modes.iter().any(|x| x.id == mapped_md) {
                            next_req_id += 1;
                            let _ = send_jsonrpc_line(
                                &mut stdin,
                                &serde_json::json!({
                                    "jsonrpc": "2.0",
                                    "id": next_req_id,
                                    "method": "session/set_config_option",
                                    "params": { "sessionId": sid, "configId": cfg_id, "value": mapped_md }
                                }),
                            );
                        }
                    } else if res.pointer("/modes/availableModes").is_some() {
                        next_req_id += 1;
                        let _ = send_jsonrpc_line(
                            &mut stdin,
                            &serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": next_req_id,
                                "method": "session/set_mode",
                                "params": { "sessionId": sid, "modeId": mapped_md }
                            }),
                        );
                    }
                }
                if let Some(ef) = effort.map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    if let Some(ref cfg_id) = parsed_caps.effort_config_id {
                        let mapped_ef = if ef == "max"
                            && !parsed_caps.efforts.iter().any(|x| x.id == "max")
                            && parsed_caps.efforts.iter().any(|x| x.id == "xhigh")
                        {
                            "xhigh"
                        } else {
                            ef
                        };
                        next_req_id += 1;
                        let _ = send_jsonrpc_line(
                            &mut stdin,
                            &serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": next_req_id,
                                "method": "session/set_config_option",
                                "params": { "sessionId": sid, "configId": cfg_id, "value": mapped_ef }
                            }),
                        );
                    }
                }

                // Build ACP prompt content blocks (text + optional base64 images)
                let mut blocks: Vec<serde_json::Value> = Vec::new();
                if supports_image {
                    use base64::Engine as _;
                    for att in attachments {
                        if att.kind == "image" && !att.path.trim().is_empty() {
                            if let Ok(bytes) = fs::read(att.path.trim()) {
                                let mime = if att.path.to_ascii_lowercase().ends_with(".jpg")
                                    || att.path.to_ascii_lowercase().ends_with(".jpeg")
                                {
                                    "image/jpeg"
                                } else if att.path.to_ascii_lowercase().ends_with(".webp") {
                                    "image/webp"
                                } else {
                                    "image/png"
                                };
                                blocks.push(serde_json::json!({
                                    "type": "image",
                                    "mimeType": mime,
                                    "data": base64::engine::general_purpose::STANDARD.encode(bytes)
                                }));
                            }
                        }
                    }
                }
                blocks.push(serde_json::json!({
                    "type": "text",
                    "text": full_prompt_text
                }));

                next_req_id += 1;
                prompt_req_id = next_req_id;
                let prompt_req = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": prompt_req_id,
                    "method": "session/prompt",
                    "params": {
                        "sessionId": sid,
                        "prompt": blocks
                    }
                });
                if let Err(e) = send_jsonrpc_line(&mut stdin, &prompt_req) {
                    emit_stream_event(
                        pipe_writer,
                        &HarnessStreamEvent {
                            turn_id: turn_id.to_string(),
                            harness: harness_id.to_string(),
                            kind: "error".to_string(),
                            session_id: resolved_sid.clone(),
                            text: None,
                            tool_name: None,
                            error: Some(format!("ACP session/prompt 发送失败: {e}")),
                        },
                    );
                    finished = true;
                    break;
                }
                stage = 3;
                continue;
            }

            if id_num == prompt_req_id && stage == 3 {
                if is_acp_harness_id(harness_id) {
                    if let Some(ref sid) = resolved_sid {
                        record_acp_turn_history(
                            harness_id,
                            sid,
                            &effective_cwd,
                            model,
                            effort,
                            mode,
                            prompt,
                            &accumulated_text,
                        );
                    }
                }
                emit_stream_event(
                    pipe_writer,
                    &HarnessStreamEvent {
                        turn_id: turn_id.to_string(),
                        harness: harness_id.to_string(),
                        kind: "completed".to_string(),
                        session_id: resolved_sid.clone(),
                        text: if accumulated_text.is_empty() {
                            None
                        } else {
                            Some(accumulated_text.clone())
                        },
                        tool_name: None,
                        error: None,
                    },
                );
                finished = true;
                break;
            }
            continue;
        }

        // Handle ACP `session/update` notifications
        if v.get("method").and_then(|m| m.as_str()) == Some("session/update") {
            let Some(update) = v.pointer("/params/update") else {
                continue;
            };
            let kind = update
                .get("sessionUpdate")
                .and_then(|s| s.as_str())
                .unwrap_or("");
            match kind {
                "agent_message_chunk" => {
                    if let Some(txt) = update
                        .pointer("/content/text")
                        .and_then(|s| s.as_str())
                        .filter(|s| !s.is_empty())
                    {
                        accumulated_text.push_str(txt);
                        emit_stream_event(
                            pipe_writer,
                            &HarnessStreamEvent {
                                turn_id: turn_id.to_string(),
                                harness: harness_id.to_string(),
                                kind: "delta".to_string(),
                                session_id: resolved_sid.clone(),
                                text: Some(txt.to_string()),
                                tool_name: None,
                                error: None,
                            },
                        );
                    }
                }
                "tool_call" | "tool_call_update" => {
                    let title = update
                        .get("title")
                        .or_else(|| update.get("kind"))
                        .and_then(|s| s.as_str())
                        .unwrap_or("工具调用");
                    emit_stream_event(
                        pipe_writer,
                        &HarnessStreamEvent {
                            turn_id: turn_id.to_string(),
                            harness: harness_id.to_string(),
                            kind: "tool".to_string(),
                            session_id: resolved_sid.clone(),
                            text: None,
                            tool_name: Some(title.to_string()),
                            error: None,
                        },
                    );
                }
                _ => {}
            }
        }
    }

    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();

    if !finished && (stage >= 3 || emit_pre_prompt_errors) {
        emit_stream_event(
            pipe_writer,
            &HarnessStreamEvent {
                turn_id: turn_id.to_string(),
                harness: harness_id.to_string(),
                kind: "error".to_string(),
                session_id: resolved_sid,
                text: None,
                tool_name: None,
                error: Some("ACP Agent 提前退出了连接".to_string()),
            },
        );
    }
    finished || stage >= 3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_acp_v1_config_options_and_legacy_modes() {
        let sample = serde_json::json!({
            "sessionId": "ses_123",
            "configOptions": [
                {
                    "id": "model",
                    "name": "Model",
                    "category": "model",
                    "type": "select",
                    "currentValue": "opencode-go/deepseek-v4-pro",
                    "options": [
                        { "value": "opencode-go/deepseek-v4-pro", "name": "DeepSeek V4 Pro" },
                        { "value": "opencode-go/claude-haiku-5-5", "name": "Claude Haiku 5.5" }
                    ]
                },
                {
                    "id": "mode",
                    "name": "Mode",
                    "category": "mode",
                    "type": "select",
                    "currentValue": "code",
                    "options": [
                        { "value": "code", "name": "Code", "description": "Full coding mode" },
                        { "value": "plan", "name": "Plan", "description": "Read-only planning" }
                    ]
                }
            ]
        });
        let caps = parse_acp_session_options_from_result(&sample);
        assert_eq!(caps.models.len(), 2);
        assert_eq!(caps.default_model, "opencode-go/deepseek-v4-pro");
        assert_eq!(caps.model_config_id.as_deref(), Some("model"));
        assert_eq!(caps.modes.len(), 2);
        assert_eq!(caps.default_mode, "code");
        assert_eq!(caps.mode_config_id.as_deref(), Some("mode"));
    }

    #[test]
    fn normalizes_custom_acp_slugs_and_discovers_entries() {
        assert_eq!(normalize_acp_slug("acp:My Custom Agent!"), "my-custom-agent");
        let custom = vec![CustomAcpHarness {
            id: "my-bot".to_string(),
            label: "My Bot".to_string(),
            command: "definitely_nonexistent_acp_bin_999".to_string(),
            args: vec!["--acp".to_string()],
            accent: "#22d3ee".to_string(),
            default_cwd: None,
        }];
        let list = discover_acp_agents(&custom);
        let found = list.iter().find(|a| a.id == "acp:my-bot").expect("custom agent must appear");
        assert!(found.custom);
        assert!(!found.available);
        assert_eq!(found.badge, "MY");
    }
}
