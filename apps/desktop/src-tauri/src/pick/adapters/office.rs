use crate::broker::BrokerRequest;
use crate::pick::adapters::{AdapterContext, AdapterOutput};
use std::path::Path;

const WORD_EXES: &[&str] = &["winword.exe", "wps.exe"];
const EXCEL_EXES: &[&str] = &["excel.exe", "et.exe"];
const PPT_EXES: &[&str] = &["powerpnt.exe", "wpp.exe"];

pub fn matches(ctx: &AdapterContext) -> bool {
    let exe = ctx.proc_meta.exe_name.as_str();
    WORD_EXES.contains(&exe) || EXCEL_EXES.contains(&exe) || PPT_EXES.contains(&exe)
}

pub fn run(ctx: &AdapterContext) -> Option<AdapterOutput> {
    let exe = ctx.proc_meta.exe_name.as_str();
    // WPS single-binary (`wps.exe`) can also host spreadsheets/PDF; inspect progIds in order
    let script = if EXCEL_EXES.contains(&exe) {
        excel_com_script()
    } else if PPT_EXES.contains(&exe) {
        ppt_com_script()
    } else {
        word_or_wps_com_script()
    };

    let resp = ctx.broker.call(BrokerRequest::RunPs51 {
        script: script.to_string(),
        timeout_ms: 1300,
    });

    let mut out = AdapterOutput::new("office");
    if resp.ok && !resp.text.trim().is_empty() {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&resp.text) {
            if let Some(full_name) = val.get("fullName").and_then(|v| v.as_str()) {
                if !full_name.is_empty() {
                    out.lines.push(format!("office_document_path: {full_name}"));
                    if Path::new(full_name).exists() {
                        out.paths.push(full_name.to_string());
                    }
                    out.badge = Path::new(full_name)
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string());
                }
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
      try { $selText = [string]$app.Selection.Text; if ($selText.Length -gt 1200) { $selText = $selText.Substring(0, 1200) } } catch {}
      @{ fullName = [string]$doc.FullName; location = "Document: $($doc.Name)"; selection = $selText } | ConvertTo-Json -Compress
      exit 0
    }
    if ($null -ne $app.ActiveWorkbook) {
      $wb = $app.ActiveWorkbook
      $sh = $app.ActiveSheet.Name
      $addr = $app.Selection.Address()
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
  try { $addr = [string]$app.Selection.Address() } catch {}
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
