//! Pluggable Deep App Adapter System (`atp-adapter/v1`).
//!
//! Unifies all 15 built-in per-app deep tracking adapters and externally installed community
//! plugins into a single hot-pluggable registry stored under
//! `%LOCALAPPDATA%\AnythingToPrompt\adapter-plugins\`:
//!
//! - **Hot Plug/Unplug (zero restart)**: toggle any built-in or external adapter on/off instantly.
//! - **3 Runtime Modes**:
//!   1. `builtin`: Rust native adapters (`solidworks`, `office`, `browser`, `ide`, `explorer`,
//!      `terminal`, `pdf`, `chat`, `outlook`, `notes`, `cad`, `engine`, `dev_design`, `media`,
//!      `sys_tools`).
//!   2. `declarative`: zero-code JSON plugins combining `uiProfile` grounding rules and
//!      deterministic metadata extractors (0ms overhead, 100% sandboxed).
//!   3. `command`: external executable / script plugins (`.py`, `.mjs`, `.ps1`, `.exe`) communicating
//!      over `json_stdio_v1` (`atp-adapter/v1`) with strict timeout kill, 64KB stdout cap, and
//!      explicit user trust confirmation before execution.
//! - **Community Distribution**: install from HTTPS URL (auto-rewrites GitHub blob URLs to raw),
//!   `.zip` bundle, local `.json` / folder, or community Registry Index (`registry.json`), and
//!   export any plugin as a shareable single-file JSON.

