pub mod acp;
pub mod broker;
pub mod captures;
pub mod config;
pub mod cursor;
pub mod dsh;
pub mod elevation;
pub mod everything_ipc;
pub mod harness;
pub mod harness_registry;
pub mod harness_sync;
pub mod hooks;
pub mod mcp_sidecar;
pub mod ntfs_usn;
pub mod pick;
pub mod plugin_sync;
pub mod process_guard;
pub mod search;
pub mod state;
pub mod tray;
pub mod universal;
pub mod win32_util;
pub mod window;

use crate::broker::BrokerClient;
use crate::config::AppConfig;
use crate::dsh::{
    apply_discovery_to_bar, build_post_auth_bar_url, discover_or_start_dsh, is_port_listening,
    DshConnectionState,
};
use crate::hooks::{start_input_hooks, HookEvent, HookSharedConfig};
use crate::state::{
    BarStateMachine, BridgeMode, DevTargetState, StateEvent, VisibilityState, WindowAction,
};
use crate::tray::TrayHandles;
use crate::window::{
    activate_and_focus_bar, apply_bar_layout, hide_bar, hide_overlay, hwnd_of, init_bar_window,
    init_overlay_window, log_window_diagnostics, position_and_show_overlay, show_bar_noactivate,
    ShapeRectDip, DEFAULT_LIP_HEIGHT_DIP,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::Duration;
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use windows::Win32::UI::WindowsAndMessaging::{ChangeWindowMessageFilter, MSGFLT_ADD, WM_COPYDATA};

#[derive(Debug, Default, Clone)]
pub struct CliOptions {
    pub broker_pipe: Option<String>,
    pub hook_event: Option<(String, Option<String>)>,
    pub mcp_sidecar: bool,
    pub antigravity_sidecar: bool,
    pub mcp_harness: Option<String>,
    pub ensure_running: bool,
    pub launched_by: Option<String>,
    pub install_plugins: bool,
    pub uninstall_plugins: bool,
    pub elevated_relaunch: bool,
    pub no_elevate: bool,
    pub dev_state: Option<DevTargetState>,
    pub dev_dump: bool,
    pub dev_allow_capture: bool,
    pub dev_single_ctrl_close: bool,
    pub dev_draft: Option<String>,
    pub dev_pick_tap: Option<(i32, i32)>,
    pub dev_pick_stroke: Option<(i32, i32, i32, i32)>,
    pub dev_pick_lasso: Option<Vec<(i32, i32)>>,
    pub probe_window: Option<String>,
    pub probe_out: Option<String>,
    pub passthrough_args: Vec<String>,
}

fn parse_csv_ints(raw: &str) -> Vec<i32> {
    raw.split(',')
        .filter_map(|s| s.trim().parse::<i32>().ok())
        .collect()
}

impl CliOptions {
    pub fn parse<I, S>(args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let vec: Vec<String> = args.into_iter().map(|s| s.as_ref().to_string()).collect();
        let mut out = CliOptions::default();
        let mut i = 1usize; // skip argv[0]
        while i < vec.len() {
            let arg = vec[i].as_str();
            match arg {
                "--broker" => {
                    if i + 1 < vec.len() {
                        out.broker_pipe = Some(vec[i + 1].clone());
                        i += 1;
                    }
                }
                "--hook-event" => {
                    if i + 1 < vec.len() {
                        let harness = vec[i + 1].clone();
                        let inline = if i + 2 < vec.len() && !vec[i + 2].starts_with("--") {
                            i += 1;
                            Some(vec[i + 1].clone())
                        } else {
                            None
                        };
                        out.hook_event = Some((harness, inline));
                        i += 1;
                    }
                }
                "--mcp-sidecar" => {
                    out.mcp_sidecar = true;
                }
                "--antigravity-sidecar" => {
                    out.antigravity_sidecar = true;
                }
                "--probe-window" => {
                    if i + 1 < vec.len() {
                        out.probe_window = Some(vec[i + 1].clone());
                        i += 1;
                    }
                }
                "--out" => {
                    if i + 1 < vec.len() {
                        out.probe_out = Some(vec[i + 1].clone());
                        i += 1;
                    }
                }
                "--harness" => {
                    if i + 1 < vec.len() {
                        out.mcp_harness = Some(vec[i + 1].clone());
                        i += 1;
                    }
                }
                "--ensure-running" => {
                    out.ensure_running = true;
                    out.passthrough_args.push(vec[i].clone());
                }
                "--launched-by" => {
                    if i + 1 < vec.len() {
                        out.launched_by = Some(vec[i + 1].clone());
                        out.passthrough_args.push(vec[i].clone());
                        out.passthrough_args.push(vec[i + 1].clone());
                        i += 1;
                    }
                }
                "--install-plugins" => {
                    out.install_plugins = true;
                }
                "--uninstall-plugins" => {
                    out.uninstall_plugins = true;
                }
                "--elevated-relaunch" => {
                    out.elevated_relaunch = true;
                }
                "--no-elevate" => {
                    out.no_elevate = true;
                    out.passthrough_args.push(vec[i].clone());
                }
                "--dev-state" => {
                    if i + 1 < vec.len() {
                        out.dev_state = DevTargetState::parse(&vec[i + 1]);
                        out.passthrough_args.push(vec[i].clone());
                        out.passthrough_args.push(vec[i + 1].clone());
                        i += 1;
                    }
                }
                "--dev-dump" => {
                    out.dev_dump = true;
                    out.passthrough_args.push(vec[i].clone());
                }
                "--dev-allow-capture" => {
                    out.dev_allow_capture = true;
                    out.passthrough_args.push(vec[i].clone());
                }
                "--dev-single-ctrl-close" => {
                    out.dev_single_ctrl_close = true;
                    out.passthrough_args.push(vec[i].clone());
                }
                "--dev-draft" => {
                    if i + 1 < vec.len() {
                        let raw = vec[i + 1].clone();
                        out.dev_draft = Some(if raw == "__EMPTY__" {
                            String::new()
                        } else {
                            raw.clone()
                        });
                        out.passthrough_args.push(vec[i].clone());
                        out.passthrough_args.push(raw);
                        i += 1;
                    }
                }
                "--dev-pick-tap" => {
                    if i + 1 < vec.len() {
                        let nums = parse_csv_ints(&vec[i + 1]);
                        if nums.len() == 2 {
                            out.dev_pick_tap = Some((nums[0], nums[1]));
                        }
                        out.passthrough_args.push(vec[i].clone());
                        out.passthrough_args.push(vec[i + 1].clone());
                        i += 1;
                    }
                }
                "--dev-pick-stroke" => {
                    if i + 1 < vec.len() {
                        let nums = parse_csv_ints(&vec[i + 1]);
                        if nums.len() == 4 {
                            out.dev_pick_stroke = Some((nums[0], nums[1], nums[2], nums[3]));
                        }
                        out.passthrough_args.push(vec[i].clone());
                        out.passthrough_args.push(vec[i + 1].clone());
                        i += 1;
                    }
                }
                "--dev-pick-lasso" => {
                    if i + 1 < vec.len() {
                        let nums = parse_csv_ints(&vec[i + 1]);
                        if nums.len() >= 4 && nums.len().is_multiple_of(2) {
                            let pts: Vec<(i32, i32)> =
                                nums.as_chunks::<2>().0.iter().map(|c| (c[0], c[1])).collect();
                            out.dev_pick_lasso = Some(pts);
                        }
                        out.passthrough_args.push(vec[i].clone());
                        out.passthrough_args.push(vec[i + 1].clone());
                        i += 1;
                    }
                }
                _ => {
                    out.passthrough_args.push(vec[i].clone());
                }
            }
            i += 1;
        }
        out
    }

    pub fn is_direct_exe_launch(&self) -> bool {
        self.broker_pipe.is_none()
            && self.hook_event.is_none()
            && !self.mcp_sidecar
            && !self.antigravity_sidecar
            && !self.ensure_running
            && !self.install_plugins
            && !self.uninstall_plugins
            && self.dev_state.is_none()
            && !self.dev_dump
            && !self.dev_single_ctrl_close
            && self.dev_draft.is_none()
            && self.dev_pick_tap.is_none()
            && self.dev_pick_stroke.is_none()
            && self.dev_pick_lasso.is_none()
    }
}

#[derive(Debug, Clone)]
pub struct CachedLayout {
    pub width_dip: i32,
    pub height_dip: i32,
    pub shapes: Vec<ShapeRectDip>,
}

#[derive(Debug, Default)]
pub struct PickCounterState {
    pub composer_count: usize,
    pub in_flight: usize,
    pub last_alloc_at: Option<std::time::Instant>,
}

impl PickCounterState {
    pub fn allocate_next(&mut self) -> usize {
        if self.in_flight > 0
            && self
                .last_alloc_at
                .map(|t| t.elapsed() > Duration::from_secs(4))
                .unwrap_or(false)
        {
            self.in_flight = 0;
        }
        let next = self.composer_count + self.in_flight + 1;
        self.in_flight += 1;
        self.last_alloc_at = Some(std::time::Instant::now());
        next
    }

    pub fn sync_from_composer(&mut self, live_count: usize, after_insert: bool) {
        self.composer_count = live_count;
        if after_insert {
            self.in_flight = self.in_flight.saturating_sub(1);
        } else if live_count == 0
            && self
                .last_alloc_at
                .map(|t| t.elapsed() > Duration::from_millis(600))
                .unwrap_or(true)
        {
            self.in_flight = 0;
        }
    }

    pub fn reset(&mut self) {
        self.composer_count = 0;
        self.in_flight = 0;
        self.last_alloc_at = None;
    }
}

pub struct AppShared {
    pub config: Mutex<AppConfig>,
    pub state_machine: Mutex<BarStateMachine>,
    pub last_bridge_mode: Mutex<BridgeMode>,
    pub last_layout: Mutex<CachedLayout>,
    pub dsh_conn: Arc<Mutex<DshConnectionState>>,
    pub tray_handles: Mutex<Option<TrayHandles>>,
    pub broker: Arc<BrokerClient>,
    pub search_engine: Arc<search::SearchEngine>,
    pub hook_shared: Arc<HookSharedConfig>,
    pub pick_counter: Mutex<PickCounterState>,
    pub compose_waiters: Mutex<HashMap<String, mpsc::Sender<String>>>,
    pub last_outside_hide_at: Mutex<Option<std::time::Instant>>,
    pub live_sync_enabled: Arc<AtomicBool>,
    pub last_target_window: Mutex<Option<universal::TargetWindowInfo>>,
    pub attached_sw_entities: Mutex<std::collections::HashSet<String>>,
}

impl AppShared {
    pub fn allocate_pick_number(&self) -> usize {
        self.pick_counter
            .lock()
            .map(|mut st| st.allocate_next())
            .unwrap_or(1)
    }
}

pub fn was_recently_hidden_by_outside_click(app: &AppHandle, window_ms: u64) -> bool {
    let Some(shared) = app.try_state::<AppShared>() else {
        return false;
    };
    shared
        .last_outside_hide_at
        .lock()
        .ok()
        .and_then(|g| *g)
        .map(|t| t.elapsed() < Duration::from_millis(window_ms))
        .unwrap_or(false)
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BarReportPayload {
    pub kind: String,
    pub target_height: i32,
    pub width: i32,
    pub shapes: Vec<ShapeRectDip>,
    pub session_id: String,
    pub latest_assistant_id: String,
    pub streaming: bool,
    pub user_message_count: usize,
    pub hovered: bool,
    pub pinned: bool,
    pub popup_open: bool,
    pub modal: bool,
    pub composer_focused: bool,
    pub capsule_count: usize,
    pub after_insert: bool,
    pub sw_entity_keys: Option<Vec<String>>,
    pub detail: String,
}

#[tauri::command]
fn bar_report(app: AppHandle, payload: BarReportPayload) {
    let Some(shared) = app.try_state::<AppShared>() else {
        return;
    };

    match payload.kind.as_str() {
        "layout" => {
            let dock_snap = window::get_dock_runtime_snapshot();
            let is_vertical = dock_snap.edge.is_side() || dock_snap.vertical;
            let width = if is_vertical {
                let sw = payload.width.clamp(360, 960);
                window::set_dock_side_width_dip(sw);
                if let Ok(mut cfg) = shared.config.lock() {
                    if cfg.side_width != sw {
                        cfg.side_width = sw;
                        let _ = cfg.save();
                    }
                }
                sw
            } else {
                let hw = payload.width.clamp(480, 960);
                window::set_dock_bar_width_dip(hw);
                if let Ok(mut cfg) = shared.config.lock() {
                    if cfg.bar_width != hw {
                        cfg.bar_width = hw;
                        let _ = cfg.save();
                    }
                }
                hw
            };
            let height = payload.target_height.clamp(DEFAULT_LIP_HEIGHT_DIP, 1400);

            let mut prev_h = 0i32;
            if let Ok(mut cached) = shared.last_layout.lock() {
                prev_h = cached.height_dip;
                cached.width_dip = width;
                cached.height_dip = height;
                cached.shapes = payload.shapes.clone();
            }

            static LOGGED_FIRST_LAYOUT: AtomicBool = AtomicBool::new(false);
            let crossed_lip = (prev_h <= DEFAULT_LIP_HEIGHT_DIP + 4) != (height <= DEFAULT_LIP_HEIGHT_DIP + 4);
            if !LOGGED_FIRST_LAYOUT.swap(true, Ordering::AcqRel) || crossed_lip {
                log::info!(
                    "[LAYOUT] w={width} h={height} (prev_h={prev_h}) shapes={}",
                    payload.shapes.len()
                );
            }

            if let Some(first_shape) = payload.shapes.first() {
                if first_shape.h >= 60 && first_shape.h <= 280 {
                    window::update_dock_composer_height_dip(first_shape.h);
                }
            }
            if let Some(bar_win) = app.get_webview_window("bar") {
                apply_bar_layout(&bar_win, width, height, &payload.shapes);
            }
        }

        "activity" => {
            dispatch_state_event(
                &app,
                StateEvent::AssistantActivity {
                    session_id: payload.session_id,
                    assistant_id: payload.latest_assistant_id,
                    streaming: payload.streaming,
                },
            );
        }

        "bubble_notify" => {
            dispatch_state_event(&app, StateEvent::IncomingBubbleNotice);
        }

        "ui" => {
            dispatch_state_event(
                &app,
                StateEvent::UiUpdated {
                    hovered: payload.hovered,
                    pinned: payload.pinned,
                    popup_open: payload.popup_open,
                    modal: payload.modal,
                    composer_focused: payload.composer_focused,
                },
            );
        }

        "step_down" => {
            dispatch_state_event(&app, StateEvent::StepDown);
        }

        "composer_pointerdown" => {
            dispatch_state_event(&app, StateEvent::ComposerPointerDown);
        }

        "summon_from_lip" => {
            dispatch_state_event(&app, StateEvent::SummonFromLip);
        }

        "submitted" => {
            if let Ok(mut set) = shared.attached_sw_entities.lock() {
                set.clear();
            }
            dispatch_state_event(&app, StateEvent::Submitted);
        }

        "capsules" => {
            if let Ok(mut set) = shared.attached_sw_entities.lock() {
                if payload.capsule_count == 0 {
                    set.clear();
                } else if let Some(keys) = payload.sw_entity_keys {
                    set.clear();
                    for k in keys {
                        if !k.is_empty() {
                            set.insert(k);
                        }
                    }
                }
            }
            if let Ok(mut st) = shared.pick_counter.lock() {
                st.sync_from_composer(payload.capsule_count, payload.after_insert);
                log::info!(
                    "[CAPSULES] composer_count={} after_insert={} in_flight={}",
                    st.composer_count,
                    payload.after_insert,
                    st.in_flight
                );
            }
        }

        "plugin_not_loaded" => {
            let port = shared
                .dsh_conn
                .lock()
                .ok()
                .and_then(|c| c.active_port)
                .unwrap_or_else(|| {
                    shared
                        .config
                        .lock()
                        .map(|c| c.preferred_port)
                        .unwrap_or(3080)
                });
            let _ = plugin_sync::ensure_dsh_plugin_synced();
            let width = shared.config.lock().map(|c| c.bar_width).unwrap_or(760);
            let lip_shape = ShapeRectDip {
                x: 0,
                y: 0,
                w: width,
                h: DEFAULT_LIP_HEIGHT_DIP,
                r: 24,
            };
            if let Some(bar_win) = app.get_webview_window("bar") {
                apply_bar_layout(&bar_win, width, DEFAULT_LIP_HEIGHT_DIP, &[lip_shape]);
            }
            apply_discovery_to_bar(
                &app,
                &shared.dsh_conn,
                dsh::DiscoveryOutcome::MissingPlugin { port },
            );
        }

        "diag" => {
            log::info!("[WEBVIEW-DIAG] {}", payload.detail);
        }

        "capsule_rejected" => {
            if !payload.detail.trim().is_empty() {
                pick::self_heal::record_user_rejection(Some(&app), payload.detail.trim());
            }
        }

        other => {
            log::debug!("Unhandled bar_report kind: {other}");
        }
    }
}

#[tauri::command]
fn pick_list_profiles() -> serde_json::Value {
    serde_json::json!({
        "profiles": pick::profile::list_all_profiles(),
        "visualBoxProvider": pick::hitmap::active_visual_box_provider_name(),
    })
}

#[tauri::command]
fn pick_reset_profile(exe_name: String) -> Result<bool, String> {
    pick::self_heal::clear_runtime_health(&exe_name);
    pick::hitmap::invalidate_all_hitmaps();
    pick::profile::reset_learned_profile(&exe_name).map_err(|e| e.to_string())
}

#[tauri::command]
fn pick_export_profile(exe_name: String) -> Result<String, String> {
    pick::profile::export_profile_json(&exe_name).map_err(|e| e.to_string())
}

#[tauri::command]
fn pick_import_profile(raw_json: String) -> Result<pick::profile::AppUiProfile, String> {
    let saved = pick::profile::import_profile_json(&raw_json).map_err(|e| e.to_string())?;
    pick::self_heal::clear_runtime_health(&saved.exe_name);
    pick::hitmap::invalidate_all_hitmaps();
    Ok(saved)
}

#[tauri::command]
fn adapter_plugins_list() -> pick::adapter_plugins::AdapterPluginsOverview {
    pick::adapter_plugins::list_plugins()
}

#[tauri::command]
fn adapter_plugins_reload() -> pick::adapter_plugins::AdapterPluginsOverview {
    pick::hitmap::invalidate_all_hitmaps();
    pick::adapter_plugins::reload_from_disk()
}

#[tauri::command]
fn adapter_plugins_set_enabled(
    id: String,
    enabled: bool,
) -> Result<pick::adapter_plugins::AdapterPluginItem, String> {
    let item = pick::adapter_plugins::set_plugin_enabled(&id, enabled).map_err(|e| e.to_string())?;
    pick::hitmap::invalidate_all_hitmaps();
    Ok(item)
}

#[tauri::command]
async fn adapter_plugins_install_url(
    url: String,
    trust_command: Option<bool>,
) -> Result<pick::adapter_plugins::AdapterPluginItem, String> {
    let trust = trust_command.unwrap_or(false);
    let item = tauri::async_runtime::spawn_blocking(move || {
        pick::adapter_plugins::install_from_url(&url, trust)
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
    .map_err(|e| e.to_string())?;
    pick::hitmap::invalidate_all_hitmaps();
    Ok(item)
}

#[tauri::command]
fn adapter_plugins_install_json(
    raw_json: String,
    source_hint: Option<String>,
    trust_command: Option<bool>,
) -> Result<pick::adapter_plugins::AdapterPluginItem, String> {
    let item = pick::adapter_plugins::install_from_json(
        &raw_json,
        source_hint.as_deref(),
        trust_command.unwrap_or(false),
    )
    .map_err(|e| e.to_string())?;
    pick::hitmap::invalidate_all_hitmaps();
    Ok(item)
}

#[tauri::command]
async fn adapter_plugins_install_file(
    path: String,
    trust_command: Option<bool>,
) -> Result<pick::adapter_plugins::AdapterPluginItem, String> {
    let trust = trust_command.unwrap_or(false);
    let item = tauri::async_runtime::spawn_blocking(move || {
        pick::adapter_plugins::install_from_local_path(&path, trust)
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
    .map_err(|e| e.to_string())?;
    pick::hitmap::invalidate_all_hitmaps();
    Ok(item)
}

#[tauri::command]
fn adapter_plugins_uninstall(id: String) -> Result<bool, String> {
    let removed = pick::adapter_plugins::uninstall_plugin(&id).map_err(|e| e.to_string())?;
    pick::hitmap::invalidate_all_hitmaps();
    Ok(removed)
}

#[tauri::command]
fn adapter_plugins_export(id: String) -> Result<String, String> {
    pick::adapter_plugins::export_plugin_json(&id).map_err(|e| e.to_string())
}

#[tauri::command]
async fn adapter_plugins_fetch_registry(
    registry_url: Option<String>,
) -> Result<pick::adapter_plugins::RegistryFetchResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        pick::adapter_plugins::fetch_registry(registry_url.as_deref())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))
}

#[tauri::command]
async fn adapter_plugins_test(
    id: String,
) -> Result<pick::adapter_plugins::PluginTestReport, String> {
    tauri::async_runtime::spawn_blocking(move || pick::adapter_plugins::test_plugin_dry_run(&id))
        .await
        .map_err(|e| format!("task join error: {e}"))?
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn adapter_plugins_open_dir() -> Result<String, String> {
    let dir = pick::adapter_plugins::plugins_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path_str = dir.to_string_lossy().to_string();
    let _ = std::process::Command::new("explorer.exe")
        .arg(&path_str)
        .spawn();
    Ok(path_str)
}

#[tauri::command]
async fn vision_get_config(state: tauri::State<'_, AppShared>) -> Result<serde_json::Value, String> {
    let vcfg = state
        .config
        .lock()
        .map(|c| c.vision.clone())
        .unwrap_or_default();
    let probe_cfg = vcfg.clone();
    let hw = tauri::async_runtime::spawn_blocking(move || {
        pick::vision::probe_local_hardware(&probe_cfg)
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "config": vcfg,
        "hardware": hw,
    }))
}

#[tauri::command]
async fn vision_probe_hardware(
    state: tauri::State<'_, AppShared>,
) -> Result<pick::vision::HardwareProbeReport, String> {
    let vcfg = state
        .config
        .lock()
        .map(|c| c.vision.clone())
        .unwrap_or_default();
    tauri::async_runtime::spawn_blocking(move || pick::vision::probe_local_hardware(&vcfg))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn vision_update_config(
    state: tauri::State<'_, AppShared>,
    patch: serde_json::Value,
    apply_preset: Option<bool>,
) -> Result<config::VisionConfig, String> {
    let mut cfg = state
        .config
        .lock()
        .map_err(|_| "config lock poisoned".to_string())?;
    let mut current = serde_json::to_value(&cfg.vision).map_err(|e| e.to_string())?;
    if let (Some(dst), Some(src)) = (current.as_object_mut(), patch.as_object()) {
        for (k, v) in src {
            dst.insert(k.clone(), v.clone());
        }
    }
    let mut next: config::VisionConfig =
        serde_json::from_value(current).map_err(|e| format!("无效的视觉配置: {e}"))?;
    if apply_preset.unwrap_or(false) {
        let p = next.preset;
        next.apply_preset(p);
    }
    next.clamp();
    cfg.vision = next.clone();
    cfg.save().map_err(|e| e.to_string())?;
    drop(cfg);
    pick::hitmap::register_visual_box_provider(
        pick::vision::sidecar_client::maybe_build_omniparser_provider(),
    );
    Ok(next)
}

#[tauri::command]
async fn vision_test_pipeline(
    state: tauri::State<'_, AppShared>,
) -> Result<serde_json::Value, String> {
    let vcfg = state
        .config
        .lock()
        .map(|c| c.vision.clone())
        .unwrap_or_default();
    tauri::async_runtime::spawn_blocking(move || pick::vision::test_active_pipeline(&vcfg))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn vision_unload_models(
    state: tauri::State<'_, AppShared>,
) -> Result<serde_json::Value, String> {
    let vcfg = state
        .config
        .lock()
        .map(|c| c.vision.clone())
        .unwrap_or_default();
    tauri::async_runtime::spawn_blocking(move || pick::vision::unload_all_local_models(&vcfg))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn bar_reconnect(app: AppHandle) {
    trigger_dsh_reconnect(app);
}

fn push_dock_state_to_bar(app: &AppHandle, edge: config::DockEdge, collapsed: bool) {
    let snap = window::get_dock_runtime_snapshot();
    let vertical = snap.vertical || edge.is_side();
    if let Some(bar_win) = app.get_webview_window("bar") {
        let js = format!(
            "(function(){{ if (typeof window.__DSB_DOCK_STATE__ === 'function') {{ window.__DSB_DOCK_STATE__({{\"edge\":\"{}\",\"collapsed\":{},\"vertical\":{},\"unread\":{}}}); }} }})();",
            edge.as_str(),
            if collapsed { "true" } else { "false" },
            if vertical { "true" } else { "false" },
            if snap.unread { "true" } else { "false" }
        );
        let _ = bar_win.eval(&js);
    }
}

pub fn apply_dock_edge(app: &AppHandle, edge: config::DockEdge, start_collapsed: bool) {
    apply_dock_edge_inner(app, edge, start_collapsed, false);
}

fn apply_dock_edge_inner(
    app: &AppHandle,
    edge: config::DockEdge,
    start_collapsed: bool,
    from_drag: bool,
) {
    let Some(shared) = app.try_state::<AppShared>() else {
        return;
    };
    let (bar_width, next_vertical) = if let Ok(mut cfg) = shared.config.lock() {
        let mut changed = false;
        if cfg.dock_edge != edge {
            cfg.dock_edge = edge;
            changed = true;
        }
        // Strict edge-orientation coupling:
        // - Left / Right dock MUST always be Vertical
        // - Top / Bottom dock MUST always be Horizontal
        // - Free floating (`None`) preserves current horizontal/vertical orientation!
        let desired_vertical = match edge {
            config::DockEdge::Left | config::DockEdge::Right => true,
            config::DockEdge::Top | config::DockEdge::Bottom => false,
            config::DockEdge::None => cfg.vertical_mode,
        };
        if cfg.vertical_mode != desired_vertical {
            cfg.vertical_mode = desired_vertical;
            changed = true;
        }
        if changed {
            let _ = cfg.save();
        }
        (cfg.bar_width, cfg.vertical_mode)
    } else {
        (760, edge.is_side())
    };
    window::set_dock_vertical_mode(next_vertical);
    window::set_active_dock_state(edge, start_collapsed, None);
    let state_ev = if from_drag && !start_collapsed {
        StateEvent::SetDockEdgeFromDrag { edge }
    } else {
        StateEvent::SetDockEdge {
            edge,
            start_collapsed,
        }
    };
    dispatch_state_event(app, state_ev);
    // Note: when `edge.is_docked()`, `dispatch_state_event` ALREADY called `push_dock_state_to_bar`!
    // Never push a duplicate `__DSB_DOCK_STATE__` 0.1ms later (which would cancel active morph animations).
    if !edge.is_docked() {
        push_dock_state_to_bar(app, edge, start_collapsed);
        let target_w = if next_vertical {
            window::get_dock_side_width_dip()
        } else {
            bar_width
        };
        if let Some(bar_win) = app.get_webview_window("bar") {
            apply_bar_layout(&bar_win, target_w, window::BAR_STAGE_HEIGHT_DIP, &[]);
        }
    }
}

pub fn apply_vertical_mode(app: &AppHandle, vertical: bool) {
    let Some(shared) = app.try_state::<AppShared>() else {
        return;
    };
    let current_snap = window::get_dock_runtime_snapshot();
    // Enforce edge-orientation invariant when toggling orientation via the button:
    // - Switching to Horizontal while docked Left/Right -> undock to free Horizontal bar
    // - Switching to Vertical while docked Top/Bottom -> undock to free Vertical panel
    // - Free floating (`None`) stays free floating in the chosen orientation!
    let next_edge = if (!vertical && current_snap.edge.is_side())
        || (vertical && matches!(current_snap.edge, config::DockEdge::Top | config::DockEdge::Bottom))
    {
        config::DockEdge::None
    } else {
        current_snap.edge
    };
    let bar_width = if let Ok(mut cfg) = shared.config.lock() {
        let mut changed = false;
        if cfg.vertical_mode != vertical {
            cfg.vertical_mode = vertical;
            changed = true;
        }
        if cfg.dock_edge != next_edge {
            cfg.dock_edge = next_edge;
            changed = true;
        }
        if changed {
            let _ = cfg.save();
        }
        cfg.bar_width
    } else {
        760
    };
    window::set_dock_vertical_mode(vertical);
    let mut pushed_by_state = false;
    if next_edge != current_snap.edge {
        window::set_active_dock_state(next_edge, false, None);
        // When undocking to `None`, `SetDockEdge` returns `none()` if already Summon, so we push once below.
        let was_collapsed = current_snap.collapsed;
        dispatch_state_event(
            app,
            StateEvent::SetDockEdge {
                edge: next_edge,
                start_collapsed: false,
            },
        );
        pushed_by_state = next_edge.is_docked() || was_collapsed;
    }
    let collapsed = next_edge.is_docked() && current_snap.collapsed;
    if let Some(bar_win) = app.get_webview_window("bar") {
        if next_edge.is_docked() {
            window::slide_docked_bar(&bar_win, next_edge, collapsed, bar_width, !collapsed);
        } else {
            // Keep the constant 960dip stage unmoved (ZERO WM_SIZE / swapchain resize!)
            // and immediately open the union RGN of horizontal + vertical bands for the 260ms in-place GPU morph!
            window::open_orientation_morph_rgn(&bar_win, bar_width);
        }
    }
    if !pushed_by_state {
        push_dock_state_to_bar(app, next_edge, collapsed);
    }
}

#[tauri::command]
fn bar_drag_begin(app: AppHandle) -> bool {
    window::cancel_slide_anim();
    if let Some(shared) = app.try_state::<AppShared>() {
        if let Ok(mut sm) = shared.state_machine.lock() {
            sm.dock_locked_open = true;
        }
    }
    app.get_webview_window("bar")
        .map(|w| window::begin_bar_drag(&w))
        .unwrap_or(false)
}

#[tauri::command]
fn bar_drag_step(app: AppHandle) -> window::DragStepResult {
    app.get_webview_window("bar")
        .map(|w| window::step_bar_drag(&w))
        .unwrap_or(window::DragStepResult {
            moved: false,
            snap_preview: config::DockEdge::None,
            undocked: false,
        })
}

#[tauri::command]
fn bar_drag_end(app: AppHandle) -> window::DragEndResult {
    let res = window::end_bar_drag();
    if res.snapped_edge.is_docked() {
        apply_dock_edge_inner(&app, res.snapped_edge, false, true);
    } else if res.undocked {
        apply_dock_edge_inner(&app, config::DockEdge::None, false, true);
    } else if let Some(shared) = app.try_state::<AppShared>() {
        // Release the drag lock when dropping back on the same docked edge so hover auto-collapse resumes
        if let Ok(mut sm) = shared.state_machine.lock() {
            if sm.dock_edge.is_docked() {
                sm.dock_locked_open = false;
            }
        }
    }
    res
}

#[tauri::command]
fn bar_drag_reset(app: AppHandle) {
    let Some(shared) = app.try_state::<AppShared>() else {
        return;
    };
    apply_dock_edge(&app, config::DockEdge::None, false);
    let cached = shared
        .last_layout
        .lock()
        .map(|c| c.clone())
        .unwrap_or_else(|_| CachedLayout {
            width_dip: 760,
            height_dip: window::BAR_STAGE_HEIGHT_DIP,
            shapes: Vec::new(),
        });
    if let Some(bar_win) = app.get_webview_window("bar") {
        window::reset_bar_anchor(&bar_win, cached.width_dip, cached.height_dip, &cached.shapes);
    }
}

#[tauri::command]
fn bar_get_dock_state() -> window::DockStateSnapshot {
    window::get_dock_runtime_snapshot()
}

#[tauri::command]
fn bar_set_dock_edge(app: AppHandle, edge: String, start_collapsed: Option<bool>) -> window::DockStateSnapshot {
    let parsed = config::DockEdge::parse(&edge);
    apply_dock_edge(&app, parsed, start_collapsed.unwrap_or(false));
    window::get_dock_runtime_snapshot()
}

#[tauri::command]
fn bar_set_vertical_mode(app: AppHandle, vertical: bool) -> window::DockStateSnapshot {
    apply_vertical_mode(&app, vertical);
    window::get_dock_runtime_snapshot()
}

#[tauri::command]
fn bar_set_dock_unread(app: AppHandle, unread: bool) {
    let prev = window::get_dock_runtime_snapshot();
    if prev.unread == unread {
        return;
    }
    window::set_dock_unread(unread);
    if prev.edge.is_docked() && prev.collapsed {
        let bar_width = app
            .try_state::<AppShared>()
            .and_then(|s| s.config.lock().ok().map(|c| c.bar_width))
            .unwrap_or(760);
        if let Some(bar_win) = app.get_webview_window("bar") {
            window::slide_docked_bar(&bar_win, prev.edge, true, bar_width, false);
        }
    }
}

#[tauri::command]
fn bar_dock_expand(app: AppHandle) {
    dispatch_state_event(&app, StateEvent::DockEdgeClickExpand);
}

#[tauri::command]
fn overlay_compose_done(app: AppHandle, id: String, data_url: String) {
    if let Some(shared) = app.try_state::<AppShared>() {
        if let Ok(mut map) = shared.compose_waiters.lock() {
            if let Some(tx) = map.remove(&id) {
                let _ = tx.send(data_url);
            }
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HarnessSendPayload {
    harness: String,
    turn_id: String,
    cwd: Option<String>,
    project_id: Option<String>,
    session_id: Option<String>,
    permission_mode: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    effort: Option<String>,
    #[serde(default)]
    sandbox_mode: Option<String>,
    #[serde(default)]
    execution_mode: Option<String>,
    #[serde(default)]
    fast_speed: Option<bool>,
    #[serde(default)]
    plan_mode: Option<bool>,
    #[serde(default)]
    sandbox_enabled: Option<bool>,
    #[serde(default)]
    chrome_enabled: Option<bool>,
    #[serde(default)]
    enabled_plugins: Vec<String>,
    #[serde(default)]
    attachments: Vec<harness::TurnAttachment>,
    prompt: String,
}

fn normalize_harness_route(raw: &str) -> String {
    let trimmed = raw.trim().to_ascii_lowercase();
    if trimmed == "universal" {
        return "universal".to_string();
    }
    if acp::is_acp_harness_id(&trimmed) {
        return format!("acp:{}", acp::normalize_acp_slug(&trimmed));
    }
    harness::HarnessKind::parse(&trimmed).as_str().to_string()
}

fn read_custom_acp_list(app: &AppHandle) -> Vec<config::CustomAcpHarness> {
    app.try_state::<AppShared>()
        .and_then(|s| s.config.lock().ok().map(|c| c.custom_acp_harnesses.clone()))
        .unwrap_or_default()
}

#[tauri::command]
fn harness_get_active(app: AppHandle) -> String {
    app.try_state::<AppShared>()
        .and_then(|s| s.config.lock().ok().map(|c| c.active_harness.clone()))
        .unwrap_or_else(|| "dsh".to_string())
}

#[tauri::command]
fn harness_set_active(app: AppHandle, harness: String) -> String {
    let normalized = normalize_harness_route(&harness);
    if let Some(shared) = app.try_state::<AppShared>() {
        if let Ok(mut cfg) = shared.config.lock() {
            if cfg.active_harness != normalized {
                cfg.active_harness = normalized.clone();
                let _ = cfg.save();
            }
        }
    }
    normalized
}

#[tauri::command]
fn harness_list_available(app: AppHandle) -> Vec<harness::HarnessAvailability> {
    let dsh_connected = app
        .try_state::<AppShared>()
        .and_then(|s| s.dsh_conn.lock().ok().map(|c| c.active_port.is_some()))
        .unwrap_or(false);
    let custom_acp = read_custom_acp_list(&app);
    let target_label = app
        .try_state::<AppShared>()
        .and_then(|s| {
            s.last_target_window
                .lock()
                .ok()
                .and_then(|g| g.as_ref().map(|t| t.display_label.clone()))
        });
    harness::list_harness_availability(dsh_connected, &custom_acp, target_label.as_deref())
}

#[tauri::command]
fn harness_load_catalog(app: AppHandle, harness: String) -> harness::ExternalHarnessCatalog {
    let norm = normalize_harness_route(&harness);
    if norm == "universal" {
        let target = universal_get_target(app);
        let title = if target.valid && !target.display_label.is_empty() {
            target.display_label.clone()
        } else {
            "上一个聚焦的任意窗口".to_string()
        };
        return harness::ExternalHarnessCatalog {
            harness: "universal".to_string(),
            available: true,
            exe_path: None,
            groups: vec![harness::ExternalWorkspaceGroup {
                workspace_id: "universal-focus".to_string(),
                label: "当前焦点窗口".to_string(),
                path: String::new(),
                convs: vec![harness::ExternalConversation {
                    id: "universal-active".to_string(),
                    title,
                    updated_at: target.updated_at_ms,
                    source: "universal".to_string(),
                    cwd: String::new(),
                    rollout_path: None,
                }],
            }],
            error: None,
        };
    }
    if acp::is_acp_harness_id(&norm) {
        let custom_acp = read_custom_acp_list(&app);
        return acp::load_acp_catalog(&norm, &custom_acp);
    }
    match harness::HarnessKind::parse(&norm) {
        harness::HarnessKind::Antigravity => harness::load_antigravity_catalog(),
        harness::HarnessKind::Codex => harness::load_codex_catalog(),
        harness::HarnessKind::ClaudeCode => harness::load_claude_code_catalog(),
        harness::HarnessKind::Dsh => harness::ExternalHarnessCatalog {
            harness: "dsh".to_string(),
            available: true,
            exe_path: None,
            groups: Vec::new(),
            error: None,
        },
    }
}

#[tauri::command]
fn harness_load_messages(harness: String, session_id: String) -> Vec<harness::ExternalChatMessage> {
    let norm = normalize_harness_route(&harness);
    if norm == "universal" {
        return Vec::new();
    }
    if acp::is_acp_harness_id(&norm) {
        return acp::load_acp_messages(&norm, &session_id);
    }
    let kind = harness::HarnessKind::parse(&norm);
    harness::load_external_messages(kind, &session_id)
}

#[tauri::command]
fn harness_list_options(app: AppHandle, harness: String) -> harness::HarnessComposerOptions {
    let norm = normalize_harness_route(&harness);
    if norm == "universal" {
        return harness::HarnessComposerOptions {
            harness: "universal".to_string(),
            models: vec![harness::HarnessOptionItem {
                id: "universal-target".into(),
                label: "当前目标窗口".into(),
                desc: "自动投递到唤出悬浮条前的焦点窗口".into(),
            }],
            efforts: Vec::new(),
            modes: vec![
                harness::HarnessOptionItem {
                    id: "paste_only".into(),
                    label: "仅粘贴".into(),
                    desc: "切回目标窗口并 Ctrl+V 粘贴，由你确认后发送（推荐）".into(),
                },
                harness::HarnessOptionItem {
                    id: "paste_and_enter".into(),
                    label: "粘贴并发送".into(),
                    desc: "切回目标窗口粘贴后自动按 Enter 发送".into(),
                },
                harness::HarnessOptionItem {
                    id: "clipboard_only".into(),
                    label: "仅复制".into(),
                    desc: "不切换窗口，仅将整理好的 Prompt 写入剪贴板".into(),
                },
            ],
            plugins: Vec::new(),
            actions: Vec::new(),
            default_model: "universal-target".to_string(),
            default_effort: String::new(),
            default_mode: "paste_only".to_string(),
        };
    }
    if acp::is_acp_harness_id(&norm) {
        let custom_acp = read_custom_acp_list(&app);
        return acp::probe_acp_options(&norm, &custom_acp);
    }
    let kind = harness::HarnessKind::parse(&norm);
    harness::load_harness_composer_options(kind)
}

#[tauri::command]
fn harness_send_message(app: AppHandle, payload: HarnessSendPayload) -> Result<(), String> {
    let Some(shared) = app.try_state::<AppShared>() else {
        return Err("应用状态未就绪".to_string());
    };
    let norm = normalize_harness_route(&payload.harness);
    log::info!(
        "[HARNESS_SEND] harness={} turn_id={} session={:?} project={:?} cwd={:?} prompt_chars={} atts={}",
        norm,
        payload.turn_id,
        payload.session_id,
        payload.project_id,
        payload.cwd,
        payload.prompt.chars().count(),
        payload.attachments.len()
    );

    if norm == "universal" {
        let mode = payload
            .execution_mode
            .as_deref()
            .or(payload.permission_mode.as_deref())
            .or(payload.sandbox_mode.as_deref())
            .unwrap_or("paste_only")
            .to_string();
        let full_text = harness::build_prompt_with_file_attachments(
            &payload.prompt,
            &payload.attachments,
            true,
        );
        let target_hwnd = shared
            .last_target_window
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|t| t.hwnd))
            .unwrap_or(0);
        let turn_id = payload.turn_id.clone();
        let app_clone = app.clone();
        if mode != "clipboard_only" {
            dispatch_state_event(&app, StateEvent::OutsideClickHide);
        }
        std::thread::spawn(move || {
            if mode != "clipboard_only" {
                std::thread::sleep(Duration::from_millis(60));
            }
            match universal::deliver_to_target_window(target_hwnd, &full_text, &mode) {
                Ok(msg) => {
                    harness::push_harness_event_to_bar(
                        &app_clone,
                        &harness::HarnessStreamEvent {
                            turn_id,
                            harness: "universal".to_string(),
                            kind: "completed".to_string(),
                            session_id: Some("universal-active".to_string()),
                            text: Some(msg),
                            tool_name: None,
                            error: None,
                        },
                    );
                }
                Err(err) => {
                    harness::push_harness_event_to_bar(
                        &app_clone,
                        &harness::HarnessStreamEvent {
                            turn_id,
                            harness: "universal".to_string(),
                            kind: "error".to_string(),
                            session_id: Some("universal-active".to_string()),
                            text: None,
                            tool_name: None,
                            error: Some(err),
                        },
                    );
                }
            }
        });
        return Ok(());
    }

    if acp::is_acp_harness_id(&norm) {
        let custom_acp = read_custom_acp_list(&app);
        let Some(spec) = acp::resolve_acp_agent_spec(&norm, &custom_acp) else {
            return Err(format!("未找到 ACP Harness：{norm}"));
        };
        let broker = shared.broker.clone();
        let mode = payload
            .execution_mode
            .or(payload.permission_mode)
            .or(payload.sandbox_mode);
        return harness::start_acp_harness_turn(
            app.clone(),
            broker,
            norm,
            spec,
            payload.turn_id,
            payload.cwd,
            payload.session_id,
            payload.model,
            payload.effort,
            mode,
            payload.attachments,
            payload.prompt,
        );
    }

    let kind = harness::HarnessKind::parse(&norm);
    let broker = shared.broker.clone();
    harness::start_external_harness_turn(
        app.clone(),
        broker,
        harness::ExternalTurnRequest {
            harness: kind,
            turn_id: payload.turn_id,
            cwd: payload.cwd,
            project_id: payload.project_id,
            session_id: payload.session_id,
            permission_mode: payload.permission_mode,
            model: payload.model,
            effort: payload.effort,
            sandbox_mode: payload.sandbox_mode,
            execution_mode: payload.execution_mode,
            fast_speed: payload.fast_speed,
            plan_mode: payload.plan_mode,
            sandbox_enabled: payload.sandbox_enabled,
            chrome_enabled: payload.chrome_enabled,
            enabled_plugins: payload.enabled_plugins,
            attachments: payload.attachments,
            prompt: payload.prompt,
        },
    )
}

#[tauri::command]
fn harness_session_stats(harness: String, session_id: String) -> harness::HarnessSessionStats {
    let norm = normalize_harness_route(&harness);
    if acp::is_acp_harness_id(&norm) || norm == "universal" {
        let turns = acp::load_acp_messages(&norm, &session_id)
            .into_iter()
            .filter(|m| m.role == "assistant")
            .count();
        return harness::HarnessSessionStats {
            harness: norm.clone(),
            session_id: session_id.clone(),
            model: "ACP".to_string(),
            turns,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            total_tokens: 0,
            summary_markdown: format!("### 📊 会话统计\n\n- **目标**：`{norm}`\n- **回复轮次**：`{turns}` 轮"),
        };
    }
    let kind = harness::HarnessKind::parse(&norm);
    harness::load_session_stats(kind, &session_id)
}

#[tauri::command]
fn harness_session_state(harness: String, session_id: String) -> harness::HarnessSessionState {
    let norm = normalize_harness_route(&harness);
    if norm == "universal" {
        return harness::HarnessSessionState {
            harness: "universal".to_string(),
            session_id,
            source_note: "万能投递模式：发送后将自动切回目标窗口粘贴".to_string(),
            ..Default::default()
        };
    }
    if acp::is_acp_harness_id(&norm) {
        return acp::load_acp_session_state(&norm, &session_id);
    }
    let kind = harness::HarnessKind::parse(&norm);
    harness::load_session_state(kind, &session_id)
}

#[tauri::command]
fn harness_upsert_custom_acp(
    app: AppHandle,
    mut item: config::CustomAcpHarness,
) -> Result<Vec<config::CustomAcpHarness>, String> {
    let Some(shared) = app.try_state::<AppShared>() else {
        return Err("应用状态未就绪".to_string());
    };
    let cmd = item.command.trim().to_string();
    if cmd.is_empty() {
        return Err("启动命令不能为空（例如 opencode / gemini / npx）".to_string());
    }
    let label = if item.label.trim().is_empty() {
        cmd.clone()
    } else {
        item.label.trim().to_string()
    };
    let slug = if item.id.trim().is_empty() {
        acp::normalize_acp_slug(&label)
    } else {
        acp::normalize_acp_slug(&item.id)
    };
    item.id = slug.clone();
    item.label = label;
    item.command = cmd;
    if !item.accent.trim().starts_with('#') {
        item.accent = "#38bdf8".to_string();
    }
    let mut cfg = shared
        .config
        .lock()
        .map_err(|_| "配置锁不可用".to_string())?;
    if let Some(existing) = cfg
        .custom_acp_harnesses
        .iter_mut()
        .find(|x| acp::normalize_acp_slug(&x.id) == slug)
    {
        *existing = item;
    } else {
        cfg.custom_acp_harnesses.push(item);
    }
    let list = cfg.custom_acp_harnesses.clone();
    let _ = cfg.save();
    Ok(list)
}

#[tauri::command]
fn harness_remove_custom_acp(
    app: AppHandle,
    id: String,
) -> Result<Vec<config::CustomAcpHarness>, String> {
    let Some(shared) = app.try_state::<AppShared>() else {
        return Err("应用状态未就绪".to_string());
    };
    let slug = acp::normalize_acp_slug(&id);
    let mut cfg = shared
        .config
        .lock()
        .map_err(|_| "配置锁不可用".to_string())?;
    cfg.custom_acp_harnesses
        .retain(|x| acp::normalize_acp_slug(&x.id) != slug);
    let list = cfg.custom_acp_harnesses.clone();
    let _ = cfg.save();
    Ok(list)
}

#[tauri::command]
fn universal_get_target(app: AppHandle) -> universal::TargetWindowInfo {
    let cached = app.try_state::<AppShared>().and_then(|s| {
        s.last_target_window
            .lock()
            .ok()
            .and_then(|g| g.clone())
    });
    if let Some(mut info) = cached {
        info.valid = universal::is_target_hwnd_still_valid(info.hwnd);
        return info;
    }
    universal::TargetWindowInfo::default()
}

#[tauri::command]
fn harness_cancel_turn(app: AppHandle, turn_id: String) -> bool {
    let Some(shared) = app.try_state::<AppShared>() else {
        return false;
    };
    let resp = shared
        .broker
        .call(crate::broker::BrokerRequest::CancelHarnessTurn { turn_id });
    resp.ok
}

#[tauri::command]
fn harness_focus_desktop(
    app: AppHandle,
    harness: String,
    session_id: Option<String>,
    title: Option<String>,
    workspace_path: Option<String>,
) -> Result<String, String> {
    let kind = harness::HarnessKind::parse(&harness);
    let Some(shared) = app.try_state::<AppShared>() else {
        return Err("应用状态未就绪".to_string());
    };
    let config = shared.config.lock().map(|c| c.clone()).unwrap_or_default();
    let broker = shared.broker.clone();
    let dsh_conn = shared.dsh_conn.clone();
    // Step Bar out of the way so the target desktop app / browser receives clean foreground focus
    dispatch_state_event(&app, StateEvent::OutsideClickHide);
    harness::focus_harness_desktop(
        app.clone(),
        kind,
        session_id,
        title,
        workspace_path,
        config,
        broker,
        dsh_conn,
    )
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AppSettingsSnapshot {
    version: String,
    is_elevated: bool,
    dsh_status: String,
    config: AppConfig,
    sync_status: harness_sync::ExternalSyncPluginStatus,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct AppSettingsPatch {
    output_target: Option<config::OutputTarget>,
    startup_breath_pulse: Option<bool>,
    hide_from_screen_capture: Option<bool>,
    show_tray_icon: Option<bool>,
    sync_external_plugins: Option<bool>,
    prefer_admin: Option<bool>,
    autostart: Option<bool>,
}

fn build_settings_snapshot(app: &AppHandle) -> AppSettingsSnapshot {
    let (cfg, dsh_status) = if let Some(shared) = app.try_state::<AppShared>() {
        let cfg = shared
            .config
            .lock()
            .map(|c| c.clone())
            .unwrap_or_default();
        let dsh_st = shared
            .dsh_conn
            .lock()
            .map(|d| d.status_label.clone())
            .unwrap_or_else(|_| "未连接".to_string());
        (cfg, dsh_st)
    } else {
        (AppConfig::load_or_create(), "未连接".to_string())
    };
    let sync_status = harness_sync::query_external_sync_status(cfg.sync_external_plugins);
    AppSettingsSnapshot {
        version: env!("CARGO_PKG_VERSION").to_string(),
        is_elevated: elevation::is_elevated(),
        dsh_status,
        config: cfg,
        sync_status,
    }
}

fn apply_capture_affinity_to_windows(app: &AppHandle, exclude: bool) {
    if let Some(bar_win) = app.get_webview_window("bar") {
        if let Some(h) = hwnd_of(&bar_win) {
            window::apply_capture_affinity(h, exclude);
        }
    }
    if let Some(ov_win) = app.get_webview_window("overlay") {
        if let Some(h) = hwnd_of(&ov_win) {
            window::apply_capture_affinity(h, exclude);
        }
    }
}

fn pick_folder_dialog_blocking(initial_dir: Option<String>) -> Option<String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{
        FileOpenDialog, IFileOpenDialog, IShellItem, SHCreateItemFromParsingName,
        FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
    };

    let resolved_dir: Option<PathBuf> = initial_dir
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .and_then(|p| {
            if p.is_dir() {
                Some(p)
            } else {
                p.parent().filter(|par| par.is_dir()).map(Path::to_path_buf)
            }
        })
        .or_else(dirs::home_dir);

    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let com_inited = hr.is_ok();
        let result = (|| -> Option<String> {
            let dialog: IFileOpenDialog =
                CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
            let opts = dialog.GetOptions().ok()?;
            dialog
                .SetOptions(opts | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST)
                .ok()?;
            let title_w: Vec<u16> = OsStr::new("选择或新建工作区文件夹")
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let _ = dialog.SetTitle(PCWSTR(title_w.as_ptr()));
            if let Some(dir) = resolved_dir {
                let dir_w: Vec<u16> = OsStr::new(&dir.to_string_lossy().to_string())
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect();
                if let Ok(item) =
                    SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(dir_w.as_ptr()), None)
                {
                    let _ = dialog.SetFolder(&item);
                }
            }
            if dialog.Show(None).is_err() {
                return None;
            }
            let item = dialog.GetResult().ok()?;
            let display: PWSTR = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
            let picked = display.to_string().ok();
            CoTaskMemFree(Some(display.0 as *const std::ffi::c_void));
            picked.filter(|s| !s.trim().is_empty())
        })();
        if com_inited {
            CoUninitialize();
        }
        result
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PickedAttachment {
    id: String,
    kind: String, // "image" | "file" | "folder"
    name: String,
    path: String,
    preview_data_url: Option<String>,
}

fn image_mime_for_ext(ext: &str) -> Option<&'static str> {
    match ext.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        "bmp" => Some("image/bmp"),
        _ => None,
    }
}

fn build_picked_attachment(path: &std::path::Path) -> PickedAttachment {
    use base64::Engine;
    static ATT_SEQ: AtomicU64 = AtomicU64::new(1);
    let seq = ATT_SEQ.fetch_add(1, Ordering::Relaxed);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path_str = path.to_string_lossy().to_string();
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| path_str.clone());
    if path.is_dir() {
        return PickedAttachment {
            id: format!("att-{ts}-{seq}"),
            kind: "folder".to_string(),
            name,
            path: path_str,
            preview_data_url: None,
        };
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    if let Some(mime) = image_mime_for_ext(ext) {
        let preview = std::fs::metadata(path)
            .ok()
            .filter(|m| m.len() <= 4 * 1024 * 1024)
            .and_then(|_| std::fs::read(path).ok())
            .map(|bytes| {
                format!(
                    "data:{mime};base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(bytes)
                )
            });
        return PickedAttachment {
            id: format!("att-{ts}-{seq}"),
            kind: "image".to_string(),
            name,
            path: path_str,
            preview_data_url: preview,
        };
    }
    PickedAttachment {
        id: format!("att-{ts}-{seq}"),
        kind: "file".to_string(),
        name,
        path: path_str,
        preview_data_url: None,
    }
}

fn pick_files_dialog_blocking(
    initial_dir: Option<String>,
    images_only: bool,
) -> Vec<PickedAttachment> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
    use windows::Win32::UI::Shell::{
        FileOpenDialog, IFileOpenDialog, IShellItem, SHCreateItemFromParsingName,
        FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, SIGDN_FILESYSPATH,
    };

    let resolved_dir: Option<PathBuf> = initial_dir
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .and_then(|p| {
            if p.is_dir() {
                Some(p)
            } else {
                p.parent().filter(|par| par.is_dir()).map(Path::to_path_buf)
            }
        })
        .or_else(dirs::home_dir);

    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let com_inited = hr.is_ok();
        let result = (|| -> Vec<PickedAttachment> {
            let Ok(dialog) =
                CoCreateInstance::<_, IFileOpenDialog>(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)
            else {
                return Vec::new();
            };
            let Ok(opts) = dialog.GetOptions() else {
                return Vec::new();
            };
            let _ = dialog.SetOptions(
                opts | FOS_ALLOWMULTISELECT | FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST,
            );
            let title_text = if images_only {
                "选择要附加的图片"
            } else {
                "选择要附加的文件"
            };
            let title_w: Vec<u16> = OsStr::new(title_text)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let _ = dialog.SetTitle(PCWSTR(title_w.as_ptr()));
            if images_only {
                let name_w: Vec<u16> = OsStr::new("图片文件")
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect();
                let spec_w: Vec<u16> = OsStr::new("*.png;*.jpg;*.jpeg;*.webp;*.gif;*.bmp")
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect();
                let filters = [COMDLG_FILTERSPEC {
                    pszName: PCWSTR(name_w.as_ptr()),
                    pszSpec: PCWSTR(spec_w.as_ptr()),
                }];
                let _ = dialog.SetFileTypes(&filters);
            }
            if let Some(dir) = resolved_dir {
                let dir_w: Vec<u16> = OsStr::new(&dir.to_string_lossy().to_string())
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect();
                if let Ok(item) =
                    SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(dir_w.as_ptr()), None)
                {
                    let _ = dialog.SetFolder(&item);
                }
            }
            if dialog.Show(None).is_err() {
                return Vec::new();
            }
            let Ok(arr) = dialog.GetResults() else {
                return Vec::new();
            };
            let count = arr.GetCount().unwrap_or(0);
            let mut out = Vec::new();
            for i in 0..count {
                if let Ok(item) = arr.GetItemAt(i) {
                    if let Ok(display) = item.GetDisplayName(SIGDN_FILESYSPATH) {
                        let raw: PWSTR = display;
                        if let Ok(p) = raw.to_string() {
                            if !p.trim().is_empty() {
                                out.push(build_picked_attachment(Path::new(&p)));
                            }
                        }
                        CoTaskMemFree(Some(raw.0 as *const std::ffi::c_void));
                    }
                }
            }
            out
        })();
        if com_inited {
            CoUninitialize();
        }
        result
    }
}

fn read_clipboard_files_win32() -> Vec<PickedAttachment> {
    use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
    use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

    const CF_HDROP: u32 = 15;
    let mut paths = Vec::new();
    unsafe {
        if OpenClipboard(None).is_err() {
            return Vec::new();
        }
        if let Ok(handle) = GetClipboardData(CF_HDROP) {
            if !handle.0.is_null() {
                let hdrop = HDROP(handle.0);
                let count = DragQueryFileW(hdrop, 0xFFFF_FFFF, None);
                for idx in 0..count.min(20) {
                    let len = DragQueryFileW(hdrop, idx, None);
                    if len == 0 {
                        continue;
                    }
                    let mut buf = vec![0u16; (len as usize) + 1];
                    let written = DragQueryFileW(hdrop, idx, Some(&mut buf));
                    if written > 0 {
                        let p = String::from_utf16_lossy(&buf[..written as usize]);
                        if !p.trim().is_empty() {
                            paths.push(p);
                        }
                    }
                }
            }
        }
        let _ = CloseClipboard();
    }
    paths
        .into_iter()
        .map(|p| build_picked_attachment(std::path::Path::new(&p)))
        .collect()
}

#[tauri::command]
fn app_ensure_default_workspace() -> String {
    harness::ensure_default_workspace_dir()
        .to_string_lossy()
        .to_string()
}

#[tauri::command]
async fn app_pick_folder(initial_dir: Option<String>) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || pick_folder_dialog_blocking(initial_dir))
        .await
        .ok()
        .flatten()
}

#[tauri::command]
async fn app_pick_files(
    initial_dir: Option<String>,
    images_only: Option<bool>,
) -> Vec<PickedAttachment> {
    let img = images_only.unwrap_or(false);
    tauri::async_runtime::spawn_blocking(move || pick_files_dialog_blocking(initial_dir, img))
        .await
        .unwrap_or_default()
}

#[tauri::command]
fn app_read_clipboard_files() -> Vec<PickedAttachment> {
    read_clipboard_files_win32()
}

#[tauri::command]
fn app_describe_paths(paths: Vec<String>) -> Vec<PickedAttachment> {
    paths
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(|s| build_picked_attachment(std::path::Path::new(&s)))
        .collect()
}

#[tauri::command]
fn app_save_clipboard_image(
    base64_data: String,
    mime: Option<String>,
) -> Result<PickedAttachment, String> {
    use base64::Engine;
    static PASTE_SEQ: AtomicU64 = AtomicU64::new(1);

    let raw_b64 = if let Some((_, rest)) = base64_data.split_once(";base64,") {
        rest.trim()
    } else {
        base64_data.trim()
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(raw_b64)
        .map_err(|e| format!("图片解码失败: {e}"))?;
    let mime_str = mime
        .as_deref()
        .unwrap_or("image/png")
        .trim()
        .to_ascii_lowercase();
    let ext = match mime_str.as_str() {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/bmp" => "bmp",
        _ => "png",
    };
    let dir = AppConfig::local_data_dir().join("attachments");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建附件目录失败: {e}"))?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let seq = PASTE_SEQ.fetch_add(1, Ordering::Relaxed);
    let file_path = dir.join(format!("paste-{ts}-{seq}.{ext}"));
    std::fs::write(&file_path, &bytes).map_err(|e| format!("保存粘贴图片失败: {e}"))?;
    Ok(build_picked_attachment(&file_path))
}

#[tauri::command]
fn app_get_settings(app: AppHandle) -> AppSettingsSnapshot {
    build_settings_snapshot(&app)
}

#[tauri::command]
fn app_update_settings(app: AppHandle, patch: AppSettingsPatch) -> AppSettingsSnapshot {
    if let Some(shared) = app.try_state::<AppShared>() {
        let mut hide_changed: Option<bool> = None;
        let mut sync_changed: Option<bool> = None;
        let mut autostart_changed: Option<bool> = None;
        let mut tray_changed: Option<bool> = None;

        if let Ok(mut cfg) = shared.config.lock() {
            if let Some(v) = patch.output_target {
                cfg.output_target = v;
            }
            if let Some(v) = patch.startup_breath_pulse {
                cfg.startup_breath_pulse = v;
            }
            if let Some(v) = patch.hide_from_screen_capture {
                if cfg.hide_from_screen_capture != v {
                    cfg.hide_from_screen_capture = v;
                    hide_changed = Some(v);
                }
            }
            if let Some(v) = patch.show_tray_icon {
                if cfg.show_tray_icon != v {
                    cfg.show_tray_icon = v;
                    tray_changed = Some(v);
                }
            }
            if let Some(v) = patch.sync_external_plugins {
                if cfg.sync_external_plugins != v {
                    cfg.sync_external_plugins = v;
                    sync_changed = Some(v);
                }
            }
            if let Some(v) = patch.prefer_admin {
                cfg.prefer_admin = v;
            }
            if let Some(v) = patch.autostart {
                if cfg.autostart != v {
                    cfg.autostart = v;
                    autostart_changed = Some(v);
                }
            }
            let _ = cfg.save();
        }

        if let Some(exclude) = hide_changed {
            apply_capture_affinity_to_windows(&app, exclude);
        }
        if let Some(enable_sync) = sync_changed {
            shared.live_sync_enabled.store(enable_sync, Ordering::Release);
            let _ = harness_sync::apply_external_sync_plugins(enable_sync);
        }
        if let Some(enable_auto) = autostart_changed {
            let _ = tray::apply_autostart_setting(enable_auto);
        }
        if let Some(show_tray) = tray_changed {
            if show_tray {
                let needs_create = shared
                    .tray_handles
                    .lock()
                    .map(|g| g.is_none())
                    .unwrap_or(false);
                if needs_create {
                    if let Ok(handles) = tray::setup_tray(&app) {
                        if let Ok(mut slot) = shared.tray_handles.lock() {
                            *slot = Some(handles);
                        }
                        tray::refresh_tray_status(&app);
                    }
                }
            } else {
                if let Some(t) = app.tray_by_id("dsb-tray") {
                    let _ = t.set_visible(false);
                }
            }
        }
    }
    build_settings_snapshot(&app)
}

#[tauri::command]
fn app_settings_action(app: AppHandle, action: String) -> Result<String, String> {
    match action.as_str() {
        "open_config" => {
            let p = AppConfig::config_path();
            if let Some(parent) = p.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::process::Command::new("explorer.exe").arg(&p).spawn();
            Ok("opened_config".to_string())
        }
        "open_captures" => {
            let dir = AppConfig::captures_dir();
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::process::Command::new("explorer.exe").arg(&dir).spawn();
            Ok("opened_captures".to_string())
        }
        "reconnect_dsh" => {
            trigger_dsh_reconnect(app);
            Ok("reconnecting".to_string())
        }
        "preview_breath" => {
            cursor::play_ready_breath_pulse();
            Ok("pulsing".to_string())
        }
        "resync_plugins" => {
            let _ = plugin_sync::ensure_dsh_plugin_synced();
            let enabled = app
                .try_state::<AppShared>()
                .and_then(|s| s.config.lock().ok().map(|c| c.sync_external_plugins))
                .unwrap_or(true);
            let st = harness_sync::apply_external_sync_plugins(enabled);
            serde_json::to_string(&st).map_err(|e| e.to_string())
        }
        "quit" => {
            cursor::restore_system_cursors();
            app.exit(0);
            Ok("exiting".to_string())
        }
        other => Err(format!("未知操作: {other}")),
    }
}

pub fn handle_pipe_control_command(app: &AppHandle, action: &str, _arg: Option<&str>) {
    match action {
        "ping" => {}
        "reload_config" => {
            let latest = AppConfig::load_or_create();
            if let Some(shared) = app.try_state::<AppShared>() {
                shared
                    .live_sync_enabled
                    .store(latest.sync_external_plugins, Ordering::Release);
                apply_capture_affinity_to_windows(app, latest.hide_from_screen_capture);
                if let Ok(mut guard) = shared.config.lock() {
                    *guard = latest;
                }
            }
            if let Some(bar_win) = app.get_webview_window("bar") {
                let _ = bar_win.eval(
                    "(function(){ if (window.__DSB_BRIDGE__ && window.__DSB_BRIDGE__.refreshSettings) { window.__DSB_BRIDGE__.refreshSettings(); } })();",
                );
            }
        }
        "reload_bar" => {
            let _ = plugin_sync::ensure_dsh_plugin_synced();
            if let Some(bar_win) = app.get_webview_window("bar") {
                let _ = bar_win.eval("window.location.reload();");
            }
        }
        "open_settings" => {
            dispatch_state_event(app, StateEvent::DoubleCtrlOpen);
            if let Some(bar_win) = app.get_webview_window("bar") {
                let _ = bar_win.eval(
                    "(function(){ if (window.__DSB_BRIDGE__ && window.__DSB_BRIDGE__.openSettings) { window.__DSB_BRIDGE__.openSettings(); } })();",
                );
            }
        }
        "preview_breath" => {
            cursor::play_ready_breath_pulse();
        }
        "open_captures" => {
            let _ = app_settings_action(app.clone(), "open_captures".to_string());
        }
        "resync_plugins" => {
            let _ = app_settings_action(app.clone(), "resync_plugins".to_string());
        }
        _ => {}
    }
}

#[tauri::command]
fn quick_search_query(app: AppHandle, query: String, limit: Option<usize>) -> Vec<search::SearchHit> {
    let Some(shared) = app.try_state::<AppShared>() else {
        return Vec::new();
    };
    shared.search_engine.query(&query, limit.unwrap_or(8))
}

#[tauri::command]
fn quick_search_status(app: AppHandle) -> search::SearchIndexStatus {
    app.try_state::<AppShared>()
        .map(|s| s.search_engine.status())
        .unwrap_or(search::SearchIndexStatus {
            ready: false,
            indexing: false,
            entry_count: 0,
            last_build_ms: 0,
            engine_mode: "high_value".to_string(),
            drives: Vec::new(),
        })
}

#[tauri::command]
fn quick_search_build_capsule(
    app: AppHandle,
    path: String,
    name: String,
    kind: search::SearchEntryKind,
) -> search::QuickSearchCapsule {
    let n = app
        .try_state::<AppShared>()
        .map(|s| {
            s.search_engine.record_use(&path);
            s.allocate_pick_number()
        })
        .unwrap_or(1);
    search::build_quick_search_capsule(n, &path, &name, kind)
}

#[tauri::command]
fn quick_search_open_path(app: AppHandle, path: String) -> Result<(), String> {
    let Some(shared) = app.try_state::<AppShared>() else {
        return Err("应用状态未就绪".to_string());
    };
    shared.search_engine.record_use(&path);
    search::open_search_path_medium(&shared.broker, &path)
}

#[tauri::command]
fn quick_search_refresh(app: AppHandle) {
    if let Some(shared) = app.try_state::<AppShared>() {
        shared.search_engine.trigger_refresh_async();
    }
}

pub fn dispatch_state_event(app: &AppHandle, event: StateEvent) {
    let Some(shared) = app.try_state::<AppShared>() else {
        return;
    };

    let (fx, vis_snapshot, dock_edge_snapshot) = {
        let Ok(mut sm) = shared.state_machine.lock() else {
            return;
        };
        let fx = sm.apply(event);
        (fx, sm.visibility, sm.dock_edge)
    };

    shared
        .hook_shared
        .set_visibility_tier(vis_snapshot.tier_u8());

    if fx.reset_pick_counter {
        if let Ok(mut st) = shared.pick_counter.lock() {
            st.reset();
        }
    }
    if fx.record_sent_message {
        let cfg = shared.config.lock().map(|c| c.clone()).unwrap_or_default();
        std::thread::spawn(move || {
            captures::rotate_captures_on_send(&cfg);
        });
    }

    if let Some(mode) = fx.bridge_mode {
        if let Ok(mut last_m) = shared.last_bridge_mode.lock() {
            *last_m = mode;
        }
        if let Ok(mut conn) = shared.dsh_conn.lock() {
            conn.desired_initial_mode = match mode {
                BridgeMode::Lip => "lip".to_string(),
                _ => "summon".to_string(),
            };
        }

        if let Some(bar_win) = app.get_webview_window("bar") {
            // Immediately shrink physical window to 56px lip when entering Lip mode so zero invisible region lingers
            if mode == BridgeMode::Lip {
                let width = shared.config.lock().map(|c| c.bar_width).unwrap_or(760);
                let lip_shape = ShapeRectDip {
                    x: 0,
                    y: 0,
                    w: width,
                    h: DEFAULT_LIP_HEIGHT_DIP,
                    r: 24,
                };
                if let Ok(mut cached) = shared.last_layout.lock() {
                    cached.width_dip = width;
                    cached.height_dip = DEFAULT_LIP_HEIGHT_DIP;
                    cached.shapes = vec![lip_shape];
                }
                apply_bar_layout(&bar_win, width, DEFAULT_LIP_HEIGHT_DIP, &[lip_shape]);
            }

            let js = format!(
                "window.__DSB_BRIDGE__ && window.__DSB_BRIDGE__.setMode('{}');",
                mode.as_str()
            );
            let _ = bar_win.eval(&js);
        }
    }

    let vis_js_str = match vis_snapshot {
        VisibilityState::Hidden => "hidden",
        VisibilityState::Lip => "lip",
        VisibilityState::DockedCollapsed => "docked_collapsed",
        VisibilityState::Summon => "summon",
    };

    // Execute WindowAction BEFORE pushing JS dock state so `SLIDE_ANIM_ACTIVE` is armed
    // before the WebView can emit a racing `bar_report(layout)` frame!
    if let Some(win_act) = fx.window_action {
        let bar_width = shared.config.lock().map(|c| c.bar_width).unwrap_or(760);
        if let Some(bar_win) = app.get_webview_window("bar") {
            match win_act {
                WindowAction::Hide => {
                    window::cancel_slide_anim();
                    hide_bar(&bar_win);
                }
                WindowAction::ShowNoActivate => {
                    show_bar_noactivate(&bar_win);
                }
                WindowAction::ActivateAndFocus => {
                    activate_and_focus_bar(&bar_win);
                }
                WindowAction::DockCollapse => {
                    window::slide_docked_bar(&bar_win, dock_edge_snapshot, true, bar_width, false);
                }
                WindowAction::DockExpandNoActivate => {
                    window::slide_docked_bar(&bar_win, dock_edge_snapshot, false, bar_width, false);
                }
                WindowAction::DockExpandAndFocus => {
                    window::slide_docked_bar(&bar_win, dock_edge_snapshot, false, bar_width, true);
                }
            }
        }
        log::info!(
            "Bar visibility -> {:?} (dock={:?}, act={:?})",
            vis_snapshot,
            dock_edge_snapshot,
            win_act
        );
    }

    if fx.window_action.is_some() || fx.bridge_mode.is_some() {
        if let Some(bar_win) = app.get_webview_window("bar") {
            let js = format!(
                "window.__DSB_BRIDGE__ && window.__DSB_BRIDGE__.setVisibility && window.__DSB_BRIDGE__.setVisibility('{vis_js_str}');"
            );
            let _ = bar_win.eval(&js);
        }
        push_dock_state_to_bar(
            app,
            dock_edge_snapshot,
            vis_snapshot == VisibilityState::DockedCollapsed,
        );
    }
}

pub fn trigger_dsh_reconnect(app: AppHandle) {
    std::thread::Builder::new()
        .name("dsb-reconnect".to_string())
        .spawn(move || {
            let Some(shared) = app.try_state::<AppShared>() else {
                return;
            };
            let _ = plugin_sync::ensure_dsh_plugin_synced();
            let cfg = shared.config.lock().map(|c| c.clone()).unwrap_or_default();
            let outcome = discover_or_start_dsh(&cfg, &shared.broker, true);
            apply_discovery_to_bar(&app, &shared.dsh_conn, outcome);
        })
        .ok();
}

fn init_file_logger() {
    let logs_dir = AppConfig::logs_dir();
    let _ = std::fs::create_dir_all(&logs_dir);
    let log_file = logs_dir.join("bar.log");
    if let Ok(file) = OpenOptions::new().create(true).append(true).open(log_file) {
        let _ = simplelog::WriteLogger::init(
            simplelog::LevelFilter::Info,
            simplelog::Config::default(),
            file,
        );
    }
}

struct BgDwellState {
    key: String,
    origin: (i32, i32),
    started_at: std::time::Instant,
    unlocked: bool,
}

const BG_DWELL_UNLOCK_MS: u64 = 480;
const BG_DWELL_JITTER_SQ: i64 = 100; // 10px physical tolerance

fn update_bg_dwell_on_hover(
    dwell: &mut Option<BgDwellState>,
    hov: &pick::HoverInspection,
    x: i32,
    y: i32,
) {
    if hov.kind != pick::HoverTargetKind::Background || hov.target_key.is_empty() {
        *dwell = None;
        return;
    }
    let now = std::time::Instant::now();
    match dwell {
        Some(st) if st.key == hov.target_key => {
            if !st.unlocked {
                let dx = (x - st.origin.0) as i64;
                let dy = (y - st.origin.1) as i64;
                if dx * dx + dy * dy > BG_DWELL_JITTER_SQ {
                    st.origin = (x, y);
                    st.started_at = now;
                } else if st.started_at.elapsed() >= Duration::from_millis(BG_DWELL_UNLOCK_MS) {
                    st.unlocked = true;
                }
            }
        }
        _ => {
            *dwell = Some(BgDwellState {
                key: hov.target_key.clone(),
                origin: (x, y),
                started_at: now,
                unlocked: false,
            });
        }
    }
}

fn is_bg_dwell_unlocked(dwell: &mut Option<BgDwellState>) -> bool {
    if let Some(st) = dwell.as_mut() {
        if !st.unlocked && st.started_at.elapsed() >= Duration::from_millis(BG_DWELL_UNLOCK_MS) {
            st.unlocked = true;
        }
        return st.unlocked;
    }
    false
}

type HoverSlot = Arc<(Mutex<Option<(u64, i32, i32)>>, Condvar)>;

fn start_hook_event_router(app: AppHandle, rx: mpsc::Receiver<HookEvent>) {
    let own_pid = std::process::id();
    let pen_active = Arc::new(AtomicBool::new(false));
    let pen_gen = Arc::new(AtomicU64::new(0));
    let bg_dwell: Arc<Mutex<Option<BgDwellState>>> = Arc::new(Mutex::new(None));
    let hover_slot: HoverSlot = Arc::new((Mutex::new(None), Condvar::new()));

    // Dedicated coalescing UIA hover thread so slow Electron/Chromium UIA queries NEVER block
    // PenEnd, OutsideClick, or DoubleCtrlToggle in the router!
    {
        let uia_app = app.clone();
        let uia_active = pen_active.clone();
        let uia_gen = pen_gen.clone();
        let uia_dwell = bg_dwell.clone();
        let uia_slot = hover_slot.clone();
        std::thread::Builder::new()
            .name("dsb-uia-hover".to_string())
            .spawn(move || {
                let mut cached_uia = uiautomation::UIAutomation::new()
                    .ok()
                    .and_then(|auto| auto.get_control_view_walker().ok().map(|w| (auto, w)));
                let mut last_query_at = std::time::Instant::now() - Duration::from_secs(1);

                loop {
                    let req = {
                        let (lock, cv) = &*uia_slot;
                        let Ok(mut guard) = lock.lock() else {
                            break;
                        };
                        while guard.is_none() {
                            match cv.wait(guard) {
                                Ok(g) => guard = g,
                                Err(_) => return,
                            }
                        }
                        guard.take()
                    };

                    let Some((req_gen, x, y)) = req else {
                        continue;
                    };
                    if !uia_active.load(Ordering::Acquire)
                        || uia_gen.load(Ordering::Acquire) != req_gen
                    {
                        continue;
                    }

                    let since_last = last_query_at.elapsed();
                    if since_last < Duration::from_millis(55) {
                        std::thread::sleep(Duration::from_millis(55) - since_last);
                        // Re-check if a newer position arrived while sleeping
                        if let Ok(guard) = uia_slot.0.lock() {
                            if guard.is_some() {
                                continue;
                            }
                        }
                        if !uia_active.load(Ordering::Acquire)
                            || uia_gen.load(Ordering::Acquire) != req_gen
                        {
                            continue;
                        }
                    }

                    if cached_uia.is_none() {
                        cached_uia = uiautomation::UIAutomation::new().ok().and_then(|auto| {
                            auto.get_control_view_walker().ok().map(|w| (auto, w))
                        });
                    }

                    last_query_at = std::time::Instant::now();
                    let hov = pick::inspect_hover_at_with_context(
                        Some(&uia_app),
                        cached_uia.as_ref().map(|(a, w)| (a, w)),
                        x,
                        y,
                        own_pid,
                    );

                    // Drop stale hover results if Ctrl was released while UIA was querying
                    if !uia_active.load(Ordering::Acquire)
                        || uia_gen.load(Ordering::Acquire) != req_gen
                    {
                        continue;
                    }

                    if let Ok(mut d) = uia_dwell.lock() {
                        update_bg_dwell_on_hover(&mut d, &hov, x, y);
                    }
                    if let Some(ov) = uia_app.get_webview_window("overlay") {
                        let _ = ov.emit("pen:hover", hov);
                    }
                }
            })
            .expect("failed to start UIA hover worker");
    }

    type WordSelRequest = (isize, (i32, i32), (i32, i32));
    let word_sel_slot: Arc<(Mutex<Option<WordSelRequest>>, std::sync::Condvar)> =
        Arc::new((Mutex::new(None), std::sync::Condvar::new()));

    {
        let word_slot = word_sel_slot.clone();
        let word_pen_active = pen_active.clone();
        std::thread::Builder::new()
            .name("dsb-word-text-sel".to_string())
            .spawn(move || {
                let (lock, cv) = &*word_slot;
                loop {
                    let req = {
                        let mut guard = match lock.lock() {
                            Ok(g) => g,
                            Err(_) => break,
                        };
                        while guard.is_none() {
                            guard = match cv.wait(guard) {
                                Ok(g) => g,
                                Err(_) => return,
                            };
                        }
                        guard.take()
                    };
                    let Some((hwnd_raw, start, end)) = req else {
                        continue;
                    };
                    if !word_pen_active.load(Ordering::Acquire) || hwnd_raw == 0 {
                        continue;
                    }
                    let hwnd = windows::Win32::Foundation::HWND(hwnd_raw as *mut std::ffi::c_void);
                    let _ = pick::adapters::office::select_word_text_uia(hwnd, start, end, true);
                    std::thread::sleep(Duration::from_millis(28));
                }
            })
            .expect("failed to start Word text-selection worker");
    }

    std::thread::Builder::new()
        .name("dsb-hook-router".to_string())
        .spawn(move || {
            let mut pending_raw_capture: Option<
                std::thread::JoinHandle<Option<captures::RawScreenCapture>>,
            > = None;
            let mut pending_tap_allow_bg = false;
            let mut active_drag_mode = pick::adapters::office::OfficeDragMode::NormalStroke;
            let mut active_drag_hwnd: isize = 0;
            let mut hover_ibeam_active = false;
            let mut last_office_hover_check = std::time::Instant::now() - Duration::from_secs(1);

            let push_hover = |req_gen: u64, x: i32, y: i32| {
                let (lock, cv) = &*hover_slot;
                if let Ok(mut guard) = lock.lock() {
                    *guard = Some((req_gen, x, y));
                    cv.notify_one();
                }
            };

            let push_word_sel = |hwnd_raw: isize, start: (i32, i32), end: (i32, i32)| {
                let (lock, cv) = &*word_sel_slot;
                if let Ok(mut guard) = lock.lock() {
                    *guard = Some((hwnd_raw, start, end));
                    cv.notify_one();
                }
            };

            // Hoisted once outside the event loop — zero per-hover Arc allocation!
            let scout_refresh_cb: Arc<dyn Fn(i32, i32) + Send + Sync> = {
                let hs = hover_slot.clone();
                let pg = pen_gen.clone();
                let pa = pen_active.clone();
                Arc::new(move |rx: i32, ry: i32| {
                    if !pa.load(Ordering::Acquire) {
                        return;
                    }
                    let g = pg.load(Ordering::Acquire);
                    let (lock, cv) = &*hs;
                    if let Ok(mut guard) = lock.lock() {
                        *guard = Some((g, rx, ry));
                        cv.notify_one();
                    }
                })
            };

            while let Ok(ev) = rx.recv() {
                match ev {
                    HookEvent::SingleCtrlClose => {
                        log::info!("SingleCtrlClose received -> hiding Bar");
                        dispatch_state_event(&app, StateEvent::SingleCtrlClose);
                    }
                    HookEvent::DoubleCtrlOpen { foreground_hwnd } => {
                        let was_summon = app
                            .try_state::<AppShared>()
                            .and_then(|s| s.state_machine.lock().ok().map(|sm| sm.visibility))
                            == Some(VisibilityState::Summon);
                        if !was_summon && foreground_hwnd != 0 {
                            if let Some(info) = universal::capture_target_window(foreground_hwnd) {
                                if let Some(shared) = app.try_state::<AppShared>() {
                                    if let Ok(mut slot) = shared.last_target_window.lock() {
                                        *slot = Some(info.clone());
                                    }
                                }
                                if let Some(bar_win) = app.get_webview_window("bar") {
                                    if let Ok(json) = serde_json::to_string(&info) {
                                        let js = format!(
                                            "(function(){{ if (typeof window.__DSB_TARGET_WINDOW__ === 'function') {{ window.__DSB_TARGET_WINDOW__({json}); }} }})();"
                                        );
                                        let _ = bar_win.eval(&js);
                                    }
                                }
                            }
                        }
                        dispatch_state_event(&app, StateEvent::DoubleCtrlOpen);
                        if !was_summon && foreground_hwnd != 0 {
                            let auto_app = app.clone();
                            std::thread::spawn(move || {
                                pick::maybe_auto_attach_explorer_selection(
                                    &auto_app,
                                    foreground_hwnd,
                                );
                                pick::maybe_auto_attach_solidworks_selection(
                                    &auto_app,
                                    foreground_hwnd,
                                );
                            });
                        }
                    }
                    HookEvent::OutsideClick { x, y } => {
                        if window::is_bar_dragging() {
                            continue;
                        }
                        let click_pt = windows::Win32::Foundation::POINT { x, y };
                        if window::is_ime_or_voice_helper_at_point(click_pt) {
                            log::info!(
                                "OutsideClick at ({x}, {y}) hit IME/voice helper popup -> keeping Bar open"
                            );
                            continue;
                        }
                        log::info!("OutsideClick received -> hiding Bar");
                        let was_vis = app
                            .try_state::<AppShared>()
                            .and_then(|s| s.state_machine.lock().ok().map(|sm| sm.is_visible()))
                            .unwrap_or(false);
                        dispatch_state_event(&app, StateEvent::OutsideClickHide);
                        if was_vis {
                            if let Some(shared) = app.try_state::<AppShared>() {
                                if let Ok(mut slot) = shared.last_outside_hide_at.lock() {
                                    *slot = Some(std::time::Instant::now());
                                }
                            }
                        }
                    }
                    HookEvent::PenStart { cursor_x, cursor_y } => {
                        let gen = pen_gen.fetch_add(1, Ordering::AcqRel) + 1;
                        pen_active.store(true, Ordering::Release);
                        log::info!("PenStart (gen={gen}) at ({cursor_x}, {cursor_y})");
                        pending_raw_capture = None;
                        pending_tap_allow_bg = false;
                        active_drag_mode = pick::adapters::office::OfficeDragMode::NormalStroke;
                        active_drag_hwnd = 0;
                        hover_ibeam_active = false;
                        if let Ok(mut d) = bg_dwell.lock() {
                            *d = None;
                        }
                        // Capture SolidWorks target HWND before Bar activates so double-tap-hold Ctrl
                        // immediately ingests any pre-selected SolidWorks features/faces/edges.
                        let sw_auto_hwnd: isize = pick::adapters::solidworks::detect_solidworks_at(
                            cursor_x, cursor_y,
                        )
                        .map(|h| h.root_hwnd.0 as isize)
                        .or_else(|| {
                            let fg = unsafe {
                                windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow()
                            };
                            pick::adapters::solidworks::detect_solidworks_hwnd(fg)
                                .map(|h| h.root_hwnd.0 as isize)
                        })
                        .unwrap_or(0);
                        dispatch_state_event(&app, StateEvent::PenStarted);
                        if sw_auto_hwnd != 0 {
                            let auto_app = app.clone();
                            std::thread::spawn(move || {
                                pick::maybe_auto_attach_solidworks_selection(
                                    &auto_app,
                                    sw_auto_hwnd,
                                );
                            });
                        }
                        cursor::enable_tech_cursor();
                        if let Some(hit) =
                            pick::adapters::office::detect_office_canvas_at(cursor_x, cursor_y)
                        {
                            if hit.in_canvas
                                && hit.kind == pick::adapters::office::OfficeAppKind::Word
                            {
                                cursor::set_ibeam_override(true);
                                hover_ibeam_active = true;
                            }
                        }
                        pick::scout::on_session_start(cursor_x, cursor_y);
                        if let Some(ov) = app.get_webview_window("overlay") {
                            let (mon, scale) = position_and_show_overlay(&ov);
                            let _ = ov.emit(
                                "pen:show",
                                serde_json::json!({
                                    "monitorRect": {
                                        "x": mon.monitor_left,
                                        "y": mon.monitor_top,
                                        "w": mon.monitor_right - mon.monitor_left,
                                        "h": mon.monitor_bottom - mon.monitor_top,
                                    },
                                    "scale": scale,
                                    "cursor": { "x": cursor_x, "y": cursor_y },
                                }),
                            );
                        }
                        push_hover(gen, cursor_x, cursor_y);
                    }
                    HookEvent::PenHover { x, y } => {
                        if !pen_active.load(Ordering::Acquire) {
                            continue;
                        }
                        let cur_gen = pen_gen.load(Ordering::Acquire);
                        if last_office_hover_check.elapsed() >= Duration::from_millis(60) {
                            last_office_hover_check = std::time::Instant::now();
                            let want_ibeam = pick::adapters::office::detect_office_canvas_at(x, y)
                                .map(|h| {
                                    h.in_canvas
                                        && h.kind == pick::adapters::office::OfficeAppKind::Word
                                })
                                .unwrap_or(false);
                            if want_ibeam != hover_ibeam_active {
                                cursor::set_ibeam_override(want_ibeam);
                                hover_ibeam_active = want_ibeam;
                            }
                        }
                        if let Ok(mut d) = bg_dwell.lock() {
                            if let Some(st) = d.as_mut() {
                                if !st.unlocked {
                                    let dx = (x - st.origin.0) as i64;
                                    let dy = (y - st.origin.1) as i64;
                                    if dx * dx + dy * dy > BG_DWELL_JITTER_SQ {
                                        st.origin = (x, y);
                                        st.started_at = std::time::Instant::now();
                                    } else if st.started_at.elapsed()
                                        >= Duration::from_millis(BG_DWELL_UNLOCK_MS)
                                    {
                                        st.unlocked = true;
                                    }
                                }
                            }
                        }
                        if let Some(ov) = app.get_webview_window("overlay") {
                            let _ = ov.emit("pen:cursor", serde_json::json!({ "x": x, "y": y }));
                        }
                        pick::scout::on_hover_point(&app, x, y, own_pid, scout_refresh_cb.clone());
                        push_hover(cur_gen, x, y);
                    }
                    HookEvent::PenMouseDown { x, y, button } => {
                        pending_tap_allow_bg = bg_dwell
                            .lock()
                            .map(|mut d| is_bg_dwell_unlocked(&mut d))
                            .unwrap_or(false);
                        // Spawn pre-drag snapshot asynchronously so PenMouseDown returns in <0.1ms
                        // and the first PenDrag frames paint on the overlay with 0ms capture lag!
                        pending_raw_capture = Some(std::thread::spawn(move || {
                            pick::capture_mousedown_snapshot(x, y)
                        }));
                        // Right button ALWAYS draws a red doodle screenshot (never hijacked by Office text/cell drag).
                        if button == hooks::PenButton::Right {
                            active_drag_mode = pick::adapters::office::OfficeDragMode::NormalStroke;
                            active_drag_hwnd = 0;
                        } else if let Some(hit) = pick::adapters::office::detect_office_canvas_at(x, y) {
                            if hit.in_canvas {
                                active_drag_mode = hit.kind.drag_mode();
                                active_drag_hwnd = hit.root_hwnd.0 as isize;
                                if active_drag_mode
                                    == pick::adapters::office::OfficeDragMode::WordTextSelect
                                {
                                    cursor::set_ibeam_override(true);
                                    hover_ibeam_active = true;
                                }
                            } else {
                                active_drag_mode =
                                    pick::adapters::office::OfficeDragMode::NormalStroke;
                                active_drag_hwnd = 0;
                            }
                        } else {
                            active_drag_mode = pick::adapters::office::OfficeDragMode::NormalStroke;
                            active_drag_hwnd = 0;
                        }
                    }
                    HookEvent::PenDrag { points, button } => {
                        if points.len() < 2 {
                            continue;
                        }
                        let first = points[0];
                        let last = *points.last().unwrap_or(&first);
                        if button == hooks::PenButton::Right {
                            if let Some(ov) = app.get_webview_window("overlay") {
                                let pts: Vec<serde_json::Value> = points
                                    .into_iter()
                                    .map(|(x, y)| serde_json::json!({ "x": x, "y": y }))
                                    .collect();
                                let _ = ov.emit(
                                    "pen:stroke",
                                    serde_json::json!({
                                        "mode": "red_doodle",
                                        "color": "red",
                                        "points": pts,
                                    }),
                                );
                            }
                        } else {
                            match active_drag_mode {
                                pick::adapters::office::OfficeDragMode::WordTextSelect => {
                                    push_word_sel(active_drag_hwnd, first, last);
                                    if let Some(ov) = app.get_webview_window("overlay") {
                                        let _ = ov.emit(
                                            "pen:stroke",
                                            serde_json::json!({
                                                "mode": "word_text",
                                                "start": { "x": first.0, "y": first.1 },
                                                "end": { "x": last.0, "y": last.1 },
                                            }),
                                        );
                                    }
                                }
                                pick::adapters::office::OfficeDragMode::ExcelCellBox => {
                                    if let Some(ov) = app.get_webview_window("overlay") {
                                        let _ = ov.emit(
                                            "pen:stroke",
                                            serde_json::json!({
                                                "mode": "excel_box",
                                                "start": { "x": first.0, "y": first.1 },
                                                "end": { "x": last.0, "y": last.1 },
                                            }),
                                        );
                                    }
                                }
                                pick::adapters::office::OfficeDragMode::NormalStroke => {
                                    if let Some(ov) = app.get_webview_window("overlay") {
                                        let pts: Vec<serde_json::Value> = points
                                            .into_iter()
                                            .map(|(x, y)| serde_json::json!({ "x": x, "y": y }))
                                            .collect();
                                        let _ = ov.emit(
                                            "pen:stroke",
                                            serde_json::json!({
                                                "mode": "blue_select",
                                                "color": "blue",
                                                "points": pts,
                                            }),
                                        );
                                    }
                                }
                            }
                        }
                    }
                    HookEvent::PenTap { x, y } => {
                        let pre_handle = pending_raw_capture.take();
                        let allow_bg = pending_tap_allow_bg;
                        pending_tap_allow_bg = false;
                        active_drag_mode = pick::adapters::office::OfficeDragMode::NormalStroke;
                        active_drag_hwnd = 0;
                        let tap_app = app.clone();
                        std::thread::spawn(move || {
                            let pre = pre_handle.and_then(|h| h.join().ok().flatten());
                            pick::execute_tap_pick(&tap_app, x, y, pre, allow_bg);
                        });
                    }
                    HookEvent::PenStroke { points, button } => {
                        let pre_handle = pending_raw_capture.take();
                        let need_pre = button == hooks::PenButton::Right
                            || active_drag_mode
                                != pick::adapters::office::OfficeDragMode::NormalStroke;
                        pending_tap_allow_bg = false;
                        active_drag_mode = pick::adapters::office::OfficeDragMode::NormalStroke;
                        active_drag_hwnd = 0;
                        if let Ok(mut guard) = word_sel_slot.0.lock() {
                            *guard = None;
                        }
                        let stroke_app = app.clone();
                        std::thread::spawn(move || {
                            let pre = if need_pre {
                                pre_handle.and_then(|h| h.join().ok().flatten())
                            } else {
                                None
                            };
                            pick::execute_stroke_pick(&stroke_app, points, button, pre);
                        });
                    }
                    HookEvent::PenWheelScroll => {
                        pick::scout::on_scroll_invalidate(&app);
                    }
                    HookEvent::ScanAbort => {
                        pick::brush_scan::trigger_abort();
                    }
                    HookEvent::PenEnd {
                        picked_any,
                        cancelled_by_other_key,
                    } => {
                        pick::scout::on_session_end();
                        pick::hover_diff::on_selection_session_end();
                        pen_active.store(false, Ordering::Release);
                        pen_gen.fetch_add(1, Ordering::AcqRel);
                        log::info!(
                            "PenEnd (picked_any={picked_any}, cancelled_by_other_key={cancelled_by_other_key})"
                        );
                        pending_raw_capture = None;
                        pending_tap_allow_bg = false;
                        active_drag_mode = pick::adapters::office::OfficeDragMode::NormalStroke;
                        active_drag_hwnd = 0;
                        hover_ibeam_active = false;
                        if let Ok(mut d) = bg_dwell.lock() {
                            *d = None;
                        }
                        if let Ok(mut guard) = hover_slot.0.lock() {
                            *guard = None;
                        }
                        if let Ok(mut guard) = word_sel_slot.0.lock() {
                            *guard = None;
                        }
                        cursor::restore_system_cursors();
                        if let Some(ov) = app.get_webview_window("overlay") {
                            let _ = ov.emit("pen:hide", serde_json::json!({}));
                            hide_overlay(&ov);
                        }
                        dispatch_state_event(
                            &app,
                            StateEvent::PenEnded {
                                picked_any,
                                cancelled_by_other_key,
                            },
                        );
                    }
                }
            }
        })
        .expect("failed to start hook event router");
}

fn start_dock_watcher(app: AppHandle) {
    let _ = std::thread::Builder::new()
        .name("atp-dock-watcher".to_string())
        .spawn(move || {
            let mut hover_enter_at: Option<std::time::Instant> = None;
            let mut leave_start_at: Option<std::time::Instant> = None;
            let mut fs_check_tick: u8 = 0;

            loop {
                let state_snap = app.try_state::<AppShared>().and_then(|s| {
                    s.state_machine
                        .lock()
                        .ok()
                        .map(|sm| (sm.dock_edge, sm.visibility, sm.dock_locked_open, sm.pinned, sm.popup_open, sm.modal, sm.composer_focused, sm.pen_active))
                });
                let Some((dock_edge, vis, locked_open, pinned, popup_open, modal, composer_focused, pen_active)) = state_snap else {
                    std::thread::sleep(Duration::from_millis(250));
                    continue;
                };

                if !dock_edge.is_docked() {
                    hover_enter_at = None;
                    leave_start_at = None;
                    std::thread::sleep(Duration::from_millis(260));
                    continue;
                }

                let Some(bar_win) = app.get_webview_window("bar") else {
                    std::thread::sleep(Duration::from_millis(200));
                    continue;
                };

                // Periodic fullscreen detection (~400ms cadence) to auto-hide 10px edge strip over videos/games
                fs_check_tick = fs_check_tick.wrapping_add(1);
                if fs_check_tick.is_multiple_of(8) {
                    let is_fs = window::is_fullscreen_on_dock_monitor(hwnd_of(&bar_win));
                    dispatch_state_event(&app, StateEvent::FullscreenChanged { active: is_fs });
                }

                match vis {
                    VisibilityState::DockedCollapsed => {
                        leave_start_at = None;
                        if window::is_cursor_in_dock_peek_zone(&bar_win) {
                            let started = *hover_enter_at.get_or_insert_with(std::time::Instant::now);
                            if started.elapsed() >= Duration::from_millis(120) {
                                hover_enter_at = None;
                                dispatch_state_event(&app, StateEvent::DockEdgePeekEnter);
                            }
                        } else {
                            hover_enter_at = None;
                        }
                    }
                    VisibilityState::Summon => {
                        hover_enter_at = None;
                        if !locked_open
                            && !pinned
                            && !popup_open
                            && !modal
                            && !composer_focused
                            && !pen_active
                            && !window::is_bar_dragging()
                            && !window::is_slide_anim_active()
                        {
                            if window::is_cursor_inside_bar_now(&bar_win) {
                                leave_start_at = None;
                            } else {
                                let left = *leave_start_at.get_or_insert_with(std::time::Instant::now);
                                if left.elapsed() >= Duration::from_millis(400) {
                                    leave_start_at = None;
                                    dispatch_state_event(&app, StateEvent::DockAutoCollapseIfUnlocked);
                                }
                            }
                        } else {
                            leave_start_at = None;
                        }
                    }
                    _ => {
                        hover_enter_at = None;
                        leave_start_at = None;
                    }
                }

                std::thread::sleep(Duration::from_millis(50));
            }
        });
}

fn start_dsh_health_monitor(app: AppHandle) {
    std::thread::Builder::new()
        .name("dsb-dsh-health".to_string())
        .spawn(move || {
            // Initial discovery
            if let Some(shared) = app.try_state::<AppShared>() {
                let cfg = shared.config.lock().map(|c| c.clone()).unwrap_or_default();
                let outcome = discover_or_start_dsh(&cfg, &shared.broker, true);
                apply_discovery_to_bar(&app, &shared.dsh_conn, outcome);
            }

            loop {
                let active_port = app
                    .try_state::<AppShared>()
                    .and_then(|s| s.dsh_conn.lock().ok().and_then(|c| c.active_port));
                let sleep_secs = if active_port.is_some() { 5 } else { 2 };
                std::thread::sleep(Duration::from_secs(sleep_secs));

                let Some(shared) = app.try_state::<AppShared>() else {
                    break;
                };
                let active_port = shared.dsh_conn.lock().ok().and_then(|s| s.active_port);

                if let Some(port) = active_port {
                    if !is_port_listening(port, 300) {
                        log::warn!("DSH port {port} stopped responding; attempting re-discovery.");
                        let cfg = shared.config.lock().map(|c| c.clone()).unwrap_or_default();
                        let outcome = discover_or_start_dsh(&cfg, &shared.broker, false);
                        apply_discovery_to_bar(&app, &shared.dsh_conn, outcome);
                    }
                } else {
                    let cfg = shared.config.lock().map(|c| c.clone()).unwrap_or_default();
                    let dsh_home = AppConfig::dsh_home_dir();
                    if let Some(inst) = dsh::scan_live_instances(&dsh_home, cfg.preferred_port) {
                        log::info!(
                            "Detected DSH instance on port {} after plugin sync/restart; auto-connecting.",
                            inst.port
                        );
                        apply_discovery_to_bar(
                            &app,
                            &shared.dsh_conn,
                            dsh::DiscoveryOutcome::Connected {
                                port: inst.port,
                                launch_url: inst.launch_url,
                            },
                        );
                    }
                }
            }
        })
        .ok();
}

pub fn run() {
    let cli = CliOptions::parse(std::env::args());

    // 0. Ultra-fast (<5ms) synchronous hook relay — exits before logger/UAC/Tauri
    if let Some((ref harness, ref inline)) = cli.hook_event {
        harness_sync::run_hook_event_cli(harness, inline.as_deref());
        return;
    }

    // 0b. Native stdio MCP sidecar loaded by Antigravity / Claude Code / Codex plugins
    if cli.mcp_sidecar {
        mcp_sidecar::run_mcp_sidecar(cli.mcp_harness.as_deref());
        return;
    }

    // 0b2. Official Antigravity Plugin Sidecar for silent background `agentapi` messaging
    if cli.antigravity_sidecar {
        harness_sync::run_antigravity_sidecar_cli();
        return;
    }

    // 0b3. Universal read-only UI capability probe CLI (`--probe-window <exe|hwnd>`)
    if let Some(ref target) = cli.probe_window {
        pick::probe::run_probe_window_cli(target, cli.probe_out.as_deref());
        return;
    }

    // 0c. One-shot plugin registration / removal CLI flags (used by installers)
    if cli.install_plugins {
        let _ = plugin_sync::ensure_dsh_plugin_synced();
        let st = harness_sync::apply_external_sync_plugins(true);
        println!("{}", serde_json::to_string_pretty(&st).unwrap_or_default());
        return;
    }
    if cli.uninstall_plugins {
        let st = harness_sync::apply_external_sync_plugins(false);
        println!("{}", serde_json::to_string_pretty(&st).unwrap_or_default());
        return;
    }

    // 1. Medium-integrity broker child mode exits before UI/single-instance
    if let Some(pipe_name) = cli.broker_pipe.as_deref() {
        let _ = broker::run_broker_client(pipe_name);
        return;
    }

    init_file_logger();

    // Always restore Windows system cursors if the process ever panics while Alt pen mode is active
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        cursor::restore_system_cursors();
        prev_hook(info);
    }));

    let config = AppConfig::load_or_create();

    // 2. Startup UAC elevation check (before single-instance registration!)
    let is_control_invocation = cli.dev_state.is_some()
        || cli.dev_dump
        || cli.dev_pick_tap.is_some()
        || cli.dev_pick_stroke.is_some();
    if config.prefer_admin
        && !cli.no_elevate
        && !cli.elevated_relaunch
        && !is_control_invocation
        && !elevation::is_elevated()
        && elevation::try_relaunch_elevated(&cli.passthrough_args)
    {
        return;
    }

    // Allow medium-integrity second instances to forward CLI args via WM_COPYDATA when elevated
    unsafe {
        let _ = ChangeWindowMessageFilter(WM_COPYDATA, MSGFLT_ADD);
    }

    log::info!(
        "Starting Anything-to-Prompt v{} (pid={}, elevated={})",
        env!("CARGO_PKG_VERSION"),
        std::process::id(),
        elevation::is_elevated()
    );

    // When the user directly double-clicks the EXE, automatically detect and terminate any
    // residual/stuck Bar, broker, or orphaned sidecar processes before acquiring single-instance.
    if cli.is_direct_exe_launch() {
        let report = process_guard::cleanup_stale_processes_on_direct_launch();
        if report.total_killed() > 0 {
            cursor::restore_system_cursors();
            log::info!(
                "Direct EXE launch cleaned {} residual process(es) (bar={}, broker={}, orphan_sidecar={}, webview={}; kept_live_sidecars={})",
                report.total_killed(),
                report.killed_bar,
                report.killed_broker,
                report.killed_orphan_sidecar,
                report.killed_webview,
                report.kept_live_sidecar,
            );
        }
    }

    // Ensure our single built-in DSH companion plugin (@local/anything-to-prompt) is synced into ~/.dsh/profiles/web
    let _ = plugin_sync::ensure_dsh_plugin_synced();

    let hook_shared = HookSharedConfig::new(
        config.accept_injected_input,
        config.double_tap_ms,
        config.ctrl_hold_ms,
    );
    let initial_width = config.bar_width;
    let exclude_capture = config.hide_from_screen_capture && !cli.dev_allow_capture;
    let startup_breath_pulse = config.startup_breath_pulse;
    let show_tray_icon = config.show_tray_icon;
    let sync_external_plugins = config.sync_external_plugins;
    let live_sync_enabled = Arc::new(AtomicBool::new(sync_external_plugins));
    let live_sync_for_setup = live_sync_enabled.clone();

    let initial_mode_str = match cli.dev_state {
        Some(DevTargetState::Lip) | Some(DevTargetState::Hidden) => "lip",
        _ => "summon",
    };

    let dsh_conn = Arc::new(Mutex::new(DshConnectionState {
        active_port: None,
        active_origin: None,
        desired_initial_mode: initial_mode_str.to_string(),
        status_label: "正在检测…".to_string(),
    }));

    let search_engine = Arc::new(search::SearchEngine::new(config.search_roots.clone()));
    let search_engine_for_setup = search_engine.clone();

    let app_shared = AppShared {
        config: Mutex::new(config),
        state_machine: Mutex::new(BarStateMachine::new()),
        last_bridge_mode: Mutex::new(BridgeMode::Summon),
        last_layout: Mutex::new(CachedLayout {
            width_dip: initial_width,
            height_dip: DEFAULT_LIP_HEIGHT_DIP,
            shapes: vec![ShapeRectDip {
                x: 0,
                y: 0,
                w: initial_width,
                h: DEFAULT_LIP_HEIGHT_DIP,
                r: 24,
            }],
        }),
        dsh_conn: dsh_conn.clone(),
        tray_handles: Mutex::new(None),
        broker: Arc::new(BrokerClient::new()),
        search_engine,
        hook_shared: hook_shared.clone(),
        pick_counter: Mutex::new(PickCounterState::default()),
        compose_waiters: Mutex::new(HashMap::new()),
        last_outside_hide_at: Mutex::new(None),
        live_sync_enabled,
        last_target_window: Mutex::new(None),
        attached_sw_entities: Mutex::new(std::collections::HashSet::new()),
    };

    let startup_dev_state = cli.dev_state;
    let startup_dev_dump = cli.dev_dump;

    tauri::Builder::default()
        .manage(app_shared)
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            let forwarded = CliOptions::parse(argv);
            if let Some(target) = forwarded.dev_state {
                log::info!("Single-instance forwarded --dev-state {:?}", target);
                dispatch_state_event(app, StateEvent::DevForceState(target));
            } else if forwarded.dev_single_ctrl_close {
                log::info!("Single-instance forwarded --dev-single-ctrl-close");
                dispatch_state_event(app, StateEvent::SingleCtrlClose);
            } else if let Some(ref draft_text) = forwarded.dev_draft {
                if let Some(bar_win) = app.get_webview_window("bar") {
                    if let Ok(json_txt) = serde_json::to_string(draft_text) {
                        let js = format!(
                            "(function(){{ const b = window.__DSB_BRIDGE__; if (b && typeof b.setDraft === 'function') {{ b.setDraft({json_txt}); }} }})();"
                        );
                        let _ = bar_win.eval(&js);
                    }
                }
            } else if forwarded.ensure_running {
                // Silent companion wake-up from a Harness plugin — already running, never steal focus!
                log::info!(
                    "Single-instance received --ensure-running from {:?}; staying quietly armed.",
                    forwarded.launched_by
                );
            } else if !forwarded.dev_dump
                && forwarded.dev_pick_tap.is_none()
                && forwarded.dev_pick_stroke.is_none()
                && forwarded.dev_pick_lasso.is_none()
            {
                dispatch_state_event(app, StateEvent::DoubleCtrlOpen);
            }
            if let Some((tx, ty)) = forwarded.dev_pick_tap {
                let handle = app.clone();
                std::thread::spawn(move || {
                    if let Some(ov) = handle.get_webview_window("overlay") {
                        let (mon, scale) = position_and_show_overlay(&ov);
                        let _ = ov.emit(
                            "pen:show",
                            serde_json::json!({
                                "monitorRect": {
                                    "x": mon.monitor_left,
                                    "y": mon.monitor_top,
                                    "w": mon.monitor_right - mon.monitor_left,
                                    "h": mon.monitor_bottom - mon.monitor_top,
                                },
                                "scale": scale,
                            }),
                        );
                    }
                    let pre = pick::capture_mousedown_snapshot(tx, ty);
                    pick::execute_tap_pick(&handle, tx, ty, pre, false);
                    if let Some(ov) = handle.get_webview_window("overlay") {
                        let _ = ov.emit("pen:hide", serde_json::json!({}));
                        hide_overlay(&ov);
                    }
                    dispatch_state_event(
                        &handle,
                        StateEvent::PenEnded {
                            picked_any: true,
                            cancelled_by_other_key: false,
                        },
                    );
                });
            }
            if let Some((x1, y1, x2, y2)) = forwarded.dev_pick_stroke {
                let handle = app.clone();
                std::thread::spawn(move || {
                    if let Some(ov) = handle.get_webview_window("overlay") {
                        let (mon, scale) = position_and_show_overlay(&ov);
                        let _ = ov.emit(
                            "pen:show",
                            serde_json::json!({
                                "monitorRect": {
                                    "x": mon.monitor_left,
                                    "y": mon.monitor_top,
                                    "w": mon.monitor_right - mon.monitor_left,
                                    "h": mon.monitor_bottom - mon.monitor_top,
                                },
                                "scale": scale,
                            }),
                        );
                    }
                    let pre = pick::capture_mousedown_snapshot(x1, y1);
                    let pts = vec![(x1, y1), (x2, y1), (x2, y2), (x1, y2), (x1, y1)];
                    pick::execute_stroke_pick(&handle, pts, hooks::PenButton::Right, pre);
                    if let Some(ov) = handle.get_webview_window("overlay") {
                        let _ = ov.emit("pen:hide", serde_json::json!({}));
                        hide_overlay(&ov);
                    }
                    dispatch_state_event(
                        &handle,
                        StateEvent::PenEnded {
                            picked_any: true,
                            cancelled_by_other_key: false,
                        },
                    );
                });
            }
            if let Some(lasso_pts) = forwarded.dev_pick_lasso {
                let handle = app.clone();
                std::thread::spawn(move || {
                    if let Some(ov) = handle.get_webview_window("overlay") {
                        let (mon, scale) = position_and_show_overlay(&ov);
                        let _ = ov.emit(
                            "pen:show",
                            serde_json::json!({
                                "monitorRect": {
                                    "x": mon.monitor_left,
                                    "y": mon.monitor_top,
                                    "w": mon.monitor_right - mon.monitor_left,
                                    "h": mon.monitor_bottom - mon.monitor_top,
                                },
                                "scale": scale,
                            }),
                        );
                    }
                    let (sx, sy) = lasso_pts[0];
                    let pre = pick::capture_mousedown_snapshot(sx, sy);
                    pick::execute_stroke_pick(&handle, lasso_pts, hooks::PenButton::Left, pre);
                    if let Some(ov) = handle.get_webview_window("overlay") {
                        let _ = ov.emit("pen:hide", serde_json::json!({}));
                        hide_overlay(&ov);
                    }
                    dispatch_state_event(
                        &handle,
                        StateEvent::PenEnded {
                            picked_any: true,
                            cancelled_by_other_key: false,
                        },
                    );
                });
            }
            if forwarded.dev_dump {
                let handle = app.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(280));
                    if let Some(bar_win) = handle.get_webview_window("bar") {
                        log_window_diagnostics(&bar_win, "forwarded");
                    }
                });
            }
        }))
        .on_page_load(|webview, payload| {
            if webview.label() != "bar" || payload.event() != PageLoadEvent::Finished {
                return;
            }
            let app = webview.app_handle();
            let Some(shared) = app.try_state::<AppShared>() else {
                return;
            };

            let (active_origin, desired_mode) = shared
                .dsh_conn
                .lock()
                .map(|c| (c.active_origin.clone(), c.desired_initial_mode.clone()))
                .unwrap_or((None, "summon".to_string()));

            if let Some(origin) = active_origin {
                if payload.url().as_str().starts_with(&origin) {
                    if let Some(next_url) = build_post_auth_bar_url(payload.url(), &desired_mode) {
                        log::info!("Auth cookie established; redirecting Bar to {}", next_url);
                        let _ = webview.navigate(next_url);
                        return;
                    }

                    // Sync current BridgeMode & active Harness once DSH Bar clone page finishes loading
                    log::info!("Bar clone page loaded: {}", payload.url());
                    let mode = shared
                        .last_bridge_mode
                        .lock()
                        .map(|m| *m)
                        .unwrap_or(BridgeMode::Summon);
                    let active_harness = shared
                        .config
                        .lock()
                        .map(|c| c.active_harness.clone())
                        .unwrap_or_else(|_| "dsh".to_string());
                    let (vis, dock_edge) = shared
                        .state_machine
                        .lock()
                        .map(|sm| (sm.visibility, sm.dock_edge))
                        .unwrap_or((VisibilityState::Hidden, config::DockEdge::None));
                    let vis_str = match vis {
                        VisibilityState::Hidden => "hidden",
                        VisibilityState::Lip => "lip",
                        VisibilityState::DockedCollapsed => "docked_collapsed",
                        VisibilityState::Summon => "summon",
                    };
                    let dock_snap = window::get_dock_runtime_snapshot();
                    let js = format!(
                        "window.__DSB_BRIDGE__ && window.__DSB_BRIDGE__.setMode('{}'); window.__DSB_BRIDGE__ && window.__DSB_BRIDGE__.setVisibility && window.__DSB_BRIDGE__.setVisibility('{}'); window.__DSB_BRIDGE__ && window.__DSB_BRIDGE__.setHarness && window.__DSB_BRIDGE__.setHarness('{}'); window.__DSB_DOCK_STATE__ && window.__DSB_DOCK_STATE__({{\"edge\":\"{}\",\"collapsed\":{},\"vertical\":{},\"unread\":{}}});",
                        mode.as_str(),
                        vis_str,
                        normalize_harness_route(&active_harness),
                        dock_edge.as_str(),
                        if vis == VisibilityState::DockedCollapsed { "true" } else { "false" },
                        if dock_snap.vertical || dock_edge.is_side() { "true" } else { "false" },
                        if dock_snap.unread { "true" } else { "false" }
                    );
                    let _ = webview.eval(&js);

                    if vis == VisibilityState::Summon {
                        let _ = webview.eval(
                            "setTimeout(() => window.__DSB_BRIDGE__ && window.__DSB_BRIDGE__.focusComposer(), 60);",
                        );
                    }

                    // Safety net: if DSH hasn't loaded @local/deepseek-bar yet, switch to clean guide pill
                    // instead of ever exposing raw DSH top header chrome.
                    let _ = webview.eval(
                        r#"(function(){
                          const sendDiag = (tag) => {
                            try {
                              const core = window.__TAURI__ && window.__TAURI__.core;
                              if (!core || typeof core.invoke !== 'function') return;
                              const card = document.querySelector('[data-dsb-card]');
                              const comp = document.querySelector('[data-composer-card]');
                              const ext = document.querySelector('[data-dsb-external-composer]');
                              const cr = card ? card.getBoundingClientRect() : null;
                              const ir = (ext || comp) ? (ext || comp).getBoundingClientRect() : null;
                              const detail = JSON.stringify({
                                tag,
                                url: location.href,
                                vis: document.visibilityState,
                                hidden: document.hidden,
                                hasBridge: Boolean(window.__DSB_BRIDGE__),
                                harness: document.documentElement.getAttribute('data-dsb-harness'),
                                mode: document.documentElement.getAttribute('data-dsb-mode'),
                                cloneAttr: document.documentElement.getAttribute('data-dsh-bar-clone'),
                                inner: window.innerWidth + 'x' + window.innerHeight,
                                cardRect: cr ? [Math.round(cr.left), Math.round(cr.top), Math.round(cr.width), Math.round(cr.height)] : null,
                                compRect: ir ? [Math.round(ir.left), Math.round(ir.top), Math.round(ir.width), Math.round(ir.height)] : null,
                                rootLen: (document.getElementById('root') && document.getElementById('root').innerHTML.length) || 0,
                                text: ((document.body && document.body.innerText) || '').replace(/\s+/g, ' ').slice(0, 120)
                              });
                              core.invoke('bar_report', { payload: { kind: 'diag', detail } }).catch(() => {});
                            } catch (e) {}
                          };
                          setTimeout(() => sendDiag('t+800ms'), 800);
                          setTimeout(() => {
                            sendDiag('t+2000ms');
                            if (!window.__DSB_BRIDGE__ && window.__TAURI__ && window.__TAURI__.core) {
                              window.__TAURI__.core.invoke('bar_report', { payload: { kind: 'plugin_not_loaded' } }).catch(() => {});
                            }
                          }, 2000);
                        })();"#,
                    );
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            bar_report,
            bar_reconnect,
            bar_drag_begin,
            bar_drag_step,
            bar_drag_end,
            bar_drag_reset,
            bar_get_dock_state,
            bar_set_dock_edge,
            bar_set_vertical_mode,
            bar_set_dock_unread,
            bar_dock_expand,
            overlay_compose_done,
            harness_get_active,
            harness_set_active,
            harness_list_available,
            harness_list_options,
            harness_load_catalog,
            harness_load_messages,
            harness_send_message,
            harness_cancel_turn,
            harness_focus_desktop,
            harness_session_stats,
            harness_session_state,
            harness_upsert_custom_acp,
            harness_remove_custom_acp,
            universal_get_target,
            app_get_settings,
            app_update_settings,
            app_settings_action,
            app_ensure_default_workspace,
            app_pick_folder,
            app_pick_files,
            app_read_clipboard_files,
            app_describe_paths,
            app_save_clipboard_image,
            quick_search_query,
            quick_search_status,
            quick_search_build_capsule,
            quick_search_open_path,
            quick_search_refresh,
            pick_list_profiles,
            pick_reset_profile,
            pick_export_profile,
            pick_import_profile,
            adapter_plugins_list,
            adapter_plugins_reload,
            adapter_plugins_set_enabled,
            adapter_plugins_install_url,
            adapter_plugins_install_json,
            adapter_plugins_install_file,
            adapter_plugins_uninstall,
            adapter_plugins_export,
            adapter_plugins_fetch_registry,
            adapter_plugins_test,
            adapter_plugins_open_dir,
            vision_get_config,
            vision_probe_hardware,
            vision_update_config,
            vision_test_pipeline,
            vision_unload_models
        ])
        .setup(move |app| {
            let app_handle = app.handle().clone();
            pick::hitmap::register_visual_box_provider(
                pick::vision::sidecar_client::maybe_build_omniparser_provider(),
            );
            let tier = if elevation::is_elevated() {
                "admin"
            } else {
                "user"
            };
            let wv_base = AppConfig::local_data_dir().join("webview");
            let bar_data_dir = wv_base.join(format!("bar-{tier}"));
            let overlay_data_dir = wv_base.join(format!("overlay-{tier}"));
            let _ = std::fs::create_dir_all(&bar_data_dir);
            let _ = std::fs::create_dir_all(&overlay_data_dir);

            let bar_win = WebviewWindowBuilder::new(
                app,
                "bar",
                WebviewUrl::App("index.html".into()),
            )
            .title("Anything-to-Prompt")
            .inner_size(initial_width as f64, DEFAULT_LIP_HEIGHT_DIP as f64)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .shadow(false)
            .visible(false)
            .focused(false)
            .data_directory(bar_data_dir)
            .build()?;

            init_bar_window(&bar_win, exclude_capture, initial_width);
            if let Some(h) = hwnd_of(&bar_win) {
                hook_shared.set_bar_hwnd(h);
            }

            let overlay_win = WebviewWindowBuilder::new(
                app,
                "overlay",
                WebviewUrl::App("overlay.html".into()),
            )
            .title("Anything-to-Prompt Overlay")
            .inner_size(800.0, 600.0)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .shadow(false)
            .visible(false)
            .focused(false)
            .focusable(false)
            .data_directory(overlay_data_dir)
            .build()?;

            init_overlay_window(&overlay_win, exclude_capture);
            if let Some(h) = hwnd_of(&overlay_win) {
                pick::set_overlay_hwnd(h);
            }

            if show_tray_icon {
                if let Ok(handles) = tray::setup_tray(&app_handle) {
                    if let Ok(mut slot) = app.state::<AppShared>().tray_handles.lock() {
                        *slot = Some(handles);
                    }
                }
            }

            // Deploy canonical runtime exe + 4-harness companion/sync plugins & start real-time sync bus
            let _ = harness_sync::apply_external_sync_plugins(sync_external_plugins);
            harness_sync::start_live_sync_services(app_handle.clone(), live_sync_for_setup.clone());

            let (hook_tx, hook_rx) = mpsc::channel::<HookEvent>();
            start_input_hooks(hook_shared.clone(), hook_tx);
            start_hook_event_router(app_handle.clone(), hook_rx);
            start_dsh_health_monitor(app_handle.clone());
            start_dock_watcher(app_handle.clone());
            search_engine_for_setup.spawn_background_indexer();

            let (initial_dock, initial_vertical, initial_side_width) = app_handle
                .try_state::<AppShared>()
                .and_then(|s| s.config.lock().ok().map(|c| (c.dock_edge, c.vertical_mode, c.side_width)))
                .unwrap_or((config::DockEdge::None, false, window::SIDE_DOCK_WIDTH_DIP));
            window::set_dock_side_width_dip(initial_side_width);
            window::set_dock_vertical_mode(initial_vertical || initial_dock.is_side());
            if let Some(target) = startup_dev_state {
                dispatch_state_event(&app_handle, StateEvent::DevForceState(target));
            } else {
                if initial_dock.is_docked() {
                    let dock_app = app_handle.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(650));
                        apply_dock_edge(&dock_app, initial_dock, true);
                    });
                }
                if startup_breath_pulse && !startup_dev_dump {
                    // Subtle breathing-light cursor pulse once hooks & tray are armed
                    cursor::play_ready_breath_pulse();
                }
            }
            if startup_dev_dump {
                let dump_app = app_handle.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(600));
                    if let Some(bar_win) = dump_app.get_webview_window("bar") {
                        log_window_diagnostics(&bar_win, "startup");
                    }
                });
            }

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Anything-to-Prompt")
        .run(|_app_handle, event| {
            if matches!(event, tauri::RunEvent::Exit | tauri::RunEvent::ExitRequested { .. }) {
                pick::vision::sidecar_client::shutdown_sidecar();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_counter_follows_live_composer_capsule_count_and_handles_rapid_picks() {
        let mut st = PickCounterState::default();

        // Empty composer -> #1, then JS reports 1 capsule
        assert_eq!(st.allocate_next(), 1);
        st.sync_from_composer(1, true);
        assert_eq!(st.in_flight, 0);

        // Second pick -> #2, then JS reports 2 capsules
        assert_eq!(st.allocate_next(), 2);
        st.sync_from_composer(2, true);

        // User deletes one capsule -> composer now has 1 -> next pick is #2 (not #3!)
        st.sync_from_composer(1, false);
        assert_eq!(st.allocate_next(), 2);
        st.sync_from_composer(2, true);

        // User deletes ALL capsules -> composer now has 0 -> next pick resets to #1!
        st.sync_from_composer(0, false);
        assert_eq!(st.allocate_next(), 1);

        // Rapid second pick before #1 finishes inserting -> gets #2 with zero collision
        assert_eq!(st.allocate_next(), 2);
        st.sync_from_composer(1, true);
        st.sync_from_composer(2, true);
        assert_eq!(st.in_flight, 0);
        assert_eq!(st.allocate_next(), 3);
    }

    #[test]
    fn all_remote_and_local_capabilities_allow_all_bar_commands() {
        let remote = include_str!("../capabilities/remote.json");
        let local = include_str!("../capabilities/local.json");
        for perm in [
            "allow-bar-report",
            "allow-bar-reconnect",
            "allow-bar-drag-begin",
            "allow-bar-drag-step",
            "allow-bar-drag-end",
            "allow-bar-drag-reset",
            "allow-harness-get-active",
            "allow-harness-set-active",
            "allow-harness-list-available",
            "allow-harness-list-options",
            "allow-harness-load-catalog",
            "allow-harness-load-messages",
            "allow-harness-send-message",
            "allow-harness-cancel-turn",
            "allow-harness-focus-desktop",
            "allow-harness-session-stats",
            "allow-app-get-settings",
            "allow-app-update-settings",
            "allow-app-settings-action",
            "allow-app-ensure-default-workspace",
            "allow-app-pick-folder",
            "allow-app-pick-files",
            "allow-app-read-clipboard-files",
            "allow-app-describe-paths",
            "allow-app-save-clipboard-image",
            "allow-quick-search-query",
            "allow-quick-search-status",
            "allow-quick-search-build-capsule",
            "allow-quick-search-open-path",
            "allow-quick-search-refresh",
        ] {
            assert!(remote.contains(perm), "remote.json missing {perm}");
            assert!(local.contains(perm), "local.json missing {perm}");
        }
    }
}
