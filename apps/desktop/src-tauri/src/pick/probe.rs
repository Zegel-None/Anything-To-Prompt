use crate::captures::ScreenRect;
use crate::pick::adapters::{office, solidworks};
use crate::pick::profile::{
    enforce_hard_invariants, get_effective_profile, normalize_exe_key, now_unix_secs,
    AppUiProfile, GroundingLayer, ProfileSource, UiaPolicy, PROFILE_SCHEMA_VERSION,
};
use crate::win32_util::window_title_text;
use serde::{Deserialize, Serialize};
use std::ffi::c_void;
use std::time::{Duration, Instant};
use windows::core::{Interface, BOOL};
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::System::Com::{
    CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Variant::{VARIANT, VT_DISPATCH, VT_I4};
use windows::Win32::UI::Accessibility::{
    AccessibleChildren, AccessibleObjectFromWindow, IAccessible,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GetClassNameW, GetDlgCtrlID, GetWindowLongW, GetWindowRect,
    GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, GWL_STYLE, OBJID_CLIENT,
};

const CHILDID_SELF: i32 = 0;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeChildWindow {
    pub hwnd: isize,
    pub class_name: String,
    pub text: String,
    pub ctrl_id: i32,
    pub style: u32,
    pub rect: ScreenRect,
    pub is_toolbar_candidate: bool,
    pub is_status_bar: bool,
    pub is_canvas_candidate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeMsaaItem {
    pub name: String,
    pub description: String,
    pub role: String,
    pub rect: ScreenRect,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeContainerReport {
    pub hwnd: isize,
    pub class_name: String,
    pub rect: ScreenRect,
    pub msaa_elapsed_ms: u64,
    pub msaa_child_count: i32,
    pub msaa_valid_items: usize,
    pub sample_items: Vec<ProbeMsaaItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeUiaSummary {
    pub attempted: bool,
    pub skipped_reason: Option<String>,
    pub elapsed_ms: u64,
    pub timed_out: bool,
    pub succeeded: bool,
    pub sample_control_type: String,
    pub sample_name: String,
    pub sample_rect: Option<ScreenRect>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeReport {
    pub exe_name: String,
    pub root_hwnd: isize,
    pub window_title: String,
    pub window_class: String,
    pub window_rect: ScreenRect,
    pub total_elapsed_ms: u64,
    pub win32_child_count: usize,
    pub status_bars: Vec<ProbeChildWindow>,
    pub toolbar_candidates: Vec<ProbeChildWindow>,
    pub msaa_containers: Vec<ProbeContainerReport>,
    pub uia_summary: ProbeUiaSummary,
    pub synthesized_profile: AppUiProfile,
}

fn rect_to_screen_rect(r: RECT) -> ScreenRect {
    ScreenRect {
        x: r.left,
        y: r.top,
        w: (r.right - r.left).max(0),
        h: (r.bottom - r.top).max(0),
    }
}

pub fn is_toolbar_class_name(cls_lower: &str) -> bool {
    cls_lower.contains("toolbar")
        || cls_lower.contains("afx:toolbar")
        || cls_lower.contains("afxcontrolbar")
        || cls_lower.contains("bcgp")
        || cls_lower.contains("commandbar")
        || cls_lower.contains("rebarwindow32")
        || cls_lower.contains("netuihwnd")
        || cls_lower.contains("owl.dock")
        || cls_lower.contains("owl.controlbar")
}

pub fn is_status_bar_class_name(cls_lower: &str) -> bool {
    cls_lower.contains("statusbar") || cls_lower.contains("msctls_statusbar32")
}

pub fn is_canvas_class_name(cls_lower: &str) -> bool {
    cls_lower.starts_with("afxframeorview")
        || cls_lower.contains("mdiclient")
        || cls_lower.contains("opengl")
        || cls_lower.contains("swgraphics")
        || cls_lower.contains("ghost_windowclass")
        || cls_lower.contains("psviewc")
}

/// Non-blocking Win32 child window enumeration for `root_hwnd`.
pub fn enumerate_win32_tree(root_hwnd: HWND) -> Vec<ProbeChildWindow> {
    struct Ctx {
        out: Vec<ProbeChildWindow>,
        max_items: usize,
    }
    unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = &mut *(lparam.0 as *mut Ctx);
        if ctx.out.len() >= ctx.max_items {
            return BOOL(0);
        }
        if !IsWindowVisible(hwnd).as_bool() {
            return BOOL(1);
        }
        let mut rc = RECT::default();
        if GetWindowRect(hwnd, &mut rc).is_err() {
            return BOOL(1);
        }
        let srect = rect_to_screen_rect(rc);
        if srect.w < 4 || srect.h < 4 {
            return BOOL(1);
        }
        let mut cls_buf = [0u16; 128];
        let clen = GetClassNameW(hwnd, &mut cls_buf).max(0) as usize;
        let class_name = String::from_utf16_lossy(&cls_buf[..clen]);
        let cls_lower = class_name.to_ascii_lowercase();
        let ctrl_id = GetDlgCtrlID(hwnd);
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        let is_tb = is_toolbar_class_name(&cls_lower);
        let is_sb = is_status_bar_class_name(&cls_lower);
        let is_cv = is_canvas_class_name(&cls_lower);
        // Only issue non-blocking WM_GETTEXT on small/interesting controls (bars, buttons, statics)
        let text = if is_sb || is_tb || srect.h <= 64 {
            solidworks::read_hwnd_text_nonblocking(hwnd, 45)
        } else {
            String::new()
        };
        ctx.out.push(ProbeChildWindow {
            hwnd: hwnd.0 as isize,
            class_name,
            text,
            ctrl_id,
            style,
            rect: srect,
            is_toolbar_candidate: is_tb,
            is_status_bar: is_sb,
            is_canvas_candidate: is_cv,
        });
        BOOL(1)
    }

    let mut ctx = Ctx {
        out: Vec::with_capacity(128),
        max_items: 512,
    };
    unsafe {
        let _ = EnumChildWindows(
            Some(root_hwnd),
            Some(enum_cb),
            LPARAM(&mut ctx as *mut Ctx as isize),
        );
    }
    ctx.out
}

fn role_id_to_name(role_id: i32) -> &'static str {
    match role_id {
        0x0A => "Client",
        0x16 => "ToolBar",
        0x19 => "MenuItem",
        0x22 => "ListItem",
        0x24 => "OutlineItem",
        0x25 => "PageTab",
        0x2B => "PushButton",
        0x2C => "CheckButton",
        0x2D => "RadioButton",
        0x2E => "ComboBox",
        0x38 => "ButtonDropDown",
        0x39 => "ButtonMenu",
        0x3A => "ButtonDropDownGrid",
        0x15 => "Separator",
        _ => "Control",
    }
}

fn read_msaa_item_props(acc: &IAccessible, child_var: &VARIANT) -> Option<ProbeMsaaItem> {
    let mut left = 0i32;
    let mut top = 0i32;
    let mut width = 0i32;
    let mut height = 0i32;
    if unsafe { acc.accLocation(&mut left, &mut top, &mut width, &mut height, child_var) }.is_err()
    {
        return None;
    }
    if width < 8 || height < 8 || width > 1200 || height > 400 {
        return None;
    }

    let name = unsafe { acc.get_accName(child_var) }
        .ok()
        .map(|b| b.to_string().replace('\0', "").trim().to_string())
        .unwrap_or_default();
    let description = unsafe { acc.get_accDescription(child_var) }
        .ok()
        .map(|b| b.to_string().replace('\0', "").trim().to_string())
        .unwrap_or_default();
    let role_str = unsafe { acc.get_accRole(child_var) }
        .ok()
        .and_then(|v| i32::try_from(&v).ok())
        .map(|id| role_id_to_name(id).to_string())
        .unwrap_or_else(|| "Control".to_string());

    if role_str == "Separator" {
        return None;
    }

    Some(ProbeMsaaItem {
        name,
        description,
        role: role_str,
        rect: ScreenRect {
            x: left,
            y: top,
            w: width,
            h: height,
        },
    })
}

fn collect_msaa_children_recursive(
    acc: &IAccessible,
    container_rect: ScreenRect,
    depth: u8,
    out: &mut Vec<ProbeMsaaItem>,
    max_items: usize,
) -> i32 {
    if depth > 2 || out.len() >= max_items {
        return 0;
    }
    let count = unsafe { acc.accChildCount() }.unwrap_or(0);
    if count <= 0 {
        return count;
    }
    let fetch = count.min(128) as usize;
    let mut children: Vec<VARIANT> = vec![VARIANT::default(); fetch];
    let mut obtained = 0i32;
    let hr = unsafe { AccessibleChildren(acc, 0, &mut children, &mut obtained) };
    if hr.is_err() || obtained <= 0 {
        return count;
    }

    for var in children.iter().take(obtained as usize) {
        if out.len() >= max_items {
            break;
        }
        let vt = var.vt();
        if vt == VT_I4 {
            if let Some(item) = read_msaa_item_props(acc, var) {
                let covers_whole = item.rect.w * 10 >= container_rect.w * 9
                    && item.rect.h * 10 >= container_rect.h * 9
                    && container_rect.w > 220;
                if !covers_whole {
                    out.push(item);
                }
            }
        } else if vt == VT_DISPATCH {
            if let Ok(disp) = windows::Win32::System::Com::IDispatch::try_from(var) {
                if let Ok(child_acc) = disp.cast::<IAccessible>() {
                    let self_var = VARIANT::from(CHILDID_SELF);
                    let child_count = unsafe { child_acc.accChildCount() }.unwrap_or(0);
                    if let Some(item) = read_msaa_item_props(&child_acc, &self_var) {
                        let covers_whole = item.rect.w * 10 >= container_rect.w * 9
                            && item.rect.h * 10 >= container_rect.h * 9
                            && container_rect.w > 220;
                        let is_leaf_button = matches!(
                            item.role.as_str(),
                            "PushButton"
                                | "CheckButton"
                                | "RadioButton"
                                | "ButtonDropDown"
                                | "ButtonMenu"
                                | "ButtonDropDownGrid"
                                | "PageTab"
                                | "MenuItem"
                        );
                        if !covers_whole && (is_leaf_button || child_count == 0) {
                            out.push(item);
                            continue;
                        }
                    }
                    if child_count > 0 && depth < 2 {
                        let _ = collect_msaa_children_recursive(
                            &child_acc,
                            container_rect,
                            depth + 1,
                            out,
                            max_items,
                        );
                    }
                }
            }
        }
    }
    count
}

/// Container-scoped, one-shot MSAA batch snapshot (`AccessibleObjectFromWindow` + `AccessibleChildren`).
/// Never touches UIA and never polls on mouse move.
pub fn probe_container_msaa_children(
    container_hwnd: HWND,
    container_rect: ScreenRect,
) -> (Vec<ProbeMsaaItem>, i32, u64) {
    let start = Instant::now();
    let com_hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let need_uninit = com_hr.is_ok();

    let mut out = Vec::new();
    let mut raw_ptr: *mut c_void = std::ptr::null_mut();
    let hr = unsafe {
        AccessibleObjectFromWindow(
            container_hwnd,
            OBJID_CLIENT.0 as u32,
            &IAccessible::IID,
            &mut raw_ptr,
        )
    };
    let mut child_count = 0i32;
    if hr.is_ok() && !raw_ptr.is_null() {
        let acc = unsafe { IAccessible::from_raw(raw_ptr) };
        child_count = collect_msaa_children_recursive(&acc, container_rect, 0, &mut out, 160);
    }

    if need_uninit {
        unsafe { CoUninitialize() };
    }
    (out, child_count, start.elapsed().as_millis() as u64)
}

/// Watchdog-guarded single-point UIA probe on an isolated worker thread.
/// Strictly skipped if `policy == UiaPolicy::Forbidden` (e.g., SLDWORKS.exe).
pub fn probe_uia_with_watchdog(
    root_hwnd: HWND,
    point: (i32, i32),
    policy: UiaPolicy,
    timeout_ms: u64,
) -> ProbeUiaSummary {
    if policy == UiaPolicy::Forbidden {
        return ProbeUiaSummary {
            attempted: false,
            skipped_reason: Some("uia_policy=forbidden".to_string()),
            elapsed_ms: 0,
            timed_out: false,
            succeeded: false,
            sample_control_type: String::new(),
            sample_name: String::new(),
            sample_rect: None,
        };
    }

    let (tx, rx) = std::sync::mpsc::channel();
    let hwnd_raw = root_hwnd.0 as isize;
    let (px, py) = point;
    let _ = std::thread::Builder::new()
        .name("atp-uia-probe".to_string())
        .spawn(move || {
            let t0 = Instant::now();
            let res = (|| -> Option<(String, String, ScreenRect)> {
                let auto = uiautomation::UIAutomation::new().ok()?;
                let pt = uiautomation::types::Point::new(px, py);
                let el = auto.element_from_point(pt).ok()?;
                let ct = el
                    .get_control_type()
                    .map(|c| format!("{c:?}"))
                    .unwrap_or_default();
                let name = el.get_name().unwrap_or_default();
                let br = el.get_bounding_rectangle().ok()?;
                let _ = hwnd_raw;
                Some((
                    ct,
                    name,
                    ScreenRect {
                        x: br.get_left(),
                        y: br.get_top(),
                        w: br.get_width(),
                        h: br.get_height(),
                    },
                ))
            })();
            let _ = tx.send((res, t0.elapsed().as_millis() as u64));
        });

    match rx.recv_timeout(Duration::from_millis(timeout_ms.clamp(60, 1200))) {
        Ok((Some((ct, name, rect)), ms)) => ProbeUiaSummary {
            attempted: true,
            skipped_reason: None,
            elapsed_ms: ms,
            timed_out: false,
            succeeded: true,
            sample_control_type: ct,
            sample_name: name,
            sample_rect: Some(rect),
        },
        Ok((None, ms)) => ProbeUiaSummary {
            attempted: true,
            skipped_reason: None,
            elapsed_ms: ms,
            timed_out: false,
            succeeded: false,
            sample_control_type: String::new(),
            sample_name: String::new(),
            sample_rect: None,
        },
        Err(_) => ProbeUiaSummary {
            attempted: true,
            skipped_reason: None,
            elapsed_ms: timeout_ms,
            timed_out: true,
            succeeded: false,
            sample_control_type: String::new(),
            sample_name: String::new(),
            sample_rect: None,
        },
    }
}

pub fn synthesize_profile_from_probe(report: &ProbeReport) -> AppUiProfile {
    let base = get_effective_profile(&report.exe_name);
    let mut prof = AppUiProfile {
        schema_version: PROFILE_SCHEMA_VERSION,
        exe_name: normalize_exe_key(&report.exe_name),
        display_name: if !base.display_name.is_empty() {
            base.display_name.clone()
        } else {
            report.exe_name.clone()
        },
        source: ProfileSource::Learned,
        uia_policy: base.uia_policy,
        preferred_layers: Vec::new(),
        ignore_background: base.ignore_background,
        status_bar_classes: report
            .status_bars
            .iter()
            .map(|w| w.class_name.to_ascii_lowercase())
            .collect(),
        toolbar_classes: report
            .toolbar_candidates
            .iter()
            .map(|w| w.class_name.to_ascii_lowercase())
            .collect(),
        ignore_canvas_classes: base.ignore_canvas_classes.clone(),
        learned_regions: base.learned_regions.clone(),
        semantic_adapter: base.semantic_adapter.clone(),
        uia_timeout_ms: base.uia_timeout_ms,
        learned_at_unix: now_unix_secs(),
        notes: String::new(),
    };
    prof.status_bar_classes.sort();
    prof.status_bar_classes.dedup();
    prof.toolbar_classes.sort();
    prof.toolbar_classes.dedup();

    let msaa_total_items: usize = report
        .msaa_containers
        .iter()
        .map(|c| c.msaa_valid_items)
        .sum();
    let msaa_max_ms: u64 = report
        .msaa_containers
        .iter()
        .map(|c| c.msaa_elapsed_ms)
        .max()
        .unwrap_or(0);

    // Decide UIA policy & layer order from objective measurements
    let uia_healthy = report.uia_summary.succeeded
        && !report.uia_summary.timed_out
        && report.uia_summary.elapsed_ms <= 90
        && report.uia_summary.sample_rect.map_or(false, |r| {
            r.w * 10 < report.window_rect.w * 8 || r.h * 10 < report.window_rect.h * 8
        });

    if prof.uia_policy != UiaPolicy::Forbidden {
        if report.uia_summary.timed_out || report.uia_summary.elapsed_ms > 200 {
            prof.uia_policy = UiaPolicy::Forbidden;
        } else if uia_healthy {
            prof.uia_policy = UiaPolicy::Preferred;
            prof.preferred_layers.push(GroundingLayer::Uia);
        } else {
            prof.uia_policy = UiaPolicy::WatchdogOnly;
        }
    }

    if msaa_total_items > 0 && msaa_max_ms <= 300 {
        prof.preferred_layers.push(GroundingLayer::MsaaHitMap);
    }
    prof.preferred_layers.push(GroundingLayer::StaticVisualHitMap);
    if prof.uia_policy == UiaPolicy::WatchdogOnly
        && !prof.preferred_layers.contains(&GroundingLayer::Uia)
    {
        prof.preferred_layers.push(GroundingLayer::Uia);
    }

    prof.notes = format!(
        "auto-probed: win32_children={}, toolbars={}, msaa_items={} (max {}ms), uia_ok={} ({}ms, timeout={})",
        report.win32_child_count,
        report.toolbar_candidates.len(),
        msaa_total_items,
        msaa_max_ms,
        report.uia_summary.succeeded,
        report.uia_summary.elapsed_ms,
        report.uia_summary.timed_out
    );
    enforce_hard_invariants(&mut prof);
    prof
}

/// Runs a complete, safe, read-only capability probe on `root_hwnd`.
pub fn probe_window(
    root_hwnd: HWND,
    exe_name: &str,
    sample_point: Option<(i32, i32)>,
) -> ProbeReport {
    let t0 = Instant::now();
    let exe_key = normalize_exe_key(exe_name);
    let effective = get_effective_profile(&exe_key);

    let mut root_rc = RECT::default();
    let _ = unsafe { GetWindowRect(root_hwnd, &mut root_rc) };
    let win_rect = rect_to_screen_rect(root_rc);
    let win_title = window_title_text(root_hwnd);
    let mut cls_buf = [0u16; 128];
    let clen = unsafe { GetClassNameW(root_hwnd, &mut cls_buf) }.max(0) as usize;
    let win_class = String::from_utf16_lossy(&cls_buf[..clen]);

    let children = enumerate_win32_tree(root_hwnd);
    let status_bars: Vec<ProbeChildWindow> = children
        .iter()
        .filter(|c| c.is_status_bar)
        .cloned()
        .collect();
    let mut toolbar_candidates: Vec<ProbeChildWindow> = children
        .iter()
        .filter(|c| c.is_toolbar_candidate && c.rect.w >= 48 && c.rect.h >= 20)
        .cloned()
        .collect();
    // Sort largest/top-most toolbar candidates first and cap MSAA probing to top 10 containers
    toolbar_candidates.sort_by(|a, b| {
        a.rect
            .y
            .cmp(&b.rect.y)
            .then_with(|| (b.rect.w * b.rect.h).cmp(&(a.rect.w * a.rect.h)))
    });

    let mut msaa_containers = Vec::new();
    for tb in toolbar_candidates.iter().take(10) {
        let (items, count, elapsed_ms) =
            probe_container_msaa_children(HWND(tb.hwnd as *mut c_void), tb.rect);
        msaa_containers.push(ProbeContainerReport {
            hwnd: tb.hwnd,
            class_name: tb.class_name.clone(),
            rect: tb.rect,
            msaa_elapsed_ms: elapsed_ms,
            msaa_child_count: count,
            msaa_valid_items: items.len(),
            sample_items: items.into_iter().take(24).collect(),
        });
    }

    let uia_pt = sample_point.unwrap_or_else(|| {
        if let Some(tb) = toolbar_candidates.first() {
            (tb.rect.x + tb.rect.w / 4, tb.rect.y + tb.rect.h / 2)
        } else {
            (win_rect.x + win_rect.w / 4, win_rect.y + 80.min(win_rect.h / 2))
        }
    });
    let uia_summary = probe_uia_with_watchdog(
        root_hwnd,
        uia_pt,
        effective.uia_policy,
        effective.uia_timeout_ms.max(180),
    );

    let mut report = ProbeReport {
        exe_name: exe_key,
        root_hwnd: root_hwnd.0 as isize,
        window_title: win_title,
        window_class: win_class,
        window_rect: win_rect,
        total_elapsed_ms: t0.elapsed().as_millis() as u64,
        win32_child_count: children.len(),
        status_bars,
        toolbar_candidates,
        msaa_containers,
        uia_summary,
        synthesized_profile: effective,
    };
    report.synthesized_profile = synthesize_profile_from_probe(&report);
    report.total_elapsed_ms = t0.elapsed().as_millis() as u64;
    report
}

fn find_top_window_for_target(target: &str) -> Option<(HWND, String)> {
    let trimmed = target.trim();
    if let Some(hex) = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")) {
        if let Ok(raw) = isize::from_str_radix(hex, 16) {
            let h = HWND(raw as *mut c_void);
            if unsafe { IsWindow(Some(h)).as_bool() } {
                let mut pid = 0u32;
                unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
                return Some((h, office::fast_exe_basename_for_pid(pid)));
            }
        }
    } else if let Ok(raw) = trimmed.parse::<isize>() {
        let h = HWND(raw as *mut c_void);
        if unsafe { IsWindow(Some(h)).as_bool() } {
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
            return Some((h, office::fast_exe_basename_for_pid(pid)));
        }
    }

    let wanted = normalize_exe_key(trimmed);
    struct FindCtx {
        wanted: String,
        best: Option<(HWND, String, i64)>,
    }
    unsafe extern "system" fn enum_top(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = &mut *(lparam.0 as *mut FindCtx);
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return BOOL(1);
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return BOOL(1);
        }
        let exe = normalize_exe_key(&office::fast_exe_basename_for_pid(pid));
        if exe == ctx.wanted {
            let mut rc = RECT::default();
            if GetWindowRect(hwnd, &mut rc).is_ok() {
                let area = ((rc.right - rc.left).max(0) as i64) * ((rc.bottom - rc.top).max(0) as i64);
                if area > 10_000 && ctx.best.as_ref().map_or(true, |b| area > b.2) {
                    ctx.best = Some((hwnd, exe, area));
                }
            }
        }
        BOOL(1)
    }

    let mut ctx = FindCtx {
        wanted,
        best: None,
    };
    unsafe {
        let _ = EnumWindows(Some(enum_top), LPARAM(&mut ctx as *mut FindCtx as isize));
    }
    ctx.best.map(|(h, exe, _)| (h, exe))
}

/// CLI entrypoint for `anything-to-prompt.exe --probe-window <exe_or_hwnd> [--out <path>]`.
pub fn run_probe_window_cli(target: &str, out_path: Option<&str>) {
    let Some((hwnd, exe)) = find_top_window_for_target(target) else {
        eprintln!(
            "{}",
            serde_json::json!({
                "ok": false,
                "error": format!("No visible non-minimized window found for target '{target}'")
            })
        );
        std::process::exit(2);
    };
    let report = probe_window(hwnd, &exe, None);
    let json = serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".to_string());
    if let Some(p) = out_path {
        if let Err(e) = std::fs::write(p, &json) {
            eprintln!("Failed to write probe report to {p}: {e}");
            std::process::exit(1);
        }
        println!("Wrote probe report to {p}");
    } else {
        println!("{json}");
    }
}