use crate::broker::BrokerRequest;
use crate::config::AppConfig;
use crate::pick::adapters::{AdapterContext, AdapterOutput};
use crate::pick::profile::{self, AppUiProfile, GroundingLayer, ProfileSource, UiaPolicy};
use anyhow::{anyhow, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use windows::Win32::System::Threading::CREATE_NO_WINDOW;

pub const ADAPTER_PLUGIN_SCHEMA_VERSION: u32 = 1;
pub const ADAPTER_PROTOCOL_V1: &str = "atp-adapter/v1";
pub const DEFAULT_REGISTRY_URL: &str =
    "https://raw.githubusercontent.com/Zegel-None/Anything-To-Prompt/main/adapter-registry/index.json";

const MAX_DOWNLOAD_BYTES: usize = 2 * 1024 * 1024; // 2 MiB safety cap
const MAX_PLUGIN_STDOUT_BYTES: usize = 64 * 1024; // 64 KiB stdout cap
const MIN_TIMEOUT_MS: u64 = 200;
const MAX_TIMEOUT_MS: u64 = 3500;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PluginMatchRule {
    pub exes: Vec<String>,
    pub window_classes: Vec<String>,
    pub title_contains: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PluginUiProfilePatch {
    pub uia_policy: Option<UiaPolicy>,
    pub preferred_layers: Option<Vec<GroundingLayer>>,
    pub ignore_background: Option<bool>,
    pub toolbar_classes: Vec<String>,
    pub ignore_canvas_classes: Vec<String>,
    pub status_bar_classes: Vec<String>,
    pub uia_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeclarativeExtractRule {
    /// Source field: "window_title" | "cmdline" | "exe_path" | "element_name" | "element_value" | "window_class"
    pub field: String,
    /// Extractor op: "whole" | "contains" | "prefix" | "suffix" | "between" | "split" | "file_ext"
    #[serde(default = "default_extract_op")]
    pub op: String,
    #[serde(default)]
    pub arg1: String,
    #[serde(default)]
    pub arg2: String,
    #[serde(default)]
    pub index: i32,
    pub output_key: String,
    #[serde(default)]
    pub is_path: bool,
    #[serde(default)]
    pub set_badge: bool,
}

fn default_extract_op() -> String {
    "whole".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PluginRuntime {
    Builtin,
    Declarative {
        #[serde(default)]
        rules: Vec<DeclarativeExtractRule>,
        #[serde(default, rename = "badgeTemplate")]
        badge_template: Option<String>,
        #[serde(default, rename = "detailTemplate")]
        detail_template: Option<String>,
    },
    Command {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default, rename = "inlineScript")]
        inline_script: Option<String>,
        #[serde(default, rename = "inlineFilename")]
        inline_filename: Option<String>,
        #[serde(default = "default_protocol")]
        protocol: String,
    },
}

fn default_protocol() -> String {
    "json_stdio_v1".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterPluginManifest {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub homepage: String,
    #[serde(default = "default_category")]
    pub category: String,
    #[serde(default, rename = "match")]
    pub match_rule: PluginMatchRule,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default = "default_priority")]
    pub priority: i32,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub override_builtin: bool,
    #[serde(default)]
    pub replaces_builtin_id: Option<String>,
    /// "inherit" (default) or "medium" (runs PowerShell script via medium-integrity broker for COM access).
    #[serde(default = "default_integrity")]
    pub integrity: String,
    #[serde(default)]
    pub ui_profile: Option<PluginUiProfilePatch>,
    pub runtime: PluginRuntime,
}

fn default_schema_version() -> u32 {
    ADAPTER_PLUGIN_SCHEMA_VERSION
}
fn default_version() -> String {
    "1.0.0".to_string()
}
fn default_category() -> String {
    "general".to_string()
}
fn default_priority() -> i32 {
    100
}
fn default_timeout_ms() -> u64 {
    1200
}
fn default_integrity() -> String {
    "inherit".to_string()
}

#[derive(Debug, Clone)]
struct InstalledExternalPlugin {
    manifest: AdapterPluginManifest,
    dir: PathBuf,
    source_url: Option<String>,
    installed_at_unix: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct PluginStateFile {
    disabled_builtins: HashSet<String>,
    enabled_externals: HashMap<String, bool>,
    trusted_commands: HashSet<String>,
    source_urls: HashMap<String, String>,
    installed_at: HashMap<String, i64>,
    registry_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterPluginItem {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub homepage: String,
    pub category: String,
    pub exes: Vec<String>,
    pub capabilities: Vec<String>,
    pub runtime_kind: String, // "builtin" | "declarative" | "command"
    pub command_preview: Option<String>,
    pub is_builtin: bool,
    pub enabled: bool,
    pub requires_trust_confirm: bool,
    pub override_builtin: bool,
    pub timeout_ms: u64,
    pub source_url: Option<String>,
    pub installed_at_unix: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterPluginsOverview {
    pub plugins: Vec<AdapterPluginItem>,
    pub plugins_dir: String,
    pub registry_url: String,
    pub builtin_count: usize,
    pub external_count: usize,
    pub enabled_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryPluginEntry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub category: String,
    pub exes: Vec<String>,
    pub runtime_kind: String,
    pub download_url: String,
    #[serde(default)]
    pub homepage: String,
    #[serde(default)]
    pub inline_manifest: Option<serde_json::Value>,
    #[serde(default)]
    pub installed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryFetchResult {
    pub ok: bool,
    pub source: String, // "remote" | "curated_fallback"
    pub registry_url: String,
    pub warning: Option<String>,
    pub entries: Vec<RegistryPluginEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginTestReport {
    pub ok: bool,
    pub plugin_id: String,
    pub runtime_kind: String,
    pub latency_ms: u64,
    pub badge: Option<String>,
    pub lines: Vec<String>,
    pub paths: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalTapOverrideResult {
    pub plugin_id: String,
    pub title: String,
    pub detail: String,
    pub badge: Option<String>,
    pub lines: Vec<String>,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StdioPluginRequest {
    protocol: String,
    hook: String,
    hwnd: isize,
    pid: u32,
    exe_name: String,
    exe_path: String,
    cmdline: String,
    window_title: String,
    window_class: String,
    tap_point: Option<[i32; 2]>,
    primary_element: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct StdioPluginResponse {
    pub ok: bool,
    pub badge: Option<String>,
    pub title: Option<String>,
    pub detail: Option<String>,
    pub lines: Vec<String>,
    pub paths: Vec<String>,
    pub override_capsule: bool,
    pub error: Option<String>,
}

struct PluginStore {
    builtins: Vec<AdapterPluginManifest>,
    externals: HashMap<String, InstalledExternalPlugin>,
    state: PluginStateFile,
    disk_loaded: bool,
}

static STORE: OnceLock<Mutex<PluginStore>> = OnceLock::new();

fn store() -> &'static Mutex<PluginStore> {
    STORE.get_or_init(|| {
        Mutex::new(PluginStore {
            builtins: builtin_adapter_manifests(),
            externals: HashMap::new(),
            state: PluginStateFile::default(),
            disk_loaded: false,
        })
    })
}

pub fn plugins_dir() -> PathBuf {
    AppConfig::local_data_dir().join("adapter-plugins")
}

fn state_file_path() -> PathBuf {
    plugins_dir().join("state.json")
}

fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn builtin_manifest(
    id: &str,
    name: &str,
    category: &str,
    exes: &[&str],
    capabilities: &[&str],
    description: &str,
) -> AdapterPluginManifest {
    AdapterPluginManifest {
        schema_version: ADAPTER_PLUGIN_SCHEMA_VERSION,
        id: id.to_string(),
        name: name.to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        author: "Anything-to-Prompt Official".to_string(),
        description: description.to_string(),
        homepage: "https://github.com/Zegel-None/Anything-To-Prompt".to_string(),
        category: category.to_string(),
        match_rule: PluginMatchRule {
            exes: exes.iter().map(|s| s.to_ascii_lowercase()).collect(),
            window_classes: Vec::new(),
            title_contains: Vec::new(),
        },
        capabilities: capabilities.iter().map(|s| s.to_string()).collect(),
        priority: 50,
        timeout_ms: 1500,
        override_builtin: false,
        replaces_builtin_id: None,
        integrity: "medium".to_string(),
        ui_profile: None,
        runtime: PluginRuntime::Builtin,
    }
}

pub fn builtin_adapter_manifests() -> Vec<AdapterPluginManifest> {
    vec![
        builtin_manifest(
            "solidworks",
            "SOLIDWORKS 3D CAD 深度追踪",
            "cad_3d",
            &["sldworks.exe"],
            &["tap_override", "auto_attach", "enrich_pick", "ui_profile"],
            "零 UIA 原生 COM 射线拾取面/边/顶点、特征树节点参数/尺寸与工具栏识别",
        ),
        builtin_manifest(
            "office",
            "Office / WPS 办公套件追踪",
            "office",
            &[
                "winword.exe",
                "excel.exe",
                "powerpnt.exe",
                "wps.exe",
                "et.exe",
                "wpp.exe",
            ],
            &["tap_override", "enrich_pick"],
            "Excel 单元格坐标与公式、Word 选区段落与样式、PPT 元素结构化读取",
        ),
        builtin_manifest(
            "ide",
            "IDE / 代码编辑器深度追踪",
            "dev",
            &[
                "code.exe",
                "cursor.exe",
                "windsurf.exe",
                "devenv.exe",
                "idea64.exe",
                "pycharm64.exe",
                "clion64.exe",
                "webstorm64.exe",
            ],
            &["enrich_pick"],
            "自动定位工作区根路径、当前打开源码文件与行号上下文",
        ),
        builtin_manifest(
            "browser",
            "浏览器地址栏与页面追踪",
            "web",
            &[
                "chrome.exe",
                "msedge.exe",
                "firefox.exe",
                "brave.exe",
                "arc.exe",
                "opera.exe",
            ],
            &["enrich_pick"],
            "读取当前活动标签页真实 URL、标题与页面 DOM 控件语义",
        ),
        builtin_manifest(
            "explorer",
            "资源管理器与桌面选中文件追踪",
            "system",
            &["explorer.exe"],
            &["auto_attach", "enrich_pick"],
            "通过 Shell COM 读取当前选中的文件/文件夹真实路径与元数据",
        ),
        builtin_manifest(
            "terminal",
            "终端与命令行环境追踪",
            "dev",
            &[
                "windowsterminal.exe",
                "powershell.exe",
                "pwsh.exe",
                "cmd.exe",
                "warp.exe",
                "alacritty.exe",
                "wezterm-gui.exe",
            ],
            &["enrich_pick"],
            "识别终端当前工作目录、Shell 进程与窗口上下文",
        ),
        builtin_manifest(
            "pdf",
            "PDF 阅读器路径与页码追踪",
            "office",
            &[
                "acrobat.exe",
                "acrord32.exe",
                "sumatrapdf.exe",
                "foxitpdfreader.exe",
                "foxitreader.exe",
                "pdfxedit.exe",
            ],
            &["enrich_pick"],
            "提取打开的 PDF 本地文件路径与当前阅读页码",
        ),
        builtin_manifest(
            "chat",
            "聊天软件消息上下文追踪",
            "communication",
            &[
                "weixin.exe",
                "wechat.exe",
                "feishu.exe",
                "lark.exe",
                "dingtalk.exe",
                "qq.exe",
                "ms-teams.exe",
                "slack.exe",
                "telegram.exe",
                "discord.exe",
            ],
            &["enrich_pick"],
            "点选某条聊天消息时自动采样前后 ±5 条上下文对话",
        ),
        builtin_manifest(
            "outlook",
            "Outlook 邮件深度追踪",
            "communication",
            &["outlook.exe", "olk.exe"],
            &["enrich_pick"],
            "读取当前查看或选中邮件的主题、发件人、时间与正文摘要",
        ),
        builtin_manifest(
            "notes",
            "笔记软件（Obsidian / Notion / Typora）",
            "office",
            &["obsidian.exe", "onenote.exe", "notion.exe", "typora.exe"],
            &["enrich_pick"],
            "解析 Obsidian Vault 根路径与 Markdown 源文件路径",
        ),
        builtin_manifest(
            "cad",
            "AutoCAD 等通用 CAD 追踪",
            "cad_3d",
            &["acad.exe", "zwcad.exe", "gstarcad.exe"],
            &["enrich_pick"],
            "通过中权限 COM 读取 AutoCAD 活动图纸路径与文档状态",
        ),
        builtin_manifest(
            "engine",
            "3D / 游戏引擎（Unity / Unreal / Blender）",
            "cad_3d",
            &["unity.exe", "unrealeditor.exe", "blender.exe"],
            &["enrich_pick"],
            "解析 Unity/Unreal 工程根目录及 Blender 活动 `.blend` 文件路径",
        ),
        builtin_manifest(
            "dev_design",
            "设计与研发工具（Figma / PS / Postman / DB）",
            "design",
            &[
                "figma.exe",
                "photoshop.exe",
                "githubdesktop.exe",
                "postman.exe",
                "dbeaver.exe",
                "navicat.exe",
            ],
            &["enrich_pick"],
            "提取设计稿/接口集合/数据库连接上下文与源文件信息",
        ),
        builtin_manifest(
            "media",
            "音视频播放器追踪（PotPlayer / VLC / mpv）",
            "media",
            &["potplayermini64.exe", "vlc.exe", "mpv.exe"],
            &["enrich_pick"],
            "解析当前正在播放的音视频本地文件路径与媒体标题",
        ),
        builtin_manifest(
            "sys_tools",
            "系统与效率工具（Everything / TotalCmd / Docker）",
            "system",
            &[
                "taskmgr.exe",
                "procexp64.exe",
                "everything.exe",
                "totalcmd64.exe",
                "docker desktop.exe",
            ],
            &["enrich_pick"],
            "适配系统工具、对比工具与容器管理工具的窗口上下文",
        ),
    ]
}

pub fn validate_plugin_id(id: &str) -> Result<String> {
    let trimmed = id.trim().to_ascii_lowercase();
    if trimmed.len() < 2 || trimmed.len() > 64 {
        return Err(anyhow!("插件 ID 长度需在 2~64 字符之间"));
    }
    let bytes = trimmed.as_bytes();
    if !bytes[0].is_ascii_alphanumeric() {
        return Err(anyhow!("插件 ID 必须以字母或数字开头"));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err(anyhow!("插件 ID 仅允许小写字母、数字、'-' 与 '_'"));
    }
    Ok(trimmed)
}

fn validate_safe_rel_filename(name: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains("..")
        || trimmed.contains(':')
    {
        return Err(anyhow!("插件脚本文件名不合法（禁止路径穿越）: {trimmed}"));
    }
    Ok(trimmed.to_string())
}

pub fn normalize_and_validate_manifest(mut m: AdapterPluginManifest) -> Result<AdapterPluginManifest> {
    if m.schema_version != ADAPTER_PLUGIN_SCHEMA_VERSION {
        return Err(anyhow!(
            "不支持的插件 schemaVersion={}（当前仅支持 {}）",
            m.schema_version,
            ADAPTER_PLUGIN_SCHEMA_VERSION
        ));
    }
    m.id = validate_plugin_id(&m.id)?;
    if m.name.trim().is_empty() {
        m.name = m.id.clone();
    }
    m.timeout_ms = m.timeout_ms.clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS);
    m.match_rule.exes = m
        .match_rule
        .exes
        .iter()
        .map(|e| profile::normalize_exe_key(e))
        .filter(|e| !e.is_empty())
        .collect();
    m.match_rule.window_classes = m
        .match_rule
        .window_classes
        .iter()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    m.match_rule.title_contains = m
        .match_rule
        .title_contains
        .iter()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();

    if m.match_rule.exes.is_empty()
        && m.match_rule.window_classes.is_empty()
        && m.match_rule.title_contains.is_empty()
    {
        return Err(anyhow!(
            "插件 '{}' 的 match 规则不能为空（至少填写 exes / windowClasses / titleContains 之一）",
            m.id
        ));
    }

    match &mut m.runtime {
        PluginRuntime::Builtin => {
            return Err(anyhow!("外部插件不允许声明 runtime.kind = builtin"));
        }
        PluginRuntime::Declarative { .. } => {}
        PluginRuntime::Command {
            command,
            args,
            inline_filename,
            protocol,
            ..
        } => {
            if command.trim().is_empty() {
                return Err(anyhow!("command 运行时插件必须提供 command 字段"));
            }
            for a in args.iter() {
                if a.contains("..") {
                    return Err(anyhow!("插件 args 中禁止包含 '..' 路径穿越: {a}"));
                }
            }
            if let Some(fname) = inline_filename.as_mut() {
                *fname = validate_safe_rel_filename(fname)?;
            }
            if protocol.trim().is_empty() {
                *protocol = "json_stdio_v1".to_string();
            }
        }
    }

    Ok(m)
}

fn save_state_file(state: &PluginStateFile) -> Result<()> {
    let dir = plugins_dir();
    fs::create_dir_all(&dir)?;
    let path = state_file_path();
    let raw = serde_json::to_string_pretty(state)?;
    fs::write(&path, raw)?;
    Ok(())
}

fn load_state_file() -> PluginStateFile {
    let path = state_file_path();
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<PluginStateFile>(&s).ok())
        .unwrap_or_default()
}

fn scan_externals_from_disk(state: &PluginStateFile) -> HashMap<String, InstalledExternalPlugin> {
    let mut map = HashMap::new();
    let root = plugins_dir();
    let Ok(entries) = fs::read_dir(&root) else {
        return map;
    };
    for ent in entries.flatten() {
        let p = ent.path();
        if !p.is_dir() {
            continue;
        }
        let manifest_path = p.join("adapter.json");
        if !manifest_path.is_file() {
            continue;
        }
        let Ok(raw) = fs::read_to_string(&manifest_path) else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<AdapterPluginManifest>(&raw) else {
            log::warn!(
                "[ADAPTER-PLUGIN] Failed to parse manifest: {}",
                manifest_path.display()
            );
            continue;
        };
        match normalize_and_validate_manifest(parsed) {
            Ok(m) => {
                let id = m.id.clone();
                let source_url = state.source_urls.get(&id).cloned();
                let installed_at_unix = state.installed_at.get(&id).copied().unwrap_or(0);
                map.insert(
                    id,
                    InstalledExternalPlugin {
                        manifest: m,
                        dir: p,
                        source_url,
                        installed_at_unix,
                    },
                );
            }
            Err(e) => {
                log::warn!(
                    "[ADAPTER-PLUGIN] Invalid manifest at {}: {e}",
                    manifest_path.display()
                );
            }
        }
    }
    map
}

fn ensure_loaded(st: &mut PluginStore) {
    if st.disk_loaded {
        return;
    }
    st.state = load_state_file();
    st.externals = scan_externals_from_disk(&st.state);
    st.disk_loaded = true;
}

fn is_external_enabled(st: &PluginStore, ext: &InstalledExternalPlugin) -> bool {
    let id = &ext.manifest.id;
    let is_cmd = matches!(ext.manifest.runtime, PluginRuntime::Command { .. });
    if is_cmd && !st.state.trusted_commands.contains(id) {
        return false;
    }
    st.state
        .enabled_externals
        .get(id)
        .copied()
        .unwrap_or(!is_cmd)
}

fn to_plugin_item(st: &PluginStore, m: &AdapterPluginManifest, ext: Option<&InstalledExternalPlugin>) -> AdapterPluginItem {
    let is_builtin = matches!(m.runtime, PluginRuntime::Builtin);
    let (runtime_kind, command_preview) = match &m.runtime {
        PluginRuntime::Builtin => ("builtin".to_string(), None),
        PluginRuntime::Declarative { .. } => ("declarative".to_string(), None),
        PluginRuntime::Command { command, args, .. } => {
            let preview = if args.is_empty() {
                command.clone()
            } else {
                format!("{command} {}", args.join(" "))
            };
            ("command".to_string(), Some(preview))
        }
    };
    let (enabled, requires_trust_confirm, source_url, installed_at_unix) = if is_builtin {
        (!st.state.disabled_builtins.contains(&m.id), false, None, 0)
    } else if let Some(ext_rec) = ext {
        let is_cmd = matches!(m.runtime, PluginRuntime::Command { .. });
        let trusted = !is_cmd || st.state.trusted_commands.contains(&m.id);
        let en = is_external_enabled(st, ext_rec);
        (
            en,
            is_cmd && !trusted,
            ext_rec.source_url.clone(),
            ext_rec.installed_at_unix,
        )
    } else {
        (false, false, None, 0)
    };

    AdapterPluginItem {
        id: m.id.clone(),
        name: m.name.clone(),
        version: m.version.clone(),
        author: m.author.clone(),
        description: m.description.clone(),
        homepage: m.homepage.clone(),
        category: m.category.clone(),
        exes: m.match_rule.exes.clone(),
        capabilities: m.capabilities.clone(),
        runtime_kind,
        command_preview,
        is_builtin,
        enabled,
        requires_trust_confirm,
        override_builtin: m.override_builtin,
        timeout_ms: m.timeout_ms,
        source_url,
        installed_at_unix,
    }
}

fn build_overview_locked(st: &PluginStore) -> AdapterPluginsOverview {
    let mut plugins = Vec::with_capacity(st.builtins.len() + st.externals.len());
    for ext in st.externals.values() {
        plugins.push(to_plugin_item(st, &ext.manifest, Some(ext)));
    }
    for b in &st.builtins {
        plugins.push(to_plugin_item(st, b, None));
    }
    plugins.sort_by(|a, b| {
        a.is_builtin
            .cmp(&b.is_builtin)
            .then_with(|| b.enabled.cmp(&a.enabled))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    let enabled_count = plugins.iter().filter(|p| p.enabled).count();
    let registry_url = st
        .state
        .registry_url
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_REGISTRY_URL.to_string());
    AdapterPluginsOverview {
        builtin_count: st.builtins.len(),
        external_count: st.externals.len(),
        enabled_count,
        plugins,
        plugins_dir: plugins_dir().to_string_lossy().to_string(),
        registry_url,
    }
}

/// Fast O(1) runtime check used by every built-in deep adapter (`solidworks`, `office`, `ide`,
/// `browser`, `explorer`, `terminal`, `pdf`, `chat`, `outlook`, `notes`, `cad`, `engine`,
/// `dev_design`, `media`, `sys_tools`).
///
/// Also returns `false` if an enabled external plugin explicitly declares `replacesBuiltinId == id`.
pub fn is_builtin_adapter_enabled(id: &str) -> bool {
    let Ok(mut st) = store().lock() else {
        return true;
    };
    ensure_loaded(&mut st);
    if st.state.disabled_builtins.contains(id) {
        return false;
    }
    for ext in st.externals.values() {
        if is_external_enabled(&st, ext)
            && ext
                .manifest
                .replaces_builtin_id
                .as_deref()
                .is_some_and(|r| r.eq_ignore_ascii_case(id))
        {
            return false;
        }
    }
    true
}

/// Checks if an enabled external plugin replaces the built-in adapter for `exe_name` with
/// `overrideBuiltin = true`.
pub fn has_enabled_external_override_for_exe(exe_name: &str) -> bool {
    let key = profile::normalize_exe_key(exe_name);
    if key.is_empty() {
        return false;
    }
    let Ok(mut st) = store().lock() else {
        return false;
    };
    ensure_loaded(&mut st);
    st.externals.values().any(|ext| {
        is_external_enabled(&st, ext)
            && ext.manifest.override_builtin
            && ext.manifest.match_rule.exes.iter().any(|e| e == &key)
    })
}

/// Applies any enabled external plugin's `uiProfile` overlay onto `base_profile` (called by
/// `profile::get_effective_profile`). Hard safety invariants (`SLDWORKS.exe` zero-UIA) are always
/// re-enforced afterwards by `profile::enforce_hard_invariants`.
pub fn apply_plugin_ui_profile_overlay(exe_name: &str, base: &mut AppUiProfile) {
    let key = profile::normalize_exe_key(exe_name);
    if key.is_empty() {
        return;
    }
    let Ok(mut st) = store().lock() else {
        return;
    };
    ensure_loaded(&mut st);

    let mut matching: Vec<&AdapterPluginManifest> = st
        .externals
        .values()
        .filter(|ext| {
            is_external_enabled(&st, ext)
                && ext.manifest.ui_profile.is_some()
                && ext.manifest.match_rule.exes.iter().any(|e| e == &key)
        })
        .map(|ext| &ext.manifest)
        .collect();
    matching.sort_by_key(|m| m.priority);

    for m in matching {
        let Some(patch) = m.ui_profile.as_ref() else {
            continue;
        };
        if let Some(pol) = patch.uia_policy {
            base.uia_policy = pol;
        }
        if let Some(layers) = patch.preferred_layers.as_ref() {
            if !layers.is_empty() {
                base.preferred_layers = layers.clone();
            }
        }
        if let Some(ig) = patch.ignore_background {
            base.ignore_background = ig;
        }
        if let Some(t) = patch.uia_timeout_ms {
            base.uia_timeout_ms = t;
        }
        for c in &patch.toolbar_classes {
            let norm = c.to_ascii_lowercase();
            if !norm.is_empty() && !base.toolbar_classes.contains(&norm) {
                base.toolbar_classes.push(norm);
            }
        }
        for c in &patch.ignore_canvas_classes {
            let norm = c.to_ascii_lowercase();
            if !norm.is_empty() && !base.ignore_canvas_classes.contains(&norm) {
                base.ignore_canvas_classes.push(norm);
            }
        }
        for c in &patch.status_bar_classes {
            let norm = c.to_ascii_lowercase();
            if !norm.is_empty() && !base.status_bar_classes.contains(&norm) {
                base.status_bar_classes.push(norm);
            }
        }
        base.source = ProfileSource::Imported;
        base.semantic_adapter = Some(format!("plugin:{}", m.id));
    }
}

pub fn list_plugins() -> AdapterPluginsOverview {
    let Ok(mut st) = store().lock() else {
        return AdapterPluginsOverview {
            plugins: Vec::new(),
            plugins_dir: plugins_dir().to_string_lossy().to_string(),
            registry_url: DEFAULT_REGISTRY_URL.to_string(),
            builtin_count: 0,
            external_count: 0,
            enabled_count: 0,
        };
    };
    ensure_loaded(&mut st);
    build_overview_locked(&st)
}

pub fn reload_from_disk() -> AdapterPluginsOverview {
    let Ok(mut st) = store().lock() else {
        return list_plugins();
    };
    st.state = load_state_file();
    st.externals = scan_externals_from_disk(&st.state);
    st.disk_loaded = true;
    build_overview_locked(&st)
}

pub fn set_plugin_enabled(id: &str, enabled: bool) -> Result<AdapterPluginItem> {
    let norm = id.trim().to_ascii_lowercase();
    let Ok(mut st) = store().lock() else {
        return Err(anyhow!("plugin store lock poisoned"));
    };
    ensure_loaded(&mut st);

    if let Some(b) = st.builtins.iter().find(|b| b.id == norm).cloned() {
        if enabled {
            st.state.disabled_builtins.remove(&norm);
        } else {
            st.state.disabled_builtins.insert(norm.clone());
        }
        save_state_file(&st.state)?;
        return Ok(to_plugin_item(&st, &b, None));
    }

    if let Some(ext) = st.externals.get(&norm).cloned() {
        if enabled && matches!(ext.manifest.runtime, PluginRuntime::Command { .. }) {
            // Explicitly toggling ON a command plugin records user trust consent.
            st.state.trusted_commands.insert(norm.clone());
        }
        st.state.enabled_externals.insert(norm.clone(), enabled);
        save_state_file(&st.state)?;
        return Ok(to_plugin_item(&st, &ext.manifest, Some(&ext)));
    }

    Err(anyhow!("未找到插件: {id}"))
}

pub fn uninstall_plugin(id: &str) -> Result<bool> {
    let norm = validate_plugin_id(id)?;
    let Ok(mut st) = store().lock() else {
        return Err(anyhow!("plugin store lock poisoned"));
    };
    ensure_loaded(&mut st);
    if st.builtins.iter().any(|b| b.id == norm) {
        return Err(anyhow!("内置插件不可卸载，可直接点击开关热禁用"));
    }
    let Some(ext) = st.externals.remove(&norm) else {
        return Ok(false);
    };
    if ext.dir.exists() {
        fs::remove_dir_all(&ext.dir)
            .with_context(|| format!("删除插件目录失败: {}", ext.dir.display()))?;
    }
    st.state.enabled_externals.remove(&norm);
    st.state.trusted_commands.remove(&norm);
    st.state.source_urls.remove(&norm);
    st.state.installed_at.remove(&norm);
    save_state_file(&st.state)?;
    Ok(true)
}

fn install_manifest_and_files(
    manifest: AdapterPluginManifest,
    extra_source_dir: Option<&Path>,
    source_url: Option<String>,
    trust_command: bool,
) -> Result<AdapterPluginItem> {
    let mut m = normalize_and_validate_manifest(manifest)?;

    let Ok(mut st) = store().lock() else {
        return Err(anyhow!("plugin store lock poisoned"));
    };
    ensure_loaded(&mut st);

    if st.builtins.iter().any(|b| b.id == m.id) {
        return Err(anyhow!(
            "插件 ID '{}' 与内置插件重名；若需替换内置适配器，请换用新 ID 并设置 replacesBuiltinId: \"{}\"",
            m.id,
            m.id
        ));
    }

    let dest_dir = plugins_dir().join(&m.id);
    if dest_dir.exists() {
        let _ = fs::remove_dir_all(&dest_dir);
    }
    fs::create_dir_all(&dest_dir)?;

    // Copy companion files if installing from a local unpacked folder or extracted zip
    if let Some(src_dir) = extra_source_dir {
        copy_dir_flat_safe(src_dir, &dest_dir, 0)?;
    }

    // Extract inlineScript for single-file shareable JSON command plugins
    if let PluginRuntime::Command {
        inline_script,
        inline_filename,
        args,
        command,
        ..
    } = &mut m.runtime
    {
        if let Some(script_body) = inline_script.as_ref().filter(|s| !s.trim().is_empty()) {
            let fname = if let Some(f) = inline_filename.as_ref().filter(|s| !s.trim().is_empty()) {
                validate_safe_rel_filename(f)?
            } else {
                let cmd_l = command.to_ascii_lowercase();
                if cmd_l.contains("python") {
                    "adapter.py".to_string()
                } else if cmd_l.contains("node") {
                    "adapter.mjs".to_string()
                } else if cmd_l.contains("powershell") || cmd_l.contains("pwsh") {
                    "adapter.ps1".to_string()
                } else {
                    "adapter.txt".to_string()
                }
            };
            fs::write(dest_dir.join(&fname), script_body.as_bytes())?;
            *inline_filename = Some(fname.clone());
            if args.is_empty() {
                let cmd_l = command.to_ascii_lowercase();
                if cmd_l.contains("powershell") || cmd_l.contains("pwsh") {
                    *args = vec![
                        "-NoProfile".to_string(),
                        "-ExecutionPolicy".to_string(),
                        "Bypass".to_string(),
                        "-File".to_string(),
                        fname,
                    ];
                } else {
                    *args = vec![fname];
                }
            }
        }
    }

    let manifest_json = serde_json::to_string_pretty(&m)?;
    fs::write(dest_dir.join("adapter.json"), manifest_json)?;

    let is_cmd = matches!(m.runtime, PluginRuntime::Command { .. });
    let now = now_unix_secs();
    if is_cmd {
        if trust_command {
            st.state.trusted_commands.insert(m.id.clone());
            st.state.enabled_externals.insert(m.id.clone(), true);
        } else {
            st.state.trusted_commands.remove(&m.id);
            st.state.enabled_externals.insert(m.id.clone(), false);
        }
    } else {
        st.state.enabled_externals.insert(m.id.clone(), true);
    }

    if let Some(ref u) = source_url {
        st.state.source_urls.insert(m.id.clone(), u.clone());
    }
    st.state.installed_at.insert(m.id.clone(), now);
    save_state_file(&st.state)?;

    let ext = InstalledExternalPlugin {
        manifest: m.clone(),
        dir: dest_dir,
        source_url,
        installed_at_unix: now,
    };
    st.externals.insert(m.id.clone(), ext.clone());

    Ok(to_plugin_item(&st, &m, Some(&ext)))
}

fn copy_dir_flat_safe(src: &Path, dst: &Path, depth: usize) -> Result<()> {
    if depth > 4 {
        return Err(anyhow!("插件包目录层级过深（最大 4 层）"));
    }
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let fname = entry.file_name();
        let fname_str = fname.to_string_lossy();
        if fname_str.starts_with('.') || fname_str == "__MACOSX" {
            continue;
        }
        let sp = entry.path();
        let dp = dst.join(&fname);
        let meta = fs::symlink_metadata(&sp)?;
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            copy_dir_flat_safe(&sp, &dp, depth + 1)?;
        } else if meta.is_file() {
            if meta.len() > MAX_DOWNLOAD_BYTES as u64 {
                return Err(anyhow!("插件包内单文件超过 2MB 上限: {fname_str}"));
            }
            fs::copy(&sp, &dp)?;
        }
    }
    Ok(())
}

pub fn install_from_json(
    raw_json: &str,
    source_hint: Option<&str>,
    trust_command: bool,
) -> Result<AdapterPluginItem> {
    let manifest: AdapterPluginManifest =
        serde_json::from_str(raw_json).map_err(|e| anyhow!("插件 adapter.json 格式无效: {e}"))?;
    install_manifest_and_files(
        manifest,
        None,
        source_hint.map(|s| s.to_string()),
        trust_command,
    )
}

/// Normalizes user-pasted GitHub blob URLs (`github.com/u/r/blob/branch/file`) to raw URLs.
pub fn normalize_plugin_download_url(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("下载地址不能为空"));
    }
    let parsed = url::Url::parse(trimmed).map_err(|e| anyhow!("URL 格式无效: {e}"))?;
    let scheme = parsed.scheme();
    let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
    let is_local_http = scheme == "http" && matches!(host.as_str(), "127.0.0.1" | "localhost");
    if scheme != "https" && !is_local_http {
        return Err(anyhow!("出于安全限制，仅允许 https://（或本机 localhost）插件地址"));
    }

    if host == "github.com" {
        let segs: Vec<&str> = parsed
            .path_segments()
            .map(|it| it.collect())
            .unwrap_or_default();
        // /owner/repo/blob/<branch>/<rest...> -> raw.githubusercontent.com/owner/repo/<branch>/<rest...>
        if segs.len() >= 5 && segs[2] == "blob" {
            let owner = segs[0];
            let repo = segs[1];
            let rest = segs[3..].join("/");
            return Ok(format!("https://raw.githubusercontent.com/{owner}/{repo}/{rest}"));
        }
    }
    Ok(trimmed.to_string())
}

fn read_http_bytes_capped(url: &str, timeout_secs: u64) -> Result<Vec<u8>> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(timeout_secs))
        .build();
    let resp = agent
        .get(url)
        .set("User-Agent", "Anything-to-Prompt-PluginManager/1.0")
        .call()
        .map_err(|e| anyhow!("下载失败 ({url}): {e}"))?;

    let mut buf = Vec::new();
    let mut reader = resp.into_reader().take((MAX_DOWNLOAD_BYTES + 1) as u64);
    reader.read_to_end(&mut buf)?;
    if buf.len() > MAX_DOWNLOAD_BYTES {
        return Err(anyhow!("插件体积超过 2MB 安全限制"));
    }
    Ok(buf)
}

fn find_manifest_root(dir: &Path) -> Option<PathBuf> {
    if dir.join("adapter.json").is_file() {
        return Some(dir.to_path_buf());
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return None;
    };
    for ent in entries.flatten() {
        let p = ent.path();
        if p.is_dir() && p.join("adapter.json").is_file() {
            return Some(p);
        }
    }
    None
}

fn install_from_zip_bytes(
    zip_bytes: &[u8],
    source_url: Option<String>,
    trust_command: bool,
) -> Result<AdapterPluginItem> {
    let tmp_root = std::env::temp_dir().join(format!(
        "atp-plugin-zip-{}-{}",
        std::process::id(),
        now_unix_secs()
    ));
    let _ = fs::remove_dir_all(&tmp_root);
    fs::create_dir_all(&tmp_root)?;
    let zip_path = tmp_root.join("bundle.zip");
    let extract_dir = tmp_root.join("out");
    fs::write(&zip_path, zip_bytes)?;

    let ps_cmd = format!(
        "Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force",
        zip_path.display().to_string().replace('\'', "''"),
        extract_dir.display().to_string().replace('\'', "''"),
    );
    let out = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps_cmd])
        .creation_flags(CREATE_NO_WINDOW.0)
        .output()
        .context("无法调用 PowerShell 解压插件 zip 包")?;
    if !out.status.success() {
        let _ = fs::remove_dir_all(&tmp_root);
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(anyhow!("解压插件 zip 失败: {}", stderr.trim()));
    }

    let Some(root_dir) = find_manifest_root(&extract_dir) else {
        let _ = fs::remove_dir_all(&tmp_root);
        return Err(anyhow!("zip 插件包内未找到 adapter.json"));
    };
    let raw = fs::read_to_string(root_dir.join("adapter.json"))?;
    let manifest: AdapterPluginManifest =
        serde_json::from_str(&raw).map_err(|e| anyhow!("zip 内 adapter.json 无效: {e}"))?;
    let res = install_manifest_and_files(manifest, Some(&root_dir), source_url, trust_command);
    let _ = fs::remove_dir_all(&tmp_root);
    res
}

