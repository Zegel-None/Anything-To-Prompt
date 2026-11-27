//! Deep Inspection Primitives shared by the Shake-to-Wake Dose Engine:
//! - Revealed organic photo cutouts (`photo_object`) & expanded submenu items (`menu_item`)
//! - Deep UIA sibling/descendant sampling & paragraph coalescing (`Dose 1` on modern UIA apps)
//! - Safe multi-level popup drawer crawler (`Dose 2`, max depth 4, Esc/release instant abort,
//!   with `hooks::set_suppress_synthetic_esc` guard so synthetic Esc NEVER exits Selection Mode!)

use crate::captures::{capture_screen_rect_rgba, ScreenRect};
use crate::pick::hitmap::{self, HitMapEntry, HitMapSource};
use crate::pick::self_heal;
use crate::pick::vision::VisionSegmentItem;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP,
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEINPUT, VK_ESCAPE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetCursorPos, GetWindowLongW, GetWindowRect,
    GetWindowThreadProcessId, IsWindowVisible, SetCursorPos, GWL_STYLE,
};

pub const MAX_MENU_DEPTH: usize = 4;
const DRAWER_TOTAL_BUDGET: Duration = Duration::from_millis(3500);
const DRAWER_POPUP_WAIT: Duration = Duration::from_millis(240);
const SUBMENU_HOVER_DWELL: Duration = Duration::from_millis(240);

/// True geometric overlap deduplication.
pub fn rects_overlap_significantly(a: ScreenRect, b: ScreenRect) -> bool {
    let ix0 = a.x.max(b.x);
    let iy0 = a.y.max(b.y);
    let ix1 = (a.x + a.w).min(b.x + b.w);
    let iy1 = (a.y + a.h).min(b.y + b.h);
    let iw = (ix1 - ix0).max(0) as i64;
    let ih = (iy1 - iy0).max(0) as i64;
    if iw == 0 || ih == 0 {
        return false;
    }
    let inter = iw * ih;
    let area_a = ((a.w.max(1) as i64) * (a.h.max(1) as i64)).max(1);
    let area_b = ((b.w.max(1) as i64) * (b.h.max(1) as i64)).max(1);
    let min_area = area_a.min(area_b);
    if inter * 100 >= min_area * 38 {
        return true;
    }
    let acx = a.x + a.w / 2;
    let acy = a.y + a.h / 2;
    let bcx = b.x + b.w / 2;
    let bcy = b.y + b.h / 2;
    (acx - bcx).abs() <= 10 && (acy - bcy).abs() <= 10
}

#[derive(Debug, Clone)]
pub struct RevealedEntry {
    pub key: String,
    pub rect: ScreenRect,
    pub role: String,
    pub label: String,
    pub semantic_kind: String, // "photo_object" | "menu_item" | "ui_vlm_fixed"
    pub polygon: Option<Vec<(i32, i32)>>,
}

#[derive(Default)]
struct RevealSessionState {
    active: bool,
    revealed: Vec<RevealedEntry>,
}

struct RevealController {
    state: Mutex<RevealSessionState>,
    abort_flag: Arc<AtomicBool>,
    crawl_running: AtomicBool,
}

static CONTROLLER: OnceLock<RevealController> = OnceLock::new();

fn ctrl() -> &'static RevealController {
    CONTROLLER.get_or_init(|| RevealController {
        state: Mutex::new(RevealSessionState::default()),
        abort_flag: Arc::new(AtomicBool::new(false)),
        crawl_running: AtomicBool::new(false),
    })
}

