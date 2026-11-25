use crate::broker::{BrokerClient, BrokerRequest};
use crate::captures::ScreenRect;
use crate::pick::adapters::office::fast_exe_basename_for_pid;
use crate::pick::adapters::{AdapterContext, AdapterOutput};
use crate::pick::resolve_root_window_at;
use crate::window::dpi_scale_for_hwnd;
use serde::Deserialize;
use std::path::Path;
use windows::Win32::Foundation::HWND;

#[derive(Debug, Clone)]
pub struct SolidWorksWindowHit {
    pub root_hwnd: HWND,
    pub pid: u32,
    pub win_title: String,
    pub win_class: String,
    pub dpi_scale: f64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwDimensionItem {
    #[serde(default)]
    pub full_name: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub value: f64,
    #[serde(default)]
    pub unit: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwPickedEntity {
    #[serde(default)]
    pub select_type_id: i32,
    #[serde(default)]
    pub select_type: String, // "FEATURE" | "FACE" | "EDGE" | "VERTEX" | "SKETCH" | "COMPONENT" | "DIMENSION" | "DRAWING_VIEW" | "MATE" | "BODY"
    #[serde(default)]
    pub feature_name: String,
    #[serde(default)]
    pub feature_type: String,
    #[serde(default)]
    pub suppressed: bool,
    #[serde(default)]
    pub error_code: i32,
    #[serde(default)]
    pub face_count: i32,
    #[serde(default)]
    pub parents: Vec<String>,
    #[serde(default)]
    pub children: Vec<String>,
    #[serde(default)]
    pub dimensions: Vec<SwDimensionItem>,
    #[serde(default)]
    pub entity_geometry: String,
    #[serde(default)]
    pub normal: Option<[f64; 3]>,
    #[serde(default)]
    pub point_mm: Option<[f64; 3]>,
    #[serde(default)]
    pub body_index: Option<i32>,
    #[serde(default)]
    pub entity_index: Option<i32>,
    #[serde(default)]
    pub component_name: String,
    #[serde(default)]
    pub component_path: String,
    #[serde(default)]
    pub component_config: String,
    #[serde(default)]
    pub dimension_display: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolidWorksPickResult {
    #[serde(default)]
    pub doc_title: String,
    #[serde(default)]
    pub doc_path: String,
    #[serde(default)]
    pub doc_type: String, // "Part" | "Assembly" | "Drawing"
    #[serde(default)]
    pub active_config: String,
    #[serde(default)]
    pub pick_tier: String, // "tree_name" | "viewport_ray" | "current_selection" | "document_only"
    #[serde(default)]
    pub selection_count: i32,
    #[serde(default)]
    pub items: Vec<SwPickedEntity>,
    #[serde(default)]
    pub recent_features: Vec<String>,
    #[serde(default)]
    pub viewport_rect: Option<ScreenRect>,
}

pub fn is_solidworks_exe(exe_name: &str) -> bool {
    exe_name.eq_ignore_ascii_case("sldworks.exe")
}

pub fn matches(ctx: &AdapterContext) -> bool {
    is_solidworks_exe(&ctx.proc_meta.exe_name)
}

pub fn detect_solidworks_at(x: i32, y: i32) -> Option<SolidWorksWindowHit> {
    let (root_hwnd, pid, win_title, win_class) = resolve_root_window_at(x, y);
    if pid == 0 {
        return None;
    }
    let exe = fast_exe_basename_for_pid(pid);
    if !is_solidworks_exe(&exe) {
        return None;
    }
    Some(SolidWorksWindowHit {
        root_hwnd,
        pid,
        win_title,
        win_class,
        dpi_scale: dpi_scale_for_hwnd(root_hwnd),
    })
}

/// Cleans SolidWorks FeatureManager tree item UIA text into a candidate COM feature name.
/// Examples:
/// - `(-) 草图1` -> `草图1`
/// - `凸台-拉伸1 [默认]` -> `凸台-拉伸1`
/// - `(f) 底座-1 (默认<<默认>_显示状态 1>)` -> `底座-1`
pub fn clean_feature_tree_label(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    // Strip leading SolidWorks sketch/component state markers like `(-) `, `(+) `, `(?) `, `(f) `, `(->) `
    for prefix in ["(-) ", "(+) ", "(?) ", "(f) ", "(-)", "(+)", "(?)", "(f)"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.trim().to_string();
            break;
        }
    }
    // Strip trailing configuration/display-state annotations
    if let Some(idx) = s.find(" [") {
        s = s[..idx].trim().to_string();
    }
    if let Some(idx) = s.find(" (") {
        s = s[..idx].trim().to_string();
    }
    if let Some(idx) = s.find('<') {
        if idx > 0 {
            s = s[..idx].trim().to_string();
        }
    }
    s
}

/// Maps a unit normal vector `(nx, ny, nz)` to a Zegel-SWE cardinal direction selector (`+z`, `-z`, etc.)
/// when aligned within ~5 degrees (`|dot| >= 0.996`).
pub fn classify_semantic_direction(normal: [f64; 3]) -> Option<&'static str> {
    let len = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
    if len < 1e-6 {
        return None;
    }
    let nx = normal[0] / len;
    let ny = normal[1] / len;
    let nz = normal[2] / len;
    const THRESH: f64 = 0.996;
    if nz >= THRESH {
        Some("+z")
    } else if nz <= -THRESH {
        Some("-z")
    } else if nx >= THRESH {
        Some("+x")
    } else if nx <= -THRESH {
        Some("-x")
    } else if ny >= THRESH {
        Some("+y")
    } else if ny <= -THRESH {
        Some("-y")
    } else {
        None
    }
}

/// Pure mathematical inversion of SolidWorks `IModelView.Transform.ArrayData` (16 doubles)
/// from viewport client pixel `(cx, cy)` to a 3D model-space ray `(origin_m, dir_unit)`.
///
/// SolidWorks `IMathTransform.ArrayData` layout:
/// - `[0..9]`: 3x3 orthogonal rotation matrix R (row-major: `v_view = s * (v_model * R) + t`)
/// - `[9..12]`: translation `(tx, ty, tz)` in screen pixels
/// - `[12]`: uniform scale `s` (pixels per meter)
pub fn invert_model_view_transform_to_ray(
    arr: &[f64],
    cx: f64,
    cy: f64,
) -> Option<([f64; 3], [f64; 3])> {
    if arr.len() < 13 {
        return None;
    }
    let s = arr[12];
    if !s.is_finite() || s.abs() < 1e-9 {
        return None;
    }
    let tx = arr[9];
    let ty = arr[10];
    let tz = arr[11];

    // Row-major R; since R is orthogonal, inverse is R^T.
    // For row-vector convention `v_screen = s * (v_model * R) + t`:
    // `v_model = ((v_screen - t) / s) * R^T`, i.e. dot with rows of R.
    let ux = (cx - tx) / s;
    let uy = (cy - ty) / s;
    // Start ray well in front of the model (+Z in SolidWorks viewport points toward viewer)
    let uz_front = (10_000.0 - tz) / s;

    let ox = ux * arr[0] + uy * arr[1] + uz_front * arr[2];
    let oy = ux * arr[3] + uy * arr[4] + uz_front * arr[5];
    let oz = ux * arr[6] + uy * arr[7] + uz_front * arr[8];

    // Ray points into the screen: screen direction (0, 0, -1) -> model direction = (0,0,-1) * R^T
    let dx = -arr[2];
    let dy = -arr[5];
    let dz = -arr[8];
    let dlen = (dx * dx + dy * dy + dz * dz).sqrt();
    if dlen < 1e-9 {
        return None;
    }

    Some(([ox, oy, oz], [dx / dlen, dy / dlen, dz / dlen]))
}

fn escape_ps_single_quoted(raw: &str) -> String {
    raw.replace('\'', "''")
}

/// Runs 3-tier SolidWorks COM feature & entity extraction strictly at medium integrity via Broker:
/// 1. FeatureManager Tree label match (`FeatureByName` + `FirstFeature/GetNextFeature`)
/// 2. 3D Graphics Viewport ray-pick (`IModelView.GetViewHWnd` + `Transform.ArrayData` + `Extension.SelectByRay`)
/// 3. Existing `ISelectionMgr` selection readback (multi-select, faces, edges, sketches, mates, dims, components)
pub fn pick_solidworks_feature_com(
    broker: &BrokerClient,
    screen_x: i32,
    screen_y: i32,
    uia_label: Option<&str>,
) -> Option<SolidWorksPickResult> {
    let cleaned_label = uia_label
        .map(clean_feature_tree_label)
        .unwrap_or_default();
    let label_ps = escape_ps_single_quoted(&cleaned_label);

    let script = format!(
        r#"
$ErrorActionPreference = 'SilentlyContinue'
$sw = $null
try {{
  $sw = [Runtime.InteropServices.Marshal]::GetActiveObject('SldWorks.Application')
}} catch {{}}
if ($null -eq $sw -or $null -eq $sw.ActiveDoc) {{ exit 0 }}

$doc = $sw.ActiveDoc
$docTitle = [string]$doc.GetTitle()
$docPath = [string]$doc.GetPathName()
$docTypeNum = 0
try {{ $docTypeNum = [int]$doc.GetType() }} catch {{}}
$docType = switch ($docTypeNum) {{ 1 {{ 'Part' }} 2 {{ 'Assembly' }} 3 {{ 'Drawing' }} default {{ 'Unknown' }} }}
$activeConfig = ''
try {{ $activeConfig = [string]$doc.ConfigurationManager.ActiveConfiguration.Name }} catch {{}}

$containerNodes = @(
  '收藏','历史记录','选择集','传感器','设计活页夹','注解','材质','标注','光源与相机','实体','曲面实体','备注','方程式','材质 <未指定>',
  'Favorites','History','Selection Sets','Sensors','Design Binder','Annotations','Material','Callouts','Lights and Cameras','Solid Bodies','Surface Bodies','Notes','Equations'
)

function Get-FeatureDimensions($feat) {{
  $dims = New-Object System.Collections.Generic.List[object]
  try {{
    $dd = $feat.GetFirstDisplayDimension()
    $guard = 0
    while ($null -ne $dd -and $guard -lt 24) {{
      $guard++
      try {{
        $d = $dd.GetDimension2(0)
        if ($null -ne $d) {{
          $fn = [string]$d.FullName
          $dn = [string]$d.Name
          $sv = [double]$d.SystemValue
          $dtype = 0
          try {{ $dtype = [int]$d.GetType() }} catch {{}}
          if ($dtype -eq 1) {{
            $val = [Math]::Round($sv * 180.0 / [Math]::PI, 3)
            $unit = 'deg'
          }} else {{
            $val = [Math]::Round($sv * 1000.0, 4)
            $unit = 'mm'
          }}
          $dims.Add(@{{ fullName = $fn; name = $dn; value = $val; unit = $unit }})
        }}
      }} catch {{}}
      $dd = $feat.GetNextDisplayDimension($dd)
    }}
  }} catch {{}}
  return $dims
}}

function Inspect-FeatureObj($feat) {{
  $fname = ''; $ftype = ''; $supp = $false; $errCode = 0; $fcnt = 0
  $parents = @(); $children = @()
  try {{ $fname = [string]$feat.Name }} catch {{}}
  try {{ $ftype = [string]$feat.GetTypeName2() }} catch {{}}
  try {{
    $supArr = $feat.IsSuppressed2(1, $null)
    if ($supArr -is [System.Array] -and $supArr.Length -gt 0) {{ $supp = [bool]$supArr[0] }}
    elseif ($null -ne $supArr) {{ $supp = [bool]$supArr }}
  }} catch {{}}
  try {{
    $isWarn = $false
    $errCode = [int]$feat.GetErrorCode2([ref]$isWarn)
  }} catch {{}}
  try {{ $fcnt = [int]$feat.GetFaceCount() }} catch {{}}
  try {{
    $ps = $feat.GetParents()
    if ($null -ne $ps) {{
      $parents = @($ps | Select-Object -First 8 | ForEach-Object {{ try {{ [string]$_.Name }} catch {{ '' }} }} | Where-Object {{ $_ }})
    }}
  }} catch {{}}
  try {{
    $cs = $feat.GetChildren()
    if ($null -ne $cs) {{
      $children = @($cs | Select-Object -First 8 | ForEach-Object {{ try {{ [string]$_.Name }} catch {{ '' }} }} | Where-Object {{ $_ }})
    }}
  }} catch {{}}
  $dims = Get-FeatureDimensions $feat
  return @{{
    featureName = $fname
    featureType = $ftype
    suppressed = $supp
    errorCode = $errCode
    faceCount = $fcnt
    parents = $parents
    children = $children
    dimensions = $dims
  }}
}}

function Find-BodyEntityIndex($docObj, $kind, $targetObj) {{
  try {{
    $bodies = $docObj.GetBodies2(0, $true)
    if ($null -eq $bodies) {{ return $null }}
    for ($bi = 0; $bi -lt $bodies.Length; $bi++) {{
      $arr = if ($kind -eq 'FACE') {{ $bodies[$bi].GetFaces() }} elseif ($kind -eq 'EDGE') {{ $bodies[$bi].GetEdges() }} else {{ $bodies[$bi].GetVertices() }}
      if ($null -eq $arr) {{ continue }}
      for ($ei = 0; $ei -lt $arr.Length; $ei++) {{
        if ($arr[$ei] -eq $targetObj) {{
          return @{{ bodyIndex = $bi; entityIndex = $ei }}
        }}
      }}
    }}
  }} catch {{}}
  return $null
}}

$sm = $doc.SelectionManager
$pickTier = ''
$viewportRectObj = $null
$treeFeatObj = $null

# Tier 1: FeatureManager tree item name match
$uiaLabel = '{label_ps}'
if (-not [string]::IsNullOrWhiteSpace($uiaLabel) -and ($containerNodes -notcontains $uiaLabel)) {{
  try {{ $treeFeatObj = $doc.FeatureByName($uiaLabel) }} catch {{}}
  if ($null -eq $treeFeatObj) {{
    try {{
      $f = $doc.FirstFeature()
      $guard = 0
      while ($null -ne $f -and $guard -lt 400) {{
        $guard++
        $fn = [string]$f.Name
        if ($fn -eq $uiaLabel) {{ $treeFeatObj = $f; break }}
        $f = $f.GetNextFeature()
      }}
    }} catch {{}}
  }}
  if ($null -ne $treeFeatObj) {{
    $pickTier = 'tree_name'
    try {{ $null = $treeFeatObj.Select2($false, 0) }} catch {{}}
  }}
}}

# Tier 2: 3D Graphics Viewport ray-pick using IModelView.GetViewHWnd() + Transform.ArrayData + SelectByRay
if ($pickTier -eq '' -and $docTypeNum -ne 3) {{
  try {{
    $view = $doc.ActiveView
    if ($null -ne $view) {{
      $vhwnd = [IntPtr]::Zero
      try {{ $vhwnd = [IntPtr][int64]$view.GetViewHWnd() }} catch {{}}
      if ($vhwnd -ne [IntPtr]::Zero) {{
        Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class SwWinRect {{
  [StructLayout(LayoutKind.Sequential)]
  public struct RECT {{ public int Left; public int Top; public int Right; public int Bottom; }}
  [DllImport("user32.dll")]
  public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);
}}
"@ -ErrorAction SilentlyContinue
        $rc = New-Object SwWinRect+RECT
        if ([SwWinRect]::GetWindowRect($vhwnd, [ref]$rc)) {{
          $vw = $rc.Right - $rc.Left
          $vh = $rc.Bottom - $rc.Top
          if ($vw -gt 20 -and $vh -gt 20) {{
            $viewportRectObj = @{{ x = $rc.Left; y = $rc.Top; w = $vw; h = $vh }}
            $px = {screen_x}
            $py = {screen_y}
            if ($px -ge $rc.Left -and $px -lt $rc.Right -and $py -ge $rc.Top -and $py -lt $rc.Bottom) {{
              $cx = [double]($px - $rc.Left)
              $cy = [double]($py - $rc.Top)
              $xf = $view.Transform
              if ($null -ne $xf) {{
                $arr = $xf.ArrayData
                if ($null -ne $arr -and $arr.Length -ge 13) {{
                  $s = [double]$arr[12]
                  if ([Math]::Abs($s) -gt 1e-9) {{
                    $tx = [double]$arr[9]; $ty = [double]$arr[10]; $tz = [double]$arr[11]
                    $ux = ($cx - $tx) / $s
                    $uy = ($cy - $ty) / $s
                    $uz = (10000.0 - $tz) / $s
                    $ox = $ux * [double]$arr[0] + $uy * [double]$arr[1] + $uz * [double]$arr[2]
                    $oy = $ux * [double]$arr[3] + $uy * [double]$arr[4] + $uz * [double]$arr[5]
                    $oz = $ux * [double]$arr[6] + $uy * [double]$arr[7] + $uz * [double]$arr[8]
                    $dx = -[double]$arr[2]; $dy = -[double]$arr[5]; $dz = -[double]$arr[8]
                    $dlen = [Math]::Sqrt($dx*$dx + $dy*$dy + $dz*$dz)
                    if ($dlen -gt 1e-9) {{
                      $dx /= $dlen; $dy /= $dlen; $dz /= $dlen
                      $rad = [Math]::Max(0.0005, 4.0 / [Math]::Abs($s))
                      $ext = $doc.Extension
                      foreach ($tw in @(2, 1, 3)) {{
                        $null = $ext.SelectByRay($ox, $oy, $oz, $dx, $dy, $dz, $rad, $tw, $false, 0, 0)
                        $c = [int]$sm.GetSelectedObjectCount2(-1)
                        if ($c -gt 0) {{
                          $pickTier = 'viewport_ray'
                          break
                        }}
                      }}
                    }}
                  }}
                }}
              }}
            }}
          }}
        }}
      }}
    }}
  }} catch {{}}
}}

# Tier 3: Inspect SelectionManager objects (covers Tier 1, Tier 2, and user's pre-selection!)
$selCount = 0
try {{ $selCount = [int]$sm.GetSelectedObjectCount2(-1) }} catch {{}}
$items = New-Object System.Collections.Generic.List[object]

if ($selCount -gt 0) {{
  if ($pickTier -eq '') {{ $pickTier = 'current_selection' }}
  $limit = [Math]::Min($selCount, 10)
  for ($i = 1; $i -le $limit; $i++) {{
    $tid = 0
    try {{ $tid = [int]$sm.GetSelectedObjectType3($i, -1) }} catch {{}}
    $obj = $null
    try {{ $obj = $sm.GetSelectedObject6($i, -1) }} catch {{}}
    if ($null -eq $obj) {{ continue }}

    $ptMm = $null
    try {{
      $pt = $sm.GetSelectionPoint2($i, -1)
      if ($null -ne $pt -and $pt.Length -ge 3) {{
        $ptMm = @([Math]::Round([double]$pt[0]*1000, 3), [Math]::Round([double]$pt[1]*1000, 3), [Math]::Round([double]$pt[2]*1000, 3))
      }}
    }} catch {{}}

    $rec = @{{
      selectTypeId = $tid
      selectType = 'OBJECT'
      featureName = ''
      featureType = ''
      suppressed = $false
      errorCode = 0
      faceCount = 0
      parents = @()
      children = @()
      dimensions = @()
      entityGeometry = ''
      normal = $null
      pointMm = $ptMm
      bodyIndex = $null
      entityIndex = $null
      componentName = ''
      componentPath = ''
      componentConfig = ''
      dimensionDisplay = ''
    }}

    if ($tid -eq 2) {{
      # swSelFACES
      $rec.selectType = 'FACE'
      try {{
        $ownerFeat = $obj.GetFeature()
        if ($null -ne $ownerFeat) {{
          $fi = Inspect-FeatureObj $ownerFeat
          foreach ($k in $fi.Keys) {{ $rec[$k] = $fi[$k] }}
        }}
      }} catch {{}}
      try {{
        $nrm = $obj.Normal
        if ($null -ne $nrm -and $nrm.Length -ge 3) {{
          $rec.normal = @([Math]::Round([double]$nrm[0], 4), [Math]::Round([double]$nrm[1], 4), [Math]::Round([double]$nrm[2], 4))
        }}
      }} catch {{}}
      $geomParts = @()
      try {{
        $surf = $obj.GetSurface()
        if ($null -ne $surf) {{
          if ($surf.IsPlane()) {{ $geomParts += 'Plane' }}
          elseif ($surf.IsCylinder()) {{
            $cp = $surf.CylinderParams
            if ($null -ne $cp -and $cp.Length -ge 7) {{
              $geomParts += ('Cylinder R=' + [Math]::Round([double]$cp[6]*1000, 3) + 'mm')
            }} else {{ $geomParts += 'Cylinder' }}
          }}
          elseif ($surf.IsCone()) {{ $geomParts += 'Cone' }}
          elseif ($surf.IsSphere()) {{ $geomParts += 'Sphere' }}
          elseif ($surf.IsTorus()) {{ $geomParts += 'Torus' }}
          else {{ $geomParts += 'Surface' }}
        }}
      }} catch {{}}
      try {{
        $areaMm2 = [Math]::Round([double]$obj.GetArea() * 1e6, 2)
        if ($areaMm2 -gt 0) {{ $geomParts += ('area=' + $areaMm2 + 'mm2') }}
      }} catch {{}}
      $rec.entityGeometry = ($geomParts -join ', ')
      $idxInfo = Find-BodyEntityIndex $doc 'FACE' $obj
      if ($null -ne $idxInfo) {{ $rec.bodyIndex = $idxInfo.bodyIndex; $rec.entityIndex = $idxInfo.entityIndex }}
    }}
    elseif ($tid -eq 1) {{
      # swSelEDGES
      $rec.selectType = 'EDGE'
      try {{
        $adj = $obj.GetTwoAdjacentFaces2()
        if ($null -ne $adj) {{
          foreach ($af in $adj) {{
            if ($null -ne $af) {{
              $of = $af.GetFeature()
              if ($null -ne $of) {{
                $fi = Inspect-FeatureObj $of
                foreach ($k in $fi.Keys) {{ $rec[$k] = $fi[$k] }}
                break
              }}
            }}
          }}
        }}
      }} catch {{}}
      $geomParts = @()
      try {{
        $crv = $obj.GetCurve()
        if ($null -ne $crv) {{
          if ($crv.IsLine()) {{ $geomParts += 'Line' }}
          elseif ($crv.IsCircle()) {{
            $cp = $crv.CircleParams
            if ($null -ne $cp -and $cp.Length -ge 7) {{
              $geomParts += ('Circle R=' + [Math]::Round([double]$cp[6]*1000, 3) + 'mm')
            }} else {{ $geomParts += 'Circle' }}
          }}
          elseif ($crv.IsEllipse()) {{ $geomParts += 'Ellipse' }}
          else {{ $geomParts += 'Curve' }}
        }}
      }} catch {{}}
      $rec.entityGeometry = ($geomParts -join ', ')
      $idxInfo = Find-BodyEntityIndex $doc 'EDGE' $obj
      if ($null -ne $idxInfo) {{ $rec.bodyIndex = $idxInfo.bodyIndex; $rec.entityIndex = $idxInfo.entityIndex }}
    }}
    elseif ($tid -eq 3) {{
      # swSelVERTICES
      $rec.selectType = 'VERTEX'
      try {{
        $vp = $obj.GetPoint()
        if ($null -ne $vp -and $vp.Length -ge 3) {{
          $rec.pointMm = @([Math]::Round([double]$vp[0]*1000, 3), [Math]::Round([double]$vp[1]*1000, 3), [Math]::Round([double]$vp[2]*1000, 3))
        }}
      }} catch {{}}
      $idxInfo = Find-BodyEntityIndex $doc 'VERTEX' $obj
      if ($null -ne $idxInfo) {{ $rec.bodyIndex = $idxInfo.bodyIndex; $rec.entityIndex = $idxInfo.entityIndex }}
    }}
    elseif ($tid -eq 20) {{
      # swSelCOMPONENTS
      $rec.selectType = 'COMPONENT'
      try {{ $rec.componentName = [string]$obj.Name2 }} catch {{}}
      try {{ $rec.componentPath = [string]$obj.GetPathName() }} catch {{}}
      try {{ $rec.componentConfig = [string]$obj.ReferencedConfiguration }} catch {{}}
      try {{ $rec.suppressed = [bool]$obj.IsSuppressed() }} catch {{}}
      $rec.featureName = $rec.componentName
      $rec.featureType = 'Component2'
    }}
    elseif ($tid -eq 14) {{
      # swSelDIMENSIONS
      $rec.selectType = 'DIMENSION'
      try {{
        $d = $obj.GetDimension2(0)
        if ($null -ne $d) {{
          $fn = [string]$d.FullName
          $sv = [double]$d.SystemValue
          $dtype = 0; try {{ $dtype = [int]$d.GetType() }} catch {{}}
          if ($dtype -eq 1) {{
            $rec.dimensionDisplay = "$fn = $([Math]::Round($sv * 180.0 / [Math]::PI, 3)) deg"
          }} else {{
            $rec.dimensionDisplay = "$fn = $([Math]::Round($sv * 1000.0, 4)) mm"
          }}
          $rec.featureName = $fn
          $rec.featureType = 'DisplayDimension'
        }}
      }} catch {{}}
    }}
    elseif ($tid -eq 12) {{
      # swSelDRAWINGVIEWS
      $rec.selectType = 'DRAWING_VIEW'
      try {{ $rec.featureName = [string]$obj.Name }} catch {{}}
      $rec.featureType = 'DrawingView'
      try {{
        $refDoc = $obj.ReferencedDocument
        if ($null -ne $refDoc) {{ $rec.componentPath = [string]$refDoc.GetPathName() }}
      }} catch {{}}
    }}
    else {{
      # Feature / Sketch / RefPlane / Mate / CoordSys / Body
      $rec.selectType = switch ($tid) {{
        9 {{ 'SKETCH' }}
        4 {{ 'DATUM_PLANE' }}
        5 {{ 'DATUM_AXIS' }}
        6 {{ 'DATUM_POINT' }}
        34 {{ 'MATE' }}
        49 {{ 'COORD_SYS' }}
        76 {{ 'SOLID_BODY' }}
        default {{ 'FEATURE' }}
      }}
      $fi = Inspect-FeatureObj $obj
      foreach ($k in $fi.Keys) {{ $rec[$k] = $fi[$k] }}
    }}

    $items.Add($rec)
  }}
}} elseif ($null -ne $treeFeatObj) {{
  $fi = Inspect-FeatureObj $treeFeatObj
  $rec = @{{
    selectTypeId = 22
    selectType = 'FEATURE'
    featureName = $fi.featureName
    featureType = $fi.featureType
    suppressed = $fi.suppressed
    errorCode = $fi.errorCode
    faceCount = $fi.faceCount
    parents = $fi.parents
    children = $fi.children
    dimensions = $fi.dimensions
    entityGeometry = ''
    normal = $null
    pointMm = $null
    bodyIndex = $null
    entityIndex = $null
    componentName = ''
    componentPath = ''
    componentConfig = ''
    dimensionDisplay = ''
  }}
  $items.Add($rec)
}}

# Collect last 10 real feature names (Zegel-SWE two-layer filter: GetTypeName2 non-empty + exclude containers)
$recent = New-Object System.Collections.Generic.List[string]
try {{
  $f = $doc.FirstFeature()
  $guard = 0
  while ($null -ne $f -and $guard -lt 400) {{
    $guard++
    $fn = ''; $ft = ''
    try {{ $fn = [string]$f.Name }} catch {{}}
    try {{ $ft = [string]$f.GetTypeName2() }} catch {{}}
    if (-not [string]::IsNullOrWhiteSpace($fn) -and -not [string]::IsNullOrWhiteSpace($ft) -and ($containerNodes -notcontains $fn)) {{
      $recent.Add("$fn($ft)")
    }}
    $f = $f.GetNextFeature()
  }}
}} catch {{}}
$recentTail = if ($recent.Count -gt 10) {{ $recent.GetRange($recent.Count - 10, 10) }} else {{ $recent }}

if ($pickTier -eq '') {{ $pickTier = 'document_only' }}

@{{
  docTitle = $docTitle
  docPath = $docPath
  docType = $docType
  activeConfig = $activeConfig
  pickTier = $pickTier
  selectionCount = $selCount
  items = $items
  recentFeatures = $recentTail
  viewportRect = $viewportRectObj
}} | ConvertTo-Json -Compress -Depth 6
"#
    );