pub fn install_from_url(raw_url: &str, trust_command: bool) -> Result<AdapterPluginItem> {
    let url = normalize_plugin_download_url(raw_url)?;
    let bytes = read_http_bytes_capped(&url, 12)?;
    let is_zip = url.to_ascii_lowercase().ends_with(".zip")
        || (bytes.len() >= 4 && &bytes[..4] == b"PK\x03\x04");
    if is_zip {
        install_from_zip_bytes(&bytes, Some(url), trust_command)
    } else {
        let text = String::from_utf8(bytes).map_err(|_| anyhow!("下载的文件不是有效 UTF-8 JSON"))?;
        install_from_json(&text, Some(&url), trust_command)
    }
}

pub fn install_from_local_path(raw_path: &str, trust_command: bool) -> Result<AdapterPluginItem> {
    let p = Path::new(raw_path.trim().trim_matches('"'));
    if !p.exists() {
        return Err(anyhow!("路径不存在: {}", p.display()));
    }
    if p.is_dir() {
        let Some(root_dir) = find_manifest_root(p) else {
            return Err(anyhow!("所选目录内未找到 adapter.json"));
        };
        let raw = fs::read_to_string(root_dir.join("adapter.json"))?;
        let manifest: AdapterPluginManifest =
            serde_json::from_str(&raw).map_err(|e| anyhow!("adapter.json 解析失败: {e}"))?;
        return install_manifest_and_files(
            manifest,
            Some(&root_dir),
            Some(format!("local://{}", root_dir.display())),
            trust_command,
        );
    }
    let ext = p
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext == "zip" {
        let bytes = fs::read(p)?;
        return install_from_zip_bytes(
            &bytes,
            Some(format!("local://{}", p.display())),
            trust_command,
        );
    }
    let text = fs::read_to_string(p)?;
    install_from_json(
        &text,
        Some(&format!("local://{}", p.display())),
        trust_command,
    )
}

