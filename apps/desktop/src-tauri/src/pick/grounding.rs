use crate::captures::ScreenRect;
use crate::window::dpi_scale_for_hwnd;
use serde::{Deserialize, Serialize};
use uiautomation::patterns::{
    UIExpandCollapsePattern, UIInvokePattern, UIRangeValuePattern, UIScrollPattern,
    UISelectionItemPattern, UITextPattern, UITogglePattern, UIValuePattern,
};
use uiautomation::UIElement;
use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, MonitorFromPoint, HDC, HMONITOR, MONITORINFO,
    MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;

const CF_UNICODETEXT: u32 = 13;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct MonitorSpace {
    pub index: usize,
    pub is_primary: bool,
    pub origin_x: i32,
    pub origin_y: i32,
    pub width_px: i32,
    pub height_px: i32,
    pub dpi_scale: f64,
}

impl MonitorSpace {
    pub fn logical_size(&self) -> (i32, i32) {
        let s = self.dpi_scale.max(0.5);
        (
            ((self.width_px as f64) / s).round() as i32,
            ((self.height_px as f64) / s).round() as i32,
        )
    }

    /// Queries the exact monitor geometry, deterministic 0-based index (primary = 0),
    /// and effective DPI scale at screen point `(x, y)`.
    pub fn for_point(x: i32, y: i32, fallback_hwnd: HWND) -> Self {
        unsafe {
            let hmon = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(hmon, &mut mi).as_bool() {
                let scale = if !fallback_hwnd.0.is_null() {
                    dpi_scale_for_hwnd(fallback_hwnd)
                } else {
                    1.0
                };
                return Self {
                    index: 0,
                    is_primary: true,
                    origin_x: 0,
                    origin_y: 0,
                    width_px: 1920,
                    height_px: 1080,
                    dpi_scale: scale,
                };
            }

            let is_primary = (mi.dwFlags & MONITORINFOF_PRIMARY) != 0;
            let mut dpi_x: u32 = 0;
            let mut dpi_y: u32 = 0;
            let dpi_scale = if GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y)
                .is_ok()
                && dpi_x >= 48
            {
                (dpi_x as f64) / 96.0
            } else if !fallback_hwnd.0.is_null() {
                dpi_scale_for_hwnd(fallback_hwnd)
            } else {
                1.0
            };

            let index = if is_primary {
                0
            } else {
                resolve_monitor_index(hmon)
            };

            Self {
                index,
                is_primary,
                origin_x: mi.rcMonitor.left,
                origin_y: mi.rcMonitor.top,
                width_px: (mi.rcMonitor.right - mi.rcMonitor.left).max(1),
                height_px: (mi.rcMonitor.bottom - mi.rcMonitor.top).max(1),
                dpi_scale,
            }
        }
    }
}

unsafe extern "system" fn enum_monitors_proc(
    hmon: HMONITOR,
    _hdc: HDC,
    _rc: *mut RECT,
    lparam: LPARAM,
) -> windows::core::BOOL {
    let list = &mut *(lparam.0 as *mut Vec<(isize, bool, i32, i32)>);
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if GetMonitorInfoW(hmon, &mut mi).as_bool() {
        let is_primary = (mi.dwFlags & MONITORINFOF_PRIMARY) != 0;
        list.push((
            hmon.0 as isize,
            is_primary,
            mi.rcMonitor.left,
            mi.rcMonitor.top,
        ));
    }
    true.into()
}

fn resolve_monitor_index(target: HMONITOR) -> usize {
    let mut list: Vec<(isize, bool, i32, i32)> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(enum_monitors_proc),
            LPARAM(&mut list as *mut _ as isize),
        );
    }
    // Primary monitor always gets index 0; remaining monitors sorted left-to-right, top-to-bottom.
    list.sort_by_key(|&(_, is_primary, left, top)| (!is_primary, left, top));
    list.iter()
        .position(|&(raw, _, _, _)| raw == target.0 as isize)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct MultiCoord {
    pub px: (i32, i32),
    pub dip: (i32, i32),
    pub norm1000: (i32, i32),
    pub win_rel_px: (i32, i32),
    pub win_rel_ratio: (f64, f64),
}

