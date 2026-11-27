use crate::config::DockEdge;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::WebviewWindow;
use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMNCRP_DISABLED, DWMWA_NCRENDERING_POLICY,
};
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, CreateRoundRectRgn, DeleteObject, GetMonitorInfoW, MonitorFromPoint,
    MonitorFromWindow, SetWindowRgn, HRGN, MONITORINFO, MONITOR_DEFAULTTONEAREST, RGN_AND, RGN_OR,
};
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, GetAncestor, GetClientRect, GetCursorPos, GetForegroundWindow,
    GetWindowLongPtrW, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible,
    SetForegroundWindow, SetWindowDisplayAffinity, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    WindowFromPoint, GA_ROOT, GWL_EXSTYLE, GWL_STYLE, HWND_TOPMOST, SC_KEYMENU, STYLESTRUCT,
    SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE,
    SW_SHOW,
    SW_SHOWNOACTIVATE, WDA_EXCLUDEFROMCAPTURE, WDA_NONE, WINDOWPOS, WINDOW_DISPLAY_AFFINITY,
    WM_ERASEBKGND, WM_NCACTIVATE, WM_NCCALCSIZE, WM_NCDESTROY, WM_NCPAINT, WM_STYLECHANGING,
    WM_SYSCOMMAND,
    WM_WINDOWPOSCHANGING, WS_BORDER, WS_CAPTION, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_DLGFRAME,
    WS_EX_APPWINDOW, WS_EX_CLIENTEDGE, WS_EX_DLGMODALFRAME, WS_EX_NOACTIVATE, WS_EX_STATICEDGE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_WINDOWEDGE, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP,
    WS_SYSMENU, WS_THICKFRAME,
};

pub const BOTTOM_MARGIN_DIP: i32 = 12;
pub const DEFAULT_LIP_HEIGHT_DIP: i32 = 56;
/// Constant backing stage height for Bar mode so opening/closing popups, menus, hover-peek,
/// unread toasts, and history sheets NEVER calls SetWindowPos / WM_SIZE on WebView2 (zero flash!).
/// Everything outside the active `SetWindowRgn` rounded-rect shapes is 100% transparent & click-through.
pub const BAR_STAGE_HEIGHT_DIP: i32 = 680;
/// Constant backing stage width (DIP) matching the maximum allowed bar/side width so switching
/// Horizontal <-> Vertical or dragging width (360..=960dip) NEVER resizes the WebView2 swapchain!
pub const BAR_STAGE_MAX_WIDTH_DIP: i32 = 960;
/// Unread wallet-card pop-out peek thickness in DIP for Left/Right/Top edges (~9px glowing card edge).
pub const DOCK_PEEK_DIP: i32 = 9;
/// Invisible edge-trigger hotzone width (physical px) for Left/Right/Top when idle (0px visible bleed).
pub const DOCK_INVISIBLE_HOTZONE_PX: i32 = 4;
/// Bottom dock handle-only hover zone height (DIP) right above the taskbar seam.
pub const DOCK_BOTTOM_HANDLE_ZONE_DIP: i32 = 20;
/// Width of the floating rounded Edge/Arc vertical side panel (both side-docked and free-floating vertical).
pub const SIDE_DOCK_WIDTH_DIP: i32 = 420;
/// Height ratio of the floating rounded vertical side panel relative to monitor work area height.
pub const SIDE_DOCK_HEIGHT_RATIO: f64 = 0.84;
/// Edge proximity threshold (physical px) for magnetic snap during handle drag.
pub const EDGE_SNAP_THRESHOLD_PX: i32 = 28;
/// Inward pull distance (physical px) from a docked edge to release dock back to free floating.
pub const UNDOCK_PULL_THRESHOLD_PX: i32 = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockStateSnapshot {
    pub edge: DockEdge,
    pub collapsed: bool,
    pub vertical: bool,
    pub unread: bool,
    pub snap_preview: DockEdge,
}

#[derive(Debug, Clone, Copy)]
struct ActiveDockRuntime {
    edge: DockEdge,
    collapsed: bool,
    vertical_mode: bool,
    unread: bool,
    last_slid_edge: DockEdge,
    last_slid_collapsed: bool,
    work_bounds: (i32, i32, i32, i32), // (left, top, right, bottom) phys px of docked monitor
    composer_height_dip: i32,
    bar_width_dip: i32,
    side_width_dip: i32,
}

static ACTIVE_DOCK: Mutex<ActiveDockRuntime> = Mutex::new(ActiveDockRuntime {
    edge: DockEdge::None,
    collapsed: false,
    vertical_mode: false,
    unread: false,
    last_slid_edge: DockEdge::None,
    last_slid_collapsed: false,
    work_bounds: (0, 0, 1920, 1032),
    composer_height_dip: 92,
    bar_width_dip: 760,
    side_width_dip: SIDE_DOCK_WIDTH_DIP,
});
type ScaledShapeTuple = (i32, i32, i32, i32, i32);
static SLIDE_ANIM_GEN: AtomicU64 = AtomicU64::new(0);
static SLIDE_ANIM_ACTIVE: AtomicBool = AtomicBool::new(false);
static RGN_ANIM_LOCK: Mutex<()> = Mutex::new(());
static LAST_SCALED_SHAPES: Mutex<Vec<ScaledShapeTuple>> = Mutex::new(Vec::new());
static LAST_EXPANDED_SHAPES: Mutex<Vec<ScaledShapeTuple>> = Mutex::new(Vec::new());

pub fn is_slide_anim_active() -> bool {
    SLIDE_ANIM_ACTIVE.load(Ordering::Acquire)
}

pub fn cancel_slide_anim() {
    SLIDE_ANIM_GEN.fetch_add(1, Ordering::AcqRel);
    SLIDE_ANIM_ACTIVE.store(false, Ordering::Release);
}

