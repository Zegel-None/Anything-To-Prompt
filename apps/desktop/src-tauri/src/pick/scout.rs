//! Stealth Scout Halo + Shake-to-Wake Continuous Dose Engine.
//!
//! Unified zero-MMB architecture inside Selection Mode:
//! - **Dose 0 (Default Lightweight · 0 VRAM, 0.3~3ms)**:
//!   Background Stealth Scout silently pre-warms `Win32 Native` + `MSAA` + `0ms StaticVisual`
//!   around the cursor into `HITMAP_CACHE`. Screen stays completely clean — only the single box
//!   under the cursor lights up.
//!   (Exception — **Scheme B**: if `AppUiProfile` learned `auto_wake_omniparser` for this
//!    `(exe, container_class)` AND GPU auto-wake is permitted by `VisionConfig`, the Stealth Scout
//!    automatically pre-warms `OmniParser v2` silently in the background!)
//! - **Dose 1 (Shake-to-Wake · >= 2 reversals within 650ms)**:
//!   Emits a subtle amber cursor pulse, wakes `OmniParser v2` + `Hover-Diff` + Deep UIA on the
//!   owning container / `768x384` overlapping grid tile using overlay-free `PrintWindow` (with
//!   all-black-frame fallback), merges additively into `HITMAP_CACHE`, records Scheme B strategy
//!   preference when OmniParser beats static segmentation, and immediately refreshes hover!
//! - **Dose 2 (Continued Shake after Dose 1 Result Landed)**:
//!   Emits a violet-cyan boost pulse and dispatches smartly:
//!   1. Expandable `▾`/`▸` menu trigger nearby -> safe multi-level drawer crawl (depth <= 4)
//!   2. Natural photo / landscape region -> full-scene SAM 2.1 organic semantic matting
//!   3. Stubborn custom UI (when `rebrush_vlm_audit` is enabled) -> additive VLM arbitration!

use crate::captures::{
    capture_hwnd_via_print_window, capture_screen_rect_rgba, RawScreenCapture, ScreenRect,
};
use crate::config::{AppConfig, SegmentationBackend, VisionConfig, VlmBackend};
use crate::pick::adapters::{office, solidworks};
use crate::pick::hitmap::{self, ContainerLayoutKind, HitMapEntry, HitMapSource};
use crate::pick::profile::{self, UiaPolicy};
use crate::pick::rebrush::{DoseCellKey, DoseEscalation, ShakeDoseDetector};
use crate::pick::{brush_scan, enricher, hover_diff, self_heal, vision, win32_controls};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};

type ScoutJob = (AppHandle, u64, i32, i32, u32, Arc<dyn Fn(i32, i32) + Send + Sync>);

#[derive(Default)]
struct ScoutState {
    active: bool,
    fallback_snapshot: Option<Arc<RawScreenCapture>>,
    cached_vision_cfg: VisionConfig,
    scouted_tiles: HashSet<(isize, i32, i32)>,
    dose1_tiles: HashSet<(isize, i32, i32)>,
    shake: ShakeDoseDetector,
    last_cursor: Option<(i32, i32)>,
}

struct ScoutController {
    state: Mutex<ScoutState>,
    latest_job: Mutex<Option<ScoutJob>>,
    cv: Condvar,
    worker_started: AtomicBool,
    dose_inflight: AtomicBool,
    session_gen: AtomicU64,
}

static SCOUT: OnceLock<ScoutController> = OnceLock::new();

fn ctrl() -> &'static ScoutController {
    SCOUT.get_or_init(|| ScoutController {
        state: Mutex::new(ScoutState::default()),
        latest_job: Mutex::new(None),
        cv: Condvar::new(),
        worker_started: AtomicBool::new(false),
        dose_inflight: AtomicBool::new(false),
        session_gen: AtomicU64::new(0),
    })
}

/// RAII guard guaranteeing `dose_inflight` is ALWAYS released even on early return or panic.
struct DoseInflightGuard<'a>(&'a AtomicBool);
impl Drop for DoseInflightGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn emit_overlay(app: &AppHandle, event: &str, payload: serde_json::Value) {
    if let Some(ov) = app.get_webview_window("overlay") {
        let _ = ov.emit(event, payload);
    }
}

fn capture_monitor_snapshot_at(x: i32, y: i32) -> Option<Arc<RawScreenCapture>> {
    let hmon = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if unsafe { GetMonitorInfoW(hmon, &mut mi) }.as_bool() {
        let rc = mi.rcMonitor;
        let w = (rc.right - rc.left).max(0);
        let h = (rc.bottom - rc.top).max(0);
        if w >= 64 && h >= 64 {
            if let Ok(cap) = capture_screen_rect_rgba(rc.left, rc.top, w, h) {
                return Some(Arc::new(cap));
            }
        }
    }
    None
}

