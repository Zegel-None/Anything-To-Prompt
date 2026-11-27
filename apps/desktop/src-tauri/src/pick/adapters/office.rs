use crate::broker::{BrokerClient, BrokerRequest};
use crate::captures::ScreenRect;
use crate::pick::adapters::{AdapterContext, AdapterOutput};
use crate::pick::resolve_root_window_at;
use crate::window::{dpi_scale_for_hwnd, get_hwnd_class_name};
use serde::Deserialize;
use std::path::Path;
use uiautomation::patterns::UITextPattern;
use uiautomation::types::{Handle as UiaHandle, Point as UiaPoint, TextPatternRangeEndpoint};
use uiautomation::{UIAutomation, UIElement};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::HiDpi::{
    LogicalToPhysicalPointForPerMonitorDPI, PhysicalToLogicalPointForPerMonitorDPI,
};
use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, WindowFromPoint};

/// Concise, suite-aware MCP routing guidance for the AI agent when an Office/WPS document is open.
pub fn format_live_edit_guidance(app_arg: &str, is_wps: bool, target_locator: &str) -> Vec<String> {
    let preferred_mcp = if is_wps {
        "wps-office (lc2panda/wps-skills, 适用 WPS 最新版)"
    } else {
        match app_arg {
            "excel" => "excel-live (@negokaz/excel-mcp-server, 适用 MS Office 2016-2026)",
            "ppt" => "ppt-live (ykuwai/ppt-mcp, 适用 MS Office 2016-2026)",
            _ => "word-live (ykarapazar/word-mcp-live, 适用 MS Office 2016-2026)",
        }
    };
    let mut out = vec![
        format!(
            "live_edit_rule: 原文档当前处于打开状态，请勿用 python-docx / openpyxl / python-pptx 覆写磁盘文件；若需修改请优先调用实时 MCP [{preferred_mcp}] 在当前打开文档中原地修改。"
        ),
    ];
    if !target_locator.is_empty() {
        out.push(format!("live_edit_target ({app_arg}): {target_locator}"));
    }
    out
}

