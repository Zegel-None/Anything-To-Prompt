use crate::captures::{capture_hwnd_via_print_window, RawScreenCapture, ScreenRect};
use crate::pick::hitmap::{self, HitMapEntry, HitMapSource};
use image::RgbaImage;
use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HWND;

const ROI_HALF_W: i32 = 760;
const ROI_HALF_H: i32 = 180;
const PIXEL_L1_DIFF_THRESH: i32 = 16; // Catches subtle color/tint/border changes while ignoring 1-2 LSB noise
const MIN_BTN_W: i32 = 14;
const MIN_BTN_H: i32 = 14;
const MAX_BTN_W: i32 = 960;
const MAX_BTN_H: i32 = 240;
const MIN_CHANGED_PIXELS: usize = 20;
const MAX_OUTSIDE_CHANGE_RATIO: f64 = 0.22;
const CAPTURE_THROTTLE: Duration = Duration::from_millis(68);

#[derive(Debug, Clone)]
pub struct HoverDiffDetection {
    pub rect: ScreenRect,
    pub confirmed_reverted: bool,
    pub hwnd_raw: isize,
}

#[derive(Default)]
struct HoverDiffState {
    baselines: HashMap<isize, (RawScreenCapture, (i32, i32), bool)>, // (frame, initial_cursor, initial_patched)
    pending_candidates: Vec<(isize, ScreenRect)>,
    confirmed: Vec<(isize, ScreenRect)>,
    last_capture_at: Option<Instant>,
}

static HOVER_DIFF_STATE: OnceLock<Mutex<HoverDiffState>> = OnceLock::new();

fn state_lock() -> &'static Mutex<HoverDiffState> {
    HOVER_DIFF_STATE.get_or_init(|| Mutex::new(HoverDiffState::default()))
}

pub fn on_selection_session_end() {
    if let Ok(mut st) = state_lock().lock() {
        st.baselines.clear();
        st.pending_candidates.clear();
        st.confirmed.clear();
        st.last_capture_at = None;
    }
}

/// Returns true if almost all sampled pixels are pure black (e.g., PrintWindow on exclusive DirectX/OpenGL).
pub fn is_frame_mostly_black(img: &RgbaImage) -> bool {
    let w = img.width();
    let h = img.height();
    if w < 16 || h < 16 {
        return true;
    }
    let sx = (w / 8).max(1);
    let sy = (h / 8).max(1);
    let mut total = 0u32;
    let mut black = 0u32;
    let mut y = sy / 2;
    while y < h {
        let mut x = sx / 2;
        while x < w {
            let p = img.get_pixel(x, y).0;
            total += 1;
            if p[0] <= 2 && p[1] <= 2 && p[2] <= 2 {
                black += 1;
            }
            x += sx;
        }
        y += sy;
    }
    total > 0 && (black as f64 / total as f64) >= 0.98
}

#[inline]
fn pixel_l1_diff(a: [u8; 4], b: [u8; 4]) -> i32 {
    (a[0] as i32 - b[0] as i32).abs()
        + (a[1] as i32 - b[1] as i32).abs()
        + (a[2] as i32 - b[2] as i32).abs()
}