fn crop_from_raw(
    raw: &RawScreenCapture,
    rect: ScreenRect,
) -> Option<(image::RgbaImage, i32, i32)> {
    let sx0 = (rect.x - raw.origin_x).clamp(0, raw.image.width() as i32);
    let sy0 = (rect.y - raw.origin_y).clamp(0, raw.image.height() as i32);
    let sx1 = (rect.x + rect.w - raw.origin_x).clamp(sx0, raw.image.width() as i32);
    let sy1 = (rect.y + rect.h - raw.origin_y).clamp(sy0, raw.image.height() as i32);
    let cw = (sx1 - sx0) as u32;
    let ch = (sy1 - sy0) as u32;
    if cw < 16 || ch < 16 {
        return None;
    }
    let cropped = image::imageops::crop_imm(&raw.image, sx0 as u32, sy0 as u32, cw, ch).to_image();
    Some((cropped, raw.origin_x + sx0, raw.origin_y + sy0))
}

/// Checks if a PrintWindow capture is virtually all-black (DirectX/OpenGL surface quirk).
fn is_nearly_all_black(img: &image::RgbaImage) -> bool {
    let (w, h) = (img.width(), img.height());
    if w < 8 || h < 8 {
        return true;
    }
    let sx = (w / 16).max(1);
    let sy = (h / 12).max(1);
    let mut non_black = 0usize;
    let mut total = 0usize;
    let mut y = sy / 2;
    while y < h {
        let mut x = sx / 2;
        while x < w {
            let p = img.get_pixel(x, y).0;
            if p[0] > 8 || p[1] > 8 || p[2] > 8 {
                non_black += 1;
            }
            total += 1;
            x += sx;
        }
        y += sy;
    }
    total > 0 && non_black * 100 < total * 3
}

/// Captures `cap_rect` completely FREE of our own blue overlay frames:
/// 1. For SolidWorks (`SLDWORKS.exe`), NEVER invokes `PrintWindow` (synchronous `WM_PRINT` stalls
///    SolidWorks's STA UI/COM thread!); instead uses the clean pre-overlay session snapshot first
///    and falls back to lock-free DWM screen `BitBlt`.
/// 2. For all other apps, prefers `PrintWindow(PW_RENDERFULLCONTENT)` on `root_hwnd` (fresh, zero
///    overlay, zero flicker), falling back to the clean pre-overlay session snapshot if PrintWindow
///    fails or yields an all-black DirectX/OpenGL frame.
pub fn capture_clean_region_no_overlay(
    root_hwnd: HWND,
    cap_rect: ScreenRect,
    fallback_snap: Option<&RawScreenCapture>,
) -> Option<(image::RgbaImage, i32, i32)> {
    if crate::captures::is_print_window_unsafe_hwnd(root_hwnd) {
        if let Some(snap) = fallback_snap {
            if let Some(res) = crop_from_raw(snap, cap_rect) {
                return Some(res);
            }
        }
        if cap_rect.w >= 16 && cap_rect.h >= 16 {
            if let Ok(raw) =
                capture_screen_rect_rgba(cap_rect.x, cap_rect.y, cap_rect.w, cap_rect.h)
            {
                return Some((raw.image, raw.origin_x, raw.origin_y));
            }
        }
        return None;
    }
    if !root_hwnd.0.is_null() {
        if let Ok(pw_cap) = capture_hwnd_via_print_window(root_hwnd) {
            if let Some((crop, ox, oy)) = crop_from_raw(&pw_cap, cap_rect) {
                if !is_nearly_all_black(&crop) {
                    return Some((crop, ox, oy));
                }
            }
        }
    }
    if let Some(snap) = fallback_snap {
        if let Some(res) = crop_from_raw(snap, cap_rect) {
            return Some(res);
        }
    }
    None
}

fn compute_scout_tile(
    child_hwnd: HWND,
    child_rect: ScreenRect,
    is_popup: bool,
    x: i32,
    y: i32,
) -> ((isize, i32, i32), ScreenRect) {
    let hwnd_raw = child_hwnd.0 as isize;
    let is_compact = (child_rect.w <= 2600 && child_rect.h <= 480)
        || (child_rect.w <= 640 && child_rect.h <= 1600)
        || is_popup;
    if is_compact {
        ((hwnd_raw, 0, 0), child_rect)
    } else {
        let gx = x.div_euclid(512) * 512;
        let gy = y.div_euclid(256) * 256;
        let tx0 = (gx - 128).max(child_rect.x);
        let ty0 = (gy - 64).max(child_rect.y);
        let tx1 = (gx + 640).min(child_rect.x + child_rect.w);
        let ty1 = (gy + 320).min(child_rect.y + child_rect.h);
        (
            (hwnd_raw, gx, gy),
            ScreenRect {
                x: tx0,
                y: ty0,
                w: (tx1 - tx0).max(0),
                h: (ty1 - ty0).max(0),
            },
        )
    }
}