pub fn is_bar_dragging() -> bool {
    BAR_DRAG_SESSION.lock().ok().and_then(|g| *g).is_some()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShapeRectDip {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub r: i32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct MonitorGeometry {
    pub monitor_left: i32,
    pub monitor_top: i32,
    pub monitor_right: i32,
    pub monitor_bottom: i32,
    pub anchor_left: i32,
    pub anchor_top: i32,
    pub anchor_right: i32,
    pub anchor_bottom: i32,
}

pub fn hwnd_of(win: &WebviewWindow) -> Option<HWND> {
    win.hwnd().ok().map(|raw| HWND(raw.0))
}

pub fn dpi_scale_for_hwnd(hwnd: HWND) -> f64 {
    unsafe {
        let dpi = GetDpiForWindow(hwnd);
        if dpi == 0 {
            1.0
        } else {
            (dpi as f64) / 96.0
        }
    }
}

pub fn compute_bottom_anchored_rect(
    anchor: (i32, i32, i32, i32), // (left, top, right, bottom) in physical px
    width_dip: i32,
    height_dip: i32,
    margin_dip: i32,
    scale: f64,
) -> (i32, i32, i32, i32) {
    let (left, top, right, bottom) = anchor;
    let avail_w = (right - left).max(320);
    let avail_h = (bottom - top).max(120);

    let phys_w = ((width_dip as f64) * scale).round() as i32;
    let phys_h = ((height_dip as f64) * scale).round() as i32;
    let phys_margin = ((margin_dip as f64) * scale).round() as i32;

    let clamped_w = phys_w.clamp(320, avail_w);
    let clamped_h = phys_h.clamp(40, (avail_h - phys_margin).max(40));

    let x = left + (avail_w - clamped_w) / 2;
    let y = bottom - phys_margin - clamped_h;
    (x, y, clamped_w, clamped_h)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CustomBarAnchor {
    center_x: i32,
    bottom_y: i32,
}

#[derive(Debug, Clone, Copy)]
struct BarDragSession {
    start_cursor: POINT,
    start_rect: RECT,
    started_edge: DockEdge,
    snap_preview: DockEdge,
    undocked_in_drag: bool,
    moved_enough: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DragStepResult {
    pub moved: bool,
    pub snap_preview: DockEdge,
    pub undocked: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DragEndResult {
    pub snapped_edge: DockEdge,
    pub undocked: bool,
}

static CUSTOM_BAR_ANCHOR: Mutex<Option<CustomBarAnchor>> = Mutex::new(None);
static BAR_DRAG_SESSION: Mutex<Option<BarDragSession>> = Mutex::new(None);

pub fn get_dock_runtime_snapshot() -> DockStateSnapshot {
    let (edge, collapsed, vertical, unread) = ACTIVE_DOCK
        .lock()
        .map(|d| (d.edge, d.collapsed, d.vertical_mode, d.unread))
        .unwrap_or((DockEdge::None, false, false, false));
    let snap_preview = BAR_DRAG_SESSION
        .lock()
        .ok()
        .and_then(|g| g.map(|s| s.snap_preview))
        .unwrap_or(DockEdge::None);
    DockStateSnapshot {
        edge,
        collapsed,
        vertical,
        unread,
        snap_preview,
    }
}

pub fn set_dock_vertical_mode(vertical: bool) {
    if let Ok(mut guard) = ACTIVE_DOCK.lock() {
        guard.vertical_mode = vertical;
    }
}

pub fn set_dock_side_width_dip(width_dip: i32) {
    if let Ok(mut guard) = ACTIVE_DOCK.lock() {
        guard.side_width_dip = width_dip.clamp(360, 960);
    }
}

pub fn set_dock_bar_width_dip(width_dip: i32) {
    if let Ok(mut guard) = ACTIVE_DOCK.lock() {
        guard.bar_width_dip = width_dip.clamp(420, 960);
    }
}

pub fn compute_constant_stage_phys(work_bounds: (i32, i32, i32, i32), scale: f64) -> (i32, i32) {
    let (wl, wt, wr, wb) = work_bounds;
    let avail_w = (wr - wl).max(360);
    let avail_h = (wb - wt).max(420);
    let stage_w = (((BAR_STAGE_MAX_WIDTH_DIP as f64) * scale).round() as i32).clamp(360, avail_w);
    let side_h = ((avail_h as f64) * SIDE_DOCK_HEIGHT_RATIO).round() as i32;
    let min_stage_h = ((BAR_STAGE_HEIGHT_DIP as f64) * scale).round() as i32;
    let stage_h = side_h.max(min_stage_h).clamp(420, avail_h);
    (stage_w, stage_h)
}

pub fn get_dock_side_width_dip() -> i32 {
    ACTIVE_DOCK
        .lock()
        .map(|d| d.side_width_dip.clamp(360, 960))
        .unwrap_or(SIDE_DOCK_WIDTH_DIP)
}

pub fn set_dock_unread(unread: bool) {
    if let Ok(mut guard) = ACTIVE_DOCK.lock() {
        guard.unread = unread;
    }
}

pub fn set_active_dock_state(edge: DockEdge, collapsed: bool, monitor_pt: Option<POINT>) {
    let bounds = monitor_pt
        .map(query_monitor_work_bounds_at)
        .unwrap_or_else(|| {
            let mut pt = POINT::default();
            unsafe {
                let _ = GetCursorPos(&mut pt);
            }
            query_monitor_work_bounds_at(pt)
        });
    if let Ok(mut guard) = ACTIVE_DOCK.lock() {
        guard.edge = edge;
        guard.collapsed = collapsed;
        if !collapsed {
            guard.unread = false;
        }
        if edge.is_docked() {
            guard.work_bounds = bounds;
        } else {
            guard.last_slid_edge = DockEdge::None;
            guard.last_slid_collapsed = false;
        }
    }
    if edge.is_docked() {
        if let Ok(mut anchor) = CUSTOM_BAR_ANCHOR.lock() {
            *anchor = None;
        }
    }
}

pub fn update_dock_composer_height_dip(composer_h_dip: i32) {
    if composer_h_dip > 30 {
        if let Ok(mut guard) = ACTIVE_DOCK.lock() {
            guard.composer_height_dip = composer_h_dip.clamp(64, 280);
        }
    }
}

pub fn compute_docked_bar_rect(
    edge: DockEdge,
    _collapsed: bool,
    _unread: bool,
    _vertical_mode: bool,
    work_bounds: (i32, i32, i32, i32),
    _bar_width_dip: i32,
    _composer_h_dip: i32,
    scale: f64,
) -> (i32, i32, i32, i32) {
    let (wl, wt, wr, wb) = work_bounds;
    let avail_w = (wr - wl).max(360);
    let avail_h = (wb - wt).max(420);
    let (stage_w, stage_h) = compute_constant_stage_phys(work_bounds, scale);
    let phys_margin = ((BOTTOM_MARGIN_DIP as f64) * scale).round() as i32;

    // Constant-size backing stage (`960dip × max(680dip, 84% work_h)`) across ALL 4 dock edges
    // AND free-floating horizontal/vertical! Switching Horizontal <-> Vertical, switching edges,
    // or resizing width NEVER triggers WM_SIZE / WebView2 swapchain reallocation (zero half-width tear!).
    match edge {
        DockEdge::Bottom => {
            let x = wl + (avail_w - stage_w) / 2;
            let y = wb - stage_h;
            (x, y, stage_w, stage_h)
        }
        DockEdge::Top => {
            let x = wl + (avail_w - stage_w) / 2;
            (x, wt, stage_w, stage_h)
        }
        DockEdge::Left => {
            let y = wt + (avail_h - stage_h) / 2;
            (wl, y, stage_w, stage_h)
        }
        DockEdge::Right => {
            let y = wt + (avail_h - stage_h) / 2;
            (wr - stage_w, y, stage_w, stage_h)
        }
        DockEdge::None => {
            let x = wl + (avail_w - stage_w) / 2;
            let y = (wb - phys_margin - stage_h).max(wt);
            (x, y, stage_w, stage_h)
        }
    }
}

/// Computes the exact local physical `SetWindowRgn` shape for a collapsed docked Bar:
/// - Left/Right/Top idle -> `None` (1x1 transparent keepalive bezel pixel, 0px visible!)
/// - Left/Right/Top unread -> `Some((l, t, r, b, diam))` (~9dip glowing wallet-card edge)
/// - Bottom -> `Some((l, t, r, b, diam))` (centered minimal handle pill)
pub fn compute_dock_collapsed_shape_phys(
    edge: DockEdge,
    unread: bool,
    phys_w: i32,
    phys_h: i32,
    scale: f64,
) -> Option<(i32, i32, i32, i32, i32)> {
    let peek = ((DOCK_PEEK_DIP as f64) * scale).round().max(8.0) as i32;
    let r_edge = (16.0 * scale).round() as i32;
    let bar_w_dip = ACTIVE_DOCK
        .lock()
        .map(|d| d.bar_width_dip.clamp(420, 960))
        .unwrap_or(760);
    let bar_w_phys = (((bar_w_dip as f64) * scale).round() as i32).min(phys_w);
    match edge {
        DockEdge::Left => {
            if unread {
                Some((0, 0, peek.min(phys_w), phys_h, r_edge))
            } else {
                None
            }
        }
        DockEdge::Right => {
            if unread {
                Some(((phys_w - peek).max(0), 0, phys_w, phys_h, r_edge))
            } else {
                None
            }
        }
        DockEdge::Top => {
            if unread {
                let l = ((phys_w - bar_w_phys) / 2).max(0);
                Some((l, 0, (l + bar_w_phys).min(phys_w), peek.min(phys_h), r_edge))
            } else {
                None
            }
        }
        DockEdge::Bottom => {
            let hw = (76.0 * scale).round() as i32;
            let hh = (20.0 * scale).round() as i32;
            let l = ((phys_w - hw) / 2).max(0);
            let t = (phys_h - hh).max(0);
            let diam = (16.0 * scale).round() as i32;
            Some((l, t, (l + hw).min(phys_w), phys_h, diam))
        }
        DockEdge::None => None,
    }
}

pub fn detect_snap_edge_for_cursor(
    cur: POINT,
    work_bounds: (i32, i32, i32, i32),
    moved_enough: bool,
) -> DockEdge {
    if !moved_enough {
        return DockEdge::None;
    }
    let (wl, wt, wr, wb) = work_bounds;
    let d_left = (cur.x - wl).abs();
    let d_right = (wr - cur.x).abs();
    let d_top = (cur.y - wt).abs();
    let d_bottom = (wb - cur.y).abs();

    // Pick closest edge within magnetic snap zone (Bottom requires explicit pull to taskbar seam)
    let mut best = (DockEdge::None, i32::MAX);
    if cur.x <= wl + EDGE_SNAP_THRESHOLD_PX && d_left < best.1 {
        best = (DockEdge::Left, d_left);
    }
    if cur.x >= wr - EDGE_SNAP_THRESHOLD_PX && d_right < best.1 {
        best = (DockEdge::Right, d_right);
    }
    if cur.y <= wt + EDGE_SNAP_THRESHOLD_PX && d_top < best.1 {
        best = (DockEdge::Top, d_top);
    }
    if cur.y >= wb - (EDGE_SNAP_THRESHOLD_PX + 12) && d_bottom < best.1 {
        best = (DockEdge::Bottom, d_bottom);
    }
    best.0
}

pub fn should_undock_on_inward_pull(
    started_edge: DockEdge,
    start_cursor: POINT,
    cur: POINT,
) -> bool {
    match started_edge {
        DockEdge::Left => cur.x - start_cursor.x >= UNDOCK_PULL_THRESHOLD_PX,
        DockEdge::Right => start_cursor.x - cur.x >= UNDOCK_PULL_THRESHOLD_PX,
        DockEdge::Top => cur.y - start_cursor.y >= UNDOCK_PULL_THRESHOLD_PX,
        DockEdge::Bottom => start_cursor.y - cur.y >= UNDOCK_PULL_THRESHOLD_PX,
        DockEdge::None => false,
    }
}

/// Checks if the physical cursor point lies inside the dock wakeup trigger zone:
/// - Left / Right / Top: ultra-thin 4px invisible screen-edge hotzone when idle, or ~13dip wallet strip when unread
/// - Bottom: centered minimal handle pill zone right above the taskbar seam
pub fn is_cursor_in_dock_peek_zone(bar_win: &WebviewWindow) -> bool {
    let Some(hwnd) = hwnd_of(bar_win) else {
        return false;
    };
    let dock = ACTIVE_DOCK.lock().ok().map(|d| *d);
    let Some(dock) = dock else {
        return false;
    };
    if !dock.edge.is_docked() || !dock.collapsed {
        return false;
    }
    let scale = dpi_scale_for_hwnd(hwnd);
    let edge_zone = if dock.unread {
        (((DOCK_PEEK_DIP + 4) as f64) * scale).round() as i32
    } else {
        DOCK_INVISIBLE_HOTZONE_PX
    };
    let bottom_zone = ((DOCK_BOTTOM_HANDLE_ZONE_DIP as f64) * scale).round() as i32;
    let bottom_half_w = (72.0 * scale).round() as i32;
    unsafe {
        let mut pt = POINT::default();
        let mut rc = RECT::default();
        if GetCursorPos(&mut pt).is_err() || GetWindowRect(hwnd, &mut rc).is_err() {
            return false;
        }
        let (wl, wt, wr, wb) = dock.work_bounds;
        match dock.edge {
            DockEdge::Bottom => {
                let cx = wl + (wr - wl) / 2;
                pt.x >= cx - bottom_half_w
                    && pt.x <= cx + bottom_half_w
                    && pt.y >= wb - bottom_zone
                    && pt.y <= wb + 2
            }
            DockEdge::Top => {
                pt.x >= rc.left && pt.x <= rc.right && pt.y >= wt - 2 && pt.y <= wt + edge_zone
            }
            DockEdge::Left => {
                pt.y >= rc.top && pt.y <= rc.bottom && pt.x >= wl - 2 && pt.x <= wl + edge_zone
            }
            DockEdge::Right => {
                pt.y >= rc.top && pt.y <= rc.bottom && pt.x >= wr - edge_zone && pt.x <= wr + 2
            }
            DockEdge::None => false,
        }
    }
}

/// Checks if the cursor is currently inside the visible interactive shapes of the expanded Bar.
pub fn is_cursor_inside_bar_now(bar_win: &WebviewWindow) -> bool {
    let Some(hwnd) = hwnd_of(bar_win) else {
        return false;
    };
    unsafe {
        let mut pt = POINT::default();
        if GetCursorPos(&mut pt).is_err() {
            return false;
        }
        is_point_inside_bar_shapes(pt, hwnd)
    }
}

/// Returns `true` if a non-desktop, non-ATP foreground window currently covers the full monitor
/// where the docked Bar lives (used to auto-hide the 10px edge strip during fullscreen video/games).
pub fn is_fullscreen_on_dock_monitor(own_bar_hwnd: Option<HWND>) -> bool {
    let dock = ACTIVE_DOCK.lock().ok().map(|d| *d);
    let Some(dock) = dock else {
        return false;
    };
    if !dock.edge.is_docked() {
        return false;
    }
    let (wl, wt, wr, wb) = dock.work_bounds;
    let center = POINT {
        x: (wl + wr) / 2,
        y: (wt + wb) / 2,
    };
    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() || Some(fg) == own_bar_hwnd {
            return false;
        }
        let mut fg_pid = 0u32;
        GetWindowThreadProcessId(fg, Some(&mut fg_pid));
        if fg_pid == std::process::id() {
            return false;
        }
        let dock_mon = MonitorFromPoint(center, MONITOR_DEFAULTTONEAREST);
        let fg_mon = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
        if fg_mon != dock_mon {
            return false;
        }
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(dock_mon, &mut mi).as_bool() {
            return false;
        }
        let mut fg_rc = RECT::default();
        if GetWindowRect(fg, &mut fg_rc).is_err() {
            return false;
        }
        let covers = fg_rc.left <= mi.rcMonitor.left
            && fg_rc.top <= mi.rcMonitor.top
            && fg_rc.right >= mi.rcMonitor.right
            && fg_rc.bottom >= mi.rcMonitor.bottom;
        if !covers {
            return false;
        }
        let cls = get_hwnd_class_name(fg);
        cls != "Progman" && cls != "WorkerW"
    }
}

pub fn compute_custom_anchored_rect(
    center_x: i32,
    bottom_y: i32,
    phys_w: i32,
    phys_h: i32,
    bounds: (i32, i32, i32, i32),
) -> (i32, i32) {
    let (left, top, right, bottom) = bounds;
    let min_x = left;
    let max_x = (right - phys_w).max(left);
    let x = (center_x - phys_w / 2).clamp(min_x, max_x);
    let min_y = top;
    let max_y = (bottom - phys_h).max(top);
    let y = (bottom_y - phys_h).clamp(min_y, max_y);
    (x, y)
}

fn query_monitor_work_bounds_at(pt: POINT) -> (i32, i32, i32, i32) {
    unsafe {
        let hmon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(hmon, &mut mi).as_bool() {
            (mi.rcWork.left, mi.rcWork.top, mi.rcWork.right, mi.rcWork.bottom)
        } else {
            (0, 0, 1920, 1032)
        }
    }
}

pub fn begin_bar_drag(bar_win: &WebviewWindow) -> bool {
    let Some(hwnd) = hwnd_of(bar_win) else {
        return false;
    };
    let started_edge = ACTIVE_DOCK
        .lock()
        .map(|d| d.edge)
        .unwrap_or(DockEdge::None);
    unsafe {
        let mut pt = POINT::default();
        let mut rc = RECT::default();
        if GetCursorPos(&mut pt).is_err() || GetWindowRect(hwnd, &mut rc).is_err() {
            return false;
        }
        if let Ok(mut guard) = BAR_DRAG_SESSION.lock() {
            *guard = Some(BarDragSession {
                start_cursor: pt,
                start_rect: rc,
                started_edge,
                snap_preview: DockEdge::None,
                undocked_in_drag: false,
                moved_enough: false,
            });
        }
        true
    }
}

pub fn step_bar_drag(bar_win: &WebviewWindow) -> DragStepResult {
    let empty = DragStepResult {
        moved: false,
        snap_preview: DockEdge::None,
        undocked: false,
    };
    let Some(hwnd) = hwnd_of(bar_win) else {
        return empty;
    };
    let Some(mut sess) = BAR_DRAG_SESSION.lock().ok().and_then(|g| *g) else {
        return empty;
    };
    unsafe {
        let mut cur = POINT::default();
        if GetCursorPos(&mut cur).is_err() {
            return empty;
        }
        let dx = cur.x - sess.start_cursor.x;
        let dy = cur.y - sess.start_cursor.y;
        if dx.abs() >= 14 || dy.abs() >= 14 {
            sess.moved_enough = true;
        }
        if sess.started_edge.is_docked()
            && should_undock_on_inward_pull(sess.started_edge, sess.start_cursor, cur)
        {
            sess.undocked_in_drag = true;
        }
        let w = (sess.start_rect.right - sess.start_rect.left).max(320);
        let h = (sess.start_rect.bottom - sess.start_rect.top).max(40);
        let scale = dpi_scale_for_hwnd(hwnd);
        let (comp_h_dip, is_vert, bar_w_dip, side_w_dip) = ACTIVE_DOCK
            .lock()
            .map(|d| {
                (
                    d.composer_height_dip,
                    d.vertical_mode || d.edge.is_side(),
                    d.bar_width_dip,
                    d.side_width_dip,
                )
            })
            .unwrap_or((92, false, 760, SIDE_DOCK_WIDTH_DIP));
        let comp_h = ((comp_h_dip.clamp(68, 260) as f64) * scale).round() as i32;
        let panel_w = (((if is_vert { side_w_dip } else { bar_w_dip } as f64) * scale).round() as i32)
            .clamp(320, w);
        // Horizontal offset of the visible panel inside the constant 960dip stage HWND
        let panel_off_x = match sess.started_edge {
            DockEdge::Left => 0,
            DockEdge::Right => (w - panel_w).max(0),
            _ => ((w - panel_w) / 2).max(0),
        };
        let work_bounds = query_monitor_work_bounds_at(cur);
        let (ml, mt, mr, mb) = work_bounds;
        let mut snap_preview = detect_snap_edge_for_cursor(cur, work_bounds, sess.moved_enough);

        // Allow free movement of the visible panel across the entire monitor:
        let min_x = ml - panel_off_x - panel_w + 120.min(panel_w);
        let max_x = mr - panel_off_x - 120.min(panel_w);
        let (min_y, max_y) = if sess.started_edge == DockEdge::Top && !is_vert {
            (mt, (mb - comp_h.min(h)).max(mt))
        } else if is_vert {
            (mt - 32, (mb - 120.min(h)).max(mt - 32))
        } else {
            let top_allow = mt - (h - comp_h.min(h)).max(0);
            (top_allow, (mb - h).max(top_allow))
        };
        let raw_y = sess.start_rect.top + dy;
        let x = (sess.start_rect.left + dx).clamp(min_x, max_x.max(min_x));
        let y = raw_y.clamp(min_y, max_y);

        // When dragging a horizontal bar downward by its top handle, pushing the composer bottom
        // hard past the taskbar seam also snaps cleanly to Bottom dock (no need to push cursor 100px deeper).
        if sess.moved_enough && snap_preview == DockEdge::None && !is_vert {
            let composer_bottom_raw = if sess.started_edge == DockEdge::Top {
                raw_y + comp_h
            } else {
                raw_y + h
            };
            if composer_bottom_raw >= mb + 24 {
                snap_preview = DockEdge::Bottom;
            }
        }
        sess.snap_preview = snap_preview;

        let _ = SetWindowPos(
            hwnd,
            None,
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
        // Save the panel's true visual center_x & bottom_y so undocking from Left/Right/Top -> Free
        // keeps the panel pixel-exact under the user's cursor!
        let visual_center_x = x + panel_off_x + panel_w / 2;
        let visual_bottom_y = if sess.started_edge == DockEdge::Top && !is_vert {
            (y + comp_h).clamp(mt + comp_h, mb)
        } else {
            y + h
        };
        if let Ok(mut anchor) = CUSTOM_BAR_ANCHOR.lock() {
            *anchor = Some(CustomBarAnchor {
                center_x: visual_center_x,
                bottom_y: visual_bottom_y,
            });
        }
        if let Ok(mut guard) = BAR_DRAG_SESSION.lock() {
            *guard = Some(sess);
        }
        DragStepResult {
            moved: true,
            snap_preview,
            undocked: sess.undocked_in_drag && !snap_preview.is_docked(),
        }
    }
}

pub fn end_bar_drag() -> DragEndResult {
    let sess = BAR_DRAG_SESSION
        .lock()
        .ok()
        .and_then(|mut g| g.take());
    let Some(s) = sess else {
        return DragEndResult {
            snapped_edge: DockEdge::None,
            undocked: false,
        };
    };
    DragEndResult {
        snapped_edge: s.snap_preview,
        undocked: s.undocked_in_drag && !s.snap_preview.is_docked(),
    }
}

pub fn reset_bar_anchor(
    bar_win: &WebviewWindow,
    width_dip: i32,
    height_dip: i32,
    shapes: &[ShapeRectDip],
) {
    if let Ok(mut anchor) = CUSTOM_BAR_ANCHOR.lock() {
        *anchor = None;
    }
    set_active_dock_state(DockEdge::None, false, None);
    let _ = end_bar_drag();
    apply_bar_layout(bar_win, width_dip, height_dip, shapes);
}

pub fn scale_shape_to_phys(shape: &ShapeRectDip, scale: f64) -> (i32, i32, i32, i32, i32) {
    let left = ((shape.x as f64) * scale).floor() as i32;
    let top = ((shape.y as f64) * scale).floor() as i32;
    let right = (((shape.x + shape.w) as f64) * scale).ceil() as i32;
    let bottom = (((shape.y + shape.h) as f64) * scale).ceil() as i32;
    let diameter = (((shape.r * 2) as f64) * scale).round() as i32;
    (left, top, right, bottom, diameter)
}

pub fn query_cursor_monitor_geometry(own_bar_hwnd: Option<HWND>) -> MonitorGeometry {
    unsafe {
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let hmon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);

        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(hmon, &mut mi).as_bool() {
            return MonitorGeometry {
                monitor_left: 0,
                monitor_top: 0,
                monitor_right: 1920,
                monitor_bottom: 1080,
                anchor_left: 0,
                anchor_top: 0,
                anchor_right: 1920,
                anchor_bottom: 1032,
            };
        }

        // Check if foreground window is a fullscreen app on this monitor
        let mut use_full_monitor = false;
        let fg = GetForegroundWindow();
        if !fg.0.is_null() && Some(fg) != own_bar_hwnd {
            let fg_mon = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
            if fg_mon == hmon {
                let mut fg_rc = RECT::default();
                if GetWindowRect(fg, &mut fg_rc).is_ok() {
                    let covers_mon = fg_rc.left <= mi.rcMonitor.left
                        && fg_rc.top <= mi.rcMonitor.top
                        && fg_rc.right >= mi.rcMonitor.right
                        && fg_rc.bottom >= mi.rcMonitor.bottom;
                    // Exclude desktop WorkerW/Progman from fullscreen check
                    let class_name = get_hwnd_class_name(fg);
                    if covers_mon && class_name != "Progman" && class_name != "WorkerW" {
                        use_full_monitor = true;
                    }
                }
            }
        }

        let anchor = if use_full_monitor {
            mi.rcMonitor
        } else {
            mi.rcWork
        };

        MonitorGeometry {
            monitor_left: mi.rcMonitor.left,
            monitor_top: mi.rcMonitor.top,
            monitor_right: mi.rcMonitor.right,
            monitor_bottom: mi.rcMonitor.bottom,
            anchor_left: anchor.left,
            anchor_top: anchor.top,
            anchor_right: anchor.right,
            anchor_bottom: anchor.bottom,
        }
    }
}

pub fn get_hwnd_class_name(hwnd: HWND) -> String {
    unsafe {
        let mut buf = [0u16; 128];
        let len = windows::Win32::UI::WindowsAndMessaging::GetClassNameW(hwnd, &mut buf);
        if len <= 0 {
            String::new()
        } else {
            String::from_utf16_lossy(&buf[..len as usize])
        }
    }
}

const BAR_SUBCLASS_ID: usize = 0xD5B0_0001;
static OVERLAY_HWND_RAW: AtomicU64 = AtomicU64::new(0);

unsafe fn visible_overlay_hwnd() -> Option<HWND> {
    let raw = OVERLAY_HWND_RAW.load(Ordering::Acquire);
    if raw == 0 {
        return None;
    }
    let hwnd = HWND(raw as usize as *mut std::ffi::c_void);
    if IsWindowVisible(hwnd).as_bool() {
        Some(hwnd)
    } else {
        None
    }
}

/// Returns `overlay_hwnd` while Selection Overlay is visible so Bar always sits strictly
/// underneath the click-through Overlay (allowing blue highlight boxes & tooltips to draw ON TOP
/// of Bar's own elements!), otherwise returns `HWND_TOPMOST`.
unsafe fn bar_z_insert_after() -> HWND {
    visible_overlay_hwnd().unwrap_or(HWND_TOPMOST)
}

pub fn raise_overlay_topmost_if_visible() {
    unsafe {
        if let Some(ov_hwnd) = visible_overlay_hwnd() {
            let _ = SetWindowPos(
                ov_hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }
}

const BAR_STYLE_STRIP_MASK: u32 = WS_CAPTION.0
    | WS_THICKFRAME.0
    | WS_SYSMENU.0
    | WS_MINIMIZEBOX.0
    | WS_MAXIMIZEBOX.0
    | WS_BORDER.0
    | WS_DLGFRAME.0;

const BAR_EXSTYLE_STRIP_MASK: u32 = WS_EX_APPWINDOW.0
    | WS_EX_WINDOWEDGE.0
    | WS_EX_DLGMODALFRAME.0
    | WS_EX_CLIENTEDGE.0
    | WS_EX_STATICEDGE.0;

fn clean_bar_style(style: u32) -> u32 {
    (style & !BAR_STYLE_STRIP_MASK) | WS_POPUP.0 | WS_CLIPCHILDREN.0 | WS_CLIPSIBLINGS.0
}

fn clean_bar_ex_style(ex_style: u32, no_activate: bool) -> u32 {
    let base = (ex_style & !BAR_EXSTYLE_STRIP_MASK) | WS_EX_TOOLWINDOW.0;
    if no_activate {
        base | WS_EX_NOACTIVATE.0
    } else {
        base & !WS_EX_NOACTIVATE.0
    }
}

/// Subclass installed on the Bar HWND to guarantee zero Win32 non-client chrome forever:
/// - `WM_NCCALCSIZE` -> 0 (client rect ALWAYS equals full window rect, so WebView2 never shrinks)
/// - `WM_NCPAINT` -> 0 (GDI/uxtheme can NEVER draw a classic/Aero Basic titlebar or border)
/// - `WM_NCACTIVATE` -> 1 (allows activation/deactivation without ever calling DefWindowProc NC paint)
/// - `WM_STYLECHANGING` -> strips WS_CAPTION/WS_SYSMENU/WS_THICKFRAME if tao ever attempts to re-add them
unsafe extern "system" fn bar_borderless_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    uid_subclass: usize,
    _dw_ref_data: usize,
) -> LRESULT {
    match msg {
        WM_NCCALCSIZE => LRESULT(0),
        WM_NCPAINT => LRESULT(0),
        WM_ERASEBKGND => LRESULT(1),
        WM_NCACTIVATE => LRESULT(1),
        // Prevent pressing Alt / Ctrl+Alt while Bar is focused from entering Win32's modal menu loop
        WM_SYSCOMMAND if (wparam.0 & 0xFFF0) == (SC_KEYMENU as usize) => LRESULT(0),
        WM_WINDOWPOSCHANGING => {
            if lparam.0 != 0 {
                if let Some(ov_hwnd) = visible_overlay_hwnd() {
                    let wp = &mut *(lparam.0 as *mut WINDOWPOS);
                    if (wp.flags.0 & SWP_NOZORDER.0) == 0 && wp.hwndInsertAfter != ov_hwnd {
                        wp.hwndInsertAfter = ov_hwnd;
                    }
                }
            }
            DefSubclassProc(hwnd, msg, wparam, lparam)
        }
        WM_STYLECHANGING => {
            if lparam.0 != 0 {
                let kind = wparam.0 as i32;
                let st = &mut *(lparam.0 as *mut STYLESTRUCT);
                if kind == GWL_STYLE.0 {
                    st.styleNew = clean_bar_style(st.styleNew);
                } else if kind == GWL_EXSTYLE.0 {
                    let keep_noact = (st.styleNew & WS_EX_NOACTIVATE.0) != 0;
                    st.styleNew = clean_bar_ex_style(st.styleNew, keep_noact);
                }
            }
            DefSubclassProc(hwnd, msg, wparam, lparam)
        }
        WM_NCDESTROY => {
            let _ = RemoveWindowSubclass(hwnd, Some(bar_borderless_subclass_proc), uid_subclass);
            DefSubclassProc(hwnd, msg, wparam, lparam)
        }
        _ => DefSubclassProc(hwnd, msg, wparam, lparam),
    }
}

/// Strips Win32 caption/border/sysmenu styles (without disabling DWM so `transparent(true)`
/// and DirectComposition WebView2 rendering remain 100% intact).
unsafe fn enforce_borderless_popup_style(hwnd: HWND, no_activate: bool) -> bool {
    let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
    let next_style = clean_bar_style(style);
    let style_changed = next_style != style;
    if style_changed {
        let _ = SetWindowLongPtrW(hwnd, GWL_STYLE, next_style as usize as isize);
    }

    let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
    let next_ex = clean_bar_ex_style(ex, no_activate);
    let ex_changed = next_ex != ex;
    if ex_changed {
        let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, next_ex as usize as isize);
    }
    style_changed || ex_changed
}

pub fn init_bar_window(bar_win: &WebviewWindow, exclude_from_capture: bool, width_dip: i32) {
    let Some(hwnd) = hwnd_of(bar_win) else {
        return;
    };
    let scale = dpi_scale_for_hwnd(hwnd);
    let mon = query_cursor_monitor_geometry(Some(hwnd));
    let (stage_w, stage_h) = compute_constant_stage_phys(
        (mon.anchor_left, mon.anchor_top, mon.anchor_right, mon.anchor_bottom),
        scale,
    );
    // Sync Tauri / WebView2 ICoreWebView2Controller bounds ONCE to the constant 960dip stage size
    let _ = bar_win.set_size(tauri::PhysicalSize::new(stage_w as u32, stage_h as u32));
    unsafe {
        let _ = SetWindowSubclass(hwnd, Some(bar_borderless_subclass_proc), BAR_SUBCLASS_ID, 0);
        let _ = enforce_borderless_popup_style(hwnd, true);
        // Ensure ZERO legacy GDI SetWindowRgn is ever installed on the DirectComposition WebView2 window!
        let _ = SetWindowRgn(hwnd, None, false);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
        apply_capture_affinity(hwnd, exclude_from_capture);
    }

    let default_shape = ShapeRectDip {
        x: ((BAR_STAGE_MAX_WIDTH_DIP - width_dip).max(0)) / 2,
        y: BAR_STAGE_HEIGHT_DIP - 90,
        w: width_dip,
        h: 90,
        r: 24,
    };
    apply_bar_layout(bar_win, width_dip, BAR_STAGE_HEIGHT_DIP, &[default_shape]);
    log_window_diagnostics(bar_win, "init");
}

pub fn init_overlay_window(overlay_win: &WebviewWindow, exclude_from_capture: bool) {
    // Use Tauri's DirectComposition-safe click-through API (never touch WebView2 child HWNDs with WS_EX_LAYERED!)
    let _ = overlay_win.set_ignore_cursor_events(true);

    let Some(hwnd) = hwnd_of(overlay_win) else {
        return;
    };
    OVERLAY_HWND_RAW.store(hwnd.0 as usize as u64, Ordering::Release);
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let next_ex = ex | (WS_EX_TOOLWINDOW.0 as isize) | (WS_EX_NOACTIVATE.0 as isize);
        let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, next_ex);
        let ncrp = DWMNCRP_DISABLED.0 as u32;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_NCRENDERING_POLICY,
            &ncrp as *const _ as *const _,
            std::mem::size_of::<u32>() as u32,
        );
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
        // Overlay is excluded from screen captures by default so pen strokes never pollute raw screenshots
        // (honors `--dev-allow-capture` when explicitly requested for visual demo/E2E verification).
        if exclude_from_capture {
            let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
        }
    }
}

pub fn apply_capture_affinity(hwnd: HWND, exclude: bool) {
    unsafe {
        let affinity: WINDOW_DISPLAY_AFFINITY = if exclude {
            WDA_EXCLUDEFROMCAPTURE
        } else {
            WDA_NONE
        };
        let _ = SetWindowDisplayAffinity(hwnd, affinity);
    }
}

type ScaledRgnKey = (i32, i32, Option<(i32, i32, i32, i32)>, Vec<(i32, i32, i32, i32, i32)>);
static LAST_BAR_RGN_KEY: Mutex<Option<ScaledRgnKey>> = Mutex::new(None);
static LAST_BAR_SHAPES_PHYS: Mutex<Vec<(i32, i32, i32, i32)>> = Mutex::new(Vec::new());

/// Temporarily removes `SetWindowRgn` (unclipped full DirectComposition surface!) during a
/// 240ms GPU animation so ZERO pixels are ever chopped half-way ("先出左边/先出右边") and DWM
/// performs ZERO GDI region validation per frame. Restored cleanly once the animation settles.
unsafe fn clear_window_rgn_for_anim(hwnd: HWND, keep_shapes_phys: Option<Vec<(i32, i32, i32, i32)>>) {
    if let Ok(mut slot) = LAST_BAR_RGN_KEY.lock() {
        *slot = None;
    }
    if let Some(boxes) = keep_shapes_phys {
        if let Ok(mut slot) = LAST_BAR_SHAPES_PHYS.lock() {
            *slot = boxes;
        }
    }
    let _ = SetWindowRgn(hwnd, None, false);
}

unsafe fn apply_shapes_region_clipped(
    hwnd: HWND,
    phys_w: i32,
    phys_h: i32,
    scale: f64,
    scaled_shapes: &[(i32, i32, i32, i32, i32)],
    work_clip_local: Option<(i32, i32, i32, i32)>,
    b_redraw: bool,
) {
    let rgn_key: ScaledRgnKey = (phys_w, phys_h, work_clip_local, scaled_shapes.to_vec());
    let rgn_changed = LAST_BAR_RGN_KEY
        .lock()
        .map(|mut slot| {
            if slot.as_ref() == Some(&rgn_key) {
                false
            } else {
                *slot = Some(rgn_key);
                true
            }
        })
        .unwrap_or(true);
    if !rgn_changed {
        return;
    }

    let mut phys_boxes: Vec<(i32, i32, i32, i32)> = if scaled_shapes.is_empty() {
        vec![(0, 0, phys_w, phys_h)]
    } else {
        scaled_shapes
            .iter()
            .map(|&(l, t, r, b, _)| (l, t, r, b))
            .collect()
    };
    if let Some((cl, ct, cr, cb)) = work_clip_local {
        phys_boxes = phys_boxes
            .into_iter()
            .filter_map(|(l, t, r, b)| {
                let nl = l.max(cl);
                let nt = t.max(ct);
                let nr = r.min(cr);
                let nb = b.min(cb);
                if nr > nl && nb > nt {
                    Some((nl, nt, nr, nb))
                } else {
                    None
                }
            })
            .collect();
    }
    if let Ok(mut slot) = LAST_BAR_SHAPES_PHYS.lock() {
        *slot = phys_boxes;
    }

    let master_rgn: HRGN = if scaled_shapes.is_empty() {
        let diam = (48.0 * scale).round() as i32;
        CreateRoundRectRgn(0, 0, phys_w + 1, phys_h + 1, diam, diam)
    } else {
        let mut acc: Option<HRGN> = None;
        for &(l, t, r, b, diam) in scaled_shapes {
            let part = CreateRoundRectRgn(l, t, r + 1, b + 1, diam, diam);
            if let Some(existing) = acc {
                let _ = CombineRgn(Some(existing), Some(existing), Some(part), RGN_OR);
                let _ = DeleteObject(part.into());
            } else {
                acc = Some(part);
            }
        }
        acc.unwrap_or_else(|| {
            let diam = (48.0 * scale).round() as i32;
            CreateRoundRectRgn(0, 0, phys_w + 1, phys_h + 1, diam, diam)
        })
    };

    if let Some((cl, ct, cr, cb)) = work_clip_local {
        let clip_rgn = CreateRectRgn(cl, ct, cr, cb);
        let _ = CombineRgn(Some(master_rgn), Some(master_rgn), Some(clip_rgn), RGN_AND);
        let _ = DeleteObject(clip_rgn.into());
    }

    if SetWindowRgn(hwnd, Some(master_rgn), b_redraw) == 0 {
        // On Win32 failure, caller retains HRGN ownership and MUST free it to avoid GDI leak.
        let _ = DeleteObject(master_rgn.into());
    }
}

unsafe fn apply_empty_window_rgn(hwnd: HWND, edge: DockEdge, phys_w: i32, phys_h: i32) {
    let keepalive = match edge {
        DockEdge::Right => ((phys_w - 1).max(0), 0, phys_w.max(1), 1),
        _ => (0, 0, 1, 1),
    };
    let empty_key: ScaledRgnKey = (phys_w, phys_h, Some(keepalive), Vec::new());
    let changed = LAST_BAR_RGN_KEY
        .lock()
        .map(|mut slot| {
            if slot.as_ref() == Some(&empty_key) {
                false
            } else {
                *slot = Some(empty_key);
                true
            }
        })
        .unwrap_or(true);
    if !changed {
        return;
    }
    if let Ok(mut slot) = LAST_BAR_SHAPES_PHYS.lock() {
        slot.clear();
    }
    // 1x1 transparent bezel pixel keeps DWM non-occluded (0 visible pixels since DOM is translated off-screen).
    let rgn = CreateRectRgn(keepalive.0, keepalive.1, keepalive.2, keepalive.3);
    if SetWindowRgn(hwnd, Some(rgn), false) == 0 {
        let _ = DeleteObject(rgn.into());
    }
}

unsafe fn apply_collapsed_dock_rgn(
    hwnd: HWND,
    edge: DockEdge,
    unread: bool,
    phys_w: i32,
    phys_h: i32,
    scale: f64,
    clip: Option<(i32, i32, i32, i32)>,
) {
    if let Some(shape) = compute_dock_collapsed_shape_phys(edge, unread, phys_w, phys_h, scale) {
        apply_shapes_region_clipped(hwnd, phys_w, phys_h, scale, &[shape], clip, false);
    } else {
        apply_empty_window_rgn(hwnd, edge, phys_w, phys_h);
    }
}

/// Computes the tight physical shape band for a docked panel inside the constant 960dip stage.
fn compute_dock_band_shape_phys(
    edge: DockEdge,
    phys_w: i32,
    phys_h: i32,
    bar_w_phys: i32,
    side_w_phys: i32,
    comp_h_phys: i32,
    scale: f64,
) -> (i32, i32, i32, i32, i32) {
    let diam = (36.0 * scale).round() as i32;
    let bw = bar_w_phys.min(phys_w);
    let sw = side_w_phys.min(phys_w);
    match edge {
        DockEdge::Bottom => {
            let l = ((phys_w - bw) / 2).max(0);
            (l, (phys_h - comp_h_phys - 24).max(0), (l + bw).min(phys_w), phys_h, diam)
        }
        DockEdge::Top => {
            let l = ((phys_w - bw) / 2).max(0);
            (l, 0, (l + bw).min(phys_w), (comp_h_phys + 24).min(phys_h), diam)
        }
        DockEdge::Left => (0, 0, sw, phys_h, diam),
        DockEdge::Right => ((phys_w - sw).max(0), 0, phys_w, phys_h, diam),
        DockEdge::None => {
            let l = ((phys_w - bw) / 2).max(0);
            (l, (phys_h - comp_h_phys - 24).max(0), (l + bw).min(phys_w), phys_h, diam)
        }
    }
}

/// Immediately opens `SetWindowRgn` to the union of the centered horizontal composer band AND
/// centered vertical panel band inside the constant 960dip stage so an in-place Horizontal <-> Vertical
/// GPU morph has 100% unclipped canvas on Frame 0 (with ZERO `SetWindowPos` / `WM_SIZE`!).
pub fn open_orientation_morph_rgn(bar_win: &WebviewWindow, _bar_width_dip: i32) {
    let Some(hwnd) = hwnd_of(bar_win) else {
        return;
    };
    // Clear SetWindowRgn completely during the 250ms GPU orientation morph so ZERO pixels are
    // ever clipped half-way ("先出左边/先出右边"); React restores tight RGN at t = 270ms!
    unsafe {
        clear_window_rgn_for_anim(hwnd, None);
    }
}

/// GPU-composited dock slide coordinator: keeps the docked HWND at a constant 960dip stage size
/// flush against its screen edge (ZERO `WM_SIZE` / swapchain resize stalls!).
pub fn slide_docked_bar(
    bar_win: &WebviewWindow,
    edge: DockEdge,
    collapsed: bool,
    bar_width_dip: i32,
    activate_and_focus: bool,
) {
    if !edge.is_docked() {
        return;
    }
    let Some(hwnd) = hwnd_of(bar_win) else {
        return;
    };
    let scale = dpi_scale_for_hwnd(hwnd);
    let (work_bounds, composer_h_dip, unread, vertical_mode, side_w_dip, prev_slid_edge, prev_slid_collapsed) = {
        let Ok(mut guard) = ACTIVE_DOCK.lock() else {
            return;
        };
        let prev_e = guard.last_slid_edge;
        let prev_c = guard.last_slid_collapsed;
        guard.edge = edge;
        guard.collapsed = collapsed;
        guard.bar_width_dip = bar_width_dip.clamp(420, 960);
        guard.last_slid_edge = edge;
        guard.last_slid_collapsed = collapsed;
        if !collapsed {
            guard.unread = false;
        }
        (
            guard.work_bounds,
            guard.composer_height_dip,
            guard.unread,
            guard.vertical_mode,
            guard.side_width_dip,
            prev_e,
            prev_c,
        )
    };
    let (target_x, target_y, phys_w, phys_h) = compute_docked_bar_rect(
        edge,
        collapsed,
        unread,
        vertical_mode,
        work_bounds,
        bar_width_dip,
        composer_h_dip,
        scale,
    );
    let (wl, wt, wr, wb) = work_bounds;
    let clip = Some((wl - target_x, wt - target_y, wr - target_x, wb - target_y));
    let comp_h_phys = ((composer_h_dip.clamp(68, 260) as f64) * scale).round() as i32;
    let bar_w_phys = ((bar_width_dip.clamp(420, 960) as f64) * scale).round() as i32;
    let side_w_phys = ((side_w_dip.clamp(360, 960) as f64) * scale).round() as i32;

    let mut start_rc = RECT::default();
    let was_visible = unsafe {
        IsWindowVisible(hwnd).as_bool() && GetWindowRect(hwnd, &mut start_rc).is_ok()
    };
    let same_size = was_visible
        && (start_rc.right - start_rc.left) == phys_w
        && (start_rc.bottom - start_rc.top) == phys_h;
    let same_rect = same_size && start_rc.left == target_x && start_rc.top == target_y;
    let edge_changed = prev_slid_edge != edge;

    let anim_gen = SLIDE_ANIM_GEN.fetch_add(1, Ordering::AcqRel) + 1;

    if edge_changed {
        if let Ok(mut slot) = LAST_SCALED_SHAPES.lock() {
            slot.clear();
        }
        if let Ok(mut slot) = LAST_EXPANDED_SHAPES.lock() {
            slot.clear();
        }
    }

    // Move-only (`SWP_NOSIZE`) whenever constant stage size already matches — ZERO WebView2 swapchain resize!
    unsafe {
        let _ = enforce_borderless_popup_style(hwnd, !activate_and_focus);
        if !same_rect {
            let insert_after = bar_z_insert_after();
            let mut flags = SWP_NOACTIVATE;
            if same_size {
                flags |= SWP_NOSIZE;
            }
            let _ = SetWindowPos(
                hwnd,
                Some(insert_after),
                target_x,
                target_y,
                phys_w,
                phys_h,
                flags,
            );
        }
        if !was_visible {
            let _ = ShowWindow(
                hwnd,
                if activate_and_focus {
                    SW_SHOW
                } else {
                    SW_SHOWNOACTIVATE
                },
            );
        }
    }

    if !collapsed {
        let band_shape = compute_dock_band_shape_phys(
            edge,
            phys_w,
            phys_h,
            bar_w_phys,
            side_w_phys,
            comp_h_phys,
            scale,
        );
        let final_shapes: Vec<(i32, i32, i32, i32, i32)> = if edge_changed || !same_rect {
            vec![band_shape]
        } else {
            let cached = LAST_EXPANDED_SHAPES
                .lock()
                .map(|g| g.clone())
                .unwrap_or_default();
            if !cached.is_empty() {
                cached
            } else {
                vec![band_shape]
            }
        };
        let final_boxes: Vec<(i32, i32, i32, i32)> = final_shapes
            .iter()
            .map(|&(l, t, r, b, _)| (l, t, r, b))
            .collect();
        // Clear GDI SetWindowRgn completely during the 260ms GPU slide/morph (unclipped DirectComposition!),
        // while keeping `LAST_BAR_SHAPES_PHYS` tight to `final_boxes` so OutsideClick works on click #1!
        unsafe {
            clear_window_rgn_for_anim(hwnd, Some(final_boxes));
        }

        // Restore tight resting SetWindowRgn once the 250ms GPU animation finishes!
        SLIDE_ANIM_ACTIVE.store(true, Ordering::Release);
        let hwnd_raw = hwnd.0 as isize;
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(260));
            let _rgn_guard = RGN_ANIM_LOCK.lock();
            if SLIDE_ANIM_GEN.load(Ordering::Acquire) != anim_gen {
                return;
            }
            let hwnd = HWND(hwnd_raw as *mut std::ffi::c_void);
            if unsafe { !windows::Win32::UI::WindowsAndMessaging::IsWindow(Some(hwnd)).as_bool() } {
                SLIDE_ANIM_ACTIVE.store(false, Ordering::Release);
                return;
            }
            let raw_live = LAST_EXPANDED_SHAPES
                .lock()
                .ok()
                .filter(|g| !g.is_empty())
                .map(|g| g.clone())
                .unwrap_or_default();
            let has_visible_shape = raw_live.iter().any(|&(l, t, r, b, _)| {
                let il = l.max(0);
                let it = t.max(0);
                let ir = r.min(phys_w);
                let ib = b.min(phys_h);
                (ir - il) >= 32 && (ib - it) >= 24
            });
            let live_shapes = if has_visible_shape {
                raw_live
            } else {
                final_shapes
            };
            unsafe {
                apply_shapes_region_clipped(hwnd, phys_w, phys_h, scale, &live_shapes, clip, false);
            }
            SLIDE_ANIM_ACTIVE.store(false, Ordering::Release);
        });

        if activate_and_focus {
            focus_hwnd_without_sendinput(hwnd);
            let _ = bar_win.set_focus();
            raise_overlay_topmost_if_visible();
            let _ = bar_win.eval(
                "if (window.__DSB_BRIDGE__) { window.__DSB_BRIDGE__.focusComposer(); setTimeout(() => window.__DSB_BRIDGE__ && window.__DSB_BRIDGE__.focusComposer(), 60); }",
            );
        }
        return;
    }

    // COLLAPSING (or toggling unread while already collapsed):
    if edge_changed || !same_rect || (prev_slid_collapsed && unread) {
        SLIDE_ANIM_ACTIVE.store(false, Ordering::Release);
        unsafe {
            apply_collapsed_dock_rgn(hwnd, edge, unread, phys_w, phys_h, scale, clip);
        }
        return;
    }

    // Normal smooth collapse: clear GDI SetWindowRgn during the 260ms GPU glide (zero mid-air chop!),
    // THEN seal to 1x1 keepalive / 9dip unread / bottom handle pill!
    SLIDE_ANIM_ACTIVE.store(true, Ordering::Release);
    unsafe {
        clear_window_rgn_for_anim(hwnd, Some(Vec::new()));
    }
    let hwnd_raw = hwnd.0 as isize;
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(265));
        let _rgn_guard = RGN_ANIM_LOCK.lock();
        if SLIDE_ANIM_GEN.load(Ordering::Acquire) != anim_gen {
            return;
        }
        let hwnd = HWND(hwnd_raw as *mut std::ffi::c_void);
        if unsafe { !windows::Win32::UI::WindowsAndMessaging::IsWindow(Some(hwnd)).as_bool() } {
            SLIDE_ANIM_ACTIVE.store(false, Ordering::Release);
            return;
        }
        unsafe {
            apply_collapsed_dock_rgn(hwnd, edge, unread, phys_w, phys_h, scale, clip);
        }
        SLIDE_ANIM_ACTIVE.store(false, Ordering::Release);
    });
}