/// Pure geometric & pixel-difference button detector:
/// Compares `baseline` (idle window) against `live` (cursor hovering at `(cursor_x, cursor_y)`).
/// Detects both full-button hover color fills AND thin 1px hover border highlights while rejecting
/// detached tooltips, scrolling pages, and full-screen video playback.
pub fn extract_hover_reactive_rect(
    baseline: &RgbaImage,
    live: &RgbaImage,
    origin_x: i32,
    origin_y: i32,
    cursor_x: i32,
    cursor_y: i32,
) -> Option<ScreenRect> {
    if baseline.dimensions() != live.dimensions() {
        return None;
    }
    let iw = baseline.width() as i32;
    let ih = baseline.height() as i32;
    let cx = cursor_x - origin_x;
    let cy = cursor_y - origin_y;
    if cx < 0 || cy < 0 || cx >= iw || cy >= ih {
        return None;
    }

    let rx0 = (cx - ROI_HALF_W).clamp(0, iw - 1);
    let ry0 = (cy - ROI_HALF_H).clamp(0, ih - 1);
    let rx1 = (cx + ROI_HALF_W).clamp(rx0 + 1, iw);
    let ry1 = (cy + ROI_HALF_H).clamp(ry0 + 1, ih);
    let rw = (rx1 - rx0) as usize;
    let rh = (ry1 - ry0) as usize;
    if rw < MIN_BTN_W as usize || rh < MIN_BTN_H as usize {
        return None;
    }

    // Build 2x2 downsampled change grid for ultra-fast (<0.3ms) connected-component analysis
    let gw = (rw + 1) / 2;
    let gh = (rh + 1) / 2;
    let mut grid = vec![0u8; gw * gh];
    let mut total_changed_cells = 0usize;

    for dy in 0..rh {
        let py = (ry0 + dy as i32) as u32;
        let gy = dy / 2;
        for dx in 0..rw {
            let px = (rx0 + dx as i32) as u32;
            let diff = pixel_l1_diff(baseline.get_pixel(px, py).0, live.get_pixel(px, py).0);
            if diff >= PIXEL_L1_DIFF_THRESH {
                let idx = gy * gw + (dx / 2);
                if grid[idx] == 0 {
                    grid[idx] = 1;
                    total_changed_cells += 1;
                }
            }
        }
    }

    if total_changed_cells < (MIN_CHANGED_PIXELS / 4).max(4) {
        return None;
    }

    // 1-cell (2px) dilation bridges subtle 1px hover borders + icon shifts into one cohesive button
    let mut dilated = grid.clone();
    for gy in 0..gh {
        for gx in 0..gw {
            if grid[gy * gw + gx] == 1 {
                let y0 = gy.saturating_sub(1);
                let y1 = (gy + 1).min(gh - 1);
                let x0 = gx.saturating_sub(1);
                let x1 = (gx + 1).min(gw - 1);
                for ny in y0..=y1 {
                    for nx in x0..=x1 {
                        dilated[ny * gw + nx] = 1;
                    }
                }
            }
        }
    }

    let cursor_gx = ((cx - rx0) / 2).clamp(0, gw as i32 - 1) as usize;
    let cursor_gy = ((cy - ry0) / 2).clamp(0, gh as i32 - 1) as usize;

    // Extract all connected components and find the one enclosing `(cursor_gx, cursor_gy)`
    let mut visited = vec![false; gw * gh];
    let mut best_comp: Option<(usize, usize, usize, usize, usize)> = None; // (min_x, min_y, max_x, max_y, orig_cells)
    let mut q: VecDeque<(usize, usize)> = VecDeque::new();

    for sy in 0..gh {
        for sx in 0..gw {
            let sidx = sy * gw + sx;
            if dilated[sidx] == 0 || visited[sidx] {
                continue;
            }
            visited[sidx] = true;
            q.push_back((sx, sy));
            let (mut min_x, mut max_x) = (sx, sx);
            let (mut min_y, mut max_y) = (sy, sy);
            let mut orig_cells = 0usize;

            while let Some((x, y)) = q.pop_front() {
                if grid[y * gw + x] == 1 {
                    orig_cells += 1;
                }
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);

                let ny0 = y.saturating_sub(1);
                let ny1 = (y + 1).min(gh - 1);
                let nx0 = x.saturating_sub(1);
                let nx1 = (x + 1).min(gw - 1);
                for ny in ny0..=ny1 {
                    for nx in nx0..=nx1 {
                        let nidx = ny * gw + nx;
                        if dilated[nidx] == 1 && !visited[nidx] {
                            visited[nidx] = true;
                            q.push_back((nx, ny));
                        }
                    }
                }
            }

            // Must enclose cursor (within 3 grid cells = 6px margin, handles hollow border-only hover!)
            let encloses_cursor = cursor_gx + 3 >= min_x
                && cursor_gx <= max_x + 3
                && cursor_gy + 3 >= min_y
                && cursor_gy <= max_y + 3;
            if encloses_cursor {
                let area = (max_x - min_x + 1) * (max_y - min_y + 1);
                let replace = best_comp.is_none_or(|(bx0, by0, bx1, by1, _)| {
                    area < (bx1 - bx0 + 1) * (by1 - by0 + 1)
                });
                if replace {
                    best_comp = Some((min_x, min_y, max_x, max_y, orig_cells));
                }
            }
        }
    }

    let (cmin_x, cmin_y, cmax_x, cmax_y, comp_orig_cells) = best_comp?;

    // Anti-video / anti-scroll guard:
    // 1. Reject if the single changed blob saturates >= 65% of the entire local ROI (page scroll / video);
    // 2. Reject if large parts of the ROI OUTSIDE this button also changed!
    let total_roi_cells = (gw * gh).max(1);
    let comp_box_cells = (cmax_x - cmin_x + 1) * (cmax_y - cmin_y + 1);
    if (comp_box_cells as f64 / total_roi_cells as f64) >= 0.65 {
        return None;
    }
    let outside_roi_cells = total_roi_cells.saturating_sub(comp_box_cells).max(1);
    let outside_changed_cells = total_changed_cells.saturating_sub(comp_orig_cells);
    if (outside_changed_cells as f64 / outside_roi_cells as f64) > MAX_OUTSIDE_CHANGE_RATIO {
        return None;
    }

    // Refine exact 1px tight bounds on full-resolution diff inside the winning component bbox
    let fx0 = (rx0 + (cmin_x as i32) * 2).clamp(rx0, rx1 - 1);
    let fy0 = (ry0 + (cmin_y as i32) * 2).clamp(ry0, ry1 - 1);
    let fx1 = (rx0 + (cmax_x as i32 + 1) * 2).clamp(fx0 + 1, rx1);
    let fy1 = (ry0 + (cmax_y as i32 + 1) * 2).clamp(fy0 + 1, ry1);

    let mut tmin_x = fx1;
    let mut tmin_y = fy1;
    let mut tmax_x = fx0;
    let mut tmax_y = fy0;
    let mut exact_changed = 0usize;

    for py in fy0..fy1 {
        for px in fx0..fx1 {
            if pixel_l1_diff(
                baseline.get_pixel(px as u32, py as u32).0,
                live.get_pixel(px as u32, py as u32).0,
            ) >= PIXEL_L1_DIFF_THRESH
            {
                exact_changed += 1;
                tmin_x = tmin_x.min(px);
                tmax_x = tmax_x.max(px);
                tmin_y = tmin_y.min(py);
                tmax_y = tmax_y.max(py);
            }
        }
    }

    if exact_changed < MIN_CHANGED_PIXELS || tmax_x <= tmin_x || tmax_y <= tmin_y {
        return None;
    }

    let w = tmax_x - tmin_x + 1;
    let h = tmax_y - tmin_y + 1;
    if !(MIN_BTN_W..=MAX_BTN_W).contains(&w) || !(MIN_BTN_H..=MAX_BTN_H).contains(&h) {
        return None;
    }

    let rect = ScreenRect {
        x: origin_x + tmin_x,
        y: origin_y + tmin_y,
        w,
        h,
    };

    // Final check: cursor must lie inside (or within 4px of) the refined button rect
    if cursor_x < rect.x - 4
        || cursor_x > rect.x + rect.w + 4
        || cursor_y < rect.y - 4
        || cursor_y > rect.y + rect.h + 4
    {
        return None;
    }

    Some(rect)
}

