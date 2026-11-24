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

/// COM-backed Excel / WPS Spreadsheets single-cell hit-test & selection at `(x, y)`.
/// Returns `None` if `(x, y)` did not hit a worksheet cell (e.g. Ribbon/titlebar), allowing UIA fallback.
pub fn pick_excel_cell_com(
    broker: &BrokerClient,
    x: i32,
    y: i32,
    dpi_scale: f64,
) -> Option<ExcelPickResult> {
    let scale = dpi_scale.max(0.5);
    let lx = ((x as f64) / scale).round() as i32;
    let ly = ((y as f64) / scale).round() as i32;

    let script = format!(
        r#"
$app = $null
$usedProgId = ''
foreach ($prog in @('Excel.Application', 'KET.Application', 'Ket.Application')) {{
  try {{ $app = [Runtime.InteropServices.Marshal]::GetActiveObject($prog); if ($null -ne $app) {{ $usedProgId = $prog; break }} }} catch {{}}
}}
if ($null -eq $app -or $null -eq $app.ActiveWorkbook) {{ exit 0 }}
$wb = $app.ActiveWorkbook
$win = $app.ActiveWindow
$sh = $app.ActiveSheet

$cell = $null
try {{ $cell = $win.RangeFromPoint([int]{x}, [int]{y}) }} catch {{}}
if ($null -eq $cell) {{
  try {{ $cell = $win.RangeFromPoint([int]{lx}, [int]{ly}) }} catch {{}}
}}
if ($null -eq $cell) {{ exit 0 }}

$addr = ''
try {{ $addr = [string]$cell.Address($false, $false) }} catch {{ exit 0 }}
if ([string]::IsNullOrWhiteSpace($addr)) {{ exit 0 }}

try {{ [void]$cell.Select() }} catch {{}}
$row = [int]$cell.Row
$col = [int]$cell.Column
$disp = ''; try {{ $disp = [string]$cell.Text }} catch {{}}
$val = ''; try {{ if ($null -ne $cell.Value2) {{ $val = [string]$cell.Value2 }} }} catch {{}}
$formula = ''; try {{ $f = [string]$cell.Formula; if ($f.StartsWith('=')) {{ $formula = $f }} }} catch {{}}
$colHdr = ''; if ($row -gt 1) {{ try {{ $colHdr = ([string]$sh.Cells.Item(1, $col).Text).Trim() }} catch {{}} }}
$rowHdr = ''; if ($col -gt 1) {{ try {{ $rowHdr = ([string]$sh.Cells.Item($row, 1).Text).Trim() }} catch {{}} }}

$rectObj = $null
try {{
  $sx1 = [int]$win.PointsToScreenPixelsX([double]$cell.Left)
  $sy1 = [int]$win.PointsToScreenPixelsY([double]$cell.Top)
  $sx2 = [int]$win.PointsToScreenPixelsX([double]($cell.Left + $cell.Width))
  $sy2 = [int]$win.PointsToScreenPixelsY([double]($cell.Top + $cell.Height))
  $sw = [Math]::Max(16, $sx2 - $sx1)
  $shPx = [Math]::Max(14, $sy2 - $sy1)
  if ($sw -lt 2000 -and $shPx -lt 1200) {{
    $rectObj = @{{ x = $sx1; y = $sy1; w = $sw; h = $shPx }}
  }}
}} catch {{}}

@{{
  progId = $usedProgId
  fullName = [string]$wb.FullName
  workbookName = [string]$wb.Name
  sheet = [string]$sh.Name
  address = $addr
  isRange = $false
  rows = 1
  cols = 1
  displayText = $disp
  value = $val
  formula = $formula
  colHeader = $colHdr
  rowHeader = $rowHdr
  screenRect = $rectObj
}} | ConvertTo-Json -Compress -Depth 4
"#
    );

    let resp = broker.call(BrokerRequest::RunPs51 {
        script,
        timeout_ms: 1600,
    });
    if !resp.ok || resp.text.trim().is_empty() {
        return None;
    }
    let parsed = serde_json::from_str::<ExcelPickResult>(resp.text.trim()).ok()?;
    if parsed.address.is_empty() {
        None
    } else {
        Some(parsed)
    }
}

