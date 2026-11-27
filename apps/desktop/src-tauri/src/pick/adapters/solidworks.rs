//! SolidWorks universal adapter & `sw-target/v2` protocol definitions.
//!
//! Strictly adheres to `anything-to-prompt-integration-spec.md`:
//! - Zero UIA traversal on `SLDWORKS.exe` (region detection uses lightweight Win32 `WindowFromPoint`
//!   + `GetClassNameW` only).
//! - Tool-agnostic two-part `[SW-TARGET v2]` capsule format (human summary + single-line compact
//!   UTF-8 JSON), with ZERO hardcoded MCP tool names.
//! - Exhaustive 149+ `swSelectType_e` mapping into 8 mutually-exclusive `SwTargetFamily` variants.

use crate::broker::{BrokerClient, BrokerRequest};
use crate::pick::adapters::office::fast_exe_basename_for_pid;
use crate::pick::adapters::{AdapterContext, AdapterOutput};
use crate::pick::{resolve_root_window_at, ScreenRect};
use crate::window::dpi_scale_for_hwnd;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::WindowsAndMessaging::{
    ChildWindowFromPointEx, EnumChildWindows, GetClassNameW, GetDlgCtrlID, GetParent,
    GetSystemMetrics, GetWindowLongW, GetWindowRect, IsWindowVisible, SendMessageTimeoutW,
    WindowFromPoint, CWP_SKIPINVISIBLE, CWP_SKIPTRANSPARENT, GWL_STYLE, SMTO_ABORTIFHUNG,
    SMTO_BLOCK, SM_CXICON, SM_CYICON,
};

// =============================================================================
// Spec-mandated protocol constants (`anything-to-prompt-integration-spec.md`)
// =============================================================================
pub const SW_SCHEMA_V2: &str = "sw-target/v2";
/// FR-1: Maximum number of selected items extracted from `ISelectionMgr`.
pub const SPEC_MAX_TARGETS: usize = 30;
/// FR-2 (#3): Maximum direct parent / child features per target.
pub const SPEC_MAX_PARENTS_CHILDREN: usize = 8;
/// FR-3: Maximum configuration names listed in `doc_context.doc.config_names`.
pub const SPEC_MAX_CONFIG_NAMES: usize = 10;
/// FR-3: Maximum error feature names listed in `doc_context.model_summary.error_features`.
pub const SPEC_MAX_ERROR_FEATURES: usize = 5;
/// FR-3: Maximum non-empty custom property entries in `doc_context.custom_properties`.
pub const SPEC_MAX_CUSTOM_PROPERTIES: usize = 20;
/// FR-4: Target P95 latency SLA for Mode A (ms).
pub const SPEC_TARGET_SLA_MS: u64 = 100;
/// FR-4: Hard timeout for a single read-only Mode A COM probe (`CurrentSelectionOnly`) (ms).
pub const SPEC_HARD_TIMEOUT_MS: u64 = 1500;
/// Hard timeout for interactive COM probes (`ViewportRayOnly` / `FeatureTreeClick` / `Auto`) (ms),
/// allowing SolidWorks Assemblies to complete B-Rep `SelectByRay` + topology enrichment without
/// spurious timeout degradation.
pub const SPEC_INTERACTIVE_TIMEOUT_MS: u64 = 3500;
/// FR-4: Delay before retrying once on `RPC_E_CALL_REJECTED` (ms).
pub const SPEC_BUSY_RETRY_DELAY_MS: u64 = 150;
/// FR-4: Number of retries on `RPC_E_CALL_REJECTED`.
pub const SPEC_BUSY_MAX_RETRIES: u32 = 1;