pub fn point_hits_any_phys_rect(rx: i32, ry: i32, rects: &[(i32, i32, i32, i32)]) -> bool {
    rects
        .iter()
        .any(|&(l, t, r, b)| rx >= l && rx <= r && ry >= t && ry <= b)
}

/// Fast, GDI-free hit-test for `WH_MOUSE_LL`: checks both the HWND bounding rect AND the exact
/// active `SetWindowRgn` physical shape rects so clicks in the transparent upper stage above the
/// composer accurately register as outside clicks (and pass through to windows underneath).
pub fn is_point_inside_bar_shapes(pt: POINT, hwnd: HWND) -> bool {
    if hwnd.0.is_null() {
        return false;
    }
    unsafe {
        let mut rc = RECT::default();
        if GetWindowRect(hwnd, &mut rc).is_err() {
            return false;
        }
        if pt.x < rc.left || pt.x > rc.right || pt.y < rc.top || pt.y > rc.bottom {
            return false;
        }
        let rx = pt.x - rc.left;
        let ry = pt.y - rc.top;
        if let Ok(guard) = LAST_BAR_SHAPES_PHYS.try_lock() {
            if !guard.is_empty() {
                return point_hits_any_phys_rect(rx, ry, &guard);
            }
            // When docked & collapsed with an empty shape list (invisible 1x1 keepalive on L/R/T),
            // NEVER treat the invisible 960x900 backing stage as "on the Bar"!
            if let Ok(dock) = ACTIVE_DOCK.try_lock() {
                if dock.edge.is_docked() && dock.collapsed {
                    return false;
                }
            }
        }
        true
    }
}