fn ensure_scout_worker_started() {
    let c = ctrl();
    if c.worker_started
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("atp-stealth-scout".to_string())
        .spawn(move || loop {
            let job = {
                let Ok(mut guard) = c.latest_job.lock() else {
                    break;
                };
                while guard.is_none() {
                    guard = match c.cv.wait(guard) {
                        Ok(g) => g,
                        Err(_) => return,
                    };
                }
                guard.take()
            };
            let Some((app, gen, x, y, own_pid, on_refresh)) = job else {
                continue;
            };
            std::thread::sleep(Duration::from_millis(24));
            if c.latest_job.lock().map(|g| g.is_some()).unwrap_or(false) {
                continue;
            }
            if c.session_gen.load(Ordering::Acquire) != gen {
                continue;
            }
            let active = c.state.lock().map(|s| s.active).unwrap_or(false);
            if !active {
                continue;
            }
            run_dose0_stealth_scout(&app, gen, x, y, own_pid, &*on_refresh);
        });
}

/// Called on `PenStart` — bumps session generation, resets state, and captures a clean fallback snapshot.
pub fn on_session_start(x: i32, y: i32) {
    ensure_scout_worker_started();
    let c = ctrl();
    c.session_gen.fetch_add(1, Ordering::AcqRel);
    brush_scan::reset_abort_flag();
    enricher::clear_session_cache();
    let snap = capture_monitor_snapshot_at(x, y);
    let vision_cfg = AppConfig::load_or_create().vision;
    if let Ok(mut st) = c.state.lock() {
        st.active = true;
        st.fallback_snapshot = snap;
        st.cached_vision_cfg = vision_cfg;
        st.scouted_tiles.clear();
        st.dose1_tiles.clear();
        st.shake.clear();
        st.last_cursor = Some((x, y));
    }
}

/// Called on passive `WM_MOUSEWHEEL` inside Selection Mode — invalidates ALL scrolled caches,
/// resets shake dose state so newly scrolled content can wake Dose 1 fresh, clears stale photo
/// cutouts on the overlay, clears hover_diff baselines, and refreshes the DirectX/OpenGL fallback snapshot!
pub fn on_scroll_invalidate(app: &AppHandle) {
    hitmap::invalidate_visual_hitmaps_only();
    enricher::clear_session_cache();
    hover_diff::on_selection_session_end();
    brush_scan::clear_revealed_cutouts(app);

    let last_pt = if let Ok(mut st) = ctrl().state.lock() {
        st.scouted_tiles.clear();
        st.dose1_tiles.clear();
        st.shake.clear();
        st.fallback_snapshot = None;
        st.last_cursor
    } else {
        None
    };

    // Refresh fallback snapshot asynchronously after 140ms so scroll animations settle
    // and DirectX/OpenGL windows never lose fallback capture after a wheel scroll!
    if let Some((lx, ly)) = last_pt {
        let gen = ctrl().session_gen.load(Ordering::Acquire);
        let _ = std::thread::Builder::new()
            .name("atp-scroll-resnap".to_string())
            .spawn(move || {
                std::thread::sleep(Duration::from_millis(140));
                if ctrl().session_gen.load(Ordering::Acquire) != gen {
                    return;
                }
                if let Some(fresh) = capture_monitor_snapshot_at(lx, ly) {
                    if ctrl().session_gen.load(Ordering::Acquire) == gen {
                        if let Ok(mut st) = ctrl().state.lock() {
                            if st.active && st.fallback_snapshot.is_none() {
                                st.fallback_snapshot = Some(fresh);
                            }
                        }
                    }
                }
            });
    }
}

/// Called on `PenEnd` — bumps generation (discarding any stale in-flight background work),
/// cleans up session state, and stops any in-flight drawer crawl.
pub fn on_session_end() {
    let c = ctrl();
    c.session_gen.fetch_add(1, Ordering::AcqRel);
    brush_scan::on_selection_session_end();
    enricher::clear_session_cache();
    if let Ok(mut st) = c.state.lock() {
        st.active = false;
        st.fallback_snapshot = None;
        st.scouted_tiles.clear();
        st.dose1_tiles.clear();
        st.shake.clear();
        st.last_cursor = None;
    }
    if let Ok(mut j) = c.latest_job.lock() {
        *j = None;
    }
}

/// Returns the clean fallback snapshot captured at session start (or refreshed after scroll).
pub fn current_fallback_snapshot() -> Option<Arc<RawScreenCapture>> {
    ctrl()
        .state
        .lock()
        .ok()
        .and_then(|st| st.fallback_snapshot.clone())
}