#[derive(Debug, Clone)]
pub struct SolidWorksWindowHit {
    pub root_hwnd: HWND,
    pub pid: u32,
    pub win_title: String,
    pub win_class: String,
    pub dpi_scale: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwWindowRegion {
    Viewport3D,
    FeatureTree,
    ToolbarOrMenu,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwTargetFamily {
    Topology,
    SketchEntity,
    FeatureOrRef,
    Body,
    Dimension,
    Assembly,
    DrawingAnnotation,
    GenericObject,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SwDocInfo {
    pub title: String,
    pub path: String,
    #[serde(rename = "type")]
    pub doc_type: String,
    pub active_config: String,
    pub config_names: Vec<String>,
    pub is_dirty: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SwEditState {
    pub in_sketch_edit: bool,
    pub active_sketch_name: String,
    pub in_component_edit: bool,
    pub editing_component: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SwUnits {
    pub length: String,
    pub angle: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SwModelSummary {
    pub solid_body_count: usize,
    pub surface_body_count: usize,
    pub bbox_mm: Option<[f64; 6]>,
    pub equation_count: usize,
    pub error_feature_count: usize,
    pub error_features: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SwDocContext {
    pub doc: SwDocInfo,
    pub edit_state: SwEditState,
    pub units: SwUnits,
    pub model_summary: SwModelSummary,
    pub custom_properties: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwTargetV2Item {
    pub index: usize,
    pub select_type_id: i32,
    pub select_type_name: String,
    pub family: SwTargetFamily,
    pub persist_ref_b64: Option<String>,
    pub mark: i32,
    pub display_name: String,
    pub point_mm: Option<[f64; 3]>,
    pub component_chain: Vec<String>,
    pub details: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwTargetV2Payload {
    pub schema: String,
    pub pick_mode: String,
    pub doc_context: SwDocContext,
    pub targets: Vec<SwTargetV2Item>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SwProbeOutcome {
    Ok(SwTargetV2Payload),
    Busy,
    TimedOut,
    Unavailable,
}

// =============================================================================
// Self-Calibrating Adaptive Latency Guard (Zero Magic Budgets)
// =============================================================================

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AdaptiveSkipFlags {
    pub skip_bbox: bool,
    pub skip_custom_props: bool,
    pub skip_equations: bool,
    pub skip_whats_wrong: bool,
}

/// Runtime self-calibrating guard anchored strictly to `SPEC_TARGET_SLA_MS` (`100ms`).
///
/// Measures real wall-clock duration of optional heavy `doc_context` stages per document.
/// Only when an optional stage repeatedly causes total pick time to breach `sla_ms` across
/// consecutive picks on the same document does it defer that specific stage (and automatically
/// resets whenever the active document changes).
#[derive(Debug, Clone)]
pub struct SwAdaptiveProfiler {
    sla_ms: u64,
    active_doc_key: String,
    consec_slow_bbox: u32,
    consec_slow_props: u32,
    consec_slow_eq: u32,
    consec_slow_err: u32,
    skip: AdaptiveSkipFlags,
}

impl SwAdaptiveProfiler {
    const CONSECUTIVE_CONFIRMATIONS: u32 = 3;

    pub fn new(sla_ms: u64) -> Self {
        Self {
            sla_ms: sla_ms.max(1),
            active_doc_key: String::new(),
            consec_slow_bbox: 0,
            consec_slow_props: 0,
            consec_slow_eq: 0,
            consec_slow_err: 0,
            skip: AdaptiveSkipFlags::default(),
        }
    }

    pub fn begin_doc(&mut self, doc_key: &str) -> AdaptiveSkipFlags {
        if self.active_doc_key != doc_key {
            self.active_doc_key = doc_key.to_string();
            self.consec_slow_bbox = 0;
            self.consec_slow_props = 0;
            self.consec_slow_eq = 0;
            self.consec_slow_err = 0;
            self.skip = AdaptiveSkipFlags::default();
        }
        self.skip
    }

    pub fn record_sample(
        &mut self,
        doc_key: &str,
        total_ms: u64,
        bbox_ms: u64,
        props_ms: u64,
        eq_ms: u64,
        err_ms: u64,
    ) {
        if self.active_doc_key != doc_key {
            return;
        }
        let half_sla = self.sla_ms / 2;
        let breached = total_ms > self.sla_ms;

        Self::update_stage(breached, bbox_ms, half_sla, &mut self.consec_slow_bbox, &mut self.skip.skip_bbox);
        Self::update_stage(breached, props_ms, half_sla, &mut self.consec_slow_props, &mut self.skip.skip_custom_props);
        Self::update_stage(breached, eq_ms, half_sla, &mut self.consec_slow_eq, &mut self.skip.skip_equations);
        Self::update_stage(breached, err_ms, half_sla, &mut self.consec_slow_err, &mut self.skip.skip_whats_wrong);
    }

    fn update_stage(
        breached_sla: bool,
        stage_ms: u64,
        stage_thresh: u64,
        counter: &mut u32,
        skip_flag: &mut bool,
    ) {
        if *skip_flag {
            return;
        }
        if breached_sla && stage_ms > stage_thresh {
            *counter += 1;
            if *counter >= Self::CONSECUTIVE_CONFIRMATIONS {
                *skip_flag = true;
            }
        } else {
            *counter = 0;
        }
    }
}

// =============================================================================
// Exhaustive SolidWorks `swSelectType_e` -> (SelectByID2 Name, SwTargetFamily)
// =============================================================================
pub fn sw_select_type_meta(type_id: i32) -> (&'static str, SwTargetFamily) {
    use SwTargetFamily::*;
    match type_id {
        // 1. Topology
        1 => ("EDGE", Topology),
        2 => ("FACE", Topology),
        3 => ("VERTEX", Topology),
        24 => ("LOOP", Topology),
        44 => ("SILHOUETTE", Topology),
        147 => ("MESHEDGE", Topology),
        148 => ("MESHFACET", Topology),
        149 => ("MESHVERTEX", Topology),

        // 2. Sketch Entity
        10 => ("SKETCHSEGMENT", SketchEntity),
        11 => ("SKETCHPOINT", SketchEntity),
        25 => ("EXTSKETCHSEGMENT", SketchEntity),
        26 => ("EXTSKETCHPOINT", SketchEntity),
        38 => ("SKETCHTEXT", SketchEntity),
        83 => ("SKETCHHATCH", SketchEntity),
        86 => ("MIDPOINT", SketchEntity),
        96 => ("SKETCHREGION", SketchEntity),
        97 => ("SKETCHCONTOUR", SketchEntity),

        // 3. Feature or Reference Geometry
        4 => ("PLANE", FeatureOrRef),
        5 => ("AXIS", FeatureOrRef),
        6 => ("DATUMPOINT", FeatureOrRef),
        8 => ("ATTRIBUTE", FeatureOrRef),
        9 => ("SKETCH", FeatureOrRef),
        22 => ("BODYFEATURE", FeatureOrRef),
        23 => ("REFCURVE", FeatureOrRef),
        27 => ("HELIX", FeatureOrRef),
        28 => ("REFERENCECURVES", FeatureOrRef),
        29 => ("REFSURFACE", FeatureOrRef),
        31 => ("INCONTEXTFEAT", FeatureOrRef),
        42 => ("WELD", FeatureOrRef),
        49 => ("COORDSYS", FeatureOrRef),
        50 => ("DATUM", FeatureOrRef),
        57 => ("EQNFOLDER", FeatureOrRef),
        58 => ("SENSOR", FeatureOrRef),
        60 => ("BELTCHAINFEATS", FeatureOrRef),
        84 => ("IMPORTFOLDER", FeatureOrRef),
        106 => ("WELDMENT", FeatureOrRef),
        107 => ("SUBWELDFOLDER", FeatureOrRef),
        112 => ("BDYFOLDER", FeatureOrRef),
        114 => ("SUBATOMFOLDER", FeatureOrRef),
        115 => ("WELDBEADS", FeatureOrRef),
        122 => ("SWIFTFEATURE", FeatureOrRef),
        125 => ("HOLESERIES", FeatureOrRef),
        131 => ("COSMETICWELD", FeatureOrRef),
        136 => ("SELECTIONSETFOLDER", FeatureOrRef),
        137 => ("SELECTIONSETNODE", FeatureOrRef),
        143 => ("ADVSTRUCTURALMEMBER", FeatureOrRef),
        144 => ("STRUCTURALSYSTEM", FeatureOrRef),
        145 => ("CORNERMANAGEMENT", FeatureOrRef),
        146 => ("BOUNDINGBOX", FeatureOrRef),

        // 4. Body
        75 => ("SURFACEBODY", Body),
        76 => ("SOLIDBODY", Body),
        78 => ("SURFBODIESFIRST", Body),
        80 => ("PICTUREBODIES", Body),
        81 => ("SOLIDBODIESFIRST", Body),
        141 => ("MESHFACETEDBODY", Body),
        142 => ("GRAPHICSBODY", Body),

        // 5. Dimension
        14 => ("DIMENSION", Dimension),

        // 6. Assembly
        20 => ("COMPONENT", Assembly),
        21 => ("MATE", Assembly),
        32 => ("MATEGROUP", Assembly),
        33 => ("EXPLODEDVIEWS", Assembly),
        34 => ("MATE", Assembly),
        41 => ("COMPPATTERN", Assembly),
        124 => ("MATESET", Assembly),

        // 7. Drawing & Annotations
        12 => ("DRAWINGVIEW", DrawingAnnotation),
        13 => ("GTOL", DrawingAnnotation),
        15 => ("NOTE", DrawingAnnotation),
        16 => ("SECTIONLINE", DrawingAnnotation),
        17 => ("DETAILCIRCLE", DrawingAnnotation),
        18 => ("SECTIONTEXT", DrawingAnnotation),
        19 => ("SHEET", DrawingAnnotation),
        30 => ("CENTERMARKSYMS", DrawingAnnotation),
        39 => ("SFSYMBOL", DrawingAnnotation),
        40 => ("DATUMTAG", DrawingAnnotation),
        47 => ("ARROW", DrawingAnnotation),
        48 => ("ZONE", DrawingAnnotation),
        54 => ("BOMTEMP", DrawingAnnotation),
        56 => ("BOM", DrawingAnnotation),
        59 => ("CUSTOMSYMBOL", DrawingAnnotation),
        82 => ("LEADER", DrawingAnnotation),
        87 => ("CUSTOMSYM", DrawingAnnotation),
        98 => ("ANNOTATIONTABLES", DrawingAnnotation),
        99 => ("BLOCKINST", DrawingAnnotation),
        103 => ("CENTERLINE", DrawingAnnotation),
        104 => ("HOLETABLEFEATS", DrawingAnnotation),
        105 => ("HOLETABLEAXES", DrawingAnnotation),
        109 => ("REVISIONTABLE", DrawingAnnotation),
        111 => ("WELDMENTTABLE", DrawingAnnotation),
        113 => ("REVISIONTABLEFEAT", DrawingAnnotation),
        121 => ("SWIFTANNOTATION", DrawingAnnotation),
        126 => ("LEADERPOINTS", DrawingAnnotation),
        128 => ("TITLEBLOCK", DrawingAnnotation),
        129 => ("TITLEBLOCKTABLEFEAT", DrawingAnnotation),
        132 => ("MAGNETICLINES", DrawingAnnotation),
        133 => ("PUNCHTABLEFEATS", DrawingAnnotation),
        134 => ("REVISIONCLOUDS", DrawingAnnotation),
        135 => ("BORDER", DrawingAnnotation),
        139 => ("BENDTABLE", DrawingAnnotation),
        140 => ("BENDTABLEFEAT", DrawingAnnotation),

        // 8. Generic Object Fallback (all remaining & future swSelectType_e IDs)
        7 => ("OLEITEM", GenericObject),
        45 => ("CONFIGURATIONS", GenericObject),
        46 => ("OBJHANDLES", GenericObject),
        51 => ("ROUTE", GenericObject),
        52 => ("CONNECTIONPOINT", GenericObject),
        53 => ("ROUTINGPOINT", GenericObject),
        55 => ("ROUTEPOINT", GenericObject),
        77 => ("FRAMEPOINT", GenericObject),
        79 => ("MANIPULATOR", GenericObject),
        85 => ("VIEWERHYPERLINK", GenericObject),
        88 => ("ROUTECURVE", GenericObject),
        108 => ("EXCLUDELIST", GenericObject),
        110 => ("SUBSKETCHINST", GenericObject),
        116 => ("EMBEDLINKDOC", GenericObject),
        117 => ("JOURNAL", GenericObject),
        118 => ("DOCSFOLDER", GenericObject),
        119 => ("COMMENTSFOLDER", GenericObject),
        120 => ("COMMENT", GenericObject),
        123 => ("CAMERAS", GenericObject),
        127 => ("SWIFTSCHEMA", GenericObject),
        130 => ("OBJGROUP", GenericObject),
        138 => ("HOLESERIESBOX", GenericObject),
        _ => ("UNKNOWN", GenericObject),
    }
}

pub fn is_solidworks_exe(exe_name: &str) -> bool {
    exe_name.eq_ignore_ascii_case("sldworks.exe")
}

pub fn matches(ctx: &AdapterContext) -> bool {
    is_solidworks_exe(&ctx.proc_meta.exe_name)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwPickMode {
    /// Strictly read-only `ISelectionMgr` probe (used on DoubleCtrl / PenStart auto-attach).
    CurrentSelectionOnly,
    /// Explicitly ray-pick at `(screen_x, screen_y)` in 3D viewport (never returns stale selection).
    ViewportRayOnly,
    /// Click a node in FeatureManager Tree and read the newly selected entity from `ISelectionMgr`.
    FeatureTreeClick,
    /// Mode A (`ISelectionMgr`) if non-empty, else Mode B (`SelectByRay`).
    Auto,
}

impl SwPickMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            SwPickMode::CurrentSelectionOnly => "current_selection_only",
            SwPickMode::ViewportRayOnly => "viewport_ray_only",
            SwPickMode::FeatureTreeClick => "feature_tree_click",
            SwPickMode::Auto => "auto",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim() {
            "current_selection_only" => SwPickMode::CurrentSelectionOnly,
            "viewport_ray_only" => SwPickMode::ViewportRayOnly,
            "feature_tree_click" => SwPickMode::FeatureTreeClick,
            _ => SwPickMode::Auto,
        }
    }
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

/// Zero-UIA SolidWorks window check by HWND (for `DoubleCtrlOpen` / `PenStart` auto-attach).
pub fn detect_solidworks_hwnd(hwnd: HWND) -> Option<SolidWorksWindowHit> {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetAncestor, GetClassNameW, GetWindowTextW, GetWindowThreadProcessId, GA_ROOT,
    };
    if hwnd.0.is_null() {
        return None;
    }
    let root_hwnd = unsafe {
        let r = GetAncestor(hwnd, GA_ROOT);
        if r.0.is_null() { hwnd } else { r }
    };
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(root_hwnd, Some(&mut pid));
    }
    if pid == 0 {
        return None;
    }
    let exe = fast_exe_basename_for_pid(pid);
    if !is_solidworks_exe(&exe) {
        return None;
    }
    let mut title_buf = [0u16; 256];
    let tlen = unsafe { GetWindowTextW(root_hwnd, &mut title_buf) }.max(0) as usize;
    let tnul = title_buf[..tlen].iter().position(|&c| c == 0).unwrap_or(tlen);
    let win_title = String::from_utf16_lossy(&title_buf[..tnul]);
    let mut cls_buf = [0u16; 128];
    let clen = unsafe { GetClassNameW(root_hwnd, &mut cls_buf) }.max(0) as usize;
    let cnul = cls_buf[..clen].iter().position(|&c| c == 0).unwrap_or(clen);
    let win_class = String::from_utf16_lossy(&cls_buf[..cnul]);
    Some(SolidWorksWindowHit {
        root_hwnd,
        pid,
        win_title,
        win_class,
        dpi_scale: dpi_scale_for_hwnd(root_hwnd),
    })
}

/// Canonical document key for SolidWorks per-entity deduplication.
pub fn payload_doc_key(sw: &SwTargetV2Payload) -> &str {
    if !sw.doc_context.doc.path.is_empty() {
        sw.doc_context.doc.path.as_str()
    } else {
        sw.doc_context.doc.title.as_str()
    }
}

/// Canonical per-entity identity key for deduplication across Bar sessions and continuous Ctrl picks.
/// Intentionally omits `point_mm` so clicking the same face/edge/feature at a slightly different
/// pixel coordinate produces the exact same key (preventing duplicate capsules).
pub fn target_entity_key(doc_key: &str, t: &SwTargetV2Item) -> String {
    if let Some(pref) = t.persist_ref_b64.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        format!("{doc_key}|{}|pid:{pref}", t.select_type_id)
    } else {
        format!("{doc_key}|{}|name:{}", t.select_type_id, t.display_name.trim())
    }
}

/// Deterministic fingerprint of a SolidWorks selection for session-level deduplication.
pub fn selection_fingerprint(sw: &SwTargetV2Payload) -> String {
    let doc_key = payload_doc_key(sw);
    let mut keys: Vec<String> = sw
        .targets
        .iter()
        .map(|t| target_entity_key(doc_key, t))
        .collect();
    keys.sort();
    keys.join("||")
}

/// Returns true if `hwnd` is a floating dropdown / cascading popup menu window (e.g. `#32768`,
/// SolidWorks BCGP/MFC `Afx:` popup menu, or small frameless `WS_POPUP` menu) rather than the main frame.
pub fn is_popup_menu_hwnd(hwnd: HWND, root_hwnd: HWND, cls_lower: &str) -> bool {
    if cls_lower == "#32768" || cls_lower.contains("popup") || cls_lower.contains("bcgppopup") {
        return true;
    }
    if hwnd == root_hwnd {
        let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) } as u32;
        const WS_POPUP_BIT: u32 = 0x8000_0000;
        const WS_CAPTION_BIT: u32 = 0x00C0_0000;
        if (style & WS_POPUP_BIT) != 0 && (style & WS_CAPTION_BIT) != WS_CAPTION_BIT {
            let mut rc = RECT::default();
            if unsafe { GetWindowRect(hwnd, &mut rc) }.is_ok() {
                let w = (rc.right - rc.left).max(0);
                let h = (rc.bottom - rc.top).max(0);
                if (60..=720).contains(&w) && (24..=1200).contains(&h) {
                    return true;
                }
            }
        }
    }
    false
}

/// Zero-UIA lightweight Win32 region classifier for `SLDWORKS.exe` (P0-1).
/// Only calls `WindowFromPoint` + `GetClassNameW` + `GetWindowRect` (<0.1ms).
pub fn classify_solidworks_region_at(root_hwnd: HWND, x: i32, y: i32) -> SwWindowRegion {
    let child = unsafe { WindowFromPoint(POINT { x, y }) };
    if child.0.is_null() {
        return SwWindowRegion::Other;
    }
    let mut buf = [0u16; 128];
    let len = unsafe { GetClassNameW(child, &mut buf) }.max(0) as usize;
    let cls = String::from_utf16_lossy(&buf[..len]);
    let cls_lower = cls.to_ascii_lowercase();

    // Popup / cascading dropdown menus MUST NEVER be misclassified as 3D Viewport!
    if is_popup_menu_hwnd(child, root_hwnd, &cls_lower) {
        return SwWindowRegion::ToolbarOrMenu;
    }

    if cls_lower.contains("systreeview32") {
        return SwWindowRegion::FeatureTree;
    }
    if cls_lower.contains("syslistview32") {
        return SwWindowRegion::ToolbarOrMenu;
    }
    if cls_lower.contains("toolbarwindow32")
        || cls_lower.contains("statusbar")
        || cls_lower.contains("rebarwindow32")
        || cls_lower.contains("netuihwnd")
        || cls_lower == "#32768"
        || cls_lower == "button"
        || cls_lower == "combobox"
        || cls_lower == "edit"
        || cls_lower == "systabcontrol32"
        || cls_lower.contains("afxcontrolbar")
    {
        return SwWindowRegion::ToolbarOrMenu;
    }

    // SolidWorks 3D graphics viewport uses `AfxFrameOrView...` / `MDIClient` / OpenGL child windows
    // occupying a substantial area inside the root frame.
    let mut child_rc = RECT::default();
    let mut root_rc = RECT::default();
    let (cw, ch) = if unsafe { GetWindowRect(child, &mut child_rc) }.is_ok() {
        ((child_rc.right - child_rc.left).max(0), (child_rc.bottom - child_rc.top).max(0))
    } else {
        (0, 0)
    };
    let (rw, rh) = if unsafe { GetWindowRect(root_hwnd, &mut root_rc) }.is_ok() {
        ((root_rc.right - root_rc.left).max(1), (root_rc.bottom - root_rc.top).max(1))
    } else {
        (1, 1)
    };

    if (cls_lower.starts_with("afxframeorview")
        || cls_lower.starts_with("afx:")
        || cls_lower.contains("mdiclient")
        || cls_lower.contains("opengl")
        || cls_lower.contains("swgraphics"))
        && cw * 4 >= rw
        && ch * 4 >= rh
    {
        return SwWindowRegion::Viewport3D;
    }

    SwWindowRegion::Other
}

/// Pure Win32 (zero-UIA, non-blocking) metadata for a SolidWorks button, option, checkbox,
/// PropertyManager control, dialog widget, toolbar icon, or menu item.
#[derive(Debug, Clone)]
pub struct SwWin32ControlInfo {
    pub hwnd_raw: isize,
    pub class_name: String,
    pub role: String,
    pub text: String,
    pub description: String,
    pub checked: Option<bool>,
    pub ctrl_id: i32,
    pub rect: ScreenRect,
    pub is_composite: bool,
    pub parent_chain: Vec<String>,
}

/// Checks if a Win32 control name is merely an internal SolidWorks MFC container class/title
/// (e.g. `swCmdMgr`, `CommandManager`) rather than an actual user-facing button label.
/// Explicitly preserves real English SolidWorks commands starting with "Sw" (`Sweep`, `Swept Cut`, `Switch Window`).
pub fn is_generic_sw_container_label(label: &str) -> bool {
    let t = label.trim();
    if t.is_empty() {
        return true;
    }
    let lower = t.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "swcmdmgr"
            | "swtoolbar"
            | "swpane"
            | "swtab"
            | "swmenu"
            | "swdockbar"
            | "swframe"
            | "commandmanager"
            | "toolbar"
            | "menubar"
            | "statusbar"
            | "ribbon"
            | "panel"
            | "pane"
            | "client"
    ) || lower.starts_with("afx:")
}

/// Sanitizes WinRT OCR output on SolidWorks ribbon/toolbar buttons by stripping stray leading or
/// trailing icon artifacts (e.g., `@`, `。`, `0` misread from circular icons, dropdown triangles).
pub fn clean_sw_ocr_label(raw: &str) -> String {
    let mut s = raw
        .trim_matches(|c: char| {
            c.is_whitespace()
                || matches!(
                    c,
                    '.' | '·'
                        | '．'
                        | ','
                        | '，'
                        | ';'
                        | ':'
                        | '|'
                        | '!'
                        | '`'
                        | '\''
                        | '"'
                        | '•'
                        | '@'
                        | '。'
                        | '°'
                        | '□'
                        | '■'
                        | '◆'
                        | '▲'
                        | '▼'
                        | '▾'
                        | '▸'
                        | '~'
                        | '_'
                        | '*'
                )
        })
        .to_string();
    // Strip stray leading ASCII digit/punctuation noise directly preceding CJK text
    // (e.g. `0向导` or `0异型孔向导` when a circular hole icon leaks into the crop edge)
    loop {
        let mut chars = s.chars();
        let Some(first) = chars.next() else {
            break;
        };
        let Some(second) = chars.next() else {
            break;
        };
        let is_cjk = |c: char| ('\u{4E00}'..='\u{9FFF}').contains(&c);
        if (first.is_ascii_digit() || matches!(first, 'o' | 'O' | 'Q' | 'c' | 'C' | '-'))
            && is_cjk(second)
        {
            s = s[first.len_utf8()..].to_string();
            continue;
        }
        break;
    }
    s
}

/// Detects the exact pixel bounding box of a SolidWorks CommandManager / MFC BCGControlBar button
/// currently highlighted by SolidWorks's native hover state (e.g. crisp `#FFFFFF` fill + `#B1B1B1`
/// 1px border on `#F7F7F7` light toolbar, or distinct uniform highlight fill on Medium/Dark themes).
///
/// Uses a **position-invariant 2D connected-component BFS** on the button's uniform hover fill so
/// the detected rectangle flows cleanly *around* opaque icons and Chinese text strokes, picking the
/// largest connected hover-fill component whose bounding box contains `(cursor_x, cursor_y)`.
/// This guarantees the blue overlay frame stays 100% rock-solid (zero shrinking/flickering) as the
/// cursor moves anywhere across the icon, text, or margins of a button (~0.15ms, zero UIA/MSAA).
pub fn detect_hovered_button_rect_in_rgba(
    img: &image::RgbaImage,
    origin_x: i32,
    origin_y: i32,
    toolbar_rc: RECT,
    cursor_x: i32,
    cursor_y: i32,
) -> Option<ScreenRect> {
    let iw = img.width() as i32;
    let ih = img.height() as i32;
    if iw < 24 || ih < 16 {
        return None;
    }

    // Restrict scan strictly inside the toolbar window intersected with captured image
    let x_min = (toolbar_rc.left - origin_x).clamp(0, iw - 1);
    let y_min = (toolbar_rc.top - origin_y).clamp(0, ih - 1);
    let x_max = (toolbar_rc.right - origin_x - 1).clamp(x_min, iw - 1);
    let y_max = (toolbar_rc.bottom - origin_y - 1).clamp(y_min, ih - 1);
    let lx = cursor_x - origin_x;
    let ly = cursor_y - origin_y;
    if lx < x_min || lx > x_max || ly < y_min || ly > y_max {
        return None;
    }

    let px_rgb = |x: i32, y: i32| -> [u8; 3] {
        let p = img.get_pixel(x as u32, y as u32);
        [p[0], p[1], p[2]]
    };
    let max_diff = |a: [u8; 3], b: [u8; 3]| -> i32 {
        (a[0] as i32 - b[0] as i32)
            .abs()
            .max((a[1] as i32 - b[1] as i32).abs())
            .max((a[2] as i32 - b[2] as i32).abs())
    };
    let sat = |c: [u8; 3]| -> i32 {
        let hi = c[0].max(c[1]).max(c[2]) as i32;
        let lo = c[0].min(c[1]).min(c[2]) as i32;
        hi - lo
    };
    let lum = |c: [u8; 3]| -> i32 { (c[0] as i32 * 77 + c[1] as i32 * 150 + c[2] as i32 * 29) >> 8 };

    // 1. Estimate toolbar background color `bg` as mode of low-saturation pixels along strip margins
    let mut bg_bins: std::collections::HashMap<(u8, u8, u8), u32> = std::collections::HashMap::new();
    let sample_bg = |x: i32, y: i32, bins: &mut std::collections::HashMap<(u8, u8, u8), u32>| {
        let c = px_rgb(x, y);
        if sat(c) <= 24 {
            let k = (c[0] & !3, c[1] & !3, c[2] & !3);
            *bins.entry(k).or_insert(0) += 1;
        }
    };
    for x in (x_min..=x_max).step_by(2) {
        sample_bg(x, (y_min + 1).min(y_max), &mut bg_bins);
        sample_bg(x, y_max.saturating_sub(1).max(y_min), &mut bg_bins);
    }
    for y in (y_min..=y_max).step_by(2) {
        sample_bg((x_min + 1).min(x_max), y, &mut bg_bins);
        sample_bg(x_max.saturating_sub(1).max(x_min), y, &mut bg_bins);
    }
    for y in (y_min..=y_max).step_by(6) {
        for x in (x_min..=x_max).step_by(8) {
            sample_bg(x, y, &mut bg_bins);
        }
    }
    let bg_q = bg_bins.into_iter().max_by_key(|(_, cnt)| *cnt)?.0;
    let bg = [bg_q.0, bg_q.1, bg_q.2];
    let bg_lum = lum(bg);

    // 2. Find dominant uniform hover fill color `F` across a generous window around `(lx, ly)`
    type FillBinMap = std::collections::HashMap<(u8, u8, u8), (u32, u64, u64, u64)>;
    let mut fill_bins: FillBinMap = std::collections::HashMap::new();
    let wx0 = (lx - 44).max(x_min);
    let wx1 = (lx + 44).min(x_max);
    let wy0 = (ly - 34).max(y_min);
    let wy1 = (ly + 34).min(y_max);
    for y in wy0..=wy1 {
        for x in wx0..=wx1 {
            let c = px_rgb(x, y);
            let d = max_diff(c, bg);
            if !(6..=95).contains(&d) {
                continue;
            }
            let s = sat(c);
            let l = lum(c);
            let is_accent_blue = c[2] >= c[0] && (c[2] as i32 - c[0] as i32) <= 65 && s <= 75;
            if s > 32 && !is_accent_blue {
                continue;
            }
            if bg_lum >= 180 && l < 175 {
                continue;
            }
            if bg_lum <= 90 && l > 155 {
                continue;
            }
            let k = (c[0] & !3, c[1] & !3, c[2] & !3);
            let entry = fill_bins.entry(k).or_insert((0, 0, 0, 0));
            entry.0 += 1;
            entry.1 += c[0] as u64;
            entry.2 += c[1] as u64;
            entry.3 += c[2] as u64;
        }
    }

    let (_, (fill_cnt, sr, sg, sb)) = fill_bins.into_iter().max_by_key(|(_, v)| v.0)?;
    if fill_cnt < 48 {
        return None;
    }
    let fill = [
        (sr / fill_cnt as u64) as u8,
        (sg / fill_cnt as u64) as u8,
        (sb / fill_cnt as u64) as u8,
    ];
    if max_diff(fill, bg) < 6 {
        return None;
    }

    let is_border = |x: i32, y: i32| -> bool {
        let c = px_rgb(x, y);
        sat(c) <= 45 && max_diff(c, bg) >= 16 && max_diff(c, fill) >= 16
    };

    // 3. Build a compact 2D boolean mask of `is_fill` pixels in a local ROI around `(lx, ly)` and
    // find all 4-connected components via BFS. Pick the LARGEST component whose bounding box
    // contains `(lx, ly)` (or sits immediately above `(lx, ly)` for a split-button dropdown tail!).
    let roi_x0 = (lx - 150).max(x_min);
    let roi_x1 = (lx + 150).min(x_max);
    let roi_y0 = (ly - 84).max(y_min);
    let roi_y1 = (ly + 84).min(y_max);
    let rw = (roi_x1 - roi_x0 + 1) as usize;
    let rh = (roi_y1 - roi_y0 + 1) as usize;
    if rw < 16 || rh < 14 {
        return None;
    }

    let mut mask = vec![false; rw * rh];
    for ry in 0..rh {
        let y = roi_y0 + ry as i32;
        for rx in 0..rw {
            let x = roi_x0 + rx as i32;
            mask[ry * rw + rx] = max_diff(px_rgb(x, y), fill) <= 5;
        }
    }

    let mut visited = vec![false; rw * rh];
    let mut queue: Vec<(usize, usize)> = Vec::with_capacity(2048);
    let mut best_comp: Option<(u32, i32, i32, i32, i32)> = None; // (area, x0, y0, x1, y1)

    for sy in 0..rh {
        for sx in 0..rw {
            let idx = sy * rw + sx;
            if !mask[idx] || visited[idx] {
                continue;
            }
            visited[idx] = true;
            queue.clear();
            queue.push((sx, sy));
            let mut head = 0usize;
            let mut min_x = sx;
            let mut max_x = sx;
            let mut min_y = sy;
            let mut max_y = sy;
            let mut area = 0u32;

            while head < queue.len() {
                let (cx, cy) = queue[head];
                head += 1;
                area += 1;
                if cx < min_x {
                    min_x = cx;
                }
                if cx > max_x {
                    max_x = cx;
                }
                if cy < min_y {
                    min_y = cy;
                }
                if cy > max_y {
                    max_y = cy;
                }
                // 4-connected neighbors
                if cx > 0 {
                    let ni = cy * rw + (cx - 1);
                    if mask[ni] && !visited[ni] {
                        visited[ni] = true;
                        queue.push((cx - 1, cy));
                    }
                }
                if cx + 1 < rw {
                    let ni = cy * rw + (cx + 1);
                    if mask[ni] && !visited[ni] {
                        visited[ni] = true;
                        queue.push((cx + 1, cy));
                    }
                }
                if cy > 0 {
                    let ni = (cy - 1) * rw + cx;
                    if mask[ni] && !visited[ni] {
                        visited[ni] = true;
                        queue.push((cx, cy - 1));
                    }
                }
                if cy + 1 < rh {
                    let ni = (cy + 1) * rw + cx;
                    if mask[ni] && !visited[ni] {
                        visited[ni] = true;
                        queue.push((cx, cy + 1));
                    }
                }
            }

            if area < 48 {
                continue;
            }
            let cx0 = roi_x0 + min_x as i32;
            let cy0 = roi_y0 + min_y as i32;
            let cx1 = roi_x0 + max_x as i32;
            let cy1 = roi_y0 + max_y as i32;
            let cw = cx1 - cx0 + 1;
            let ch = cy1 - cy0 + 1;
            // Generous HiDPI-safe bounds (supports 100%–200% DPI CommandManager buttons)
            if !(14..=360).contains(&cw) || !(14..=180).contains(&ch) {
                continue;
            }

            // Accept if cursor is inside this component's bounding box (with 3px border margin)
            // OR cursor is in a split-button dropdown tail directly below a tall button (`<= 36px` below).
            let inside_main = lx >= cx0 - 3 && lx <= cx1 + 3 && ly >= cy0 - 3 && ly <= cy1 + 3;
            let inside_split_tail =
                ch >= 38 && cw <= 140 && lx >= cx0 - 2 && lx <= cx1 + 2 && ly > cy1 && ly <= cy1 + 36;
            if !inside_main && !inside_split_tail {
                continue;
            }

            if best_comp.is_none_or(|(best_area, ..)| area > best_area) {
                best_comp = Some((area, cx0, cy0, cx1, cy1));
            }
        }
    }

    let (_, mut bx0, mut by0, mut bx1, mut by1) = best_comp?;

    // 4. Include the crisp 1px outer border if present right outside `[bx0..=bx1, by0..=by1]`
    let w_now = (bx1 - bx0 + 1).max(1);
    let h_now = (by1 - by0 + 1).max(1);
    if by0 > y_min
        && (bx0..=bx1).filter(|&x| is_border(x, by0 - 1)).count() * 2 >= w_now as usize
    {
        by0 -= 1;
    }
    if by1 < y_max
        && (bx0..=bx1).filter(|&x| is_border(x, by1 + 1)).count() * 2 >= w_now as usize
    {
        by1 += 1;
    }
    if bx0 > x_min
        && (by0..=by1).filter(|&y| is_border(bx0 - 1, y)).count() * 2 >= h_now as usize
    {
        bx0 -= 1;
    }
    if bx1 < x_max
        && (by0..=by1).filter(|&y| is_border(bx1 + 1, y)).count() * 2 >= h_now as usize
    {
        bx1 += 1;
    }

    // 5. Split-button pairing (both directions!):
    // (a) Hovering main top half -> extend down to include the 12..26px dropdown arrow tail.
    // (b) Hovering bottom dropdown arrow tail (`cur_h in 12..=26`) -> extend UP to include the
    //     36..68px main icon+caption half directly above so OCR reads `异型孔向导` instead of `▾`!
    let cur_w = bx1 - bx0 + 1;
    let cur_h = by1 - by0 + 1;
    if cur_h >= 40 && (28..=96).contains(&cur_w) && by1 + 10 <= y_max {
        let mut tail_y = by1 + 1;
        let max_tail = (by1 + 26).min(y_max);
        let mut found_bottom = None;
        while tail_y <= max_tail {
            let side_border = is_border(bx0, tail_y) && is_border(bx1, tail_y);
            let bottom_bar =
                (bx0..=bx1).filter(|&x| is_border(x, tail_y)).count() * 2 >= cur_w as usize;
            if bottom_bar && tail_y >= by1 + 10 {
                found_bottom = Some(tail_y);
                break;
            }
            if !side_border {
                break;
            }
            tail_y += 1;
        }
        if let Some(tb) = found_bottom {
            by1 = tb;
        }
    } else if (12..=26).contains(&cur_h) && (28..=96).contains(&cur_w) && by0 >= y_min + 36 {
        let mut head_y = by0 - 1;
        let min_head = (by0 - 72).max(y_min);
        let mut found_top = None;
        while head_y >= min_head {
            let side_border = is_border(bx0, head_y) && is_border(bx1, head_y);
            let top_bar =
                (bx0..=bx1).filter(|&x| is_border(x, head_y)).count() * 2 >= cur_w as usize;
            if top_bar && by0 - head_y >= 34 {
                found_top = Some(head_y);
                break;
            }
            if !side_border {
                break;
            }
            if head_y == min_head {
                break;
            }
            head_y -= 1;
        }
        if let Some(ht) = found_top {
            by0 = ht;
        }
    }

    let final_w = bx1 - bx0 + 1;
    let final_h = by1 - by0 + 1;
    if !(16..=360).contains(&final_w) || !(16..=192).contains(&final_h) {
        return None;
    }
    let sx0 = origin_x + bx0;
    let sy0 = origin_y + by0;
    if cursor_x < sx0 - 3
        || cursor_x > sx0 + final_w + 3
        || cursor_y < sy0 - 3
        || cursor_y > sy0 + final_h + 3
    {
        return None;
    }

    Some(ScreenRect {
        x: sx0,
        y: sy0,
        w: final_w,
        h: final_h,
    })
}

/// Fast (<0.5ms) zero-UIA screen strip capture + native SolidWorks hover-highlight rect detection,
/// with a lightweight containment hysteresis lock so micro-glitches on opaque icons or split-button
/// seams NEVER cause the blue hover frame to shrink or flicker while moving inside the same button!
fn detect_sw_toolbar_hovered_button_rect(
    toolbar_hwnd: isize,
    toolbar_rc: RECT,
    x: i32,
    y: i32,
) -> Option<ScreenRect> {
    use std::sync::{Mutex, OnceLock};
    static LAST_BTN: OnceLock<Mutex<Option<(isize, ScreenRect)>>> = OnceLock::new();
    let cache = LAST_BTN.get_or_init(|| Mutex::new(None));

    let strip_x0 = (x - 160).max(toolbar_rc.left);
    let strip_x1 = (x + 160).min(toolbar_rc.right);
    let strip_y0 = (y - 96).max(toolbar_rc.top);
    let strip_y1 = (y + 96).min(toolbar_rc.bottom);
    let sw = strip_x1 - strip_x0;
    let sh = strip_y1 - strip_y0;
    if sw < 24 || sh < 16 {
        return None;
    }
    let cap = crate::captures::capture_screen_rect_rgba(strip_x0, strip_y0, sw, sh).ok()?;
    let detected =
        detect_hovered_button_rect_in_rgba(&cap.image, cap.origin_x, cap.origin_y, toolbar_rc, x, y);

    let mut guard = cache.lock().ok()?;
    if let Some((prev_hwnd, prev_rc)) = *guard {
        let inside_prev = prev_hwnd == toolbar_hwnd
            && x >= prev_rc.x
            && x <= prev_rc.x + prev_rc.w
            && y >= prev_rc.y
            && y <= prev_rc.y + prev_rc.h;
        if inside_prev {
            match detected {
                Some(new_rc) => {
                    // If the new rect is strictly smaller and nested inside `prev_rc` (e.g. cursor
                    // crossed a solid icon seam or split divider), keep `prev_rc` rock-solid!
                    let nested_smaller = new_rc.x >= prev_rc.x - 2
                        && new_rc.y >= prev_rc.y - 2
                        && new_rc.x + new_rc.w <= prev_rc.x + prev_rc.w + 2
                        && new_rc.y + new_rc.h <= prev_rc.y + prev_rc.h + 2
                        && (new_rc.w * new_rc.h) < (prev_rc.w * prev_rc.h);
                    if nested_smaller {
                        return Some(prev_rc);
                    }
                    *guard = Some((toolbar_hwnd, new_rc));
                    return Some(new_rc);
                }
                None => {
                    // Cursor is still inside `prev_rc` (e.g. transiently over a dense dark icon);
                    // keep `prev_rc` ONLY if SolidWorks is still showing a highlight inside it.
                    let cx0 = (prev_rc.x + 2 - cap.origin_x).clamp(0, cap.image.width() as i32 - 1);
                    let cy0 = (prev_rc.y + 2 - cap.origin_y).clamp(0, cap.image.height() as i32 - 1);
                    let cx1 = (prev_rc.x + prev_rc.w - 3 - cap.origin_x).clamp(cx0, cap.image.width() as i32 - 1);
                    let cy1 = (prev_rc.y + prev_rc.h - 3 - cap.origin_y).clamp(cy0, cap.image.height() as i32 - 1);
                    // Sample outside top-left margin as reference bg
                    let ref_px = cap.image.get_pixel(0, 0);
                    let mut bright_diff = 0u32;
                    for sx in (cx0..=cx1).step_by(3) {
                        let p = cap.image.get_pixel(sx as u32, cy0 as u32);
                        let d = (p[0] as i32 - ref_px[0] as i32)
                            .abs()
                            .max((p[1] as i32 - ref_px[1] as i32).abs())
                            .max((p[2] as i32 - ref_px[2] as i32).abs());
                        if d >= 6 {
                            bright_diff += 1;
                        }
                    }
                    if bright_diff >= 4 && cy1 > cy0 {
                        return Some(prev_rc);
                    }
                    *guard = None;
                    return None;
                }
            }
        }
    }

    if let Some(rc) = detected {
        *guard = Some((toolbar_hwnd, rc));
        Some(rc)
    } else {
        *guard = None;
        None
    }
}

/// Zero-UIA non-blocking reader for SolidWorks's bottom Status Bar help text (displays the exact
/// official command description whenever the mouse hovers over any CommandManager/toolbar button).
pub fn read_solidworks_status_bar_text(root_hwnd: HWND) -> String {
    struct Ctx {
        root_bottom: i32,
        found: Option<HWND>,
    }
    unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = &mut *(lparam.0 as *mut Ctx);
        if !IsWindowVisible(hwnd).as_bool() {
            return BOOL(1);
        }
        let mut cls_buf = [0u16; 96];
        let clen = GetClassNameW(hwnd, &mut cls_buf).max(0) as usize;
        let cls = String::from_utf16_lossy(&cls_buf[..clen]).to_ascii_lowercase();
        if cls.contains("statusbar") {
            let mut rc = RECT::default();
            if GetWindowRect(hwnd, &mut rc).is_ok() && rc.bottom >= ctx.root_bottom - 60 {
                ctx.found = Some(hwnd);
                return BOOL(0);
            }
        }
        BOOL(1)
    }

    let mut root_rc = RECT::default();
    if unsafe { GetWindowRect(root_hwnd, &mut root_rc) }.is_err() {
        return String::new();
    }
    let mut ctx = Ctx {
        root_bottom: root_rc.bottom,
        found: None,
    };
    unsafe {
        let _ = EnumChildWindows(
            Some(root_hwnd),
            Some(enum_cb),
            LPARAM(&mut ctx as *mut Ctx as isize),
        );
    }
    let Some(sb_hwnd) = ctx.found else {
        return String::new();
    };
    // Try standard WM_GETTEXT first (works on MFC CStatusBar / BCGPStatusBar)
    let txt = read_hwnd_text_nonblocking(sb_hwnd, 60);
    let trimmed = txt.trim();
    if !trimmed.is_empty() && trimmed != "就绪" && !trimmed.eq_ignore_ascii_case("ready") {
        return trimmed.to_string();
    }
    String::new()
}

/// Pure local Windows WinRT OCR fallback (`Windows.Media.Ocr`, ~15ms, zero process touch) on a
/// small BGRA crop around a SolidWorks toolbar/ribbon button.
///
/// When `is_exact_button == true`, `crop_rect` is the exact button bounding box detected from
/// SolidWorks's native hover frame — we strip the icon zone (left icon on small horizontal buttons,
/// top icon + bottom dropdown arrow on large vertical buttons) so WinRT OCR ONLY sees the crisp
/// single-button caption with zero icon garbage and zero bleed from adjacent buttons!
pub fn ocr_crop_from_raw_capture(
    raw: &crate::captures::RawScreenCapture,
    crop_rect: ScreenRect,
    click_pt: (i32, i32),
    is_exact_button: bool,
) -> Option<String> {
    use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap};
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::DataWriter;

    let img_w = raw.image.width();
    let img_h = raw.image.height();
    let rgba = raw.image.as_raw();
    if img_w == 0 || img_h == 0 || rgba.is_empty() {
        return None;
    }

    // Compute crop bounds in image coordinates
    let (rx0, ry0, rx1, ry1) = if is_exact_button && crop_rect.w >= 20 && crop_rect.h >= 16 {
        let mut bx0 = crop_rect.x + 1;
        let mut by0 = crop_rect.y + 1;
        let bx1 = crop_rect.x + crop_rect.w - 1;
        let mut by1 = crop_rect.y + crop_rect.h - 1;

        if crop_rect.h <= 36 && crop_rect.w > crop_rect.h + 14 {
            // Small horizontal ribbon button (e.g. `扫描切除`, `放样切除`, `边界切除`):
            // 16x16 icon sits on the left ~20..24px; strip it so OCR sees ONLY the clean text!
            let icon_skip = (crop_rect.h - 2).clamp(18, 26);
            if bx0 + icon_skip + 10 < bx1 {
                bx0 += icon_skip;
            }
        } else if crop_rect.h >= 44 && crop_rect.w <= 128 {
            // Large vertical ribbon button (e.g. `线性阵列`, `异型孔向导`, `拉伸凸台/基体`):
            // Icon occupies top ~20..22px; text lines start around +23px; split-button dropdown
            // arrow sits in bottom ~16px (when total height >= 68px).
            let top_skip = (crop_rect.h * 28 / 100).clamp(18, 22);
            if by0 + top_skip + 14 < by1 {
                by0 += top_skip;
            }
            if crop_rect.h >= 68 && by1 - 11 > by0 + 14 {
                by1 -= 11;
            }
        }

        (
            (bx0 - raw.origin_x).clamp(0, img_w as i32 - 1) as u32,
            (by0 - raw.origin_y).clamp(0, img_h as i32 - 1) as u32,
            (bx1 - raw.origin_x).clamp(0, img_w as i32) as u32,
            (by1 - raw.origin_y).clamp(0, img_h as i32) as u32,
        )
    } else {
        // Fallback strip when no exact button frame was detected (e.g. disabled button)
        let half_w = (crop_rect.w * 2).max(130);
        let half_h = (crop_rect.h).max(60);
        (
            (click_pt.0 - half_w - raw.origin_x).clamp(0, img_w as i32 - 1) as u32,
            (click_pt.1 - half_h - raw.origin_y).clamp(0, img_h as i32 - 1) as u32,
            (click_pt.0 + half_w - raw.origin_x).clamp(0, img_w as i32) as u32,
            (click_pt.1 + half_h - raw.origin_y).clamp(0, img_h as i32) as u32,
        )
    };

    if rx1 <= rx0 + 6 || ry1 <= ry0 + 6 {
        return None;
    }
    let cw = rx1 - rx0;
    let ch = ry1 - ry0;

    // Smooth 4× (exact button) or 3× (strip) upscale + grayscale luminance + generous uniform
    // padding around exact-button crops so WinRT OCR has crisp stroke separation on 9pt Chinese.
    let scale = if is_exact_button { 4u32 } else { 3u32 };
    let pad = if is_exact_button { 24u32 } else { 0u32 };
    let uw = cw * scale;
    let uh = ch * scale;
    let ow = uw + pad * 2;
    let oh = uh + pad * 2;
    let cropped = image::imageops::crop_imm(&raw.image, rx0, ry0, cw, ch).to_image();
    let upscaled = image::imageops::resize(
        &cropped,
        uw,
        uh,
        image::imageops::FilterType::CatmullRom,
    );

    // Estimate uniform padding luminance from crop top-left corner so Dark & Light themes both pad cleanly
    let corner_px = cropped.get_pixel(0, 0);
    let pad_lum = ((corner_px[0] as u32 * 77 + corner_px[1] as u32 * 150 + corner_px[2] as u32 * 29) >> 8) as u8;
    let mut bgra = vec![pad_lum; (ow * oh * 4) as usize];
    for a in bgra.iter_mut().skip(3).step_by(4) {
        *a = 255;
    }
    for y in 0..uh {
        for x in 0..uw {
            let px = upscaled.get_pixel(x, y);
            let mut lum = ((px[0] as u32 * 77 + px[1] as u32 * 150 + px[2] as u32 * 29) >> 8) as i32;
            // Mild contrast stretch around mid-tone on exact-button crops for crisp CJK strokes
            if is_exact_button {
                lum = ((lum - 160) * 135 / 100 + 160).clamp(0, 255);
            }
            let lum_u8 = lum as u8;
            let di = (((y + pad) * ow + (x + pad)) * 4) as usize;
            bgra[di] = lum_u8;
            bgra[di + 1] = lum_u8;
            bgra[di + 2] = lum_u8;
            bgra[di + 3] = 255;
        }
    }

    let engine = OcrEngine::TryCreateFromUserProfileLanguages().ok()?;
    let writer = DataWriter::new().ok()?;
    writer.WriteBytes(&bgra).ok()?;
    let buf = writer.DetachBuffer().ok()?;
    let bitmap = SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
        &buf,
        BitmapPixelFormat::Bgra8,
        ow as i32,
        oh as i32,
        BitmapAlphaMode::Premultiplied,
    )
    .ok()?;
    let result = engine.RecognizeAsync(&bitmap).ok()?.join().ok()?;

    // For exact single-button crops, every recognized line belongs exclusively to THIS button!
    if is_exact_button {
        let mut lines_sorted: Vec<(f32, String)> = Vec::new();
        if let Ok(lines) = result.Lines() {
            for line in lines {
                let Ok(words) = line.Words() else {
                    continue;
                };
                let mut line_txt = String::new();
                let mut min_y = f32::MAX;
                for word in words {
                    if let Ok(wt) = word.Text() {
                        line_txt.push_str(wt.to_string().trim());
                    }
                    if let Ok(br) = word.BoundingRect() {
                        min_y = min_y.min(br.Y);
                    }
                }
                if !line_txt.is_empty() {
                    lines_sorted.push((min_y, line_txt));
                }
            }
        }
        lines_sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let joined: String = if lines_sorted.is_empty() {
            result.Text().ok()?.to_string().replace(['\r', '\n', ' '], "")
        } else {
            lines_sorted.into_iter().map(|(_, t)| t).collect()
        };
        let cleaned = clean_sw_ocr_label(&joined);
        return (!cleaned.is_empty() && cleaned.chars().count() <= 28).then_some(cleaned);
    }

    // Click position in upscaled crop coordinates (fallback multi-button strip path)
    let target_x = ((click_pt.0 - raw.origin_x) as f32 - rx0 as f32) * scale as f32;
    let target_y = ((click_pt.1 - raw.origin_y) as f32 - ry0 as f32) * scale as f32;

    // Collect each OCR line with its exact bounding box (in upscaled crop px)
    #[derive(Clone)]
    struct LineBox {
        text: String,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
    }
    let mut line_boxes: Vec<LineBox> = Vec::new();
    if let Ok(lines) = result.Lines() {
        for line in lines {
            let Ok(words) = line.Words() else {
                continue;
            };
            let mut clusters: Vec<LineBox> = Vec::new();
            for word in words {
                let Ok(wtxt) = word.Text() else {
                    continue;
                };
                let wtxt = wtxt.to_string().trim().to_string();
                if wtxt.is_empty() {
                    continue;
                }
                let Ok(br) = word.BoundingRect() else {
                    continue;
                };
                let wx0 = br.X;
                let wy0 = br.Y;
                let wx1 = br.X + br.Width;
                let wy1 = br.Y + br.Height;
                let gap_limit = (br.Height * 0.72).max(12.0);
                if let Some(last) = clusters.last_mut() {
                    if wx0 - last.x1 <= gap_limit {
                        last.text.push_str(&wtxt);
                        last.x1 = last.x1.max(wx1);
                        last.y0 = last.y0.min(wy0);
                        last.y1 = last.y1.max(wy1);
                        continue;
                    }
                }
                clusters.push(LineBox {
                    text: wtxt,
                    x0: wx0,
                    y0: wy0,
                    x1: wx1,
                    y1: wy1,
                });
            }
            line_boxes.extend(clusters);
        }
    }

    if line_boxes.is_empty() {
        let fallback = clean_sw_ocr_label(
            &result.Text().ok()?.to_string().replace(['\r', '\n', ' '], ""),
        );
        return (!fallback.is_empty() && fallback.chars().count() <= 24).then_some(fallback);
    }

    let dist_to_box = |b: &LineBox| -> f32 {
        let dx = if target_x < b.x0 {
            b.x0 - target_x
        } else if target_x > b.x1 {
            target_x - b.x1
        } else {
            0.0
        };
        let dy = if target_y < b.y0 {
            b.y0 - target_y
        } else if target_y > b.y1 {
            target_y - b.y1
        } else {
            0.0
        };
        (dx * 2.0).hypot(dy)
    };

    let best_idx = line_boxes
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            dist_to_box(a)
                .partial_cmp(&dist_to_box(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, _)| i)?;

    let best = line_boxes[best_idx].clone();
    let best_h = (best.y1 - best.y0).max(12.0);
    let best_cx = 0.5 * (best.x0 + best.x1);

    let mut parts: Vec<(f32, String)> = vec![(best.y0, best.text.clone())];
    for (i, other) in line_boxes.iter().enumerate() {
        if i == best_idx {
            continue;
        }
        let other_cx = 0.5 * (other.x0 + other.x1);
        let horiz_aligned = (other_cx - best_cx).abs() <= best_h * 1.3;
        let v_gap = if other.y0 >= best.y1 {
            other.y0 - best.y1
        } else if best.y0 >= other.y1 {
            best.y0 - other.y1
        } else {
            0.0
        };
        if horiz_aligned && v_gap <= best_h * 0.42 {
            parts.push((other.y0, other.text.clone()));
        }
    }
    parts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let merged: String = parts.into_iter().map(|(_, t)| t).collect();
    let cleaned = clean_sw_ocr_label(&merged);
    if !cleaned.is_empty() && cleaned.chars().count() <= 28 {
        Some(cleaned)
    } else {
        None
    }
}

impl SwWin32ControlInfo {
    pub fn display_label(&self) -> String {
        let state_suffix = match self.checked {
            Some(true) => " [已勾选]",
            Some(false) => " [未勾选]",
            None => "",
        };
        if !self.text.is_empty() {
            format!("{}{}", self.text, state_suffix)
        } else if let Some(parent) = self.parent_chain.iter().find(|p| {
            let up = p.trim().to_ascii_uppercase();
            !up.starts_with("SOLIDWORKS") && !is_generic_sw_container_label(p)
        }) {
            format!("{} · {}", parent, self.role_zh())
        } else {
            format!("SolidWorks {}", self.role_zh())
        }
    }

