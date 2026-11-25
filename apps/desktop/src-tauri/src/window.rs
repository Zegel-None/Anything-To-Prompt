use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tauri::WebviewWindow;
use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMNCRP_DISABLED, DWMWA_NCRENDERING_POLICY,
};
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateRoundRectRgn, DeleteObject, GetMonitorInfoW, MonitorFromPoint,
    MonitorFromWindow, SetWindowRgn, HRGN, MONITORINFO, MONITOR_DEFAULTTONEAREST, RGN_OR,
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
    SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOW,
    SW_SHOWNOACTIVATE, WDA_EXCLUDEFROMCAPTURE, WDA_NONE, WINDOWPOS, WINDOW_DISPLAY_AFFINITY,
    WM_NCACTIVATE, WM_NCCALCSIZE, WM_NCDESTROY, WM_NCPAINT, WM_STYLECHANGING, WM_SYSCOMMAND,
    WM_WINDOWPOSCHANGING, WS_BORDER, WS_CAPTION, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_DLGFRAME,
    WS_EX_APPWINDOW, WS_EX_CLIENTEDGE, WS_EX_DLGMODALFRAME, WS_EX_NOACTIVATE, WS_EX_STATICEDGE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_WINDOWEDGE, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP,
    WS_SYSMENU, WS_THICKFRAME,
};

pub const BOTTOM_MARGIN_DIP: i32 = 12;
pub const DEFAULT_LIP_HEIGHT_DIP: i32 = 56;

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
    unsafe {
        let _ = SetWindowSubclass(hwnd, Some(bar_borderless_subclass_proc), BAR_SUBCLASS_ID, 0);
        let _ = enforce_borderless_popup_style(hwnd, true);
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
        x: 0,
        y: 0,
        w: width_dip,
        h: DEFAULT_LIP_HEIGHT_DIP,
        r: 24,
    };
    apply_bar_layout(bar_win, width_dip, DEFAULT_LIP_HEIGHT_DIP, &[default_shape]);
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

/// Positions Bar anchored to the bottom of the cursor's monitor work area and clips
/// its Win32 hit-test/paint region to the exact union of `shapes` (24px rounded slab + popups).
pub fn apply_bar_layout(
    bar_win: &WebviewWindow,
    width_dip: i32,
    height_dip: i32,
    shapes: &[ShapeRectDip],
) {
    let Some(hwnd) = hwnd_of(bar_win) else {
        return;
    };
    let scale = dpi_scale_for_hwnd(hwnd);
    let mon = query_cursor_monitor_geometry(Some(hwnd));
    let (x, y, phys_w, phys_h) = compute_bottom_anchored_rect(
        (
            mon.anchor_left,
            mon.anchor_top,
            mon.anchor_right,
            mon.anchor_bottom,
        ),
        width_dip,
        height_dip,
        BOTTOM_MARGIN_DIP,
        scale,
    );

    unsafe {
        let overlay_vis = visible_overlay_hwnd();
        let already_topmost = (GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0) != 0;
        if overlay_vis.is_none() && already_topmost {
            // Preserve existing topmost Z-order without fighting floating voice-input capsules
            // (Chatterfly, LogiVoice) or IME candidate windows on every resize/layout report!
            let _ = SetWindowPos(
                hwnd,
                None,
                x,
                y,
                phys_w,
                phys_h,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        } else {
            let insert_after = overlay_vis.unwrap_or(HWND_TOPMOST);
            let _ = SetWindowPos(
                hwnd,
                Some(insert_after),
                x,
                y,
                phys_w,
                phys_h,
                SWP_NOACTIVATE,
            );
        }

        let master_rgn: HRGN = if shapes.is_empty() {
            let diam = (48.0 * scale).round() as i32;
            CreateRoundRectRgn(0, 0, phys_w + 1, phys_h + 1, diam, diam)
        } else {
            let mut acc: Option<HRGN> = None;
            for s in shapes {
                if s.w <= 2 || s.h <= 2 {
                    continue;
                }
                let (l, t, r, b, diam) = scale_shape_to_phys(s, scale);
                // CreateRoundRectRgn excludes right/bottom edge by 1px in GDI, so +1 keeps exact border
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

        // Note: Windows takes ownership of `master_rgn` on success; do NOT DeleteObject it!
        let _ = SetWindowRgn(hwnd, Some(master_rgn), true);
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
}