/// Positions Bar anchored to the bottom of the cursor's monitor work area and clips
/// its Win32 hit-test/paint region to the exact union of `shapes` (24px rounded slab + popups).
pub fn apply_bar_layout(
    bar_win: &WebviewWindow,
    width_dip: i32,
    _height_dip: i32,
    shapes: &[ShapeRectDip],
) {
    let Some(hwnd) = hwnd_of(bar_win) else {
        return;
    };
    // Never fight an active handle drag!
    if BAR_DRAG_SESSION.lock().ok().and_then(|g| *g).is_some() {
        return;
    }
    let scale = dpi_scale_for_hwnd(hwnd);
    let dock_rt = ACTIVE_DOCK.lock().ok().map(|d| *d);
    let (x, y, phys_w, phys_h, work_clip_local) = if let Some(d) = dock_rt.filter(|d| d.edge.is_docked()) {
        let (dx, dy, dw, dh) = compute_docked_bar_rect(
            d.edge,
            d.collapsed,
            d.unread,
            d.vertical_mode,
            d.work_bounds,
            width_dip,
            d.composer_height_dip,
            scale,
        );
        let (wl, wt, wr, wb) = d.work_bounds;
        (dx, dy, dw, dh, Some((wl - dx, wt - dy, wr - dx, wb - dy)))
    } else {
        // Free-floating (BOTH Horizontal and Vertical) ALWAYS shares the exact same constant
        // `960dip × stage_h` backing stage so toggling Horizontal <-> Vertical or resizing width
        // NEVER resizes the WebView2 HWND or triggers swapchain reallocation!
        let is_vert = dock_rt.map(|d| d.vertical_mode).unwrap_or(false);
        let custom_anchor = CUSTOM_BAR_ANCHOR.lock().ok().and_then(|g| *g);
        let bounds = if let Some(anchor) = custom_anchor {
            query_monitor_work_bounds_at(POINT {
                x: anchor.center_x,
                y: anchor.bottom_y - 24,
            })
        } else {
            let mon = query_cursor_monitor_geometry(Some(hwnd));
            (mon.anchor_left, mon.anchor_top, mon.anchor_right, mon.anchor_bottom)
        };
        let (stage_w, stage_h) = compute_constant_stage_phys(bounds, scale);
        let (wl, wt, wr, wb) = bounds;
        let avail_w = (wr - wl).max(360);
        let avail_h = (wb - wt).max(420);
        let (cx, cy) = if let Some(anchor) = custom_anchor {
            let comp_h_dip = dock_rt.map(|d| d.composer_height_dip).unwrap_or(92);
            let comp_h_phys = ((comp_h_dip.clamp(68, 260) as f64) * scale).round() as i32;
            let bar_w_dip = dock_rt.map(|d| d.bar_width_dip).unwrap_or(width_dip);
            let side_w_dip = dock_rt.map(|d| d.side_width_dip).unwrap_or(SIDE_DOCK_WIDTH_DIP);
            let panel_w = (((if is_vert { side_w_dip } else { bar_w_dip } as f64) * scale).round() as i32)
                .clamp(320, stage_w);
            let pad_x = ((stage_w - panel_w) / 2).max(0);
            let top_pad = if is_vert {
                0
            } else {
                (stage_h - comp_h_phys.min(stage_h)).max(0)
            };
            let stage_bounds = (wl - pad_x, wt - top_pad, wr + pad_x, wb);
            compute_custom_anchored_rect(anchor.center_x, anchor.bottom_y, stage_w, stage_h, stage_bounds)
        } else if is_vert {
            (wl + (avail_w - stage_w) / 2, wt + (avail_h - stage_h) / 2)
        } else {
            let phys_margin = ((BOTTOM_MARGIN_DIP as f64) * scale).round() as i32;
            (wl + (avail_w - stage_w) / 2, (wb - phys_margin - stage_h).max(wt))
        };
        (cx, cy, stage_w, stage_h, None)
    };

    // Never overwrite SetWindowRgn while a GPU dock slide is in flight, or while any docked edge is collapsed
    // (Rust owns the collapsed RGN / centered bottom handle pill cleanly!).
    if SLIDE_ANIM_ACTIVE.load(Ordering::Acquire) {
        return;
    }
    if dock_rt.is_some_and(|d| d.edge.is_docked() && d.collapsed) {
        return;
    }

    let scaled_shapes: Vec<(i32, i32, i32, i32, i32)> = shapes
        .iter()
        .filter(|s| s.w > 2 && s.h > 2)
        .map(|s| scale_shape_to_phys(s, scale))
        .collect();
    if let Ok(mut slot) = LAST_SCALED_SHAPES.lock() {
        *slot = scaled_shapes.clone();
    }
    // Never poison LAST_EXPANDED_SHAPES with mid-slide or off-screen collapsed DOM rects!
    let has_onscreen_shape = scaled_shapes.iter().any(|&(l, t, r, b, _)| {
        let il = l.max(0);
        let it = t.max(0);
        let ir = r.min(phys_w);
        let ib = b.min(phys_h);
        (ir - il) >= 32 && (ib - it) >= 24
    });
    if has_onscreen_shape
        && !SLIDE_ANIM_ACTIVE.load(Ordering::Acquire)
        && !dock_rt.map(|d| d.collapsed).unwrap_or(false)
    {
        if let Ok(mut slot) = LAST_EXPANDED_SHAPES.lock() {
            *slot = scaled_shapes.clone();
        }
    }

    unsafe {
        let overlay_vis = visible_overlay_hwnd();
        let already_topmost = (GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0) != 0;
        let mut cur_rc = RECT::default();
        let got_rc = GetWindowRect(hwnd, &mut cur_rc).is_ok();
        let same_size = got_rc
            && (cur_rc.right - cur_rc.left) == phys_w
            && (cur_rc.bottom - cur_rc.top) == phys_h;
        let same_rect = same_size && cur_rc.left == x && cur_rc.top == y;
        let pos_changed = !same_rect;

        if overlay_vis.is_none() && already_topmost {
            if pos_changed {
                let mut flags = SWP_NOZORDER | SWP_NOACTIVATE;
                if same_size {
                    flags |= SWP_NOSIZE;
                }
                let _ = SetWindowPos(hwnd, None, x, y, phys_w, phys_h, flags);
            }
        } else {
            let insert_after = overlay_vis.unwrap_or(HWND_TOPMOST);
            let mut flags = SWP_NOACTIVATE;
            if same_size {
                flags |= SWP_NOSIZE;
            }
            if !pos_changed {
                flags |= SWP_NOMOVE;
            }
            let _ = SetWindowPos(hwnd, Some(insert_after), x, y, phys_w, phys_h, flags);
        }

        apply_shapes_region_clipped(
            hwnd,
            phys_w,
            phys_h,
            scale,
            &scaled_shapes,
            work_clip_local,
            !pos_changed,
        );
    }
}