    pub fn role_zh(&self) -> &'static str {
        match self.role.as_str() {
            "Button" => "按钮",
            "CheckBox" => "复选框",
            "RadioButton" => "单选项",
            "GroupBox" => "选项组",
            "Edit" => "输入框",
            "ComboBox" => "下拉框",
            "Tab" => "选项卡",
            "Toolbar" => "工具栏按钮",
            "Menu" => "菜单项",
            "Label" => "选项标签",
            "Spin/Slider" => "数值调节",
            _ => "界面控件",
        }
    }
}

pub fn read_hwnd_text_nonblocking(hwnd: HWND, timeout_ms: u32) -> String {
    const WM_GETTEXT_MSG: u32 = 0x000D;
    let mut buf = [0u16; 256];
    let mut written = 0usize;
    let ok = unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_GETTEXT_MSG,
            WPARAM(buf.len()),
            LPARAM(buf.as_mut_ptr() as isize),
            SMTO_ABORTIFHUNG | SMTO_BLOCK,
            timeout_ms,
            Some(&mut written),
        )
    };
    if ok.0 == 0 || written == 0 {
        return String::new();
    }
    let len = written.min(buf.len() - 1);
    let nul_pos = buf[..len].iter().position(|&c| c == 0).unwrap_or(len);
    let raw = String::from_utf16_lossy(&buf[..nul_pos]);
    strip_win32_mnemonics(&raw)
}