/// Called on every `PenHover(x, y)` inside Selection Mode:
/// 1. Schedules latest-wins Dose 0 Stealth Scout (with velocity lookahead)
/// 2. Feeds `ShakeDoseDetector`: stationary aiming NEVER triggers; deliberate shake escalates
///    Dose 1 -> Dose 2 continuously!
pub fn on_hover_point(
    app: &AppHandle,
    x: i32,
    y: i32,
    own_pid: u32,
    on_refresh: Arc<dyn Fn(i32, i32) + Send + Sync>,
) {
    let c = ctrl();
    let gen = c.session_gen.load(Ordering::Acquire);
    let now = Instant::now();
    let (lookahead_pt, escalation) = {
        let Ok(mut st) = c.state.lock() else {
            return;
        };
        if !st.active {
            return;
        }
        let la = if let Some((lx, ly)) = st.last_cursor {
            let dx = x - lx;
            let dy = y - ly;
            let dist = ((dx * dx + dy * dy) as f64).sqrt();
            if dist >= 6.0 {
                let step = 48.0f64.min(dist * 2.0);
                (
                    x + ((dx as f64) / dist * step).round() as i32,
                    y + ((dy as f64) / dist * step).round() as i32,
                )
            } else {
                (x, y)
            }
        } else {
            (x, y)
        };
        st.last_cursor = Some((x, y));
        // Only observe shake escalation if global worker is not busy (avoids needless cell transitions)
        let esc = if !c.dose_inflight.load(Ordering::Acquire) {
            st.shake.observe(x, y, now)
        } else {
            None
        };
        (la, esc)
    };

    // Enqueue Dose 0 background Stealth Scout (non-blocking, latest-wins)
    if let Ok(mut slot) = c.latest_job.lock() {
        *slot = Some((
            app.clone(),
            gen,
            lookahead_pt.0,
            lookahead_pt.1,
            own_pid,
            on_refresh.clone(),
        ));
        c.cv.notify_one();
    }

    // Handle Shake-to-Wake escalations asynchronously
    if let Some(esc) = escalation {
        match esc {
            DoseEscalation::Dose1Wake { cell, x: sx, y: sy } => {
                spawn_dose1_wake(app, gen, cell, sx, sy, own_pid, on_refresh);
            }
            DoseEscalation::Dose2Boost { cell, x: sx, y: sy } => {
                spawn_dose2_boost(app, gen, cell, sx, sy, own_pid, on_refresh);
            }
        }
    }
}

/// Dose 0 background Stealth Scout:
/// - Always zero-VRAM lightweight (`Win32 Native` + `MSAA` + `0ms StaticVisual`) by default!
/// - Only wakes `OmniParser v2` automatically when **Scheme B** (`profile::should_auto_wake_omniparser`)
///   previously learned that this `(exe, container_class)` benefits from it AND GPU auto-wake is enabled.
fn run_dose0_stealth_scout(
    _app: &AppHandle,
    gen: u64,
    x: i32,
    y: i32,
    own_pid: u32,
    on_refresh: &(dyn Fn(i32, i32) + Send + Sync),
) {
    let (root_hwnd, target_pid, _win_title, _win_class) = crate::pick::resolve_root_window_at(x, y);
    if target_pid == 0 || (own_pid != 0 && target_pid == own_pid) {
        return;
    }
    let target_exe = office::fast_exe_basename_for_pid(target_pid);
    if target_exe.is_empty() {
        return;
    }
    if solidworks::is_solidworks_exe(&target_exe)
        && solidworks::classify_solidworks_region_at(root_hwnd, x, y)
            == solidworks::SwWindowRegion::Viewport3D
    {
        return;
    }

    let prof = profile::get_effective_profile(&target_exe);
    let (child_hwnd, child_class, child_rect) = crate::pick::resolve_child_hwnd_at(root_hwnd, x, y);
    if child_rect.w < 16 || child_rect.h < 14 {
        return;
    }
    let cls_lower = child_class.to_ascii_lowercase();
    let is_popup = solidworks::is_popup_menu_hwnd(child_hwnd, root_hwnd, &cls_lower);
    if !is_popup
        && prof
            .ignore_canvas_classes
            .iter()
            .any(|c| !c.is_empty() && cls_lower.contains(c))
    {
        return;
    }

    let gpu_auto_ok = ctrl()
        .state
        .lock()
        .map(|s| vision::sidecar_client::is_gpu_auto_wake_permitted(&s.cached_vision_cfg))
        .unwrap_or(false);
    let scheme_b_auto_wake =
        gpu_auto_ok && profile::should_auto_wake_omniparser(&target_exe, &cls_lower);

    if prof.uia_policy == UiaPolicy::Preferred && !scheme_b_auto_wake && !is_popup {
        return;
    }

    let (tile_key, cap_rect) = compute_scout_tile(child_hwnd, child_rect, is_popup, x, y);
    let newly_claimed = ctrl()
        .state
        .lock()
        .map(|mut st| st.scouted_tiles.insert(tile_key))
        .unwrap_or(false);
    if !newly_claimed {
        return;
    }

    let hwnd_raw = child_hwnd.0 as isize;
    let mut populated = false;

    // 1. Zero-UIA Native Win32 direct control reader (`#32768` menus, `ToolbarWindow32`, `SysTabControl32`)
    let native_items = win32_controls::read_native_win32_control_entries(child_hwnd, &child_class);
    if !native_items.is_empty() {
        hitmap::merge_entries_into_hitmap(
            hwnd_raw,
            child_rect,
            HitMapSource::MsaaSnapshot,
            native_items,
        );
        populated = true;
    }

    // 2. Fast container MSAA batch snapshot
    let msaa_items = hitmap::build_msaa_entries(child_hwnd, child_rect);
    if !msaa_items.is_empty() {
        hitmap::merge_entries_into_hitmap(
            hwnd_raw,
            child_rect,
            HitMapSource::MsaaSnapshot,
            msaa_items,
        );
        populated = true;
    }

    // 3. Visual segmentation on overlay-free PrintWindow / fallback snapshot
    if cap_rect.w >= 20 && cap_rect.h >= 16 {
        let fallback = current_fallback_snapshot();
        if let Some((img, ox, oy)) =
            capture_clean_region_no_overlay(root_hwnd, cap_rect, fallback.as_deref())
        {
            if ctrl().session_gen.load(Ordering::Acquire) != gen {
                return;
            }
            let win_rect = crate::pick::get_window_screen_rect_pub(root_hwnd);
            let mut ran_omni = false;
            if scheme_b_auto_wake && hitmap::active_visual_box_provider_name().is_some() {
                let omni_rects = hitmap::detect_with_registered_box_provider(&img, ox, oy);
                if ctrl().session_gen.load(Ordering::Acquire) != gen {
                    return;
                }
                if !omni_rects.is_empty() {
                    let omni_items: Vec<HitMapEntry> = omni_rects
                        .into_iter()
                        .map(|rect| {
                            let trig = hitmap::detect_expandable_trigger_in_rgba(
                                &img, ox, oy, rect, win_rect,
                            );
                            HitMapEntry {
                                rect,
                                label: String::new(),
                                description: "omniparser_v2".to_string(),
                                role: "Button".to_string(),
                                source: HitMapSource::ExternalBoxProvider,
                                is_expandable: trig.is_some(),
                                trigger_pt: trig,
                            }
                        })
                        .collect();
                    hitmap::merge_entries_into_hitmap(
                        hwnd_raw,
                        child_rect,
                        HitMapSource::ExternalBoxProvider,
                        omni_items,
                    );
                    ran_omni = true;
                    populated = true;
                }
            }
            if !ran_omni {
                let (_layout, visual_items) = hitmap::segment_container_auto_in_rgba(
                    &img,
                    ox,
                    oy,
                    &child_class,
                    is_popup,
                    ContainerLayoutKind::Auto,
                );
                if !visual_items.is_empty() {
                    hitmap::merge_entries_into_hitmap(
                        hwnd_raw,
                        child_rect,
                        HitMapSource::StaticVisual,
                        visual_items,
                    );
                    populated = true;
                }
            }
        }
    }

    if populated && ctrl().session_gen.load(Ordering::Acquire) == gen {
        if let Some((cx, cy)) = ctrl().state.lock().ok().and_then(|s| s.last_cursor) {
            on_refresh(cx, cy);
        }
    }
}