    let resp = broker.call(BrokerRequest::RunPs51 {
        script,
        timeout_ms: 2200,
    });
    if !resp.ok || resp.text.trim().is_empty() {
        return None;
    }
    serde_json::from_str::<SolidWorksPickResult>(resp.text.trim()).ok()
}

/// Formats structured, Zegel-SWE & DoneGate ecosystem-aligned lines for a SolidWorks blue capsule.
pub fn format_solidworks_capsule_lines(sw: &SolidWorksPickResult) -> Vec<String> {
    let mut lines = Vec::new();
    let doc_label = if !sw.doc_path.is_empty() {
        sw.doc_path.as_str()
    } else {
        sw.doc_title.as_str()
    };
    lines.push(format!(
        "sw_document: title=\"{}\" type={} config=\"{}\" path=\"{}\"",
        sw.doc_title, sw.doc_type, sw.active_config, sw.doc_path
    ));
    lines.push(format!(
        "sw_pick_source: tier={} selection_count={}",
        sw.pick_tier, sw.selection_count
    ));

    if let Some(first) = sw.items.first() {
        // Unified [SW-TARGET] structured header for Zegel-SWE + DoneGate ecosystem interop
        let dir_sel = first.normal.and_then(classify_semantic_direction);
        let semantic_selector = if let Some(d) = dir_sel {
            format!("__current_selection__ | {d}")
        } else if let Some(idx) = first.entity_index {
            format!("__current_selection__ | index:{idx}")
        } else {
            "__current_selection__".to_string()
        };
        lines.push(format!(
            "[SW-TARGET] doc=\"{}\" kind={} feature=\"{}\" featureType=\"{}\" selector=\"{}\"",
            doc_label,
            first.select_type,
            first.feature_name,
            first.feature_type,
            semantic_selector
        ));
    }

    for (idx, item) in sw.items.iter().enumerate() {
        let prefix = if sw.items.len() > 1 {
            format!("sw_selected[{}]", idx + 1)
        } else {
            "sw_target_feature".to_string()
        };

        let mut parts = vec![format!("kind={}", item.select_type)];
        if !item.feature_name.is_empty() {
            parts.push(format!("name=\"{}\"", item.feature_name));
        }
        if !item.feature_type.is_empty() {
            parts.push(format!("type={}", item.feature_type));
        }
        if item.suppressed {
            parts.push("suppressed=true".to_string());
        }
        if item.error_code != 0 {
            parts.push(format!("error_code={}", item.error_code));
        }
        if item.face_count > 0 {
            parts.push(format!("faces={}", item.face_count));
        }
        if !item.entity_geometry.is_empty() {
            parts.push(format!("geometry=\"{}\"", item.entity_geometry));
        }
        if let Some(n) = item.normal {
            let dir_hint = classify_semantic_direction(n)
                .map(|d| format!(" dir={d}"))
                .unwrap_or_default();
            parts.push(format!(
                "normal=({:.3},{:.3},{:.3}){}",
                n[0], n[1], n[2], dir_hint
            ));
        }
        if let Some(pt) = item.point_mm {
            parts.push(format!(
                "point_mm=({:.2},{:.2},{:.2})",
                pt[0], pt[1], pt[2]
            ));
        }
        if let (Some(bi), Some(ei)) = (item.body_index, item.entity_index) {
            parts.push(format!("body_index={bi} index:{ei}"));
        }
        if !item.component_path.is_empty() {
            parts.push(format!(
                "component_path=\"{}\" config=\"{}\"",
                item.component_path, item.component_config
            ));
        }
        if !item.dimension_display.is_empty() {
            parts.push(format!("dimension=\"{}\"", item.dimension_display));
        }
        lines.push(format!("{prefix}: {}", parts.join(" | ")));

        if !item.dimensions.is_empty() {
            let dim_str: Vec<String> = item
                .dimensions
                .iter()
                .take(10)
                .map(|d| format!("{}={} {}", d.full_name, d.value, d.unit))
                .collect();
            lines.push(format!("sw_dimensions: {}", dim_str.join("; ")));
        }
        if !item.parents.is_empty() || !item.children.is_empty() {
            lines.push(format!(
                "sw_relations: parents=[{}] children=[{}]",
                item.parents.join(", "),
                item.children.join(", ")
            ));
        }
    }

    if sw.items.is_empty() && !sw.recent_features.is_empty() {
        lines.push(format!(
            "sw_feature_tree_tail: {}",
            sw.recent_features.join(" -> ")
        ));
    }

    // Zegel-SWE MCP ecosystem routing hint
    if let Some(first) = sw.items.first() {
        let mut hints = vec![
            "solidworks_selection_count".to_string(),
            "solidworks_feature_whats_wrong".to_string(),
        ];
        if !first.feature_name.is_empty() {
            hints.insert(
                0,
                format!("solidworks_feature_info(feature_name=\"{}\")", first.feature_name),
            );
            hints.insert(
                1,
                format!(
                    "solidworks_feature_readback(feature_name=\"{}\")",
                    first.feature_name
                ),
            );
        }
        if matches!(first.select_type.as_str(), "FACE" | "EDGE" | "VERTEX") {
            hints.push("solidworks_resolve_selection(selector=\"__current_selection__\")".to_string());
            hints.push("solidworks_selection_point()".to_string());
        }
        lines.push(format!(
            "sw_mcp_ecosystem (Zegel-SWE): 实体/特征已在 SolidWorks 活动文档中定位；若需查询或修改参数，优先调用 Zegel-SWE MCP [solidworks]：{}",
            hints.join(" · ")
        ));
    } else {
        lines.push(
            "sw_mcp_ecosystem (Zegel-SWE): 可调用 Zegel-SWE MCP [solidworks]：solidworks_feature_list_names · solidworks_feature_info · solidworks_feature_readback".to_string(),
        );
    }

    lines
}