/// Ray-casting point-in-polygon test so hovering/clicking an organic photo cutout hits exact silhouettes.
pub fn point_in_polygon(px: i32, py: i32, poly: &[(i32, i32)]) -> bool {
    if poly.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (xi, yi) = (poly[i].0 as f64, poly[i].1 as f64);
        let (xj, yj) = (poly[j].0 as f64, poly[j].1 as f64);
        let intersect = ((yi > py as f64) != (yj > py as f64))
            && ((px as f64) < (xj - xi) * (py as f64 - yi) / ((yj - yi) + 1e-9) + xi);
        if intersect {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Returns the best matching Dose-2 revealed item (organic photo polygon, expanded submenu item,
/// or VLM-corrected control) at screen pixel `(x, y)` during an active Selection Mode session.
pub fn find_revealed_item_at(x: i32, y: i32) -> Option<RevealedEntry> {
    let st = ctrl().state.lock().ok()?;
    if !st.active || st.revealed.is_empty() {
        return None;
    }
    // 1. Exact organic polygon hit (SAM 2.1 natural photo cutouts)
    let mut best_poly: Option<(i64, RevealedEntry)> = None;
    for item in &st.revealed {
        if let Some(poly) = item.polygon.as_deref() {
            if point_in_polygon(x, y, poly) {
                let area = (item.rect.w.max(1) as i64) * (item.rect.h.max(1) as i64);
                if best_poly.as_ref().is_none_or(|(a, _)| area < *a) {
                    best_poly = Some((area, item.clone()));
                }
            }
        }
    }
    if let Some((_, hit)) = best_poly {
        return Some(hit);
    }
    // 2. Expanded submenu items (`menu_item`) or VLM-corrected controls (`ui_vlm_fixed`)
    let mut best_box: Option<(i64, RevealedEntry)> = None;
    for item in &st.revealed {
        if matches!(item.semantic_kind.as_str(), "menu_item" | "ui_vlm_fixed")
            && x >= item.rect.x
            && x <= item.rect.x + item.rect.w
            && y >= item.rect.y
            && y <= item.rect.y + item.rect.h
        {
            let area = (item.rect.w.max(1) as i64) * (item.rect.h.max(1) as i64);
            if best_box.as_ref().is_none_or(|(a, _)| area < *a) {
                best_box = Some((area, item.clone()));
            }
        }
    }
    best_box.map(|(_, h)| h)
}

pub fn is_abort_flagged() -> bool {
    ctrl().abort_flag.load(Ordering::Acquire)
}

pub fn trigger_abort() {
    ctrl().abort_flag.store(true, Ordering::Release);
}

pub fn reset_abort_flag() {
    ctrl().abort_flag.store(false, Ordering::Release);
}

fn emit_overlay(app: &AppHandle, event: &str, payload: serde_json::Value) {
    if let Some(ov) = app.get_webview_window("overlay") {
        let _ = ov.emit(event, payload);
    }
}

/// Clears Dose 2 revealed photo cutouts & menu items (e.g. after mouse wheel scroll).
pub fn clear_revealed_cutouts(app: &AppHandle) {
    let had_any = if let Ok(mut st) = ctrl().state.lock() {
        let had = !st.revealed.is_empty();
        st.revealed.clear();
        st.active = false;
        had
    } else {
        false
    };
    if had_any {
        emit_overlay(app, "pen:scan_clear", serde_json::json!({}));
    }
}

/// Immediately aborts any in-flight drawer crawl and clears revealed session items on `PenEnd`.
pub fn on_selection_session_end() {
    let c = ctrl();
    c.abort_flag.store(true, Ordering::Release);
    if let Ok(mut st) = c.state.lock() {
        st.active = false;
        st.revealed.clear();
    }
}

/// Publishes SAM 2.1 organic photo cutouts onto the overlay & session hit list.
pub fn publish_photo_cutouts(app: &AppHandle, items: Vec<VisionSegmentItem>) {
    if items.is_empty() {
        return;
    }
    let mut newly_revealed: Vec<serde_json::Value> = Vec::new();
    let total_after: usize;
    {
        let Ok(mut st) = ctrl().state.lock() else {
            return;
        };
        st.active = true;
        for item in items {
            let key = format!(
                "{}:{}:{}:{}:{}",
                item.semantic_kind, item.rect.x, item.rect.y, item.rect.w, item.rect.h
            );
            let is_photo = item.semantic_kind == "photo_object";
            if st.revealed.iter().any(|e| {
                e.key == key || (!is_photo && rects_overlap_significantly(e.rect, item.rect))
            }) {
                continue;
            }
            let poly_json: Option<Vec<[i32; 2]>> = item
                .polygon
                .as_ref()
                .map(|pts| pts.iter().map(|&(px, py)| [px, py]).collect());
            st.revealed.push(RevealedEntry {
                key: key.clone(),
                rect: item.rect,
                role: item.role.clone(),
                label: item.label.clone(),
                semantic_kind: item.semantic_kind.clone(),
                polygon: item.polygon,
            });
            newly_revealed.push(serde_json::json!({
                "key": key,
                "rect": item.rect,
                "label": item.label,
                "role": item.role,
                "semanticKind": item.semantic_kind,
                "polygon": poly_json,
                "isExpandable": false,
            }));
        }
        total_after = st.revealed.len();
    }
    if !newly_revealed.is_empty() {
        emit_overlay(
            app,
            "pen:scan_reveal",
            serde_json::json!({
                "items": newly_revealed,
                "replaceKeys": [],
                "totalCount": total_after,
            }),
        );
    }
}

fn publish_menu_items(app: &AppHandle, items: Vec<HitMapEntry>) {
    if items.is_empty() || is_abort_flagged() {
        return;
    }
    let mut newly_revealed: Vec<serde_json::Value> = Vec::new();
    let total_after: usize;
    {
        let Ok(mut st) = ctrl().state.lock() else {
            return;
        };
        st.active = true;
        for entry in items {
            if st
                .revealed
                .iter()
                .any(|ex| rects_overlap_significantly(ex.rect, entry.rect))
            {
                continue;
            }
            let key = format!(
                "menu:{}:{}:{}:{}",
                entry.rect.x, entry.rect.y, entry.rect.w, entry.rect.h
            );
            let lbl = if entry.label.trim().is_empty() {
                "子菜单项".to_string()
            } else {
                entry.label.clone()
            };
            st.revealed.push(RevealedEntry {
                key: key.clone(),
                rect: entry.rect,
                role: entry.role.clone(),
                label: lbl.clone(),
                semantic_kind: "menu_item".to_string(),
                polygon: None,
            });
            newly_revealed.push(serde_json::json!({
                "key": key,
                "rect": entry.rect,
                "label": lbl,
                "role": entry.role,
                "semanticKind": "menu_item",
                "isExpandable": entry.is_expandable,
            }));
        }
        total_after = st.revealed.len();
    }
    if !newly_revealed.is_empty() {
        emit_overlay(
            app,
            "pen:scan_reveal",
            serde_json::json!({
                "items": newly_revealed,
                "replaceKeys": [],
                "totalCount": total_after,
            }),
        );
    }
}

// =============================================================================
// Deep UIA Sibling/Descendant Sampling & Paragraph Coalescing (Dose 1)
// =============================================================================

fn union_rect(a: ScreenRect, b: ScreenRect) -> ScreenRect {
    let x0 = a.x.min(b.x);
    let y0 = a.y.min(b.y);
    let x1 = (a.x + a.w).max(b.x + b.w);
    let y1 = (a.y + a.h).max(b.y + b.h);
    ScreenRect {
        x: x0,
        y: y0,
        w: (x1 - x0).max(0),
        h: (y1 - y0).max(0),
    }
}

pub fn should_merge_text_paragraph_rects(a: ScreenRect, b: ScreenRect) -> bool {
    let (top, bot) = if a.y <= b.y { (a, b) } else { (b, a) };
    let u = union_rect(top, bot);
    if u.h > 320 || u.w > 1280 {
        return false;
    }
    let top_cy = top.y + top.h / 2;
    let bot_cy = bot.y + bot.h / 2;
    let max_h = top.h.max(bot.h).max(16);
    if (top.y - bot.y).abs() <= 8 && (top_cy - bot_cy).abs() <= max_h / 2 {
        let (left, right) = if top.x <= bot.x { (top, bot) } else { (bot, top) };
        let hgap = right.x - (left.x + left.w);
        if (-8..=24).contains(&hgap) {
            return true;
        }
    }
    let vgap = bot.y - (top.y + top.h);
    if !(-2..=7).contains(&vgap) || top.w.max(bot.w) < 200 {
        return false;
    }
    let left_aligned = (top.x - bot.x).abs() <= 18;
    let ov_x0 = top.x.max(bot.x);
    let ov_x1 = (top.x + top.w).min(bot.x + bot.w);
    let ov_w = (ov_x1 - ov_x0).max(0);
    let min_w = top.w.min(bot.w).max(1);
    left_aligned && ov_w * 100 >= min_w * 55
}

pub fn coalesce_text_paragraphs(mut entries: Vec<HitMapEntry>) -> Vec<HitMapEntry> {
    if entries.len() <= 1 {
        return entries;
    }
    let mut non_text: Vec<HitMapEntry> = Vec::new();
    let mut text_items: Vec<HitMapEntry> = Vec::new();
    for e in entries.drain(..) {
        if e.role == "Text" {
            text_items.push(e);
        } else {
            non_text.push(e);
        }
    }
    if text_items.len() <= 1 {
        non_text.extend(text_items);
        return non_text;
    }

    text_items.sort_by(|a, b| {
        let a_cy = a.rect.y + a.rect.h / 2;
        let b_cy = b.rect.y + b.rect.h / 2;
        let tol = (a.rect.h.min(b.rect.h).max(12) / 2).clamp(6, 14);
        if (a_cy - b_cy).abs() <= tol {
            a.rect.x.cmp(&b.rect.x)
        } else {
            a.rect.y.cmp(&b.rect.y)
        }
    });

    let mut merged: Vec<HitMapEntry> = Vec::new();
    for item in text_items {
        if let Some(target) = merged
            .iter_mut()
            .find(|ex| should_merge_text_paragraph_rects(ex.rect, item.rect))
        {
            target.rect = union_rect(target.rect, item.rect);
            if !item.label.trim().is_empty() {
                if target.label.trim().is_empty() {
                    target.label = item.label;
                } else if !target.label.contains(item.label.trim()) {
                    target.label.push(' ');
                    target.label.push_str(item.label.trim());
                }
            }
        } else {
            merged.push(item);
        }
    }
    let mut changed = true;
    while changed {
        changed = false;
        let mut i = 0;
        while i < merged.len() {
            let mut j = i + 1;
            while j < merged.len() {
                if should_merge_text_paragraph_rects(merged[i].rect, merged[j].rect) {
                    let other = merged.remove(j);
                    merged[i].rect = union_rect(merged[i].rect, other.rect);
                    changed = true;
                } else {
                    j += 1;
                }
            }
            i += 1;
        }
    }
    non_text.extend(merged);
    non_text
}

fn try_promote_to_enclosing_widget(
    walker: &uiautomation::UITreeWalker,
    el: &uiautomation::UIElement,
    win_rect: ScreenRect,
) -> Option<(ScreenRect, String, String)> {
    let mut cur = el.clone();
    for _ in 0..3 {
        let Ok(parent) = walker.get_parent(&cur) else {
            break;
        };
        let pct = parent
            .get_control_type()
            .map(|c| format!("{c:?}"))
            .unwrap_or_default();
        if matches!(
            pct.as_str(),
            "Button" | "Hyperlink" | "MenuItem" | "TabItem" | "ListItem" | "TreeItem" | "SplitButton"
        ) {
            if let Ok(br) = parent.get_bounding_rectangle() {
                let pr = ScreenRect {
                    x: br.get_left(),
                    y: br.get_top(),
                    w: br.get_width(),
                    h: br.get_height(),
                };
                if pr.w >= 16 && pr.h >= 16 && pr.w <= 1200 && pr.h <= 180 && pr.w < win_rect.w {
                    let pname = parent.get_name().unwrap_or_default();
                    return Some((pr, pct, pname));
                }
            }
        }
        cur = parent;
    }
    None
}

/// Dose 1 Deep UIA sampler: probes a dense halo around `(x, y)` plus immediate sibling controls
/// inside modern UIA apps (NEVER called on SolidWorks!).
pub fn sample_deep_uia_around_point(
    root_hwnd: HWND,
    target_pid: u32,
    win_rect: ScreenRect,
    win_class: &str,
    x: i32,
    y: i32,
) -> Vec<HitMapEntry> {
    let Ok(auto) = uiautomation::UIAutomation::new() else {
        return Vec::new();
    };
    let Ok(walker) = auto.get_control_view_walker() else {
        return Vec::new();
    };

    let mut raw_uia: Vec<crate::pick::UiaElementInfo> = Vec::new();
    let offsets = [
        (0, 0),
        (-36, 0),
        (36, 0),
        (0, -28),
        (0, 28),
        (-48, -28),
        (48, 28),
    ];
    for (dx, dy) in offsets {
        if is_abort_flagged() {
            break;
        }
        let Some((el, mut info)) = crate::pick::inspect_uia_element_with_node_at(
            &auto,
            &walker,
            root_hwnd,
            target_pid,
            x + dx,
            y + dy,
        ) else {
            continue;
        };
        if let Some((full_rect, full_role, full_name)) =
            try_promote_to_enclosing_widget(&walker, &el, win_rect)
        {
            info.rect = full_rect;
            info.control_type = full_role;
            if !full_name.trim().is_empty() {
                info.name = full_name;
            }
        }
        if crate::pick::classify_hover_target(&info, win_rect, win_class)
            != crate::pick::HoverTargetKind::Element
        {
            continue;
        }
        if (12..=1280).contains(&info.rect.w)
            && (12..=360).contains(&info.rect.h)
            && !raw_uia
                .iter()
                .any(|e| e.rect == info.rect && e.control_type == info.control_type)
        {
            raw_uia.push(info);
        }
    }

    // Collect immediate siblings in a 192x192 neighborhood so adjacent tiny icons are never missed
    if !is_abort_flagged() {
        let bounds = ScreenRect {
            x: x - 96,
            y: y - 96,
            w: 192,
            h: 192,
        };
        collect_uia_nearby_siblings(
            &auto,
            &walker,
            root_hwnd,
            target_pid,
            win_rect,
            win_class,
            (x, y),
            bounds,
            &mut raw_uia,
        );
    }

    let coalesced = crate::pick::coalesce_inline_text_runs(raw_uia);
    let entries: Vec<HitMapEntry> = coalesced
        .into_iter()
        .map(|info| {
            let label = crate::pick::resolve_primary_element_label(&info).unwrap_or_default();
            let is_top_menu_item =
                info.control_type == "MenuItem" && (info.rect.y - win_rect.y).abs() <= 60;
            let is_expandable = is_top_menu_item
                || matches!(info.control_type.as_str(), "SplitButton" | "ComboBox");
            let trigger_pt = if is_expandable {
                Some((info.rect.x + info.rect.w / 2, info.rect.y + info.rect.h / 2))
            } else {
                None
            };
            HitMapEntry {
                rect: info.rect,
                label,
                description: info.help_text,
                role: info.control_type,
                source: HitMapSource::MsaaSnapshot,
                is_expandable,
                trigger_pt,
            }
        })
        .collect();
    coalesce_text_paragraphs(entries)
}

#[allow(clippy::too_many_arguments)]
fn collect_uia_nearby_siblings(
    auto: &uiautomation::UIAutomation,
    walker: &uiautomation::UITreeWalker,
    root_hwnd: HWND,
    target_pid: u32,
    win_rect: ScreenRect,
    win_class: &str,
    anchor: (i32, i32),
    bounds: ScreenRect,
    out: &mut Vec<crate::pick::UiaElementInfo>,
) {
    let Some((anchor_el, _)) = crate::pick::inspect_uia_element_with_node_at(
        auto, walker, root_hwnd, target_pid, anchor.0, anchor.1,
    ) else {
        return;
    };
    let Ok(parent) = walker.get_parent(&anchor_el) else {
        return;
    };
    let container = walker.get_parent(&parent).unwrap_or(parent);
    let mut stack = vec![(container, 0u8)];
    let mut visited = 0usize;
    let deadline = Instant::now() + Duration::from_millis(140);

    while let Some((node, depth)) = stack.pop() {
        if visited >= 140 || Instant::now() >= deadline || is_abort_flagged() {
            break;
        }
        let mut child = walker.get_first_child(&node).ok();
        while let Some(c) = child {
            visited += 1;
            if visited >= 140 || Instant::now() >= deadline || is_abort_flagged() {
                break;
            }
            if let Ok(br) = c.get_bounding_rectangle() {
                let r = ScreenRect {
                    x: br.get_left(),
                    y: br.get_top(),
                    w: br.get_width(),
                    h: br.get_height(),
                };
                let intersects = r.x < bounds.x + bounds.w
                    && r.x + r.w > bounds.x
                    && r.y < bounds.y + bounds.h
                    && r.y + r.h > bounds.y;
                if intersects && r.w >= 12 && r.h >= 12 {
                    let ct = c
                        .get_control_type()
                        .map(|k| format!("{k:?}"))
                        .unwrap_or_default();
                    let name = c.get_name().unwrap_or_default();
                    let info = crate::pick::UiaElementInfo {
                        name,
                        control_type: ct.clone(),
                        control_type_id: 0,
                        automation_id: c.get_automation_id().unwrap_or_default(),
                        class_name: c.get_classname().unwrap_or_default(),
                        value: String::new(),
                        help_text: String::new(),
                        text_snippet: String::new(),
                        rect: r,
                        sibling_index: 0,
                        ancestors: Vec::new(),
                        is_root_or_empty: false,
                        affordance: None,
                    };
                    if r.w <= 1280
                        && r.h <= 320
                        && matches!(
                            ct.as_str(),
                            "Button"
                                | "Text"
                                | "Hyperlink"
                                | "MenuItem"
                                | "ListItem"
                                | "TreeItem"
                                | "TabItem"
                                | "Edit"
                                | "ComboBox"
                                | "CheckBox"
                                | "RadioButton"
                        )
                        && crate::pick::classify_hover_target(&info, win_rect, win_class)
                            == crate::pick::HoverTargetKind::Element
                        && !out.iter().any(|e| e.rect == r && e.control_type == ct)
                    {
                        out.push(info);
                    }
                    if depth < 3 {
                        stack.push((c.clone(), depth + 1));
                    }
                }
            }
            child = walker.get_next_sibling(&c).ok();
        }
    }
}

// =============================================================================
// Safe Multi-Level Expandable Drawer Crawler (Dose 2A)
// =============================================================================

fn snapshot_pid_popups(target_pid: u32) -> Vec<(HWND, String, ScreenRect)> {
    struct Ctx {
        pid: u32,
        out: Vec<(HWND, String, ScreenRect)>,
    }
    unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = &mut *(lparam.0 as *mut Ctx);
        if !IsWindowVisible(hwnd).as_bool() {
            return BOOL(1);
        }
        let mut p = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut p));
        if p != ctx.pid {
            return BOOL(1);
        }
        let mut cls_buf = [0u16; 128];
        let clen = GetClassNameW(hwnd, &mut cls_buf).max(0) as usize;
        let cls = String::from_utf16_lossy(&cls_buf[..clen]);
        let cls_lower = cls.to_ascii_lowercase();
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        const WS_POPUP_BIT: u32 = 0x8000_0000;
        const WS_CAPTION_BIT: u32 = 0x00C0_0000;
        let is_pop = cls_lower == "#32768"
            || cls_lower.contains("popup")
            || cls_lower.contains("bcgppopup")
            || ((style & WS_POPUP_BIT) != 0 && (style & WS_CAPTION_BIT) != WS_CAPTION_BIT);
        if is_pop {
            let mut rc = RECT::default();
            if GetWindowRect(hwnd, &mut rc).is_ok() {
                let srect = ScreenRect {
                    x: rc.left,
                    y: rc.top,
                    w: (rc.right - rc.left).max(0),
                    h: (rc.bottom - rc.top).max(0),
                };
                if (60..=760).contains(&srect.w) && (24..=1200).contains(&srect.h) {
                    ctx.out.push((hwnd, cls, srect));
                }
            }
        }
        BOOL(1)
    }
    let mut ctx = Ctx {
        pid: target_pid,
        out: Vec::new(),
    };
    unsafe {
        let _ = EnumWindows(Some(enum_cb), LPARAM(&mut ctx as *mut Ctx as isize));
    }
    ctx.out
}

