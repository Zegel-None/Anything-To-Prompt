use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

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
    pub exclude_from_capture: bool,
    pub accept_injected_input: bool,
    pub app_source_map: HashMap<String, String>,
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
            exclude_from_capture: true,
            accept_injected_input: false,
            app_source_map: HashMap::new(),
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

    pub fn logs_dir() -> PathBuf {
        Self::local_data_dir().join("logs")
    }

    pub fn captures_dir() -> PathBuf {
        Self::local_data_dir().join("captures")
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
    }
}
