pub mod adapters;
pub mod fs_meta;
pub mod grounding;
pub mod process;

use crate::captures::{
    capture_target_window_bitblt, save_annotated_capture, save_red_dual_captures, RawScreenCapture,
    ScreenRect,
};
use crate::config::OutputTarget;
use crate::hooks::PenButton;
use crate::pick::adapters::office::{self, OfficeAppKind};
use crate::pick::adapters::solidworks;
use crate::pick::adapters::{collect_deep_adapter_outputs, AdapterContext};
use crate::pick::fs_meta::{format_human_bytes, inspect_path_metadata};
use crate::pick::grounding::{BboxCoords, MonitorSpace, UiaAffordance};
use crate::pick::process::inspect_process;
use crate::state::StateEvent;
use crate::window::get_hwnd_class_name;
use crate::{dispatch_state_event, AppShared};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{AppHandle, Emitter, Manager};
use uiautomation::patterns::UITextPattern;
use uiautomation::types::{
    ControlType, Handle as UiaHandle, Point as UiaPoint, TextPatternRangeEndpoint, TextUnit,
    UIProperty,
};
use uiautomation::{UIAutomation, UIElement, UITreeWalker};
use windows::Win32::Foundation::{COLORREF, HWND, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetForegroundWindow, GetLayeredWindowAttributes, GetTopWindow, GetWindow,
    GetWindowLongPtrW, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
    IsWindowVisible, WindowFromPoint, GA_ROOT, GWL_EXSTYLE, GW_HWNDNEXT,
    LAYERED_WINDOW_ATTRIBUTES_FLAGS, LWA_ALPHA, WS_EX_LAYERED, WS_EX_TRANSPARENT,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiaElementInfo {
    pub name: String,
    pub control_type: String,
    pub control_type_id: i32,
    pub automation_id: String,
    pub class_name: String,
    pub value: String,
    pub help_text: String,
    pub text_snippet: String,
    pub rect: ScreenRect,
    pub sibling_index: usize,
    pub ancestors: Vec<String>,
    pub is_root_or_empty: bool,
    #[serde(default)]
    pub affordance: Option<UiaAffordance>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HoverTargetKind {
    None,
    Element,
    Background,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HoverInspection {
    pub ok: bool,
    pub kind: HoverTargetKind,
    pub target_key: String,
    pub rect: Option<ScreenRect>,
    pub cursor: serde_json::Value,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapsuleQuoteSummary {
    pub label: String,
    pub title: String,
    pub detail: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapsulePayload {
    pub text: String,
    pub summary: CapsuleQuoteSummary,
}

pub fn clean_uia_variant_string(raw: &str) -> String {
    let trimmed = raw.trim();
    if matches!(trimmed, "" | "EMPTY" | "NULL" | "null" | "STRING()") {
        return String::new();
    }
    if let Some(inner) = trimmed
        .strip_prefix("STRING(")
        .and_then(|s| s.strip_suffix(')'))
    {
        return inner.trim().to_string();
    }
    trimmed.to_string()
}

pub fn circled_badge(n: usize) -> String {
    const CIRCLED: &[&str] = &[
        "①", "②", "③", "④", "⑤", "⑥", "⑦", "⑧", "⑨", "⑩", "⑪", "⑫", "⑬", "⑭", "⑮", "⑯", "⑰", "⑱",
        "⑲", "⑳",
    ];
    if (1..=CIRCLED.len()).contains(&n) {
        CIRCLED[n - 1].to_string()
    } else {
        format!("#{n}")
    }
}

pub fn get_window_title(hwnd: HWND) -> String {
    if hwnd.0.is_null() {
        return String::new();
    }
    unsafe {
        let mut buf = vec![0u16; 512];
        let len = GetWindowTextW(hwnd, &mut buf);
        if len <= 0 {
            String::new()
        } else {
            String::from_utf16_lossy(&buf[..len as usize])
        }
    }
}

pub fn should_skip_overlay_window(
    ex_style: isize,
    is_lwa_zero_alpha: bool,
    class_name: &str,
    is_foreground: bool,
) -> bool {
    // Any click-through overlay (NVIDIA GeForce Overlay, DeepSeek Bar Overlay, Game Bar, etc.)
    if (ex_style & (WS_EX_TRANSPARENT.0 as isize)) != 0 {
        return true;
    }
    // Invisible layered window (alpha == 0)
    if is_lwa_zero_alpha {
        return true;
    }
    // Known full-screen GPU / HUD overlay classes when passive (not foreground)
    if !is_foreground && class_name.eq_ignore_ascii_case("CEF-OSC-WIDGET") {
        return true;
    }
    false
}

unsafe fn is_window_passive_overlay(hwnd: HWND, fg_hwnd: HWND) -> bool {
    let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
    let mut zero_alpha = false;
    if (ex & (WS_EX_LAYERED.0 as isize)) != 0 {
        let mut cr_key = COLORREF(0);
        let mut alpha = 255u8;
        let mut flags = LAYERED_WINDOW_ATTRIBUTES_FLAGS(0);
        if GetLayeredWindowAttributes(hwnd, Some(&mut cr_key), Some(&mut alpha), Some(&mut flags))
            .is_ok()
            && (flags.0 & LWA_ALPHA.0) != 0
            && alpha == 0
        {
            zero_alpha = true;
        }
    }
    let cls = get_hwnd_class_name(hwnd);
    should_skip_overlay_window(ex, zero_alpha, &cls, hwnd == fg_hwnd)
}

static OVERLAY_HWND_RAW: AtomicU64 = AtomicU64::new(0);

pub fn set_overlay_hwnd(hwnd: HWND) {
    OVERLAY_HWND_RAW.store(hwnd.0 as usize as u64, Ordering::Release);
}

fn is_own_overlay_window(hwnd: HWND) -> bool {
    let raw = OVERLAY_HWND_RAW.load(Ordering::Acquire);
    if raw != 0 && hwnd.0 as usize as u64 == raw {
        return true;
    }
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    if pid == std::process::id() {
        let title = get_window_title(hwnd);
        if title == "Anything-to-Prompt Overlay" {
            return true;
        }
    }
    false
}

/// Finds the topmost interactive (non-click-through, non-cloaked) root window at `(x, y)`
/// — automatically passes through Anything-to-Prompt Overlay, NVIDIA GeForce Overlay (`CEF-OSC-WIDGET`),
/// Game Bar, and any other `WS_EX_TRANSPARENT` HUD, while allowing picks on Anything-to-Prompt itself.
pub fn resolve_root_window_at(x: i32, y: i32) -> (HWND, u32, String, String) {
    unsafe {
        let fg_hwnd = GetForegroundWindow();

        // 1. Fast path: Win32 WindowFromPoint natively skips WS_EX_TRANSPARENT / per-pixel transparent overlays
        let hit = WindowFromPoint(POINT { x, y });
        if !hit.0.is_null() {
            let root = GetAncestor(hit, GA_ROOT);
            let top = if root.0.is_null() { hit } else { root };
            let mut pid = 0u32;
            GetWindowThreadProcessId(top, Some(&mut pid));
            if pid != 0 && !is_own_overlay_window(top) && !is_window_passive_overlay(top, fg_hwnd) {
                let title = get_window_title(top);
                let class_name = get_hwnd_class_name(top);
                return (top, pid, title, class_name);
            }
        }

        // 2. Z-order fallback walk (skipping our overlay, cloaked windows, and all click-through/transparent overlays)
        let mut cur = GetTopWindow(None).unwrap_or_default();
        while !cur.0.is_null() {
            if IsWindowVisible(cur).as_bool() && !IsIconic(cur).as_bool() {
                let mut cloaked: u32 = 0;
                let _ = DwmGetWindowAttribute(
                    cur,
                    DWMWA_CLOAKED,
                    &mut cloaked as *mut _ as *mut _,
                    std::mem::size_of::<u32>() as u32,
                );
                if cloaked == 0
                    && !is_own_overlay_window(cur)
                    && !is_window_passive_overlay(cur, fg_hwnd)
                {
                    let mut rc = RECT::default();
                    if GetWindowRect(cur, &mut rc).is_ok()
                        && rc.right - rc.left > 4
                        && rc.bottom - rc.top > 4
                        && x >= rc.left
                        && x < rc.right
                        && y >= rc.top
                        && y < rc.bottom
                    {
                        let mut pid = 0u32;
                        GetWindowThreadProcessId(cur, Some(&mut pid));
                        if pid != 0 {
                            let title = get_window_title(cur);
                            let class_name = get_hwnd_class_name(cur);
                            return (cur, pid, title, class_name);
                        }
                    }
                }
            }
            cur = GetWindow(cur, GW_HWNDNEXT).unwrap_or_default();
        }
    }
    (HWND::default(), 0, String::new(), String::new())
}

fn find_target_uia_element(
    automation: &UIAutomation,
    walker: &UITreeWalker,
    root_hwnd: HWND,
    target_pid: u32,
    x: i32,
    y: i32,
) -> Option<UIElement> {
    let is_own_bar = target_pid == std::process::id();
    if let Ok(direct) = automation.element_from_point(UiaPoint::new(x, y)) {
        let el_pid = direct.get_process_id().unwrap_or(0);
        let el_name = direct.get_name().unwrap_or_default();
        let el_ct = direct.get_control_type().ok();
        let direct_is_overlay = el_name == "Anything-to-Prompt Overlay"
            || direct
                .get_native_window_handle()
                .ok()
                .map(|h| {
                    let raw: isize = h.into();
                    if raw == 0 {
                        false
                    } else {
                        let hw = HWND(raw as *mut std::ffi::c_void);
                        let root = unsafe { GetAncestor(hw, GA_ROOT) };
                        let top = if root.0.is_null() { hw } else { root };
                        is_own_overlay_window(top)
                    }
                })
                .unwrap_or(false);
        let direct_is_coarse_shell = is_own_bar
            && matches!(el_ct, Some(ControlType::Window) | Some(ControlType::Document));
        if !direct_is_overlay && !direct_is_coarse_shell {
            if el_pid == target_pid {
                return Some(direct);
            }
            // WebView2 child DOM elements inside Anything-to-Prompt (or other WebView2 apps) run in
            // msedgewebview2.exe with a child PID; accept them when their rect lies inside root_hwnd.
            let win_rc = get_window_screen_rect(root_hwnd);
            if let Ok(el_rc) = direct.get_bounding_rectangle() {
                let cx = (el_rc.get_left() + el_rc.get_right()) / 2;
                let cy = (el_rc.get_top() + el_rc.get_bottom()) / 2;
                if cx >= win_rc.x
                    && cx <= win_rc.x + win_rc.w
                    && cy >= win_rc.y
                    && cy <= win_rc.y + win_rc.h
                {
                    return Some(direct);
                }
            }
        }
    }

    // Fallback: walk down target_hwnd's UIA ControlView tree to deepest element containing (x, y)
    let root_el = automation
        .element_from_handle(UiaHandle::from(root_hwnd.0 as isize))
        .ok()?;
    Some(drill_deepest_containing(&root_el, walker, x, y, 0))
}

fn drill_deepest_containing(
    el: &UIElement,
    walker: &UITreeWalker,
    x: i32,
    y: i32,
    depth: usize,
) -> UIElement {
    if depth >= 16 {
        return el.clone();
    }
    let mut best_child: Option<(UIElement, i64)> = None;
    if let Ok(mut child) = walker.get_first_child(el) {
        let mut scanned = 0usize;
        loop {
            scanned += 1;
            if scanned > 120 {
                break;
            }
            if let Ok(r) = child.get_bounding_rectangle() {
                let w = (r.get_right() - r.get_left()) as i64;
                let h = (r.get_bottom() - r.get_top()) as i64;
                if w > 0
                    && h > 0
                    && x >= r.get_left()
                    && x <= r.get_right()
                    && y >= r.get_top()
                    && y <= r.get_bottom()
                {
                    let area = w * h;
                    let replace = best_child
                        .as_ref()
                        .map(|(_, best_area)| area <= *best_area)
                        .unwrap_or(true);
                    if replace {
                        best_child = Some((child.clone(), area));
                    }
                }
            }
            let Ok(next) = walker.get_next_sibling(&child) else {
                break;
            };
            child = next;
        }
    }

    if let Some((chosen, _)) = best_child {
        drill_deepest_containing(&chosen, walker, x, y, depth + 1)
    } else {
        el.clone()
    }
}

fn get_window_screen_rect(hwnd: HWND) -> ScreenRect {
    if hwnd.0.is_null() {
        return ScreenRect {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        };
    }
    unsafe {
        let mut rc = RECT::default();
        if GetWindowRect(hwnd, &mut rc).is_ok() {
            ScreenRect {
                x: rc.left,
                y: rc.top,
                w: (rc.right - rc.left).max(0),
                h: (rc.bottom - rc.top).max(0),
            }
        } else {
            ScreenRect {
                x: 0,
                y: 0,
                w: 0,
                h: 0,
            }
        }
    }
}

pub fn is_desktop_window_class(win_class: &str) -> bool {
    matches!(win_class, "Progman" | "WorkerW")
}

/// Returns true for Office / WPS suite and similar document, code-editor, PDF, note,
/// and creative/design canvas apps where hovering over the canvas must NEVER trigger
/// the 0.5s background dwell ring or select the entire window background.
pub fn should_ignore_background_for_app(exe_name: &str, win_class: &str) -> bool {
    let exe = exe_name.trim().to_ascii_lowercase();
    if matches!(
        exe.as_str(),
        // Microsoft Office & WPS Office
        "winword.exe"
            | "excel.exe"
            | "powerpnt.exe"
            | "wps.exe"
            | "wpsoffice.exe"
            | "et.exe"
            | "wpp.exe"
            | "wpspdf.exe"
            | "visio.exe"
            | "onenote.exe"
            | "msaccess.exe"
            | "mspub.exe"
            // PDF / Document / Markdown / Note / Text editors
            | "acrobat.exe"
            | "acrord32.exe"
            | "foxitpdfreader.exe"
            | "foxitreader.exe"
            | "sumatrapdf.exe"
            | "obsidian.exe"
            | "notion.exe"
            | "typora.exe"
            | "xmind.exe"
            | "notepad.exe"
            | "notepad++.exe"
            | "sublime_text.exe"
            // Code editors & IDEs
            | "code.exe"
            | "cursor.exe"
            | "windsurf.exe"
            | "antigravity.exe"
            | "devenv.exe"
            | "idea64.exe"
            | "pycharm64.exe"
            | "webstorm64.exe"
            | "clion64.exe"
            | "goland64.exe"
            | "rider64.exe"
            | "fleet.exe"
            // Design / Creative / Canvas / CAD / Video editors
            | "photoshop.exe"
            | "illustrator.exe"
            | "indesign.exe"
            | "afterfx.exe"
            | "premiere pro.exe"
            | "figma.exe"
            | "blender.exe"
            | "acad.exe"
            | "mspaint.exe"
            | "krita.exe"
            | "inkscape.exe"
            | "jianyingpro.exe"
            | "capcut.exe"
    ) {
        return true;
    }

    matches!(
        win_class,
        "OpusApp" | "XLMAIN" | "PPTFrameClass" | "ETMain" | "WPPMain" | "SunAwtFrame"
    )
}

/// Classifies a hovered UIA target into an immediate foreground `Element` vs a
/// dwell-gated `Background` (desktop wallpaper, root window, or genuine full-window backdrop).
pub fn classify_hover_target(
    info: &UiaElementInfo,
    win_rect: ScreenRect,
    win_class: &str,
) -> HoverTargetKind {
    // 1. Windows desktop: icons (ListItem) highlight immediately; desktop canvas requires 0.5s dwell
    if is_desktop_window_class(win_class) {
        if info.control_type == "ListItem"
            && !info.name.trim().is_empty()
            && info.rect.w > 2
            && info.rect.h > 2
        {
            return HoverTargetKind::Element;
        }
        return HoverTargetKind::Background;
    }

    // 2. Root window or full web Document canvas is always a background target
    if info.is_root_or_empty || matches!(info.control_type.as_str(), "Window" | "Document") {
        return HoverTargetKind::Background;
    }

    let valid_rect = info.rect.w > 2 && info.rect.h > 2;

    // 3. Explicit foreground content & interactive controls (including unnamed <img alt=""> avatars)
    let is_foreground_control = matches!(
        info.control_type.as_str(),
        "Image"
            | "Button"
            | "Hyperlink"
            | "Text"
            | "Edit"
            | "MenuItem"
            | "TabItem"
            | "ListItem"
            | "TreeItem"
            | "CheckBox"
            | "RadioButton"
            | "ComboBox"
            | "DataItem"
            | "HeaderItem"
            | "Slider"
            | "Spinner"
            | "Thumb"
            | "SplitButton"
            | "ToolBar"
            | "TitleBar"
            | "StatusBar"
            | "MenuBar"
            | "Tab"
    );
    if is_foreground_control && valid_rect {
        return HoverTargetKind::Element;
    }

    let has_identity = !info.name.trim().is_empty()
        || (!info.value.trim().is_empty() && info.value != "[PASSWORD_REDACTED]")
        || !info.automation_id.trim().is_empty()
        || !info.class_name.trim().is_empty();

    // 4. Container types (Pane / Group / Custom / List / Table / Tree):
    //    Only genuine huge backdrop plates (>= 55% window area, or >= 75%W x >= 60%H, or blank nameless/classless spacer)
    //    require 0.5s dwell. Regular cards/components (e.g. feed items, avatars, small cards) highlight immediately!
    let is_container_type = matches!(
        info.control_type.as_str(),
        "Pane" | "Group" | "Custom" | "List" | "Table" | "Tree"
    );
    if is_container_type {
        if !valid_rect {
            return HoverTargetKind::Background;
        }
        let el_area = (info.rect.w as i64) * (info.rect.h as i64);
        let win_w = win_rect.w.max(1) as i64;
        let win_h = win_rect.h.max(1) as i64;
        let win_area = (win_w * win_h).max(1);
        let is_huge_backdrop = (el_area * 100 >= win_area * 55)
            || ((info.rect.w as i64) * 100 >= win_w * 75
                && (info.rect.h as i64) * 100 >= win_h * 60);
        if is_huge_backdrop || !has_identity {
            return HoverTargetKind::Background;
        }
        return HoverTargetKind::Element;
    }

    if has_identity && valid_rect {
        HoverTargetKind::Element
    } else if win_rect.w > 24 && win_rect.h > 24 {
        HoverTargetKind::Background
    } else {
        HoverTargetKind::None
    }
}

fn format_background_label(
    info: Option<&UiaElementInfo>,
    win_title: &str,
    win_class: &str,
) -> String {
    if is_desktop_window_class(win_class) {
        return "桌面 · Windows 桌面".to_string();
    }
    if let Some(el) = info {
        let n = el.name.trim();
        if !n.is_empty() && n != win_title.trim() {
            return format!("背景 · {}", truncate_chars(n, 26));
        }
    }
    let wt = win_title.trim();
    if !wt.is_empty() {
        format!("窗口背景 · {}", truncate_chars(wt, 26))
    } else if let Some(el) = info {
        format!("{} 背景", el.control_type)
    } else {
        "窗口背景".to_string()
    }
}

pub fn inspect_hover_at(x: i32, y: i32, own_pid: u32) -> HoverInspection {
    let uia_pair = UIAutomation::new()
        .ok()
        .and_then(|auto| auto.get_control_view_walker().ok().map(|w| (auto, w)));
    match uia_pair {
        Some((auto, walker)) => inspect_hover_at_with_uia(Some((&auto, &walker)), x, y, own_pid),
        None => inspect_hover_at_with_uia(None, x, y, own_pid),
    }
}

pub fn inspect_hover_at_with_uia(
    uia: Option<(&UIAutomation, &UITreeWalker)>,
    x: i32,
    y: i32,
    _own_pid: u32,
) -> HoverInspection {
    let cursor = serde_json::json!({ "x": x, "y": y });
    let (root_hwnd, target_pid, win_title, win_class) = resolve_root_window_at(x, y);
    if target_pid == 0 {
        return HoverInspection {
            ok: false,
            kind: HoverTargetKind::None,
            target_key: String::new(),
            rect: None,
            cursor,
            label: String::new(),
        };
    }

    let win_rect = get_window_screen_rect(root_hwnd);
    let target_exe = office::fast_exe_basename_for_pid(target_pid);
    let ignore_bg = should_ignore_background_for_app(&target_exe, &win_class);

    // Inside Office / WPS document/grid/slide canvases, keep hover completely clean & distraction-free
    // (no giant pane frame, no 0.5s background dwell ring) so text drag / cell pick / shape click feel native.
    if let Some(hit) = office::detect_office_canvas_at(x, y) {
        if hit.in_canvas {
            return HoverInspection {
                ok: false,
                kind: HoverTargetKind::None,
                target_key: String::new(),
                rect: None,
                cursor,
                label: String::new(),
            };
        }
    }

    let uia_info = uia.and_then(|(automation, walker)| {
        let el = find_target_uia_element(automation, walker, root_hwnd, target_pid, x, y)?;
        Some(extract_uia_element_info(&el, walker, false))
    });

    if let Some(ref info) = uia_info {
        match classify_hover_target(info, win_rect, &win_class) {
            HoverTargetKind::Element => {
                let display = resolve_primary_element_label(info).unwrap_or_else(|| {
                    if !info.class_name.is_empty() {
                        truncate_chars(&info.class_name, 26)
                    } else {
                        String::new()
                    }
                });
                let label = if display.is_empty() {
                    info.control_type.clone()
                } else {
                    format!("{} · {}", info.control_type, display)
                };
                let target_key = format!(
                    "el:{}:{}:{}:{}:{}",
                    target_pid, info.control_type_id, info.rect.x, info.rect.y, info.rect.w
                );
                return HoverInspection {
                    ok: true,
                    kind: HoverTargetKind::Element,
                    target_key,
                    rect: Some(info.rect),
                    cursor,
                    label,
                };
            }
            HoverTargetKind::Background if !ignore_bg => {
                let bg_rect = if is_desktop_window_class(&win_class) {
                    if win_rect.w > 24 && win_rect.h > 24 {
                        win_rect
                    } else {
                        info.rect
                    }
                } else if info.rect.w > 24 && info.rect.h > 24 {
                    info.rect
                } else {
                    win_rect
                };
                if bg_rect.w > 24 && bg_rect.h > 24 {
                    let target_key = format!(
                        "bg:{}:{}:{}:{}:{}",
                        target_pid, bg_rect.x, bg_rect.y, bg_rect.w, bg_rect.h
                    );
                    return HoverInspection {
                        ok: true,
                        kind: HoverTargetKind::Background,
                        target_key,
                        rect: Some(bg_rect),
                        cursor,
                        label: format_background_label(Some(info), &win_title, &win_class),
                    };
                }
            }
            HoverTargetKind::Background | HoverTargetKind::None => {}
        }
    } else if !ignore_bg && win_rect.w > 24 && win_rect.h > 24 {
        let target_key = format!(
            "bg:{}:{}:{}:{}:{}",
            target_pid, win_rect.x, win_rect.y, win_rect.w, win_rect.h
        );
        return HoverInspection {
            ok: true,
            kind: HoverTargetKind::Background,
            target_key,
            rect: Some(win_rect),
            cursor,
            label: format_background_label(None, &win_title, &win_class),
        };
    }

    HoverInspection {
        ok: false,
        kind: HoverTargetKind::None,
        target_key: String::new(),
        rect: None,
        cursor,
        label: String::new(),
    }
}

pub fn is_valid_tap_element(info: &UiaElementInfo) -> bool {
    if info.is_root_or_empty {
        return false;
    }
    let has_identity = !info.name.trim().is_empty()
        || (!info.value.trim().is_empty() && info.value != "[PASSWORD_REDACTED]")
        || !info.automation_id.trim().is_empty();
    has_identity && info.rect.w > 2 && info.rect.h > 2
}

fn extract_uia_element_info(
    el: &UIElement,
    walker: &UITreeWalker,
    include_ancestors_and_text: bool,
) -> UiaElementInfo {
    let name = el.get_name().unwrap_or_default().trim().to_string();
    let ct = el.get_control_type().unwrap_or(ControlType::Custom);
    let ct_name = format!("{ct:?}");
    let ct_id = ct as i32;
    let automation_id = el
        .get_automation_id()
        .unwrap_or_default()
        .trim()
        .to_string();
    let class_name = el.get_classname().unwrap_or_default().trim().to_string();
    let help_text = el.get_help_text().unwrap_or_default().trim().to_string();

    let is_password = el
        .get_property_value(UIProperty::IsPassword)
        .ok()
        .and_then(|v| v.try_into().ok())
        .unwrap_or(false);

    let value = if is_password {
        "[PASSWORD_REDACTED]".to_string()
    } else {
        el.get_property_value(UIProperty::ValueValue)
            .ok()
            .map(|v| clean_uia_variant_string(&v.to_string()))
            .unwrap_or_default()
    };

    let rect = match el.get_bounding_rectangle() {
        Ok(r) => ScreenRect {
            x: r.get_left(),
            y: r.get_top(),
            w: (r.get_right() - r.get_left()).max(0),
            h: (r.get_bottom() - r.get_top()).max(0),
        },
        Err(_) => ScreenRect {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        },
    };

    // Check whether element is the root window itself
    let parent_opt = walker.get_parent(el).ok();
    let grand_parent_opt = parent_opt.as_ref().and_then(|p| walker.get_parent(p).ok());
    let is_top_window = ct == ControlType::Window || grand_parent_opt.is_none();

    let mut sibling_index = 0usize;
    let mut ancestors = Vec::new();
    let mut text_snippet = String::new();
    let mut affordance = None;

    if include_ancestors_and_text {
        affordance = Some(UiaAffordance::extract(el));
        // Count previous siblings (cap 64)
        let mut prev_cur = el.clone();
        for _ in 0..64 {
            if let Ok(prev) = walker.get_previous_sibling(&prev_cur) {
                sibling_index += 1;
                prev_cur = prev;
            } else {
                break;
            }
        }

        // Walk up 8 ancestor levels
        let mut cur = el.clone();
        for _ in 0..8 {
            let Ok(p) = walker.get_parent(&cur) else {
                break;
            };
            let p_ct = p.get_control_type().unwrap_or(ControlType::Custom);
            let p_name = truncate_chars(p.get_name().unwrap_or_default().trim(), 48);
            let p_aid = p.get_automation_id().unwrap_or_default();
            if p_name.is_empty() && p_aid.is_empty() && p_ct == ControlType::Pane {
                // Likely desktop root; stop climbing if its parent doesn't exist
                if walker.get_parent(&p).is_err() {
                    break;
                }
            }
            ancestors.push(format!(
                "{p_ct:?}(name=\"{p_name}\", id=\"{}\")",
                truncate_chars(p_aid.trim(), 32)
            ));
            cur = p;
        }

        if !is_password {
            if let Ok(tp) = el.get_pattern::<UITextPattern>() {
                if let Ok(dr) = tp.get_document_range() {
                    if let Ok(t) = dr.get_text(2400) {
                        text_snippet = t.trim().to_string();
                    }
                }
            }
        }
    }

    UiaElementInfo {
        name,
        control_type: ct_name,
        control_type_id: ct_id,
        automation_id,
        class_name,
        value,
        help_text,
        text_snippet,
        rect,
        sibling_index,
        ancestors,
        is_root_or_empty: is_top_window,
        affordance,
    }
}

pub fn capture_mousedown_snapshot(x: i32, y: i32) -> Option<RawScreenCapture> {
    let (root_hwnd, pid, _, _) = resolve_root_window_at(x, y);
    if pid == 0 {
        return None;
    }
    capture_target_window_bitblt(root_hwnd, x, y).ok()
}

pub fn execute_tap_pick(
    app: &AppHandle,
    x: i32,
    y: i32,
    pre_capture: Option<RawScreenCapture>,
    allow_background: bool,
) {
    let Some(shared) = app.try_state::<AppShared>() else {
        return;
    };

    // Specialized Office / WPS tap handling:
    // - Excel / WPS Spreadsheets: single-click picks a single worksheet cell
    // - PowerPoint / WPS Presentation: single-click picks a single design element (text/shape/table/picture)
    if try_execute_office_tap_pick(app, &shared, x, y, pre_capture.as_ref()) {
        return;
    }

    // Specialized SolidWorks tap handling:
    // - FeatureManager tree items -> exact feature COM readback (type, dims, parents/children)
    // - 3D graphics viewport -> ray-pick (SelectByRay) + owning feature + Zegel-SWE semantic selectors
    // - Pre-selected faces/edges/features/components/dims -> full ISelectionMgr readback into blue capsule
    if try_execute_solidworks_tap_pick(app, &shared, x, y, pre_capture.as_ref()) {
        return;
    }

    let (root_hwnd, target_pid, win_title, win_class) = resolve_root_window_at(x, y);
    if target_pid == 0 {
        return;
    }

    let win_rect = get_window_screen_rect(root_hwnd);
    let target_exe = office::fast_exe_basename_for_pid(target_pid);
    let ignore_bg = should_ignore_background_for_app(&target_exe, &win_class);

    let uia_info = UIAutomation::new().ok().and_then(|auto| {
        let walker = auto.get_control_view_walker().ok()?;
        let el = find_target_uia_element(&auto, &walker, root_hwnd, target_pid, x, y)?;
        Some(extract_uia_element_info(&el, &walker, true))
    });

    let mut kind = match uia_info.as_ref() {
        Some(info) => classify_hover_target(info, win_rect, &win_class),
        None if win_rect.w > 24 && win_rect.h > 24 => HoverTargetKind::Background,
        None => HoverTargetKind::None,
    };
    if ignore_bg && kind == HoverTargetKind::Background {
        kind = HoverTargetKind::None;
    }

    if kind == HoverTargetKind::None {
        if !ignore_bg {
            emit_pick_toast(app, "无法抓取有效信息 · 仅可涂鸦");
        }
        return;
    }

    if kind == HoverTargetKind::Background && !allow_background {
        log::info!(
            "Tap pick on background ignored before 0.5s dwell at ({x},{y}) in '{}' ({})",
            win_title,
            win_class
        );
        emit_pick_toast(app, "在背景停留 0.5 秒后可点选 · 或拖拽涂鸦");
        return;
    }

    let is_bg = kind == HoverTargetKind::Background;
    let el_info = match uia_info {
        Some(mut info) => {
            if is_bg {
                if is_desktop_window_class(&win_class) {
                    if win_rect.w > 24 && win_rect.h > 24 {
                        info.rect = win_rect;
                    }
                    if info.name.is_empty() {
                        info.name = "Windows 桌面".to_string();
                    }
                } else if info.rect.w <= 24 || info.rect.h <= 24 {
                    info.rect = win_rect;
                }
                if info.name.is_empty() && !win_title.trim().is_empty() {
                    info.name = win_title.trim().to_string();
                }
            }
            info
        }
        None => UiaElementInfo {
            name: if is_desktop_window_class(&win_class) {
                "Windows 桌面".to_string()
            } else if !win_title.trim().is_empty() {
                win_title.trim().to_string()
            } else {
                "窗口背景".to_string()
            },
            control_type: "Window".to_string(),
            control_type_id: ControlType::Window as i32,
            automation_id: String::new(),
            class_name: win_class.clone(),
            value: String::new(),
            help_text: String::new(),
            text_snippet: String::new(),
            rect: win_rect,
            sibling_index: 0,
            ancestors: Vec::new(),
            is_root_or_empty: true,
            affordance: None,
        },
    };

    let cfg = shared.config.lock().map(|c| c.clone()).unwrap_or_default();
    let n = shared.allocate_pick_number();

    if let Some(ov) = app.get_webview_window("overlay") {
        let _ = ov.emit(
            "pen:picked",
            serde_json::json!({
                "n": n,
                "rect": el_info.rect,
            }),
        );
    }

    let raw_cap = pre_capture.or_else(|| capture_target_window_bitblt(root_hwnd, x, y).ok());
    let screenshot_path = raw_cap.as_ref().and_then(|raw| {
        save_annotated_capture(app, raw, Some(el_info.rect), &[], n, &cfg)
            .ok()
            .map(|p| p.to_string_lossy().to_string())
    });

    let proc_meta = inspect_process(target_pid, &cfg.app_source_map);
    let adapter_outputs = collect_deep_adapter_outputs(
        AdapterContext {
            hwnd: root_hwnd.0 as isize,
            window_title: win_title.clone(),
            window_class: win_class.clone(),
            proc_meta: proc_meta.clone(),
            primary_element: Some(el_info.clone()),
            tap_point: Some((x, y)),
            broker: shared.broker.clone(),
        },
        1500,
    );

    let mon = MonitorSpace::for_point(x, y, root_hwnd);
    let grounding_lines = grounding::format_tap_grounding(
        root_hwnd,
        win_rect,
        &mon,
        (x, y),
        el_info.rect,
        &el_info.control_type,
        el_info.affordance.as_ref(),
    );

    let pick_kind = if is_bg { "background" } else { "tap" };
    let capsule = build_capsule_payload(
        n,
        pick_kind,
        &win_title,
        &win_class,
        &proc_meta,
        Some(&el_info),
        &[],
        &adapter_outputs,
        &grounding_lines,
        Some((&mon, win_rect)),
        screenshot_path.as_deref(),
    );

    log::info!(
        "Committed Tap Pick #{n}: title='{}' detail='{}' kind={} screenshot={:?}",
        capsule.summary.title,
        capsule.summary.detail,
        pick_kind,
        screenshot_path
    );
    inject_capsule_into_bar(app, &capsule);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlueGestureKind {
    /// Monotonic diagonal drag (e.g. top-left to bottom-right) -> CAD-style Box Select
    Box,
    /// Curved, looping, or irregular freehand stroke -> Freehand Lasso Select
    Lasso,
}

/// CAD-style automatic Box vs Lasso trajectory classifier.
/// Straight/monotonic diagonal drags classify as `Box`; curving, hooking, reversing,
/// or looping trajectories seamlessly switch to `Lasso`.
pub fn classify_blue_gesture(points: &[(i32, i32)]) -> BlueGestureKind {
    if points.len() < 3 {
        return BlueGestureKind::Box;
    }
    let start = points[0];
    let end = *points.last().unwrap_or(&start);
    let dx_total = (end.0 - start.0) as f64;
    let dy_total = (end.1 - start.1) as f64;
    let chord = dx_total.hypot(dy_total);

    let mut arc_len = 0.0_f64;
    let (mut min_x, mut max_x) = (start.0, start.0);
    let (mut min_y, mut max_y) = (start.1, start.1);
    for w in points.windows(2) {
        let seg_dx = (w[1].0 - w[0].0) as f64;
        let seg_dy = (w[1].1 - w[0].1) as f64;
        arc_len += seg_dx.hypot(seg_dy);
        min_x = min_x.min(w[1].0);
        max_x = max_x.max(w[1].0);
        min_y = min_y.min(w[1].1);
        max_y = max_y.max(w[1].1);
    }

    if arc_len < 36.0 {
        return BlueGestureKind::Box;
    }
    if chord < 24.0 {
        // Closed loop returning near start point
        return BlueGestureKind::Lasso;
    }

    let overshoot_x_pos = max_x - start.0.max(end.0);
    let overshoot_x_neg = start.0.min(end.0) - min_x;
    let overshoot_y_pos = max_y - start.1.max(end.1);
    let overshoot_y_neg = start.1.min(end.1) - min_y;
    let max_overshoot = overshoot_x_pos
        .max(overshoot_x_neg)
        .max(overshoot_y_pos)
        .max(overshoot_y_neg);
    if max_overshoot > 26 {
        return BlueGestureKind::Lasso;
    }

    let straightness = chord / arc_len.max(1.0);
    if straightness < 0.80 {
        return BlueGestureKind::Lasso;
    }

    let mut max_perp_dev = 0.0_f64;
    for &(px, py) in &points[1..points.len() - 1] {
        let cross = ((px - start.0) as f64 * dy_total - (py - start.1) as f64 * dx_total).abs();
        let perp = cross / chord;
        if perp > max_perp_dev {
            max_perp_dev = perp;
        }
    }
    if max_perp_dev > 34.0 && (max_perp_dev / chord) > 0.24 {
        return BlueGestureKind::Lasso;
    }

    BlueGestureKind::Box
}

pub fn point_in_polygon(pt: (i32, i32), poly: &[(i32, i32)]) -> bool {
    if poly.len() < 3 {
        return false;
    }
    let (px, py) = (pt.0 as f64, pt.1 as f64);
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (xi, yi) = (poly[i].0 as f64, poly[i].1 as f64);
        let (xj, yj) = (poly[j].0 as f64, poly[j].1 as f64);
        let intersects = ((yi > py) != (yj > py))
            && (px < (xj - xi) * (py - yi) / ((yj - yi) + 1e-12) + xi);
        if intersects {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn rect_contains_pt(r: ScreenRect, pt: (i32, i32)) -> bool {
    pt.0 >= r.x && pt.0 <= r.x + r.w && pt.1 >= r.y && pt.1 <= r.y + r.h
}

pub fn rects_intersect(a: ScreenRect, b: ScreenRect) -> bool {
    a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y
}

fn segments_intersect(a: (i32, i32), b: (i32, i32), c: (i32, i32), d: (i32, i32)) -> bool {
    fn orient(p: (i32, i32), q: (i32, i32), r: (i32, i32)) -> i64 {
        (q.0 - p.0) as i64 * (r.1 - p.1) as i64 - (q.1 - p.1) as i64 * (r.0 - p.0) as i64
    }
    fn on_seg(p: (i32, i32), q: (i32, i32), r: (i32, i32)) -> bool {
        q.0 >= p.0.min(r.0) && q.0 <= p.0.max(r.0) && q.1 >= p.1.min(r.1) && q.1 <= p.1.max(r.1)
    }
    let o1 = orient(a, b, c);
    let o2 = orient(a, b, d);
    let o3 = orient(c, d, a);
    let o4 = orient(c, d, b);

    if (o1 > 0 && o2 < 0 || o1 < 0 && o2 > 0) && (o3 > 0 && o4 < 0 || o3 < 0 && o4 > 0) {
        return true;
    }
    (o1 == 0 && on_seg(a, c, b))
        || (o2 == 0 && on_seg(a, d, b))
        || (o3 == 0 && on_seg(c, a, d))
        || (o4 == 0 && on_seg(c, b, d))
}

pub fn segment_intersects_rect(a: (i32, i32), b: (i32, i32), r: ScreenRect) -> bool {
    if rect_contains_pt(r, a) || rect_contains_pt(r, b) {
        return true;
    }
    let seg_min_x = a.0.min(b.0);
    let seg_max_x = a.0.max(b.0);
    let seg_min_y = a.1.min(b.1);
    let seg_max_y = a.1.max(b.1);
    if seg_max_x < r.x || seg_min_x > r.x + r.w || seg_max_y < r.y || seg_min_y > r.y + r.h {
        return false;
    }
    let tl = (r.x, r.y);
    let tr = (r.x + r.w, r.y);
    let br = (r.x + r.w, r.y + r.h);
    let bl = (r.x, r.y + r.h);
    segments_intersect(a, b, tl, tr)
        || segments_intersect(a, b, tr, br)
        || segments_intersect(a, b, br, bl)
        || segments_intersect(a, b, bl, tl)
}

/// True if the freehand blue polyline touches/crosses `r` OR encloses `r` (center or any corner inside polygon).
pub fn polyline_touches_or_encloses_rect(points: &[(i32, i32)], r: ScreenRect) -> bool {
    if points.is_empty() || r.w <= 0 || r.h <= 0 {
        return false;
    }
    for w in points.windows(2) {
        if segment_intersects_rect(w[0], w[1], r) {
            return true;
        }
    }
    if points.len() >= 3 {
        let cx = r.x + r.w / 2;
        let cy = r.y + r.h / 2;
        let probe_pts = [
            (cx, cy),
            (r.x + 2, r.y + 2),
            (r.x + r.w - 2, r.y + 2),
            (r.x + r.w - 2, r.y + r.h - 2),
            (r.x + 2, r.y + r.h - 2),
        ];
        if probe_pts.iter().any(|&p| point_in_polygon(p, points)) {
            return true;
        }
    }
    false
}

fn is_strict_foreground_for_blue(
    info: &UiaElementInfo,
    win_rect: ScreenRect,
    win_class: &str,
) -> bool {
    if !is_valid_tap_element(info) {
        return false;
    }
    // Desktop: ONLY allow actual desktop icons (ListItem), never wallpaper/shell list/pane
    if is_desktop_window_class(win_class) {
        return info.control_type == "ListItem";
    }
    if classify_hover_target(info, win_rect, win_class) != HoverTargetKind::Element {
        return false;
    }
    // Never treat giant window-filling container panels as selected foreground items
    let win_area = (win_rect.w.max(1) as i64) * (win_rect.h.max(1) as i64);
    let el_area = (info.rect.w.max(0) as i64) * (info.rect.h.max(0) as i64);
    let is_container = matches!(
        info.control_type.as_str(),
        "Pane" | "Group" | "Window" | "Document" | "Custom" | "List" | "Table" | "Tree"
    );
    if is_container && el_area * 100 >= win_area * 55 {
        return false;
    }
    true
}

struct BlueTextRangeHit {
    text: String,
    start_pt: (i32, i32),
    end_pt: (i32, i32),
    bbox: ScreenRect,
    container_info: UiaElementInfo,
}

fn point_distance_to_rect(pt: (i32, i32), r: ScreenRect) -> i32 {
    let dx = if pt.0 < r.x {
        r.x - pt.0
    } else if pt.0 > r.x + r.w {
        pt.0 - (r.x + r.w)
    } else {
        0
    };
    let dy = if pt.1 < r.y {
        r.y - pt.1
    } else if pt.1 > r.y + r.h {
        pt.1 - (r.y + r.h)
    } else {
        0
    };
    ((dx as f64).hypot(dy as f64)) as i32
}

fn find_text_pattern_ancestor(
    el: &UIElement,
    walker: &UITreeWalker,
) -> Option<(UIElement, UITextPattern)> {
    let mut cur = el.clone();
    for _ in 0..5 {
        if let Ok(tp) = cur.get_pattern::<UITextPattern>() {
            return Some((cur, tp));
        }
        match walker.get_parent(&cur) {
            Ok(p) => cur = p,
            Err(_) => break,
        }
    }
    None
}

fn range_char_distance_to_pt(
    tp: &UITextPattern,
    pt: (i32, i32),
) -> Option<i32> {
    let probe = tp.get_range_from_point(UiaPoint::new(pt.0, pt.1)).ok()?;
    let enclosing = probe.get_enclosing_element().ok()?;
    let r = enclosing.get_bounding_rectangle().ok()?;
    let sr = ScreenRect {
        x: r.get_left(),
        y: r.get_top(),
        w: (r.get_right() - r.get_left()).max(1),
        h: (r.get_bottom() - r.get_top()).max(1),
    };
    Some(point_distance_to_rect(pt, sr))
}

fn try_extract_win32_edit_char_range(
    el: &UIElement,
    tp: Option<&UITextPattern>,
    start_pt: (i32, i32),
    end_pt: (i32, i32),
) -> Option<String> {
    use uiautomation::patterns::UIValuePattern;
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::Graphics::Gdi::ScreenToClient;
    use windows::Win32::UI::WindowsAndMessaging::SendMessageW;

    const EM_CHARFROMPOS: u32 = 0x00D7;
    // Standard Win32 Edit / WinForms TextBox passes packed MAKELPARAM(x, y) by value (safe cross-process).
    // RichEdit expects a pointer in LPARAM which is NOT marshaled cross-process, so skip it here.
    let class_lower = el
        .get_classname()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if class_lower.contains("richedit") || !class_lower.contains("edit") {
        return None;
    }
    let raw_hwnd: isize = el.get_native_window_handle().ok()?.into();
    if raw_hwnd == 0 {
        return None;
    }
    let hwnd = HWND(raw_hwnd as *mut std::ffi::c_void);
    let mut c1 = POINT {
        x: start_pt.0,
        y: start_pt.1,
    };
    let mut c2 = POINT {
        x: end_pt.0,
        y: end_pt.1,
    };
    unsafe {
        if !ScreenToClient(hwnd, &mut c1).as_bool() || !ScreenToClient(hwnd, &mut c2).as_bool() {
            return None;
        }
    }

    let char_index_at = |pt: &POINT| -> usize {
        let lparam = ((pt.x & 0xFFFF) | ((pt.y & 0xFFFF) << 16)) as isize;
        let res = unsafe {
            SendMessageW(
                hwnd,
                EM_CHARFROMPOS,
                Some(WPARAM(0)),
                Some(LPARAM(lparam)),
            )
        };
        (res.0 as usize) & 0xFFFF
    };

    let idx1 = char_index_at(&c1);
    let idx2 = char_index_at(&c2);
    let lo = idx1.min(idx2);
    let hi = idx1.max(idx2);
    if hi <= lo {
        return None;
    }

    let full = tp
        .and_then(|p| p.get_document_range().ok())
        .and_then(|d| d.get_text(32_000).ok())
        .or_else(|| {
            el.get_pattern::<UIValuePattern>()
                .ok()
                .and_then(|vp| vp.get_value().ok())
        })?;
    let utf16: Vec<u16> = full.encode_utf16().collect();
    if lo >= utf16.len() {
        return None;
    }
    let end_excl = (hi + 1).min(utf16.len());
    let sliced = String::from_utf16_lossy(&utf16[lo..end_excl])
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .trim()
        .to_string();
    if sliced.chars().count() >= 2 {
        Some(sliced)
    } else {
        None
    }
}

/// Optimized diagonal/horizontal blue text sweep:
/// 1) Uses UIA `UITextPattern::get_range_from_point(start..end)` to capture exact characters
///    from the first character touched to the last character touched across multiple lines.
/// 2) Falls back to reading-order sibling `Text` sweep when both endpoints hit sibling text rows.
fn try_extract_blue_text_range(
    points: &[(i32, i32)],
    bbox: ScreenRect,
    root_hwnd: HWND,
    expected_pid: u32,
    win_class: &str,
) -> Option<BlueTextRangeHit> {
    if points.len() < 2 || is_desktop_window_class(win_class) {
        return None;
    }
    let start_pt = points[0];
    let end_pt = *points.last()?;
    let chord = ((end_pt.0 - start_pt.0) as f64).hypot((end_pt.1 - start_pt.1) as f64);
    if chord < 18.0 {
        return None;
    }
    // Skip closed lasso loops around icons/controls; diagonal/horizontal text sweeps are open strokes
    let mut arc_len = 0.0_f64;
    for w in points.windows(2) {
        arc_len += ((w[1].0 - w[0].0) as f64).hypot((w[1].1 - w[0].1) as f64);
    }
    if chord / arc_len.max(1.0) < 0.65 {
        return None;
    }

    let auto = UIAutomation::new().ok()?;
    let walker = auto.get_control_view_walker().ok()?;
    let el_start = find_target_uia_element(&auto, &walker, root_hwnd, expected_pid, start_pt.0, start_pt.1)?;
    let el_end = find_target_uia_element(&auto, &walker, root_hwnd, expected_pid, end_pt.0, end_pt.1)?;

    // Reject if user clearly started/ended on an interactive non-text widget (button, checkbox, icon, etc.)
    let start_ct = format!("{:?}", el_start.get_control_type().unwrap_or(ControlType::Custom));
    let end_ct = format!("{:?}", el_end.get_control_type().unwrap_or(ControlType::Custom));
    let is_widget = |ct: &str| {
        matches!(
            ct,
            "Button"
                | "CheckBox"
                | "RadioButton"
                | "ComboBox"
                | "MenuItem"
                | "TabItem"
                | "ListItem"
                | "TreeItem"
                | "Slider"
                | "ScrollBar"
                | "Image"
        )
    };
    if is_widget(&start_ct) || is_widget(&end_ct) {
        return None;
    }

    // Tier 1: Exact character-level UIA TextPattern RangeFromPoint
    if let Some((tp_el, tp)) = find_text_pattern_ancestor(&el_start, &walker) {
        let start_near = range_char_distance_to_pt(&tp, start_pt)
            .map(|d| d <= 52)
            .unwrap_or(true);
        let end_near = range_char_distance_to_pt(&tp, end_pt)
            .map(|d| d <= 52)
            .unwrap_or(true);

        if start_near && end_near {
            let r_start_res = tp.get_range_from_point(UiaPoint::new(start_pt.0, start_pt.1));
            let r_end_res = tp.get_range_from_point(UiaPoint::new(end_pt.0, end_pt.1));
            match (r_start_res, r_end_res) {
                (Ok(r_start), Ok(r_end)) => {
                    let cmp = r_start
                        .compare_endpoints(
                            TextPatternRangeEndpoint::Start,
                            &r_end,
                            TextPatternRangeEndpoint::Start,
                        )
                        .unwrap_or(0);
                    let (first_r, last_r) = if cmp <= 0 {
                        (r_start, r_end)
                    } else {
                        (r_end, r_start)
                    };

                    let _ = first_r.move_endpoint_by_range(
                        TextPatternRangeEndpoint::End,
                        &last_r,
                        TextPatternRangeEndpoint::End,
                    );
                    // Include the final character under the end cursor tip
                    let _ = first_r.move_endpoint_by_unit(
                        TextPatternRangeEndpoint::End,
                        TextUnit::Character,
                        1,
                    );
                    if let Ok(raw_text) = first_r.get_text(8000) {
                        let cleaned = raw_text.replace("\r\n", "\n").trim().to_string();
                        if cleaned.chars().count() >= 2 {
                            let container_info = extract_uia_element_info(&tp_el, &walker, false);
                            return Some(BlueTextRangeHit {
                                text: cleaned,
                                start_pt,
                                end_pt,
                                bbox,
                                container_info,
                            });
                        }
                    }
                }
                (e1, e2) => {
                    log::debug!(
                        "[BLUE-TEXT] RangeFromPoint failed: start={:?} end={:?}",
                        e1.err(),
                        e2.err()
                    );
                }
            }
        }

        // Tier 1b (inside TextPattern provider): Win32 Edit fallback
        if let Some(text) = try_extract_win32_edit_char_range(&tp_el, Some(&tp), start_pt, end_pt) {
            let container_info = extract_uia_element_info(&tp_el, &walker, false);
            return Some(BlueTextRangeHit {
                text,
                start_pt,
                end_pt,
                bbox,
                container_info,
            });
        }
    }

    // Tier 1c: Classic Win32 Edit / WinForms TextBox (exposes ValuePattern instead of TextPattern)
    if let Some(text) = try_extract_win32_edit_char_range(&el_start, None, start_pt, end_pt) {
        let container_info = extract_uia_element_info(&el_start, &walker, false);
        return Some(BlueTextRangeHit {
            text,
            start_pt,
            end_pt,
            bbox,
            container_info,
        });
    }

    // Tier 2: Multi-line sibling `Text` sweep (for WinForms/WPF/Qt/chat UIs without TextPattern)
    if start_ct == "Text" && end_ct == "Text" {
        if let Ok(parent) = walker.get_parent(&el_start) {
            let y_min = start_pt.1.min(end_pt.1) - 6;
            let y_max = start_pt.1.max(end_pt.1) + 6;
            let mut rows: Vec<(i32, i32, String)> = Vec::new();
            if let Ok(mut child) = walker.get_first_child(&parent) {
                for _ in 0..100 {
                    let ct = format!("{:?}", child.get_control_type().unwrap_or(ControlType::Custom));
                    if ct == "Text" {
                        if let Ok(rc) = child.get_bounding_rectangle() {
                            let cy = (rc.get_top() + rc.get_bottom()) / 2;
                            if cy >= y_min && cy <= y_max {
                                let name = child.get_name().unwrap_or_default().trim().to_string();
                                if !name.is_empty() {
                                    rows.push((rc.get_top(), rc.get_left(), name));
                                }
                            }
                        }
                    }
                    match walker.get_next_sibling(&child) {
                        Ok(next) => child = next,
                        Err(_) => break,
                    }
                }
            }
            if rows.len() >= 2 {
                rows.sort_by_key(|(y, x, _)| (*y / 14, *x));
                let joined = rows
                    .into_iter()
                    .map(|(_, _, s)| s)
                    .collect::<Vec<_>>()
                    .join("\n");
                if joined.chars().count() >= 2 {
                    let container_info = extract_uia_element_info(&el_start, &walker, false);
                    return Some(BlueTextRangeHit {
                        text: joined,
                        start_pt,
                        end_pt,
                        bbox,
                        container_info,
                    });
                }
            }
        }
    }

    None
}

fn sample_blue_foreground_elements(
    points: &[(i32, i32)],
    bbox: ScreenRect,
    gesture: BlueGestureKind,
    expected_pid: u32,
    cap: usize,
) -> Vec<UiaElementInfo> {
    let mut out: Vec<UiaElementInfo> = Vec::new();
    let Ok(auto) = UIAutomation::new() else {
        return out;
    };
    let Ok(walker) = auto.get_control_view_walker() else {
        return out;
    };

    let mut probe_points: Vec<(i32, i32)> = Vec::with_capacity(64);
    match gesture {
        BlueGestureKind::Box => {
            let cols = 6usize;
            let rows = 6usize;
            for r in 0..rows {
                for c in 0..cols {
                    let px = bbox.x + ((bbox.w as usize) * (2 * c + 1) / (2 * cols)) as i32;
                    let py = bbox.y + ((bbox.h as usize) * (2 * r + 1) / (2 * rows)) as i32;
                    probe_points.push((px, py));
                }
            }
            probe_points.push((bbox.x + 4, bbox.y + 4));
            probe_points.push((bbox.x + bbox.w - 4, bbox.y + bbox.h - 4));
        }
        BlueGestureKind::Lasso => {
            // 1. Sample along the blue stroke polyline (every ~20px, up to 34 pts) -> elements TOUCHED
            let mut accum = 0.0_f64;
            let step = 20.0_f64;
            if let Some(&first) = points.first() {
                probe_points.push(first);
            }
            for w in points.windows(2) {
                let d = ((w[1].0 - w[0].0) as f64).hypot((w[1].1 - w[0].1) as f64);
                accum += d;
                if accum >= step {
                    probe_points.push(w[1]);
                    accum = 0.0;
                    if probe_points.len() >= 34 {
                        break;
                    }
                }
            }
            if let Some(&last) = points.last() {
                probe_points.push(last);
            }

            // 2. Sample interior points enclosed inside the blue polygon -> elements ENCLOSED
            let grid = 8usize;
            let mut interior_added = 0usize;
            for r in 0..grid {
                for c in 0..grid {
                    if interior_added >= 24 {
                        break;
                    }
                    let px = bbox.x + ((bbox.w as usize) * (2 * c + 1) / (2 * grid)) as i32;
                    let py = bbox.y + ((bbox.h as usize) * (2 * r + 1) / (2 * grid)) as i32;
                    if point_in_polygon((px, py), points) {
                        probe_points.push((px, py));
                        interior_added += 1;
                    }
                }
            }
        }
    }

    let mut seen_els: HashSet<String> = HashSet::new();
    let mut swept_parents: HashSet<String> = HashSet::new();
    let started = std::time::Instant::now();

    let geom_matches = |rect: ScreenRect| -> bool {
        match gesture {
            BlueGestureKind::Box => rects_intersect(bbox, rect),
            BlueGestureKind::Lasso => polyline_touches_or_encloses_rect(points, rect),
        }
    };

    for (px, py) in probe_points {
        if out.len() >= cap || started.elapsed().as_millis() > 480 {
            break;
        }
        let (rhwnd, pid, _, wclass) = resolve_root_window_at(px, py);
        if pid == 0 || (expected_pid != 0 && pid != expected_pid && !is_desktop_window_class(&wclass)) {
            continue;
        }
        let win_rect = get_window_screen_rect(rhwnd);
        let Some(el) = find_target_uia_element(&auto, &walker, rhwnd, pid, px, py) else {
            continue;
        };
        let info = extract_uia_element_info(&el, &walker, false);
        if is_strict_foreground_for_blue(&info, win_rect, &wclass) && geom_matches(info.rect) {
            let key = format!(
                "{}:{}:{}:{}:{}",
                info.control_type_id, info.rect.x, info.rect.y, info.name, info.automation_id
            );
            if seen_els.insert(key) {
                out.push(info.clone());
            }

            // Sibling container sweep: catches 100% of touched/enclosed ListItems, icons, buttons, tabs, etc.
            let is_sweepable = matches!(
                info.control_type.as_str(),
                "ListItem"
                    | "TreeItem"
                    | "DataItem"
                    | "TabItem"
                    | "Button"
                    | "CheckBox"
                    | "RadioButton"
                    | "MenuItem"
                    | "Hyperlink"
                    | "Text"
                    | "Edit"
            );
            if is_sweepable {
                if let Ok(parent) = walker.get_parent(&el) {
                    if let Ok(prc) = parent.get_bounding_rectangle() {
                        let pkey = format!(
                            "{}:{}:{}:{}",
                            pid,
                            prc.get_left(),
                            prc.get_top(),
                            prc.get_right() - prc.get_left()
                        );
                        if swept_parents.insert(pkey) {
                            if let Ok(mut child) = walker.get_first_child(&parent) {
                                for _ in 0..120 {
                                    if out.len() >= cap || started.elapsed().as_millis() > 480 {
                                        break;
                                    }
                                    if let Ok(crc) = child.get_bounding_rectangle() {
                                        let sr = ScreenRect {
                                            x: crc.get_left(),
                                            y: crc.get_top(),
                                            w: (crc.get_right() - crc.get_left()).max(0),
                                            h: (crc.get_bottom() - crc.get_top()).max(0),
                                        };
                                        if sr.w > 2 && sr.h > 2 && geom_matches(sr) {
                                            let cinfo = extract_uia_element_info(&child, &walker, false);
                                            if is_strict_foreground_for_blue(&cinfo, win_rect, &wclass) {
                                                let ckey = format!(
                                                    "{}:{}:{}:{}:{}",
                                                    cinfo.control_type_id,
                                                    cinfo.rect.x,
                                                    cinfo.rect.y,
                                                    cinfo.name,
                                                    cinfo.automation_id
                                                );
                                                if seen_els.insert(ckey) {
                                                    out.push(cinfo);
                                                }
                                            }
                                        }
                                    }
                                    match walker.get_next_sibling(&child) {
                                        Ok(next) => child = next,
                                        Err(_) => break,
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Natural top-to-bottom, left-to-right reading order
    out.sort_by_key(|e| (e.rect.y / 18, e.rect.x));
    out.truncate(cap);
    out
}

pub fn execute_stroke_pick(
    app: &AppHandle,
    points: Vec<(i32, i32)>,
    button: PenButton,
    pre_capture: Option<RawScreenCapture>,
) {
    if points.len() < 2 {
        return;
    }
    let Some(shared) = app.try_state::<AppShared>() else {
        return;
    };

    if button == PenButton::Right {
        execute_red_screenshot_pick(app, &shared, points, pre_capture);
    } else {
        execute_blue_selection_pick(app, &shared, points, pre_capture);
    }
}

/// Right-button RED freehand doodle -> always produces an annotated screenshot capsule (`红线截图`).
fn execute_red_screenshot_pick(
    app: &AppHandle,
    shared: &AppShared,
    points: Vec<(i32, i32)>,
    pre_capture: Option<RawScreenCapture>,
) {
    let (mut min_x, mut min_y) = points[0];
    let (mut max_x, mut max_y) = points[0];
    for &(px, py) in &points[1..] {
        min_x = min_x.min(px);
        min_y = min_y.min(py);
        max_x = max_x.max(px);
        max_y = max_y.max(py);
    }
    let bbox = ScreenRect {
        x: min_x,
        y: min_y,
        w: (max_x - min_x).max(24),
        h: (max_y - min_y).max(24),
    };
    let center_x = bbox.x + bbox.w / 2;
    let center_y = bbox.y + bbox.h / 2;

    let (root_hwnd, target_pid, win_title, win_class) = resolve_root_window_at(center_x, center_y);
    if target_pid == 0 {
        if let Some(ov) = app.get_webview_window("overlay") {
            let _ = ov.emit("pen:clear_stroke", serde_json::json!({}));
        }
        return;
    }

    let cfg = shared.config.lock().map(|c| c.clone()).unwrap_or_default();
    let n = shared.allocate_pick_number();
    let badge = circled_badge(n);

    if let Some(ov) = app.get_webview_window("overlay") {
        let pts_json: Vec<serde_json::Value> = points
            .iter()
            .map(|&(px, py)| serde_json::json!({ "x": px, "y": py }))
            .collect();
        let _ = ov.emit(
            "pen:picked",
            serde_json::json!({
                "n": n,
                "mode": "red_doodle",
                "color": "red",
                "strokes": pts_json,
                "rect": bbox,
            }),
        );
    }

    let raw_cap =
        pre_capture.or_else(|| capture_target_window_bitblt(root_hwnd, center_x, center_y).ok());
    let (annotated_screenshot_path, clean_screenshot_path) = match raw_cap.as_ref() {
        Some(raw) => {
            let (annot_p, clean_p) =
                save_red_dual_captures(app, raw, Some(bbox), &points, n, &cfg);
            (
                annot_p.map(|p| p.to_string_lossy().to_string()),
                clean_p.map(|p| p.to_string_lossy().to_string()),
            )
        }
        None => (None, None),
    };

    let proc_meta = inspect_process(target_pid, &cfg.app_source_map);
    let app_display = if !proc_meta.product_name.is_empty() {
        proc_meta.product_name.as_str()
    } else if !proc_meta.exe_name.is_empty() {
        proc_meta.exe_name.as_str()
    } else {
        "Screen"
    };

    let win_rect = get_window_screen_rect(root_hwnd);
    let mon = MonitorSpace::for_point(center_x, center_y, root_hwnd);
    let start_pt = points[0];
    let end_pt = *points.last().unwrap_or(&start_pt);
    let grounding_lines =
        grounding::format_drag_grounding(root_hwnd, win_rect, &mon, start_pt, end_pt, bbox);

    let mut lines = Vec::new();
    lines.push(format!(
        "[DSB-PICK #{n} badge={badge} kind=red_screenshot app=\"{}\"]",
        proc_meta.exe_name
    ));
    if let Some(ref sp) = annotated_screenshot_path {
        lines.push(format!("screenshot_annotated_path: {sp}"));
    }
    if let Some(ref cp) = clean_screenshot_path {
        lines.push(format!("screenshot_clean_path: {cp}"));
    }
    if annotated_screenshot_path.is_some() || clean_screenshot_path.is_some() {
        lines.push(
            "instruction: 本胶囊包含两张对应同一画面的截图：screenshot_annotated_path 带用户红线涂鸦用于定位关注区域；screenshot_clean_path 为无红线遮挡的原始高清参考图，请结合两张图一起阅读，避免红线遮挡小字或图标细节。"
                .to_string(),
        );
    }
    lines.push(format!("window_title: {}", win_title.trim()));
    lines.push(format!("window_class: {win_class}"));
    lines.push(format!(
        "process: pid={} exe=\"{}\" framework=\"{}\"",
        proc_meta.pid, proc_meta.exe_path, proc_meta.framework_guess
    ));
    lines.extend(grounding_lines);
    lines.push("[/DSB-PICK]".to_string());

    let capsule = CapsulePayload {
        summary: CapsuleQuoteSummary {
            label: badge.clone(),
            title: format!("{badge} 红线截图"),
            detail: format!("{app_display} · {}×{} 双图涂鸦截图", bbox.w, bbox.h),
            body: format!(
                "红线标注图 + 无遮挡原图参考 — {}",
                truncate_chars(&win_title, 88)
            ),
        },
        text: lines.join("\n"),
    };

    log::info!(
        "Committed Stroke Pick #{n}: mode=red_screenshot bbox=({},{},{}x{}) sampled=0 screenshot={:?} clean_screenshot={:?}",
        bbox.x,
        bbox.y,
        bbox.w,
        bbox.h,
        annotated_screenshot_path,
        clean_screenshot_path
    );
    inject_capsule_into_bar(app, &capsule);
}

/// Left-button BLUE drag:
/// 1) Word / Excel smart selection if inside Office canvas
/// 2) Diagonal/horizontal text range extraction (UIA TextPattern RangeFromPoint)
/// 3) CAD-style Auto-Box / Auto-Lasso foreground element group (`元素组`, strictly excluding background)
fn execute_blue_selection_pick(
    app: &AppHandle,
    shared: &AppShared,
    points: Vec<(i32, i32)>,
    pre_capture: Option<RawScreenCapture>,
) {
    // Keep Word text drag & Excel cell range box on Left drag inside Office
    if try_execute_office_drag_pick(app, shared, &points, pre_capture.as_ref()) {
        return;
    }

    let (mut min_x, mut min_y) = points[0];
    let (mut max_x, mut max_y) = points[0];
    for &(px, py) in &points[1..] {
        min_x = min_x.min(px);
        min_y = min_y.min(py);
        max_x = max_x.max(px);
        max_y = max_y.max(py);
    }
    let bbox = ScreenRect {
        x: min_x,
        y: min_y,
        w: (max_x - min_x).max(12),
        h: (max_y - min_y).max(12),
    };
    let gesture = classify_blue_gesture(&points);
    let start_pt = points[0];
    let end_pt = *points.last().unwrap_or(&start_pt);
    let center_x = bbox.x + bbox.w / 2;
    let center_y = bbox.y + bbox.h / 2;

    let (root_hwnd, target_pid, win_title, win_class) = resolve_root_window_at(start_pt.0, start_pt.1);
    if target_pid == 0 {
        if let Some(ov) = app.get_webview_window("overlay") {
            let _ = ov.emit("pen:clear_stroke", serde_json::json!({}));
        }
        return;
    }

    let cfg = shared.config.lock().map(|c| c.clone()).unwrap_or_default();
    let proc_meta = inspect_process(target_pid, &cfg.app_source_map);
    let app_display = if !proc_meta.product_name.is_empty() {
        proc_meta.product_name.as_str()
    } else if !proc_meta.exe_name.is_empty() {
        proc_meta.exe_name.as_str()
    } else {
        "App"
    };
    let win_rect = get_window_screen_rect(root_hwnd);
    let mon = MonitorSpace::for_point(center_x, center_y, root_hwnd);

    // Priority 1: Diagonal / horizontal blue text selection
    if let Some(text_hit) =
        try_extract_blue_text_range(&points, bbox, root_hwnd, target_pid, &win_class)
    {
        let n = shared.allocate_pick_number();
        let badge = circled_badge(n);
        if let Some(ov) = app.get_webview_window("overlay") {
            let pts_json: Vec<serde_json::Value> = points
                .iter()
                .map(|&(px, py)| serde_json::json!({ "x": px, "y": py }))
                .collect();
            let _ = ov.emit(
                "pen:picked",
                serde_json::json!({
                    "n": n,
                    "mode": "blue_text",
                    "color": "blue",
                    "rect": text_hit.bbox,
                    "strokes": pts_json,
                }),
            );
        }

        let grounding_lines = grounding::format_drag_grounding(
            root_hwnd,
            win_rect,
            &mon,
            text_hit.start_pt,
            text_hit.end_pt,
            text_hit.bbox,
        );
        let one_line_preview = text_hit
            .text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let short_preview = truncate_chars(&one_line_preview, 22);
        let char_count = text_hit.text.chars().count();

        let mut lines = Vec::new();
        lines.push(format!(
            "[DSB-PICK #{n} badge={badge} kind=blue_text_select chars={char_count} app=\"{}\"]",
            proc_meta.exe_name
        ));
        lines.push(format!("window_title: {}", win_title.trim()));
        lines.push(format!("window_class: {win_class}"));
        lines.push(format!(
            "process: pid={} exe=\"{}\" framework=\"{}\"",
            proc_meta.pid, proc_meta.exe_path, proc_meta.framework_guess
        ));
        lines.extend(grounding_lines);
        lines.push(format!(
            "source_control: type={} name=\"{}\"",
            text_hit.container_info.control_type,
            truncate_chars(&text_hit.container_info.name, 60)
        ));
        lines.push("selected_text:".to_string());
        lines.push(truncate_chars(&text_hit.text, 6000));
        lines.push("[/DSB-PICK]".to_string());

        let capsule = CapsulePayload {
            summary: CapsuleQuoteSummary {
                label: badge.clone(),
                title: format!("{badge} 选字 · {short_preview}"),
                detail: format!("{app_display} · {char_count} 字"),
                body: truncate_chars(&one_line_preview, 120),
            },
            text: lines.join("\n"),
        };

        log::info!(
            "Committed Stroke Pick #{n}: mode=blue_text_select chars={char_count} bbox=({},{},{}x{})",
            bbox.x,
            bbox.y,
            bbox.w,
            bbox.h
        );
        inject_capsule_into_bar(app, &capsule);
        return;
    }

    // Priority 2: CAD-style Blue Box / Freehand Lasso foreground element selection (NEVER background!)
    let raw_sampled = sample_blue_foreground_elements(&points, bbox, gesture, target_pid, 80);
    let sampled = coalesce_inline_text_runs(raw_sampled);
    if sampled.is_empty() {
        log::info!(
            "Blue {:?} found 0 foreground elements (background excluded) in bbox=({},{},{}x{})",
            gesture,
            bbox.x,
            bbox.y,
            bbox.w,
            bbox.h
        );
        if let Some(ov) = app.get_webview_window("overlay") {
            let _ = ov.emit("pen:clear_stroke", serde_json::json!({}));
        }
        emit_pick_toast(app, "蓝线范围内未检测到前景元素（已自动排除背景）");
        return;
    }

    let n = shared.allocate_pick_number();

    if let Some(ov) = app.get_webview_window("overlay") {
        let pts_json: Vec<serde_json::Value> = points
            .iter()
            .map(|&(px, py)| serde_json::json!({ "x": px, "y": py }))
            .collect();
        let item_rects: Vec<ScreenRect> = sampled.iter().map(|e| e.rect).collect();
        let _ = ov.emit(
            "pen:picked",
            serde_json::json!({
                "n": n,
                "mode": if gesture == BlueGestureKind::Lasso { "blue_lasso" } else { "blue_box" },
                "color": "blue",
                "strokes": pts_json,
                "rect": bbox,
                "itemRects": item_rects,
            }),
        );
    }

    let adapter_outputs = collect_deep_adapter_outputs(
        AdapterContext {
            hwnd: root_hwnd.0 as isize,
            window_title: win_title.clone(),
            window_class: win_class.clone(),
            proc_meta: proc_meta.clone(),
            primary_element: sampled.first().cloned(),
            tap_point: Some((center_x, center_y)),
            broker: shared.broker.clone(),
        },
        1200,
    );

    let grounding_lines =
        grounding::format_drag_grounding(root_hwnd, win_rect, &mon, start_pt, end_pt, bbox);

    let capsule = if sampled.len() == 1 {
        build_capsule_payload(
            n,
            "blue_select",
            &win_title,
            &win_class,
            &proc_meta,
            sampled.first(),
            &sampled,
            &adapter_outputs,
            &grounding_lines,
            Some((&mon, win_rect)),
            None,
        )
    } else {
        build_blue_element_group_capsule(
            n,
            gesture,
            &win_title,
            &win_class,
            &proc_meta,
            &sampled,
            &adapter_outputs,
            &grounding_lines,
        )
    };

    log::info!(
        "Committed Stroke Pick #{n}: mode={} bbox=({},{},{}x{}) sampled={}",
        if gesture == BlueGestureKind::Lasso {
            "blue_lasso"
        } else {
            "blue_box"
        },
        bbox.x,
        bbox.y,
        bbox.w,
        bbox.h,
        sampled.len()
    );
    inject_capsule_into_bar(app, &capsule);
}

/// Coalesce adjacent inline `type=Text` UIA leaf fragments on the same visual line
/// (common in Electron / streaming Markdown DOMs where every CJK word or inline code
/// token is a separate span) into a single continuous text item with a union bbox.
/// Non-Text interactive controls (Button, Edit, etc.) and far-apart columns are untouched.
fn coalesce_inline_text_runs(mut elements: Vec<UiaElementInfo>) -> Vec<UiaElementInfo> {
    if elements.len() <= 1 {
        return elements;
    }

    // Sort in natural reading order (same visual line left-to-right, then top-to-bottom)
    // so adjacent streaming text fragments are contiguous before merging.
    elements.sort_by(|a, b| {
        let a_cy = a.rect.y + a.rect.h / 2;
        let b_cy = b.rect.y + b.rect.h / 2;
        let line_tol = (a.rect.h.min(b.rect.h).max(12) / 2).clamp(6, 14);
        if (a_cy - b_cy).abs() <= line_tol {
            a.rect.x.cmp(&b.rect.x)
        } else {
            a.rect.y.cmp(&b.rect.y)
        }
    });

    fn is_mergeable_text(el: &UiaElementInfo) -> bool {
        el.control_type == "Text"
            && el.automation_id.trim().is_empty()
            && !el.name.trim().is_empty()
    }

    fn needs_space_between(prev: &str, next: &str, gap_px: i32) -> bool {
        let Some(last_ch) = prev.chars().last() else {
            return false;
        };
        let Some(first_ch) = next.chars().next() else {
            return false;
        };
        if last_ch.is_whitespace() || first_ch.is_whitespace() {
            return false;
        }
        if matches!(last_ch, '（' | '(' | '「' | '『' | '【' | '[')
            || matches!(
                first_ch,
                '）' | ')' | '，' | '。' | '！' | '？' | '、' | '：' | '；' | '」' | '』' | '】' | ']' | ',' | '.' | '!' | '?' | ':' | ';'
            )
        {
            return false;
        }
        // Inline code pills / spaced runs have >=3px visual gap; also keep space between two ASCII word tokens.
        gap_px >= 3 || (last_ch.is_ascii_alphanumeric() && first_ch.is_ascii_alphanumeric())
    }

    let mut merged: Vec<UiaElementInfo> = Vec::with_capacity(elements.len());
    for el in elements {
        if let Some(prev) = merged.last_mut() {
            if is_mergeable_text(prev) && is_mergeable_text(&el) {
                let prev_cy = prev.rect.y + prev.rect.h / 2;
                let cur_cy = el.rect.y + el.rect.h / 2;
                let max_h = prev.rect.h.max(el.rect.h).max(16);
                let same_line =
                    (prev.rect.y - el.rect.y).abs() <= 8 && (prev_cy - cur_cy).abs() <= max_h / 2;
                let gap = el.rect.x - (prev.rect.x + prev.rect.w);
                if same_line && (-6..=20).contains(&gap) {
                    if needs_space_between(&prev.name, &el.name, gap) {
                        prev.name.push(' ');
                    }
                    prev.name.push_str(&el.name);
                    let min_x = prev.rect.x.min(el.rect.x);
                    let min_y = prev.rect.y.min(el.rect.y);
                    let max_r = (prev.rect.x + prev.rect.w).max(el.rect.x + el.rect.w);
                    let max_b = (prev.rect.y + prev.rect.h).max(el.rect.y + el.rect.h);
                    prev.rect = ScreenRect {
                        x: min_x,
                        y: min_y,
                        w: (max_r - min_x).max(0),
                        h: (max_b - min_y).max(0),
                    };
                    if !el.text_snippet.is_empty() && prev.text_snippet != prev.name {
                        prev.text_snippet = prev.name.clone();
                    }
                    continue;
                }
            }
        }
        merged.push(el);
    }
    merged
}

#[allow(clippy::too_many_arguments)]
fn build_blue_element_group_capsule(
    n: usize,
    gesture: BlueGestureKind,
    win_title: &str,
    win_class: &str,
    proc_meta: &process::ProcessMetadata,
    elements: &[UiaElementInfo],
    adapter_outputs: &[adapters::AdapterOutput],
    grounding_lines: &[String],
) -> CapsulePayload {
    let badge = circled_badge(n);
    let count = elements.len();
    let gesture_tag = match gesture {
        BlueGestureKind::Box => "box",
        BlueGestureKind::Lasso => "lasso",
    };
    let gesture_zh = match gesture {
        BlueGestureKind::Box => "蓝线框选",
        BlueGestureKind::Lasso => "蓝线圈选",
    };
    let app_display = if !proc_meta.product_name.is_empty() {
        proc_meta.product_name.as_str()
    } else if !proc_meta.exe_name.is_empty() {
        proc_meta.exe_name.as_str()
    } else {
        "App"
    };

    let preview_names: Vec<String> = elements
        .iter()
        .take(4)
        .map(|e| {
            if !e.name.is_empty() {
                truncate_chars(&e.name, 16)
            } else if !e.value.is_empty() && e.value != "[PASSWORD_REDACTED]" {
                truncate_chars(&e.value, 16)
            } else {
                e.control_type.clone()
            }
        })
        .collect();
    let body_preview = if count > 4 {
        format!("{} 等 {} 项前景元素", preview_names.join("、"), count)
    } else {
        preview_names.join("、")
    };

    let mut lines = Vec::new();
    lines.push(format!(
        "[DSB-PICK #{n} badge={badge} kind=element_group gesture={gesture_tag} count={count} app=\"{}\"]",
        proc_meta.exe_name
    ));
    lines.push(format!("window_title: {}", win_title.trim()));
    lines.push(format!("window_class: {win_class}"));
    lines.push(format!(
        "process: pid={} exe=\"{}\" framework=\"{}\"",
        proc_meta.pid, proc_meta.exe_path, proc_meta.framework_guess
    ));
    lines.extend(grounding_lines.iter().cloned());
    lines.push(format!("element_group ({count} foreground items):"));
    for (idx, el) in elements.iter().enumerate() {
        let mut desc = format!(
            "  [{}] type={} rect=({},{},{}x{})",
            idx + 1,
            el.control_type,
            el.rect.x,
            el.rect.y,
            el.rect.w,
            el.rect.h
        );
        if !el.name.is_empty() {
            desc.push_str(&format!(" name=\"{}\"", truncate_chars(&el.name, 80)));
        }
        if !el.value.is_empty() && el.value != "[PASSWORD_REDACTED]" && el.value != el.name {
            desc.push_str(&format!(" value=\"{}\"", truncate_chars(&el.value, 80)));
        }
        if !el.automation_id.is_empty() {
            desc.push_str(&format!(" id=\"{}\"", el.automation_id));
        }
        lines.push(desc);
    }

    for out in adapter_outputs {
        for detail in &out.lines {
            lines.push(detail.clone());
        }
        for p in out.paths.iter().take(12) {
            if let Some(meta) = inspect_path_metadata(Path::new(p), 600) {
                append_fs_meta_lines(&mut lines, &meta);
            }
        }
    }
    lines.push("[/DSB-PICK]".to_string());

    CapsulePayload {
        summary: CapsuleQuoteSummary {
            label: badge.clone(),
            title: format!("{badge} 元素组 ({count}项)"),
            detail: format!("{app_display} · {gesture_zh}"),
            body: body_preview,
        },
        text: lines.join("\n"),
    }
}

fn is_wps_exe(exe_name: &str) -> bool {
    matches!(exe_name, "wps.exe" | "wpsoffice.exe" | "et.exe" | "wpp.exe")
}

fn office_app_display_name(exe_name: &str, kind: OfficeAppKind) -> &'static str {
    let is_wps = is_wps_exe(exe_name);
    match (is_wps, kind) {
        (true, OfficeAppKind::Word) => "WPS 文字",
        (true, OfficeAppKind::Excel) => "WPS 表格",
        (true, OfficeAppKind::PowerPoint) => "WPS 演示",
        (false, OfficeAppKind::Word) => "Word",
        (false, OfficeAppKind::Excel) => "Excel",
        (false, OfficeAppKind::PowerPoint) => "PowerPoint",
    }
}

fn try_execute_solidworks_tap_pick(
    app: &AppHandle,
    shared: &AppShared,
    x: i32,
    y: i32,
    pre_capture: Option<&RawScreenCapture>,
) -> bool {
    let Some(hit) = solidworks::detect_solidworks_at(x, y) else {
        return false;
    };

    // Inspect UIA at (x, y) so we:
    // 1. Let toolbar/menu/ribbon buttons fall through to normal UIA picking
    // 2. Capture exact FeatureManager TreeItem label & rect when clicking the feature tree
    let uia_info = UIAutomation::new().ok().and_then(|auto| {
        let walker = auto.get_control_view_walker().ok()?;
        let el = find_target_uia_element(&auto, &walker, hit.root_hwnd, hit.pid, x, y)?;
        Some(extract_uia_element_info(&el, &walker, false))
    });

    if let Some(ref info) = uia_info {
        let ct = info.control_type.as_str();
        // Keep SolidWorks CommandManager / Menu / Dialog controls on the standard UIA path
        if matches!(
            ct,
            "Button"
                | "MenuItem"
                | "TabItem"
                | "SplitButton"
                | "ComboBox"
                | "CheckBox"
                | "RadioButton"
                | "Edit"
                | "Spinner"
        ) {
            return false;
        }
    }

    let uia_label = uia_info
        .as_ref()
        .filter(|info| {
            matches!(info.control_type.as_str(), "TreeItem" | "Text" | "ListItem")
                && !info.name.trim().is_empty()
        })
        .map(|info| info.name.clone());

    let Some(sw) = solidworks::pick_solidworks_feature_com(
        &shared.broker,
        x,
        y,
        uia_label.as_deref(),
    ) else {
        return false;
    };

    if sw.items.is_empty() && sw.doc_title.is_empty() {
        return false;
    }

    let cfg = shared.config.lock().map(|c| c.clone()).unwrap_or_default();
    let n = shared.allocate_pick_number();
    let badge = circled_badge(n);

    // Highlight the TreeItem rect if picked from FeatureManager, else a crisp 52x52 reticle at the 3D click point
    let pick_rect = uia_info
        .as_ref()
        .filter(|info| info.control_type == "TreeItem" && info.rect.w > 12 && info.rect.h > 8)
        .map(|info| info.rect)
        .unwrap_or(ScreenRect {
            x: x - 26,
            y: y - 26,
            w: 52,
            h: 52,
        });

    if let Some(ov) = app.get_webview_window("overlay") {
        let _ = ov.emit(
            "pen:picked",
            serde_json::json!({ "n": n, "rect": pick_rect }),
        );
    }

    let owned_cap;
    let raw_ref = match pre_capture {
        Some(r) => Some(r),
        None => {
            owned_cap = capture_target_window_bitblt(hit.root_hwnd, x, y).ok();
            owned_cap.as_ref()
        }
    };
    let screenshot_path = raw_ref.and_then(|raw| {
        save_annotated_capture(app, raw, Some(pick_rect), &[], n, &cfg)
            .ok()
            .map(|p| p.to_string_lossy().to_string())
    });

    let proc_meta = inspect_process(hit.pid, &cfg.app_source_map);
    let (chip_title, chip_detail) = solidworks::build_solidworks_chip_title(&badge, &sw);

    let mut lines = vec![
        format!("[DSB-PICK #{n} | kind=solidworks_feature]"),
        format!(
            "app: SolidWorks ({}) | window=\"{}\"",
            proc_meta.exe_name,
            hit.win_title.trim()
        ),
    ];
    lines.extend(solidworks::format_solidworks_capsule_lines(&sw));

    let win_rect = get_window_screen_rect(hit.root_hwnd);
    let mon = MonitorSpace::for_point(x, y, hit.root_hwnd);
    let target_ct = uia_info
        .as_ref()
        .map(|i| i.control_type.as_str())
        .unwrap_or("SolidWorksViewport");
    lines.extend(grounding::format_tap_grounding(
        hit.root_hwnd,
        win_rect,
        &mon,
        (x, y),
        pick_rect,
        target_ct,
        uia_info.as_ref().and_then(|i| i.affordance.as_ref()),
    ));
    if let Some(sp) = screenshot_path.as_deref() {
        lines.push(format!(
            "debug_screenshot_path: {sp} (仅当以上信息不足或可疑时再打开查看)"
        ));
    }
    lines.push("[/DSB-PICK]".to_string());

    let body_preview = sw
        .items
        .first()
        .map(|it| {
            if !it.dimensions.is_empty() {
                it.dimensions
                    .iter()
                    .take(3)
                    .map(|d| format!("{}={}{}", d.name, d.value, d.unit))
                    .collect::<Vec<_>>()
                    .join(", ")
            } else if !it.entity_geometry.is_empty() {
                it.entity_geometry.clone()
            } else {
                format!("{} ({})", it.feature_name, it.feature_type)
            }
        })
        .unwrap_or_else(|| format!("SolidWorks · {}", sw.doc_title));

    let capsule = CapsulePayload {
        summary: CapsuleQuoteSummary {
            label: badge,
            title: truncate_chars(&chip_title, 44),
            detail: truncate_chars(&chip_detail, 44),
            body: truncate_chars(&body_preview, 110),
        },
        text: lines.join("\n"),
    };
    log::info!(
        "Committed SolidWorks Feature Pick #{n}: tier={} items={} title='{}' screenshot={:?}",
        sw.pick_tier,
        sw.items.len(),
        chip_title,
        screenshot_path
    );
    inject_capsule_into_bar(app, &capsule);
    true
}

fn try_execute_office_tap_pick(
    app: &AppHandle,
    shared: &AppShared,
    x: i32,
    y: i32,
    pre_capture: Option<&RawScreenCapture>,
) -> bool {
    let Some(hit) = office::detect_office_canvas_at(x, y) else {
        return false;
    };
    if !hit.in_canvas {
        return false;
    }

    match hit.kind {
        OfficeAppKind::Excel => {
            let Some(ex) = office::pick_excel_cell_com(
                &shared.broker,
                hit.root_hwnd,
                x,
                y,
                hit.dpi_scale,
            ) else {
                return false;
            };
            let cfg = shared.config.lock().map(|c| c.clone()).unwrap_or_default();
            let n = shared.allocate_pick_number();
            let badge = circled_badge(n);
            let rect = ex.screen_rect.unwrap_or(ScreenRect {
                x: x - 42,
                y: y - 13,
                w: 84,
                h: 26,
            });
            if let Some(ov) = app.get_webview_window("overlay") {
                let _ = ov.emit(
                    "pen:picked",
                    serde_json::json!({ "n": n, "rect": rect, "mode": "excel_cell" }),
                );
            }
            let owned_cap;
            let raw_ref = match pre_capture {
                Some(r) => Some(r),
                None => {
                    owned_cap = capture_target_window_bitblt(hit.root_hwnd, x, y).ok();
                    owned_cap.as_ref()
                }
            };
            let screenshot_path = raw_ref.and_then(|raw| {
                save_annotated_capture(app, raw, Some(rect), &[], n, &cfg)
                    .ok()
                    .map(|p| p.to_string_lossy().to_string())
            });
            let proc_meta = inspect_process(hit.pid, &cfg.app_source_map);
            let app_label = office_app_display_name(&proc_meta.exe_name, OfficeAppKind::Excel);
            let cell_ref = if ex.sheet.is_empty() {
                ex.address.clone()
            } else {
                format!("{}!{}", ex.sheet, ex.address)
            };
            let disp_short = truncate_chars(ex.display_text.trim(), 16);
            let chip_title = if disp_short.is_empty() {
                format!("{badge} {cell_ref}")
            } else {
                format!("{badge} {cell_ref} · {disp_short}")
            };
            let wb_badge = if !ex.workbook_name.is_empty() {
                ex.workbook_name.as_str()
            } else {
                hit.win_title.trim()
            };
            let chip_detail = format!("{app_label} · {}", truncate_chars(wb_badge, 26));

            let mut lines = Vec::new();
            lines.push(format!(
                "[DSB-PICK #{n} badge={badge} kind=office_excel_cell app=\"{}\"]",
                proc_meta.exe_name
            ));
            lines.push(format!("office_app: {app_label}"));
            if !ex.prog_id.is_empty() {
                lines.push(format!("office_prog_id: {}", ex.prog_id));
            }
            lines.push(format!("window_hwnd: {}", hit.root_hwnd.0 as isize));
            if !ex.full_name.is_empty() {
                lines.push(format!("office_document_path: {}", ex.full_name));
            }
            lines.push(format!("office_workbook: {}", ex.workbook_name));
            lines.push(format!("excel_sheet: {}", ex.sheet));
            lines.push(format!("excel_address: {}", ex.address));
            if !ex.col_header.is_empty() {
                lines.push(format!("excel_col_header: {}", ex.col_header));
            }
            if !ex.row_header.is_empty() {
                lines.push(format!("excel_row_header: {}", ex.row_header));
            }
            if !ex.display_text.is_empty() {
                lines.push(format!("excel_display_text: {}", ex.display_text));
            }
            if !ex.value.is_empty() && ex.value != ex.display_text {
                lines.push(format!("excel_raw_value: {}", ex.value));
            }
            if !ex.formula.is_empty() {
                lines.push(format!("excel_formula: {}", ex.formula));
            }
            let target_locator = if ex.full_name.is_empty() {
                format!("{} | {cell_ref}", ex.workbook_name)
            } else {
                format!("{} | {cell_ref}", ex.full_name)
            };
            lines.extend(office::format_live_edit_guidance(
                "excel",
                is_wps_exe(&proc_meta.exe_name),
                &target_locator,
            ));
            let win_rect = get_window_screen_rect(hit.root_hwnd);
            let mon = MonitorSpace::for_point(x, y, hit.root_hwnd);
            lines.extend(grounding::format_tap_grounding(
                hit.root_hwnd,
                win_rect,
                &mon,
                (x, y),
                rect,
                "ExcelCell",
                None,
            ));
            if let Some(sp) = screenshot_path.as_deref() {
                lines.push(format!(
                    "debug_screenshot_path: {sp} (仅当以上信息不足或可疑时再打开查看)"
                ));
            }
            lines.push("[/DSB-PICK]".to_string());

            let capsule = CapsulePayload {
                summary: CapsuleQuoteSummary {
                    label: badge,
                    title: chip_title,
                    detail: chip_detail,
                    body: format!("{cell_ref} = {}", ex.display_text),
                },
                text: lines.join("\n"),
            };
            log::info!(
                "Committed Excel Cell Pick #{n}: ref='{cell_ref}' val='{}' screenshot={:?}",
                truncate_chars(&ex.display_text, 40),
                screenshot_path
            );
            inject_capsule_into_bar(app, &capsule);
            true
        }
        OfficeAppKind::PowerPoint => {
            let Some(ppt) = office::pick_ppt_element_com(
                &shared.broker,
                hit.root_hwnd,
                x,
                y,
                hit.dpi_scale,
            ) else {
                return false;
            };
            let cfg = shared.config.lock().map(|c| c.clone()).unwrap_or_default();
            let n = shared.allocate_pick_number();
            let badge = circled_badge(n);
            let rect = ppt.screen_rect.unwrap_or(ScreenRect {
                x: x - 48,
                y: y - 24,
                w: 96,
                h: 48,
            });
            if let Some(ov) = app.get_webview_window("overlay") {
                let _ = ov.emit(
                    "pen:picked",
                    serde_json::json!({ "n": n, "rect": rect, "mode": "ppt_element" }),
                );
            }
            let owned_cap;
            let raw_ref = match pre_capture {
                Some(r) => Some(r),
                None => {
                    owned_cap = capture_target_window_bitblt(hit.root_hwnd, x, y).ok();
                    owned_cap.as_ref()
                }
            };
            let screenshot_path = raw_ref.and_then(|raw| {
                save_annotated_capture(app, raw, Some(rect), &[], n, &cfg)
                    .ok()
                    .map(|p| p.to_string_lossy().to_string())
            });
            let proc_meta = inspect_process(hit.pid, &cfg.app_source_map);
            let app_label = office_app_display_name(&proc_meta.exe_name, OfficeAppKind::PowerPoint);
            let first_line = ppt
                .text
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("");
            let elem_label = if !first_line.is_empty() {
                truncate_chars(first_line, 20)
            } else if !ppt.shape_name.is_empty() {
                truncate_chars(&ppt.shape_name, 22)
            } else {
                ppt.shape_type.clone()
            };
            let chip_title = format!("{badge} Slide {} · {elem_label}", ppt.slide_index);
            let pres_badge = if !ppt.pres_name.is_empty() {
                ppt.pres_name.as_str()
            } else {
                hit.win_title.trim()
            };
            let chip_detail = format!(
                "{app_label} · {} ({})",
                truncate_chars(pres_badge, 22),
                ppt.shape_type
            );

            let mut lines = Vec::new();
            lines.push(format!(
                "[DSB-PICK #{n} badge={badge} kind=office_ppt_element app=\"{}\"]",
                proc_meta.exe_name
            ));
            lines.push(format!("office_app: {app_label}"));
            if !ppt.prog_id.is_empty() {
                lines.push(format!("office_prog_id: {}", ppt.prog_id));
            }
            lines.push(format!("window_hwnd: {}", hit.root_hwnd.0 as isize));
            if !ppt.full_name.is_empty() {
                lines.push(format!("office_document_path: {}", ppt.full_name));
            }
            lines.push(format!("ppt_presentation: {}", ppt.pres_name));
            lines.push(format!(
                "ppt_slide: {} / {}",
                ppt.slide_index, ppt.slide_count
            ));
            lines.push(format!(
                "ppt_shape: id={} name=\"{}\" type={}",
                ppt.shape_id, ppt.shape_name, ppt.shape_type
            ));
            if let Some(gid) = ppt.parent_group_id {
                lines.push(format!(
                    "ppt_parent_group: id={gid} name=\"{}\"",
                    ppt.parent_group_name
                ));
            }
            if !ppt.geometry.is_empty() {
                lines.push(format!("ppt_geometry: {}", ppt.geometry));
            }
            if !ppt.style.is_empty() {
                lines.push(format!("ppt_style: {}", ppt.style));
            }
            if !ppt.text.is_empty() {
                lines.push(format!("ppt_shape_text:\n{}", ppt.text));
            }
            let mut target_locator = format!(
                "{} | slide={} shape_id={} shape_name=\"{}\"",
                if ppt.full_name.is_empty() {
                    ppt.pres_name.as_str()
                } else {
                    ppt.full_name.as_str()
                },
                ppt.slide_index,
                ppt.shape_id,
                ppt.shape_name
            );
            if let Some(gid) = ppt.parent_group_id {
                target_locator.push_str(&format!(" parent_group_id={gid}"));
            }
            lines.extend(office::format_live_edit_guidance(
                "ppt",
                is_wps_exe(&proc_meta.exe_name),
                &target_locator,
            ));
            let win_rect = get_window_screen_rect(hit.root_hwnd);
            let mon = MonitorSpace::for_point(x, y, hit.root_hwnd);
            lines.extend(grounding::format_tap_grounding(
                hit.root_hwnd,
                win_rect,
                &mon,
                (x, y),
                rect,
                &ppt.shape_type,
                None,
            ));
            if let Some(sp) = screenshot_path.as_deref() {
                lines.push(format!(
                    "debug_screenshot_path: {sp} (仅当以上信息不足或可疑时再打开查看)"
                ));
            }
            lines.push("[/DSB-PICK]".to_string());

            let capsule = CapsulePayload {
                summary: CapsuleQuoteSummary {
                    label: badge,
                    title: chip_title,
                    detail: chip_detail,
                    body: if ppt.text.is_empty() {
                        format!("{} ({})", ppt.shape_name, ppt.shape_type)
                    } else {
                        truncate_chars(&ppt.text, 120)
                    },
                },
                text: lines.join("\n"),
            };
            log::info!(
                "Committed PPT Element Pick #{n}: slide={} shape='{}' (id={}) screenshot={:?}",
                ppt.slide_index,
                ppt.shape_name,
                ppt.shape_id,
                screenshot_path
            );
            inject_capsule_into_bar(app, &capsule);
            true
        }
        OfficeAppKind::Word => false,
    }
}

fn try_execute_office_drag_pick(
    app: &AppHandle,
    shared: &AppShared,
    points: &[(i32, i32)],
    pre_capture: Option<&RawScreenCapture>,
) -> bool {
    if points.len() < 2 {
        return false;
    }
    let start = points[0];
    let end = *points.last().unwrap_or(&start);
    let Some(hit) = office::detect_office_canvas_at(start.0, start.1) else {
        return false;
    };
    if !hit.in_canvas {
        return false;
    }

    let bbox = ScreenRect {
        x: start.0.min(end.0),
        y: start.1.min(end.1),
        w: (start.0 - end.0).abs().max(24),
        h: (start.1 - end.1).abs().max(18),
    };

    match hit.kind {
        OfficeAppKind::PowerPoint => {
            // PowerPoint / WPS Presentation drag MUST keep drawing the red freehand stroke unchanged!
            false
        }
        OfficeAppKind::Word => {
            let uia_text = office::select_word_text_uia(hit.root_hwnd, start, end, true);
            let com_res = office::pick_word_text_com(
                &shared.broker,
                start.0,
                start.1,
                end.0,
                end.1,
                hit.dpi_scale,
            );
            let selected_text = com_res
                .as_ref()
                .map(|r| r.selected_text.trim())
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .or(uia_text)
                .unwrap_or_default();

            let cfg = shared.config.lock().map(|c| c.clone()).unwrap_or_default();
            let n = shared.allocate_pick_number();
            let badge = circled_badge(n);
            if let Some(ov) = app.get_webview_window("overlay") {
                let _ = ov.emit(
                    "pen:picked",
                    serde_json::json!({ "n": n, "rect": bbox, "mode": "word_text" }),
                );
            }
            let owned_cap;
            let raw_ref = match pre_capture {
                Some(r) => Some(r),
                None => {
                    owned_cap = capture_target_window_bitblt(
                        hit.root_hwnd,
                        bbox.x + bbox.w / 2,
                        bbox.y + bbox.h / 2,
                    )
                    .ok();
                    owned_cap.as_ref()
                }
            };
            // Save clean rectangular box screenshot (NO red freehand squiggle!)
            let screenshot_path = raw_ref.and_then(|raw| {
                save_annotated_capture(app, raw, Some(bbox), &[], n, &cfg)
                    .ok()
                    .map(|p| p.to_string_lossy().to_string())
            });
            let proc_meta = inspect_process(hit.pid, &cfg.app_source_map);
            let app_label = office_app_display_name(&proc_meta.exe_name, OfficeAppKind::Word);

            let (prog_id, full_name, doc_name, page_num, para_text, range_start, range_end) =
                match com_res {
                    Some(r) => (
                        r.prog_id,
                        r.full_name,
                        r.doc_name,
                        r.page_number,
                        r.paragraph_text,
                        r.range_start,
                        r.range_end,
                    ),
                    None => (
                        String::new(),
                        String::new(),
                        hit.win_title.trim().to_string(),
                        String::new(),
                        String::new(),
                        None,
                        None,
                    ),
                };

            let single_line = selected_text
                .replace(['\r', '\n'], " ")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let chip_title = if !single_line.is_empty() {
                format!("{badge} 「{}」", truncate_chars(&single_line, 20))
            } else {
                format!("{badge} {} 选区", truncate_chars(&doc_name, 18))
            };
            let doc_badge = if !doc_name.is_empty() {
                doc_name.as_str()
            } else {
                hit.win_title.trim()
            };
            let chip_detail = if !page_num.is_empty() {
                format!(
                    "{app_label} · {} · P{page_num}",
                    truncate_chars(doc_badge, 20)
                )
            } else {
                format!("{app_label} · {}", truncate_chars(doc_badge, 24))
            };

            let mut lines = Vec::new();
            lines.push(format!(
                "[DSB-PICK #{n} badge={badge} kind=office_word_text app=\"{}\"]",
                proc_meta.exe_name
            ));
            lines.push(format!("office_app: {app_label}"));
            if !prog_id.is_empty() {
                lines.push(format!("office_prog_id: {prog_id}"));
            }
            lines.push(format!("window_hwnd: {}", hit.root_hwnd.0 as isize));
            if !full_name.is_empty() {
                lines.push(format!("office_document_path: {full_name}"));
            }
            lines.push(format!("word_document: {doc_name}"));
            if !page_num.is_empty() {
                lines.push(format!("word_page: {page_num}"));
            }
            if let (Some(rs), Some(re)) = (range_start, range_end) {
                lines.push(format!("word_range_offsets: {rs}..{re}"));
            }
            if !selected_text.is_empty() {
                lines.push(format!("word_selected_text:\n{selected_text}"));
            } else {
                lines.push(
                    "word_selected_text: (未读取到文本字符，可能划选在空白/图片区域，请参考截图)"
                        .to_string(),
                );
            }
            if !para_text.is_empty() && para_text != selected_text {
                lines.push(format!("word_paragraph_context:\n{para_text}"));
            }
            let doc_id = if full_name.is_empty() {
                doc_name.as_str()
            } else {
                full_name.as_str()
            };
            let target_locator = match (range_start, range_end) {
                (Some(rs), Some(re)) => format!("{doc_id} | range={rs}..{re}"),
                _ => doc_id.to_string(),
            };
            lines.extend(office::format_live_edit_guidance(
                "word",
                is_wps_exe(&proc_meta.exe_name),
                &target_locator,
            ));
            let win_rect = get_window_screen_rect(hit.root_hwnd);
            let mon = MonitorSpace::for_point(start.0, start.1, hit.root_hwnd);
            lines.extend(grounding::format_drag_grounding(
                hit.root_hwnd,
                win_rect,
                &mon,
                start,
                end,
                bbox,
            ));
            if let Some(sp) = screenshot_path.as_deref() {
                lines.push(format!(
                    "debug_screenshot_path: {sp} (仅当以上信息不足或可疑时再打开查看)"
                ));
            }
            lines.push("[/DSB-PICK]".to_string());

            let capsule = CapsulePayload {
                summary: CapsuleQuoteSummary {
                    label: badge,
                    title: chip_title,
                    detail: chip_detail,
                    body: if !single_line.is_empty() {
                        truncate_chars(&single_line, 120)
                    } else {
                        format!("{app_label} — {}", truncate_chars(doc_badge, 80))
                    },
                },
                text: lines.join("\n"),
            };
            log::info!(
                "Committed Word Text Pick #{n}: chars={} range={:?}..{:?} screenshot={:?}",
                selected_text.chars().count(),
                range_start,
                range_end,
                screenshot_path
            );
            inject_capsule_into_bar(app, &capsule);
            true
        }
        OfficeAppKind::Excel => {
            let Some(ex) = office::pick_excel_range_com(
                &shared.broker,
                hit.root_hwnd,
                start.0,
                start.1,
                end.0,
                end.1,
                hit.dpi_scale,
            ) else {
                return false;
            };
            let cfg = shared.config.lock().map(|c| c.clone()).unwrap_or_default();
            let n = shared.allocate_pick_number();
            let badge = circled_badge(n);
            let rect = ex.screen_rect.unwrap_or(bbox);
            if let Some(ov) = app.get_webview_window("overlay") {
                let _ = ov.emit(
                    "pen:picked",
                    serde_json::json!({ "n": n, "rect": rect, "mode": "excel_box" }),
                );
            }
            let owned_cap;
            let raw_ref = match pre_capture {
                Some(r) => Some(r),
                None => {
                    owned_cap = capture_target_window_bitblt(
                        hit.root_hwnd,
                        rect.x + rect.w / 2,
                        rect.y + rect.h / 2,
                    )
                    .ok();
                    owned_cap.as_ref()
                }
            };
            // Save clean rectangular box screenshot (NO red freehand squiggle!)
            let screenshot_path = raw_ref.and_then(|raw| {
                save_annotated_capture(app, raw, Some(rect), &[], n, &cfg)
                    .ok()
                    .map(|p| p.to_string_lossy().to_string())
            });
            let proc_meta = inspect_process(hit.pid, &cfg.app_source_map);
            let app_label = office_app_display_name(&proc_meta.exe_name, OfficeAppKind::Excel);
            let range_ref = if ex.sheet.is_empty() {
                ex.address.clone()
            } else {
                format!("{}!{}", ex.sheet, ex.address)
            };
            let chip_title = if ex.is_range {
                format!("{badge} {range_ref} ({}×{})", ex.rows, ex.cols)
            } else {
                format!("{badge} {range_ref}")
            };
            let wb_badge = if !ex.workbook_name.is_empty() {
                ex.workbook_name.as_str()
            } else {
                hit.win_title.trim()
            };
            let chip_detail = format!("{app_label} · {}", truncate_chars(wb_badge, 26));

            let mut lines = Vec::new();
            lines.push(format!(
                "[DSB-PICK #{n} badge={badge} kind=office_excel_range app=\"{}\"]",
                proc_meta.exe_name
            ));
            lines.push(format!("office_app: {app_label}"));
            if !ex.prog_id.is_empty() {
                lines.push(format!("office_prog_id: {}", ex.prog_id));
            }
            lines.push(format!("window_hwnd: {}", hit.root_hwnd.0 as isize));
            if !ex.full_name.is_empty() {
                lines.push(format!("office_document_path: {}", ex.full_name));
            }
            lines.push(format!("office_workbook: {}", ex.workbook_name));
            lines.push(format!("excel_sheet: {}", ex.sheet));
            lines.push(format!(
                "excel_address: {} ({} rows × {} cols)",
                ex.address, ex.rows, ex.cols
            ));
            if !ex.table_preview.is_empty() {
                lines.push(format!("excel_table_preview:\n{}", ex.table_preview));
            }
            if !ex.formulas_preview.is_empty() {
                lines.push(format!("excel_formulas: {}", ex.formulas_preview));
            }
            let target_locator = if ex.full_name.is_empty() {
                format!("{} | {range_ref}", ex.workbook_name)
            } else {
                format!("{} | {range_ref}", ex.full_name)
            };
            lines.extend(office::format_live_edit_guidance(
                "excel",
                is_wps_exe(&proc_meta.exe_name),
                &target_locator,
            ));
            let win_rect = get_window_screen_rect(hit.root_hwnd);
            let mon = MonitorSpace::for_point(start.0, start.1, hit.root_hwnd);
            lines.extend(grounding::format_drag_grounding(
                hit.root_hwnd,
                win_rect,
                &mon,
                start,
                end,
                rect,
            ));
            if let Some(sp) = screenshot_path.as_deref() {
                lines.push(format!(
                    "debug_screenshot_path: {sp} (仅当以上信息不足或可疑时再打开查看)"
                ));
            }
            lines.push("[/DSB-PICK]".to_string());

            let capsule = CapsulePayload {
                summary: CapsuleQuoteSummary {
                    label: badge,
                    title: chip_title,
                    detail: chip_detail,
                    body: format!("{range_ref} ({}×{})", ex.rows, ex.cols),
                },
                text: lines.join("\n"),
            };
            log::info!(
                "Committed Excel Range Pick #{n}: ref='{range_ref}' ({}x{}) screenshot={:?}",
                ex.rows,
                ex.cols,
                screenshot_path
            );
            inject_capsule_into_bar(app, &capsule);
            true
        }
    }
}

pub fn maybe_auto_attach_explorer_selection(app: &AppHandle, fg_hwnd_raw: isize) {
    if fg_hwnd_raw == 0 {
        return;
    }
    let hwnd = HWND(fg_hwnd_raw as *mut std::ffi::c_void);
    let class_name = get_hwnd_class_name(hwnd);
    if !matches!(
        class_name.as_str(),
        "CabinetWClass" | "ExploreWClass" | "Progman" | "WorkerW"
    ) {
        return;
    }

    let Some(shared) = app.try_state::<AppShared>() else {
        return;
    };
    let resp = shared
        .broker
        .call(crate::broker::BrokerRequest::ShellSelection {
            hwnd: fg_hwnd_raw,
            class_name: class_name.clone(),
        });
    if !resp.ok || resp.paths.is_empty() {
        return;
    }

    let n = shared.allocate_pick_number();
    let badge = circled_badge(n);
    let first_name = Path::new(&resp.paths[0])
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| resp.paths[0].clone());

    let title = if resp.paths.len() == 1 {
        format!("{badge} {first_name}")
    } else {
        format!("{badge} {first_name} 等 {} 项", resp.paths.len())
    };

    let mut lines = Vec::new();
    lines.push(format!(
        "[DSB-PICK #{n} kind=explorer_selection count={}]",
        resp.paths.len()
    ));
    for p in resp.paths.iter().take(15) {
        lines.push(format!("- selected_path: {p}"));
        if let Some(meta) = inspect_path_metadata(Path::new(p), 800) {
            append_fs_meta_lines(&mut lines, &meta);
        }
    }
    lines.push("[/DSB-PICK]".to_string());

    let hidden_text = lines.join("\n");
    let capsule = CapsulePayload {
        summary: CapsuleQuoteSummary {
            label: badge,
            title,
            detail: "资源管理器已选文件".to_string(),
            body: resp.paths.join("\n"),
        },
        text: hidden_text,
    };

    log::info!(
        "Auto-attached Explorer selection #{n} ({} items)",
        resp.paths.len()
    );
    inject_capsule_into_bar(app, &capsule);
}

#[allow(dead_code)]
fn sample_uia_grid_in_rect(
    bbox: ScreenRect,
    expected_pid: u32,
    grid_side: usize,
    cap: usize,
) -> Vec<UiaElementInfo> {
    let mut out = Vec::new();
    let Ok(auto) = UIAutomation::new() else {
        return out;
    };
    let Ok(walker) = auto.get_control_view_walker() else {
        return out;
    };

    let mut seen = HashSet::new();
    let cols = grid_side.max(2);
    let rows = grid_side.max(2);

    for r in 0..rows {
        for c in 0..cols {
            if out.len() >= cap {
                return out;
            }
            let px = bbox.x + ((bbox.w as usize) * (2 * c + 1) / (2 * cols)) as i32;
            let py = bbox.y + ((bbox.h as usize) * (2 * r + 1) / (2 * rows)) as i32;
            let (rhwnd, pid, _, _) = resolve_root_window_at(px, py);
            if pid != expected_pid {
                continue;
            }
            if let Some(el) = find_target_uia_element(&auto, &walker, rhwnd, pid, px, py) {
                let info = extract_uia_element_info(&el, &walker, out.is_empty());
                if !is_valid_tap_element(&info) {
                    continue;
                }
                let key = format!(
                    "{}:{}:{}:{}:{}",
                    info.control_type_id, info.rect.x, info.rect.y, info.name, info.automation_id
                );
                if seen.insert(key) {
                    out.push(info);
                }
            }
        }
    }
    out
}

/// Extracts a meaningful control name from the nearest interactive UIA ancestor
/// (e.g. `Button(name="固定展开", id="")` when the user clicked an unnamed `<svg>`/`Image` inside it).
pub fn extract_interactive_ancestor_label(ancestors: &[String]) -> Option<String> {
    const INTERACTIVE_TYPES: &[&str] = &[
        "Button",
        "MenuItem",
        "TabItem",
        "Hyperlink",
        "CheckBox",
        "RadioButton",
        "ComboBox",
        "ListItem",
        "TreeItem",
        "SplitButton",
        "Edit",
        "Slider",
        "ToggleButton",
        "HeaderItem",
        "DataItem",
    ];
    for entry in ancestors.iter().take(3) {
        let Some((ctrl_type, rest)) = entry.split_once('(') else {
            continue;
        };
        if !INTERACTIVE_TYPES.contains(&ctrl_type.trim()) {
            continue;
        }
        if let Some(after_key) = rest.split("name=\"").nth(1) {
            if let Some((raw_name, _)) = after_key.split_once('"') {
                let cleaned = raw_name.trim();
                if !cleaned.is_empty() {
                    return Some(truncate_chars(cleaned, 28));
                }
            }
        }
    }
    None
}

/// Resolves the best short semantic label for a picked UIA element, promoting an immediate
/// named interactive ancestor (like `Button(name="固定展开")`) when the clicked leaf is an
/// unnamed icon/Image inside a button.
pub fn resolve_primary_element_label(e: &UiaElementInfo) -> Option<String> {
    if !e.name.trim().is_empty() {
        Some(truncate_chars(e.name.trim(), 28))
    } else if !e.value.trim().is_empty() && e.value != "[PASSWORD_REDACTED]" {
        Some(truncate_chars(e.value.trim(), 28))
    } else if let Some(anc) = extract_interactive_ancestor_label(&e.ancestors) {
        Some(anc)
    } else if !e.automation_id.trim().is_empty() {
        Some(truncate_chars(e.automation_id.trim(), 28))
    } else if !e.help_text.trim().is_empty() {
        Some(truncate_chars(e.help_text.trim(), 28))
    } else {
        None
    }
}

#[allow(clippy::too_many_arguments)]
fn build_capsule_payload(
    n: usize,
    kind: &str,
    win_title: &str,
    win_class: &str,
    proc_meta: &process::ProcessMetadata,
    primary_el: Option<&UiaElementInfo>,
    sampled_els: &[UiaElementInfo],
    adapter_outputs: &[adapters::AdapterOutput],
    grounding_lines: &[String],
    coord_ctx: Option<(&MonitorSpace, ScreenRect)>,
    screenshot_path: Option<&str>,
) -> CapsulePayload {
    let badge = circled_badge(n);
    let app_display = if !proc_meta.product_name.is_empty() {
        proc_meta.product_name.as_str()
    } else if !proc_meta.exe_name.is_empty() {
        proc_meta.exe_name.as_str()
    } else {
        "App"
    };

    let adapter_badge = adapter_outputs.iter().find_map(|o| o.badge.clone());
    let primary_label = primary_el
        .and_then(resolve_primary_element_label)
        .or_else(|| adapter_badge.clone())
        .unwrap_or_else(|| truncate_chars(win_title, 28));

    let chip_title = format!("{badge} {primary_label}");
    let chip_detail = match &adapter_badge {
        Some(ab) if ab != &primary_label => format!("{app_display} · {ab}"),
        _ => app_display.to_string(),
    };

    let mut lines = Vec::new();
    lines.push(format!(
        "[DSB-PICK #{n} badge={badge} kind={kind} app=\"{}\"]",
        proc_meta.exe_name
    ));
    lines.push(format!("window_title: {}", win_title.trim()));
    lines.push(format!("window_class: {win_class}"));
    lines.push(format!(
        "process: pid={} exe=\"{}\" framework=\"{}\"",
        proc_meta.pid, proc_meta.exe_path, proc_meta.framework_guess
    ));
    if !proc_meta.product_name.is_empty() || !proc_meta.file_version.is_empty() {
        lines.push(format!(
            "version_info: product=\"{}\" desc=\"{}\" ver=\"{}\" company=\"{}\"",
            proc_meta.product_name,
            proc_meta.file_description,
            proc_meta.file_version,
            proc_meta.company_name
        ));
    }
    if !proc_meta.install_dir.is_empty() {
        lines.push(format!("install_dir: {}", proc_meta.install_dir));
    }
    if !proc_meta.cmdline.is_empty() {
        lines.push(format!("cmdline: {}", proc_meta.cmdline));
    }
    if let Some(cwd) = &proc_meta.cwd {
        lines.push(format!("process_cwd: {cwd}"));
    }
    if let Some(src) = &proc_meta.mapped_source_dir {
        lines.push(format!("mapped_source_dir: {src}"));
    }

    if !grounding_lines.is_empty() {
        lines.extend(grounding_lines.iter().cloned());
    }

    if let Some(el) = primary_el {
        lines.push(format!(
            "picked_element: type={} sibling_index={} rect=({},{},{}x{})",
            el.control_type, el.sibling_index, el.rect.x, el.rect.y, el.rect.w, el.rect.h
        ));
        if !el.name.is_empty() {
            lines.push(format!("  name: {}", el.name));
        }
        if !el.automation_id.is_empty() {
            lines.push(format!("  automation_id: {}", el.automation_id));
        }
        if !el.class_name.is_empty() {
            lines.push(format!("  class_name: {}", el.class_name));
        }
        if !el.value.is_empty() {
            lines.push(format!("  value: {}", el.value));
        }
        if !el.help_text.is_empty() {
            lines.push(format!("  help_text: {}", el.help_text));
        }
        if !el.ancestors.is_empty() {
            lines.push(format!(
                "  uia_ancestors (inner->outer): {}",
                el.ancestors.join(" -> ")
            ));
        }
        if !el.text_snippet.is_empty() {
            lines.push(format!(
                "  text_pattern_excerpt:\n{}",
                truncate_chars(&el.text_snippet, 1200)
            ));
        }
    }

    if sampled_els.len() > 1 {
        lines.push(format!("stroke_sampled_elements ({}):", sampled_els.len()));
        for (idx, s) in sampled_els.iter().enumerate() {
            let center_hint = if let Some((mon, win_rect)) = coord_ctx {
                let bb = BboxCoords::compute(s.rect, mon, win_rect);
                format!(
                    " center_px=({},{}) norm1000=({},{})",
                    bb.center.px.0, bb.center.px.1, bb.center.norm1000.0, bb.center.norm1000.1
                )
            } else {
                String::new()
            };
            lines.push(format!(
                "  [{}] {} name=\"{}\" value=\"{}\" id=\"{}\"{}",
                idx + 1,
                s.control_type,
                truncate_chars(&s.name, 80),
                truncate_chars(&s.value, 80),
                truncate_chars(&s.automation_id, 40),
                center_hint,
            ));
        }
    }

    let mut discovered_paths: Vec<String> = Vec::new();
    if let Some(src) = &proc_meta.mapped_source_dir {
        discovered_paths.push(src.clone());
    }

    for ad in adapter_outputs {
        lines.push(format!("adapter[{}]:", ad.adapter));
        for l in &ad.lines {
            lines.push(format!("  {l}"));
        }
        for p in &ad.paths {
            if !discovered_paths.contains(p) {
                discovered_paths.push(p.clone());
            }
        }
    }

    for p in discovered_paths.iter().take(8) {
        if let Some(meta) = inspect_path_metadata(Path::new(p), 1000) {
            append_fs_meta_lines(&mut lines, &meta);
        }
    }

    if let Some(sp) = screenshot_path {
        lines.push(format!(
            "debug_screenshot_path: {sp} (仅当以上信息不足或可疑时再打开查看)"
        ));
    }
    lines.push("[/DSB-PICK]".to_string());

    let preview_body = format!("{} — {}", app_display, truncate_chars(win_title, 120));

    CapsulePayload {
        summary: CapsuleQuoteSummary {
            label: badge,
            title: chip_title,
            detail: chip_detail,
            body: preview_body,
        },
        text: lines.join("\n"),
    }
}

fn append_fs_meta_lines(lines: &mut Vec<String>, meta: &fs_meta::FileEntryBrief) {
    let kind_str = if meta.is_dir { "directory" } else { "file" };
    let approx = if meta.size_complete { "" } else { "+" };
    lines.push(format!(
        "path_meta[{}]: path=\"{}\" size={}{}",
        kind_str,
        meta.path,
        format_human_bytes(meta.size_bytes),
        approx
    ));
    if meta.is_dir {
        lines.push(format!(
            "  counts: files={} dirs={}",
            meta.file_count, meta.dir_count
        ));
        if !meta.children_preview.is_empty() {
            lines.push(format!(
                "  children_first_30: {}",
                meta.children_preview.join(", ")
            ));
        }
    }
    if let Some(gr) = &meta.git_root {
        lines.push(format!(
            "  git: root=\"{gr}\" branch=\"{}\"",
            meta.git_branch.as_deref().unwrap_or("unknown")
        ));
    }
}

fn emit_pick_toast(app: &AppHandle, text: &str) {
    if let Some(ov) = app.get_webview_window("overlay") {
        let _ = ov.emit(
            "pen:toast",
            serde_json::json!({
                "text": text,
                "durationMs": 1600,
            }),
        );
    }
}

fn inject_capsule_into_bar(app: &AppHandle, capsule: &CapsulePayload) {
    let target = app
        .try_state::<AppShared>()
        .and_then(|s| s.config.lock().ok().map(|c| c.output_target))
        .unwrap_or(OutputTarget::Dsh);

    if matches!(
        target,
        OutputTarget::ClipboardOnly | OutputTarget::DshAndClipboard
    ) {
        let copied = grounding::copy_text_to_clipboard(&capsule.text);
        log::info!(
            "Copied structured capsule '{}' to clipboard (ok={copied}, target={:?})",
            capsule.summary.title,
            target
        );
        if target == OutputTarget::ClipboardOnly {
            emit_pick_toast(
                app,
                "已复制坐标胶囊 · 可直接粘贴到 Codex / Claude Code / Antigravity",
            );
            return;
        }
    }

    if let Some(bar_win) = app.get_webview_window("bar") {
        if let Ok(json_arg) = serde_json::to_string(capsule) {
            let script = format!(
                "(function(){{ const b = window.__DSB_BRIDGE__; if (b && typeof b.insertCapsule === 'function') {{ b.insertCapsule({json_arg}); }} }})();"
            );
            let _ = bar_win.eval(&script);
        }
    }
    dispatch_state_event(app, StateEvent::PickInserted);
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        s.to_string()
    } else {
        let head: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{head}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circled_badges_cover_1_to_20_and_fallback_after() {
        assert_eq!(circled_badge(1), "①");
        assert_eq!(circled_badge(2), "②");
        assert_eq!(circled_badge(20), "⑳");
        assert_eq!(circled_badge(21), "#21");
    }

    #[test]
    fn skips_clickthrough_and_passive_nvidia_overlay_windows() {
        assert!(should_skip_overlay_window(
            WS_EX_TRANSPARENT.0 as isize,
            false,
            "Chrome_WidgetWin_1",
            false
        ));
        assert!(should_skip_overlay_window(0, true, "SomeHud", false));
        assert!(should_skip_overlay_window(
            0,
            false,
            "CEF-OSC-WIDGET",
            false
        ));
        // Interactive foreground window without WS_EX_TRANSPARENT is NOT skipped
        assert!(!should_skip_overlay_window(
            0,
            false,
            "Chrome_WidgetWin_1",
            true
        ));
    }

    #[test]
    fn tap_validity_rejects_root_or_nameless_panes() {
        let root_pane = UiaElementInfo {
            name: "Untitled - Notepad".into(),
            control_type: "Window".into(),
            control_type_id: 50032,
            automation_id: String::new(),
            class_name: "Notepad".into(),
            value: String::new(),
            help_text: String::new(),
            text_snippet: String::new(),
            rect: ScreenRect {
                x: 0,
                y: 0,
                w: 800,
                h: 600,
            },
            sibling_index: 0,
            ancestors: Vec::new(),
            is_root_or_empty: true,
            affordance: None,
        };
        assert!(!is_valid_tap_element(&root_pane));

        let valid_btn = UiaElementInfo {
            name: "文件(F)".into(),
            control_type: "MenuItem".into(),
            control_type_id: 50011,
            automation_id: "File".into(),
            class_name: String::new(),
            value: String::new(),
            help_text: String::new(),
            text_snippet: String::new(),
            rect: ScreenRect {
                x: 100,
                y: 120,
                w: 48,
                h: 24,
            },
            sibling_index: 0,
            ancestors: vec!["MenuBar(name=\"应用程序\", id=\"MenuBar\")".into()],
            is_root_or_empty: false,
            affordance: None,
        };
        assert!(is_valid_tap_element(&valid_btn));
    }

    #[test]
    fn classifies_foreground_elements_vs_desktop_and_large_background_panels() {
        let win_rect = ScreenRect {
            x: 0,
            y: 0,
            w: 1600,
            h: 1000,
        };
        let make = |ct: &str, name: &str, w: i32, h: i32, root: bool| UiaElementInfo {
            name: name.into(),
            control_type: ct.into(),
            control_type_id: 0,
            automation_id: String::new(),
            class_name: String::new(),
            value: String::new(),
            help_text: String::new(),
            text_snippet: String::new(),
            rect: ScreenRect { x: 20, y: 20, w, h },
            sibling_index: 0,
            ancestors: Vec::new(),
            is_root_or_empty: root,
            affordance: None,
        };

        // Small button, unnamed 55x55 avatar Image, and Bilibili feed cards -> immediate Element
        assert_eq!(
            classify_hover_target(
                &make("Button", "发送", 80, 36, false),
                win_rect,
                "Chrome_WidgetWin_1"
            ),
            HoverTargetKind::Element
        );
        assert_eq!(
            classify_hover_target(
                &make("Image", "", 55, 55, false),
                win_rect,
                "Chrome_WidgetWin_1"
            ),
            HoverTargetKind::Element
        );
        let mut bili_summary = make("Group", "", 694, 277, false);
        bili_summary.class_name = "dyn-card-opus__summary".into();
        let edge_2k = ScreenRect {
            x: 0,
            y: 0,
            w: 2560,
            h: 1400,
        };
        assert_eq!(
            classify_hover_target(&bili_summary, edge_2k, "Chrome_WidgetWin_1"),
            HoverTargetKind::Element
        );
        let mut bili_card = make("Group", "", 816, 687, false);
        bili_card.class_name = "bili-dyn-item".into();
        assert_eq!(
            classify_hover_target(&bili_card, edge_2k, "Chrome_WidgetWin_1"),
            HoverTargetKind::Element
        );

        // Full-page Document or huge backdrop Pane (>= 55% area) -> Background (requires 0.5s dwell)
        assert_eq!(
            classify_hover_target(
                &make("Document", "Antigravity", 1200, 800, false),
                win_rect,
                "Chrome_WidgetWin_1"
            ),
            HoverTargetKind::Background
        );
        assert_eq!(
            classify_hover_target(
                &make("Pane", "主背景画布", 1400, 900, false),
                win_rect,
                "Chrome_WidgetWin_1"
            ),
            HoverTargetKind::Background
        );

        // Windows desktop wallpaper -> Background; desktop icon (ListItem) -> immediate Element
        assert_eq!(
            classify_hover_target(
                &make("List", "桌面", 2560, 1440, false),
                win_rect,
                "Progman"
            ),
            HoverTargetKind::Background
        );
        assert_eq!(
            classify_hover_target(
                &make("ListItem", "此电脑", 76, 86, false),
                win_rect,
                "WorkerW"
            ),
            HoverTargetKind::Element
        );
    }

    #[test]
    fn ignores_background_selection_in_office_wps_and_canvas_editor_apps() {
        assert!(should_ignore_background_for_app("winword.exe", "OpusApp"));
        assert!(should_ignore_background_for_app("excel.exe", "XLMAIN"));
        assert!(should_ignore_background_for_app(
            "powerpnt.exe",
            "PPTFrameClass"
        ));
        assert!(should_ignore_background_for_app("wps.exe", "Qt5QWindow"));
        assert!(should_ignore_background_for_app("et.exe", "XLMAIN"));
        assert!(should_ignore_background_for_app("wpp.exe", "WPPMain"));
        assert!(should_ignore_background_for_app(
            "Code.exe",
            "Chrome_WidgetWin_1"
        ));
        assert!(should_ignore_background_for_app(
            "photoshop.exe",
            "Photoshop"
        ));
        assert!(should_ignore_background_for_app(
            "figma.exe",
            "Chrome_WidgetWin_1"
        ));
        assert!(should_ignore_background_for_app(
            "obsidian.exe",
            "Chrome_WidgetWin_1"
        ));
        // Desktop & regular apps still allow background picking
        assert!(!should_ignore_background_for_app("explorer.exe", "Progman"));
        assert!(!should_ignore_background_for_app(
            "msedge.exe",
            "Chrome_WidgetWin_1"
        ));
    }

    #[test]
    fn classifies_cad_box_vs_freehand_lasso_trajectories() {
        // Monotonic top-left to bottom-right diagonal -> Box
        let diag_box = vec![
            (100, 100),
            (140, 125),
            (185, 152),
            (230, 180),
            (280, 210),
        ];
        assert_eq!(classify_blue_gesture(&diag_box), BlueGestureKind::Box);

        // Straight horizontal text sweep -> Box
        let horiz_text = vec![(100, 200), (160, 202), (220, 201), (310, 203)];
        assert_eq!(classify_blue_gesture(&horiz_text), BlueGestureKind::Box);

        // Started top-left to bottom-right, then curved back upward into a loop -> Lasso
        let morph_lasso = vec![
            (100, 100),
            (160, 140),
            (240, 200),
            (260, 150),
            (220, 90),
            (140, 85),
        ];
        assert_eq!(classify_blue_gesture(&morph_lasso), BlueGestureKind::Lasso);

        // Closed circle -> Lasso
        let circle: Vec<(i32, i32)> = (0..=16)
            .map(|i| {
                let a = (i as f64) * std::f64::consts::TAU / 16.0;
                (200 + (80.0 * a.cos()) as i32, 200 + (80.0 * a.sin()) as i32)
            })
            .collect();
        assert_eq!(classify_blue_gesture(&circle), BlueGestureKind::Lasso);
    }

    #[test]
    fn lasso_selects_touched_and_enclosed_rects_while_dodging_outside() {
        // Square-ish lasso around (100..300, 100..300)
        let lasso = vec![
            (100, 100),
            (300, 100),
            (300, 300),
            (100, 300),
            (105, 105),
        ];
        let enclosed = ScreenRect {
            x: 160,
            y: 160,
            w: 60,
            h: 40,
        };
        let touched = ScreenRect {
            x: 285,
            y: 180,
            w: 50,
            h: 30,
        };
        let dodged = ScreenRect {
            x: 340,
            y: 180,
            w: 50,
            h: 30,
        };
        assert!(polyline_touches_or_encloses_rect(&lasso, enclosed));
        assert!(polyline_touches_or_encloses_rect(&lasso, touched));
        assert!(!polyline_touches_or_encloses_rect(&lasso, dodged));
    }

    #[test]
    fn blue_selection_strictly_excludes_desktop_and_large_background_panels() {
        let win_rect = ScreenRect {
            x: 0,
            y: 0,
            w: 1000,
            h: 800,
        };
        let desktop_pane = UiaElementInfo {
            name: "FolderView".into(),
            control_type: "List".into(),
            control_type_id: 50008,
            automation_id: "1".into(),
            class_name: "SysListView32".into(),
            value: String::new(),
            help_text: String::new(),
            text_snippet: String::new(),
            rect: win_rect,
            sibling_index: 0,
            ancestors: Vec::new(),
            is_root_or_empty: false,
            affordance: None,
        };
        assert!(!is_strict_foreground_for_blue(
            &desktop_pane,
            win_rect,
            "WorkerW"
        ));

        let desktop_icon = UiaElementInfo {
            name: "项目文档.pdf".into(),
            control_type: "ListItem".into(),
            control_type_id: 50007,
            automation_id: String::new(),
            class_name: String::new(),
            value: String::new(),
            help_text: String::new(),
            text_snippet: String::new(),
            rect: ScreenRect {
                x: 40,
                y: 60,
                w: 72,
                h: 84,
            },
            sibling_index: 1,
            ancestors: Vec::new(),
            is_root_or_empty: false,
            affordance: None,
        };
        assert!(is_strict_foreground_for_blue(
            &desktop_icon,
            win_rect,
            "WorkerW"
        ));

        let big_bg_pane = UiaElementInfo {
            name: "MainContent".into(),
            control_type: "Pane".into(),
            control_type_id: 50033,
            automation_id: "main".into(),
            class_name: "Chrome_RenderWidgetHostHWND".into(),
            value: String::new(),
            help_text: String::new(),
            text_snippet: String::new(),
            rect: ScreenRect {
                x: 0,
                y: 80,
                w: 1000,
                h: 720,
            },
            sibling_index: 0,
            ancestors: Vec::new(),
            is_root_or_empty: false,
            affordance: None,
        };
        assert!(!is_strict_foreground_for_blue(
            &big_bg_pane,
            win_rect,
            "Chrome_WidgetWin_1"
        ));
    }

    #[test]
    fn builds_blue_element_group_capsule_with_expected_title() {
        let mk_btn = |name: &str, x: i32| UiaElementInfo {
            name: name.into(),
            control_type: "Button".into(),
            control_type_id: 50000,
            automation_id: format!("btn_{x}"),
            class_name: "Button".into(),
            value: String::new(),
            help_text: String::new(),
            text_snippet: String::new(),
            rect: ScreenRect {
                x,
                y: 120,
                w: 80,
                h: 32,
            },
            sibling_index: 0,
            ancestors: Vec::new(),
            is_root_or_empty: false,
            affordance: None,
        };
        let proc_meta = process::ProcessMetadata {
            pid: 1234,
            exe_name: "explorer.exe".into(),
            exe_path: r"C:\Windows\explorer.exe".into(),
            install_dir: r"C:\Windows".into(),
            product_name: "文件资源管理器".into(),
            file_description: String::new(),
            file_version: String::new(),
            company_name: String::new(),
            cmdline: String::new(),
            cwd: None,
            framework_guess: "Win32".into(),
            mapped_source_dir: None,
        };
        let els = vec![mk_btn("新建", 40), mk_btn("删除", 140), mk_btn("重命名", 240)];
        let cap = build_blue_element_group_capsule(
            1,
            BlueGestureKind::Lasso,
            "项目目录",
            "CabinetWClass",
            &proc_meta,
            &els,
            &[],
            &[],
        );
        assert_eq!(cap.summary.title, "① 元素组 (3项)");
        assert!(cap.summary.detail.contains("蓝线圈选"));
        assert!(cap.text.contains("kind=element_group"));
        assert!(cap.text.contains("count=3"));
    }

    #[test]
    fn tap_pick_on_unnamed_icon_promotes_interactive_ancestor_button_name() {
        let el = UiaElementInfo {
            name: String::new(),
            control_type: "Image".into(),
            control_type_id: ControlType::Image as i32,
            automation_id: String::new(),
            class_name: String::new(),
            value: String::new(),
            help_text: String::new(),
            text_snippet: String::new(),
            rect: ScreenRect {
                x: 1720,
                y: 885,
                w: 25,
                h: 26,
            },
            sibling_index: 0,
            ancestors: vec![
                r#"Button(name="固定展开", id="")"#.into(),
                r#"Group(name="", id="")"#.into(),
            ],
            is_root_or_empty: false,
            affordance: None,
        };
        assert_eq!(
            resolve_primary_element_label(&el).as_deref(),
            Some("固定展开")
        );
    }

    #[test]
    fn coalesces_streaming_inline_text_runs_without_merging_lines_or_buttons() {
        let mk_text = |name: &str, x: i32, y: i32, w: i32| UiaElementInfo {
            name: name.into(),
            control_type: "Text".into(),
            control_type_id: ControlType::Text as i32,
            automation_id: String::new(),
            class_name: String::new(),
            value: String::new(),
            help_text: String::new(),
            text_snippet: String::new(),
            rect: ScreenRect { x, y, w, h: 23 },
            sibling_index: 0,
            ancestors: Vec::new(),
            is_root_or_empty: false,
            affordance: None,
        };
        let mk_btn = |name: &str, x: i32, y: i32, w: i32| UiaElementInfo {
            control_type: "Button".into(),
            control_type_id: ControlType::Button as i32,
            ..mk_text(name, x, y, w)
        };

        let items = vec![
            mk_text("现在", 970, 963, 36),
            mk_text("双击", 1005, 963, 41),
            mk_text("Ctrl", 1050, 965, 39),
            mk_text("点", 1097, 963, 19),
            mk_text("开", 1115, 963, 22),
            mk_text("！", 1137, 963, 18),
            // Same line but interactive button -> must stay separate
            mk_btn("刷新", 1160, 963, 44),
            // Same line far-apart column (> 20px gap) -> must stay separate
            mk_text("2分钟前", 1260, 963, 56),
            // Next line -> must stay separate
            mk_text("第二行", 970, 995, 54),
        ];

        let out = coalesce_inline_text_runs(items);
        assert_eq!(out.len(), 4);
        assert_eq!(out[0].name, "现在双击 Ctrl 点开！");
        assert_eq!(out[0].rect.x, 970);
        assert_eq!(out[0].rect.w, 1137 + 18 - 970);
        assert_eq!(out[1].control_type, "Button");
        assert_eq!(out[1].name, "刷新");
        assert_eq!(out[2].name, "2分钟前");
        assert_eq!(out[3].name, "第二行");
    }
}
