pub mod adapters;
pub mod fs_meta;
pub mod process;

use crate::captures::{
    capture_target_window_bitblt, save_annotated_capture, RawScreenCapture, ScreenRect,
};
use crate::pick::adapters::office::{self, OfficeAppKind};
use crate::pick::adapters::{collect_deep_adapter_outputs, AdapterContext};
use crate::pick::fs_meta::{format_human_bytes, inspect_path_metadata};
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
use uiautomation::types::{ControlType, Handle as UiaHandle, Point as UiaPoint, UIProperty};
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
    if let Ok(direct) = automation.element_from_point(UiaPoint::new(x, y)) {
        let el_pid = direct.get_process_id().unwrap_or(0);
        let el_name = direct.get_name().unwrap_or_default();
        if el_name != "Anything-to-Prompt Overlay" {
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
                let display = if !info.name.is_empty() {
                    truncate_chars(&info.name, 26)
                } else if !info.value.is_empty() && info.value != "[PASSWORD_REDACTED]" {
                    truncate_chars(&info.value, 26)
                } else if !info.automation_id.is_empty() {
                    truncate_chars(&info.automation_id, 26)
                } else if !info.class_name.is_empty() {
                    truncate_chars(&info.class_name, 26)
                } else {
                    String::new()
                };
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

    if include_ancestors_and_text {
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

pub fn execute_stroke_pick(
    app: &AppHandle,
    points: Vec<(i32, i32)>,
    pre_capture: Option<RawScreenCapture>,
) {
    if points.len() < 2 {
        return;
    }
    let Some(shared) = app.try_state::<AppShared>() else {
        return;
    };

    // Specialized Office / WPS drag handling:
    // - Word / WPS Writer: I-beam cursor text selection (NO red stroke)
    // - Excel / WPS Spreadsheets: rectangular cell range box selection (NO red stroke)
    // - PowerPoint / WPS Presentation: intentionally falls through to keep red freehand stroke unchanged!
    if try_execute_office_drag_pick(app, &shared, &points, pre_capture.as_ref()) {
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
        w: (max_x - min_x).max(24),
        h: (max_y - min_y).max(24),
    };
    let center_x = bbox.x + bbox.w / 2;
    let center_y = bbox.y + bbox.h / 2;

    let (root_hwnd, target_pid, win_title, win_class) = resolve_root_window_at(center_x, center_y);
    if target_pid == 0 {
        return;
    }

    let cfg = shared.config.lock().map(|c| c.clone()).unwrap_or_default();
    let n = shared.allocate_pick_number();

    if let Some(ov) = app.get_webview_window("overlay") {
        let pts_json: Vec<serde_json::Value> = points
            .iter()
            .map(|&(px, py)| serde_json::json!({ "x": px, "y": py }))
            .collect();
        let _ = ov.emit(
            "pen:picked",
            serde_json::json!({
                "n": n,
                "strokes": pts_json,
                "rect": bbox,
            }),
        );
    }

    let raw_cap =
        pre_capture.or_else(|| capture_target_window_bitblt(root_hwnd, center_x, center_y).ok());
    let screenshot_path = raw_cap.as_ref().and_then(|raw| {
        save_annotated_capture(app, raw, Some(bbox), &points, n, &cfg)
            .ok()
            .map(|p| p.to_string_lossy().to_string())
    });

    // Sample <= 6x6 UIA grid inside stroke bounding box (deduped, cap 30)
    let sampled = sample_uia_grid_in_rect(bbox, target_pid, 6, 30);

    let proc_meta = inspect_process(target_pid, &cfg.app_source_map);
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
        1500,
    );

    let capsule = build_capsule_payload(
        n,
        "stroke",
        &win_title,
        &win_class,
        &proc_meta,
        sampled.first(),
        &sampled,
        &adapter_outputs,
        screenshot_path.as_deref(),
    );

    log::info!(
        "Committed Stroke Pick #{n}: bbox=({},{},{}x{}) sampled={} screenshot={:?}",
        bbox.x,
        bbox.y,
        bbox.w,
        bbox.h,
        sampled.len(),
        screenshot_path
    );
    inject_capsule_into_bar(app, &capsule);
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
            let Some(ex) = office::pick_excel_cell_com(&shared.broker, x, y, hit.dpi_scale) else {
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
            let Some(ppt) = office::pick_ppt_element_com(&shared.broker, x, y, hit.dpi_scale)
            else {
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
        .and_then(|e| {
            if !e.name.is_empty() {
                Some(truncate_chars(&e.name, 28))
            } else if !e.value.is_empty() && e.value != "[PASSWORD_REDACTED]" {
                Some(truncate_chars(&e.value, 28))
            } else if !e.automation_id.is_empty() {
                Some(truncate_chars(&e.automation_id, 28))
            } else {
                None
            }
        })
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
            lines.push(format!(
                "  [{}] {} name=\"{}\" value=\"{}\" id=\"{}\"",
                idx + 1,
                s.control_type,
                truncate_chars(&s.name, 80),
                truncate_chars(&s.value, 80),
                truncate_chars(&s.automation_id, 40),
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
}