/// Verifies the "Leave-Revert" proof: after cursor moves away from `rect`, did the pixels inside `rect`
/// return back to the idle `baseline`?
pub fn verify_rect_reverted(
    baseline: &RgbaImage,
    live: &RgbaImage,
    origin_x: i32,
    origin_y: i32,
    rect: ScreenRect,
) -> bool {
    if baseline.dimensions() != live.dimensions() || rect.w <= 0 || rect.h <= 0 {
        return false;
    }
    let iw = baseline.width() as i32;
    let ih = baseline.height() as i32;
    let x0 = (rect.x - origin_x).clamp(0, iw - 1);
    let y0 = (rect.y - origin_y).clamp(0, ih - 1);
    let x1 = (rect.x + rect.w - origin_x).clamp(x0 + 1, iw);
    let y1 = (rect.y + rect.h - origin_y).clamp(y0 + 1, ih);

    let step_x = ((x1 - x0) / 24).max(1);
    let step_y = ((y1 - y0) / 16).max(1);
    let mut total = 0usize;
    let mut matching = 0usize;
    let mut py = y0;
    while py < y1 {
        let mut px = x0;
        while px < x1 {
            total += 1;
            if pixel_l1_diff(
                baseline.get_pixel(px as u32, py as u32).0,
                live.get_pixel(px as u32, py as u32).0,
            ) < PIXEL_L1_DIFF_THRESH
            {
                matching += 1;
            }
            px += step_x;
        }
        py += step_y;
    }
    total >= 8 && (matching as f64 / total as f64) >= 0.88
}