pub fn export_plugin_json(id: &str) -> Result<String> {
    let norm = id.trim().to_ascii_lowercase();
    let Ok(mut st) = store().lock() else {
        return Err(anyhow!("plugin store lock poisoned"));
    };
    ensure_loaded(&mut st);

    if let Some(ext) = st.externals.get(&norm) {
        let mut m = ext.manifest.clone();
        // Bundle small script inline on export so the exported JSON is 100% self-contained!
        if let PluginRuntime::Command {
            inline_script,
            inline_filename,
            args,
            ..
        } = &mut m.runtime
        {
            if inline_script.is_none() {
                let cand = inline_filename
                    .clone()
                    .or_else(|| args.last().cloned())
                    .filter(|s| validate_safe_rel_filename(s).is_ok());
                if let Some(fname) = cand {
                    let fpath = ext.dir.join(&fname);
                    if let Ok(text) = fs::read_to_string(&fpath) {
                        if text.len() <= 256 * 1024 {
                            *inline_script = Some(text);
                            *inline_filename = Some(fname);
                        }
                    }
                }
            }
        }
        return Ok(serde_json::to_string_pretty(&m)?);
    }

    if let Some(b) = st.builtins.iter().find(|b| b.id == norm) {
        // Export a ready-to-customize community declarative template derived from the built-in adapter.
        let sample = AdapterPluginManifest {
            schema_version: ADAPTER_PLUGIN_SCHEMA_VERSION,
            id: format!("{}-custom", b.id),
            name: format!("{}（自定义扩展）", b.name),
            version: "1.0.0".to_string(),
            author: "your-name".to_string(),
            description: format!("基于 {} 的可分享深度追踪扩展模板", b.name),
            homepage: b.homepage.clone(),
            category: b.category.clone(),
            match_rule: b.match_rule.clone(),
            capabilities: vec!["enrich_pick".to_string()],
            priority: 120,
            timeout_ms: 1200,
            override_builtin: false,
            replaces_builtin_id: None,
            integrity: "inherit".to_string(),
            ui_profile: None,
            runtime: PluginRuntime::Declarative {
                rules: vec![DeclarativeExtractRule {
                    field: "window_title".to_string(),
                    op: "whole".to_string(),
                    arg1: String::new(),
                    arg2: String::new(),
                    index: 0,
                    output_key: "active_window".to_string(),
                    is_path: false,
                    set_badge: true,
                }],
                badge_template: None,
                detail_template: Some(format!("{} · 自定义插件", b.name)),
            },
        };
        return Ok(serde_json::to_string_pretty(&sample)?);
    }

    Err(anyhow!("未找到插件: {id}"))
}

