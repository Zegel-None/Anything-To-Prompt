use crate::captures::ScreenRect;
use crate::pick::hitmap;
use crate::pick::probe;
use crate::pick::profile::{
    self, enforce_hard_invariants, get_effective_profile, is_known_profile, normalize_exe_key,
    now_unix_secs, save_learned_profile, GroundingLayer, ProfileSource, UiaPolicy,
};
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use windows::Win32::Foundation::HWND;

const HEAL_COOLDOWN: Duration = Duration::from_secs(30);
const JITTER_WINDOW: Duration = Duration::from_millis(450);
const REJECT_WINDOW: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealTrigger {
    UiaSlowOrStall,
    GiantBoxMonolith,
    HoverRectFlicker,
    UserRejection,
}

impl HealTrigger {
    pub fn reason_code(self) -> &'static str {
        match self {
            HealTrigger::UiaSlowOrStall => "uia_slow_or_stall",
            HealTrigger::GiantBoxMonolith => "giant_box_monolith",
            HealTrigger::HoverRectFlicker => "hover_rect_flicker",
            HealTrigger::UserRejection => "user_rejection",
        }
    }
}

#[derive(Debug, Default)]
struct AppRuntimeHealth {
    consecutive_slow_uia: u32,
    consecutive_giant_box: u32,
    last_cursor: Option<(i32, i32)>,
    last_rect: Option<ScreenRect>,
    last_hover_at: Option<Instant>,
    jitter_hits: u32,
    rejection_times: Vec<Instant>,
    last_healed_at: Option<Instant>,
}

#[derive(Default)]
struct SelfHealState {
    health: HashMap<String, AppRuntimeHealth>,
    onboarding_in_flight: HashSet<String>,
}

static SELF_HEAL: OnceLock<Mutex<SelfHealState>> = OnceLock::new();

fn state_lock() -> &'static Mutex<SelfHealState> {
    SELF_HEAL.get_or_init(|| Mutex::new(SelfHealState::default()))
}

pub fn emit_status_hint(
    app: &AppHandle,
    text: &str,
    cursor: Option<(i32, i32)>,
    phase: &str,
    duration_ms: u64,
) {
    if let Some(ov) = app.get_webview_window("overlay") {
        let cur = cursor.map(|(x, y)| serde_json::json!({ "x": x, "y": y }));
        let _ = ov.emit(
            "pen:status_hint",
            serde_json::json!({
                "text": text,
                "cursor": cur,
                "phase": phase,
                "durationMs": duration_ms,
            }),
        );
    }
}

/// First-time onboarding for an unknown application:
/// Shows a lightweight cursor-adjacent pill ("正在熟悉此软件界面…") while probing Win32/MSAA/watchdog-UIA
/// on a background thread and persisting the learned `AppUiProfile`. Never blocks the hover loop.
pub fn maybe_trigger_first_time_onboarding(
    app: &AppHandle,
    root_hwnd: HWND,
    exe_name: &str,
    cursor: (i32, i32),
) {
    let key = normalize_exe_key(exe_name);
    if key.is_empty() || is_known_profile(&key) {
        return;
    }

    let should_start = {
        let Ok(mut st) = state_lock().lock() else {
            return;
        };
        if st.onboarding_in_flight.contains(&key) {
            false
        } else {
            st.onboarding_in_flight.insert(key.clone());
            true
        }
    };
    if !should_start {
        return;
    }

    emit_status_hint(app, "正在熟悉此软件界面…", Some(cursor), "working", 2600);

    let app_clone = app.clone();
    let hwnd_raw = root_hwnd.0 as isize;
    let key_clone = key.clone();
    let _ = std::thread::Builder::new()
        .name(format!("atp-onboard-{key_clone}"))
        .spawn(move || {
            let hwnd = HWND(hwnd_raw as *mut std::ffi::c_void);
            let report = probe::probe_window(hwnd, &key_clone, Some(cursor));
            let _ = save_learned_profile(report.synthesized_profile);
            hitmap::invalidate_all_hitmaps();
            emit_status_hint(&app_clone, "已熟悉此软件界面", Some(cursor), "done", 900);
            if let Ok(mut st) = state_lock().lock() {
                st.onboarding_in_flight.remove(&key_clone);
            }
        });
}

