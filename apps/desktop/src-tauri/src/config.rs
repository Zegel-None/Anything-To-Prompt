use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockEdge {
    #[default]
    None,
    Top,
    Bottom,
    Left,
    Right,
}

impl DockEdge {
    pub fn as_str(self) -> &'static str {
        match self {
            DockEdge::None => "none",
            DockEdge::Top => "top",
            DockEdge::Bottom => "bottom",
            DockEdge::Left => "left",
            DockEdge::Right => "right",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "top" => DockEdge::Top,
            "bottom" => DockEdge::Bottom,
            "left" => DockEdge::Left,
            "right" => DockEdge::Right,
            _ => DockEdge::None,
        }
    }

    pub fn is_docked(self) -> bool {
        !matches!(self, DockEdge::None)
    }

    pub fn is_side(self) -> bool {
        matches!(self, DockEdge::Left | DockEdge::Right)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputTarget {
    /// Inject capsule into Anything-to-Prompt / DSH floating composer (default)
    #[default]
    Dsh,
    /// Inject into DSH floating composer AND copy structured capsule to clipboard
    DshAndClipboard,
    /// Copy structured capsule to clipboard only (ideal for Codex / Claude Code / Antigravity)
    ClipboardOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomAcpHarness {
    pub id: String,
    pub label: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub accent: String,
    #[serde(default)]
    pub default_cwd: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisionHardwarePreset {
    /// Tier 0: Lightweight office laptop (0 VRAM; Cloud VLM + optional CPU MobileSAM ONNX)
    #[default]
    LiteOffice,
    /// Tier 1: Balanced all-rounder (6-8 GB VRAM; SAM 2.1 Tiny + Florence-2-base / Ollama 7B)
    BalancedUniversal,
    /// Tier 2: High-end gaming / AI workstation (>= 12 GB VRAM; SAM 2.1 Small + Florence-2-large / Qwen2.5-VL)
    ProAiWorkstation,
    /// Tier 3: Custom mix of segmentation engine x VLM backend
    CustomHybrid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SegmentationBackend {
    #[default]
    Disabled,
    OnnxMobileSamCpu,
    GpuSam2Tiny,
    GpuSam2Small,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VlmBackend {
    #[default]
    Disabled,
    CloudOpenAiCompat,
    CloudGemini,
    LocalOllama,
    SidecarFlorence2,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VisionConfig {
    pub preset: VisionHardwarePreset,
    pub segmentation_backend: SegmentationBackend,
    pub vlm_backend: VlmBackend,
    pub auto_photo_matting: bool,
    pub rebrush_vlm_audit: bool,
    pub rebrush_pass2_threshold: u8,
    pub rebrush_pass3_threshold: u8,
    pub idle_unload_seconds: u64,
    pub cloud_provider_hint: String,
    pub cloud_base_url: String,
    pub cloud_model: String,
    pub cloud_api_key: String,
    pub ollama_base_url: String,
    pub ollama_model: String,
    pub sidecar_python_path: Option<String>,
    pub sidecar_port: u16,
}

impl Default for VisionConfig {
    fn default() -> Self {
        Self {
            preset: VisionHardwarePreset::LiteOffice,
            segmentation_backend: SegmentationBackend::Disabled,
            vlm_backend: VlmBackend::Disabled,
            auto_photo_matting: true,
            rebrush_vlm_audit: true,
            rebrush_pass2_threshold: 2,
            rebrush_pass3_threshold: 3,
            idle_unload_seconds: 180,
            cloud_provider_hint: "openai".to_string(),
            cloud_base_url: "https://api.openai.com/v1".to_string(),
            cloud_model: "gpt-4o-mini".to_string(),
            cloud_api_key: String::new(),
            ollama_base_url: "http://127.0.0.1:11434".to_string(),
            ollama_model: "qwen2.5vl:7b".to_string(),
            sidecar_python_path: None,
            sidecar_port: 18765,
        }
    }
}

impl VisionConfig {
    pub fn apply_preset(&mut self, preset: VisionHardwarePreset) {
        self.preset = preset;
        match preset {
            VisionHardwarePreset::LiteOffice => {
                self.segmentation_backend = SegmentationBackend::OnnxMobileSamCpu;
                self.vlm_backend = if !self.cloud_api_key.trim().is_empty() {
                    if self.cloud_provider_hint == "gemini" {
                        VlmBackend::CloudGemini
                    } else {
                        VlmBackend::CloudOpenAiCompat
                    }
                } else {
                    VlmBackend::Disabled
                };
            }
            VisionHardwarePreset::BalancedUniversal => {
                self.segmentation_backend = SegmentationBackend::GpuSam2Tiny;
                self.vlm_backend = VlmBackend::SidecarFlorence2;
            }
            VisionHardwarePreset::ProAiWorkstation => {
                self.segmentation_backend = SegmentationBackend::GpuSam2Small;
                self.vlm_backend = VlmBackend::LocalOllama;
            }
            VisionHardwarePreset::CustomHybrid => {}
        }
    }

    pub fn clamp(&mut self) {
        self.rebrush_pass2_threshold = self.rebrush_pass2_threshold.clamp(2, 5);
        self.rebrush_pass3_threshold = self
            .rebrush_pass3_threshold
            .max(self.rebrush_pass2_threshold + 1)
            .clamp(3, 7);
        self.idle_unload_seconds = self.idle_unload_seconds.clamp(30, 3600);
        if self.sidecar_port < 1024 {
            self.sidecar_port = 18765;
        }
        if self.ollama_base_url.trim().is_empty() {
            self.ollama_base_url = "http://127.0.0.1:11434".to_string();
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppConfig {
    pub preferred_port: u16,
    pub dsh_url: Option<String>,
    pub prefer_admin: bool,
    pub autostart: bool,
    pub bar_width: i32,
    pub double_tap_ms: u64,
    pub ctrl_hold_ms: u64,
    pub capture_keep_messages: usize,
    pub capture_orphan_ttl_minutes: u64,
    /// Opt-in privacy flag: when true, applies WDA_EXCLUDEFROMCAPTURE to Bar & Overlay.
    /// Defaults to `false` so remote desktop / virtual display / screen-streaming users
    /// (GameViewer, Parsec, Moonlight, ToDesk, RustDesk, OBS) can see the Bar & selection overlay!
    pub hide_from_screen_capture: bool,
    /// Play a gentle ~1.5s electric-blue cursor breathing pulse when the app starts ready.
    pub startup_breath_pulse: bool,
    /// Show Windows notification-area tray icon (defaults to `false` in v0.3+: all settings live inside the bar plugin settings UI).
    pub show_tray_icon: bool,
    /// Register lightweight sync/sidecar plugins for Antigravity, Claude Code, and Codex and stream live workspace conversations.
    pub sync_external_plugins: bool,
    pub accept_injected_input: bool,
    pub output_target: OutputTarget,
    pub active_harness: String,
    pub search_roots: Vec<String>,
    pub app_source_map: HashMap<String, String>,
    /// User-registered custom ACP harnesses persisted across restarts.
    #[serde(default)]
    pub custom_acp_harnesses: Vec<CustomAcpHarness>,
    /// Persisted QQ-style screen edge dock (`none` | `top` | `bottom` | `left` | `right`).
    #[serde(default)]
    pub dock_edge: DockEdge,
    /// Independent vertical side-panel layout flag (works both free-floating and docked).
    #[serde(default)]
    pub vertical_mode: bool,
    /// User-resized width (DIP) of the vertical side panel (360..=960, independent of horizontal `bar_width`).
    #[serde(default = "default_side_width")]
    pub side_width: i32,
    /// Multi-tier visual audit & natural-photo semantic matting engine settings.
    #[serde(default)]
    pub vision: VisionConfig,
}

fn default_side_width() -> i32 {
    420
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            preferred_port: 3080,
            dsh_url: None,
            prefer_admin: true,
            autostart: false,
            bar_width: 760,
            double_tap_ms: 400,
            ctrl_hold_ms: 220,
            capture_keep_messages: 20,
            capture_orphan_ttl_minutes: 60,
            hide_from_screen_capture: false,
            startup_breath_pulse: true,
            show_tray_icon: false,
            sync_external_plugins: true,
            accept_injected_input: false,
            output_target: OutputTarget::Dsh,
            active_harness: "dsh".to_string(),
            search_roots: Vec::new(),
            app_source_map: HashMap::new(),
            custom_acp_harnesses: Vec::new(),
            dock_edge: DockEdge::None,
            vertical_mode: false,
            side_width: 420,
            vision: VisionConfig::default(),
        }
    }
}

impl AppConfig {
    pub fn config_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("AnythingToPrompt")
    }

    pub fn config_path() -> PathBuf {
        Self::config_dir().join("config.json")
    }

    pub fn local_data_dir() -> PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("AnythingToPrompt")
    }

    pub fn vision_env_dir() -> PathBuf {
        Self::local_data_dir().join("vision-env")
    }

    pub fn vision_models_dir() -> PathBuf {
        Self::local_data_dir().join("models")
    }

    pub fn vision_sidecar_dir() -> PathBuf {
        Self::local_data_dir().join("vision-sidecar")
    }

    pub fn logs_dir() -> PathBuf {
        Self::local_data_dir().join("logs")
    }

    pub fn captures_dir() -> PathBuf {
        Self::local_data_dir().join("captures")
    }

    /// Canonical stable runtime binary path shared by all 4 Harness plugins
    /// (`%LOCALAPPDATA%\AnythingToPrompt\bin\anything-to-prompt.exe`).
    pub fn runtime_exe_path() -> PathBuf {
        Self::local_data_dir()
            .join("bin")
            .join("anything-to-prompt.exe")
    }

    pub fn dsh_home_dir() -> PathBuf {
        if let Ok(env_home) = std::env::var("DSH_HOME") {
            let trimmed = env_home.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed);
            }
        }
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".dsh")
    }

    pub fn load_or_create() -> Self {
        let path = Self::config_path();
        if path.exists() {
            if let Ok(raw) = fs::read_to_string(&path) {
                let clean = raw.trim_start_matches('\u{feff}');
                match serde_json::from_str::<AppConfig>(clean) {
                    Ok(mut cfg) => {
                        cfg.clamp();
                        return cfg;
                    }
                    Err(e) => {
                        log::warn!("Invalid config.json ({e}); using defaults in memory without overwriting user file.");
                        return Self::default();
                    }
                }
            }
        }
        let cfg = Self::default();
        let _ = cfg.save();
        cfg
    }

    pub fn save(&self) -> Result<()> {
        let dir = Self::config_dir();
        fs::create_dir_all(&dir).context("failed to create config dir")?;
        let raw = serde_json::to_string_pretty(self).context("failed to serialize config")?;
        fs::write(Self::config_path(), raw).context("failed to write config.json")?;
        Ok(())
    }

    pub fn clamp(&mut self) {
        self.bar_width = self.bar_width.clamp(480, 1200);
        self.double_tap_ms = self.double_tap_ms.clamp(150, 1000);
        self.ctrl_hold_ms = self.ctrl_hold_ms.clamp(150, 600);
        if self.capture_keep_messages == 0 {
            self.capture_keep_messages = 20;
        }
        if self.capture_orphan_ttl_minutes == 0 {
            self.capture_orphan_ttl_minutes = 60;
        }
        self.vision.clamp();
    }
}