/// Curated offline/fallback starter registry entries so users can browse, test, and install
/// community adapters immediately even before publishing a remote GitHub index.
pub fn curated_starter_registry_entries() -> Vec<RegistryPluginEntry> {
    vec![
        RegistryPluginEntry {
            id: "kicad-eda-deep".to_string(),
            name: "KiCad 原理图/PCB 深度追踪".to_string(),
            version: "1.0.0".to_string(),
            author: "Anything-to-Prompt Community".to_string(),
            description: "自动识别 KiCad 原理图/PCB 工程路径、当前图纸名，并配置防误触画布与工具栏识别规则（零代码即装即用）".to_string(),
            category: "cad_3d".to_string(),
            exes: vec![
                "kicad.exe".to_string(),
                "pcbnew.exe".to_string(),
                "eeschema.exe".to_string(),
            ],
            runtime_kind: "declarative".to_string(),
            download_url: "https://raw.githubusercontent.com/Zegel-None/Anything-To-Prompt/main/adapter-registry/plugins/kicad-eda-deep/adapter.json".to_string(),
            homepage: "https://github.com/Zegel-None/Anything-To-Prompt".to_string(),
            inline_manifest: Some(serde_json::json!({
                "schemaVersion": 1,
                "id": "kicad-eda-deep",
                "name": "KiCad 原理图/PCB 深度追踪",
                "version": "1.0.0",
                "author": "Anything-to-Prompt Community",
                "description": "自动识别 KiCad 原理图/PCB 工程路径与当前图纸名，并优化工具栏找框策略",
                "homepage": "https://github.com/Zegel-None/Anything-To-Prompt",
                "category": "cad_3d",
                "match": {
                    "exes": ["kicad.exe", "pcbnew.exe", "eeschema.exe"]
                },
                "capabilities": ["enrich_pick", "ui_profile"],
                "priority": 110,
                "timeoutMs": 600,
                "uiProfile": {
                    "uiaPolicy": "watchdog_only",
                    "preferredLayers": ["static_visual_hit_map", "msaa_hit_map"],
                    "ignoreBackground": true
                },
                "runtime": {
                    "kind": "declarative",
                    "rules": [
                        { "field": "cmdline", "op": "file_ext", "arg1": ".kicad_pcb", "outputKey": "kicad_pcb_path", "isPath": true, "setBadge": true },
                        { "field": "cmdline", "op": "file_ext", "arg1": ".kicad_sch", "outputKey": "kicad_sch_path", "isPath": true, "setBadge": true },
                        { "field": "cmdline", "op": "file_ext", "arg1": ".kicad_pro", "outputKey": "kicad_project_path", "isPath": true, "setBadge": false },
                        { "field": "window_title", "op": "split", "arg1": "—", "index": 0, "outputKey": "kicad_document", "isPath": false, "setBadge": true }
                    ]
                }
            })),
            installed: false,
        },
        RegistryPluginEntry {
            id: "jetbrains-workspace-pro".to_string(),
            name: "JetBrains 全家桶深度工作区追踪".to_string(),
            version: "1.0.0".to_string(),
            author: "Anything-to-Prompt Community".to_string(),
            description: "适配 IntelliJ IDEA / PyCharm / CLion / GoLand / Rider / WebStorm / RustRover，精准提取工程名与当前编辑文件".to_string(),
            category: "dev".to_string(),
            exes: vec![
                "idea64.exe".to_string(),
                "pycharm64.exe".to_string(),
                "clion64.exe".to_string(),
                "goland64.exe".to_string(),
                "rider64.exe".to_string(),
                "webstorm64.exe".to_string(),
                "rustrover64.exe".to_string(),
            ],
            runtime_kind: "declarative".to_string(),
            download_url: "https://raw.githubusercontent.com/Zegel-None/Anything-To-Prompt/main/adapter-registry/plugins/jetbrains-workspace-pro/adapter.json".to_string(),
            homepage: "https://github.com/Zegel-None/Anything-To-Prompt".to_string(),
            inline_manifest: Some(serde_json::json!({
                "schemaVersion": 1,
                "id": "jetbrains-workspace-pro",
                "name": "JetBrains 全家桶深度工作区追踪",
                "version": "1.0.0",
                "author": "Anything-to-Prompt Community",
                "description": "适配 IntelliJ / PyCharm / CLion / GoLand / Rider / RustRover，提取工程名与当前文件",
                "homepage": "https://github.com/Zegel-None/Anything-To-Prompt",
                "category": "dev",
                "match": {
                    "exes": ["idea64.exe", "pycharm64.exe", "clion64.exe", "goland64.exe", "rider64.exe", "webstorm64.exe", "rustrover64.exe"]
                },
                "capabilities": ["enrich_pick"],
                "priority": 110,
                "timeoutMs": 600,
                "runtime": {
                    "kind": "declarative",
                    "rules": [
                        { "field": "window_title", "op": "split", "arg1": " – ", "index": 0, "outputKey": "jetbrains_project", "isPath": false, "setBadge": false },
                        { "field": "window_title", "op": "split", "arg1": " – ", "index": 1, "outputKey": "jetbrains_active_file", "isPath": false, "setBadge": true },
                        { "field": "window_title", "op": "between", "arg1": "[", "arg2": "]", "outputKey": "jetbrains_workspace_hint", "isPath": true, "setBadge": false }
                    ]
                }
            })),
            installed: false,
        },
        RegistryPluginEntry {
            id: "blender-scene-deep".to_string(),
            name: "Blender 3D 场景与工程追踪".to_string(),
            version: "1.0.0".to_string(),
            author: "Anything-to-Prompt Community".to_string(),
            description: "追踪 Blender 当前 `.blend` 工程路径、场景标题，并自动配置 OpenGL 视口防误触与静态工具栏找框".to_string(),
            category: "cad_3d".to_string(),
            exes: vec!["blender.exe".to_string()],
            runtime_kind: "declarative".to_string(),
            download_url: "https://raw.githubusercontent.com/Zegel-None/Anything-To-Prompt/main/adapter-registry/plugins/blender-scene-deep/adapter.json".to_string(),
            homepage: "https://github.com/Zegel-None/Anything-To-Prompt".to_string(),
            inline_manifest: Some(serde_json::json!({
                "schemaVersion": 1,
                "id": "blender-scene-deep",
                "name": "Blender 3D 场景与工程追踪",
                "version": "1.0.0",
                "author": "Anything-to-Prompt Community",
                "description": "追踪 Blender 当前 .blend 文件路径并优化视口与侧栏工具箱识别",
                "homepage": "https://github.com/Zegel-None/Anything-To-Prompt",
                "category": "cad_3d",
                "match": {
                    "exes": ["blender.exe"]
                },
                "capabilities": ["enrich_pick", "ui_profile"],
                "priority": 115,
                "timeoutMs": 600,
                "uiProfile": {
                    "uiaPolicy": "watchdog_only",
                    "preferredLayers": ["static_visual_hit_map", "msaa_hit_map"],
                    "ignoreBackground": true,
                    "ignoreCanvasClasses": ["ghost_windowclass"]
                },
                "runtime": {
                    "kind": "declarative",
                    "rules": [
                        { "field": "cmdline", "op": "file_ext", "arg1": ".blend", "outputKey": "blend_file_path", "isPath": true, "setBadge": true },
                        { "field": "window_title", "op": "between", "arg1": "[", "arg2": "]", "outputKey": "blend_title_path", "isPath": true, "setBadge": true }
                    ]
                }
            })),
            installed: false,
        },
        RegistryPluginEntry {
            id: "rhino-3d-cad".to_string(),
            name: "Rhino 3D 曲面建模深度追踪（COM 脚本示例）".to_string(),
            version: "1.0.0".to_string(),
            author: "Anything-to-Prompt Community".to_string(),
            description: "通过中权限 PowerShell COM 脚本读取 Rhino 活动 `.3dm` 文档路径、选中物体数与当前视图名（需用户信任启用）".to_string(),
            category: "cad_3d".to_string(),
            exes: vec!["rhino.exe".to_string()],
            runtime_kind: "command".to_string(),
            download_url: "https://raw.githubusercontent.com/Zegel-None/Anything-To-Prompt/main/adapter-registry/plugins/rhino-3d-cad/adapter.json".to_string(),
            homepage: "https://github.com/Zegel-None/Anything-To-Prompt".to_string(),
            inline_manifest: Some(serde_json::json!({
                "schemaVersion": 1,
                "id": "rhino-3d-cad",
                "name": "Rhino 3D 曲面建模深度追踪",
                "version": "1.0.0",
                "author": "Anything-to-Prompt Community",
                "description": "通过中权限 COM 脚本读取 Rhino 活动 3dm 文档名、路径与选中物体摘要",
                "homepage": "https://github.com/Zegel-None/Anything-To-Prompt",
                "category": "cad_3d",
                "match": {
                    "exes": ["rhino.exe"]
                },
                "capabilities": ["enrich_pick", "ui_profile"],
                "priority": 120,
                "timeoutMs": 1500,
                "integrity": "medium",
                "uiProfile": {
                    "uiaPolicy": "watchdog_only",
                    "preferredLayers": ["static_visual_hit_map", "msaa_hit_map"],
                    "ignoreBackground": true
                },
                "runtime": {
                    "kind": "command",
                    "command": "powershell.exe",
                    "args": ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "adapter.ps1"],
                    "inlineFilename": "adapter.ps1",
                    "inlineScript": "$raw = [Console]::In.ReadToEnd()\n$req = if ($raw) { $raw | ConvertFrom-Json } else { $null }\n$lines = @()\n$paths = @()\n$badge = $null\ntry {\n  $rh = [Runtime.InteropServices.Marshal]::GetActiveObject('Rhino.Application')\n  $rs = $rh.GetScriptObject()\n  if ($null -ne $rs) {\n    $docName = [string]$rs.DocumentName()\n    $docPath = [string]$rs.DocumentPath()\n    if ($docName) { $badge = $docName; $lines += \"rhino_doc: $docName\" }\n    if ($docPath -and $docName) { $full = Join-Path $docPath $docName; if (Test-Path $full) { $paths += $full } }\n    $sel = $rs.SelectedObjects()\n    if ($null -ne $sel) { $lines += \"rhino_selected_count: $($sel.Count)\" }\n  }\n} catch {\n  if ($req -and $req.windowTitle) { $lines += \"rhino_window: $($req.windowTitle)\" }\n}\n@{ ok = $true; badge = $badge; lines = $lines; paths = $paths } | ConvertTo-Json -Compress\n",
                    "protocol": "json_stdio_v1"
                }
            })),
            installed: false,
        },
    ]
}