impl MultiCoord {
    /// Deterministically projects a physical screen coordinate `(x, y)` into all 4 coordinate
    /// systems used across mainstream Computer Use harnesses:
    /// - `px`: raw physical screen pixels (Win32 `SendInput` / DXGI / Windows-MCP)
    /// - `dip`: DPI-normalized logical pixels (PyAutoGUI / DPI-unaware tools)
    /// - `norm1000`: 0..=1000 monitor-normalized integers (Gemini / Antigravity / canvas scaling)
    /// - `win_rel`: offset & ratio relative to target window top-left (survives window moves)
    pub fn compute(x: i32, y: i32, mon: &MonitorSpace, win_rect: ScreenRect) -> Self {
        let scale = mon.dpi_scale.max(0.5);
        let dip_x = ((x as f64) / scale).round() as i32;
        let dip_y = ((y as f64) / scale).round() as i32;

        let nx = (((x - mon.origin_x) as f64) * 1000.0 / (mon.width_px.max(1) as f64))
            .round()
            .clamp(0.0, 1000.0) as i32;
        let ny = (((y - mon.origin_y) as f64) * 1000.0 / (mon.height_px.max(1) as f64))
            .round()
            .clamp(0.0, 1000.0) as i32;

        let rel_x = x - win_rect.x;
        let rel_y = y - win_rect.y;
        let rx = if win_rect.w > 0 {
            ((rel_x as f64) / (win_rect.w as f64)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let ry = if win_rect.h > 0 {
            ((rel_y as f64) / (win_rect.h as f64)).clamp(0.0, 1.0)
        } else {
            0.0
        };

        Self {
            px: (x, y),
            dip: (dip_x, dip_y),
            norm1000: (nx, ny),
            win_rel_px: (rel_x, rel_y),
            win_rel_ratio: (rx, ry),
        }
    }

    pub fn format_inline(&self) -> String {
        format!(
            "px=({},{}) dip=({},{}) norm1000=({},{}) win_rel=({},{}) win_ratio=({:.3},{:.3})",
            self.px.0,
            self.px.1,
            self.dip.0,
            self.dip.1,
            self.norm1000.0,
            self.norm1000.1,
            self.win_rel_px.0,
            self.win_rel_px.1,
            self.win_rel_ratio.0,
            self.win_rel_ratio.1,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BboxCoords {
    pub px: [i32; 4],
    pub dip: [i32; 4],
    pub norm1000: [i32; 4],
    pub center: MultiCoord,
}

impl BboxCoords {
    pub fn compute(rect: ScreenRect, mon: &MonitorSpace, win_rect: ScreenRect) -> Self {
        let x1 = rect.x;
        let y1 = rect.y;
        let x2 = rect.x + rect.w.max(0);
        let y2 = rect.y + rect.h.max(0);
        let tl = MultiCoord::compute(x1, y1, mon, win_rect);
        let br = MultiCoord::compute(x2, y2, mon, win_rect);
        let cx = x1 + rect.w.max(0) / 2;
        let cy = y1 + rect.h.max(0) / 2;
        let center = MultiCoord::compute(cx, cy, mon, win_rect);
        Self {
            px: [x1, y1, x2, y2],
            dip: [tl.dip.0, tl.dip.1, br.dip.0, br.dip.1],
            norm1000: [tl.norm1000.0, tl.norm1000.1, br.norm1000.0, br.norm1000.1],
            center,
        }
    }

    pub fn format_inline(&self) -> String {
        format!(
            "px=[{},{},{},{}] dip=[{},{},{},{}] norm1000=[{},{},{},{}]",
            self.px[0],
            self.px[1],
            self.px[2],
            self.px[3],
            self.dip[0],
            self.dip[1],
            self.dip[2],
            self.dip[3],
            self.norm1000[0],
            self.norm1000[1],
            self.norm1000[2],
            self.norm1000[3],
        )
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UiaAffordance {
    pub enabled: bool,
    pub focusable: bool,
    pub offscreen: bool,
    pub runtime_id: Vec<i32>,
    pub patterns: Vec<String>,
    pub state_summary: String,
}

impl UiaAffordance {
    pub fn extract(el: &UIElement) -> Self {
        let enabled = el.is_enabled().unwrap_or(true);
        let focusable = el.is_keyboard_focusable().unwrap_or(false);
        let offscreen = el.is_offscreen().unwrap_or(false);
        let runtime_id = el.get_runtime_id().unwrap_or_default();

        let mut patterns = Vec::new();
        let mut states = Vec::new();

        if el.get_pattern::<UIInvokePattern>().is_ok() {
            patterns.push("Invoke".to_string());
        }
        if let Ok(vp) = el.get_pattern::<UIValuePattern>() {
            patterns.push("Value".to_string());
            if vp.is_readonly().unwrap_or(false) {
                states.push("readonly=true".to_string());
            }
        }
        if let Ok(tp) = el.get_pattern::<UITogglePattern>() {
            patterns.push("Toggle".to_string());
            if let Ok(st) = tp.get_toggle_state() {
                states.push(format!("toggle={st:?}"));
            }
        }
        if let Ok(sp) = el.get_pattern::<UISelectionItemPattern>() {
            patterns.push("SelectionItem".to_string());
            if let Ok(sel) = sp.is_selected() {
                states.push(format!("selected={sel}"));
            }
        }
        if let Ok(ep) = el.get_pattern::<UIExpandCollapsePattern>() {
            patterns.push("ExpandCollapse".to_string());
            if let Ok(st) = ep.get_state() {
                states.push(format!("expand={st:?}"));
            }
        }
        if el.get_pattern::<UIScrollPattern>().is_ok() {
            patterns.push("Scroll".to_string());
        }
        if el.get_pattern::<UIRangeValuePattern>().is_ok() {
            patterns.push("RangeValue".to_string());
        }
        if el.get_pattern::<UITextPattern>().is_ok() {
            patterns.push("Text".to_string());
        }

        Self {
            enabled,
            focusable,
            offscreen,
            runtime_id,
            patterns,
            state_summary: states.join(" "),
        }
    }

    pub fn format_line(&self, control_type: &str) -> String {
        let rid = if self.runtime_id.is_empty() {
            String::new()
        } else {
            let joined = self
                .runtime_id
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(",");
            format!(" runtime_id=[{joined}]")
        };
        let state = if self.state_summary.is_empty() {
            String::new()
        } else {
            format!(" state=\"{}\"", self.state_summary)
        };
        format!(
            "  uia_affordance: type={} enabled={} focusable={} offscreen={} patterns=[{}]{}{}",
            control_type,
            self.enabled,
            self.focusable,
            self.offscreen,
            self.patterns.join(","),
            state,
            rid
        )
    }
}

fn format_monitor_and_window_lines(
    hwnd: HWND,
    win_rect: ScreenRect,
    mon: &MonitorSpace,
) -> Vec<String> {
    let (lw, lh) = mon.logical_size();
    vec![
        "computer_use_grounding:".to_string(),
        format!(
            "  monitor: index={} primary={} origin=({},{}) physical={}x{} logical={}x{} dpi_scale={:.2}",
            mon.index,
            mon.is_primary,
            mon.origin_x,
            mon.origin_y,
            mon.width_px,
            mon.height_px,
            lw,
            lh,
            mon.dpi_scale
        ),
        format!(
            "  window: hwnd=0x{:X} rect_px=({},{},{},{})",
            hwnd.0 as usize, win_rect.x, win_rect.y, win_rect.w, win_rect.h
        ),
    ]
}

/// Formats compact, deterministic Computer Use grounding lines for a single-click pick.
pub fn format_tap_grounding(
    hwnd: HWND,
    win_rect: ScreenRect,
    mon: &MonitorSpace,
    click_pt: (i32, i32),
    element_rect: ScreenRect,
    control_type: &str,
    affordance: Option<&UiaAffordance>,
) -> Vec<String> {
    let mut lines = format_monitor_and_window_lines(hwnd, win_rect, mon);
    let click = MultiCoord::compute(click_pt.0, click_pt.1, mon, win_rect);
    let has_box = element_rect.w > 0 && element_rect.h > 0;
    if has_box {
        let bbox = BboxCoords::compute(element_rect, mon, win_rect);
        lines.push(format!("  element_center: {}", bbox.center.format_inline()));
        lines.push(format!("  click_point: {}", click.format_inline()));
        lines.push(format!("  element_bbox: {}", bbox.format_inline()));
    } else {
        lines.push(format!("  click_point: {}", click.format_inline()));
    }
    if let Some(aff) = affordance {
        lines.push(aff.format_line(control_type));
    }
    lines
}

/// Formats compact, deterministic Computer Use grounding lines for a drag / box / freehand pick.
pub fn format_drag_grounding(
    hwnd: HWND,
    win_rect: ScreenRect,
    mon: &MonitorSpace,
    start_pt: (i32, i32),
    end_pt: (i32, i32),
    selection_rect: ScreenRect,
) -> Vec<String> {
    let mut lines = format_monitor_and_window_lines(hwnd, win_rect, mon);
    let bbox = BboxCoords::compute(selection_rect, mon, win_rect);
    let start = MultiCoord::compute(start_pt.0, start_pt.1, mon, win_rect);
    let end = MultiCoord::compute(end_pt.0, end_pt.1, mon, win_rect);
    lines.push(format!("  bbox_center: {}", bbox.center.format_inline()));
    lines.push(format!("  drag_start: {}", start.format_inline()));
    lines.push(format!("  drag_end: {}", end.format_inline()));
    lines.push(format!("  selection_bbox: {}", bbox.format_inline()));
    lines
}

/// Writes UTF-8 text cleanly to the Windows clipboard (`CF_UNICODETEXT`) so capsules can be
/// pasted directly into Codex, Claude Code, Antigravity, or any other AI harness.
pub fn copy_text_to_clipboard(text: &str) -> bool {
    let utf16: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let byte_len = utf16.len() * std::mem::size_of::<u16>();

    for _ in 0..5 {
        unsafe {
            if OpenClipboard(None).is_ok() {
                let _ = EmptyClipboard();
                if let Ok(hmem) = GlobalAlloc(GMEM_MOVEABLE, byte_len) {
                    let ptr = GlobalLock(hmem);
                    if !ptr.is_null() {
                        std::ptr::copy_nonoverlapping(utf16.as_ptr(), ptr as *mut u16, utf16.len());
                        let _ = GlobalUnlock(hmem);
                        let ok = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(hmem.0))).is_ok();
                        let _ = CloseClipboard();
                        return ok;
                    }
                    // Failed to lock; close clipboard and bail
                    let _ = hmem as HGLOBAL;
                }
                let _ = CloseClipboard();
                return false;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_all_four_coordinate_systems_at_125_dpi_accurately() {
        let mon = MonitorSpace {
            index: 0,
            is_primary: true,
            origin_x: 0,
            origin_y: 0,
            width_px: 2560,
            height_px: 1440,
            dpi_scale: 1.25,
        };
        let win = ScreenRect {
            x: 100,
            y: 80,
            w: 1400,
            h: 900,
        };
        let pt = MultiCoord::compute(1280, 720, &mon, win);
        assert_eq!(pt.px, (1280, 720));
        assert_eq!(pt.dip, (1024, 576));
        assert_eq!(pt.norm1000, (500, 500));
        assert_eq!(pt.win_rel_px, (1180, 640));
        assert!((pt.win_rel_ratio.0 - (1180.0 / 1400.0)).abs() < 1e-6);
        assert!((pt.win_rel_ratio.1 - (640.0 / 900.0)).abs() < 1e-6);
    }

    #[test]
    fn normalizes_negative_origin_secondary_monitor_into_0_to_1000() {
        let left_mon = MonitorSpace {
            index: 1,
            is_primary: false,
            origin_x: -1920,
            origin_y: 0,
            width_px: 1920,
            height_px: 1080,
            dpi_scale: 1.0,
        };
        let win = ScreenRect {
            x: -1920,
            y: 0,
            w: 1920,
            h: 1080,
        };
        // Center of left secondary monitor is (-960, 540) -> norm1000 must be (500, 500)
        let center = MultiCoord::compute(-960, 540, &left_mon, win);
        assert_eq!(center.px, (-960, 540));
        assert_eq!(center.norm1000, (500, 500));
        assert_eq!(center.win_rel_px, (960, 540));
    }

    #[test]
    fn formats_compact_tap_and_drag_grounding_blocks() {
        let mon = MonitorSpace {
            index: 0,
            is_primary: true,
            origin_x: 0,
            origin_y: 0,
            width_px: 2560,
            height_px: 1440,
            dpi_scale: 1.25,
        };
        let win = ScreenRect {
            x: 100,
            y: 80,
            w: 1400,
            h: 900,
        };
        let btn = ScreenRect {
            x: 780,
            y: 498,
            w: 124,
            h: 36,
        };
        let aff = UiaAffordance {
            enabled: true,
            focusable: true,
            offscreen: false,
            runtime_id: vec![42, 1001, 4],
            patterns: vec!["Invoke".into()],
            state_summary: String::new(),
        };
        let tap_lines = format_tap_grounding(
            HWND(0x1234 as *mut std::ffi::c_void),
            win,
            &mon,
            (838, 519),
            btn,
            "Button",
            Some(&aff),
        );
        let joined = tap_lines.join("\n");
        assert!(joined.contains("computer_use_grounding:"));
        assert!(joined.contains("dpi_scale=1.25"));
        assert!(joined.contains("element_center: px=(842,516)"));
        assert!(joined.contains("click_point: px=(838,519)"));
        assert!(joined.contains("patterns=[Invoke]"));
        assert!(joined.contains("runtime_id=[42,1001,4]"));

        let drag_lines = format_drag_grounding(
            HWND(0x1234 as *mut std::ffi::c_void),
            win,
            &mon,
            (200, 300),
            (600, 500),
            ScreenRect {
                x: 200,
                y: 300,
                w: 400,
                h: 200,
            },
        );
        let drag_joined = drag_lines.join("\n");
        assert!(drag_joined.contains("bbox_center: px=(400,400)"));
        assert!(drag_joined.contains("drag_start: px=(200,300)"));
        assert!(drag_joined.contains("drag_end: px=(600,500)"));
    }
}