pub fn show_bar_noactivate(bar_win: &WebviewWindow) {
    let Some(hwnd) = hwnd_of(bar_win) else {
        return;
    };
    unsafe {
        let changed = enforce_borderless_popup_style(hwnd, true);
        let mut flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
        if changed {
            flags |= SWP_FRAMECHANGED;
        }
        let insert_after = bar_z_insert_after();
        let _ = SetWindowPos(hwnd, Some(insert_after), 0, 0, 0, 0, flags);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
}

/// Clean foreground activation with ZERO synthetic `SendInput` keystrokes so global hotkey
/// hooks (Chatterfly `Ctrl+Alt`, PowerToys, AutoHotkey, IMEs) are never polluted.
pub fn focus_hwnd_without_sendinput(hwnd: HWND) {
    unsafe {
        if GetForegroundWindow() == hwnd {
            return;
        }
        let _ = SetForegroundWindow(hwnd);
        if GetForegroundWindow() != hwnd {
            let fg = GetForegroundWindow();
            let fg_tid = if fg.0.is_null() {
                0
            } else {
                GetWindowThreadProcessId(fg, None)
            };
            let cur_tid = GetCurrentThreadId();
            if fg_tid != 0 && fg_tid != cur_tid {
                let attached = AttachThreadInput(fg_tid, cur_tid, true).as_bool();
                let _ = BringWindowToTop(hwnd);
                let _ = SetForegroundWindow(hwnd);
                if attached {
                    let _ = AttachThreadInput(fg_tid, cur_tid, false);
                }
            } else {
                let _ = BringWindowToTop(hwnd);
                let _ = SetForegroundWindow(hwnd);
            }
        }
    }
}

static LOGGED_FIRST_ACTIVATE: AtomicBool = AtomicBool::new(false);

pub fn activate_and_focus_bar(bar_win: &WebviewWindow) {
    let Some(hwnd) = hwnd_of(bar_win) else {
        return;
    };
    unsafe {
        let already_fg = GetForegroundWindow() == hwnd;
        let changed = enforce_borderless_popup_style(hwnd, false);
        let mut flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
        if changed {
            flags |= SWP_FRAMECHANGED;
        }
        let overlay_vis = visible_overlay_hwnd();
        let already_topmost = (GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0) != 0;
        if overlay_vis.is_none() && already_topmost && already_fg {
            let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, flags | SWP_NOZORDER);
        } else {
            let insert_after = overlay_vis.unwrap_or(HWND_TOPMOST);
            let _ = SetWindowPos(hwnd, Some(insert_after), 0, 0, 0, 0, flags);
        }
        if !IsWindowVisible(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_SHOW);
        }
    }
    focus_hwnd_without_sendinput(hwnd);
    let _ = bar_win.set_focus();
    // If Selection Mode overlay is active, re-assert Overlay strictly above Bar after SetForegroundWindow/set_focus
    raise_overlay_topmost_if_visible();
    let _ = bar_win.eval(
        "if (window.__DSB_BRIDGE__) { window.__DSB_BRIDGE__.focusComposer(); setTimeout(() => window.__DSB_BRIDGE__ && window.__DSB_BRIDGE__.focusComposer(), 60); }",
    );
    if !LOGGED_FIRST_ACTIVATE.swap(true, Ordering::AcqRel) {
        log_window_diagnostics(bar_win, "first-activate");
    }
}