pub fn fetch_registry(custom_url: Option<&str>) -> RegistryFetchResult {
    let installed_ids: HashSet<String> = {
        let Ok(mut st) = store().lock() else {
            return RegistryFetchResult {
                ok: false,
                source: "curated_fallback".to_string(),
                registry_url: DEFAULT_REGISTRY_URL.to_string(),
                warning: Some("插件状态锁不可用".to_string()),
                entries: Vec::new(),
            };
        };
        ensure_loaded(&mut st);
        if let Some(u) = custom_url.map(str::trim).filter(|s| !s.is_empty()) {
            st.state.registry_url = Some(u.to_string());
            let _ = save_state_file(&st.state);
        }
        st.externals.keys().cloned().collect()
    };

    let target_url = custom_url
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| {
            store()
                .lock()
                .ok()
                .and_then(|st| st.state.registry_url.clone())
        })
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_REGISTRY_URL.to_string());

    let mut remote_err: Option<String> = None;
    if let Ok(norm_url) = normalize_plugin_download_url(&target_url) {
        match read_http_bytes_capped(&norm_url, 6) {
            Ok(bytes) => {
                if let Ok(text) = String::from_utf8(bytes) {
                    #[derive(Deserialize)]
                    struct RawReg {
                        #[serde(default)]
                        plugins: Vec<RegistryPluginEntry>,
                    }
                    let parsed_list = serde_json::from_str::<RawReg>(&text)
                        .map(|r| r.plugins)
                        .or_else(|_| serde_json::from_str::<Vec<RegistryPluginEntry>>(&text));
                    if let Ok(mut entries) = parsed_list {
                        for e in &mut entries {
                            e.installed = installed_ids.contains(&e.id.to_ascii_lowercase());
                        }
                        return RegistryFetchResult {
                            ok: true,
                            source: "remote".to_string(),
                            registry_url: norm_url,
                            warning: None,
                            entries,
                        };
                    } else {
                        remote_err = Some("远程市场索引 JSON 解析失败，已回退内置精选插件库".to_string());
                    }
                }
            }
            Err(_) => {
                remote_err = Some("暂未连接远程市场索引，已显示内置精选社区插件（支持直接粘贴任意 GitHub/HTTPS 链接下载）".to_string());
            }
        }
    } else {
        remote_err = Some("市场索引地址无效，已显示内置精选社区插件".to_string());
    }

    let mut fallback = curated_starter_registry_entries();
    for e in &mut fallback {
        e.installed = installed_ids.contains(&e.id.to_ascii_lowercase());
    }
    RegistryFetchResult {
        ok: true,
        source: "curated_fallback".to_string(),
        registry_url: target_url,
        warning: remote_err,
        entries: fallback,
    }
}

// ============================================================================
// Runtime Execution Engine (Declarative + Command `json_stdio_v1`)
// ============================================================================

fn plugin_matches_context(m: &AdapterPluginManifest, ctx: &AdapterContext) -> bool {
    let exe = profile::normalize_exe_key(&ctx.proc_meta.exe_name);
    let cls = ctx.window_class.to_ascii_lowercase();
    let title = ctx.window_title.to_ascii_lowercase();

    if !m.match_rule.exes.is_empty() && !m.match_rule.exes.iter().any(|e| e == &exe) {
        return false;
    }
    if !m.match_rule.window_classes.is_empty()
        && !m
            .match_rule
            .window_classes
            .iter()
            .any(|c| cls.contains(c))
    {
        return false;
    }
    if !m.match_rule.title_contains.is_empty()
        && !m
            .match_rule
            .title_contains
            .iter()
            .any(|t| title.contains(t))
    {
        return false;
    }
    true
}

fn eval_extract_rule(rule: &DeclarativeExtractRule, ctx: &AdapterContext) -> Option<String> {
    let src: &str = match rule.field.as_str() {
        "window_title" => &ctx.window_title,
        "cmdline" => &ctx.proc_meta.cmdline,
        "exe_path" => &ctx.proc_meta.exe_path,
        "window_class" => &ctx.window_class,
        "element_name" => ctx
            .primary_element
            .as_ref()
            .map(|e| e.name.as_str())
            .unwrap_or(""),
        "element_value" => ctx
            .primary_element
            .as_ref()
            .map(|e| e.value.as_str())
            .unwrap_or(""),
        _ => return None,
    };
    let trimmed = src.trim();
    if trimmed.is_empty() {
        return None;
    }

    match rule.op.as_str() {
        "whole" => Some(trimmed.to_string()),
        "contains" => {
            if !rule.arg1.is_empty()
                && trimmed
                    .to_ascii_lowercase()
                    .contains(&rule.arg1.to_ascii_lowercase())
            {
                Some(trimmed.to_string())
            } else {
                None
            }
        }
        "prefix" => trimmed
            .strip_prefix(&rule.arg1)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        "suffix" => trimmed
            .strip_suffix(&rule.arg1)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        "between" => {
            if rule.arg1.is_empty() || rule.arg2.is_empty() {
                return None;
            }
            let start = trimmed.find(&rule.arg1)? + rule.arg1.len();
            let rest = &trimmed[start..];
            let end = rest.find(&rule.arg2)?;
            let inner = rest[..end].trim();
            if inner.is_empty() {
                None
            } else {
                Some(inner.to_string())
            }
        }
        "split" => {
            if rule.arg1.is_empty() {
                return None;
            }
            let parts: Vec<&str> = trimmed.split(&rule.arg1).map(|s| s.trim()).collect();
            if parts.is_empty() {
                return None;
            }
            let idx = if rule.index < 0 {
                let back = (-rule.index) as usize;
                parts.len().checked_sub(back)?
            } else {
                rule.index as usize
            };
            parts
                .get(idx)
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty())
        }
        "file_ext" => {
            if rule.arg1.is_empty() {
                return None;
            }
            extract_path_by_ext(trimmed, &rule.arg1)
        }
        _ => None,
    }
}

fn extract_path_by_ext(cmdline: &str, ext: &str) -> Option<String> {
    let ext_l = ext.to_ascii_lowercase();
    for raw_tok in split_cmdline_tokens(cmdline) {
        let tok = raw_tok.trim_matches('"').trim();
        if tok.to_ascii_lowercase().ends_with(&ext_l) && tok.len() > ext_l.len() + 1 {
            return Some(tok.to_string());
        }
    }
    None
}