/// Dose 1: Wakes `OmniParser v2` + `Hover-Diff` + Deep UIA on the owning container/tile,
/// stores results silently into `HITMAP_CACHE`, learns Scheme B preference, and refreshes hover!
fn spawn_dose1_wake(
    app: &AppHandle,
    gen: u64,
    cell: DoseCellKey,
    x: i32,
    y: i32,
    own_pid: u32,
    on_refresh: Arc<dyn Fn(i32, i32) + Send + Sync>,
) {
    // SolidWorks 3D Viewport uses native COM `SelectByRay` directly — NEVER wake OmniParser,
    // Hover-Diff, or visual probes when moving/shaking over the 3D CAD canvas!
    if let Some(sw_hit) = solidworks::detect_solidworks_at(x, y) {
        if solidworks::classify_solidworks_region_at(sw_hit.root_hwnd, x, y)
            == solidworks::SwWindowRegion::Viewport3D
        {
            if let Ok(mut st) = ctrl().state.lock() {
                st.shake.revert_dose_inflight(cell);
            }
            return;
        }
    }

    let c = ctrl();
    if c.dose_inflight
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        if let Ok(mut st) = c.state.lock() {
            st.shake.revert_dose_inflight(cell);
        }
        return;
    }

    emit_overlay(
        app,
        "pen:scout_pulse",
        serde_json::json!({ "tier": 1, "x": x, "y": y }),
    );
    self_heal::emit_status_hint(app, "已摇醒深度探针…", Some((x, y)), "working", 1800);

    let app_c = app.clone();
    let spawn_res = std::thread::Builder::new()
        .name("atp-dose1-wake".to_string())
        .spawn(move || {
            let _guard = DoseInflightGuard(&ctrl().dose_inflight);
            let t0 = Instant::now();
            let mut newly_added = 0usize;
            let (root_hwnd, target_pid, _win_title, win_class) =
                crate::pick::resolve_root_window_at(x, y);

            if target_pid != 0 && (own_pid == 0 || target_pid != own_pid) {
                let target_exe = office::fast_exe_basename_for_pid(target_pid);
                let prof = profile::get_effective_profile(&target_exe);
                let win_rect = crate::pick::get_window_screen_rect_pub(root_hwnd);
                let (child_hwnd, child_class, child_rect) =
                    crate::pick::resolve_child_hwnd_at(root_hwnd, x, y);
                let cls_lower = child_class.to_ascii_lowercase();
                let is_popup = solidworks::is_popup_menu_hwnd(child_hwnd, root_hwnd, &cls_lower);
                let hwnd_raw = child_hwnd.0 as isize;
                let (tile_key, cap_rect) =
                    compute_scout_tile(child_hwnd, child_rect, is_popup, x, y);

                if let Ok(mut st) = ctrl().state.lock() {
                    st.dose1_tiles.insert(tile_key);
                }

                // 1. OmniParser v2 on overlay-free PrintWindow / fallback snapshot
                let mut omni_count = 0usize;
                let mut static_count = 0usize;
                if cap_rect.w >= 20 && cap_rect.h >= 16 {
                    let fallback = current_fallback_snapshot();
                    if let Some((img, ox, oy)) =
                        capture_clean_region_no_overlay(root_hwnd, cap_rect, fallback.as_deref())
                    {
                        let omni_rects =
                            hitmap::detect_with_registered_box_provider(&img, ox, oy);
                        if ctrl().session_gen.load(Ordering::Acquire) != gen {
                            return;
                        }
                        omni_count = omni_rects.len();
                        if !omni_rects.is_empty() {
                            let omni_items: Vec<HitMapEntry> = omni_rects
                                .into_iter()
                                .map(|rect| {
                                    let trig = hitmap::detect_expandable_trigger_in_rgba(
                                        &img, ox, oy, rect, win_rect,
                                    );
                                    HitMapEntry {
                                        rect,
                                        label: String::new(),
                                        description: "omniparser_v2".to_string(),
                                        role: "Button".to_string(),
                                        source: HitMapSource::ExternalBoxProvider,
                                        is_expandable: trig.is_some(),
                                        trigger_pt: trig,
                                    }
                                })
                                .collect();
                            hitmap::merge_entries_into_hitmap(
                                hwnd_raw,
                                child_rect,
                                HitMapSource::ExternalBoxProvider,
                                omni_items,
                            );
                            newly_added += omni_count;
                        }
                        static_count =
                            hitmap::get_cached_entries(hwnd_raw, HitMapSource::StaticVisual)
                                .map(|v| v.len())
                                .unwrap_or(0);
                        if omni_count == 0 && static_count == 0 {
                            let (_l, vis) = hitmap::segment_container_auto_in_rgba(
                                &img,
                                ox,
                                oy,
                                &child_class,
                                is_popup,
                                ContainerLayoutKind::Auto,
                            );
                            static_count = vis.len();
                            if !vis.is_empty() {
                                hitmap::merge_entries_into_hitmap(
                                    hwnd_raw,
                                    child_rect,
                                    HitMapSource::StaticVisual,
                                    vis,
                                );
                                newly_added += static_count;
                            }
                        }
                    }
                }

                // 2. Dynamic Hover-Diff probe at current cursor (catches hover-tint / 1px-border buttons!)
                if ctrl().session_gen.load(Ordering::Acquire) == gen {
                    if let Some((cur_x, cur_y)) =
                        ctrl().state.lock().ok().and_then(|s| s.last_cursor)
                    {
                        if !(solidworks::is_solidworks_exe(&target_exe)
                            && solidworks::classify_solidworks_region_at(root_hwnd, cur_x, cur_y)
                                == solidworks::SwWindowRegion::Viewport3D)
                        {
                            if let Some(hd) = hover_diff::probe_hover_reactive_button(
                                root_hwnd, child_hwnd, cur_x, cur_y,
                            ) {
                                hitmap::merge_entries_into_hitmap(
                                    hwnd_raw,
                                    child_rect,
                                    HitMapSource::ExternalBoxProvider,
                                    vec![HitMapEntry {
                                        rect: hd.rect,
                                        label: String::new(),
                                        description: "hover_diff".to_string(),
                                        role: "Button".to_string(),
                                        source: HitMapSource::ExternalBoxProvider,
                                        is_expandable: false,
                                        trigger_pt: None,
                                    }],
                                );
                                newly_added += 1;
                            }
                        }
                    }
                }

                // 3. Deep UIA sibling/descendant pass for modern UIA apps (NEVER on SolidWorks!)
                if ctrl().session_gen.load(Ordering::Acquire) == gen
                    && prof.uia_policy == UiaPolicy::Preferred
                    && !solidworks::is_solidworks_exe(&target_exe)
                {
                    let deep_uia = brush_scan::sample_deep_uia_around_point(
                        root_hwnd,
                        target_pid,
                        win_rect,
                        &win_class,
                        x,
                        y,
                    );
                    if !deep_uia.is_empty() {
                        newly_added += deep_uia.len();
                        hitmap::merge_entries_into_hitmap(
                            hwnd_raw,
                            child_rect,
                            HitMapSource::MsaaSnapshot,
                            deep_uia,
                        );
                    }
                }

                // 4. Scheme B strategy persistence (NO coordinates! Only `(exe, container_class)`!)
                if omni_count >= 3
                    && omni_count > static_count
                    && !target_exe.is_empty()
                    && !cls_lower.is_empty()
                {
                    let _ =
                        profile::record_auto_wake_omniparser(&target_exe, &cls_lower, omni_count);
                    log::info!(
                        "[SCOUT-SCHEME-B] learned auto_wake_omniparser for exe={} cls={} (omni={} vs static={})",
                        target_exe,
                        cls_lower,
                        omni_count,
                        static_count
                    );
                }
            }

            if ctrl().session_gen.load(Ordering::Acquire) != gen {
                return;
            }
            let elapsed = t0.elapsed().as_millis();
            if let Ok(mut st) = ctrl().state.lock() {
                st.shake.mark_dose1_complete(cell, Instant::now());
            }

            if newly_added > 0 {
                let msg = format!("深度探针就绪（补全 {newly_added} 处，{elapsed}ms）· 续摇可加剂量");
                self_heal::emit_status_hint(&app_c, &msg, Some((x, y)), "done", 2000);
            } else {
                self_heal::emit_status_hint(
                    &app_c,
                    "已深度扫描 · 若为照片/多级菜单请继续摇晃",
                    Some((x, y)),
                    "info",
                    2000,
                );
            }

            if let Some((cx, cy)) = ctrl().state.lock().ok().and_then(|s| s.last_cursor) {
                on_refresh(cx, cy);
            }
        });

    if spawn_res.is_err() {
        c.dose_inflight.store(false, Ordering::Release);
        if let Ok(mut st) = c.state.lock() {
            st.shake.revert_dose_inflight(cell);
        }
    }
}