pub fn hide_bar(bar_win: &WebviewWindow) {
    if let Some(hwnd) = hwnd_of(bar_win) {
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
    // Intentionally do NOT call `bar_win.hide()`: tao's WindowFlags::apply_diff resets
    // GWL_STYLE/GWL_EXSTYLE back to WS_CAPTION | WS_SYSMENU + SWP_FRAMECHANGED and can
    // desync WebView2's ICoreWebView2Controller visibility on subsequent ShowWindow calls.
}

pub fn position_and_show_overlay(overlay_win: &WebviewWindow) -> (MonitorGeometry, f64) {
    let mon = query_cursor_monitor_geometry(None);
    let w = (mon.monitor_right - mon.monitor_left).max(1) as u32;
    let h = (mon.monitor_bottom - mon.monitor_top).max(1) as u32;

    // Resize & position through Tauri so WebView2's ICoreWebView2Controller bounds stay in sync (prevents 800x600 top-left artifact)
    let _ = overlay_win.set_position(tauri::PhysicalPosition::new(
        mon.monitor_left,
        mon.monitor_top,
    ));
    let _ = overlay_win.set_size(tauri::PhysicalSize::new(w, h));
    let _ = overlay_win.set_ignore_cursor_events(true);

    let Some(hwnd) = hwnd_of(overlay_win) else {
        return (mon, 1.0);
    };
    OVERLAY_HWND_RAW.store(hwnd.0 as usize as u64, Ordering::Release);
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            mon.monitor_left,
            mon.monitor_top,
            w as i32,
            h as i32,
            SWP_NOACTIVATE,
        );
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        // Re-assert topmost AFTER ShowWindow so Overlay always renders strictly above Bar
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
    let scale = dpi_scale_for_hwnd(hwnd);
    (mon, scale)
}