fn strip_win32_mnemonics(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\0' {
            continue;
        }
        if c == '&' {
            if chars.peek() == Some(&'&') {
                out.push('&');
                let _ = chars.next();
            }
            continue;
        }
        out.push(c);
    }
    out.trim().to_string()
}

/// Zero-UIA Win32 inspection of SolidWorks buttons, checkboxes, PropertyManager fields,
/// CommandManager toolbars, menus, and dialogs at screen point `(x, y)`.
/// Returns `None` for the 3D graphics viewport, FeatureManager tree, and root window backdrop.
pub fn inspect_win32_ui_control_at(
    root_hwnd: HWND,
    x: i32,
    y: i32,
    include_parents: bool,
) -> Option<SwWin32ControlInfo> {
    let region = classify_solidworks_region_at(root_hwnd, x, y);
    if matches!(region, SwWindowRegion::Viewport3D) {
        return None;
    }
    // On click (`include_parents == true`), let FeatureTree proceed to L4 COM FeatureTreeClick for
    // full parametric specs; on hover (`include_parents == false`), allow exact row blue-frame boxing!
    if include_parents && matches!(region, SwWindowRegion::FeatureTree) {
        return None;
    }

    let mut target = unsafe { WindowFromPoint(POINT { x, y }) };
    if target.0.is_null() {
        return None;
    }

    // Drill into nested child controls (finds disabled controls, static labels, and widgets inside GroupBox)
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
    let cls_lower = class_name.to_ascii_lowercase();
    let is_popup_menu = is_popup_menu_hwnd(target, root_hwnd, &cls_lower);

    if target == root_hwnd && !is_popup_menu {
        return None;
    }

    let mut rc = RECT::default();
    if unsafe { GetWindowRect(target, &mut rc) }.is_err() {
        return None;
    }
    let w = (rc.right - rc.left).max(0);
    let h = (rc.bottom - rc.top).max(0);
    if w < 4 || h < 4 {
        return None;
    }

    let mut root_rc = RECT::default();
    let (rw, rh) = if unsafe { GetWindowRect(root_hwnd, &mut root_rc) }.is_ok() {
        ((root_rc.right - root_rc.left).max(1), (root_rc.bottom - root_rc.top).max(1))
    } else {
        (1, 1)
    };
    // Guard against accidentally returning an entire window/viewport pane as a UI button
    if !is_popup_menu
        && (cls_lower.starts_with("afxframeorview")
            || cls_lower.contains("mdiclient")
            || cls_lower.contains("opengl")
            || cls_lower.contains("swgraphics"))
        && w * 4 >= rw
        && h * 4 >= rh
    {
        return None;
    }

    let style = unsafe { GetWindowLongW(target, GWL_STYLE) } as u32;
    let ctrl_id = unsafe { GetDlgCtrlID(target) };
    let mut text = read_hwnd_text_nonblocking(target, 120);
    let mut description = String::new();

    let (mut role, checked) = if cls_lower == "button" {
        let bs_type = style & 0x0F;
        match bs_type {
            2 | 3 | 5 | 6 => {
                // BS_CHECKBOX / BS_AUTOCHECKBOX / BS_3STATE / BS_AUTO3STATE
                const BM_GETCHECK_MSG: u32 = 0x00F0;
                let mut state = 0usize;
                let ok = unsafe {
                    SendMessageTimeoutW(
                        target,
                        BM_GETCHECK_MSG,
                        WPARAM(0),
                        LPARAM(0),
                        SMTO_ABORTIFHUNG | SMTO_BLOCK,
                        80,
                        Some(&mut state),
                    )
                };
                let chk = if ok.0 != 0 { Some(state != 0) } else { None };
                ("CheckBox".to_string(), chk)
            }
            4 | 9 => {
                // BS_RADIOBUTTON / BS_AUTORADIOBUTTON
                const BM_GETCHECK_MSG: u32 = 0x00F0;
                let mut state = 0usize;
                let ok = unsafe {
                    SendMessageTimeoutW(
                        target,
                        BM_GETCHECK_MSG,
                        WPARAM(0),
                        LPARAM(0),
                        SMTO_ABORTIFHUNG | SMTO_BLOCK,
                        80,
                        Some(&mut state),
                    )
                };
                let chk = if ok.0 != 0 { Some(state != 0) } else { None };
                ("RadioButton".to_string(), chk)
            }
            7 => ("GroupBox".to_string(), None),
            _ => ("Button".to_string(), None),
        }
    } else if cls_lower == "edit" || cls_lower.contains("richedit") {
        ("Edit".to_string(), None)
    } else if cls_lower == "combobox" {
        ("ComboBox".to_string(), None)
    } else if cls_lower == "static" {
        ("Label".to_string(), None)
    } else if cls_lower == "systabcontrol32" {
        ("Tab".to_string(), None)
    } else if cls_lower == "#32768" {
        ("Menu".to_string(), None)
    } else if cls_lower.contains("toolbarwindow32")
        || cls_lower.contains("afxcontrolbar")
        || cls_lower.contains("bcgp")
        || cls_lower.contains("netuihwnd")
        || cls_lower.contains("rebarwindow32")
    {
        ("Toolbar".to_string(), None)
    } else if cls_lower.contains("trackbar") || cls_lower.contains("updown") {
        ("Spin/Slider".to_string(), None)
    } else {
        ("Control".to_string(), None)
    };

    // Composite owner-drawn toolbars / ribbons / popup menus / large PropertyManager containers
    // don't expose individual icon HWNDs; anchor a crisp OS-icon-sized reticle at `(x, y)` inside them.
    let is_discrete = matches!(
        role.as_str(),
        "Button" | "CheckBox" | "RadioButton" | "Edit" | "ComboBox" | "Label" | "Spin/Slider"
    ) && w <= 420
        && h <= 140;

    let (rect, is_composite) = if is_discrete {
        (
            ScreenRect {
                x: rc.left,
                y: rc.top,
                w,
                h,
            },
            false,
        )
    } else {
        let container_srect = ScreenRect {
            x: rc.left,
            y: rc.top,
            w,
            h,
        };
        // Follow the active profile's layer order (e.g. StaticVisualHitMap first after self-heal or builtin)
        let sw_prof = crate::pick::profile::get_effective_profile("sldworks.exe");
        let forced_layout = sw_prof
            .learned_regions
            .iter()
            .find(|r| r.class_name.eq_ignore_ascii_case(&cls_lower))
            .map(|r| crate::pick::hitmap::ContainerLayoutKind::parse(&r.layout_kind))
            .unwrap_or(crate::pick::hitmap::ContainerLayoutKind::Auto);
        // Always check already-woken ExternalBoxProvider (OmniParser / Hover-Diff) first so Dose 1
        // results are NEVER shadowed by coarse MsaaHitMap layers!
        let mut hit_entry: Option<crate::pick::hitmap::HitMapEntry> =
            crate::pick::hitmap::lookup_cached_any_source(target.0 as isize, x, y);
        if hit_entry.is_none() {
            for layer in &sw_prof.preferred_layers {
                match layer {
                    crate::pick::profile::GroundingLayer::MsaaHitMap => {
                        if let crate::pick::hitmap::MsaaLookup::Hit(entry) =
                            crate::pick::hitmap::lookup_or_build_msaa(target, container_srect, x, y)
                        {
                            hit_entry = Some(entry);
                            break;
                        }
                    }
                    crate::pick::profile::GroundingLayer::StaticVisualHitMap => {
                        if let Some(entry) =
                            crate::pick::hitmap::lookup_or_build_static_visual_with_hint(
                                target,
                                container_srect,
                                &class_name,
                                is_popup_menu,
                                forced_layout,
                                x,
                                y,
                            )
                        {
                            hit_entry = Some(entry);
                            break;
                        }
                    }
                    crate::pick::profile::GroundingLayer::Uia => {}
                }
            }
        }
        if let Some(entry) = hit_entry {
            if !entry.label.is_empty() && !is_generic_sw_container_label(&entry.label) {
                text = entry.label;
            }
            if description.is_empty() && !entry.description.is_empty() {
                description = entry.description;
            }
            if role == "Control" || role == "Toolbar" {
                role = entry.role;
            }
            (entry.rect, false)
        } else if let Some(exact_rc) =
            detect_sw_toolbar_hovered_button_rect(target.0 as isize, rc, x, y)
        {
            // Final visual fallback for ultra-low-contrast controls that only appear on hover
            if role == "Control" || role == "Toolbar" {
                role = "Button".to_string();
            }
            (exact_rc, false)
        } else if !include_parents {
            // During hover, NEVER show a floating cursor-centered fixed-size square over empty toolbar
            // whitespace (eliminates both the fixed-size blue box and grid-snapping cursor feel!).
            return None;
        } else {
            // On explicit click over an unhighlighted/disabled composite toolbar spot, keep a clean
            // icon-sized fallback rect so the pick still commits gracefully.
            let icon_w = unsafe { GetSystemMetrics(SM_CXICON) }.max(32);
            let icon_h = unsafe { GetSystemMetrics(SM_CYICON) }.max(32);
            let bw = (icon_w * 3 / 2).min(w).max(24);
            let bh = (icon_h * 3 / 2).min(h).max(24);
            let rx = (x - bw / 2).clamp(rc.left, (rc.right - bw).max(rc.left));
            let ry = (y - bh / 2).clamp(rc.top, (rc.bottom - bh).max(rc.top));
            (
                ScreenRect {
                    x: rx,
                    y: ry,
                    w: bw,
                    h: bh,
                },
                true,
            )
        }
    };

    if include_parents && description.is_empty() {
        description = read_solidworks_status_bar_text(root_hwnd);
    }

    if is_generic_sw_container_label(&text) {
        text.clear();
    }

    let mut parent_chain = Vec::new();
    if include_parents {
        let mut cur = target;
        for _ in 0..5 {
            let Ok(p) = (unsafe { GetParent(cur) }) else {
                break;
            };
            if p.0.is_null() || p == cur {
                break;
            }
            let ptxt = read_hwnd_text_nonblocking(p, 80);
            if !ptxt.is_empty() && !parent_chain.iter().any(|existing| existing == &ptxt) {
                parent_chain.push(ptxt);
            }
            if p == root_hwnd {
                break;
            }
            cur = p;
        }
    }

    Some(SwWin32ControlInfo {
        hwnd_raw: target.0 as isize,
        class_name,
        role,
        text,
        description,
        checked,
        ctrl_id,
        rect,
        is_composite,
        parent_chain,
    })
}