/// COM-backed Excel / WPS Spreadsheets cell-range box selection from `(x1, y1)` to `(x2, y2)`.
pub fn pick_excel_range_com(
    broker: &BrokerClient,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    dpi_scale: f64,
) -> Option<ExcelPickResult> {
    let scale = dpi_scale.max(0.5);
    let lx1 = ((x1 as f64) / scale).round() as i32;
    let ly1 = ((y1 as f64) / scale).round() as i32;
    let lx2 = ((x2 as f64) / scale).round() as i32;
    let ly2 = ((y2 as f64) / scale).round() as i32;

    let script = format!(
        r#"
$app = $null
$usedProgId = ''
foreach ($prog in @('Excel.Application', 'KET.Application', 'Ket.Application')) {{
  try {{ $app = [Runtime.InteropServices.Marshal]::GetActiveObject($prog); if ($null -ne $app) {{ $usedProgId = $prog; break }} }} catch {{}}
}}
if ($null -eq $app -or $null -eq $app.ActiveWorkbook) {{ exit 0 }}
$wb = $app.ActiveWorkbook
$win = $app.ActiveWindow
$sh = $app.ActiveSheet

function Try-ExcelCell($w, $px, $py, $lx, $ly) {{
  $c = $null
  try {{ $c = $w.RangeFromPoint([int]$px, [int]$py) }} catch {{}}
  if ($null -eq $c) {{
    try {{ $c = $w.RangeFromPoint([int]$lx, [int]$ly) }} catch {{}}
  }}
  if ($null -ne $c) {{
    try {{ $a = [string]$c.Address($false, $false); if (-not [string]::IsNullOrWhiteSpace($a)) {{ return $c }} }} catch {{}}
  }}
  return $null
}}

$c1 = Try-ExcelCell $win {x1} {y1} {lx1} {ly1}
$c2 = Try-ExcelCell $win {x2} {y2} {lx2} {ly2}
if ($null -eq $c1 -and $null -eq $c2) {{ exit 0 }}
if ($null -eq $c1) {{ $c1 = $c2 }}
if ($null -eq $c2) {{ $c2 = $c1 }}

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

# Include row-1 column headers if selection starts below row 1
if ($firstRow -gt 1) {{
  $hdrCells = @()
  for ($ci = 0; $ci -lt $maxC; $ci++) {{
    $t = ''; try {{ $t = ([string]$sh.Cells.Item(1, $firstCol + $ci).Text).Trim() }} catch {{}}
    $hdrCells += $t
  }}
  if (($hdrCells -join '').Length -gt 0) {{
    $tableLines.Add("Headers(Row1): " + ($hdrCells -join " | "))
  }}
}}

for ($ri = 0; $ri -lt $maxR; $ri++) {{
  $rowCells = @()
  for ($ci = 0; $ci -lt $maxC; $ci++) {{
    $cell = $sh.Cells.Item($firstRow + $ri, $firstCol + $ci)
    $t = ''; try {{ $t = ([string]$cell.Text -replace "[\r\n]+", " ").Trim() }} catch {{}}
    $rowCells += $t
    if ($formulaLines.Count -lt 20) {{
      try {{
        $f = [string]$cell.Formula
        if ($f.StartsWith('=')) {{
          $ca = [string]$cell.Address($false, $false)
          $formulaLines.Add("$ca = $f")
        }}
      }} catch {{}}
    }}
  }}
  $tableLines.Add(($rowCells -join " | "))
}}
if ($rowCount -gt $maxR -or $colCount -gt $maxC) {{
  $tableLines.Add("... (truncated: showing ${{maxR}}x${{maxC}} of ${{rowCount}}x${{colCount}})")
}}

$rectObj = $null
try {{
  $sx1 = [int]$win.PointsToScreenPixelsX([double]$rng.Left)
  $sy1 = [int]$win.PointsToScreenPixelsY([double]$rng.Top)
  $sx2 = [int]$win.PointsToScreenPixelsX([double]($rng.Left + $rng.Width))
  $sy2 = [int]$win.PointsToScreenPixelsY([double]($rng.Top + $rng.Height))
  $sw = [Math]::Max(18, $sx2 - $sx1)
  $shPx = [Math]::Max(16, $sy2 - $sy1)
  $rectObj = @{{ x = $sx1; y = $sy1; w = $sw; h = $shPx }}
}} catch {{}}

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
    let parsed = serde_json::from_str::<ExcelPickResult>(resp.text.trim()).ok()?;
    if parsed.address.is_empty() {
        None
    } else {
        Some(parsed)
    }
}

/// COM-backed PowerPoint / WPS Presentation single design-element hit-test at `(x, y)`.
/// Uses `ActiveWindow.RangeFromPoint(x, y)` first, then falls back to reverse Z-order
/// `PointsToScreenPixelsX/Y` hit-testing (with `GroupItems` drill-down) for full WPS compatibility.
pub fn pick_ppt_element_com(
    broker: &BrokerClient,
    x: i32,
    y: i32,
    dpi_scale: f64,
) -> Option<PptElementPickResult> {
    let scale = dpi_scale.max(0.5);
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

function Get-ShapeRect($w, $sh) {{
  try {{
    $x1 = [int]$w.PointsToScreenPixelsX([double]$sh.Left)
    $y1 = [int]$w.PointsToScreenPixelsY([double]$sh.Top)
    $x2 = [int]$w.PointsToScreenPixelsX([double]($sh.Left + $sh.Width))
    $y2 = [int]$w.PointsToScreenPixelsY([double]($sh.Top + $sh.Height))
    return @{{ x = $x1; y = $y1; w = [Math]::Max(8, $x2 - $x1); h = [Math]::Max(8, $y2 - $y1); x2 = $x2; y2 = $y2 }}
  }} catch {{ return $null }}
}}

function Drill-Group($w, $sh, $px, $py) {{
  try {{
    if ([int]$sh.Type -eq 6 -and $sh.GroupItems.Count -gt 0) {{
      $bestChild = $null
      $bestArea = [double]::MaxValue
      for ($i = $sh.GroupItems.Count; $i -ge 1; $i--) {{
        $ch = $sh.GroupItems.Item($i)
        $rc = Get-ShapeRect $w $ch
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
# 1. Direct COM RangeFromPoint
foreach ($pt in @(@({x}, {y}), @({lx}, {ly}))) {{
  if ($null -ne $target) {{ break }}
  try {{
    $hit = $win.RangeFromPoint([int]$pt[0], [int]$pt[1])
    if ($null -ne $hit) {{
      $hasName = $false
      try {{ $n = [string]$hit.Name; if (-not [string]::IsNullOrWhiteSpace($n)) {{ $hasName = $true }} }} catch {{}}
      if ($hasName) {{
        $target = Drill-Group $win $hit $pt[0] $pt[1]
      }} else {{
        try {{
          $owner = $hit.Parent.Parent
          if ($null -ne $owner -and -not [string]::IsNullOrWhiteSpace([string]$owner.Name)) {{
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
  foreach ($pt in @(@({x}, {y}), @({lx}, {ly}))) {{
    if ($null -ne $best) {{ break }}
    for ($i = $slide.Shapes.Count; $i -ge 1; $i--) {{
      $sh = $slide.Shapes.Item($i)
      try {{ if (-not $sh.Visible) {{ continue }} }} catch {{}}
      $rc = Get-ShapeRect $win $sh
      if ($null -ne $rc -and $pt[0] -ge $rc.x -and $pt[0] -le $rc.x2 -and $pt[1] -ge $rc.y -and $pt[1] -le $rc.y2) {{
        $cand = Drill-Group $win $sh $pt[0] $pt[1]
        $crc = Get-ShapeRect $win $cand
        $z = 0; try {{ $z = [int]$sh.ZOrderPosition }} catch {{}}
        $area = if ($null -ne $crc) {{ [double]$crc.w * [double]$crc.h }} else {{ [double]$rc.w * [double]$rc.h }}
        if ($z -gt $bestZ -or ($z -eq $bestZ -and $area -lt $bestArea)) {{
          $best = $cand
          $bestZ = $z
          $bestArea = $area
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

$finalRc = Get-ShapeRect $win $target
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
    let parsed = serde_json::from_str::<PptElementPickResult>(resp.text.trim()).ok()?;
    if parsed.shape_name.is_empty() && parsed.text.is_empty() {
        None
    } else {
        Some(parsed)
    }
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
}