const WORD_EXES: &[&str] = &["winword.exe"];
const EXCEL_EXES: &[&str] = &["excel.exe", "et.exe"];
const PPT_EXES: &[&str] = &["powerpnt.exe", "wpp.exe"];
const WPS_UNIFIED_EXES: &[&str] = &["wps.exe", "wpsoffice.exe"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfficeAppKind {
    Word,
    Excel,
    PowerPoint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfficeDragMode {
    /// Standard red freehand stroke (all general apps + PowerPoint/WPS Presentation)
    NormalStroke,
    /// Word / WPS Writer: I-beam cursor text selection (NO red stroke)
    WordTextSelect,
    /// Excel / WPS Spreadsheets: rectangular cell range box selection (NO red stroke)
    ExcelCellBox,
}

impl OfficeAppKind {
    pub fn drag_mode(self) -> OfficeDragMode {
        match self {
            OfficeAppKind::Word => OfficeDragMode::WordTextSelect,
            OfficeAppKind::Excel => OfficeDragMode::ExcelCellBox,
            OfficeAppKind::PowerPoint => OfficeDragMode::NormalStroke,
        }
    }
}

#[derive(Debug, Clone)]
pub struct OfficeCanvasHit {
    pub kind: OfficeAppKind,
    pub root_hwnd: HWND,
    pub pid: u32,
    pub win_title: String,
    pub win_class: String,
    pub dpi_scale: f64,
    pub in_canvas: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WordTextPickResult {
    #[serde(default)]
    pub prog_id: String,
    #[serde(default)]
    pub full_name: String,
    #[serde(default)]
    pub doc_name: String,
    #[serde(default)]
    pub selected_text: String,
    #[serde(default)]
    pub page_number: String,
    #[serde(default)]
    pub paragraph_text: String,
    #[serde(default)]
    pub range_start: Option<i32>,
    #[serde(default)]
    pub range_end: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExcelPickResult {
    #[serde(default)]
    pub prog_id: String,
    #[serde(default)]
    pub full_name: String,
    #[serde(default)]
    pub workbook_name: String,
    #[serde(default)]
    pub sheet: String,
    #[serde(default)]
    pub address: String,
    #[serde(default)]
    pub is_range: bool,
    #[serde(default)]
    pub rows: usize,
    #[serde(default)]
    pub cols: usize,
    #[serde(default)]
    pub display_text: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub formula: String,
    #[serde(default)]
    pub col_header: String,
    #[serde(default)]
    pub row_header: String,
    #[serde(default)]
    pub table_preview: String,
    #[serde(default)]
    pub formulas_preview: String,
    #[serde(default)]
    pub screen_rect: Option<ScreenRect>,
    #[serde(default)]
    pub coord_space: String,
    #[serde(default)]
    pub rect_method: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PptElementPickResult {
    #[serde(default)]
    pub prog_id: String,
    #[serde(default)]
    pub full_name: String,
    #[serde(default)]
    pub pres_name: String,
    #[serde(default)]
    pub slide_index: i32,
    #[serde(default)]
    pub slide_count: i32,
    #[serde(default)]
    pub shape_name: String,
    #[serde(default)]
    pub shape_id: i32,
    #[serde(default)]
    pub parent_group_id: Option<i32>,
    #[serde(default)]
    pub parent_group_name: String,
    #[serde(default)]
    pub shape_type: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub geometry: String,
    #[serde(default)]
    pub style: String,
    #[serde(default)]
    pub screen_rect: Option<ScreenRect>,
    #[serde(default)]
    pub coord_space: String,
}

pub fn matches(ctx: &AdapterContext) -> bool {
    let exe = ctx.proc_meta.exe_name.as_str();
    WORD_EXES.contains(&exe)
        || EXCEL_EXES.contains(&exe)
        || PPT_EXES.contains(&exe)
        || WPS_UNIFIED_EXES.contains(&exe)
}

/// Fast pure classifier for Microsoft Office & WPS Office (including WPS multi-tab `wps.exe`).
pub fn detect_office_app(
    exe_name: &str,
    win_title: &str,
    win_class: &str,
    child_class: &str,
) -> Option<OfficeAppKind> {
    let exe = exe_name.trim().to_ascii_lowercase();
    if WORD_EXES.contains(&exe.as_str()) {
        return Some(OfficeAppKind::Word);
    }
    if EXCEL_EXES.contains(&exe.as_str()) {
        return Some(OfficeAppKind::Excel);
    }
    if PPT_EXES.contains(&exe.as_str()) {
        return Some(OfficeAppKind::PowerPoint);
    }

    if WPS_UNIFIED_EXES.contains(&exe.as_str()) {
        let title_low = win_title.to_ascii_lowercase();
        let cc_low = child_class.to_ascii_lowercase();
        let wc_low = win_class.to_ascii_lowercase();

        if title_low.contains(".xlsx")
            || title_low.contains(".xls")
            || title_low.contains(".csv")
            || title_low.contains(".et")
            || win_title.contains("WPS 表格")
            || win_title.contains("WPS表格")
            || win_title.contains("工作簿")
            || title_low.contains("spreadsheet")
            || cc_low.contains("excel")
            || cc_low.contains("etview")
            || wc_low.contains("etmain")
        {
            return Some(OfficeAppKind::Excel);
        }

        if title_low.contains(".pptx")
            || title_low.contains(".ppt")
            || title_low.contains(".dps")
            || win_title.contains("WPS 演示")
            || win_title.contains("WPS演示")
            || win_title.contains("演示文稿")
            || win_title.contains("幻灯片")
            || title_low.contains("presentation")
            || cc_low.contains("wpp")
            || cc_low.contains("slide")
            || wc_low.contains("wppmain")
        {
            return Some(OfficeAppKind::PowerPoint);
        }

        if title_low.contains(".docx")
            || title_low.contains(".doc")
            || title_low.contains(".wps")
            || title_low.contains(".rtf")
            || win_title.contains("WPS 文字")
            || win_title.contains("WPS文字")
            || win_title.contains("文字文稿")
            || title_low.contains("writer")
            || cc_low.contains("_wwg")
            || cc_low.contains("wpsdoc")
        {
            return Some(OfficeAppKind::Word);
        }

        // Default fallback inside wps.exe if title doesn't have an extension yet (e.g. unsaved doc)
        return Some(OfficeAppKind::Word);
    }

    None
}

pub fn fast_exe_basename_for_pid(pid: u32) -> String {
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
            PROCESS_NAME_FORMAT(0),
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .is_ok();
        let _ = windows::Win32::Foundation::CloseHandle(handle);
        if !ok || len == 0 {
            return String::new();
        }
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        Path::new(&full)
            .file_name()
            .map(|s| s.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default()
    }
}

/// Fast synchronous (<0.5ms) Office/WPS detector at screen coordinates `(x, y)`.
/// Also distinguishes document/grid/slide canvas from top Ribbon/Titlebar.
pub fn detect_office_canvas_at(x: i32, y: i32) -> Option<OfficeCanvasHit> {
    let (root_hwnd, pid, win_title, win_class) = resolve_root_window_at(x, y);
    if pid == 0 || root_hwnd.0.is_null() {
        return None;
    }
    let exe = fast_exe_basename_for_pid(pid);
    let child_class = unsafe {
        let hit = WindowFromPoint(POINT { x, y });
        if hit.0.is_null() {
            String::new()
        } else {
            get_hwnd_class_name(hit)
        }
    };
    let kind = detect_office_app(&exe, &win_title, &win_class, &child_class)?;
    let dpi_scale = dpi_scale_for_hwnd(root_hwnd);

    let mut win_rc = RECT::default();
    let has_rc = unsafe { GetWindowRect(root_hwnd, &mut win_rc).is_ok() };
    let known_canvas_child = matches!(
        child_class.as_str(),
        "_WwG" | "EXCEL7" | "XLDESK" | "paneClassDC" | "mdiClass" | "NetUIHWND"
    );
    let top_chrome_px = (104.0 * dpi_scale).round() as i32;
    let in_canvas = if child_class == "_WwG" || child_class == "EXCEL7" || child_class == "XLDESK" {
        true
    } else if has_rc {
        y >= win_rc.top + top_chrome_px && y <= win_rc.bottom - (20.0 * dpi_scale).round() as i32
    } else {
        known_canvas_child
    };

    Some(OfficeCanvasHit {
        kind,
        root_hwnd,
        pid,
        win_title,
        win_class,
        dpi_scale,
        in_canvas,
    })
}

/// Fast (~3-8ms) UIA `UITextPattern` character-level selection between two screen points in Word / WPS.
/// When `do_select` is true, highlights the exact text natively inside Word/WPS in real time!
pub fn select_word_text_uia(
    root_hwnd: HWND,
    start: (i32, i32),
    end: (i32, i32),
    do_select: bool,
) -> Option<String> {
    let auto = UIAutomation::new().ok()?;
    let pattern = find_text_pattern_at(&auto, root_hwnd, start)?;

    let r1 = pattern
        .get_range_from_point(UiaPoint::new(start.0, start.1))
        .ok()?;
    let r2 = pattern
        .get_range_from_point(UiaPoint::new(end.0, end.1))
        .ok()?;

    let cmp = r1
        .compare_endpoints(
            TextPatternRangeEndpoint::Start,
            &r2,
            TextPatternRangeEndpoint::Start,
        )
        .unwrap_or(0);

    let chosen = if cmp <= 0 {
        let _ = r1.move_endpoint_by_range(
            TextPatternRangeEndpoint::End,
            &r2,
            TextPatternRangeEndpoint::End,
        );
        r1
    } else {
        let _ = r2.move_endpoint_by_range(
            TextPatternRangeEndpoint::End,
            &r1,
            TextPatternRangeEndpoint::End,
        );
        r2
    };

    if do_select {
        let _ = chosen.select();
    }

    let text = chosen.get_text(3000).ok()?;
    let cleaned = text.replace('\r', "\n").trim().to_string();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn find_text_pattern_at(
    auto: &UIAutomation,
    root_hwnd: HWND,
    pt: (i32, i32),
) -> Option<UITextPattern> {
    let walker = auto.get_control_view_walker().ok()?;
    if let Ok(mut el) = auto.element_from_point(UiaPoint::new(pt.0, pt.1)) {
        for _ in 0..8 {
            if let Ok(pat) = el.get_pattern::<UITextPattern>() {
                return Some(pat);
            }
            match walker.get_parent(&el) {
                Ok(parent) => el = parent,
                Err(_) => break,
            }
        }
    }
    if !root_hwnd.0.is_null() {
        if let Ok(root_el) = auto.element_from_handle(UiaHandle::from(root_hwnd.0 as isize)) {
            if let Ok(pat) = root_el.get_pattern::<UITextPattern>() {
                return Some(pat);
            }
            return find_text_pattern_descendant(&root_el, &walker, 0);
        }
    }
    None
}

fn find_text_pattern_descendant(
    el: &UIElement,
    walker: &uiautomation::UITreeWalker,
    depth: usize,
) -> Option<UITextPattern> {
    if depth > 6 {
        return None;
    }
    let mut child = walker.get_first_child(el).ok()?;
    for _ in 0..40 {
        if let Ok(pat) = child.get_pattern::<UITextPattern>() {
            return Some(pat);
        }
        if let Some(found) = find_text_pattern_descendant(&child, walker, depth + 1) {
            return Some(found);
        }
        match walker.get_next_sibling(&child) {
            Ok(next) => child = next,
            Err(_) => break,
        }
    }
    None
}

/// COM-backed Word / WPS Writer text selection between `(x1, y1)` and `(x2, y2)`.
/// Selects the exact character range natively via `ActiveWindow.RangeFromPoint` (trying physical
/// pixels then DPI-scaled fallback) and returns document path, page number, and selected text.
pub fn pick_word_text_com(
    broker: &BrokerClient,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    dpi_scale: f64,
) -> Option<WordTextPickResult> {
    let scale = dpi_scale.max(0.5);
    let lx1 = ((x1 as f64) / scale).round() as i32;
    let ly1 = ((y1 as f64) / scale).round() as i32;
    let lx2 = ((x2 as f64) / scale).round() as i32;
    let ly2 = ((y2 as f64) / scale).round() as i32;

    let script = format!(
        r#"
$app = $null
$usedProgId = ''
foreach ($prog in @('Word.Application', 'KWPS.Application', 'Kwps.Application')) {{
  try {{ $app = [Runtime.InteropServices.Marshal]::GetActiveObject($prog); if ($null -ne $app) {{ $usedProgId = $prog; break }} }} catch {{}}
}}
if ($null -eq $app -or $null -eq $app.ActiveDocument) {{ exit 0 }}
$doc = $app.ActiveDocument
$win = $app.ActiveWindow

function Try-WordRange($w, $px, $py, $lx, $ly) {{
  $r = $null
  try {{ $r = $w.RangeFromPoint([int]$px, [int]$py) }} catch {{}}
  if ($null -eq $r) {{
    try {{ $r = $w.RangeFromPoint([int]$lx, [int]$ly) }} catch {{}}
  }}
  return $r
}}

$selText = ''
$pageNum = ''
$paraText = ''
$rangeStart = $null
$rangeEnd = $null
$r1 = Try-WordRange $win {x1} {y1} {lx1} {ly1}
$r2 = Try-WordRange $win {x2} {y2} {lx2} {ly2}
if ($null -ne $r1 -and $null -ne $r2) {{
  try {{
    $s1 = [int]$r1.Start; $e1 = [int]$r1.End
    $s2 = [int]$r2.Start; $e2 = [int]$r2.End
    $s = [Math]::Min($s1, $s2)
    $e = [Math]::Max($e1, $e2)
    if ($e -gt $s) {{
      $rng = $doc.Range($s, $e)
      try {{ $rng.Select() }} catch {{}}
      $rangeStart = $s
      $rangeEnd = $e
      $selText = [string]$rng.Text
      try {{ $pageNum = [string]$rng.Information(3) }} catch {{}}
      try {{ $paraText = [string]$rng.Paragraphs.First.Range.Text }} catch {{}}
    }}
  }} catch {{}}
}}
if ([string]::IsNullOrWhiteSpace($selText)) {{
  try {{
    $selText = [string]$app.Selection.Text
    try {{ $rangeStart = [int]$app.Selection.Start; $rangeEnd = [int]$app.Selection.End }} catch {{}}
    try {{ $pageNum = [string]$app.Selection.Information(3) }} catch {{}}
  }} catch {{}}
}}
$selText = ($selText -replace "`r", "`n").Trim()
if ($selText.Length -gt 3000) {{ $selText = $selText.Substring(0, 3000) }}
$paraText = ($paraText -replace "`r", "`n").Trim()
if ($paraText.Length -gt 400) {{ $paraText = $paraText.Substring(0, 400) }}
@{{
  progId = $usedProgId
  fullName = [string]$doc.FullName
  docName = [string]$doc.Name
  selectedText = $selText
  pageNumber = $pageNum
  paragraphText = $paraText
  rangeStart = $rangeStart
  rangeEnd = $rangeEnd
}} | ConvertTo-Json -Compress
"#
    );

    let resp = broker.call(BrokerRequest::RunPs51 {
        script,
        timeout_ms: 1600,
    });
    if !resp.ok || resp.text.trim().is_empty() {
        return None;
    }
    serde_json::from_str::<WordTextPickResult>(resp.text.trim()).ok()
}

fn physical_to_window_logical(hwnd: HWND, x: i32, y: i32, dpi_scale: f64) -> (i32, i32) {
    if !hwnd.0.is_null() {
        let mut pt = POINT { x, y };
        let ok = unsafe { PhysicalToLogicalPointForPerMonitorDPI(Some(hwnd), &mut pt).as_bool() };
        if ok {
            return (pt.x, pt.y);
        }
    }
    let scale = dpi_scale.max(0.5);
    (
        ((x as f64) / scale).round() as i32,
        ((y as f64) / scale).round() as i32,
    )
}

fn scale_screen_rect(rect: ScreenRect, scale: f64) -> ScreenRect {
    ScreenRect {
        x: ((rect.x as f64) * scale).round() as i32,
        y: ((rect.y as f64) * scale).round() as i32,
        w: (((rect.w as f64) * scale).round() as i32).max(8),
        h: (((rect.h as f64) * scale).round() as i32).max(8),
    }
}

fn window_logical_rect_to_physical(hwnd: HWND, rect: ScreenRect, dpi_scale: f64) -> ScreenRect {
    if !hwnd.0.is_null() {
        let mut tl = POINT {
            x: rect.x,
            y: rect.y,
        };
        let mut br = POINT {
            x: rect.x + rect.w,
            y: rect.y + rect.h,
        };
        let ok1 = unsafe { LogicalToPhysicalPointForPerMonitorDPI(Some(hwnd), &mut tl).as_bool() };
        let ok2 = unsafe { LogicalToPhysicalPointForPerMonitorDPI(Some(hwnd), &mut br).as_bool() };
        if ok1 && ok2 && br.x > tl.x && br.y > tl.y {
            return ScreenRect {
                x: tl.x,
                y: tl.y,
                w: (br.x - tl.x).max(8),
                h: (br.y - tl.y).max(8),
            };
        }
    }
    scale_screen_rect(rect, dpi_scale.max(0.5))
}

pub fn map_office_rect_to_physical(
    hwnd: HWND,
    rect: ScreenRect,
    coord_space: &str,
    dpi_scale: f64,
) -> ScreenRect {
    match coord_space {
        "hwnd" => window_logical_rect_to_physical(hwnd, rect, dpi_scale),
        "div_dpi" => scale_screen_rect(rect, dpi_scale.max(0.5)),
        _ => rect,
    }
}

/// Final Rust-side sanity guard: rejects any COM-reported bounding rect that does not
/// even contain the physical user click/drag anchor points (e.g. broken DPI / raw-pt rects).
pub fn validate_office_rect_against_anchors(
    rect: ScreenRect,
    anchors: &[(i32, i32)],
    tolerance_px: i32,
) -> Option<ScreenRect> {
    if rect.w < 6 || rect.h < 6 || rect.w > 4800 || rect.h > 3000 {
        return None;
    }
    let rx2 = rect.x + rect.w;
    let ry2 = rect.y + rect.h;
    for &(ax, ay) in anchors {
        if ax < rect.x - tolerance_px
            || ax > rx2 + tolerance_px
            || ay < rect.y - tolerance_px
            || ay > ry2 + tolerance_px
        {
            return None;
        }
    }
    Some(rect)
}

fn query_window_bounds_tuple(hwnd: HWND) -> (i32, i32, i32, i32) {
    let mut rc = RECT::default();
    if !hwnd.0.is_null() && unsafe { GetWindowRect(hwnd, &mut rc).is_ok() } && rc.right > rc.left {
        (rc.left, rc.top, rc.right, rc.bottom)
    } else {
        (-32000, -32000, 32000, 32000)
    }
}

/// Shared PowerShell helper for MS Excel (2016–365) & WPS Spreadsheets:
/// 1) Resolves `RangeFromPoint` in the target window's exact DPI coordinate space.
/// 2) Tests analytic `PointsToScreenPixelsX/Y` candidates (Excel 365 Per-Monitor DPI,
///    Excel 2016 96-DPI, WPS raw-points bug, Zoom variations), proving every candidate
///    against all 4 edges via `RangeFromPoint` inside/outside checks.
/// 3) If no analytic candidate passes (e.g. Freeze Panes, Split Windows, scrolled WPS,
///    mixed-DPI multi-monitor, RTL sheets), locks all 4 edges to ±1px directly via
///    exponential bracket + binary search on `RangeFromPoint`.
const EXCEL_RECT_SOLVER_PS: &str = r#"
function Try-ExcelCellAt($w, $cx, $cy) {
  try {
    $c = $w.RangeFromPoint([int]$cx, [int]$cy)
    if ($null -ne $c) {
      $a = [string]$c.Address($false, $false)
      if (-not [string]::IsNullOrWhiteSpace($a)) { return $c }
    }
  } catch {}
  return $null
}

function Try-ExcelCellWithSpace($w, $hx, $hy, $px, $py, $lx, $ly, $prefSpace) {
  $order = @()
  if ($prefSpace -eq 'hwnd') {
    $order = @(@($hx, $hy, 'hwnd'), @($px, $py, 'physical'), @($lx, $ly, 'div_dpi'))
  } elseif ($prefSpace -eq 'physical') {
    $order = @(@($px, $py, 'physical'), @($hx, $hy, 'hwnd'), @($lx, $ly, 'div_dpi'))
  } elseif ($prefSpace -eq 'div_dpi') {
    $order = @(@($lx, $ly, 'div_dpi'), @($hx, $hy, 'hwnd'), @($px, $py, 'physical'))
  } else {
    $order = @(@($hx, $hy, 'hwnd'), @($px, $py, 'physical'), @($lx, $ly, 'div_dpi'))
  }
  $seen = @{}
  foreach ($item in $order) {
    $cx = [int]$item[0]; $cy = [int]$item[1]; $sp = [string]$item[2]
    $key = "$cx,$cy"
    if ($seen.ContainsKey($key)) { continue }
    $seen[$key] = $true
    $c = Try-ExcelCellAt $w $cx $cy
    if ($null -ne $c) {
      return @{ cell = $c; x = $cx; y = $cy; space = $sp }
    }
  }
  return $null
}

function Test-InExcelRange($w, $px, $py, $rMin, $rMax, $cMin, $cMax) {
  try {
    $hc = $w.RangeFromPoint([int]$px, [int]$py)
    if ($null -eq $hc) { return $false }
    $hr = [int]$hc.Row
    $hcol = [int]$hc.Column
    return ($hr -ge $rMin -and $hr -le $rMax -and $hcol -ge $cMin -and $hcol -le $cMax)
  } catch { return $false }
}

function Verify-ExcelRect($w, $x1, $y1, $x2, $y2, $rMin, $rMax, $cMin, $cMax, $axL, $ayT, $axR, $ayB, $leftProbeY, $rightProbeY, $topProbeX, $bottomProbeX) {
  $bw = $x2 - $x1; $bh = $y2 - $y1
  if ($bw -lt 6 -or $bh -lt 6 -or $bw -gt 4800 -or $bh -gt 3000) { return $false }
  if ($axL -lt ($x1 - 2) -or $axL -gt ($x2 + 2) -or $axR -lt ($x1 - 2) -or $axR -gt ($x2 + 2)) { return $false }
  if ($ayT -lt ($y1 - 2) -or $ayT -gt ($y2 + 2) -or $ayB -lt ($y1 - 2) -or $ayB -gt ($y2 + 2)) { return $false }
  $inL = [Math]::Min($x2 - 1, $x1 + 2)
  $inR = [Math]::Max($x1 + 1, $x2 - 2)
  $inT = [Math]::Min($y2 - 1, $y1 + 2)
  $inB = [Math]::Max($y1 + 1, $y2 - 2)
  if (-not (Test-InExcelRange $w $inL $leftProbeY $rMin $rMax $cMin $cMax)) { return $false }
  if (Test-InExcelRange $w ($x1 - 3) $leftProbeY $rMin $rMax $cMin $cMax) { return $false }
  if (-not (Test-InExcelRange $w $inR $rightProbeY $rMin $rMax $cMin $cMax)) { return $false }
  if (Test-InExcelRange $w ($x2 + 3) $rightProbeY $rMin $rMax $cMin $cMax) { return $false }
  if (-not (Test-InExcelRange $w $topProbeX $inT $rMin $rMax $cMin $cMax)) { return $false }
  if (Test-InExcelRange $w $topProbeX ($y1 - 3) $rMin $rMax $cMin $cMax) { return $false }
  if (-not (Test-InExcelRange $w $bottomProbeX $inB $rMin $rMax $cMin $cMax)) { return $false }
  if (Test-InExcelRange $w $bottomProbeX ($y2 + 3) $rMin $rMax $cMin $cMax) { return $false }
  return $true
}

function Find-ExcelEdge($w, [int]$fixedCoord, [int]$startIn, [int]$dir, [int]$limit, [bool]$isHorizontal, [int]$rMin, [int]$rMax, [int]$cMin, [int]$cMax) {
  $inV = [int]$startIn
  $step = 16
  $outV = $inV
  $foundOut = $false
  for ($k = 0; $k -lt 9; $k++) {
    $probe = $inV + ($dir * $step)
    if (($dir -lt 0 -and $probe -lt $limit) -or ($dir -gt 0 -and $probe -gt $limit)) {
      $probe = $limit
    }
    $ok = if ($isHorizontal) {
      Test-InExcelRange $w $probe $fixedCoord $rMin $rMax $cMin $cMax
    } else {
      Test-InExcelRange $w $fixedCoord $probe $rMin $rMax $cMin $cMax
    }
    if (-not $ok) {
      $outV = $probe
      $foundOut = $true
      break
    }
    $inV = $probe
    if ($probe -eq $limit) { return $limit }
    $step = $step * 2
  }
  if (-not $foundOut) { return $inV }
  while ([Math]::Abs($outV - $inV) -gt 1) {
    $mid = [int][Math]::Floor(($inV + $outV) / 2.0)
    if ($mid -eq $inV -or $mid -eq $outV) { break }
    $ok = if ($isHorizontal) {
      Test-InExcelRange $w $mid $fixedCoord $rMin $rMax $cMin $cMax
    } else {
      Test-InExcelRange $w $fixedCoord $mid $rMin $rMax $cMin $cMax
    }
    if ($ok) { $inV = $mid } else { $outV = $mid }
  }
  return $inV
}

function Resolve-ExcelRangeRect($w, $rng, $ax1, $ay1, $ax2, $ay2, $dpiScale, $minX, $minY, $maxX, $maxY) {
  $rMin = [int]$rng.Row
  $cMin = [int]$rng.Column
  $rMax = $rMin + [int]$rng.Rows.Count - 1
  $cMax = $cMin + [int]$rng.Columns.Count - 1

  $axL = [Math]::Min([int]$ax1, [int]$ax2)
  $axR = [Math]::Max([int]$ax1, [int]$ax2)
  $ayT = [Math]::Min([int]$ay1, [int]$ay2)
  $ayB = [Math]::Max([int]$ay1, [int]$ay2)

  $leftProbeY = $ayT; $topProbeX = $axL
  if (-not (Test-InExcelRange $w $axL $ayT $rMin $rMax $cMin $cMax)) {
    $leftProbeY = if ($ax1 -le $ax2) { [int]$ay1 } else { [int]$ay2 }
    $topProbeX = if ($ay1 -le $ay2) { [int]$ax1 } else { [int]$ax2 }
  }
  $rightProbeY = $ayB; $bottomProbeX = $axR
  if (-not (Test-InExcelRange $w $axR $ayB $rMin $rMax $cMin $cMax)) {
    $rightProbeY = if ($ax2 -ge $ax1) { [int]$ay2 } else { [int]$ay1 }
    $bottomProbeX = if ($ay2 -ge $ay1) { [int]$ax2 } else { [int]$ax1 }
  }

  # Stage 1: Fast analytic PointsToScreenPixels candidates (including active Pane for Freeze/Split Panes)
  # with mandatory 4-edge RangeFromPoint proof.
  $cands = New-Object System.Collections.ArrayList
  $p2sTargets = New-Object System.Collections.ArrayList
  try {
    $leftPt = [double]$rng.Left
    $topPt = [double]$rng.Top
    $rightPt = $leftPt + [double]$rng.Width
    $bottomPt = $topPt + [double]$rng.Height
    try {
      if ($null -ne $w.Panes -and [int]$w.Panes.Count -gt 1) {
        foreach ($pane in $w.Panes) {
          try {
            $vr = $pane.VisibleRange
            if ($null -ne $vr -and $leftPt -ge [double]$vr.Left -and $leftPt -le ([double]$vr.Left + [double]$vr.Width) -and $topPt -ge [double]$vr.Top -and $topPt -le ([double]$vr.Top + [double]$vr.Height)) {
              [void]$p2sTargets.Add($pane)
              break
            }
          } catch {}
        }
      }
    } catch {}
    [void]$p2sTargets.Add($w)

    $zoom = 1.0
    try { $zv = [double]$w.Zoom; if ($zv -ge 10 -and $zv -le 400) { $zoom = $zv / 100.0 } } catch {}
    $ptToPx = (96.0 / 72.0) * $dpiScale * $zoom
    $ptToPx96 = (96.0 / 72.0) * $zoom

    foreach ($t in $p2sTargets) {
      try {
        $rx1 = [double]$t.PointsToScreenPixelsX($leftPt)
        $ry1 = [double]$t.PointsToScreenPixelsY($topPt)
        $rx2 = [double]$t.PointsToScreenPixelsX($rightPt)
        $ry2 = [double]$t.PointsToScreenPixelsY($bottomPt)
        $ox = [double]$t.PointsToScreenPixelsX(0.0)
        $oy = [double]$t.PointsToScreenPixelsY(0.0)
        [void]$cands.Add(@($rx1, $ry1, $rx2, $ry2))
        if ([Math]::Abs([double]$dpiScale - 1.0) -gt 0.01) {
          [void]$cands.Add(@(($rx1 * $dpiScale), ($ry1 * $dpiScale), ($rx2 * $dpiScale), ($ry2 * $dpiScale)))
          [void]$cands.Add(@(($ox + ($rx1 - $ox) * $dpiScale), ($oy + ($ry1 - $oy) * $dpiScale), ($ox + ($rx2 - $ox) * $dpiScale), ($oy + ($ry2 - $oy) * $dpiScale)))
        }
        [void]$cands.Add(@(($ox + $leftPt * $ptToPx), ($oy + $topPt * $ptToPx), ($ox + $rightPt * $ptToPx), ($oy + $bottomPt * $ptToPx)))
        [void]$cands.Add(@(($ox + ($rx1 - $ox) * $ptToPx), ($oy + ($ry1 - $oy) * $ptToPx), ($ox + ($rx2 - $ox) * $ptToPx), ($oy + ($ry2 - $oy) * $ptToPx)))
        if ([Math]::Abs([double]$dpiScale - 1.0) -gt 0.01) {
          [void]$cands.Add(@(($ox + ($rx1 - $ox) * $ptToPx96), ($oy + ($ry1 - $oy) * $ptToPx96), ($ox + ($rx2 - $ox) * $ptToPx96), ($oy + ($ry2 - $oy) * $ptToPx96)))
        }
      } catch {}
    }
  } catch {}

  foreach ($cand in $cands) {
    $cx1 = [int][Math]::Round($cand[0])
    $cy1 = [int][Math]::Round($cand[1])
    $cx2 = [int][Math]::Round($cand[2])
    $cy2 = [int][Math]::Round($cand[3])
    if (Verify-ExcelRect $w $cx1 $cy1 $cx2 $cy2 $rMin $rMax $cMin $cMax $axL $ayT $axR $ayB $leftProbeY $rightProbeY $topProbeX $bottomProbeX) {
      return @{ rect = @{ x = $cx1; y = $cy1; w = [Math]::Max(8, $cx2 - $cx1); h = [Math]::Max(8, $cy2 - $cy1) }; method = 'p2s_verified' }
    }
  }

  # Stage 2: Universal RangeFromPoint exponential bracket + binary search (±1px across all Excel 2016–365 & WPS)
  $leftIn = Find-ExcelEdge $w $leftProbeY $axL (-1) $minX $true $rMin $rMax $cMin $cMax
  $rightIn = Find-ExcelEdge $w $rightProbeY $axR 1 $maxX $true $rMin $rMax $cMin $cMax
  $topIn = Find-ExcelEdge $w $topProbeX $ayT (-1) $minY $false $rMin $rMax $cMin $cMax
  $bottomIn = Find-ExcelEdge $w $bottomProbeX $ayB 1 $maxY $false $rMin $rMax $cMin $cMax
  if ($rightIn -ge $leftIn -and $bottomIn -ge $topIn) {
    $sw = [Math]::Max(8, $rightIn - $leftIn + 1)
    $shPx = [Math]::Max(8, $bottomIn - $topIn + 1)
    if ($sw -lt 4800 -and $shPx -lt 3000) {
      return @{ rect = @{ x = $leftIn; y = $topIn; w = $sw; h = $shPx }; method = 'rfp_bsearch' }
    }
  }

  return $null
}
"#;

/// COM-backed Excel / WPS Spreadsheets single-cell hit-test & selection at `(x, y)`.
/// Returns `None` if `(x, y)` did not hit a worksheet cell (e.g. Ribbon/titlebar), allowing UIA fallback.
pub fn pick_excel_cell_com(
    broker: &BrokerClient,
    root_hwnd: HWND,
    x: i32,
    y: i32,
    dpi_scale: f64,
) -> Option<ExcelPickResult> {
    let scale = dpi_scale.max(0.5);
    let (hx, hy) = physical_to_window_logical(root_hwnd, x, y, scale);
    let lx = ((x as f64) / scale).round() as i32;
    let ly = ((y as f64) / scale).round() as i32;
    let (wl, wt, wr, wb) = query_window_bounds_tuple(root_hwnd);
    let (hwl, hwt) = physical_to_window_logical(root_hwnd, wl, wt, scale);
    let (hwr, hwb) = physical_to_window_logical(root_hwnd, wr, wb, scale);
    let lwl = ((wl as f64) / scale).round() as i32;
    let lwt = ((wt as f64) / scale).round() as i32;
    let lwr = ((wr as f64) / scale).round() as i32;
    let lwb = ((wb as f64) / scale).round() as i32;
    let solver = EXCEL_RECT_SOLVER_PS;

    let script = format!(
        r#"
{solver}
$app = $null
$usedProgId = ''
foreach ($prog in @('Excel.Application', 'KET.Application', 'Ket.Application')) {{
  try {{ $app = [Runtime.InteropServices.Marshal]::GetActiveObject($prog); if ($null -ne $app) {{ $usedProgId = $prog; break }} }} catch {{}}
}}
if ($null -eq $app -or $null -eq $app.ActiveWorkbook) {{ exit 0 }}
$wb = $app.ActiveWorkbook
$win = $app.ActiveWindow
$sh = $app.ActiveSheet

$hit = Try-ExcelCellWithSpace $win {hx} {hy} {x} {y} {lx} {ly} ''
if ($null -eq $hit) {{ exit 0 }}
$cell = $hit.cell
$rng = $cell
try {{ if ($cell.MergeCells) {{ $rng = $cell.MergeArea }} }} catch {{}}

$addr = ''
try {{ $addr = [string]$rng.Address($false, $false) }} catch {{ exit 0 }}
if ([string]::IsNullOrWhiteSpace($addr)) {{ exit 0 }}

try {{ [void]$rng.Select() }} catch {{}}
$row = [int]$cell.Row
$col = [int]$cell.Column
$disp = ''; try {{ $disp = [string]$cell.Text }} catch {{}}
$val = ''; try {{ if ($null -ne $cell.Value2) {{ $val = [string]$cell.Value2 }} }} catch {{}}
$formula = ''; try {{ $f = [string]$cell.Formula; if ($f.StartsWith('=')) {{ $formula = $f }} }} catch {{}}
$colHdr = ''; if ($row -gt 1) {{ try {{ $colHdr = ([string]$sh.Cells.Item(1, $col).Text).Trim() }} catch {{}} }}
$rowHdr = ''; if ($col -gt 1) {{ try {{ $rowHdr = ([string]$sh.Cells.Item($row, 1).Text).Trim() }} catch {{}} }}

$minX = if ($hit.space -eq 'hwnd') {{ {hwl} }} elseif ($hit.space -eq 'div_dpi') {{ {lwl} }} else {{ {wl} }}
$minY = if ($hit.space -eq 'hwnd') {{ {hwt} }} elseif ($hit.space -eq 'div_dpi') {{ {lwt} }} else {{ {wt} }}
$maxX = if ($hit.space -eq 'hwnd') {{ {hwr} }} elseif ($hit.space -eq 'div_dpi') {{ {lwr} }} else {{ {wr} }}
$maxY = if ($hit.space -eq 'hwnd') {{ {hwb} }} elseif ($hit.space -eq 'div_dpi') {{ {lwb} }} else {{ {wb} }}

$solved = Resolve-ExcelRangeRect $win $rng $hit.x $hit.y $hit.x $hit.y {scale} ($minX - 4) ($minY - 4) ($maxX + 4) ($maxY + 4)
$rectObj = if ($null -ne $solved) {{ $solved.rect }} else {{ $null }}
$rectMethod = if ($null -ne $solved) {{ [string]$solved.method }} else {{ '' }}

@{{
  progId = $usedProgId
  fullName = [string]$wb.FullName
  workbookName = [string]$wb.Name
  sheet = [string]$sh.Name
  address = $addr
  isRange = ($rng.Rows.Count -gt 1 -or $rng.Columns.Count -gt 1)
  rows = [int]$rng.Rows.Count
  cols = [int]$rng.Columns.Count
  displayText = $disp
  value = $val
  formula = $formula
  colHeader = $colHdr
  rowHeader = $rowHdr
  screenRect = $rectObj
  coordSpace = [string]$hit.space
  rectMethod = $rectMethod
}} | ConvertTo-Json -Compress -Depth 4
"#
    );

    let resp = broker.call(BrokerRequest::RunPs51 {
        script,
        timeout_ms: 1900,
    });
    if !resp.ok || resp.text.trim().is_empty() {
        return None;
    }
    let mut parsed = serde_json::from_str::<ExcelPickResult>(resp.text.trim()).ok()?;
    if parsed.address.is_empty() {
        return None;
    }
    if let Some(raw_rect) = parsed.screen_rect {
        let phys = map_office_rect_to_physical(root_hwnd, raw_rect, &parsed.coord_space, scale);
        parsed.screen_rect = validate_office_rect_against_anchors(phys, &[(x, y)], 10);
    }
    Some(parsed)
}

/// COM-backed Excel / WPS Spreadsheets cell-range box selection from `(x1, y1)` to `(x2, y2)`.
pub fn pick_excel_range_com(
    broker: &BrokerClient,
    root_hwnd: HWND,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    dpi_scale: f64,
) -> Option<ExcelPickResult> {
    let scale = dpi_scale.max(0.5);
    let (hx1, hy1) = physical_to_window_logical(root_hwnd, x1, y1, scale);
    let (hx2, hy2) = physical_to_window_logical(root_hwnd, x2, y2, scale);
    let lx1 = ((x1 as f64) / scale).round() as i32;
    let ly1 = ((y1 as f64) / scale).round() as i32;
    let lx2 = ((x2 as f64) / scale).round() as i32;
    let ly2 = ((y2 as f64) / scale).round() as i32;
    let (wl, wt, wr, wb) = query_window_bounds_tuple(root_hwnd);
    let (hwl, hwt) = physical_to_window_logical(root_hwnd, wl, wt, scale);
    let (hwr, hwb) = physical_to_window_logical(root_hwnd, wr, wb, scale);
    let lwl = ((wl as f64) / scale).round() as i32;
    let lwt = ((wt as f64) / scale).round() as i32;
    let lwr = ((wr as f64) / scale).round() as i32;
    let lwb = ((wb as f64) / scale).round() as i32;
    let solver = EXCEL_RECT_SOLVER_PS;

    let script = format!(
        r#"
{solver}
$app = $null
$usedProgId = ''
foreach ($prog in @('Excel.Application', 'KET.Application', 'Ket.Application')) {{
  try {{ $app = [Runtime.InteropServices.Marshal]::GetActiveObject($prog); if ($null -ne $app) {{ $usedProgId = $prog; break }} }} catch {{}}
}}
if ($null -eq $app -or $null -eq $app.ActiveWorkbook) {{ exit 0 }}
$wb = $app.ActiveWorkbook
$win = $app.ActiveWindow
$sh = $app.ActiveSheet

$hit1 = Try-ExcelCellWithSpace $win {hx1} {hy1} {x1} {y1} {lx1} {ly1} ''
$prefSpace = if ($null -ne $hit1) {{ [string]$hit1.space }} else {{ '' }}
$hit2 = Try-ExcelCellWithSpace $win {hx2} {hy2} {x2} {y2} {lx2} {ly2} $prefSpace
if ($null -eq $hit1 -and $null -eq $hit2) {{ exit 0 }}
if ($null -eq $hit1) {{ $hit1 = $hit2 }}
if ($null -eq $hit2) {{ $hit2 = $hit1 }}
$c1 = $hit1.cell
$c2 = $hit2.cell
$usedSpace = [string]$hit1.space

$rng = $sh.Range($c1, $c2)
try {{ [void]$rng.Select() }} catch {{}}

$addr = [string]$rng.Address($false, $false)
$rowCount = [int]$rng.Rows.Count
$colCount = [int]$rng.Columns.Count
$firstRow = [int]$rng.Row
$firstCol = [int]$rng.Column

$maxR = [Math]::Min($rowCount, 40)
$maxC = [Math]::Min($colCount, 15)
$tableLines = New-Object System.Collections.Generic.List[string]
$formulaLines = New-Object System.Collections.Generic.List[string]

# Read entire preview sub-range in 2 bulk COM SafeArray calls (Value2 + Formula) instead of
# O(rows*cols) per-cell COM calls (100x faster on large selections in Excel & WPS Spreadsheets!).
$subEndRow = $firstRow + $maxR - 1
$subEndCol = $firstCol + $maxC - 1
$subRng = $sh.Range($sh.Cells.Item($firstRow, $firstCol), $sh.Cells.Item($subEndRow, $subEndCol))

# Include row-1 column headers if selection starts below row 1 (1 bulk COM call)
if ($firstRow -gt 1) {{
  $hdrCells = @()
  try {{
    $hdrRng = $sh.Range($sh.Cells.Item(1, $firstCol), $sh.Cells.Item(1, $subEndCol))
    $hv = $hdrRng.Value2
    if ($hv -is [System.Array] -and $hv.Rank -eq 2) {{
      $lb2 = $hv.GetLowerBound(1)
      for ($ci = 0; $ci -lt $maxC; $ci++) {{
        $raw = $hv.GetValue($hv.GetLowerBound(0), $lb2 + $ci)
        $hdrCells += if ($null -eq $raw) {{ '' }} else {{ ([string]$raw -replace "[\r\n]+", " ").Trim() }}
      }}
    }} else {{
      $hdrCells += if ($null -eq $hv) {{ '' }} else {{ ([string]$hv -replace "[\r\n]+", " ").Trim() }}
    }}
  }} catch {{}}
  if (($hdrCells -join '').Length -gt 0) {{
    $tableLines.Add("Headers(Row1): " + ($hdrCells -join " | "))
  }}
}}

$bulkVals = $null
$bulkForms = $null
try {{ $bulkVals = $subRng.Value2 }} catch {{}}
try {{ $bulkForms = $subRng.Formula }} catch {{}}

$has2dVals = ($bulkVals -is [System.Array] -and $bulkVals.Rank -eq 2)
$has2dForms = ($bulkForms -is [System.Array] -and $bulkForms.Rank -eq 2)
$vlb0 = if ($has2dVals) {{ $bulkVals.GetLowerBound(0) }} else {{ 0 }}
$vlb1 = if ($has2dVals) {{ $bulkVals.GetLowerBound(1) }} else {{ 0 }}
$flb0 = if ($has2dForms) {{ $bulkForms.GetLowerBound(0) }} else {{ 0 }}
$flb1 = if ($has2dForms) {{ $bulkForms.GetLowerBound(1) }} else {{ 0 }}

for ($ri = 0; $ri -lt $maxR; $ri++) {{
  $rowCells = @()
  for ($ci = 0; $ci -lt $maxC; $ci++) {{
    $rawV = if ($has2dVals) {{
      $bulkVals.GetValue($vlb0 + $ri, $vlb1 + $ci)
    }} elseif ($ri -eq 0 -and $ci -eq 0) {{
      $bulkVals
    }} else {{ $null }}
    $t = if ($null -eq $rawV) {{ '' }} else {{ ([string]$rawV -replace "[\r\n]+", " ").Trim() }}
    $rowCells += $t
    if ($formulaLines.Count -lt 20) {{
      $rawF = if ($has2dForms) {{
        $bulkForms.GetValue($flb0 + $ri, $flb1 + $ci)
      }} elseif ($ri -eq 0 -and $ci -eq 0) {{
        $bulkForms
      }} else {{ $null }}
      if ($null -ne $rawF) {{
        $fs = [string]$rawF
        if ($fs.StartsWith('=')) {{
          $colNum = $firstCol + $ci
          $colName = ''
          while ($colNum -gt 0) {{
            $rem = ($colNum - 1) % 26
            $colName = [char](65 + $rem) + $colName
            $colNum = [math]::Floor(($colNum - 1) / 26)
          }}
          $formulaLines.Add("$colName$($firstRow + $ri) = $fs")
        }}
      }}
    }}
  }}
  $tableLines.Add(($rowCells -join " | "))
}}
if ($rowCount -gt $maxR -or $colCount -gt $maxC) {{
  $tableLines.Add("... (truncated: showing ${{maxR}}x${{maxC}} of ${{rowCount}}x${{colCount}})")
}}

$minX = if ($usedSpace -eq 'hwnd') {{ {hwl} }} elseif ($usedSpace -eq 'div_dpi') {{ {lwl} }} else {{ {wl} }}
$minY = if ($usedSpace -eq 'hwnd') {{ {hwt} }} elseif ($usedSpace -eq 'div_dpi') {{ {lwt} }} else {{ {wt} }}
$maxX = if ($usedSpace -eq 'hwnd') {{ {hwr} }} elseif ($usedSpace -eq 'div_dpi') {{ {lwr} }} else {{ {wr} }}
$maxY = if ($usedSpace -eq 'hwnd') {{ {hwb} }} elseif ($usedSpace -eq 'div_dpi') {{ {lwb} }} else {{ {wb} }}

$solved = Resolve-ExcelRangeRect $win $rng $hit1.x $hit1.y $hit2.x $hit2.y {scale} ($minX - 4) ($minY - 4) ($maxX + 4) ($maxY + 4)
$rectObj = if ($null -ne $solved) {{ $solved.rect }} else {{ $null }}
$rectMethod = if ($null -ne $solved) {{ [string]$solved.method }} else {{ '' }}

@{{
  progId = $usedProgId
  fullName = [string]$wb.FullName
  workbookName = [string]$wb.Name
  sheet = [string]$sh.Name
  address = $addr
  isRange = ($rowCount -gt 1 -or $colCount -gt 1)
  rows = $rowCount
  cols = $colCount
  displayText = if ($rowCount -eq 1 -and $colCount -eq 1) {{ [string]$c1.Text }} else {{ "" }}
  tablePreview = ($tableLines -join "`n")
  formulasPreview = ($formulaLines -join "; ")
  screenRect = $rectObj
  coordSpace = $usedSpace
  rectMethod = $rectMethod
}} | ConvertTo-Json -Compress -Depth 4
"#
    );

    let resp = broker.call(BrokerRequest::RunPs51 {
        script,
        timeout_ms: 3500,
    });
    if !resp.ok || resp.text.trim().is_empty() {
        log::warn!(
            "pick_excel_range_com returned empty/err (ok={}, err={:?})",
            resp.ok,
            resp.error
        );
        return None;
    }
    let mut parsed = serde_json::from_str::<ExcelPickResult>(resp.text.trim()).ok()?;
    if parsed.address.is_empty() {
        return None;
    }
    if let Some(raw_rect) = parsed.screen_rect {
        let phys = map_office_rect_to_physical(root_hwnd, raw_rect, &parsed.coord_space, scale);
        // Validate against physical drag points when both endpoints landed inside the grid,
        // or at least one endpoint if the user dragged slightly outside the sheet viewport.
        let both_ok = validate_office_rect_against_anchors(phys, &[(x1, y1), (x2, y2)], 12);
        let one_ok = validate_office_rect_against_anchors(phys, &[(x1, y1)], 12)
            .or_else(|| validate_office_rect_against_anchors(phys, &[(x2, y2)], 12));
        parsed.screen_rect = both_ok.or(one_ok);
    }
    Some(parsed)
}

/// COM-backed PowerPoint / WPS Presentation single design-element hit-test at `(x, y)`.
/// Uses `ActiveWindow.RangeFromPoint(x, y)` first, then falls back to reverse Z-order
/// `PointsToScreenPixelsX/Y` hit-testing (with `GroupItems` drill-down) for full WPS compatibility.
pub fn pick_ppt_element_com(
    broker: &BrokerClient,
    root_hwnd: HWND,
    x: i32,
    y: i32,
    dpi_scale: f64,
) -> Option<PptElementPickResult> {
    let scale = dpi_scale.max(0.5);
    let (hx, hy) = physical_to_window_logical(root_hwnd, x, y, scale);
    let lx = ((x as f64) / scale).round() as i32;
    let ly = ((y as f64) / scale).round() as i32;

    let script = format!(
        r#"
$app = $null
$usedProgId = ''
foreach ($prog in @('PowerPoint.Application', 'KWPP.Application', 'Kwpp.Application')) {{
  try {{ $app = [Runtime.InteropServices.Marshal]::GetActiveObject($prog); if ($null -ne $app) {{ $usedProgId = $prog; break }} }} catch {{}}
}}
if ($null -eq $app -or $null -eq $app.ActivePresentation) {{ exit 0 }}
$pres = $app.ActivePresentation
$win = $app.ActiveWindow
$slide = $null
try {{ $slide = $win.View.Slide }} catch {{}}
if ($null -eq $slide) {{ exit 0 }}

function Get-ShapeRectCandidates($w, $sh, $dpiScale) {{
  $list = @()
  try {{
    $l = [double]$sh.Left; $t = [double]$sh.Top
    $r = $l + [double]$sh.Width; $b = $t + [double]$sh.Height
    $x1 = [double]$w.PointsToScreenPixelsX($l)
    $y1 = [double]$w.PointsToScreenPixelsY($t)
    $x2 = [double]$w.PointsToScreenPixelsX($r)
    $y2 = [double]$w.PointsToScreenPixelsY($b)
    $ox = [double]$w.PointsToScreenPixelsX(0.0)
    $oy = [double]$w.PointsToScreenPixelsY(0.0)
    $zoom = 1.0
    try {{ $zv = [double]$w.View.Zoom; if ($zv -ge 10 -and $zv -le 400) {{ $zoom = $zv / 100.0 }} }} catch {{}}
    $ptToPx = (96.0 / 72.0) * $dpiScale * $zoom
    $rawCands = @(
      @($x1, $y1, $x2, $y2),
      @(($x1 * $dpiScale), ($y1 * $dpiScale), ($x2 * $dpiScale), ($y2 * $dpiScale)),
      @(($ox + ($x1 - $ox) * $dpiScale), ($oy + ($y1 - $oy) * $dpiScale), ($ox + ($x2 - $ox) * $dpiScale), ($oy + ($y2 - $oy) * $dpiScale)),
      @(($ox + ($x1 - $ox) * $ptToPx), ($oy + ($y1 - $oy) * $ptToPx), ($ox + ($x2 - $ox) * $ptToPx), ($oy + ($y2 - $oy) * $ptToPx))
    )
    foreach ($c in $rawCands) {{
      $ix1 = [int][Math]::Round($c[0]); $iy1 = [int][Math]::Round($c[1])
      $ix2 = [int][Math]::Round($c[2]); $iy2 = [int][Math]::Round($c[3])
      if (($ix2 - $ix1) -ge 4 -and ($iy2 - $iy1) -ge 4) {{
        $list += ,@{{ x = $ix1; y = $iy1; w = [Math]::Max(8, $ix2 - $ix1); h = [Math]::Max(8, $iy2 - $iy1); x2 = $ix2; y2 = $iy2 }}
      }}
    }}
  }} catch {{}}
  return $list
}}

function Get-ShapeRectForPoint($w, $sh, $px, $py, $dpiScale) {{
  $cands = Get-ShapeRectCandidates $w $sh $dpiScale
  foreach ($rc in $cands) {{
    if ($px -ge ($rc.x - 4) -and $px -le ($rc.x2 + 4) -and $py -ge ($rc.y - 4) -and $py -le ($rc.y2 + 4)) {{
      return $rc
    }}
  }}
  if ($cands.Count -gt 0) {{ return $cands[0] }}
  return $null
}}

function Drill-Group($w, $sh, $px, $py, $dpiScale) {{
  try {{
    if ([int]$sh.Type -eq 6 -and $sh.GroupItems.Count -gt 0) {{
      $bestChild = $null
      $bestArea = [double]::MaxValue
      for ($i = $sh.GroupItems.Count; $i -ge 1; $i--) {{
        $ch = $sh.GroupItems.Item($i)
        $rc = Get-ShapeRectForPoint $w $ch $px $py $dpiScale
        if ($null -ne $rc -and $px -ge $rc.x -and $px -le $rc.x2 -and $py -ge $rc.y -and $py -le $rc.y2) {{
          $area = [double]$rc.w * [double]$rc.h
          if ($area -lt $bestArea) {{
            $bestArea = $area
            $bestChild = $ch
          }}
        }}
      }}
      if ($null -ne $bestChild) {{ return $bestChild }}
    }}
  }} catch {{}}
  return $sh
}}

$target = $null
$hitX = {hx}; $hitY = {hy}; $hitSpace = 'hwnd'
$ptList = @(@({hx}, {hy}, 'hwnd'), @({x}, {y}, 'physical'), @({lx}, {ly}, 'div_dpi'))
# 1. Direct COM RangeFromPoint
foreach ($pt in $ptList) {{
  if ($null -ne $target) {{ break }}
  try {{
    $hit = $win.RangeFromPoint([int]$pt[0], [int]$pt[1])
    if ($null -ne $hit) {{
      $hasName = $false
      try {{ $n = [string]$hit.Name; if (-not [string]::IsNullOrWhiteSpace($n)) {{ $hasName = $true }} }} catch {{}}
      if ($hasName) {{
        $hitX = [int]$pt[0]; $hitY = [int]$pt[1]; $hitSpace = [string]$pt[2]
        $target = Drill-Group $win $hit $hitX $hitY {scale}
      }} else {{
        try {{
          $owner = $hit.Parent.Parent
          if ($null -ne $owner -and -not [string]::IsNullOrWhiteSpace([string]$owner.Name)) {{
            $hitX = [int]$pt[0]; $hitY = [int]$pt[1]; $hitSpace = [string]$pt[2]
            $target = $owner
          }}
        }} catch {{}}
      }}
    }}
  }} catch {{}}
}}

# 2. Geometric Z-order fallback (works on WPS Kwpp.Application and all shape types)
if ($null -eq $target) {{
  $best = $null
  $bestZ = -1
  $bestArea = [double]::MaxValue
  foreach ($pt in $ptList) {{
    if ($null -ne $best) {{ break }}
    for ($i = $slide.Shapes.Count; $i -ge 1; $i--) {{
      $sh = $slide.Shapes.Item($i)
      try {{ if (-not $sh.Visible) {{ continue }} }} catch {{}}
      $rc = Get-ShapeRectForPoint $win $sh ([int]$pt[0]) ([int]$pt[1]) {scale}
      if ($null -ne $rc -and [int]$pt[0] -ge $rc.x -and [int]$pt[0] -le $rc.x2 -and [int]$pt[1] -ge $rc.y -and [int]$pt[1] -le $rc.y2) {{
        $cand = Drill-Group $win $sh ([int]$pt[0]) ([int]$pt[1]) {scale}
        $crc = Get-ShapeRectForPoint $win $cand ([int]$pt[0]) ([int]$pt[1]) {scale}
        $z = 0; try {{ $z = [int]$sh.ZOrderPosition }} catch {{}}
        $area = if ($null -ne $crc) {{ [double]$crc.w * [double]$crc.h }} else {{ [double]$rc.w * [double]$rc.h }}
        if ($z -gt $bestZ -or ($z -eq $bestZ -and $area -lt $bestArea)) {{
          $best = $cand
          $bestZ = $z
          $bestArea = $area
          $hitX = [int]$pt[0]; $hitY = [int]$pt[1]; $hitSpace = [string]$pt[2]
        }}
      }}
    }}
  }}
  $target = $best
}}

if ($null -eq $target) {{ exit 0 }}

try {{ [void]$target.Select() }} catch {{}}

$shapeTypeMap = @{{
  1='AutoShape'; 2='Callout'; 3='Chart'; 4='Comment'; 5='Freeform'; 6='Group';
  7='EmbeddedOLE'; 8='FormControl'; 9='Line'; 10='LinkedOLE'; 11='LinkedPicture';
  12='OLEControl'; 13='Picture'; 14='Placeholder'; 15='TextEffect'; 16='Media';
  17='TextBox'; 19='Table'; 24='SmartArt'; 28='Graphic'
}}
$typeCode = 0; try {{ $typeCode = [int]$target.Type }} catch {{}}
$typeName = if ($shapeTypeMap.ContainsKey($typeCode)) {{ $shapeTypeMap[$typeCode] }} else {{ "Shape($typeCode)" }}

$txt = ''
try {{
  if ($target.HasTextFrame -and $target.TextFrame.HasText) {{
    $txt = [string]$target.TextFrame.TextRange.Text
  }}
}} catch {{}}
if ([string]::IsNullOrWhiteSpace($txt)) {{
  try {{
    if ($target.HasTextFrame2 -and $target.TextFrame2.HasText) {{
      $txt = [string]$target.TextFrame2.TextRange.Text
    }}
  }} catch {{}}
}}
if ([string]::IsNullOrWhiteSpace($txt)) {{
  try {{
    if ($target.HasTable) {{
      $tbl = $target.Table
      $tLines = @()
      $mr = [Math]::Min([int]$tbl.Rows.Count, 20)
      $mc = [Math]::Min([int]$tbl.Columns.Count, 10)
      for ($r = 1; $r -le $mr; $r++) {{
        $rowVals = @()
        for ($c = 1; $c -le $mc; $c++) {{
          $ct = ''; try {{ $ct = ([string]$tbl.Cell($r, $c).Shape.TextFrame.TextRange.Text -replace "[\r\n]+", " ").Trim() }} catch {{}}
          $rowVals += $ct
        }}
        $tLines += ($rowVals -join " | ")
      }}
      $txt = $tLines -join "`n"
    }}
  }} catch {{}}
}}
$txt = ($txt -replace "`r", "`n").Trim()
if ($txt.Length -gt 2000) {{ $txt = $txt.Substring(0, 2000) }}

$geom = ''
try {{
  $geom = "left=$([Math]::Round([double]$target.Left,1))pt top=$([Math]::Round([double]$target.Top,1))pt size=$([Math]::Round([double]$target.Width,1))x$([Math]::Round([double]$target.Height,1))pt rot=$([Math]::Round([double]$target.Rotation,1))deg"
}} catch {{}}

$styleParts = @()
try {{
  if ($target.HasTextFrame -and $target.TextFrame.HasText) {{
    $fn = [string]$target.TextFrame.TextRange.Font.Name
    $fs = [double]$target.TextFrame.TextRange.Font.Size
    if ($fn) {{ $styleParts += "font=$fn ${{fs}}pt" }}
  }}
}} catch {{}}
try {{
  if ($target.Fill.Visible -and $target.Fill.Type -eq 1) {{
    $rgb = [int]$target.Fill.ForeColor.RGB
    $rCh = $rgb -band 0xFF; $gCh = ($rgb -shr 8) -band 0xFF; $bCh = ($rgb -shr 16) -band 0xFF
    $styleParts += ("fill=#{{0:X2}}{{1:X2}}{{2:X2}}" -f $rCh, $gCh, $bCh)
  }}
}} catch {{}}

$parentGroupId = $null
$parentGroupName = ''
try {{
  if ($null -ne $target.ParentGroup) {{
    $parentGroupId = [int]$target.ParentGroup.Id
    $parentGroupName = [string]$target.ParentGroup.Name
  }}
}} catch {{}}

$finalRc = Get-ShapeRectForPoint $win $target $hitX $hitY {scale}
$rectObj = if ($null -ne $finalRc) {{ @{{ x = $finalRc.x; y = $finalRc.y; w = $finalRc.w; h = $finalRc.h }} }} else {{ $null }}

@{{
  progId = $usedProgId
  fullName = [string]$pres.FullName
  presName = [string]$pres.Name
  slideIndex = [int]$slide.SlideIndex
  slideCount = [int]$pres.Slides.Count
  shapeName = [string]$target.Name
  shapeId = [int]$target.Id
  parentGroupId = $parentGroupId
  parentGroupName = $parentGroupName
  shapeType = $typeName
  text = $txt
  geometry = $geom
  style = ($styleParts -join ", ")
  screenRect = $rectObj
  coordSpace = $hitSpace
}} | ConvertTo-Json -Compress -Depth 4
"#
    );

    let resp = broker.call(BrokerRequest::RunPs51 {
        script,
        timeout_ms: 1700,
    });
    if !resp.ok || resp.text.trim().is_empty() {
        return None;
    }
    let mut parsed = serde_json::from_str::<PptElementPickResult>(resp.text.trim()).ok()?;
    if parsed.shape_name.is_empty() && parsed.text.is_empty() {
        return None;
    }
    if let Some(raw_rect) = parsed.screen_rect {
        let phys = map_office_rect_to_physical(root_hwnd, raw_rect, &parsed.coord_space, scale);
        parsed.screen_rect = validate_office_rect_against_anchors(phys, &[(x, y)], 24);
    }
    Some(parsed)
}

pub fn run(ctx: &AdapterContext) -> Option<AdapterOutput> {
    let exe = ctx.proc_meta.exe_name.as_str();
    let kind = detect_office_app(exe, &ctx.window_title, &ctx.window_class, "");
    let script = match kind {
        Some(OfficeAppKind::Excel) => excel_com_script(),
        Some(OfficeAppKind::PowerPoint) => ppt_com_script(),
        _ => word_or_wps_com_script(),
    };

    let resp = ctx.broker.call(BrokerRequest::RunPs51 {
        script: script.to_string(),
        timeout_ms: 1300,
    });

    let mut out = AdapterOutput::new("office");
    if resp.ok && !resp.text.trim().is_empty() {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&resp.text) {
            let full_name_str = val
                .get("fullName")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !full_name_str.is_empty() {
                out.lines
                    .push(format!("office_document_path: {full_name_str}"));
                if Path::new(&full_name_str).exists() {
                    out.paths.push(full_name_str.clone());
                }
                out.badge = Path::new(&full_name_str)
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string());
            }
            if let Some(loc) = val.get("location").and_then(|v| v.as_str()) {
                if !loc.is_empty() {
                    out.lines.push(format!("office_location: {loc}"));
                }
            }
            if let Some(sel) = val.get("selection").and_then(|v| v.as_str()) {
                let trimmed = sel.trim();
                if !trimmed.is_empty() {
                    out.lines.push(format!("office_selection:\n{trimmed}"));
                }
            }
            let app_arg = match kind {
                Some(OfficeAppKind::Excel) => "excel",
                Some(OfficeAppKind::PowerPoint) => "ppt",
                _ => "word",
            };
            let loc = val.get("location").and_then(|v| v.as_str()).unwrap_or("");
            let target_locator = if !full_name_str.is_empty() && !loc.is_empty() {
                format!("{full_name_str} | {loc}")
            } else if !full_name_str.is_empty() {
                full_name_str
            } else {
                loc.to_string()
            };
            let is_wps = matches!(exe, "wps.exe" | "wpsoffice.exe" | "et.exe" | "wpp.exe");
            out.lines
                .extend(format_live_edit_guidance(app_arg, is_wps, &target_locator));
        }
    }

    if out.lines.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn word_or_wps_com_script() -> &'static str {
    r#"
$app = $null
foreach ($prog in @('Word.Application', 'Kwps.Application', 'Ket.Application', 'Kwpp.Application')) {
  try { $app = [Runtime.InteropServices.Marshal]::GetActiveObject($prog); if ($null -ne $app) { break } } catch {}
}
if ($null -ne $app) {
  try {
    if ($null -ne $app.ActiveDocument) {
      $doc = $app.ActiveDocument
      $selText = ''
      try { $selText = ([string]$app.Selection.Text -replace "`r", "`n").Trim(); if ($selText.Length -gt 1200) { $selText = $selText.Substring(0, 1200) } } catch {}
      @{ fullName = [string]$doc.FullName; location = "Document: $($doc.Name)"; selection = $selText } | ConvertTo-Json -Compress
      exit 0
    }
    if ($null -ne $app.ActiveWorkbook) {
      $wb = $app.ActiveWorkbook
      $sh = $app.ActiveSheet.Name
      $addr = $app.Selection.Address($false, $false)
      $val = ''
      try { $val = [string]$app.ActiveCell.Formula } catch {}
      @{ fullName = [string]$wb.FullName; location = "Sheet: $sh Range: $addr"; selection = $val } | ConvertTo-Json -Compress
      exit 0
    }
  } catch {}
}
"#
}

fn excel_com_script() -> &'static str {
    r#"
$app = $null
foreach ($prog in @('Excel.Application', 'Ket.Application')) {
  try { $app = [Runtime.InteropServices.Marshal]::GetActiveObject($prog); if ($null -ne $app) { break } } catch {}
}
if ($null -ne $app -and $null -ne $app.ActiveWorkbook) {
  $wb = $app.ActiveWorkbook
  $sh = ''
  $addr = ''
  $formula = ''
  try { $sh = [string]$app.ActiveSheet.Name } catch {}
  try { $addr = [string]$app.Selection.Address($false, $false) } catch {}
  try { $formula = [string]$app.ActiveCell.Formula } catch {}
  @{ fullName = [string]$wb.FullName; location = "Sheet: $sh Range: $addr"; selection = $formula } | ConvertTo-Json -Compress
}
"#
}

fn ppt_com_script() -> &'static str {
    r#"
$app = $null
foreach ($prog in @('PowerPoint.Application', 'Kwpp.Application')) {
  try { $app = [Runtime.InteropServices.Marshal]::GetActiveObject($prog); if ($null -ne $app) { break } } catch {}
}
if ($null -ne $app -and $null -ne $app.ActivePresentation) {
  $pres = $app.ActivePresentation
  $slideIdx = ''
  try { $slideIdx = [string]$app.ActiveWindow.View.Slide.SlideIndex } catch {}
  @{ fullName = [string]$pres.FullName; location = "Slide #$slideIdx / $($pres.Slides.Count)"; selection = '' } | ConvertTo-Json -Compress
}
"#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_ms_office_and_wps_multitab_apps_and_drag_modes() {
        assert_eq!(
            detect_office_app("winword.exe", "Report.docx - Word", "OpusApp", "_WwG"),
            Some(OfficeAppKind::Word)
        );
        assert_eq!(
            OfficeAppKind::Word.drag_mode(),
            OfficeDragMode::WordTextSelect
        );

        assert_eq!(
            detect_office_app("excel.exe", "Budget.xlsx - Excel", "XLMAIN", "EXCEL7"),
            Some(OfficeAppKind::Excel)
        );
        assert_eq!(
            detect_office_app("et.exe", "季度报表.xlsx - WPS 表格", "XLMAIN", ""),
            Some(OfficeAppKind::Excel)
        );
        assert_eq!(
            detect_office_app("wps.exe", "2026财报.xlsx - WPS Office", "Qt5QWindow", ""),
            Some(OfficeAppKind::Excel)
        );
        assert_eq!(
            OfficeAppKind::Excel.drag_mode(),
            OfficeDragMode::ExcelCellBox
        );

        assert_eq!(
            detect_office_app(
                "powerpnt.exe",
                "Deck.pptx - PowerPoint",
                "PPTFrameClass",
                "paneClassDC"
            ),
            Some(OfficeAppKind::PowerPoint)
        );
        assert_eq!(
            detect_office_app("wpp.exe", "方案汇报.pptx - WPS 演示", "WPPMain", ""),
            Some(OfficeAppKind::PowerPoint)
        );
        assert_eq!(
            detect_office_app("wps.exe", "产品路演.pptx - WPS Office", "Qt5QWindow", ""),
            Some(OfficeAppKind::PowerPoint)
        );
        // PowerPoint drag MUST stay red freehand stroke!
        assert_eq!(
            OfficeAppKind::PowerPoint.drag_mode(),
            OfficeDragMode::NormalStroke
        );

        assert_eq!(
            detect_office_app("wps.exe", "工作簿1 - WPS Office", "Qt5QWindow", ""),
            Some(OfficeAppKind::Excel)
        );
        assert_eq!(
            detect_office_app("wps.exe", "演示文稿1 - WPS Office", "Qt5QWindow", ""),
            Some(OfficeAppKind::PowerPoint)
        );
        assert_eq!(
            detect_office_app("wps.exe", "合同正文.docx - WPS Office", "Qt5QWindow", ""),
            Some(OfficeAppKind::Word)
        );
        assert_eq!(
            detect_office_app(
                "msedge.exe",
                "Budget.xlsx - OneDrive",
                "Chrome_WidgetWin_1",
                ""
            ),
            None
        );
    }

    #[test]
    fn parses_excel_and_ppt_com_json_payloads() {
        let excel_json = r#"{"fullName":"D:\\work\\q1.xlsx","workbookName":"q1.xlsx","sheet":"Sheet1","address":"B2:D4","isRange":true,"rows":3,"cols":3,"tablePreview":"A | B | C","formulasPreview":"D4 = =SUM(D2:D3)","screenRect":{"x":200,"y":300,"w":240,"h":72}}"#;
        let ex: ExcelPickResult = serde_json::from_str(excel_json).unwrap();
        assert_eq!(ex.sheet, "Sheet1");
        assert_eq!(ex.address, "B2:D4");
        assert!(ex.is_range);
        assert_eq!(ex.rows, 3);
        assert_eq!(ex.screen_rect.unwrap().w, 240);

        let ppt_json = r#"{"progId":"KWPP.Application","fullName":"D:\\work\\deck.pptx","presName":"deck.pptx","slideIndex":3,"slideCount":12,"shapeName":"TextBox 4","shapeId":5,"parentGroupId":9,"parentGroupName":"Group 9","shapeType":"TextBox","text":"核心架构设计","geometry":"left=40pt top=80pt size=320x60pt rot=0deg","style":"font=微软雅黑 24pt","screenRect":{"x":310,"y":240,"w":420,"h":84}}"#;
        let ppt: PptElementPickResult = serde_json::from_str(ppt_json).unwrap();
        assert_eq!(ppt.prog_id, "KWPP.Application");
        assert_eq!(ppt.slide_index, 3);
        assert_eq!(ppt.shape_name, "TextBox 4");
        assert_eq!(ppt.parent_group_id, Some(9));
        assert_eq!(ppt.shape_type, "TextBox");
        assert_eq!(ppt.text, "核心架构设计");

        let word_json = r#"{"progId":"Word.Application","fullName":"D:\\work\\spec.docx","docName":"spec.docx","selectedText":"原文条款","pageNumber":"2","paragraphText":"第三条 原文条款","rangeStart":120,"rangeEnd":124}"#;
        let wd: WordTextPickResult = serde_json::from_str(word_json).unwrap();
        assert_eq!(wd.prog_id, "Word.Application");
        assert_eq!(wd.range_start, Some(120));
        assert_eq!(wd.range_end, Some(124));
        assert_eq!(wd.selected_text, "原文条款");

        let guidance =
            format_live_edit_guidance("word", false, "D:\\work\\spec.docx | range=120..124");
        assert_eq!(guidance.len(), 2);
        assert!(guidance[0].contains("python-docx"));
        assert!(guidance[0].contains("word-live"));
        assert!(guidance[1].contains("range=120..124"));

        let wps_guidance =
            format_live_edit_guidance("excel", true, "D:\\work\\q1.xlsx | Sheet1!B2:D4");
        assert!(wps_guidance[0].contains("wps-office"));
    }

    #[test]
    fn validates_and_reconciles_office_screen_rects_against_dpi_and_wps_drift() {
        // Real WPS Spreadsheets raw-point drift case from 125% DPI capture:
        // drag anchors (1775, 663) -> (1946, 828), broken p2s rect [1676, 591, 144, 116]
        let broken_wps_rect = ScreenRect {
            x: 1676,
            y: 591,
            w: 144,
            h: 116,
        };
        assert!(
            validate_office_rect_against_anchors(broken_wps_rect, &[(1775, 663), (1946, 828)], 12)
                .is_none(),
            "drifted WPS raw-points rect must be rejected"
        );

        // True physical C7:E14 rect containing both anchors passes cleanly
        let true_c7_e14 = ScreenRect {
            x: 1740,
            y: 649,
            w: 240,
            h: 193,
        };
        let checked =
            validate_office_rect_against_anchors(true_c7_e14, &[(1775, 663), (1946, 828)], 12)
                .expect("true C7:E14 rect should pass anchor validation");
        assert_eq!((checked.x, checked.y, checked.w, checked.h), (1740, 649, 240, 193));

        // div_dpi inverse transform restores 96-DPI virtualized rect to physical screen pixels
        let logical_rect = ScreenRect {
            x: 1392,
            y: 519,
            w: 192,
            h: 154,
        };
        let mapped = map_office_rect_to_physical(HWND::default(), logical_rect, "div_dpi", 1.25);
        assert_eq!(mapped.x, 1740);
        assert_eq!(mapped.y, 649);
        assert_eq!(mapped.w, 240);
        assert_eq!(mapped.h, 193);
    }

    #[test]
    fn excel_rect_solver_ps_runs_mock_wps_raw_points_and_freeze_pane_bsearch() {
        // End-to-end PowerShell 5.1 verification of EXCEL_RECT_SOLVER_PS with mock Window COM objects:
        // Case 1: WPS raw-points bug (PointsToScreenPixels adds 1:1 pt at 125% DPI) -> p2s_verified
        // Case 2: Frozen pane where PointsToScreenPixels is shifted by +200px -> rfp_bsearch locks ±1px!
        let test_script = format!(
            r#"
{solver}
# Mock Window with true physical cell grid: origin=(1580,504), colW=80px (48pt), rowH=24px (14.4pt) at 125% DPI (100% zoom)
# Target range C7:E14 => rows 7..14, cols 3..5 => true physical box x=[1740,1980), y=[648,840) (w=240, h=192)
function New-MockWin($brokenMode) {{
  $w = [pscustomobject]@{{ Zoom = 100; BrokenMode = $brokenMode }}
  $w | Add-Member -MemberType ScriptMethod -Name RangeFromPoint -Value {{
    param([int]$px, [int]$py)
    if ($px -lt 1580 -or $py -lt 504) {{ return $null }}
    $c = [int][Math]::Floor(($px - 1580) / 80.0) + 1
    $r = [int][Math]::Floor(($py - 504) / 24.0) + 1
    return [pscustomobject]@{{ Row = $r; Column = $c; Address = {{ param($a,$b) "R${{r}}C${{c}}" }} }}
  }}
  $w | Add-Member -MemberType ScriptMethod -Name PointsToScreenPixelsX -Value {{
    param([double]$pt)
    if ($this.BrokenMode -eq 'wps_raw_pt') {{ return [int][Math]::Round(1580.0 + $pt) }}
    if ($this.BrokenMode -eq 'freeze_shifted') {{ return [int][Math]::Round(1380.0 + $pt * 1.6666667) }}
    return [int][Math]::Round(1580.0 + $pt * (96.0 / 72.0) * 1.25)
  }}
  $w | Add-Member -MemberType ScriptMethod -Name PointsToScreenPixelsY -Value {{
    param([double]$pt)
    if ($this.BrokenMode -eq 'wps_raw_pt') {{ return [int][Math]::Round(504.0 + $pt) }}
    if ($this.BrokenMode -eq 'freeze_shifted') {{ return [int][Math]::Round(304.0 + $pt * 1.6666667) }}
    return [int][Math]::Round(504.0 + $pt * (96.0 / 72.0) * 1.25)
  }}
  return $w
}}
$rng = [pscustomobject]@{{
  Row = 7; Column = 3;
  Rows = [pscustomobject]@{{ Count = 8 }};
  Columns = [pscustomobject]@{{ Count = 3 }};
  Left = 96.0; Top = 86.4; Width = 144.0; Height = 115.2
}}
$r1 = Resolve-ExcelRangeRect (New-MockWin 'excel365') $rng 1775 663 1946 828 1.25 1536 244 2442 1306
$r2 = Resolve-ExcelRangeRect (New-MockWin 'wps_raw_pt') $rng 1775 663 1946 828 1.25 1536 244 2442 1306
$r3 = Resolve-ExcelRangeRect (New-MockWin 'freeze_shifted') $rng 1775 663 1946 828 1.25 1536 244 2442 1306
@{{ excel365 = $r1; wps = $r2; freeze = $r3 }} | ConvertTo-Json -Compress -Depth 5
"#,
            solver = EXCEL_RECT_SOLVER_PS
        );
        let out = crate::broker::execute_ps51_local(&test_script, 5000)
            .expect("powershell mock solver test should execute");
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
        for (label, expected_method) in [
            ("excel365", "p2s_verified"),
            ("wps", "p2s_verified"),
            ("freeze", "rfp_bsearch"),
        ] {
            let entry = &v[label];
            assert_eq!(entry["method"].as_str(), Some(expected_method), "case={label} raw={out}");
            let rx = entry["rect"]["x"].as_i64().unwrap();
            let ry = entry["rect"]["y"].as_i64().unwrap();
            let rw = entry["rect"]["w"].as_i64().unwrap();
            let rh = entry["rect"]["h"].as_i64().unwrap();
            assert!((rx - 1740).abs() <= 1, "{label} x={rx}");
            assert!((ry - 648).abs() <= 1, "{label} y={ry}");
            assert!((rw - 240).abs() <= 2, "{label} w={rw}");
            assert!((rh - 192).abs() <= 2, "{label} h={rh}");
        }
    }
}
