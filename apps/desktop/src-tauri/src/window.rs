use serde::{Deserialize, Serialize};
use tauri::WebviewWindow;
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMNCRP_DISABLED, DWMWA_NCRENDERING_POLICY,
};
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateRoundRectRgn, DeleteObject, GetMonitorInfoW, MonitorFromPoint,
    MonitorFromWindow, SetWindowRgn, HRGN, MONITORINFO, MONITOR_DEFAULTTONEAREST, RGN_OR,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetCursorPos, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect,
    GetWindowThreadProcessId, IsWindowVisible, SetForegroundWindow, SetWindowDisplayAffinity,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, WindowFromPoint, GA_ROOT, GWL_EXSTYLE,
    HWND_TOPMOST, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE,
    SW_SHOW, SW_SHOWNOACTIVATE, WDA_EXCLUDEFROMCAPTURE, WDA_NONE, WINDOW_DISPLAY_AFFINITY,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
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

pub fn init_bar_window(bar_win: &WebviewWindow, exclude_from_capture: bool, width_dip: i32) {
    let Some(hwnd) = hwnd_of(bar_win) else {
        return;
    };
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let next_ex = ex | (WS_EX_TOOLWINDOW.0 as isize) | (WS_EX_NOACTIVATE.0 as isize);
        let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, next_ex);
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
}

pub fn init_overlay_window(overlay_win: &WebviewWindow, exclude_from_capture: bool) {
    // Use Tauri's DirectComposition-safe click-through API (never touch WebView2 child HWNDs with WS_EX_LAYERED!)
    let _ = overlay_win.set_ignore_cursor_events(true);

    let Some(hwnd) = hwnd_of(overlay_win) else {
        return;
    };
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
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            x,
            y,
            phys_w,
            phys_h,
            SWP_NOACTIVATE,
        );

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
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let _ = SetWindowLongPtrW(
            hwnd,
            GWL_EXSTYLE,
            ex | (WS_EX_TOOLWINDOW.0 as isize) | (WS_EX_NOACTIVATE.0 as isize),
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
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
}

pub fn send_dummy_vk_e8() {
    unsafe {
        let inputs = [
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0xE8),
                        wScan: 0,
                        dwFlags: Default::default(),
                        time: 0,
                        dwExtraInfo: 0xD5B0_00E8,
                    },
                },
            },
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0xE8),
                        wScan: 0,
                        dwFlags: KEYEVENTF_KEYUP,
                        time: 0,
                        dwExtraInfo: 0xD5B0_00E8,
                    },
                },
            },
        ];
        let _ = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

pub fn activate_and_focus_bar(bar_win: &WebviewWindow) {
    let Some(hwnd) = hwnd_of(bar_win) else {
        return;
    };
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let cleared = (ex | (WS_EX_TOOLWINDOW.0 as isize)) & !(WS_EX_NOACTIVATE.0 as isize);
        let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, cleared);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
        let _ = ShowWindow(hwnd, SW_SHOW);
        // Satisfy Windows foreground lock from background hotkey hook
        send_dummy_vk_e8();
        let _ = SetForegroundWindow(hwnd);
    }
    let _ = bar_win.set_focus();
    let _ = bar_win.eval(
        "if (window.__DSB_BRIDGE__) { window.__DSB_BRIDGE__.focusComposer(); setTimeout(() => window.__DSB_BRIDGE__ && window.__DSB_BRIDGE__.focusComposer(), 60); }",
    );
}

pub fn hide_bar(bar_win: &WebviewWindow) {
    if let Some(hwnd) = hwnd_of(bar_win) {
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
    let _ = bar_win.hide();
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
            "[DEV-DUMP:{tag}] visible={vis} scale={scale:.2} rect=({},{})..({},{}) size={}x{} own_pid={own_pid} hit_above_own={above_own} hit_corner2_own={corner_own} hit_center_own={center_own}",
            rc.left,
            rc.top,
            rc.right,
            rc.bottom,
            rc.right - rc.left,
            rc.bottom - rc.top,
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
}