fn patch_baseline_region(
    baseline: &mut RawScreenCapture,
    live: &RawScreenCapture,
    center_x: i32,
    center_y: i32,
    half_size: i32,
) {
    if baseline.image.dimensions() != live.image.dimensions() {
        return;
    }
    let iw = baseline.image.width() as i32;
    let ih = baseline.image.height() as i32;
    let x0 = (center_x - baseline.origin_x - half_size).clamp(0, iw - 1);
    let y0 = (center_y - baseline.origin_y - half_size).clamp(0, ih - 1);
    let x1 = (center_x - baseline.origin_x + half_size).clamp(x0, iw);
    let y1 = (center_y - baseline.origin_y + half_size).clamp(y0, ih);
    for py in y0..y1 {
        for px in x0..x1 {
            let p = *live.image.get_pixel(px as u32, py as u32);
            baseline.image.put_pixel(px as u32, py as u32, p);
        }
    }
}

/// Additive live probe: detects a hover-reactive button under `(x, y)` by comparing a clean
/// `PrintWindow` frame of the target window against its idle baseline (100% immune to our overlay!).
/// Automatically confirms previous candidates via Leave-Revert and promotes verified buttons into `HITMAP_CACHE`.
pub fn probe_hover_reactive_button(
    root_hwnd: HWND,
    child_hwnd: HWND,
    x: i32,
    y: i32,
) -> Option<HoverDiffDetection> {
    // Prefer compact child Win32 toolbar HWND when valid (2-5ms!), else fall back to root_hwnd
    let target_hwnd = if !child_hwnd.0.is_null() && child_hwnd.0 != root_hwnd.0 {
        child_hwnd
    } else {
        root_hwnd
    };
    if target_hwnd.0.is_null() {
        return None;
    }
    let hwnd_raw = target_hwnd.0 as isize;

    // 1. Instant 0ms hit if we already leave-revert confirmed a hover button here in this session
    {
        if let Ok(st) = state_lock().lock() {
            if let Some((_, rect)) = st.confirmed.iter().find(|(h, r)| {
                *h == hwnd_raw && x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h
            }) {
                return Some(HoverDiffDetection {
                    rect: *rect,
                    confirmed_reverted: true,
                    hwnd_raw,
                });
            }
            if st
                .last_capture_at
                .is_some_and(|t| t.elapsed() < CAPTURE_THROTTLE)
            {
                // Also return any active pending candidate under cursor during throttle window
                if let Some((_, rect)) = st.pending_candidates.iter().find(|(h, r)| {
                    *h == hwnd_raw && x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h
                }) {
                    return Some(HoverDiffDetection {
                        rect: *rect,
                        confirmed_reverted: false,
                        hwnd_raw,
                    });
                }
                return None;
            }
        }
    }

    // 2. Capture live overlay-free window frame: use lock-free DWM screen BitBlt for SolidWorks
    // (never send synchronous `WM_PRINT` to SLDWORKS.exe's STA UI/COM thread!), PrintWindow elsewhere.
    let live = if crate::captures::is_print_window_unsafe_hwnd(root_hwnd) {
        let mut rc = windows::Win32::Foundation::RECT::default();
        let ok = unsafe {
            windows::Win32::UI::WindowsAndMessaging::GetWindowRect(target_hwnd, &mut rc).is_ok()
        };
        let w = (rc.right - rc.left).max(0);
        let h = (rc.bottom - rc.top).max(0);
        if !ok || w < 16 || h < 16 {
            return None;
        }
        match crate::captures::capture_screen_rect_rgba(rc.left, rc.top, w, h) {
            Ok(c) if !is_frame_mostly_black(&c.image) => c,
            _ => return None,
        }
    } else {
        match capture_hwnd_via_print_window(target_hwnd) {
            Ok(c) if !is_frame_mostly_black(&c.image) => c,
            _ => {
                if target_hwnd.0 != root_hwnd.0 {
                    match capture_hwnd_via_print_window(root_hwnd) {
                        Ok(c) if !is_frame_mostly_black(&c.image) => c,
                        _ => return None,
                    }
                } else {
                    return None;
                }
            }
        }
    };
    let eff_hwnd_raw = if target_hwnd.0 != root_hwnd.0
        && x >= live.origin_x
        && x < live.origin_x + live.image.width() as i32
        && y >= live.origin_y
        && y < live.origin_y + live.image.height() as i32
    {
        hwnd_raw
    } else {
        root_hwnd.0 as isize
    };

    let Ok(mut st) = state_lock().lock() else {
        return None;
    };
    st.last_capture_at = Some(Instant::now());

    // 3. First time seeing this window in this session -> record baseline
    let need_init = st
        .baselines
        .get(&eff_hwnd_raw)
        .is_none_or(|(b, _, _)| b.image.dimensions() != live.image.dimensions());
    if need_init {
        if st.baselines.len() >= 4 && !st.baselines.contains_key(&eff_hwnd_raw) {
            if let Some(evict_k) = st.baselines.keys().next().copied() {
                st.baselines.remove(&evict_k);
            }
        }
        st.baselines.insert(eff_hwnd_raw, (live, (x, y), false));
        return None;
    }

    // If cursor moved far away from where the initial baseline was taken, patch that old spot
    // with the now-unhovered pixels so even the initial button becomes detectable later!
    if let Some((base, init_pt, patched)) = st.baselines.get_mut(&eff_hwnd_raw) {
        if !*patched {
            let dx = x - init_pt.0;
            let dy = y - init_pt.1;
            if dx * dx + dy * dy >= 160 * 160 {
                let (ix, iy) = *init_pt;
                patch_baseline_region(base, &live, ix, iy, 120);
                *patched = true;
            }
        }
    }

    let HoverDiffState {
        baselines,
        pending_candidates,
        confirmed,
        ..
    } = &mut *st;
    let base_ref = &baselines.get(&eff_hwnd_raw)?.0;

    // 4. Leave-Revert Confirmation for any pending candidates the cursor just moved away from!
    let mut newly_confirmed: Vec<(isize, ScreenRect)> = Vec::new();
    let mut still_pending: Vec<(isize, ScreenRect)> = Vec::new();
    for (ch, crect) in pending_candidates.drain(..) {
        if ch == eff_hwnd_raw {
            let far_outside = x < crect.x - 16
                || x > crect.x + crect.w + 16
                || y < crect.y - 16
                || y > crect.y + crect.h + 16;
            if far_outside {
                if verify_rect_reverted(
                    &base_ref.image,
                    &live.image,
                    base_ref.origin_x,
                    base_ref.origin_y,
                    crect,
                ) {
                    newly_confirmed.push((ch, crect));
                }
                // Drop un-reverted candidate (likely non-hover animation)
            } else {
                still_pending.push((ch, crect));
            }
        } else {
            still_pending.push((ch, crect));
        }
    }
    *pending_candidates = still_pending;

    // Promote leave-revert confirmed hover buttons into HITMAP_CACHE for instant future reuse!
    for (ch, crect) in newly_confirmed {
        if !confirmed.iter().any(|(h, r)| *h == ch && *r == crect) {
            confirmed.push((ch, crect));
            let container_rect = ScreenRect {
                x: live.origin_x,
                y: live.origin_y,
                w: live.image.width() as i32,
                h: live.image.height() as i32,
            };
            hitmap::merge_entries_into_hitmap(
                ch,
                container_rect,
                HitMapSource::StaticVisual,
                vec![HitMapEntry {
                    rect: crect,
                    label: String::new(),
                    description: "hover_diff_verified".to_string(),
                    role: "Button".to_string(),
                    source: HitMapSource::StaticVisual,
                    is_expandable: false,
                    trigger_pt: None,
                }],
            );
        }
    }

    // 5. Extract live hover-reactive button rect enclosing `(x, y)`
    let detected = extract_hover_reactive_rect(
        &base_ref.image,
        &live.image,
        base_ref.origin_x,
        base_ref.origin_y,
        x,
        y,
    )?;

    if !pending_candidates
        .iter()
        .any(|(h, r)| *h == eff_hwnd_raw && *r == detected)
        && !confirmed
            .iter()
            .any(|(h, r)| *h == eff_hwnd_raw && *r == detected)
    {
        pending_candidates.push((eff_hwnd_raw, detected));
        if pending_candidates.len() > 24 {
            pending_candidates.remove(0);
        }
    }

    Some(HoverDiffDetection {
        rect: detected,
        confirmed_reverted: false,
        hwnd_raw: eff_hwnd_raw,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn detects_solid_hover_color_highlight_button_precisely() {
        let base = RgbaImage::from_pixel(320, 200, Rgba([32, 34, 38, 255]));
        let mut live = base.clone();
        // Simulate subtle hover background tint on button (80..176, 50..82)
        for y in 50..82u32 {
            for x in 80..176u32 {
                live.put_pixel(x, y, Rgba([52, 56, 64, 255]));
            }
        }
        let rect = extract_hover_reactive_rect(&base, &live, 1000, 200, 1120, 266).unwrap();
        assert_eq!(rect, ScreenRect { x: 1080, y: 250, w: 96, h: 32 });

        // Once cursor moves away and button reverts to base, Leave-Revert confirms it!
        assert!(verify_rect_reverted(&base, &base, 1000, 200, rect));
        assert!(!verify_rect_reverted(&base, &live, 1000, 200, rect));
    }

    #[test]
    fn detects_hollow_1px_hover_border_and_ignores_detached_tooltip() {
        let base = RgbaImage::from_pixel(360, 220, Rgba([242, 242, 242, 255]));
        let mut live = base.clone();
        // Hollow 1px blue hover border around button (60..110, 40..84)
        for x in 60..=110u32 {
            live.put_pixel(x, 40, Rgba([120, 174, 229, 255]));
            live.put_pixel(x, 84, Rgba([120, 174, 229, 255]));
        }
        for y in 40..=84u32 {
            live.put_pixel(60, y, Rgba([120, 174, 229, 255]));
            live.put_pixel(110, y, Rgba([120, 174, 229, 255]));
        }
        // Detached floating tooltip popped up below (130..250, 120..150) — does NOT enclose cursor!
        for y in 120..150u32 {
            for x in 130..250u32 {
                live.put_pixel(x, y, Rgba([255, 255, 225, 255]));
            }
        }
        // Cursor at center of hollow button (85, 62)
        let rect = extract_hover_reactive_rect(&base, &live, 0, 0, 85, 62).unwrap();
        assert_eq!(rect, ScreenRect { x: 60, y: 40, w: 51, h: 45 });
    }

    #[test]
    fn grazing_far_left_tip_of_long_horizontal_button_captures_entire_width() {
        let base = RgbaImage::from_pixel(900, 220, Rgba([28, 30, 34, 255]));
        let mut live = base.clone();
        // Long horizontal strip button (100..660, 80..114) = 560x34px
        for y in 80..114u32 {
            for x in 100..660u32 {
                live.put_pixel(x, y, Rgba([48, 52, 60, 255]));
            }
        }
        // Cursor barely touches the far-left tip at x=106, y=96
        let rect = extract_hover_reactive_rect(&base, &live, 0, 0, 106, 96).unwrap();
        assert_eq!(rect, ScreenRect { x: 100, y: 80, w: 560, h: 34 });
    }

    #[test]
    fn rejects_fullscreen_scroll_or_video_motion() {
        let base = RgbaImage::from_pixel(320, 200, Rgba([30, 30, 30, 255]));
        let live = RgbaImage::from_pixel(320, 200, Rgba([90, 90, 90, 255]));
        assert!(extract_hover_reactive_rect(&base, &live, 0, 0, 160, 100).is_none());
    }
}