/// Parses a single SolidWorks `IEquationMgr.Equation[k]` line into `(lhs_dim_name, rhs_expr)`.
///
/// Examples:
/// - `"D1@齿根圆柱" = "b"` -> `Some(("D1@齿根圆柱", "\"b\""))`
/// - `"D1@Boss-Extrude1" = "thickness" / 2 ' half thickness` -> `Some(("D1@Boss-Extrude1", "\"thickness\" / 2"))`
/// - Global variable `"b" = 15` (no `@`) -> `None` (not a dimension-driving equation)
pub fn parse_equation_entry(raw: &str) -> Option<(String, String)> {
    // Strip trailing comment (`'`) outside double quotes
    let mut in_quotes = false;
    let mut comment_idx = None;
    for (idx, ch) in raw.char_indices() {
        if ch == '"' {
            in_quotes = !in_quotes;
        } else if ch == '\'' && !in_quotes {
            comment_idx = Some(idx);
            break;
        }
    }
    let body = match comment_idx {
        Some(i) => raw[..i].trim(),
        None => raw.trim(),
    };
    if body.is_empty() {
        return None;
    }

    // Split at first `=` outside double quotes
    in_quotes = false;
    let mut eq_idx = None;
    for (idx, ch) in body.char_indices() {
        if ch == '"' {
            in_quotes = !in_quotes;
        } else if ch == '=' && !in_quotes {
            eq_idx = Some(idx);
            break;
        }
    }
    let eq_idx = eq_idx?;
    let lhs_raw = body[..eq_idx].trim();
    let rhs_raw = body[eq_idx + 1..].trim();
    if lhs_raw.is_empty() || rhs_raw.is_empty() {
        return None;
    }
    let lhs = lhs_raw.trim_matches('"').trim();
    // Only dimension equations carry `@` (e.g. `D1@Extrude1`); skip pure global variable defs
    if !lhs.contains('@') {
        return None;
    }
    Some((lhs.to_string(), rhs_raw.to_string()))
}

