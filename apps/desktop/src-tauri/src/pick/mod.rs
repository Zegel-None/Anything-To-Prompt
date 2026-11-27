mod blue_selection;
mod office_flow;
mod solidworks_flow;

pub use blue_selection::*;
use office_flow::*;
pub use solidworks_flow::*;

pub mod adapter_plugins;
pub mod adapters;
pub mod brush_scan;
pub mod enricher;
pub mod fs_meta;
pub mod grounding;
pub mod hitmap;
pub mod hover_diff;
pub mod probe;
pub mod process;
pub mod profile;
pub mod rebrush;
pub mod scout;
pub mod self_heal;
pub mod vision;
pub mod win32_controls;

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
    GetWindowLongPtrW, GetWindowRect, GetWindowThreadProcessId, IsIconic,
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

pub use crate::win32_util::window_title_text as get_window_title;

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
    // P0-1: Never touch Windows UIAutomation on SolidWorks (FeatureManager/Ribbon UIA hang hazard).
    if target_pid != 0 && solidworks::is_solidworks_exe(&office::fast_exe_basename_for_pid(target_pid)) {
        return None;
    }
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

pub(crate) fn get_window_screen_rect_pub(hwnd: HWND) -> ScreenRect {
    get_window_screen_rect(hwnd)
}