/// Applies a deterministic step-down optimization to `exe_name`'s profile when an anomaly fires.
pub fn apply_heal_step_down(exe_name: &str, trigger: HealTrigger) -> Option<profile::AppUiProfile> {
    let key = normalize_exe_key(exe_name);
    if key.is_empty() {
        return None;
    }
    let mut prof = get_effective_profile(&key);
    prof.source = ProfileSource::Learned;
    prof.learned_at_unix = now_unix_secs();

    match trigger {
        HealTrigger::UiaSlowOrStall => {
            prof.uia_policy = UiaPolicy::Forbidden;
            prof.preferred_layers = vec![
                GroundingLayer::MsaaHitMap,
                GroundingLayer::StaticVisualHitMap,
            ];
            prof.notes = format!("auto-healed ({}): stepped off slow UIA", trigger.reason_code());
        }
        HealTrigger::GiantBoxMonolith => {
            prof.preferred_layers = vec![
                GroundingLayer::MsaaHitMap,
                GroundingLayer::StaticVisualHitMap,
            ];
            if prof.uia_policy == UiaPolicy::Preferred {
                prof.uia_policy = UiaPolicy::WatchdogOnly;
            }
            prof.notes = format!(
                "auto-healed ({}): bypassed monolithic UIA pane",
                trigger.reason_code()
            );
        }
        HealTrigger::HoverRectFlicker => {
            // Never downgrade native UIA-preferred modern apps (Electron / IDE / Browser / Office)
            // off UIA because moving between adjacent DOM text/chips naturally changes rect sizes!
            if prof.uia_policy == UiaPolicy::Preferred {
                return None;
            }
            prof.preferred_layers = vec![
                GroundingLayer::StaticVisualHitMap,
                GroundingLayer::MsaaHitMap,
            ];
            prof.notes = format!(
                "auto-healed ({}): locked onto static visual Hit-Map",
                trigger.reason_code()
            );
        }
        HealTrigger::UserRejection => {
            // Rotate strategy: Uia -> MsaaHitMap -> StaticVisualHitMap
            let first = prof.preferred_layers.first().copied();
            prof.preferred_layers = match first {
                Some(GroundingLayer::Uia) => vec![
                    GroundingLayer::MsaaHitMap,
                    GroundingLayer::StaticVisualHitMap,
                ],
                Some(GroundingLayer::MsaaHitMap) => vec![
                    GroundingLayer::StaticVisualHitMap,
                    GroundingLayer::MsaaHitMap,
                ],
                _ => vec![
                    GroundingLayer::StaticVisualHitMap,
                    GroundingLayer::MsaaHitMap,
                ],
            };
            prof.notes = format!(
                "auto-healed ({}): switched layer after user rejection",
                trigger.reason_code()
            );
        }
    }

    enforce_hard_invariants(&mut prof);
    hitmap::invalidate_all_hitmaps();
    save_learned_profile(prof).ok()
}

fn try_fire_heal_core(exe_key: &str, trigger: HealTrigger) -> bool {
    let allowed = {
        let Ok(mut st) = state_lock().lock() else {
            return false;
        };
        let h = st.health.entry(exe_key.to_string()).or_default();
        if let Some(prev) = h.last_healed_at {
            if prev.elapsed() < HEAL_COOLDOWN {
                return false;
            }
        }
        h.last_healed_at = Some(Instant::now());
        h.consecutive_slow_uia = 0;
        h.consecutive_giant_box = 0;
        h.jitter_hits = 0;
        h.rejection_times.clear();
        true
    };
    if !allowed {
        return false;
    }

    let updated = apply_heal_step_down(exe_key, trigger);
    if updated.is_some() {
        log::info!(
            "Self-healed UI profile for '{}' due to {:?}",
            exe_key,
            trigger
        );
        true
    } else {
        false
    }
}

fn maybe_emit_heal_hint(app: Option<&AppHandle>, fired: bool, cursor: Option<(i32, i32)>) {
    if fired {
        if let Some(a) = app {
            emit_status_hint(a, "正在深度优化此软件的识别", cursor, "working", 1800);
        }
    }
}

/// Detector 1 (pure): Watchdog latency / stall guard on L1 UIA queries.
pub fn record_uia_latency_core(exe_name: &str, elapsed_ms: u64, timeout_ms: u64) -> bool {
    let key = normalize_exe_key(exe_name);
    if key.is_empty() {
        return false;
    }
    let fire = {
        let Ok(mut st) = state_lock().lock() else {
            return false;
        };
        let h = st.health.entry(key.clone()).or_default();
        if elapsed_ms > timeout_ms {
            h.consecutive_slow_uia += 1;
        } else {
            h.consecutive_slow_uia = 0;
        }
        h.consecutive_slow_uia >= 2
    };
    if fire {
        try_fire_heal_core(&key, HealTrigger::UiaSlowOrStall)
    } else {
        false
    }
}