/// Maps a unit normal vector `(nx, ny, nz)` to a cardinal direction selector (`+z`, `-z`, etc.)
/// when aligned within ~5 degrees (`cos(5°) ≈ 0.996`).
pub fn classify_semantic_direction(normal: [f64; 3]) -> Option<&'static str> {
    let len = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
    if len < 1e-6 {
        return None;
    }
    let nx = normal[0] / len;
    let ny = normal[1] / len;
    let nz = normal[2] / len;
    const COS_5_DEG: f64 = 0.996194698;
    if nz >= COS_5_DEG {
        Some("+z")
    } else if nz <= -COS_5_DEG {
        Some("-z")
    } else if nx >= COS_5_DEG {
        Some("+x")
    } else if nx <= -COS_5_DEG {
        Some("-x")
    } else if ny >= COS_5_DEG {
        Some("+y")
    } else if ny <= -COS_5_DEG {
        Some("-y")
    } else {
        None
    }
}

/// Pure mathematical inversion of SolidWorks `IModelView.Transform.ArrayData` (16 doubles)
/// from viewport client physical pixel `(cx, cy)` to a 3D model-space ray `(origin_m, dir_unit)`.
///
/// SolidWorks `IMathTransform.ArrayData` layout:
/// - `[0..9]`: 3×3 orthogonal rotation matrix `R` (row-major: `v_view = s * (v_model * R) + t`)
/// - `[9..12]`: translation `(tx, ty, tz)` in viewport pixels
/// - `[12]`: uniform scale `s` (pixels per meter)
///
/// When `visible_box_m` (`IModelView::GetVisibleBox()`) is provided, the ray origin is derived
/// geometrically from the visible bounding sphere so it works across all model scales (0.5mm to
/// 50m+) with zero arbitrary depth constants.
pub fn invert_model_view_transform_to_ray(
    arr: &[f64],
    cx: f64,
    cy: f64,
    visible_box_m: Option<[f64; 6]>,
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

    // Viewport pixel axes in right-handed 3D model space:
    //   `e_right = (arr[0], arr[3], arr[6])` (viewport +X right)
    //   `e_down  = (arr[1], arr[4], arr[7])` (viewport +Y down, Windows client coordinates)
    // By the right-hand rule in model space, `e_right × e_down` ALWAYS points INTO the screen
    // (from camera toward model), regardless of whether `det(R) = +1` (proper rotation) or
    // `det(R) = -1` (Y-flipped reflection), across Front/Back/Top/Right/Iso and arbitrary 3D views.
    let xr = [arr[0], arr[3], arr[6]];
    let yd = [arr[1], arr[4], arr[7]];
    let dx = xr[1] * yd[2] - xr[2] * yd[1];
    let dy = xr[2] * yd[0] - xr[0] * yd[2];
    let dz = xr[0] * yd[1] - xr[1] * yd[0];
    let dlen = (dx * dx + dy * dy + dz * dz).sqrt();
    if dlen < 1e-9 {
        return None;
    }
    let dir = [dx / dlen, dy / dlen, dz / dlen];

    // Base point on the viewport z=0 plane in model coordinates
    let ux = (cx - tx) / s;
    let uy = (cy - ty) / s;
    let uz_base = -tz / s;
    let base_pt = [
        ux * arr[0] + uy * arr[1] + uz_base * arr[2],
        ux * arr[3] + uy * arr[4] + uz_base * arr[5],
        ux * arr[6] + uy * arr[7] + uz_base * arr[8],
    ];

    // Shift origin backwards along `-dir` so it starts strictly in front of all visible geometry
    // (on the camera side, shooting INTO the screen so `SelectByRay` hits the nearest front face first).
    let advance_front = if let Some(b) = visible_box_m {
        let mid = [
            0.5 * (b[0] + b[3]),
            0.5 * (b[1] + b[4]),
            0.5 * (b[2] + b[5]),
        ];
        let half_diag = 0.5
            * ((b[3] - b[0]).powi(2) + (b[4] - b[1]).powi(2) + (b[5] - b[2]).powi(2)).sqrt();
        // Signed projection of (mid - base_pt) along `dir`
        let proj = (mid[0] - base_pt[0]) * dir[0]
            + (mid[1] - base_pt[1]) * dir[1]
            + (mid[2] - base_pt[2]) * dir[2];
        // Ensure `origin = base_pt - advance_front * dir` lies strictly in front of the bounding sphere
        let desired_back = (half_diag * 2.0).max(1.0 / s.abs()) - proj;
        desired_back.max(0.0)
    } else {
        // Fallback when visible box is unavailable: 2× viewport reciprocal scale (at least 1 meter)
        (2.0 / s.abs()).max(1.0)
    };

    let orig = [
        base_pt[0] - advance_front * dir[0],
        base_pt[1] - advance_front * dir[1],
        base_pt[2] - advance_front * dir[2],
    ];
    Some((orig, dir))
}

/// Calls the native Rust COM probe strictly at medium integrity via Broker.
pub fn pick_solidworks_v2(
    broker: &BrokerClient,
    screen_pos: Option<(i32, i32)>,
    pick_mode: SwPickMode,
) -> SwProbeOutcome {
    let timeout_ms = match pick_mode {
        SwPickMode::CurrentSelectionOnly => SPEC_HARD_TIMEOUT_MS,
        SwPickMode::ViewportRayOnly | SwPickMode::FeatureTreeClick | SwPickMode::Auto => {
            SPEC_INTERACTIVE_TIMEOUT_MS
        }
    };
    let resp = broker.call(BrokerRequest::SolidWorksPick {
        screen_x: screen_pos.map(|p| p.0),
        screen_y: screen_pos.map(|p| p.1),
        allow_viewport_ray: matches!(pick_mode, SwPickMode::ViewportRayOnly | SwPickMode::Auto),
        pick_mode: pick_mode.as_str().to_string(),
        timeout_ms,
    });
    if !resp.ok || resp.text.trim().is_empty() {
        return SwProbeOutcome::Unavailable;
    }
    serde_json::from_str::<SwProbeOutcome>(resp.text.trim()).unwrap_or(SwProbeOutcome::Unavailable)
}

/// Builds a concise, human-friendly blue capsule title & subtitle for `SwTargetV2Payload`.
pub fn build_solidworks_chip_title(badge: &str, sw: &SwTargetV2Payload) -> (String, String) {
    let doc = &sw.doc_context.doc;
    let doc_badge = if !doc.path.is_empty() {
        Path::new(&doc.path)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| doc.title.clone())
    } else {
        doc.title.clone()
    };

    if let Some(first) = sw.targets.first() {
        let core = format_single_target_chip_core(first);
        let multi_suffix = if sw.targets.len() > 1 {
            format!(" 等{}项", sw.targets.len())
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
            format!("{badge} SolidWorks 文档")
        } else {
            format!("{badge} {doc_badge}")
        };
        let detail = if doc.doc_type.is_empty() {
            "SolidWorks".to_string()
        } else {
            format!("SolidWorks · {}", doc.doc_type)
        };
        (title, detail)
    }
}