pub(crate) fn inspect_uia_element_with_node_at(
    automation: &UIAutomation,
    walker: &UITreeWalker,
    root_hwnd: HWND,
    target_pid: u32,
    x: i32,
    y: i32,
) -> Option<(UIElement, UiaElementInfo)> {
    let el = find_target_uia_element(automation, walker, root_hwnd, target_pid, x, y)?;
    let info = extract_uia_element_info(&el, walker, false);
    Some((el, info))
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

/// Returns true ONLY for SolidWorks and professional CAD apps where hovering/clicking the
/// empty 3D/2D model canvas must NEVER trigger the 0.5s background dwell ring or select the
/// entire window background. All other apps (Office/WPS, IDEs, browsers, editors, etc.) keep
/// normal background/canvas behavior.
pub fn should_ignore_background_for_app(exe_name: &str, _win_class: &str) -> bool {
    let exe = exe_name.trim().to_ascii_lowercase();
    matches!(
        exe.as_str(),
        "sldworks.exe"
            | "acad.exe"
            | "zwcad.exe"
            | "gstarcad.exe"
            | "bricscad.exe"
            | "ugraf.exe"
            | "xtop.exe"
            | "inventor.exe"
            | "fusion360.exe"
            | "rhino.exe"
            | "cnext.exe"
            | "freecad.exe"
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

pub(crate) fn resolve_child_hwnd_at(root_hwnd: HWND, x: i32, y: i32) -> (HWND, String, ScreenRect) {
    use windows::Win32::Graphics::Gdi::ScreenToClient;
    use windows::Win32::UI::WindowsAndMessaging::{
        ChildWindowFromPointEx, GetClassNameW, CWP_SKIPINVISIBLE, CWP_SKIPTRANSPARENT,
    };
    let mut target = unsafe { WindowFromPoint(POINT { x, y }) };
    if target.0.is_null() {
        target = root_hwnd;
    }
    for _ in 0..6 {
        let mut pt = POINT { x, y };
        if !unsafe { ScreenToClient(target, &mut pt) }.as_bool() {
            break;
        }
        let next = unsafe {
            ChildWindowFromPointEx(target, pt, CWP_SKIPINVISIBLE | CWP_SKIPTRANSPARENT)
        };
        if next.0.is_null() || next == target {
            break;
        }
        target = next;
    }
    let mut cls_buf = [0u16; 128];
    let clen = unsafe { GetClassNameW(target, &mut cls_buf) }.max(0) as usize;
    let class_name = String::from_utf16_lossy(&cls_buf[..clen]);
    let rect = get_window_screen_rect(target);
    (target, class_name, rect)
}

/// Universal non-UIA L2 (MSAA container Hit-Map) -> L3 (Static visual Hit-Map) lookup for ANY app.
fn inspect_non_uia_hitmap_at(
    root_hwnd: HWND,
    x: i32,
    y: i32,
    prof: &profile::AppUiProfile,
) -> Option<(hitmap::HitMapEntry, HWND, String)> {
    let (child_hwnd, child_class, child_rect) = resolve_child_hwnd_at(root_hwnd, x, y);
    if child_rect.w < 16 || child_rect.h < 14 {
        return None;
    }
    let cls_lower = child_class.to_ascii_lowercase();
    let is_popup = solidworks::is_popup_menu_hwnd(child_hwnd, root_hwnd, &cls_lower);
    if !is_popup
        && prof
            .ignore_canvas_classes
            .iter()
            .any(|c| !c.is_empty() && cls_lower.contains(c))
    {
        return None;
    }
    let forced_layout = prof
        .learned_regions
        .iter()
        .find(|r| r.class_name.eq_ignore_ascii_case(&cls_lower))
        .map(|r| hitmap::ContainerLayoutKind::parse(&r.layout_kind))
        .unwrap_or(hitmap::ContainerLayoutKind::Auto);
    // Always check woken ExternalBoxProvider (OmniParser / Hover-Diff) FIRST so Dose 1 results
    // are NEVER shadowed by coarse MsaaHitMap layers!
    if let Some(entry) = hitmap::lookup_cached_any_source(child_hwnd.0 as isize, x, y) {
        return Some((entry, child_hwnd, child_class));
    }
    for layer in &prof.preferred_layers {
        match layer {
            profile::GroundingLayer::MsaaHitMap => {
                if let hitmap::MsaaLookup::Hit(entry) =
                    hitmap::lookup_or_build_msaa(child_hwnd, child_rect, x, y)
                {
                    return Some((entry, child_hwnd, child_class));
                }
            }
            profile::GroundingLayer::StaticVisualHitMap => {
                if let Some(entry) = hitmap::lookup_or_build_static_visual_with_hint(
                    child_hwnd,
                    child_rect,
                    &child_class,
                    is_popup,
                    forced_layout,
                    x,
                    y,
                ) {
                    return Some((entry, child_hwnd, child_class));
                }
            }
            profile::GroundingLayer::Uia => {}
        }
    }
    None
}

pub fn inspect_hover_at_with_uia(
    uia: Option<(&UIAutomation, &UITreeWalker)>,
    x: i32,
    y: i32,
    own_pid: u32,
) -> HoverInspection {
    inspect_hover_at_with_context(None, uia, x, y, own_pid)
}

pub fn inspect_hover_at_with_context(
    app: Option<&AppHandle>,
    uia: Option<(&UIAutomation, &UITreeWalker)>,
    x: i32,
    y: i32,
    own_pid: u32,
) -> HoverInspection {
    let cursor = serde_json::json!({ "x": x, "y": y });

    // Priority 0: If Dose 2 revealed an organic photo cutout or expanded submenu item at `(x, y)`,
    // highlight it directly with zero background dwell ring or overlapping rectangle collision!
    if let Some(rev) = brush_scan::find_revealed_item_at(x, y) {
        let lbl = if rev.label.trim().is_empty() {
            rev.role.clone()
        } else {
            truncate_chars(&rev.label, 32)
        };
        return HoverInspection {
            ok: true,
            kind: HoverTargetKind::Element,
            target_key: rev.key,
            // For organic photo polygons, overlay.js already renders the cyan silhouette — suppress
            // the redundant outer blue rectangle while keeping the crisp semantic label!
            rect: if rev.polygon.is_some() {
                None
            } else {
                Some(rev.rect)
            },
            cursor,
            label: lbl,
        };
    }

    let (root_hwnd, target_pid, win_title, win_class) = resolve_root_window_at(x, y);
    if target_pid == 0 || (own_pid != 0 && target_pid == own_pid) {
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
    let prof = profile::get_effective_profile(&target_exe);
    if let Some(a) = app {
        if !target_exe.is_empty() && !profile::is_known_profile(&target_exe) {
            self_heal::maybe_trigger_first_time_onboarding(a, root_hwnd, &target_exe, (x, y));
        }
    }
    let ignore_bg =
        prof.ignore_background || should_ignore_background_for_app(&target_exe, &win_class);

    // P0-1 + CAD UX: Never run UIA hover traversal or 0.5s background dwell ring on SolidWorks.
    // Uses universal L2 MSAA Hit-Map -> L3 Static Visual Hit-Map + Win32 controls (<0.3ms cached)
    // + Stage B passive Tooltip Catcher / cached OCR enrichment!
    if solidworks::is_solidworks_exe(&target_exe) {
        if let Some(ctrl) = solidworks::inspect_win32_ui_control_at(root_hwnd, x, y, false) {
            self_heal::record_hover_stability(app, &target_exe, (x, y), ctrl.rect);
            let enriched = enricher::enrich_box_semantics(
                HWND(ctrl.hwnd_raw as *mut std::ffi::c_void),
                target_pid,
                &target_exe,
                ctrl.rect,
                &ctrl.text,
                &ctrl.description,
                &ctrl.role,
                None,
                false,
            );
            let display = if !enriched.label.is_empty() {
                enriched.label
            } else {
                ctrl.display_label()
            };
            let key = format!(
                "sw-ui:{}:{}:{}:{}:{}",
                ctrl.hwnd_raw, ctrl.rect.x, ctrl.rect.y, ctrl.rect.w, ctrl.rect.h
            );
            return HoverInspection {
                ok: true,
                kind: HoverTargetKind::Element,
                target_key: key,
                rect: Some(ctrl.rect),
                cursor,
                label: truncate_chars(&display, 32),
            };
        }
        return HoverInspection {
            ok: false,
            kind: HoverTargetKind::None,
            target_key: String::new(),
            rect: None,
            cursor,
            label: String::new(),
        };
    }

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

    // For UIA-forbidden or Hit-Map-first apps (Blender, RDP/ToDesk/Sunlogin, AutoCAD, Photoshop,
    // or auto-healed slow legacy apps), run universal L2 MSAA -> L3 Static Visual Hit-Map directly!
    let prefer_hitmap_first = prof.uia_policy == profile::UiaPolicy::Forbidden
        || !matches!(
            prof.preferred_layers.first(),
            Some(profile::GroundingLayer::Uia)
        );
    if prefer_hitmap_first {
        if let Some((entry, child_hwnd, _cls)) = inspect_non_uia_hitmap_at(root_hwnd, x, y, &prof) {
            self_heal::record_hover_stability(app, &target_exe, (x, y), entry.rect);
            let enriched = enricher::enrich_box_semantics(
                child_hwnd,
                target_pid,
                &target_exe,
                entry.rect,
                &entry.label,
                &entry.description,
                &entry.role,
                None,
                false,
            );
            let label = if enriched.label.is_empty() {
                enriched.role
            } else {
                truncate_chars(&enriched.label, 32)
            };
            let target_key = format!(
                "hm:{}:{}:{}:{}:{}",
                child_hwnd.0 as isize, entry.rect.x, entry.rect.y, entry.rect.w, entry.rect.h
            );
            return HoverInspection {
                ok: true,
                kind: HoverTargetKind::Element,
                target_key,
                rect: Some(entry.rect),
                cursor,
                label,
            };
        }
        if prof.uia_policy == profile::UiaPolicy::Forbidden {
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

    let uia_t0 = std::time::Instant::now();
    let uia_info = uia.and_then(|(automation, walker)| {
        let el = find_target_uia_element(automation, walker, root_hwnd, target_pid, x, y)?;
        Some(extract_uia_element_info(&el, walker, false))
    });
    let uia_elapsed_ms = uia_t0.elapsed().as_millis() as u64;
    if uia.is_some() {
        self_heal::record_uia_latency(app, &target_exe, uia_elapsed_ms, prof.uia_timeout_ms, (x, y));
    }

    if let Some(ref info) = uia_info {
        match classify_hover_target(info, win_rect, &win_class) {
            HoverTargetKind::Element => {
                // If Dose 1 Shake-to-Wake found a much finer button (>= 4x smaller) inside a coarse
                // custom UIA Group/Pane/Custom container, prefer the finer woken button!
                if info.rect.w > 240 || info.rect.h > 140 {
                    let (child_hwnd, _, _) = resolve_child_hwnd_at(root_hwnd, x, y);
                    if let Some(ext) = hitmap::get_cached_entries(
                        child_hwnd.0 as isize,
                        hitmap::HitMapSource::ExternalBoxProvider,
                    )
                    .and_then(|v| hitmap::find_entry_at(&v, x, y))
                    {
                        let uia_area = (info.rect.w.max(1) as i64) * (info.rect.h.max(1) as i64);
                        let ext_area = (ext.rect.w.max(1) as i64) * (ext.rect.h.max(1) as i64);
                        if uia_area >= ext_area * 4 {
                            let enriched = enricher::enrich_box_semantics(
                                child_hwnd,
                                target_pid,
                                &target_exe,
                                ext.rect,
                                &ext.label,
                                &ext.description,
                                &ext.role,
                                None,
                                false,
                            );
                            let lbl = if enriched.label.is_empty() {
                                enriched.role
                            } else {
                                truncate_chars(&enriched.label, 32)
                            };
                            return HoverInspection {
                                ok: true,
                                kind: HoverTargetKind::Element,
                                target_key: format!(
                                    "hm:{}:{}:{}:{}:{}",
                                    child_hwnd.0 as isize,
                                    ext.rect.x,
                                    ext.rect.y,
                                    ext.rect.w,
                                    ext.rect.h
                                ),
                                rect: Some(ext.rect),
                                cursor,
                                label: lbl,
                            };
                        }
                    }
                }
                self_heal::record_hover_box(app, &target_exe, info.rect, win_rect, false, (x, y));
                self_heal::record_hover_stability(app, &target_exe, (x, y), info.rect);
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
            HoverTargetKind::Background => {
                // Step down to cached Dose 1 / Scheme B OmniParser / MSAA / StaticVisual Hit-Map
                // when UIA only sees a monolithic pane or custom Canvas over a toolbar!
                let (child_hwnd, _, _) = resolve_child_hwnd_at(root_hwnd, x, y);
                if let Some(entry) = hitmap::lookup_cached_any_source(child_hwnd.0 as isize, x, y) {
                    let enriched = enricher::enrich_box_semantics(
                        child_hwnd,
                        target_pid,
                        &target_exe,
                        entry.rect,
                        &entry.label,
                        &entry.description,
                        &entry.role,
                        None,
                        false,
                    );
                    let label = if enriched.label.is_empty() {
                        enriched.role
                    } else {
                        truncate_chars(&enriched.label, 32)
                    };
                    return HoverInspection {
                        ok: true,
                        kind: HoverTargetKind::Element,
                        target_key: format!(
                            "hm:{}:{}:{}:{}:{}",
                            child_hwnd.0 as isize,
                            entry.rect.x,
                            entry.rect.y,
                            entry.rect.w,
                            entry.rect.h
                        ),
                        rect: Some(entry.rect),
                        cursor,
                        label,
                    };
                }
                if let Some((entry, child_hwnd, _cls)) =
                    inspect_non_uia_hitmap_at(root_hwnd, x, y, &prof)
                {
                    let enriched = enricher::enrich_box_semantics(
                        child_hwnd,
                        target_pid,
                        &target_exe,
                        entry.rect,
                        &entry.label,
                        &entry.description,
                        &entry.role,
                        None,
                        false,
                    );
                    let label = if enriched.label.is_empty() {
                        enriched.role
                    } else {
                        truncate_chars(&enriched.label, 32)
                    };
                    return HoverInspection {
                        ok: true,
                        kind: HoverTargetKind::Element,
                        target_key: format!(
                            "hm:{}:{}:{}:{}:{}",
                            child_hwnd.0 as isize,
                            entry.rect.x,
                            entry.rect.y,
                            entry.rect.w,
                            entry.rect.h
                        ),
                        rect: Some(entry.rect),
                        cursor,
                        label,
                    };
                }
                if !ignore_bg {
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
            }
            HoverTargetKind::None => {}
        }
    } else if let Some((entry, child_hwnd, _cls)) =
        inspect_non_uia_hitmap_at(root_hwnd, x, y, &prof)
    {
        let enriched = enricher::enrich_box_semantics(
            child_hwnd,
            target_pid,
            &target_exe,
            entry.rect,
            &entry.label,
            &entry.description,
            &entry.role,
            None,
            false,
        );
        let label = if enriched.label.is_empty() {
            enriched.role
        } else {
            truncate_chars(&enriched.label, 32)
        };
        return HoverInspection {
            ok: true,
            kind: HoverTargetKind::Element,
            target_key: format!(
                "hm:{}:{}:{}:{}:{}",
                child_hwnd.0 as isize, entry.rect.x, entry.rect.y, entry.rect.w, entry.rect.h
            ),
            rect: Some(entry.rect),
            cursor,
            label,
        };
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

    // Priority 0: If the user clicked a Dose 2 revealed item (organic photo cutout, expanded submenu item,
    // or VLM-corrected control), pick it FIRST before Office/SolidWorks tap handlers can hijack it!
    if let Some(rev) = brush_scan::find_revealed_item_at(x, y) {
        let (root_hwnd, target_pid, win_title, _win_class) = resolve_root_window_at(x, y);
        let cfg = shared.config.lock().map(|c| c.clone()).unwrap_or_default();
        let n = shared.allocate_pick_number();
        let badge = circled_badge(n);
        if let Some(ov) = app.get_webview_window("overlay") {
            let _ = ov.emit("pen:picked", serde_json::json!({ "n": n, "rect": rev.rect }));
        }
        let owned_cap;
        let raw_ref = match pre_capture.as_ref() {
            Some(r) => Some(r),
            None => {
                owned_cap = capture_target_window_bitblt(root_hwnd, x, y).ok();
                owned_cap.as_ref()
            }
        };
        let screenshot_path = raw_ref.and_then(|raw| {
            save_annotated_capture(app, raw, Some(rev.rect), &[], n, &cfg)
                .ok()
                .map(|p| p.to_string_lossy().to_string())
        });
        let target_exe = office::fast_exe_basename_for_pid(target_pid);
        let lbl = if rev.label.trim().is_empty() {
            rev.role.clone()
        } else {
            rev.label.clone()
        };
        let (kind_tag, detail_tag) = match rev.semantic_kind.as_str() {
            "photo_object" => ("photo_semantic_cutout", "智能语义抠图"),
            "menu_item" => ("expanded_submenu_item", "展开子菜单项"),
            _ => ("vlm_corrected_ui", "视觉模型纠偏控件"),
        };
        let mut lines = vec![
            format!("[DSB-PICK #{n} | kind={kind_tag}]"),
            format!(
                "应用: {} | 主体: label=\"{}\" role={} rect=({},{},{}x{})",
                target_exe, lbl, rev.role, rev.rect.x, rev.rect.y, rev.rect.w, rev.rect.h
            ),
        ];
        if let Some(poly) = rev.polygon.as_ref() {
            lines.push(format!("polygon_vertices: {}", poly.len()));
        }
        if !win_title.trim().is_empty() {
            lines.push(format!("window: \"{}\"", win_title.trim()));
        }
        if let Some(sp) = screenshot_path.as_deref() {
            lines.push(format!("screenshot_path: {sp}"));
        }
        lines.push("[/DSB-PICK]".to_string());
        let cap = CapsulePayload {
            summary: CapsuleQuoteSummary {
                label: badge.clone(),
                title: truncate_chars(&format!("{badge} {lbl}"), 44),
                detail: truncate_chars(detail_tag, 44),
                body: truncate_chars(&format!("{lbl} ({}x{})", rev.rect.w, rev.rect.h), 110),
            },
            text: lines.join("\n"),
        };
        inject_capsule_into_bar_with_meta(app, &cap, None, None);
        return;
    }

    // External community plugin `tap_override` hook (allows third-party CAD/3D/DAW plugins
    // to take over single-click entity picking cleanly without modifying Rust!)
    if try_execute_external_plugin_tap_override(app, &shared, x, y, pre_capture.as_ref()) {
        return;
    }

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
    // P0-1 + CAD UX: Never fall through to UIA or background pick on SolidWorks.
    if solidworks::is_solidworks_exe(&target_exe) {
        return;
    }
    let prof = profile::get_effective_profile(&target_exe);
    let ignore_bg =
        prof.ignore_background || should_ignore_background_for_app(&target_exe, &win_class);

    // Universal L2/L3 Hit-Map tap pick for UIA-forbidden / Hit-Map-first apps (AutoCAD, Blender,
    // Photoshop, RDP/ToDesk/Sunlogin, or auto-healed legacy apps) with Stage B semantic enrichment
    // (Native MSAA/Win32 + Passive Tooltip Catcher + 10ms built-in Windows OCR)!
    let prefer_hitmap_first = prof.uia_policy == profile::UiaPolicy::Forbidden
        || !matches!(
            prof.preferred_layers.first(),
            Some(profile::GroundingLayer::Uia)
        );
    let mut picked_hitmap: Option<(hitmap::HitMapEntry, HWND, String)> = None;
    if prefer_hitmap_first {
        picked_hitmap = inspect_non_uia_hitmap_at(root_hwnd, x, y, &prof);
    }

    let uia_info = if picked_hitmap.is_none() && prof.uia_policy != profile::UiaPolicy::Forbidden {
        UIAutomation::new().ok().and_then(|auto| {
            let walker = auto.get_control_view_walker().ok()?;
            let el = find_target_uia_element(&auto, &walker, root_hwnd, target_pid, x, y)?;
            Some(extract_uia_element_info(&el, &walker, true))
        })
    } else {
        None
    };

    // If UIA missed (returned None or Background on a Canvas/WebGL/custom button), check whether
    // Dose 1 Shake-to-Wake already cached a visually-located button at `(x, y)`!
    if picked_hitmap.is_none() {
        let uia_is_element = uia_info
            .as_ref()
            .is_some_and(|i| classify_hover_target(i, win_rect, &win_class) == HoverTargetKind::Element);
        if !uia_is_element {
            let (child_hwnd, child_class, _) = resolve_child_hwnd_at(root_hwnd, x, y);
            if let Some(entry) = hitmap::lookup_cached_any_source(child_hwnd.0 as isize, x, y) {
                picked_hitmap = Some((entry, child_hwnd, child_class));
            }
        }
    }

    if let Some((entry, child_hwnd, child_class)) = picked_hitmap {
        let cfg = shared.config.lock().map(|c| c.clone()).unwrap_or_default();
        let n = shared.allocate_pick_number();
        let badge = circled_badge(n);
        if let Some(ov) = app.get_webview_window("overlay") {
            let _ = ov.emit("pen:picked", serde_json::json!({ "n": n, "rect": entry.rect }));
        }
        let owned_cap;
        let raw_ref = match pre_capture.as_ref() {
            Some(r) => Some(r),
            None => {
                owned_cap = capture_target_window_bitblt(root_hwnd, x, y).ok();
                owned_cap.as_ref()
            }
        };
        let screenshot_path = raw_ref.and_then(|raw| {
            save_annotated_capture(app, raw, Some(entry.rect), &[], n, &cfg)
                .ok()
                .map(|p| p.to_string_lossy().to_string())
        });
        // Stage B fast enrichment (Native MSAA/Win32 + live Tooltip Catcher + session cache)
        let fast_enriched = enricher::enrich_box_semantics(
            child_hwnd,
            target_pid,
            &target_exe,
            entry.rect,
            &entry.label,
            &entry.description,
            &entry.role,
            None,
            false,
        );
        let needs_ocr = fast_enriched.label.trim().is_empty() && raw_ref.is_some();
        let cap_id = format!("cap-hm-{n}-{}", profile::now_unix_secs());
        let initial_label = if needs_ocr {
            "按钮 · 识别中…".to_string()
        } else if fast_enriched.label.trim().is_empty() {
            fast_enriched.role.clone()
        } else {
            fast_enriched.label.clone()
        };
        let app_label = if prof.display_name.is_empty() {
            target_exe.clone()
        } else {
            prof.display_name.clone()
        };
        let build_hm_capsule = |lbl: &str, desc: &str| -> CapsulePayload {
            let mut lines = vec![
                format!("[DSB-PICK #{n} | kind=ui_hitmap]"),
                format!(
                    "应用: {} ({}) | 控件: role={} label=\"{}\" class=\"{}\" rect=({},{},{}x{})",
                    app_label,
                    target_exe,
                    fast_enriched.role,
                    lbl,
                    child_class,
                    entry.rect.x,
                    entry.rect.y,
                    entry.rect.w,
                    entry.rect.h
                ),
            ];
            if !desc.is_empty() {
                lines.push(format!("description: \"{desc}\""));
            }
            if !win_title.trim().is_empty() {
                lines.push(format!("window: \"{}\"", win_title.trim()));
            }
            if let Some(sp) = screenshot_path.as_deref() {
                lines.push(format!("screenshot_path: {sp}"));
            }
            lines.push("[/DSB-PICK]".to_string());
            CapsulePayload {
                summary: CapsuleQuoteSummary {
                    label: badge.clone(),
                    title: truncate_chars(&format!("{badge} {lbl}"), 44),
                    detail: truncate_chars(&format!("{app_label} · {}", fast_enriched.role), 44),
                    body: truncate_chars(&format!("{lbl} ({child_class})"), 110),
                },
                text: lines.join("\n"),
            }
        };
        let initial_cap = build_hm_capsule(&initial_label, &fast_enriched.description);
        inject_capsule_into_bar_with_meta(
            app,
            &initial_cap,
            Some(&cap_id),
            Some(serde_json::json!({
                "exe": target_exe,
                "containerClass": child_class,
            })),
        );
        let toast_msg = if needs_ocr {
            "已抓取按钮 · 后台补全名称…".to_string()
        } else {
            format!("已加入 {app_label} 控件")
        };
        emit_pick_toast(app, &toast_msg);
        if needs_ocr {
            if let Some(raw_cloned) = raw_ref.cloned() {
                let app_bg = app.clone();
                let cap_id_bg = cap_id.clone();
                let rect_bg = entry.rect;
                let role_bg = fast_enriched.role.clone();
                let desc_bg = fast_enriched.description.clone();
                let cls_bg = child_class.clone();
                let exe_bg = target_exe.clone();
                let app_lbl_bg = app_label.clone();
                let win_t_bg = win_title.clone();
                let sp_bg = screenshot_path.clone();
                let badge_bg = badge.clone();
                let child_raw_bg = child_hwnd.0 as isize;
                let _ = std::thread::Builder::new()
                    .name(format!("atp-hm-ocr-{n}"))
                    .spawn(move || {
                        let full = enricher::enrich_box_semantics(
                            HWND(child_raw_bg as *mut std::ffi::c_void),
                            target_pid,
                            &exe_bg,
                            rect_bg,
                            "",
                            &desc_bg,
                            &role_bg,
                            Some(&raw_cloned),
                            true,
                        );
                        let final_lbl = if !full.label.trim().is_empty() {
                            full.label
                        } else {
                            "图标按钮".to_string()
                        };
                        let final_desc = full.description;
                        let mut lines = vec![
                            format!("[DSB-PICK #{n} | kind=ui_hitmap]"),
                            format!(
                                "应用: {} ({}) | 控件: role={} label=\"{}\" class=\"{}\" rect=({},{},{}x{})",
                                app_lbl_bg,
                                exe_bg,
                                role_bg,
                                final_lbl,
                                cls_bg,
                                rect_bg.x,
                                rect_bg.y,
                                rect_bg.w,
                                rect_bg.h
                            ),
                        ];
                        if !final_desc.is_empty() {
                            lines.push(format!("description: \"{final_desc}\""));
                        }
                        if !win_t_bg.trim().is_empty() {
                            lines.push(format!("window: \"{}\"", win_t_bg.trim()));
                        }
                        if let Some(sp) = sp_bg.as_deref() {
                            lines.push(format!("screenshot_path: {sp}"));
                        }
                        lines.push("[/DSB-PICK]".to_string());
                        let patched = CapsulePayload {
                            summary: CapsuleQuoteSummary {
                                label: badge_bg.clone(),
                                title: truncate_chars(&format!("{badge_bg} {final_lbl}"), 44),
                                detail: truncate_chars(&format!("{app_lbl_bg} · {role_bg}"), 44),
                                body: truncate_chars(&format!("{final_lbl} ({cls_bg})"), 110),
                            },
                            text: lines.join("\n"),
                        };
                        patch_capsule_in_bar(&app_bg, &cap_id_bg, &patched);
                    });
            }
        }
        return;
    }
    if prof.uia_policy == profile::UiaPolicy::Forbidden {
        return;
    }

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

fn try_execute_external_plugin_tap_override(
    app: &AppHandle,
    shared: &AppShared,
    x: i32,
    y: i32,
    pre_capture: Option<&RawScreenCapture>,
) -> bool {
    let (root_hwnd, target_pid, win_title, win_class) = resolve_root_window_at(x, y);
    if target_pid == 0 {
        return false;
    }
    let cfg = shared.config.lock().map(|c| c.clone()).unwrap_or_default();
    let proc_meta = inspect_process(target_pid, &cfg.app_source_map);
    let ctx = AdapterContext {
        hwnd: root_hwnd.0 as isize,
        window_title: win_title.clone(),
        window_class: win_class,
        proc_meta,
        primary_element: None,
        tap_point: Some((x, y)),
        broker: shared.broker.clone(),
    };
    let Some(res) = adapter_plugins::try_run_external_tap_override(&ctx) else {
        return false;
    };

    let n = shared.allocate_pick_number();
    let badge = circled_badge(n);
    let pick_rect = ScreenRect {
        x: x - 18,
        y: y - 18,
        w: 36,
        h: 36,
    };
    if let Some(ov) = app.get_webview_window("overlay") {
        let _ = ov.emit("pen:picked", serde_json::json!({ "n": n, "rect": pick_rect }));
    }
    let owned_cap;
    let raw_ref = match pre_capture {
        Some(r) => Some(r),
        None => {
            owned_cap = capture_target_window_bitblt(root_hwnd, x, y).ok();
            owned_cap.as_ref()
        }
    };
    let screenshot_path = raw_ref.and_then(|raw| {
        save_annotated_capture(app, raw, Some(pick_rect), &[], n, &cfg)
            .ok()
            .map(|p| p.to_string_lossy().to_string())
    });

    let mut lines = vec![format!("[DSB-PICK #{n} | kind=adapter_plugin | plugin={}]", res.plugin_id)];
    lines.extend(res.lines.iter().cloned());
    for p in &res.paths {
        lines.push(format!("path: {p}"));
    }
    if !win_title.trim().is_empty() {
        lines.push(format!("window: \"{}\"", win_title.trim()));
    }
    if let Some(sp) = screenshot_path.as_deref() {
        lines.push(format!("screenshot_path: {sp}"));
    }
    lines.push("[/DSB-PICK]".to_string());

    let body_preview = res
        .lines
        .first()
        .cloned()
        .unwrap_or_else(|| res.title.clone());
    let capsule = CapsulePayload {
        summary: CapsuleQuoteSummary {
            label: badge.clone(),
            title: truncate_chars(&format!("{badge} {}", res.title), 44),
            detail: truncate_chars(&res.detail, 44),
            body: truncate_chars(&body_preview, 110),
        },
        text: lines.join("\n"),
    };
    inject_capsule_into_bar(app, &capsule);
    true
}

pub fn maybe_auto_attach_explorer_selection(app: &AppHandle, fg_hwnd_raw: isize) {
    if !adapter_plugins::is_builtin_adapter_enabled("explorer") {
        return;
    }
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
    if expected_pid != 0 && solidworks::is_solidworks_exe(&office::fast_exe_basename_for_pid(expected_pid)) {
        return out;
    }
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
    inject_capsule_into_bar_with_meta(app, capsule, None, None);
}

pub(super) fn inject_capsule_into_bar_with_meta(
    app: &AppHandle,
    capsule: &CapsulePayload,
    capsule_id: Option<&str>,
    meta: Option<serde_json::Value>,
) {
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
        let mut wire = serde_json::json!({
            "summary": capsule.summary,
            "text": capsule.text,
        });
        if let Some(id) = capsule_id {
            wire["id"] = serde_json::Value::String(id.to_string());
        }
        if let Some(m) = meta {
            wire["meta"] = m;
        }
        if let Ok(json_arg) = serde_json::to_string(&wire) {
            let script = format!(
                "(function(){{ const b = window.__DSB_BRIDGE__; if (b && typeof b.insertCapsule === 'function') {{ b.insertCapsule({json_arg}); }} }})();"
            );
            let _ = bar_win.eval(&script);
        }
    }
    dispatch_state_event(app, StateEvent::PickInserted);
}

pub(super) fn patch_capsule_in_bar(app: &AppHandle, capsule_id: &str, patched: &CapsulePayload) {
    if let Some(bar_win) = app.get_webview_window("bar") {
        let wire = serde_json::json!({
            "id": capsule_id,
            "summary": patched.summary,
            "text": patched.text,
        });
        if let Ok(json_arg) = serde_json::to_string(&wire) {
            let script = format!(
                "(function(){{ const b = window.__DSB_BRIDGE__; if (b && typeof b.patchCapsule === 'function') {{ b.patchCapsule({json_arg}); }} }})();"
            );
            let _ = bar_win.eval(&script);
        }
    }
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
    fn ignores_background_selection_only_in_solidworks_and_cad_apps() {
        // SolidWorks & CAD apps strictly ignore empty canvas background
        assert!(should_ignore_background_for_app(
            "SLDWORKS.exe",
            "Afx:0000000140000000:8:00010003:00000000:00000000"
        ));
        assert!(should_ignore_background_for_app("acad.exe", "AfxMDIFrame"));
        assert!(should_ignore_background_for_app("zwcad.exe", "AfxMDIFrame"));
        assert!(should_ignore_background_for_app("ugraf.exe", "AfxFrame"));
        assert!(should_ignore_background_for_app("inventor.exe", "AfxFrame"));
        assert!(should_ignore_background_for_app("rhino.exe", "Rhino"));
        assert!(should_ignore_background_for_app("cnext.exe", "AfxFrame"));

        // Office / WPS / IDEs / Design / Desktop / Browsers do NOT suppress background
        assert!(!should_ignore_background_for_app("winword.exe", "OpusApp"));
        assert!(!should_ignore_background_for_app("excel.exe", "XLMAIN"));
        assert!(!should_ignore_background_for_app(
            "powerpnt.exe",
            "PPTFrameClass"
        ));
        assert!(!should_ignore_background_for_app("wps.exe", "Qt5QWindow"));
        assert!(!should_ignore_background_for_app("et.exe", "XLMAIN"));
        assert!(!should_ignore_background_for_app("wpp.exe", "WPPMain"));
        assert!(!should_ignore_background_for_app(
            "Code.exe",
            "Chrome_WidgetWin_1"
        ));
        assert!(!should_ignore_background_for_app(
            "photoshop.exe",
            "Photoshop"
        ));
        assert!(!should_ignore_background_for_app(
            "figma.exe",
            "Chrome_WidgetWin_1"
        ));
        assert!(!should_ignore_background_for_app(
            "obsidian.exe",
            "Chrome_WidgetWin_1"
        ));
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