/// Detector 1: Watchdog latency / stall guard on L1 UIA queries.
pub fn record_uia_latency(
    app: Option<&AppHandle>,
    exe_name: &str,
    elapsed_ms: u64,
    timeout_ms: u64,
    cursor: (i32, i32),
) -> bool {
    let fired = record_uia_latency_core(exe_name, elapsed_ms, timeout_ms);
    maybe_emit_heal_hint(app, fired, Some(cursor));
    fired
}

/// Detector 2 (pure): Monolithic giant-box detector.
pub fn record_hover_box_core(
    exe_name: &str,
    rect: ScreenRect,
    win_rect: ScreenRect,
    is_background: bool,
) -> bool {
    if is_background {
        return false;
    }
    let key = normalize_exe_key(exe_name);
    if key.is_empty() {
        return false;
    }
    let prof = get_effective_profile(&key);
    if matches!(
        prof.semantic_adapter.as_deref(),
        Some("terminal") | Some("ide") | Some("browser")
    ) {
        return false;
    }
    let win_area = ((win_rect.w.max(1) as i64) * (win_rect.h.max(1) as i64)).max(1);
    let box_area = (rect.w.max(0) as i64) * (rect.h.max(0) as i64);
    let is_giant = rect.w > 500 && rect.h > 220 && box_area * 100 >= win_area * 45;

    let fire = {
        let Ok(mut st) = state_lock().lock() else {
            return false;
        };
        let h = st.health.entry(key.clone()).or_default();
        if is_giant {
            h.consecutive_giant_box += 1;
        } else {
            h.consecutive_giant_box = 0;
        }
        h.consecutive_giant_box >= 3
    };
    if fire {
        try_fire_heal_core(&key, HealTrigger::GiantBoxMonolith)
    } else {
        false
    }
}

/// Detector 2: Monolithic giant-box detector (non-background UIA box swallowing an entire toolbar/pane).
pub fn record_hover_box(
    app: Option<&AppHandle>,
    exe_name: &str,
    rect: ScreenRect,
    win_rect: ScreenRect,
    is_background: bool,
    cursor: (i32, i32),
) -> bool {
    let fired = record_hover_box_core(exe_name, rect, win_rect, is_background);
    maybe_emit_heal_hint(app, fired, Some(cursor));
    fired
}

/// Detector 3 (pure): Hover flicker / jitter detector.
pub fn record_hover_stability_core(exe_name: &str, cursor: (i32, i32), rect: ScreenRect) -> bool {
    let key = normalize_exe_key(exe_name);
    if key.is_empty() {
        return false;
    }
    let now = Instant::now();
    let fire = {
        let Ok(mut st) = state_lock().lock() else {
            return false;
        };
        let h = st.health.entry(key.clone()).or_default();
        if let (Some(prev_c), Some(prev_r), Some(prev_t)) =
            (h.last_cursor, h.last_rect, h.last_hover_at)
        {
            if now.duration_since(prev_t) <= JITTER_WINDOW {
                let move_dist = (cursor.0 - prev_c.0).abs() + (cursor.1 - prev_c.1).abs();
                if move_dist <= 8 {
                    let prev_area = ((prev_r.w.max(1) as i64) * (prev_r.h.max(1) as i64)).max(1);
                    let cur_area = ((rect.w.max(1) as i64) * (rect.h.max(1) as i64)).max(1);
                    let area_diff_pct = ((cur_area - prev_area).abs() * 100) / prev_area;
                    let origin_jump = (rect.x - prev_r.x).abs() + (rect.y - prev_r.y).abs();
                    if area_diff_pct >= 35 || origin_jump >= 14 {
                        h.jitter_hits += 1;
                    } else {
                        h.jitter_hits = h.jitter_hits.saturating_sub(1);
                    }
                } else {
                    h.jitter_hits = 0;
                }
            } else {
                h.jitter_hits = 0;
            }
        }
        h.last_cursor = Some(cursor);
        h.last_rect = Some(rect);
        h.last_hover_at = Some(now);
        h.jitter_hits >= 3
    };
    if fire {
        try_fire_heal_core(&key, HealTrigger::HoverRectFlicker)
    } else {
        false
    }
}