fn format_single_target_chip_core(item: &SwTargetV2Item) -> String {
    let d = &item.details;
    match item.family {
        SwTargetFamily::Topology => {
            let entity_name = d.get("entity_name").and_then(Value::as_str).unwrap_or("");
            let owner = d.get("owner_feature").and_then(Value::as_str).unwrap_or("");
            let body = d.get("owner_body").and_then(Value::as_str).unwrap_or("");
            let base = if !owner.is_empty() {
                owner
            } else if !body.is_empty() {
                body
            } else if !entity_name.is_empty() {
                entity_name
            } else {
                item.select_type_name.as_str()
            };
            let kind_zh = match item.select_type_id {
                2 => "面",
                1 => "边",
                3 => "顶点",
                24 => "环",
                _ => item.select_type_name.as_str(),
            };
            let dir = d
                .get("dir")
                .and_then(Value::as_str)
                .map(|s| format!(" [{s}]"))
                .unwrap_or_default();
            let radius = d
                .get("radius_mm")
                .and_then(Value::as_f64)
                .map(|r| format!(" (R={r}mm)"))
                .unwrap_or_default();
            let tag = if !entity_name.is_empty() && entity_name != base {
                format!(" ({entity_name})")
            } else {
                radius
            };
            format!("{base} · {kind_zh}{dir}{tag}")
        }
        SwTargetFamily::Dimension => item.display_name.clone(),
        SwTargetFamily::Assembly => {
            if item.select_type_id == 20 {
                format!("{} (零件)", item.display_name)
            } else {
                format!("{} (配合)", item.display_name)
            }
        }
        SwTargetFamily::Body => {
            let bt = d.get("body_type").and_then(Value::as_str).unwrap_or("实体");
            format!("{} ({bt})", item.display_name)
        }
        _ => {
            if !item.display_name.is_empty() {
                item.display_name.clone()
            } else {
                item.select_type_name.clone()
            }
        }
    }
}

/// Formats the tool-agnostic two-part `[SW-TARGET v2]` capsule lines:
/// - Part 1: Concise human-readable summary lines
/// - Part 2: `[SW-TARGET v2] <compact single-line UTF-8 JSON>`
///
/// Strictly contains ZERO hardcoded MCP tool names (P0-5 / V5).
pub fn format_solidworks_capsule_lines(sw: &SwTargetV2Payload) -> Vec<String> {
    let mut lines = Vec::new();
    let doc = &sw.doc_context.doc;
    let edit = &sw.doc_context.edit_state;
    let summary = &sw.doc_context.model_summary;

    let doc_label = if !doc.title.is_empty() {
        doc.title.as_str()
    } else {
        doc.path.as_str()
    };

    let mut header = format!(
        "SolidWorks 目标: {doc_label} ({}, 配置=\"{}\", 模式={}, 选中={}项)",
        doc.doc_type,
        doc.active_config,
        sw.pick_mode,
        sw.targets.len()
    );
    if edit.in_sketch_edit {
        header.push_str(&format!(" [草图编辑态: {}]", edit.active_sketch_name));
    } else if edit.in_component_edit {
        header.push_str(&format!(" [在位编辑零件: {}]", edit.editing_component));
    }
    if summary.error_feature_count > 0 {
        header.push_str(&format!(" [报错特征: {}个]", summary.error_feature_count));
    }
    lines.push(header);

    for t in &sw.targets {
        lines.push(format_human_target_line(t));
    }

    if let Ok(compact_json) = serde_json::to_string(sw) {
        lines.push(format!("[SW-TARGET v2] {compact_json}"));
    }

    lines
}

fn format_human_target_line(t: &SwTargetV2Item) -> String {
    let mut parts = vec![format!("- #{} {}", t.index, t.select_type_name)];
    let d = &t.details;

    if !t.component_chain.is_empty() {
        parts.push(format!("装配路径={}", t.component_chain.join("/")));
    }
    if let Some(of) = d.get("owner_feature").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        let ft = d.get("feature_type").and_then(Value::as_str).unwrap_or("");
        if ft.is_empty() {
            parts.push(format!("所属特征=\"{of}\""));
        } else {
            parts.push(format!("所属特征=\"{of}\"({ft})"));
        }
    } else if let Some(fnm) = d.get("feature_name").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        let ft = d.get("feature_type").and_then(Value::as_str).unwrap_or("");
        if ft.is_empty() {
            parts.push(format!("特征=\"{fnm}\""));
        } else {
            parts.push(format!("特征=\"{fnm}\"({ft})"));
        }
    } else if !t.display_name.is_empty() && t.display_name != t.select_type_name {
        parts.push(format!("名称=\"{}\"", t.display_name));
    }

    if let Some(en) = d.get("entity_name").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        parts.push(format!("实体名=\"{en}\""));
    }

    // Geometry concise summary
    let mut geom_bits = Vec::new();
    if let Some(s) = d.get("surface").and_then(Value::as_str) {
        geom_bits.push(s.to_string());
    }
    if let Some(c) = d.get("curve").and_then(Value::as_str) {
        geom_bits.push(c.to_string());
    }
    if let Some(st) = d.get("segment_type").and_then(Value::as_str) {
        geom_bits.push(st.to_string());
    }
    if let Some(r) = d.get("radius_mm").and_then(Value::as_f64) {
        geom_bits.push(format!("R={r}mm"));
    }
    if let Some(a) = d.get("area_mm2").and_then(Value::as_f64) {
        geom_bits.push(format!("area={a}mm2"));
    }
    if let Some(l) = d.get("length_mm").and_then(Value::as_f64) {
        geom_bits.push(format!("len={l}mm"));
    }
    if !geom_bits.is_empty() {
        parts.push(format!("几何=\"{}\"", geom_bits.join(", ")));
    }

    if let Some(dir) = d.get("dir").and_then(Value::as_str) {
        parts.push(format!("法向={dir}"));
    }
    if let Some(pt) = t.point_mm {
        parts.push(format!("点=({:.1},{:.1},{:.1})mm", pt[0], pt[1], pt[2]));
    }

    if let Some(dims) = d.get("dimensions").and_then(Value::as_array) {
        if !dims.is_empty() {
            let dim_summary: Vec<String> = dims
                .iter()
                .take(4)
                .filter_map(|dv| {
                    let fnm = dv.get("full_name").and_then(Value::as_str)?;
                    let val = dv.get("value").and_then(Value::as_f64)?;
                    let unit = dv.get("unit").and_then(Value::as_str).unwrap_or("mm");
                    let eq = dv
                        .get("equation_rhs")
                        .and_then(Value::as_str)
                        .map(|rhs| format!(" (方程式={rhs})"))
                        .unwrap_or_default();
                    Some(format!("{fnm}={val} {unit}{eq}"))
                })
                .collect();
            if !dim_summary.is_empty() {
                parts.push(format!("关联尺寸: {}", dim_summary.join("; ")));
            }
        }
    }

    parts.join(" | ")
}