/// Dose 2: User kept shaking AFTER Dose 1 results landed -> escalate to heavy specialists:
/// 1. Nearby expandable `▾`/`▸` menu trigger -> safe multi-level drawer crawl
/// 2. Natural photograph / landscape -> full-scene SAM 2.1 organic semantic matting
/// 3. Stubborn custom UI (when `rebrush_vlm_audit` is enabled) -> VLM additive arbitration
fn spawn_dose2_boost(
    app: &AppHandle,
    gen: u64,
    cell: DoseCellKey,
    x: i32,
    y: i32,
    own_pid: u32,
    on_refresh: Arc<dyn Fn(i32, i32) + Send + Sync>,
) {
    // SolidWorks 3D Viewport / FeatureTree are served by native COM probes — NEVER run SAM 2.1,
    // drawer crawl, or VLM visual arbitration over them!
    if let Some(sw_hit) = solidworks::detect_solidworks_at(x, y) {
        if matches!(
            solidworks::classify_solidworks_region_at(sw_hit.root_hwnd, x, y),
            solidworks::SwWindowRegion::Viewport3D | solidworks::SwWindowRegion::FeatureTree
        ) {
            if let Ok(mut st) = ctrl().state.lock() {
                st.shake.revert_dose_inflight(cell);
            }
            return;
        }
    }

    let c = ctrl();
    if c.dose_inflight
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        if let Ok(mut st) = c.state.lock() {
            st.shake.revert_dose_inflight(cell);
        }
        return;
    }

    emit_overlay(
        app,
        "pen:scout_pulse",
        serde_json::json!({ "tier": 2, "x": x, "y": y }),
    );

    let app_c = app.clone();
    let spawn_res = std::thread::Builder::new()
        .name("atp-dose2-boost".to_string())
        .spawn(move || {
            let _guard = DoseInflightGuard(&ctrl().dose_inflight);
            let (root_hwnd, target_pid, _win_title, _win_class) =
                crate::pick::resolve_root_window_at(x, y);
            if target_pid != 0 && (own_pid == 0 || target_pid != own_pid) {
                let target_exe = office::fast_exe_basename_for_pid(target_pid);
                let win_rect = crate::pick::get_window_screen_rect_pub(root_hwnd);
                let (child_hwnd, _child_class, child_rect) =
                    crate::pick::resolve_child_hwnd_at(root_hwnd, x, y);
                let hwnd_raw = child_hwnd.0 as isize;

                // Branch 2A: Expandable `▾`/`▸` trigger nearby -> safe multi-level drawer crawl!
                let expandable_hit = hitmap::lookup_cached_any_source(hwnd_raw, x, y)
                    .filter(|e| e.is_expandable && e.trigger_pt.is_some());
                if let Some(exp) = expandable_hit {
                    if let Some(tpt) = exp.trigger_pt {
                        self_heal::emit_status_hint(
                            &app_c,
                            "续加剂量：正在展开多级子菜单…",
                            Some(tpt),
                            "working",
                            2400,
                        );
                        brush_scan::expand_single_trigger_safe(
                            &app_c,
                            &target_exe,
                            target_pid,
                            tpt,
                            exp.rect,
                        );
                        if ctrl().session_gen.load(Ordering::Acquire) == gen {
                            if let Ok(mut st) = ctrl().state.lock() {
                                st.shake.mark_dose2_complete(cell, Instant::now());
                            }
                            if let Some((cx, cy)) =
                                ctrl().state.lock().ok().and_then(|s| s.last_cursor)
                            {
                                on_refresh(cx, cy);
                            }
                        }
                        return;
                    }
                }

                // Branch 2B: Natural photograph / landscape -> full-scene SAM 2.1 organic semantic matting!
                let cfg = AppConfig::load_or_create().vision;
                let photo_probe = ScreenRect {
                    x: (x - 480).max(win_rect.x),
                    y: (y - 320).max(win_rect.y),
                    w: 960.min(win_rect.w.max(360)),
                    h: 640.min(win_rect.h.max(280)),
                };
                let fallback = current_fallback_snapshot();
                if cfg.auto_photo_matting
                    && (cfg.segmentation_backend != SegmentationBackend::Disabled
                        || cfg.vlm_backend != VlmBackend::Disabled)
                {
                    if let Some((crop, ox, oy)) = capture_clean_region_no_overlay(
                        root_hwnd,
                        photo_probe,
                        fallback.as_deref(),
                    ) {
                        if vision::is_natural_photo_region(&crop) {
                            self_heal::emit_status_hint(
                                &app_c,
                                "续加剂量：正在对照片区域语义抠图…",
                                Some((x, y)),
                                "working",
                                4500,
                            );
                            let local_pt = [
                                (x - ox).clamp(0, crop.width() as i32 - 1),
                                (y - oy).clamp(0, crop.height() as i32 - 1),
                            ];
                            let seg_res =
                                vision::segment_natural_photo(&cfg, &crop, ox, oy, &[local_pt]);
                            if ctrl().session_gen.load(Ordering::Acquire) != gen {
                                return;
                            }
                            match seg_res {
                                Ok(out) if !out.items.is_empty() => {
                                    let cnt = out.items.len();
                                    brush_scan::publish_photo_cutouts(&app_c, out.items);
                                    let msg = format!(
                                        "照片语义抠图完成：浮出 {cnt} 个轮廓（{}，{}ms）",
                                        out.backend_used, out.latency_ms
                                    );
                                    self_heal::emit_status_hint(
                                        &app_c,
                                        &msg,
                                        Some((x, y)),
                                        "done",
                                        2800,
                                    );
                                }
                                _ => {
                                    self_heal::emit_status_hint(
                                        &app_c,
                                        "照片区未检出独立轮廓",
                                        Some((x, y)),
                                        "info",
                                        1800,
                                    );
                                }
                            }
                            if let Ok(mut st) = ctrl().state.lock() {
                                st.shake.mark_dose2_complete(cell, Instant::now());
                            }
                            return;
                        }
                    }
                }

                // Branch 2C: Stubborn custom UI -> VLM Additive Visual Arbitration (strictly gated on `rebrush_vlm_audit`!)
                if cfg.rebrush_vlm_audit && cfg.vlm_backend != VlmBackend::Disabled {
                    let vlm_crop = ScreenRect {
                        x: (x - 240).max(win_rect.x),
                        y: (y - 160).max(win_rect.y),
                        w: 480.min(win_rect.w.max(180)),
                        h: 320.min(win_rect.h.max(140)),
                    };
                    if let Some((crop, ox, oy)) =
                        capture_clean_region_no_overlay(root_hwnd, vlm_crop, fallback.as_deref())
                    {
                        self_heal::emit_status_hint(
                            &app_c,
                            "续加剂量：正在调用 VLM 视觉仲裁…",
                            Some((x, y)),
                            "working",
                            3500,
                        );
                        if let Ok(audit) = vision::audit_ui_region(&cfg, &crop, ox, oy, &[]) {
                            if ctrl().session_gen.load(Ordering::Acquire) != gen {
                                return;
                            }
                            if !audit.corrected_items.is_empty() {
                                let cnt = audit.corrected_items.len();
                                let entries: Vec<HitMapEntry> = audit
                                    .corrected_items
                                    .into_iter()
                                    .map(|it| HitMapEntry {
                                        rect: it.rect,
                                        label: it.label,
                                        description: String::new(),
                                        role: it.role,
                                        source: HitMapSource::ExternalBoxProvider,
                                        is_expandable: false,
                                        trigger_pt: None,
                                    })
                                    .collect();
                                hitmap::merge_entries_into_hitmap(
                                    hwnd_raw,
                                    child_rect,
                                    HitMapSource::ExternalBoxProvider,
                                    entries,
                                );
                                let msg = format!(
                                    "VLM 仲裁补齐 {cnt} 处（{}，{}ms）",
                                    audit.backend_used, audit.latency_ms
                                );
                                self_heal::emit_status_hint(
                                    &app_c,
                                    &msg,
                                    Some((x, y)),
                                    "done",
                                    2400,
                                );
                            }
                        }
                    }
                }
            }

            if ctrl().session_gen.load(Ordering::Acquire) != gen {
                return;
            }
            if let Ok(mut st) = ctrl().state.lock() {
                st.shake.mark_dose2_complete(cell, Instant::now());
            }
            if let Some((cx, cy)) = ctrl().state.lock().ok().and_then(|s| s.last_cursor) {
                on_refresh(cx, cy);
            }
        });

    if spawn_res.is_err() {
        c.dose_inflight.store(false, Ordering::Release);
        if let Ok(mut st) = c.state.lock() {
            st.shake.revert_dose_inflight(cell);
        }
    }
}
