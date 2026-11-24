pub mod broker;
pub mod captures;
pub mod config;
pub mod cursor;
pub mod dsh;
pub mod elevation;
pub mod hooks;
pub mod pick;
pub mod plugin_sync;
pub mod state;
pub mod tray;
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
    pub elevated_relaunch: bool,
    pub no_elevate: bool,
    pub dev_state: Option<DevTargetState>,
    pub dev_dump: bool,
    pub dev_allow_capture: bool,
    pub dev_single_ctrl_close: bool,
    pub dev_draft: Option<String>,
    pub dev_pick_tap: Option<(i32, i32)>,
    pub dev_pick_stroke: Option<(i32, i32, i32, i32)>,
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
                _ => {
                    out.passthrough_args.push(vec[i].clone());
                }
            }
            i += 1;
        }
        out
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
    pub hook_shared: Arc<HookSharedConfig>,
    pub pick_counter: Mutex<PickCounterState>,
    pub compose_waiters: Mutex<HashMap<String, mpsc::Sender<String>>>,
    pub last_outside_hide_at: Mutex<Option<std::time::Instant>>,
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
}

#[tauri::command]
fn bar_report(app: AppHandle, payload: BarReportPayload) {
    let Some(shared) = app.try_state::<AppShared>() else {
        return;
    };

    match payload.kind.as_str() {
        "layout" => {
            let width = payload.width.clamp(480, 1200);
            let height = payload.target_height.clamp(DEFAULT_LIP_HEIGHT_DIP, 900);

            if let Ok(mut cfg) = shared.config.lock() {
                if cfg.bar_width != width {
                    cfg.bar_width = width;
                    let _ = cfg.save();
                }
            }

            if let Ok(mut cached) = shared.last_layout.lock() {
                cached.width_dip = width;
                cached.height_dip = height;
                cached.shapes = payload.shapes.clone();
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
            dispatch_state_event(&app, StateEvent::Submitted);
        }

        "capsules" => {
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

        other => {
            log::debug!("Unhandled bar_report kind: {other}");
        }
    }
}

#[tauri::command]
fn bar_reconnect(app: AppHandle) {
    trigger_dsh_reconnect(app);
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

pub fn dispatch_state_event(app: &AppHandle, event: StateEvent) {
    let Some(shared) = app.try_state::<AppShared>() else {
        return;
    };

    let (fx, vis_snapshot) = {
        let Ok(mut sm) = shared.state_machine.lock() else {
            return;
        };
        let fx = sm.apply(event);
        (fx, sm.visibility)
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

    if let Some(win_act) = fx.window_action {
        if let Some(bar_win) = app.get_webview_window("bar") {
            match win_act {
                WindowAction::Hide => {
                    hide_bar(&bar_win);
                }
                WindowAction::ShowNoActivate => {
                    show_bar_noactivate(&bar_win);
                }
                WindowAction::ActivateAndFocus => {
                    activate_and_focus_bar(&bar_win);
                }
            }
        }
        log::info!("Bar visibility -> {:?} ({:?})", vis_snapshot, win_act);
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
                    let hov = pick::inspect_hover_at_with_uia(
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
            let mut pending_raw_capture: Option<captures::RawScreenCapture> = None;
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
                        dispatch_state_event(&app, StateEvent::DoubleCtrlOpen);
                        if !was_summon && foreground_hwnd != 0 {
                            let auto_app = app.clone();
                            std::thread::spawn(move || {
                                pick::maybe_auto_attach_explorer_selection(
                                    &auto_app,
                                    foreground_hwnd,
                                );
                            });
                        }
                    }
                    HookEvent::OutsideClick => {
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
                        dispatch_state_event(&app, StateEvent::PenStarted);
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
                        push_hover(cur_gen, x, y);
                    }
                    HookEvent::PenMouseDown { x, y } => {
                        pending_tap_allow_bg = bg_dwell
                            .lock()
                            .map(|mut d| is_bg_dwell_unlocked(&mut d))
                            .unwrap_or(false);
                        pending_raw_capture = pick::capture_mousedown_snapshot(x, y);
                        if let Some(hit) = pick::adapters::office::detect_office_canvas_at(x, y) {
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
                    HookEvent::PenDrag { points } => {
                        if points.len() < 2 {
                            continue;
                        }
                        let first = points[0];
                        let last = *points.last().unwrap_or(&first);
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
                                            "mode": "stroke",
                                            "points": pts,
                                        }),
                                    );
                                }
                            }
                        }
                    }
                    HookEvent::PenTap { x, y } => {
                        let pre = pending_raw_capture.take();
                        let allow_bg = pending_tap_allow_bg;
                        pending_tap_allow_bg = false;
                        active_drag_mode = pick::adapters::office::OfficeDragMode::NormalStroke;
                        active_drag_hwnd = 0;
                        let tap_app = app.clone();
                        std::thread::spawn(move || {
                            pick::execute_tap_pick(&tap_app, x, y, pre, allow_bg);
                        });
                    }
                    HookEvent::PenStroke { points } => {
                        let pre = pending_raw_capture.take();
                        pending_tap_allow_bg = false;
                        active_drag_mode = pick::adapters::office::OfficeDragMode::NormalStroke;
                        active_drag_hwnd = 0;
                        if let Ok(mut guard) = word_sel_slot.0.lock() {
                            *guard = None;
                        }
                        let stroke_app = app.clone();
                        std::thread::spawn(move || {
                            pick::execute_stroke_pick(&stroke_app, points, pre);
                        });
                    }
                    HookEvent::PenEnd { picked_any } => {
                        pen_active.store(false, Ordering::Release);
                        pen_gen.fetch_add(1, Ordering::AcqRel);
                        log::info!("PenEnd (picked_any={picked_any})");
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
                        dispatch_state_event(&app, StateEvent::PenEnded { picked_any });
                    }
                }
            }
        })
        .expect("failed to start hook event router");
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
        "Starting Anything-to-Prompt (pid={}, elevated={})",
        std::process::id(),
        elevation::is_elevated()
    );

    // Ensure our single built-in DSH companion plugin (@local/anything-to-prompt) is synced into ~/.dsh/profiles/web
    let _ = plugin_sync::ensure_dsh_plugin_synced();

    let hook_shared = HookSharedConfig::new(
        config.accept_injected_input,
        config.double_tap_ms,
        config.ctrl_hold_ms,
    );
    let initial_width = config.bar_width;
    let exclude_capture = config.exclude_from_capture && !cli.dev_allow_capture;

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
        hook_shared: hook_shared.clone(),
        pick_counter: Mutex::new(PickCounterState::default()),
        compose_waiters: Mutex::new(HashMap::new()),
        last_outside_hide_at: Mutex::new(None),
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
            } else if !forwarded.dev_dump
                && forwarded.dev_pick_tap.is_none()
                && forwarded.dev_pick_stroke.is_none()
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
                    dispatch_state_event(&handle, StateEvent::PenEnded { picked_any: true });
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
                    pick::execute_stroke_pick(&handle, pts, pre);
                    if let Some(ov) = handle.get_webview_window("overlay") {
                        let _ = ov.emit("pen:hide", serde_json::json!({}));
                        hide_overlay(&ov);
                    }
                    dispatch_state_event(&handle, StateEvent::PenEnded { picked_any: true });
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

                    // Sync current BridgeMode once DSH Bar clone page finishes loading
                    let mode = shared
                        .last_bridge_mode
                        .lock()
                        .map(|m| *m)
                        .unwrap_or(BridgeMode::Summon);
                    let js = format!(
                        "window.__DSB_BRIDGE__ && window.__DSB_BRIDGE__.setMode('{}');",
                        mode.as_str()
                    );
                    let _ = webview.eval(&js);

                    let vis = shared
                        .state_machine
                        .lock()
                        .map(|sm| sm.visibility)
                        .unwrap_or(VisibilityState::Hidden);
                    if vis == VisibilityState::Summon {
                        let _ = webview.eval(
                            "setTimeout(() => window.__DSB_BRIDGE__ && window.__DSB_BRIDGE__.focusComposer(), 60);",
                        );
                    }

                    // Safety net: if DSH hasn't loaded @local/deepseek-bar yet, switch to clean guide pill
                    // instead of ever exposing raw DSH top header chrome.
                    let _ = webview.eval(
                        "setTimeout(() => { if (!window.__DSB_BRIDGE__ && window.__TAURI__ && window.__TAURI__.core) { window.__TAURI__.core.invoke('bar_report', { payload: { kind: 'plugin_not_loaded' } }).catch(() => {}); } }, 1600);",
                    );
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            bar_report,
            bar_reconnect,
            overlay_compose_done
        ])
        .setup(move |app| {
            let app_handle = app.handle().clone();
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

            if let Ok(handles) = tray::setup_tray(&app_handle) {
                if let Ok(mut slot) = app.state::<AppShared>().tray_handles.lock() {
                    *slot = Some(handles);
                }
            }

            let (hook_tx, hook_rx) = mpsc::channel::<HookEvent>();
            start_input_hooks(hook_shared.clone(), hook_tx);
            start_hook_event_router(app_handle.clone(), hook_rx);
            start_dsh_health_monitor(app_handle.clone());

            if let Some(target) = startup_dev_state {
                dispatch_state_event(&app_handle, StateEvent::DevForceState(target));
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
        .run(tauri::generate_context!())
        .expect("error while running Anything-to-Prompt");
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
}