/// Deep-adapter enrichment entry point (used for stroke/box picks in SolidWorks).
pub fn run(ctx: &AdapterContext) -> Option<AdapterOutput> {
    if !matches(ctx) {
        return None;
    }
    let mut out = AdapterOutput::new("solidworks");
    match pick_solidworks_v2(&ctx.broker, ctx.tap_point, SwPickMode::CurrentSelectionOnly) {
        SwProbeOutcome::Ok(sw) => {
            if !sw.doc_context.doc.path.is_empty() && Path::new(&sw.doc_context.doc.path).exists() {
                out.paths.push(sw.doc_context.doc.path.clone());
            }
            if let Some(first) = sw.targets.first() {
                if !first.display_name.is_empty() {
                    out.badge = Some(first.display_name.clone());
                }
            }
            if out.badge.is_none() && !sw.doc_context.doc.title.is_empty() {
                out.badge = Some(sw.doc_context.doc.title.clone());
            }
            out.lines.extend(format_solidworks_capsule_lines(&sw));
            return Some(out);
        }
        SwProbeOutcome::Busy => {
            out.lines
                .push("solidworks_status: SolidWorks 正忙，请稍候再抓".to_string());
            return Some(out);
        }
        SwProbeOutcome::TimedOut | SwProbeOutcome::Unavailable => {}
    }

    out.lines
        .push(format!("cad_window: {}", ctx.window_title.trim()));
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn maps_all_eight_families_and_unknown_type_falls_back_to_generic_object() {
        assert_eq!(sw_select_type_meta(2), ("FACE", SwTargetFamily::Topology));
        assert_eq!(sw_select_type_meta(1), ("EDGE", SwTargetFamily::Topology));
        assert_eq!(sw_select_type_meta(10), ("SKETCHSEGMENT", SwTargetFamily::SketchEntity));
        assert_eq!(sw_select_type_meta(96), ("SKETCHREGION", SwTargetFamily::SketchEntity));
        assert_eq!(sw_select_type_meta(22), ("BODYFEATURE", SwTargetFamily::FeatureOrRef));
        assert_eq!(sw_select_type_meta(4), ("PLANE", SwTargetFamily::FeatureOrRef));
        assert_eq!(sw_select_type_meta(76), ("SOLIDBODY", SwTargetFamily::Body));
        assert_eq!(sw_select_type_meta(75), ("SURFACEBODY", SwTargetFamily::Body));
        assert_eq!(sw_select_type_meta(14), ("DIMENSION", SwTargetFamily::Dimension));
        assert_eq!(sw_select_type_meta(20), ("COMPONENT", SwTargetFamily::Assembly));
        assert_eq!(sw_select_type_meta(34), ("MATE", SwTargetFamily::Assembly));
        assert_eq!(sw_select_type_meta(12), ("DRAWINGVIEW", SwTargetFamily::DrawingAnnotation));
        assert_eq!(sw_select_type_meta(13), ("GTOL", SwTargetFamily::DrawingAnnotation));
        // Obscure / future SolidWorks type ID never panics and falls back to generic_object
        assert_eq!(sw_select_type_meta(9999), ("UNKNOWN", SwTargetFamily::GenericObject));
    }

    #[test]
    fn parses_general_solidworks_equations_without_naming_assumptions() {
        assert_eq!(
            parse_equation_entry(r#""D1@齿根圆柱" = "b""#),
            Some(("D1@齿根圆柱".to_string(), r#""b""#.to_string()))
        );
        assert_eq!(
            parse_equation_entry(r#""D2@Boss-Extrude1" = "thickness" / 2 + 1.5 ' half wall"#),
            Some((
                "D2@Boss-Extrude1".to_string(),
                r#""thickness" / 2 + 1.5"#.to_string()
            ))
        );
        // Pure global variable definitions (no `@`) are skipped cleanly
        assert_eq!(parse_equation_entry(r#""thickness" = 12"#), None);
        assert_eq!(parse_equation_entry("' comment only"), None);
        assert_eq!(parse_equation_entry("   "), None);
    }

    #[test]
    fn classifies_cardinal_normals_accurately() {
        assert_eq!(classify_semantic_direction([0.0, 0.0, 1.0]), Some("+z"));
        assert_eq!(classify_semantic_direction([0.0, 0.0, -0.999]), Some("-z"));
        assert_eq!(classify_semantic_direction([1.0, 0.0, 0.0]), Some("+x"));
        assert_eq!(classify_semantic_direction([0.5, 0.5, 0.707]), None);
    }

    #[test]
    fn inverts_model_view_transform_with_visible_box_across_scales() {
        // Real SolidWorks Front View (Y-down client pixels, proper right-handed rotation det(R)=+1):
        //   +X_model -> +X_screen (right), +Y_model -> -Y_screen (up), +Z_model -> -Z_screen (out of screen)
        //   So col_0 = (1,0,0), col_1 = (0,-1,0), col_2 = (0,0,-1)
        let arr_front_det_pos = [
            1.0, 0.0, 0.0, //
            0.0, -1.0, 0.0, //
            0.0, 0.0, -1.0, //
            600.0, 450.0, 0.0, //
            1500.0, 0.0, 0.0, 0.0,
        ];
        // Small part: 100mm cube centered at origin
        let vis_small = [-0.05, -0.05, -0.05, 0.05, 0.05, 0.05];
        let (orig, dir) =
            invert_model_view_transform_to_ray(&arr_front_det_pos, 675.0, 480.0, Some(vis_small))
                .unwrap();
        assert!((orig[0] - 0.05).abs() < 1e-6);
        assert!((orig[1] - (-0.02)).abs() < 1e-6, "screen +Y down maps to model -Y");
        assert!(orig[2] > 0.05, "ray origin must start strictly in front of +Z face");
        assert!((dir[2] + 1.0).abs() < 1e-6, "ray must shoot INTO screen (-Z) to hit front +Z face first");

        // Also invariant if a driver/transform uses det(R)=-1 reflection (col_2 = +Z):
        let arr_front_det_neg = [
            1.0, 0.0, 0.0, //
            0.0, -1.0, 0.0, //
            0.0, 0.0, 1.0, //
            600.0, 450.0, 0.0, //
            1500.0, 0.0, 0.0, 0.0,
        ];
        let (orig_n, dir_n) =
            invert_model_view_transform_to_ray(&arr_front_det_neg, 675.0, 480.0, Some(vis_small))
                .unwrap();
        assert!(orig_n[2] > 0.05);
        assert!((dir_n[2] + 1.0).abs() < 1e-6);

        // Top View (+X right, +Z down, looking from +Y toward -Y)
        let arr_top = [
            1.0, 0.0, 0.0, //
            0.0, 0.0, -1.0, //
            0.0, 1.0, 0.0, //
            600.0, 450.0, 0.0, //
            1500.0, 0.0, 0.0, 0.0,
        ];
        let (orig_t, dir_t) =
            invert_model_view_transform_to_ray(&arr_top, 600.0, 450.0, Some(vis_small)).unwrap();
        assert!(orig_t[1] > 0.05, "top view ray must start above +Y face");
        assert!((dir_t[1] + 1.0).abs() < 1e-6, "top view ray must shoot toward -Y");

        // Giant assembly: 40m ship hull
        let vis_huge = [-20.0, -10.0, -5.0, 20.0, 10.0, 15.0];
        let (orig_h, dir_h) =
            invert_model_view_transform_to_ray(&arr_front_det_pos, 600.0, 450.0, Some(vis_huge))
                .unwrap();
        assert!(orig_h[2] > 15.0, "ray origin must adapt to 40m hull without magic Z cutoff");
        assert!((dir_h[2] + 1.0).abs() < 1e-6);
    }

    #[test]
    fn adaptive_profiler_degrades_only_measured_slow_stage_and_resets_on_doc_change() {
        let mut p = SwAdaptiveProfiler::new(100);
        let f0 = p.begin_doc("huge_asm.SLDASM");
        assert!(!f0.skip_bbox && !f0.skip_custom_props);

        // Two slow bbox picks: not yet degraded (needs 3 consecutive confirmations)
        p.record_sample("huge_asm.SLDASM", 140, 85, 4, 0, 2);
        p.record_sample("huge_asm.SLDASM", 150, 90, 5, 0, 2);
        assert!(!p.begin_doc("huge_asm.SLDASM").skip_bbox);

        // Third consecutive slow bbox pick breaches SLA -> defers ONLY bbox, keeps props/err
        p.record_sample("huge_asm.SLDASM", 145, 88, 5, 0, 2);
        let f3 = p.begin_doc("huge_asm.SLDASM");
        assert!(f3.skip_bbox);
        assert!(!f3.skip_custom_props);
        assert!(!f3.skip_whats_wrong);

        // Switching to a different document resets calibration cleanly
        let f_new = p.begin_doc("bracket.SLDPRT");
        assert!(!f_new.skip_bbox);
    }

    #[test]
    fn formats_sw_target_v2_across_all_scenarios_with_zero_mcp_tool_names() {
        let sw = SwTargetV2Payload {
            schema: SW_SCHEMA_V2.to_string(),
            pick_mode: "current_selection".to_string(),
            doc_context: SwDocContext {
                doc: SwDocInfo {
                    title: "assembly_top.SLDASM".to_string(),
                    path: r"D:\CAD\assembly_top.SLDASM".to_string(),
                    doc_type: "Assembly".to_string(),
                    active_config: "默认".to_string(),
                    config_names: vec!["默认".to_string()],
                    is_dirty: false,
                },
                edit_state: SwEditState {
                    in_sketch_edit: false,
                    active_sketch_name: String::new(),
                    in_component_edit: false,
                    editing_component: String::new(),
                },
                units: SwUnits {
                    length: "mm".to_string(),
                    angle: "deg".to_string(),
                },
                model_summary: SwModelSummary {
                    solid_body_count: 1,
                    surface_body_count: 0,
                    bbox_mm: Some([-50.0, -30.0, 0.0, 50.0, 30.0, 20.0]),
                    equation_count: 2,
                    error_feature_count: 0,
                    error_features: vec![],
                },
                custom_properties: BTreeMap::from([("材料".to_string(), "45钢".to_string())]),
            },
            targets: vec![
                SwTargetV2Item {
                    index: 1,
                    select_type_id: 2,
                    select_type_name: "FACE".to_string(),
                    family: SwTargetFamily::Topology,
                    persist_ref_b64: Some("AQAAtest==".to_string()),
                    mark: 0,
                    display_name: "gear_front_face".to_string(),
                    point_mm: Some([14.0, 0.0, 15.0]),
                    component_chain: vec!["sub_asm-1".to_string(), "housing-2".to_string()],
                    details: json!({
                        "entity_name": "gear_front_face",
                        "owner_feature": "凸台-拉伸1",
                        "owner_body": "凸台-拉伸1",
                        "surface": "Plane",
                        "normal": [0.0, 0.0, 1.0],
                        "dir": "+z",
                        "area_mm2": 842.3,
                        "dimensions": [
                            { "full_name": "D1@凸台-拉伸1", "value": 15.0, "unit": "mm", "equation_rhs": "\"b\"" }
                        ]
                    }),
                },
                SwTargetV2Item {
                    index: 2,
                    select_type_id: 9999,
                    select_type_name: "UNKNOWN".to_string(),
                    family: SwTargetFamily::GenericObject,
                    persist_ref_b64: Some("BQBB==".to_string()),
                    mark: 0,
                    display_name: "CustomObj".to_string(),
                    point_mm: None,
                    component_chain: vec![],
                    details: json!({ "com_type_hint": "IFutureSwObj" }),
                },
            ],
        };

        let (title, detail) = build_solidworks_chip_title("①", &sw);
        assert_eq!(title, "① 凸台-拉伸1 · 面 [+z] (gear_front_face) 等2项");
        assert_eq!(detail, "SolidWorks · assembly_top.SLDASM");

        let lines = format_solidworks_capsule_lines(&sw);
        let joined = lines.join("\n");

        // Strictly zero hardcoded v1 MCP tool names (V5)
        for forbidden in [
            "solidworks_feature_info",
            "solidworks_feature_readback",
            "solidworks_resolve_selection",
            "solidworks_selection_count",
            "solidworks_feature_whats_wrong",
            "Zegel-SWE MCP",
        ] {
            assert!(
                !joined.contains(forbidden),
                "capsule must not contain hardcoded tool hint: {forbidden}"
            );
        }

        // Single-line `[SW-TARGET v2]` JSON must parse cleanly via serde_json
        let json_line = lines
            .iter()
            .find_map(|l| l.strip_prefix("[SW-TARGET v2] "))
            .expect("missing [SW-TARGET v2] JSON line");
        assert!(!json_line.contains('\n'));
        let parsed: SwTargetV2Payload = serde_json::from_str(json_line).expect("valid v2 JSON");
        assert_eq!(parsed.schema, "sw-target/v2");
        assert_eq!(parsed.targets.len(), 2);
        assert_eq!(parsed.targets[0].component_chain, vec!["sub_asm-1", "housing-2"]);
        assert_eq!(parsed.targets[0].persist_ref_b64.as_deref(), Some("AQAAtest=="));
        assert_eq!(parsed.targets[1].family, SwTargetFamily::GenericObject);
    }

    #[test]
    #[ignore]
    fn live_ocr_on_captured_sw_ribbon_buttons() {
        // Use unannotated regions from live SolidWorks 2026 captures with exact button bounding boxes!
        let cases = [
            // Small horizontal buttons (from pick-13 where red box is on 线性阵列, so 扫描/放样/边界切除 are clean!)
            (
                r"C:\Users\Zegel\AppData\Local\AnythingToPrompt\captures\pick-1795701936005-13.png",
                ScreenRect { x: 438, y: 37, w: 94, h: 24 },
                (485, 49),
                true,
                "扫描切除",
            ),
            (
                r"C:\Users\Zegel\AppData\Local\AnythingToPrompt\captures\pick-1795701936005-13.png",
                ScreenRect { x: 438, y: 63, w: 94, h: 24 },
                (485, 75),
                true,
                "放样切除",
            ),
            (
                r"C:\Users\Zegel\AppData\Local\AnythingToPrompt\captures\pick-1795701936005-13.png",
                ScreenRect { x: 438, y: 89, w: 94, h: 24 },
                (485, 101),
                true,
                "边界切除",
            ),
            // Large vertical buttons (from pick-13 where 异型孔向导 is clean, and pick-18 where 线性阵列 is clean!)
            (
                r"C:\Users\Zegel\AppData\Local\AnythingToPrompt\captures\pick-1795701936005-13.png",
                ScreenRect { x: 342, y: 37, w: 48, h: 74 },
                (366, 70),
                true,
                "异型孔向导",
            ),
            (
                r"C:\Users\Zegel\AppData\Local\AnythingToPrompt\captures\pick-1795701946415-18.png",
                ScreenRect { x: 562, y: 37, w: 50, h: 74 },
                (587, 70),
                true,
                "线性阵列",
            ),
        ];
        for (path, rect, pt, exact, expected) in cases {
            if let Ok(dyn_img) = image::open(path) {
                let raw = crate::captures::RawScreenCapture {
                    origin_x: 0,
                    origin_y: 0,
                    image: dyn_img.to_rgba8(),
                };
                let got = ocr_crop_from_raw_capture(&raw, rect, pt, exact);
                eprintln!("[LIVE_SW_OCR] expected={expected} got={got:?}");
            }
        }
    }

    #[test]
    fn target_entity_key_ignores_point_mm_drift_for_same_face() {
        let face_a = SwTargetV2Item {
            index: 1,
            select_type_id: 2,
            select_type_name: "swSelFACES".to_string(),
            family: SwTargetFamily::Topology,
            persist_ref_b64: Some("AQAAfaceCenterHole==".to_string()),
            mark: 0,
            display_name: "中心通孔".to_string(),
            point_mm: Some([6.0, 0.0, 7.12]),
            component_chain: vec![],
            details: json!({}),
        };
        let mut face_a_shifted = face_a.clone();
        face_a_shifted.point_mm = Some([5.98, 0.45, 7.89]);

        let mut face_b = face_a.clone();
        face_b.persist_ref_b64 = Some("AQAAfaceToothFront==".to_string());

        assert_eq!(
            target_entity_key("gear.SLDPRT", &face_a),
            target_entity_key("gear.SLDPRT", &face_a_shifted)
        );
        assert_ne!(
            target_entity_key("gear.SLDPRT", &face_a),
            target_entity_key("gear.SLDPRT", &face_b)
        );
    }

    #[test]
    fn clean_sw_ocr_label_strips_icon_artifacts() {
        assert_eq!(clean_sw_ocr_label("@放样切除"), "放样切除");
        assert_eq!(clean_sw_ocr_label("。0异型孔向导"), "异型孔向导");
        assert_eq!(clean_sw_ocr_label("•线性阵列▾"), "线性阵列");
        assert_eq!(clean_sw_ocr_label("Instant3D"), "Instant3D");
    }

    #[test]
    fn detect_hovered_button_rect_recovers_exact_small_and_split_buttons() {
        let mut img = image::RgbaImage::from_pixel(300, 100, image::Rgba([247, 247, 247, 255]));
        // Draw a small horizontal SolidWorks hovered button at (40, 12, 92x24) with 1px border (177) + white fill (255)
        for y in 12..36 {
            for x in 40..132 {
                let border = x == 40 || x == 131 || y == 12 || y == 35;
                let px = if border {
                    image::Rgba([177, 177, 177, 255])
                } else {
                    image::Rgba([255, 255, 255, 255])
                };
                img.put_pixel(x, y, px);
            }
        }
        // Add synthetic dark text strokes inside the small button (x=66..120, y=18..29)
        for y in 18..29 {
            for x in (66..120).step_by(3) {
                img.put_pixel(x, y, image::Rgba([30, 30, 30, 255]));
            }
        }
        let tb_rc = RECT {
            left: 100,
            top: 30,
            right: 400,
            bottom: 130,
        };
        let got_small = detect_hovered_button_rect_in_rgba(&img, 100, 30, tb_rc, 175, 52)
            .expect("small hovered button must be detected");
        assert_eq!((got_small.x, got_small.y, got_small.w, got_small.h), (140, 42, 92, 24));

        // Empty toolbar whitespace must return None (zero fixed cursor box!)
        assert!(detect_hovered_button_rect_in_rgba(&img, 100, 30, tb_rc, 320, 80).is_none());

        // Draw a large vertical split-button at (170, 10, 48x74): active top (10..66) + dropdown tail (66..84)
        for y in 10..66 {
            for x in 170..218 {
                let border = x == 170 || x == 217 || y == 10 || y == 65;
                let px = if border {
                    image::Rgba([177, 177, 177, 255])
                } else {
                    image::Rgba([255, 255, 255, 255])
                };
                img.put_pixel(x, y, px);
            }
        }
        for y in 66..84 {
            img.put_pixel(170, y, image::Rgba([177, 177, 177, 255]));
            img.put_pixel(217, y, image::Rgba([177, 177, 177, 255]));
        }
        for x in 170..218 {
            img.put_pixel(x, 83, image::Rgba([177, 177, 177, 255]));
        }
        // Draw an opaque 24x24 icon WITH an enclosed 6x6 white hole inside the split button to prove
        // moving the cursor over the icon, the hole, the text, or the dropdown tail ALL return the
        // exact same outer (270, 40, 48x74) rectangle with zero shrinking or flickering!
        for y in 15..39 {
            for x in 182..206 {
                let inside_hole = (191..197).contains(&x) && (24..30).contains(&y);
                let px = if inside_hole {
                    image::Rgba([255, 255, 255, 255])
                } else {
                    image::Rgba([40, 120, 200, 255])
                };
                img.put_pixel(x, y, px);
            }
        }
        for &(tx, ty) in &[
            (294, 57), // dead center of opaque icon's enclosed white hole!
            (285, 50), // opaque blue icon pixel!
            (290, 78), // text region!
            (273, 43), // top-left corner!
            (294, 105), // split-button dropdown tail!
        ] {
            let got_split = detect_hovered_button_rect_in_rgba(&img, 100, 30, tb_rc, tx, ty)
                .expect("split hovered button must be detected at every interior point");
            assert_eq!(
                (got_split.x, got_split.y, got_split.w, got_split.h),
                (270, 40, 48, 74),
                "button rect drifted at cursor ({tx},{ty})"
            );
        }
    }
}