pub fn hide_overlay(overlay_win: &WebviewWindow) {
    if let Some(hwnd) = hwnd_of(overlay_win) {
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

pub fn is_ime_or_voice_helper_identity(exe_name: &str, class_name: &str) -> bool {
    let exe = exe_name.trim().to_ascii_lowercase();
    let cls = class_name.trim().to_ascii_lowercase();
    if exe == "voiceinput.exe"
        || exe == "fytool.exe"
        || exe == "textinputhost.exe"
        || exe == "ctfmon.exe"
        || exe == "logioptionsplus_logivoice.exe"
        || exe.starts_with("chatterfly")
        || exe.starts_with("sogou")
        || exe.starts_with("baiduime")
        || exe.starts_with("ifly")
        || exe.starts_with("qqpinyin")
        || exe.starts_with("msime")
        || exe.starts_with("wechatime")
        || exe.starts_with("wetype")
    {
        return true;
    }
    cls.contains("chatterfly")
        || cls.contains("voiceinput")
        || cls.contains("logivoice")
        || cls.contains("candidate")
        || cls.contains("msctf")
        || cls.contains("inputapp")
        || cls.contains("sogou_ime")
        || cls.contains("ime_comp")
        || cls.contains("ime_status")
}

fn query_process_exe_basename(pid: u32) -> String {
    if pid == 0 {
        return String::new();
    }
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return String::new();
        };
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(handle);
        if !ok || len == 0 {
            return String::new();
        }
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        full.rsplit(['\\', '/']).next().unwrap_or("").to_string()
    }
}

/// Returns `true` if `pt` lands on a floating voice-input capsule (Chatterfly, LogiVoice)
/// or an IME candidate/toolbar window so clicking it does not dismiss our floating Bar.
pub fn is_ime_or_voice_helper_at_point(pt: POINT) -> bool {
    unsafe {
        let hit = WindowFromPoint(pt);
        if hit.0.is_null() {
            return false;
        }
        let root = GetAncestor(hit, GA_ROOT);
        let target = if root.0.is_null() { hit } else { root };
        let class_name = get_hwnd_class_name(target);
        let hit_class = if target != hit {
            get_hwnd_class_name(hit)
        } else {
            String::new()
        };
        let mut pid = 0u32;
        GetWindowThreadProcessId(target, Some(&mut pid));
        let exe_name = query_process_exe_basename(pid);
        is_ime_or_voice_helper_identity(&exe_name, &class_name)
            || (!hit_class.is_empty() && is_ime_or_voice_helper_identity(&exe_name, &hit_class))
    }
}

pub fn is_point_on_bar_window(pt: POINT, bar_hwnd: HWND) -> bool {
    if bar_hwnd.0.is_null() {
        return false;
    }
    unsafe {
        if !IsWindowVisible(bar_hwnd).as_bool() {
            return false;
        }
        let hit = WindowFromPoint(pt);
        if hit.0.is_null() {
            return false;
        }
        if hit == bar_hwnd {
            return true;
        }
        let root = GetAncestor(hit, GA_ROOT);
        if root == bar_hwnd {
            return true;
        }
        let mut hit_pid = 0u32;
        GetWindowThreadProcessId(hit, Some(&mut hit_pid));
        hit_pid == std::process::id()
    }
}