fn split_cmdline_tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    for ch in s.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ' ' | '\t' if !in_quotes => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(ch),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn run_declarative_plugin(
    m: &AdapterPluginManifest,
    rules: &[DeclarativeExtractRule],
    badge_template: Option<&str>,
    ctx: &AdapterContext,
) -> Option<AdapterOutput> {
    let mut out = AdapterOutput::new(format!("plugin:{}", m.id));
    for r in rules {
        if let Some(val) = eval_extract_rule(r, ctx) {
            out.lines.push(format!("{}: {}", r.output_key, val));
            if r.is_path && Path::new(&val).exists() {
                out.paths.push(val.clone());
            }
            if r.set_badge && out.badge.is_none() {
                let b = if r.is_path {
                    Path::new(&val)
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or(val)
                } else {
                    val
                };
                out.badge = Some(b);
            }
        }
    }
    if out.badge.is_none() {
        if let Some(tpl) = badge_template.filter(|s| !s.trim().is_empty()) {
            out.badge = Some(tpl.to_string());
        }
    }
    if out.lines.is_empty() && out.paths.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn build_stdio_request(hook: &str, ctx: &AdapterContext) -> StdioPluginRequest {
    StdioPluginRequest {
        protocol: ADAPTER_PROTOCOL_V1.to_string(),
        hook: hook.to_string(),
        hwnd: ctx.hwnd,
        pid: ctx.proc_meta.pid,
        exe_name: ctx.proc_meta.exe_name.clone(),
        exe_path: ctx.proc_meta.exe_path.clone(),
        cmdline: ctx.proc_meta.cmdline.clone(),
        window_title: ctx.window_title.clone(),
        window_class: ctx.window_class.clone(),
        tap_point: ctx.tap_point.map(|(x, y)| [x, y]),
        primary_element: ctx
            .primary_element
            .as_ref()
            .and_then(|e| serde_json::to_value(e).ok()),
    }
}

fn execute_command_plugin(
    ext: &InstalledExternalPlugin,
    command: &str,
    args: &[String],
    req: &StdioPluginRequest,
    ctx: Option<&AdapterContext>,
    timeout_ms: u64,
) -> Result<StdioPluginResponse> {
    let eff_timeout = timeout_ms.clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS);
    let req_json = serde_json::to_string(req)?;

    // Special fast path: PowerShell script plugin requesting `"integrity": "medium"` when host is
    // elevated -> execute through the existing Medium-Integrity Broker so COM/ROT access works!
    let cmd_l = command.to_ascii_lowercase();
    if ext.manifest.integrity.eq_ignore_ascii_case("medium")
        && (cmd_l == "powershell" || cmd_l == "powershell.exe" || cmd_l == "pwsh" || cmd_l == "pwsh.exe")
    {
        if let Some(c) = ctx {
            if let Some(script_arg) = args.last().filter(|a| a.to_ascii_lowercase().ends_with(".ps1")) {
                let script_path = ext.dir.join(script_arg);
                if let Ok(script_body) = fs::read_to_string(&script_path) {
                    let req_b64 = base64::engine::general_purpose::STANDARD.encode(req_json.as_bytes());
                    let wrapped = format!(
                        "$reqJson = [System.Text.Encoding]::UTF8.GetString([System.Convert]::FromBase64String('{req_b64}')); \
                         $oldIn = [Console]::In; \
                         [Console]::SetIn((New-Object System.IO.StringReader($reqJson))); \
                         try {{ {script_body} }} finally {{ [Console]::SetIn($oldIn) }}"
                    );
                    let resp = c.broker.call(BrokerRequest::RunPs51 {
                        script: wrapped,
                        timeout_ms: eff_timeout,
                    });
                    if !resp.ok {
                        return Err(anyhow!(
                            "medium broker plugin error: {}",
                            resp.error.unwrap_or_default()
                        ));
                    }
                    let parsed = serde_json::from_str::<StdioPluginResponse>(resp.text.trim())
                        .map_err(|e| anyhow!("插件输出 JSON 无效: {e}"))?;
                    return Ok(parsed);
                }
            }
        }
    }

    let mut child = Command::new(command)
        .args(args)
        .current_dir(&ext.dir)
        .creation_flags(CREATE_NO_WINDOW.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("启动插件命令失败: {command}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(req_json.as_bytes());
        let _ = stdin.write_all(b"\n");
    }

    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let out_t = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(ref mut r) = out_pipe {
            let _ = r
                .take(MAX_PLUGIN_STDOUT_BYTES as u64)
                .read_to_end(&mut buf);
        }
        buf
    });
    let err_t = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(ref mut r) = err_pipe {
            let _ = r.take(8192).read_to_end(&mut buf);
        }
        buf
    });

    let deadline = Instant::now() + Duration::from_millis(eff_timeout);
    loop {
        match child.try_wait()? {
            Some(status) => {
                let out_bytes = out_t.join().unwrap_or_default();
                let err_bytes = err_t.join().unwrap_or_default();
                let stdout = String::from_utf8_lossy(&out_bytes).trim().to_string();
                if stdout.is_empty() {
                    let stderr = String::from_utf8_lossy(&err_bytes).trim().to_string();
                    if status.success() {
                        return Ok(StdioPluginResponse::default());
                    }
                    return Err(anyhow!("插件退出码 {:?}: {stderr}", status.code()));
                }
                let parsed = serde_json::from_str::<StdioPluginResponse>(&stdout)
                    .map_err(|e| anyhow!("插件 stdout JSON 解析失败: {e}"))?;
                return Ok(parsed);
            }
            None => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = out_t.join();
                    let _ = err_t.join();
                    return Err(anyhow!("插件执行超时 ({eff_timeout}ms)"));
                }
                std::thread::sleep(Duration::from_millis(15));
            }
        }
    }
}

/// Runs all matching, enabled external plugins for `enrich_pick` within `budget_ms`.
pub fn run_external_plugins_for_context(ctx: &AdapterContext, budget_ms: u64) -> Vec<AdapterOutput> {
    let candidates: Vec<InstalledExternalPlugin> = {
        let Ok(mut st) = store().lock() else {
            return Vec::new();
        };
        ensure_loaded(&mut st);
        let mut list: Vec<InstalledExternalPlugin> = st
            .externals
            .values()
            .filter(|ext| {
                is_external_enabled(&st, ext) && plugin_matches_context(&ext.manifest, ctx)
            })
            .cloned()
            .collect();
        list.sort_by(|a, b| b.manifest.priority.cmp(&a.manifest.priority));
        list
    };

    if candidates.is_empty() {
        return Vec::new();
    }

    let started = Instant::now();
    let deadline = started + Duration::from_millis(budget_ms.max(300));
    let mut outputs = Vec::new();

    for ext in candidates {
        if Instant::now() >= deadline {
            break;
        }
        let rem_ms = deadline
            .saturating_duration_since(Instant::now())
            .as_millis() as u64;
        if rem_ms < 80 {
            break;
        }
        match &ext.manifest.runtime {
            PluginRuntime::Declarative {
                rules,
                badge_template,
                ..
            } => {
                if let Some(out) =
                    run_declarative_plugin(&ext.manifest, rules, badge_template.as_deref(), ctx)
                {
                    outputs.push(out);
                }
            }
            PluginRuntime::Command { command, args, .. } => {
                let req = build_stdio_request("enrich_pick", ctx);
                let run_budget = ext.manifest.timeout_ms.min(rem_ms);
                match execute_command_plugin(&ext, command, args, &req, Some(ctx), run_budget) {
                    Ok(resp) if resp.ok && (!resp.lines.is_empty() || !resp.paths.is_empty()) => {
                        let mut out = AdapterOutput::new(format!("plugin:{}", ext.manifest.id));
                        out.badge = resp.badge.filter(|s| !s.trim().is_empty());
                        out.lines = resp.lines;
                        out.paths = resp.paths;
                        outputs.push(out);
                    }
                    Ok(_) => {}
                    Err(e) => {
                        log::warn!("[ADAPTER-PLUGIN] '{}' failed: {e}", ext.manifest.id);
                    }
                }
            }
            PluginRuntime::Builtin => {}
        }
    }
    outputs
}

/// Checks if any enabled external plugin declares capability `"tap_override"` for `ctx` and returns
/// an authoritative capsule override (allowing community plugins to provide SolidWorks/Office-level
/// single-click CAD/3D/DAW entity picking without modifying Rust!).
pub fn try_run_external_tap_override(ctx: &AdapterContext) -> Option<ExternalTapOverrideResult> {
    let candidates: Vec<InstalledExternalPlugin> = {
        let Ok(mut st) = store().lock() else {
            return None;
        };
        ensure_loaded(&mut st);
        let mut list: Vec<InstalledExternalPlugin> = st
            .externals
            .values()
            .filter(|ext| {
                is_external_enabled(&st, ext)
                    && ext
                        .manifest
                        .capabilities
                        .iter()
                        .any(|c| c.eq_ignore_ascii_case("tap_override"))
                    && plugin_matches_context(&ext.manifest, ctx)
            })
            .cloned()
            .collect();
        list.sort_by(|a, b| b.manifest.priority.cmp(&a.manifest.priority));
        list
    };

    for ext in candidates {
        if let PluginRuntime::Command { command, args, .. } = &ext.manifest.runtime {
            let req = build_stdio_request("tap_override", ctx);
            if let Ok(resp) =
                execute_command_plugin(&ext, command, args, &req, Some(ctx), ext.manifest.timeout_ms)
            {
                if resp.ok && resp.override_capsule && (!resp.lines.is_empty() || resp.title.is_some())
                {
                    return Some(ExternalTapOverrideResult {
                        plugin_id: ext.manifest.id.clone(),
                        title: resp
                            .title
                            .filter(|s| !s.trim().is_empty())
                            .unwrap_or_else(|| ext.manifest.name.clone()),
                        detail: resp
                            .detail
                            .filter(|s| !s.trim().is_empty())
                            .unwrap_or_else(|| format!("插件 · {}", ext.manifest.name)),
                        badge: resp.badge,
                        lines: resp.lines,
                        paths: resp.paths,
                    });
                }
            }
        }
    }
    None
}