/// Builds a concise, human-friendly blue capsule title for the picked SolidWorks feature/entity.
pub fn build_solidworks_chip_title(badge: &str, sw: &SolidWorksPickResult) -> (String, String) {
    let doc_badge = if !sw.doc_path.is_empty() {
        Path::new(&sw.doc_path)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| sw.doc_title.clone())
    } else {
        sw.doc_title.clone()
    };

    if let Some(first) = sw.items.first() {
        let core = if !first.dimension_display.is_empty() {
            first.dimension_display.clone()
        } else if !first.feature_name.is_empty() {
            match first.select_type.as_str() {
                "FACE" => {
                    let dir = first
                        .normal
                        .and_then(classify_semantic_direction)
                        .map(|d| format!(" [{d}]"))
                        .unwrap_or_default();
                    format!("{} · 面{}", first.feature_name, dir)
                }
                "EDGE" => format!("{} · 边", first.feature_name),
                "VERTEX" => format!("{} · 顶点", first.feature_name),
                "COMPONENT" => format!("{} (零件)", first.feature_name),
                _ if !first.feature_type.is_empty() => {
                    format!("{} ({})", first.feature_name, first.feature_type)
                }
                _ => first.feature_name.clone(),
            }
        } else if !first.entity_geometry.is_empty() {
            format!("{} {}", first.select_type, first.entity_geometry)
        } else {
            first.select_type.clone()
        };

        let multi_suffix = if sw.items.len() > 1 {
            format!(" 等{}项", sw.items.len())
        } else {
            String::new()
        };

        let title = format!("{badge} {core}{multi_suffix}");
        let detail = if doc_badge.is_empty() {
            "SolidWorks".to_string()
        } else {
            format!("SolidWorks · {doc_badge}")
        };
        (title, detail)
    } else {
        let title = if doc_badge.is_empty() {
            format!("{badge} SolidWorks 模型")
        } else {
            format!("{badge} {doc_badge}")
        };
        (title, format!("SolidWorks · {}", sw.doc_type))
    }
}