pub fn log_window_diagnostics(bar_win: &WebviewWindow, tag: &str) {
    let Some(hwnd) = hwnd_of(bar_win) else {
        return;
    };
    unsafe {
        let vis = IsWindowVisible(hwnd).as_bool();
        let mut rc = RECT::default();
        let _ = GetWindowRect(hwnd, &mut rc);
        let mut crc = RECT::default();
        let _ = GetClientRect(hwnd, &mut crc);
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let scale = dpi_scale_for_hwnd(hwnd);

        let above_pt = POINT {
            x: (rc.left + rc.right) / 2,
            y: rc.top - 10,
        };
        let corner_out_pt = POINT {
            x: rc.left + 2,
            y: rc.top + 2,
        };
        let center_pt = POINT {
            x: (rc.left + rc.right) / 2,
            y: (rc.top + rc.bottom) / 2,
        };

        let above_own = is_point_on_bar_window(above_pt, hwnd);
        let corner_own = is_point_on_bar_window(corner_out_pt, hwnd);
        let center_own = is_point_on_bar_window(center_pt, hwnd);

        let own_pid = std::process::id();
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );

        log::info!(
            "[DEV-DUMP:{tag}] visible={vis} scale={scale:.2} style=0x{style:08x} ex=0x{ex_style:08x} rect=({},{})..({},{}) size={}x{} client={}x{} own_pid={own_pid} hit_above_own={above_own} hit_corner2_own={corner_own} hit_center_own={center_own}",
            rc.left,
            rc.top,
            rc.right,
            rc.bottom,
            rc.right - rc.left,
            rc.bottom - rc.top,
            crc.right - crc.left,
            crc.bottom - crc.top,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bottom_anchor_math_centers_horizontally_and_respects_margin() {
        // 1920x1040 work area at 100% scale (1.0)
        let (x, y, w, h) = compute_bottom_anchored_rect((0, 0, 1920, 1040), 760, 56, 12, 1.0);
        assert_eq!(w, 760);
        assert_eq!(h, 56);
        assert_eq!(x, (1920 - 760) / 2);
        assert_eq!(y, 1040 - 12 - 56);

        // Growing upward keeps bottom edge fixed at 1028
        let (x2, y2, w2, h2) = compute_bottom_anchored_rect((0, 0, 1920, 1040), 760, 380, 12, 1.0);
        assert_eq!(x2, x);
        assert_eq!(w2, 760);
        assert_eq!(y2 + h2, y + h);
    }

    #[test]
    fn shape_scaling_at_125_percent_dpi() {
        let shape = ShapeRectDip {
            x: 0,
            y: 100,
            w: 760,
            h: 56,
            r: 24,
        };
        let (l, t, r, b, diam) = scale_shape_to_phys(&shape, 1.25);
        assert_eq!(l, 0);
        assert_eq!(t, 125);
        assert_eq!(r, 950);
        assert_eq!(b, 195);
        assert_eq!(diam, 60);
    }

    #[test]
    fn clean_bar_styles_strip_all_caption_and_border_bits() {
        let tao_default = WS_CAPTION.0 | WS_SYSMENU.0 | WS_THICKFRAME.0 | WS_CLIPSIBLINGS.0;
        let cleaned = clean_bar_style(tao_default);
        assert_eq!(cleaned & WS_CAPTION.0, 0);
        assert_eq!(cleaned & WS_SYSMENU.0, 0);
        assert_eq!(cleaned & WS_THICKFRAME.0, 0);
        assert_eq!(cleaned & WS_BORDER.0, 0);
        assert_eq!(cleaned & WS_DLGFRAME.0, 0);
        assert_ne!(cleaned & WS_POPUP.0, 0);

        let ex_cleaned = clean_bar_ex_style(
            WS_EX_APPWINDOW.0 | WS_EX_WINDOWEDGE.0 | WS_EX_DLGMODALFRAME.0,
            false,
        );
        assert_eq!(ex_cleaned & WS_EX_APPWINDOW.0, 0);
        assert_eq!(ex_cleaned & WS_EX_WINDOWEDGE.0, 0);
        assert_eq!(ex_cleaned & WS_EX_DLGMODALFRAME.0, 0);
        assert_eq!(ex_cleaned & WS_EX_NOACTIVATE.0, 0);
        assert_ne!(ex_cleaned & WS_EX_TOOLWINDOW.0, 0);
    }

    #[test]
    fn recognizes_chatterfly_and_ime_floating_helpers() {
        assert!(is_ime_or_voice_helper_identity("voiceinput.exe", "Chrome_WidgetWin_1"));
        assert!(is_ime_or_voice_helper_identity("ChatterflyCloud.exe", "SomeWnd"));
        assert!(is_ime_or_voice_helper_identity("TextInputHost.exe", "ApplicationFrameWindow"));
        assert!(is_ime_or_voice_helper_identity("unknown.exe", "MSCTFIME_Candidate"));
        assert!(!is_ime_or_voice_helper_identity("chrome.exe", "Chrome_WidgetWin_1"));
        assert!(!is_ime_or_voice_helper_identity("Antigravity.exe", "Chrome_WidgetWin_1"));
    }

    #[test]
    fn custom_anchor_keeps_bottom_edge_fixed_and_clamps_to_bounds() {
        let bounds = (0, 0, 1920, 1040);
        let (x1, y1) = compute_custom_anchored_rect(1200, 820, 760, 152, bounds);
        assert_eq!(x1, 1200 - 380);
        assert_eq!(y1 + 152, 820);

        // Expanding upward keeps bottom edge fixed at 820
        let (x2, y2) = compute_custom_anchored_rect(1200, 820, 760, 490, bounds);
        assert_eq!(x2, x1);
        assert_eq!(y2 + 490, 820);

        // Near top edge: clamps cleanly to y=0 without flying off-screen
        let (_x3, y3) = compute_custom_anchored_rect(1200, 300, 760, 490, bounds);
        assert_eq!(y3, 0);
    }

    #[test]
    fn shape_hit_test_rejects_transparent_upper_stage_and_accepts_composer_and_popup() {
        let shapes = vec![
            (0, 590, 812, 680), // bottom composer inside 680px stage
            (20, 260, 320, 582), // open popup above composer
        ];
        // Transparent click-through area above composer & outside popup -> outside!
        assert!(!point_hits_any_phys_rect(500, 120, &shapes));
        assert!(!point_hits_any_phys_rect(500, 400, &shapes));
        // Inside open popup -> inside!
        assert!(point_hits_any_phys_rect(150, 400, &shapes));
        // Inside bottom composer -> inside!
        assert!(point_hits_any_phys_rect(400, 640, &shapes));
    }

    #[test]
    fn docked_bar_rect_computes_all_four_edges_expanded_and_collapsed() {
        let work = (0, 0, 1920, 1040);
        let (stage_w, stage_h) = compute_constant_stage_phys(work, 1.0);
        assert_eq!(stage_w, BAR_STAGE_MAX_WIDTH_DIP);
        assert_eq!(stage_h, (1040.0 * SIDE_DOCK_HEIGHT_RATIO).round() as i32);

        // Bottom dock: constant 960dip stage flush at work_bottom
        let (bx, by, bw, bh) =
            compute_docked_bar_rect(DockEdge::Bottom, false, false, false, work, 760, 104, 1.0);
        assert_eq!((bw, bh), (stage_w, stage_h));
        assert_eq!(bx, (1920 - stage_w) / 2);
        assert_eq!(by + bh, 1040);

        let (bcx, bcy, bcw, bch) =
            compute_docked_bar_rect(DockEdge::Bottom, true, false, false, work, 760, 104, 1.0);
        assert_eq!((bcx, bcy, bcw, bch), (bx, by, bw, bh));

        // Top dock: constant stage flush at top; collapsed RGN is None idle / 9dip unread
        let (tx, ty, tw, th) =
            compute_docked_bar_rect(DockEdge::Top, false, false, false, work, 760, 104, 1.0);
        assert_eq!((tw, th), (stage_w, stage_h));
        assert_eq!(tx, (1920 - stage_w) / 2);
        assert_eq!(ty, 0);
        assert_eq!(compute_dock_collapsed_shape_phys(DockEdge::Top, false, tw, th, 1.0), None);
        assert_eq!(
            compute_dock_collapsed_shape_phys(DockEdge::Top, true, tw, th, 1.0).map(|s| s.3),
            Some(DOCK_PEEK_DIP)
        );

        // Left dock: constant stage flush at left; collapsed RGN is None idle / 9dip unread
        let (lx, ly, lw, lh) =
            compute_docked_bar_rect(DockEdge::Left, false, false, true, work, 760, 104, 1.0);
        assert_eq!((lw, lh), (stage_w, stage_h));
        assert_eq!(lx, 0);
        assert_eq!(ly, (1040 - stage_h) / 2);
        assert_eq!(compute_dock_collapsed_shape_phys(DockEdge::Left, false, lw, lh, 1.0), None);
        assert_eq!(
            compute_dock_collapsed_shape_phys(DockEdge::Left, true, lw, lh, 1.0).map(|s| s.2),
            Some(DOCK_PEEK_DIP)
        );

        // Right dock: constant stage flush at right; collapsed RGN is None idle / 9dip unread
        let (rx, ry, rw, rh) =
            compute_docked_bar_rect(DockEdge::Right, false, false, true, work, 760, 104, 1.0);
        assert_eq!((rw, rh), (stage_w, stage_h));
        assert_eq!(rx + rw, 1920);
        assert_eq!(ry, ly);
        assert_eq!(compute_dock_collapsed_shape_phys(DockEdge::Right, false, rw, rh, 1.0), None);
        assert_eq!(
            compute_dock_collapsed_shape_phys(DockEdge::Right, true, rw, rh, 1.0).map(|s| (s.0, s.2)),
            Some((rw - DOCK_PEEK_DIP, rw))
        );
        // Bottom collapsed RGN is centered handle pill
        assert!(compute_dock_collapsed_shape_phys(DockEdge::Bottom, false, bw, bh, 1.0).is_some());
    }

    #[test]
    fn edge_snap_detection_and_inward_pull_undock_thresholds() {
        let work = (0, 0, 1920, 1040);
        assert_eq!(detect_snap_edge_for_cursor(POINT { x: 10, y: 500 }, work, true), DockEdge::Left);
        assert_eq!(detect_snap_edge_for_cursor(POINT { x: 1910, y: 500 }, work, true), DockEdge::Right);
        assert_eq!(detect_snap_edge_for_cursor(POINT { x: 960, y: 12 }, work, true), DockEdge::Top);
        assert_eq!(detect_snap_edge_for_cursor(POINT { x: 960, y: 1030 }, work, true), DockEdge::Bottom);
        assert_eq!(detect_snap_edge_for_cursor(POINT { x: 960, y: 520 }, work, true), DockEdge::None);
        assert_eq!(detect_snap_edge_for_cursor(POINT { x: 10, y: 500 }, work, false), DockEdge::None);

        // Inward pull >= 50px undocks; small jitter stays docked
        assert!(!should_undock_on_inward_pull(DockEdge::Left, POINT { x: 10, y: 500 }, POINT { x: 40, y: 500 }));
        assert!(should_undock_on_inward_pull(DockEdge::Left, POINT { x: 10, y: 500 }, POINT { x: 65, y: 500 }));
        assert!(!should_undock_on_inward_pull(DockEdge::Right, POINT { x: 1910, y: 500 }, POINT { x: 1880, y: 500 }));
        assert!(should_undock_on_inward_pull(DockEdge::Right, POINT { x: 1910, y: 500 }, POINT { x: 1850, y: 500 }));
        assert!(!should_undock_on_inward_pull(DockEdge::Top, POINT { x: 960, y: 10 }, POINT { x: 960, y: 40 }));
        assert!(should_undock_on_inward_pull(DockEdge::Top, POINT { x: 960, y: 10 }, POINT { x: 960, y: 70 }));
        assert!(!should_undock_on_inward_pull(DockEdge::Bottom, POINT { x: 960, y: 1030 }, POINT { x: 960, y: 1000 }));
        assert!(should_undock_on_inward_pull(DockEdge::Bottom, POINT { x: 960, y: 1030 }, POINT { x: 960, y: 960 }));
    }
}