fn sleep_abortable(dur: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < dur {
        if is_abort_flagged() {
            return false;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    !is_abort_flagged()
}

/// Safely dismisses crawled popups while masking synthetic `VK_ESCAPE` from our own Raw Input
/// keyboard listener so dismissing a popup NEVER cancels Selection Mode!
fn send_esc_dismiss_guarded(times: usize) {
    crate::hooks::set_suppress_synthetic_esc(true);
    for _ in 0..times {
        let inputs = [
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VK_ESCAPE,
                        ..Default::default()
                    },
                },
            },
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VK_ESCAPE,
                        dwFlags: KEYEVENTF_KEYUP,
                        ..Default::default()
                    },
                },
            },
        ];
        unsafe {
            let _ = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    std::thread::sleep(Duration::from_millis(35));
    crate::hooks::set_suppress_synthetic_esc(false);
}

fn send_targeted_chevron_click(x: i32, y: i32) {
    unsafe {
        let _ = SetCursorPos(x, y);
    }
    std::thread::sleep(Duration::from_millis(18));
    let inputs = [
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dwFlags: MOUSEEVENTF_LEFTDOWN,
                    dwExtraInfo: 0xD5B0_00E8,
                    ..Default::default()
                },
            },
        },
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dwFlags: MOUSEEVENTF_LEFTUP,
                    dwExtraInfo: 0xD5B0_00E8,
                    ..Default::default()
                },
            },
        },
    ];
    unsafe {
        let _ = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

/// Expands a single `▾`/`▸` menu trigger safely (hover-160ms-first then targeted click, max depth 4),
/// reveals clickable `menu_item` boxes with native `#32768`/MSAA labels, and dismisses the live popup
/// with guarded `Esc` (never killing Selection Mode!).
pub fn expand_single_trigger_safe(
    app: &AppHandle,
    _exe_name: &str,
    target_pid: u32,
    trigger_pt: (i32, i32),
    _entry_rect: ScreenRect,
) {
    let c = ctrl();
    if c.crawl_running
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    c.abort_flag.store(false, Ordering::Release);

    let mut orig_pt = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut orig_pt);
    }
    let budget_start = Instant::now();
    let before: HashSet<isize> = snapshot_pid_popups(target_pid)
        .into_iter()
        .map(|(h, _, _)| h.0 as isize)
        .collect();

    unsafe {
        let _ = SetCursorPos(trigger_pt.0, trigger_pt.1);
    }
    let mut opened: Option<(HWND, String, ScreenRect)> = None;
    if sleep_abortable(Duration::from_millis(160)) {
        for pop in snapshot_pid_popups(target_pid) {
            if !before.contains(&(pop.0 .0 as isize)) {
                opened = Some(pop);
                break;
            }
        }
    }
    if opened.is_none() && !is_abort_flagged() {
        send_targeted_chevron_click(trigger_pt.0, trigger_pt.1);
        let wait_t0 = Instant::now();
        while wait_t0.elapsed() < DRAWER_POPUP_WAIT {
            if is_abort_flagged() {
                break;
            }
            for pop in snapshot_pid_popups(target_pid) {
                if !before.contains(&(pop.0 .0 as isize)) {
                    opened = Some(pop);
                    break;
                }
            }
            if opened.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    if let Some((pop_hwnd, pop_cls, pop_rect)) = opened {
        let mut known = before;
        known.insert(pop_hwnd.0 as isize);
        let max_depth = crawl_popup_recursive(
            app,
            target_pid,
            pop_hwnd,
            &pop_cls,
            pop_rect,
            1,
            &mut known,
            budget_start,
        );
        send_esc_dismiss_guarded(max_depth + 1);
        let _ = sleep_abortable(Duration::from_millis(50));
        self_heal::emit_status_hint(
            app,
            "多级子菜单已展开采集完成，可直接点选浮出的菜单项",
            Some(trigger_pt),
            "done",
            2400,
        );
    }

    unsafe {
        let _ = SetCursorPos(orig_pt.x, orig_pt.y);
    }
    c.crawl_running.store(false, Ordering::Release);
}

#[allow(clippy::too_many_arguments)]
fn crawl_popup_recursive(
    app: &AppHandle,
    target_pid: u32,
    popup_hwnd: HWND,
    popup_cls: &str,
    popup_rect: ScreenRect,
    depth: usize,
    known_popups: &mut HashSet<isize>,
    budget_start: Instant,
) -> usize {
    if depth > MAX_MENU_DEPTH || is_abort_flagged() || budget_start.elapsed() >= DRAWER_TOTAL_BUDGET
    {
        return depth;
    }

    let mut items =
        crate::pick::win32_controls::read_native_win32_control_entries(popup_hwnd, popup_cls);
    if items.is_empty() {
        items = hitmap::build_msaa_entries(popup_hwnd, popup_rect);
    }
    if !items.is_empty() {
        hitmap::merge_entries_into_hitmap(
            popup_hwnd.0 as isize,
            popup_rect,
            HitMapSource::MsaaSnapshot,
            items.clone(),
        );
    }
    if let Ok(cap) = capture_screen_rect_rgba(popup_rect.x, popup_rect.y, popup_rect.w, popup_rect.h)
    {
        let visual = hitmap::segment_tree_or_menu_rows_entries(
            &cap.image,
            cap.origin_x,
            cap.origin_y,
            true,
        );
        if !visual.is_empty() {
            hitmap::merge_entries_into_hitmap(
                popup_hwnd.0 as isize,
                popup_rect,
                HitMapSource::StaticVisual,
                visual.clone(),
            );
            items.extend(visual);
        }
    }

    let submenu_rows: Vec<(i32, i32)> = items
        .iter()
        .filter(|e| e.is_expandable)
        .filter_map(|e| e.trigger_pt)
        .take(3)
        .collect();

    publish_menu_items(app, items);

    let mut deepest = depth;
    if depth < MAX_MENU_DEPTH {
        for (sx, sy) in submenu_rows {
            if is_abort_flagged() || budget_start.elapsed() >= DRAWER_TOTAL_BUDGET {
                break;
            }
            unsafe {
                let _ = SetCursorPos(sx, sy);
            }
            if !sleep_abortable(SUBMENU_HOVER_DWELL) {
                break;
            }
            let new_sub = snapshot_pid_popups(target_pid)
                .into_iter()
                .find(|(h, _, _)| !known_popups.contains(&(h.0 as isize)));
            if let Some((sub_hwnd, sub_cls, sub_rect)) = new_sub {
                known_popups.insert(sub_hwnd.0 as isize);
                let d = crawl_popup_recursive(
                    app,
                    target_pid,
                    sub_hwnd,
                    &sub_cls,
                    sub_rect,
                    depth + 1,
                    known_popups,
                    budget_start,
                );
                deepest = deepest.max(d);
            }
        }
    }
    deepest
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometric_overlap_dedup_prevents_offset_duplicate_boxes() {
        let a = ScreenRect { x: 100, y: 50, w: 120, h: 28 };
        let b_shifted = ScreenRect { x: 106, y: 52, w: 118, h: 28 };
        let c_next_row = ScreenRect { x: 100, y: 82, w: 120, h: 28 };
        assert!(rects_overlap_significantly(a, b_shifted));
        assert!(!rects_overlap_significantly(a, c_next_row));
    }

    #[test]
    fn continuous_text_coalesces_multiline_paragraph_and_preserves_single_line_and_paragraph_gap() {
        let mk_txt = |x, y, w, h, s: &str| HitMapEntry {
            rect: ScreenRect { x, y, w, h },
            label: s.to_string(),
            description: String::new(),
            role: "Text".to_string(),
            source: HitMapSource::MsaaSnapshot,
            is_expandable: false,
            trigger_pt: None,
        };
        let items = vec![
            mk_txt(100, 100, 180, 22, "校验与部署"),
            mk_txt(100, 146, 120, 20, "对 Electron"),
            mk_txt(224, 146, 260, 20, "/ 浏览器原生支持 UIA 的软件，"),
            mk_txt(100, 170, 380, 20, "刷扫直接读取零偏移精确控件矩形"),
            mk_txt(100, 194, 90, 20, "不会偏移。"),
        ];
        let out = coalesce_text_paragraphs(items);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].rect, ScreenRect { x: 100, y: 100, w: 180, h: 22 });
        assert_eq!(out[1].rect, ScreenRect { x: 100, y: 146, w: 384, h: 68 });
    }

    #[test]
    fn abort_flag_trips_immediately_and_clears_on_new_session() {
        reset_abort_flag();
        assert!(!is_abort_flagged());
        trigger_abort();
        assert!(is_abort_flagged());
        reset_abort_flag();
    }

    #[test]
    fn point_in_polygon_accurately_hits_organic_silhouette() {
        let poly = vec![(100, 100), (200, 100), (220, 180), (150, 220), (80, 170)];
        assert!(point_in_polygon(150, 150, &poly));
        assert!(!point_in_polygon(50, 50, &poly));
        assert!(!point_in_polygon(240, 150, &poly));
    }
}