/// Deep-adapter enrichment entry point (used for stroke/box picks in SolidWorks).
pub fn run(ctx: &AdapterContext) -> Option<AdapterOutput> {
    if !matches(ctx) {
        return None;
    }
    let (x, y) = ctx.tap_point.unwrap_or((0, 0));
    let uia_label = ctx
        .primary_element
        .as_ref()
        .map(|e| e.name.as_str())
        .filter(|s| !s.trim().is_empty());

    let mut out = AdapterOutput::new("solidworks");
    if let Some(sw) = pick_solidworks_feature_com(&ctx.broker, x, y, uia_label) {
        if !sw.doc_path.is_empty() && Path::new(&sw.doc_path).exists() {
            out.paths.push(sw.doc_path.clone());
        }
        if let Some(first) = sw.items.first() {
            if !first.feature_name.is_empty() {
                out.badge = Some(first.feature_name.clone());
            }
        }
        if out.badge.is_none() && !sw.doc_title.is_empty() {
            out.badge = Some(sw.doc_title.clone());
        }
        out.lines.extend(format_solidworks_capsule_lines(&sw));
        return Some(out);
    }

    out.lines
        .push(format!("cad_window: {}", ctx.window_title.trim()));
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_feature_manager_tree_labels_for_chinese_and_english_ui() {
        assert_eq!(clean_feature_tree_label("(-) 草图1"), "草图1");
        assert_eq!(clean_feature_tree_label("凸台-拉伸1 [默认]"), "凸台-拉伸1");
        assert_eq!(
            clean_feature_tree_label("(f) 底座-1 (默认<<默认>_显示状态 1>)"),
            "底座-1"
        );
        assert_eq!(clean_feature_tree_label("Fillet1"), "Fillet1");
    }

    #[test]
    fn classifies_cardinal_normals_into_zegel_swe_semantic_directions() {
        assert_eq!(classify_semantic_direction([0.0, 0.0, 1.0]), Some("+z"));
        assert_eq!(classify_semantic_direction([0.0, 0.0, -0.999]), Some("-z"));
        assert_eq!(classify_semantic_direction([1.0, 0.0, 0.0]), Some("+x"));
        assert_eq!(classify_semantic_direction([0.5, 0.5, 0.707]), None);
    }

    #[test]
    fn inverts_identity_model_view_transform_to_expected_3d_ray() {
        // Identity rotation, translation (400, 300, 0), scale 1000 px/m
        let arr = [
            1.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, //
            0.0, 0.0, 1.0, //
            400.0, 300.0, 0.0, //
            1000.0, 0.0, 0.0, 0.0,
        ];
        let (orig, dir) = invert_model_view_transform_to_ray(&arr, 450.0, 320.0).unwrap();
        assert!((orig[0] - 0.05).abs() < 1e-6);
        assert!((orig[1] - 0.02).abs() < 1e-6);
        assert!((orig[2] - 10.0).abs() < 1e-6);
        assert!((dir[0]).abs() < 1e-6);
        assert!((dir[1]).abs() < 1e-6);
        assert!((dir[2] + 1.0).abs() < 1e-6);
    }

    #[test]
    fn formats_solidworks_capsule_with_sw_target_and_zegel_swe_mcp_hints() {
        let sw = SolidWorksPickResult {
            doc_title: "支架.SLDPRT".to_string(),
            doc_path: r"D:\CAD\支架.SLDPRT".to_string(),
            doc_type: "Part".to_string(),
            active_config: "默认".to_string(),
            pick_tier: "viewport_ray".to_string(),
            selection_count: 1,
            items: vec![SwPickedEntity {
                select_type_id: 2,
                select_type: "FACE".to_string(),
                feature_name: "凸台-拉伸1".to_string(),
                feature_type: "Extrusion".to_string(),
                face_count: 6,
                entity_geometry: "Plane, area=6000mm2".to_string(),
                normal: Some([0.0, 0.0, 1.0]),
                point_mm: Some([50.0, 30.0, 20.0]),
                body_index: Some(0),
                entity_index: Some(4),
                dimensions: vec![SwDimensionItem {
                    full_name: "D1@凸台-拉伸1".to_string(),
                    name: "D1".to_string(),
                    value: 20.0,
                    unit: "mm".to_string(),
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let (title, detail) = build_solidworks_chip_title("①", &sw);
        assert_eq!(title, "① 凸台-拉伸1 · 面 [+z]");
        assert_eq!(detail, "SolidWorks · 支架.SLDPRT");

        let lines = format_solidworks_capsule_lines(&sw);
        let joined = lines.join("\n");
        assert!(joined.contains("[SW-TARGET]"));
        assert!(joined.contains("feature=\"凸台-拉伸1\""));
        assert!(joined.contains("selector=\"__current_selection__ | +z\""));
        assert!(joined.contains("D1@凸台-拉伸1=20 mm"));
        assert!(joined.contains("solidworks_feature_info(feature_name=\"凸台-拉伸1\")"));
    }
}