/// Dry-run self-test for an installed plugin using a synthetic `AdapterContext`.
pub fn test_plugin_dry_run(id: &str) -> Result<PluginTestReport> {
    let norm = id.trim().to_ascii_lowercase();
    let (builtin_opt, ext_opt) = {
        let Ok(mut st) = store().lock() else {
            return Err(anyhow!("plugin store lock poisoned"));
        };
        ensure_loaded(&mut st);
        (
            st.builtins.iter().find(|b| b.id == norm).cloned(),
            st.externals.get(&norm).cloned(),
        )
    };

    if let Some(b) = builtin_opt {
        return Ok(PluginTestReport {
            ok: true,
            plugin_id: b.id,
            runtime_kind: "builtin".to_string(),
            latency_ms: 0,
            badge: Some(b.name),
            lines: vec![format!("内置原生适配器就绪（目标进程: {}）", b.match_rule.exes.join(", "))],
            paths: Vec::new(),
            error: None,
        });
    }

    let Some(ext) = ext_opt else {
        return Err(anyhow!("未找到插件: {id}"));
    };

    let sample_exe = ext
        .manifest
        .match_rule
        .exes
        .first()
        .cloned()
        .unwrap_or_else(|| "sample.exe".to_string());
    let sample_ctx = AdapterContext {
        hwnd: 0,
        window_title: format!("DemoProject – main.demo [C:\\Demo] — {}", ext.manifest.name),
        window_class: ext
            .manifest
            .match_rule
            .window_classes
            .first()
            .cloned()
            .unwrap_or_else(|| "DemoWindowClass".to_string()),
        proc_meta: crate::pick::process::ProcessMetadata {
            pid: 0,
            exe_name: sample_exe.clone(),
            exe_path: format!("C:\\Apps\\{sample_exe}"),
            cmdline: format!("\"{sample_exe}\" \"C:\\Demo\\sample.kicad_pcb\" \"C:\\Demo\\scene.blend\""),
            cwd: None,
            ..Default::default()
        },
        primary_element: None,
        tap_point: Some((640, 480)),
        broker: std::sync::Arc::new(crate::broker::BrokerClient::new()),
    };

    let t0 = Instant::now();
    match &ext.manifest.runtime {
        PluginRuntime::Declarative {
            rules,
            badge_template,
            ..
        } => {
            let out = run_declarative_plugin(
                &ext.manifest,
                rules,
                badge_template.as_deref(),
                &sample_ctx,
            );
            Ok(PluginTestReport {
                ok: true,
                plugin_id: ext.manifest.id,
                runtime_kind: "declarative".to_string(),
                latency_ms: t0.elapsed().as_millis() as u64,
                badge: out.as_ref().and_then(|o| o.badge.clone()),
                lines: out.map(|o| o.lines).unwrap_or_else(|| {
                    vec!["零代码规则校验通过（匹配真实窗口时将自动提取字段）".to_string()]
                }),
                paths: Vec::new(),
                error: None,
            })
        }
        PluginRuntime::Command { command, args, .. } => {
            let req = build_stdio_request("enrich_pick", &sample_ctx);
            match execute_command_plugin(
                &ext,
                command,
                args,
                &req,
                Some(&sample_ctx),
                ext.manifest.timeout_ms,
            ) {
                Ok(resp) => Ok(PluginTestReport {
                    ok: resp.ok,
                    plugin_id: ext.manifest.id,
                    runtime_kind: "command".to_string(),
                    latency_ms: t0.elapsed().as_millis() as u64,
                    badge: resp.badge,
                    lines: resp.lines,
                    paths: resp.paths,
                    error: resp.error,
                }),
                Err(e) => Ok(PluginTestReport {
                    ok: false,
                    plugin_id: ext.manifest.id,
                    runtime_kind: "command".to_string(),
                    latency_ms: t0.elapsed().as_millis() as u64,
                    badge: None,
                    lines: Vec::new(),
                    paths: Vec::new(),
                    error: Some(e.to_string()),
                }),
            }
        }
        PluginRuntime::Builtin => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_blob_url_rewrites_to_raw_and_rejects_http() {
        let got = normalize_plugin_download_url(
            "https://github.com/Zegel-None/Anything-To-Prompt/blob/main/adapter-registry/plugins/kicad/adapter.json",
        )
        .expect("github blob rewrite");
        assert_eq!(
            got,
            "https://raw.githubusercontent.com/Zegel-None/Anything-To-Prompt/main/adapter-registry/plugins/kicad/adapter.json"
        );
        assert!(normalize_plugin_download_url("http://evil.example.com/p.json").is_err());
        assert!(normalize_plugin_download_url("http://127.0.0.1:8080/p.json").is_ok());
    }

    #[test]
    fn manifest_validation_rejects_bad_id_and_path_traversal() {
        assert!(validate_plugin_id("../escape").is_err());
        assert!(validate_plugin_id("Bad Upper").is_err());
        assert_eq!(validate_plugin_id("My_Plugin-1").unwrap(), "my_plugin-1");

        let bad_json = r#"{
            "schemaVersion": 1,
            "id": "traversal-test",
            "name": "Bad",
            "match": { "exes": ["demo.exe"] },
            "runtime": {
                "kind": "command",
                "command": "python",
                "args": ["../../evil.py"]
            }
        }"#;
        let m: AdapterPluginManifest = serde_json::from_str(bad_json).unwrap();
        assert!(normalize_and_validate_manifest(m).is_err());
    }

    #[test]
    fn declarative_plugin_extracts_title_and_cmdline_fields() {
        let manifest: AdapterPluginManifest = serde_json::from_str(
            r#"{
                "schemaVersion": 1,
                "id": "kicad-unit",
                "name": "KiCad",
                "match": { "exes": ["pcbnew.exe"] },
                "runtime": {
                    "kind": "declarative",
                    "rules": [
                        { "field": "cmdline", "op": "file_ext", "arg1": ".kicad_pcb", "outputKey": "pcb_path", "isPath": false, "setBadge": true },
                        { "field": "window_title", "op": "split", "arg1": " — ", "index": 0, "outputKey": "board_name", "isPath": false, "setBadge": false }
                    ]
                }
            }"#,
        )
        .unwrap();
        let m = normalize_and_validate_manifest(manifest).unwrap();
        let ctx = AdapterContext {
            hwnd: 0,
            window_title: "PowerBoard.kicad_pcb — PCB Editor".to_string(),
            window_class: "wxWindowNR".to_string(),
            proc_meta: crate::pick::process::ProcessMetadata {
                pid: 42,
                exe_name: "pcbnew.exe".to_string(),
                exe_path: r"C:\KiCad\bin\pcbnew.exe".to_string(),
                cmdline: r#""C:\KiCad\bin\pcbnew.exe" "D:\HW\PowerBoard.kicad_pcb""#.to_string(),
                cwd: None,
                ..Default::default()
            },
            primary_element: None,
            tap_point: None,
            broker: std::sync::Arc::new(crate::broker::BrokerClient::new()),
        };
        assert!(plugin_matches_context(&m, &ctx));
        if let PluginRuntime::Declarative {
            ref rules,
            ref badge_template,
            ..
        } = m.runtime
        {
            let out = run_declarative_plugin(&m, rules, badge_template.as_deref(), &ctx)
                .expect("declarative output");
            assert_eq!(out.adapter, "plugin:kicad-unit");
            assert_eq!(out.badge.as_deref(), Some(r"D:\HW\PowerBoard.kicad_pcb"));
            assert!(out
                .lines
                .iter()
                .any(|l| l == r"pcb_path: D:\HW\PowerBoard.kicad_pcb"));
            assert!(out.lines.iter().any(|l| l == "board_name: PowerBoard.kicad_pcb"));
        } else {
            panic!("expected declarative runtime");
        }
    }

    #[test]
    fn command_plugin_stdio_roundtrip_works_end_to_end() {
        let tmp = std::env::temp_dir().join(format!("atp-cmd-plugin-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&tmp);
        // Write a tiny .cmd script that prints valid JSON cleanly without cmd quote-stripping
        let json_out = tmp.join("out.json");
        fs::write(
            &json_out,
            r#"{"ok":true,"badge":"demo-badge","lines":["hello: world"]}"#,
        )
        .unwrap();
        let manifest = normalize_and_validate_manifest(
            serde_json::from_str::<AdapterPluginManifest>(
                r#"{
                    "schemaVersion": 1,
                    "id": "echo-test",
                    "name": "Echo Test",
                    "match": { "exes": ["demo.exe"] },
                    "timeoutMs": 1500,
                    "runtime": {
                        "kind": "command",
                        "command": "cmd.exe",
                        "args": ["/c", "type out.json"]
                    }
                }"#,
            )
            .unwrap(),
        )
        .unwrap();
        let ext = InstalledExternalPlugin {
            manifest: manifest.clone(),
            dir: tmp.clone(),
            source_url: None,
            installed_at_unix: 0,
        };
        let req = StdioPluginRequest {
            protocol: ADAPTER_PROTOCOL_V1.to_string(),
            hook: "enrich_pick".to_string(),
            hwnd: 0,
            pid: 1,
            exe_name: "demo.exe".to_string(),
            exe_path: String::new(),
            cmdline: String::new(),
            window_title: "Demo".to_string(),
            window_class: String::new(),
            tap_point: None,
            primary_element: None,
        };
        if let PluginRuntime::Command {
            ref command,
            ref args,
            ..
        } = manifest.runtime
        {
            let resp = execute_command_plugin(&ext, command, args, &req, None, 1500)
                .expect("command plugin roundtrip");
            assert!(resp.ok);
            assert_eq!(resp.badge.as_deref(), Some("demo-badge"));
            assert_eq!(resp.lines, vec!["hello: world".to_string()]);
        }
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn curated_starter_manifests_all_validate_cleanly() {
        for entry in curated_starter_registry_entries() {
            let raw = entry
                .inline_manifest
                .expect("curated entry must carry inline_manifest");
            let m: AdapterPluginManifest =
                serde_json::from_value(raw).expect("curated manifest json valid");
            let norm = normalize_and_validate_manifest(m).expect("curated manifest validates");
            assert_eq!(norm.id, entry.id);
        }
    }
}