/// Detector 3: Hover flicker / jitter detector (tiny cursor movement causing wild rect size/origin jumps).
pub fn record_hover_stability(
    app: Option<&AppHandle>,
    exe_name: &str,
    cursor: (i32, i32),
    rect: ScreenRect,
) -> bool {
    let fired = record_hover_stability_core(exe_name, cursor, rect);
    maybe_emit_heal_hint(app, fired, Some(cursor));
    fired
}

/// Detector 4 (pure): User rejection signal.
pub fn record_user_rejection_core(exe_name: &str) -> bool {
    let key = normalize_exe_key(exe_name);
    if key.is_empty() {
        return false;
    }
    let now = Instant::now();
    let fire = {
        let Ok(mut st) = state_lock().lock() else {
            return false;
        };
        let h = st.health.entry(key.clone()).or_default();
        h.rejection_times
            .retain(|t| now.duration_since(*t) <= REJECT_WINDOW);
        h.rejection_times.push(now);
        h.rejection_times.len() >= 2
    };
    if fire {
        try_fire_heal_core(&key, HealTrigger::UserRejection)
    } else {
        false
    }
}

/// Detector 4: User rejection signal (deleting a freshly picked capsule within 5s or switching to drag-lasso).
pub fn record_user_rejection(app: Option<&AppHandle>, exe_name: &str) -> bool {
    let fired = record_user_rejection_core(exe_name);
    maybe_emit_heal_hint(app, fired, None);
    fired
}

pub fn clear_runtime_health(exe_name: &str) {
    let key = normalize_exe_key(exe_name);
    if let Ok(mut st) = state_lock().lock() {
        st.health.remove(&key);
        st.onboarding_in_flight.remove(&key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detector_uia_stall_triggers_after_two_slow_hits() {
        let exe = "unit_test_stall_app.exe";
        clear_runtime_health(exe);
        assert!(!record_uia_latency_core(exe, 220, 120));
        assert!(record_uia_latency_core(exe, 250, 120));
        let prof = get_effective_profile(exe);
        assert_eq!(prof.uia_policy, UiaPolicy::Forbidden);
        let _ = profile::reset_learned_profile(exe);
        clear_runtime_health(exe);
    }

    #[test]
    fn detector_giant_box_triggers_after_three_monoliths() {
        let exe = "unit_test_giant_app.exe";
        clear_runtime_health(exe);
        let win = ScreenRect { x: 0, y: 0, w: 1200, h: 800 };
        let big = ScreenRect { x: 10, y: 10, w: 1000, h: 600 };
        assert!(!record_hover_box_core(exe, big, win, false));
        assert!(!record_hover_box_core(exe, big, win, false));
        assert!(record_hover_box_core(exe, big, win, false));
        let prof = get_effective_profile(exe);
        assert_eq!(prof.preferred_layers[0], GroundingLayer::MsaaHitMap);
        let _ = profile::reset_learned_profile(exe);
        clear_runtime_health(exe);
    }

    #[test]
    fn detector_flicker_triggers_on_wild_rect_jumps_with_tiny_cursor_move() {
        let exe = "unit_test_flicker_app.exe";
        clear_runtime_health(exe);
        let r1 = ScreenRect { x: 100, y: 40, w: 80, h: 70 };
        let r2 = ScreenRect { x: 125, y: 50, w: 28, h: 24 };
        assert!(!record_hover_stability_core(exe, (130, 60), r1));
        assert!(!record_hover_stability_core(exe, (132, 60), r2));
        assert!(!record_hover_stability_core(exe, (134, 61), r1));
        assert!(record_hover_stability_core(exe, (135, 61), r2));
        let prof = get_effective_profile(exe);
        assert_eq!(prof.preferred_layers[0], GroundingLayer::StaticVisualHitMap);
        let _ = profile::reset_learned_profile(exe);
        clear_runtime_health(exe);
    }

    #[test]
    fn detector_user_rejection_triggers_after_two_rejections() {
        let exe = "unit_test_reject_app.exe";
        clear_runtime_health(exe);
        assert!(!record_user_rejection_core(exe));
        assert!(record_user_rejection_core(exe));
        let prof = get_effective_profile(exe);
        assert_ne!(prof.preferred_layers[0], GroundingLayer::Uia);
        let _ = profile::reset_learned_profile(exe);
        clear_runtime_health(exe);
    }
}
