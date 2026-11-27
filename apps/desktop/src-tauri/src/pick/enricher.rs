//! Stage B — Decoupled Semantic Enricher (`Geometry Box -> Rich Semantics`).
//!
//! Any Stage A geometry locator (UIA, Win32 Native, MSAA, 0ms StaticVisual, OmniParser v2,
//! Hover-Diff, SAM 2.1) only needs to find the exact button/region bounding box.
//! Stage B enriches that box with semantic metadata WITHOUT ever deleting or shifting its rect:
//!   1. Preserve existing high-confidence label/description
//!   2. Passive Native Tooltip Catcher (`tooltips_class32` / `BCGPToolTip` / `AfxTooltip`) — 0ms, 0 VRAM!
//!   3. Cached Native Win32 / MSAA metadata lookup inside the box
//!   4. Non-SolidWorks UIA property lookup at box center (0 UIA on SolidWorks hard invariant!)
//!   5. Built-in Windows `OcrEngine` (`Windows.Media.Ocr`, ~10ms, 0 VRAM) with per-rect session cache.

use crate::captures::{RawScreenCapture, ScreenRect};
use crate::pick::adapters::solidworks;
use crate::pick::hitmap::{self, HitMapSource};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible,
    SendMessageTimeoutW, SMTO_ABORTIFHUNG, SMTO_BLOCK, WM_GETTEXT,
};

#[derive(Debug, Clone, Default)]
pub struct EnrichedBoxInfo {
    pub label: String,
    pub description: String,
    pub role: String,
    pub source_note: &'static str,
}

type EnrichCacheKey = (isize, i32, i32, i32, i32);

#[derive(Default)]
struct EnrichCache {
    /// `(hwnd_raw, x/4, y/4, w/4, h/4) -> (label, description)`
    entries: HashMap<EnrichCacheKey, (String, String)>,
}

static ENRICH_CACHE: OnceLock<Mutex<EnrichCache>> = OnceLock::new();

fn cache() -> &'static Mutex<EnrichCache> {
    ENRICH_CACHE.get_or_init(|| Mutex::new(EnrichCache::default()))
}

pub fn clear_session_cache() {
    if let Ok(mut g) = cache().lock() {
        g.entries.clear();
    }
}

fn rect_cache_key(hwnd_raw: isize, r: ScreenRect) -> EnrichCacheKey {
    (hwnd_raw, r.x / 4, r.y / 4, r.w / 4, r.h / 4)
}

fn is_internal_detector_tag(s: &str) -> bool {
    matches!(
        s.trim(),
        "omniparser_v2" | "hover_diff" | "hover_diff_verified"
    )
}

/// Returns `true` if a Win32 window class looks like a native desktop tooltip popup.
pub fn is_tooltip_window_class(class_lower: &str) -> bool {
    class_lower == "tooltips_class32"
        || class_lower.contains("tooltip")
        || class_lower.contains("bcgptooltip")
        || class_lower.contains("afxtooltip")
        || class_lower.contains("netuitooltip")
}

fn read_hwnd_text_fast(hwnd: HWND, timeout_ms: u32) -> String {
    let mut buf = [0u16; 384];
    let mut out_len = 0usize;
    let ok = unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_GETTEXT,
            WPARAM(buf.len()),
            LPARAM(buf.as_mut_ptr() as isize),
            SMTO_ABORTIFHUNG | SMTO_BLOCK,
            timeout_ms,
            Some(&mut out_len),
        )
    };
    if ok.0 == 0 || out_len == 0 {
        return String::new();
    }
    let n = out_len.min(buf.len().saturating_sub(1));
    String::from_utf16_lossy(&buf[..n])
        .replace(['\r', '\n'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Passively reads any visible native tooltip popup belonging to `target_pid` near `near_pt`.
/// Zero-cost miracle for pure-icon CAD/creative buttons (e.g. SolidWorks "拉伸切除 - 切除实体模型")!
pub fn capture_visible_tooltip_for_pid(target_pid: u32, near_pt: (i32, i32)) -> Option<String> {
    if target_pid == 0 {
        return None;
    }
    struct Ctx {
        pid: u32,
        near_pt: (i32, i32),
        best: Option<(i64, String)>,
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
        let mut cls_buf = [0u16; 96];
        let clen = GetClassNameW(hwnd, &mut cls_buf).max(0) as usize;
        let cls_lower = String::from_utf16_lossy(&cls_buf[..clen]).to_ascii_lowercase();
        if !is_tooltip_window_class(&cls_lower) {
            return BOOL(1);
        }
        let mut rc = RECT::default();
        if GetWindowRect(hwnd, &mut rc).is_err() {
            return BOOL(1);
        }
        let w = rc.right - rc.left;
        let h = rc.bottom - rc.top;
        if !(12..=720).contains(&w) || !(10..=360).contains(&h) {
            return BOOL(1);
        }
        let cx = (rc.left + rc.right) / 2;
        let cy = (rc.top + rc.bottom) / 2;
        let d2 = ((cx - ctx.near_pt.0) as i64).pow(2) + ((cy - ctx.near_pt.1) as i64).pow(2);
        // Must sit reasonably close to the hovered button (<= 380px)
        if d2 > 380 * 380 {
            return BOOL(1);
        }
        let text = read_hwnd_text_fast(hwnd, 45);
        if !text.trim().is_empty() && ctx.best.as_ref().is_none_or(|(bd, _)| d2 < *bd) {
            ctx.best = Some((d2, text));
        }
        BOOL(1)
    }

    let mut ctx = Ctx {
        pid: target_pid,
        near_pt,
        best: None,
    };
    unsafe {
        let _ = EnumWindows(Some(enum_cb), LPARAM(&mut ctx as *mut Ctx as isize));
    }
    ctx.best.map(|(_, t)| t)
}

/// Enriches a Stage A geometry box with underlying semantic metadata without ever modifying `rect`.
#[allow(clippy::too_many_arguments)]
pub fn enrich_box_semantics(
    child_hwnd: HWND,
    target_pid: u32,
    target_exe: &str,
    rect: ScreenRect,
    existing_label: &str,
    existing_desc: &str,
    existing_role: &str,
    clean_cap: Option<&RawScreenCapture>,
    allow_ocr: bool,
) -> EnrichedBoxInfo {
    let mut out = EnrichedBoxInfo {
        label: existing_label.trim().to_string(),
        description: existing_desc.trim().to_string(),
        role: if existing_role.trim().is_empty() {
            "Button".to_string()
        } else {
            existing_role.trim().to_string()
        },
        source_note: "existing",
    };

    let hwnd_raw = child_hwnd.0 as isize;
    let ck = rect_cache_key(hwnd_raw, rect);
    let cx = rect.x + rect.w / 2;
    let cy = rect.y + rect.h / 2;

    // 0. Check session enrichment cache first (<0.01ms)
    if let Ok(g) = cache().lock() {
        if let Some((clbl, cdesc)) = g.entries.get(&ck) {
            if out.label.is_empty() && !clbl.is_empty() {
                out.label = clbl.clone();
                out.source_note = "cache";
            }
            if out.description.is_empty() && !cdesc.is_empty() {
                out.description = cdesc.clone();
            }
        }
    }

    // 1. Cross-check cached MSAA / Win32 native entries for this container at box center
    if out.label.is_empty() && hwnd_raw != 0 {
        if let Some(msaa_entries) = hitmap::get_cached_entries(hwnd_raw, HitMapSource::MsaaSnapshot)
        {
            if let Some(hit) = hitmap::find_entry_at(&msaa_entries, cx, cy) {
                if !hit.label.trim().is_empty() {
                    out.label = hit.label.trim().to_string();
                    out.source_note = "msaa_native";
                }
                if out.description.is_empty() && !hit.description.trim().is_empty() {
                    out.description = hit.description.trim().to_string();
                }
                if !hit.role.trim().is_empty() && out.role == "Button" {
                    out.role = hit.role;
                }
            }
        }
    }

    // 2. Passive Native Tooltip Catcher (zero-VRAM, reads live `tooltips_class32` / `BCGPToolTip`)
    //    Skip EnumWindows overhead when both label and real description are already populated!
    if out.label.is_empty() || out.description.is_empty() || is_internal_detector_tag(&out.description)
    {
        if let Some(tip) = capture_visible_tooltip_for_pid(target_pid, (cx, cy)) {
            if out.label.is_empty() {
                // Split "Title — Description" or take concise leading segment as label, full text as description
                let first = tip
                    .split(['—', '–', '\t'])
                    .next()
                    .map(str::trim)
                    .unwrap_or(&tip);
                let lbl = if first.chars().count() <= 28 {
                    first.to_string()
                } else {
                    first.chars().take(28).collect::<String>()
                };
                out.label = lbl;
                out.source_note = "native_tooltip";
            }
            if out.description.is_empty() || is_internal_detector_tag(&out.description) {
                out.description = tip;
            }
        }
    }

    // 3. Built-in Windows OcrEngine (~10ms, 0 VRAM) when label is still empty and clean capture is available
    if allow_ocr && out.label.is_empty() {
        if let Some(raw) = clean_cap {
            if let Some(ocr_lbl) = solidworks::ocr_crop_from_raw_capture(raw, rect, (cx, cy), true) {
                if !ocr_lbl.trim().is_empty() {
                    out.label = ocr_lbl;
                    out.source_note = "windows_ocr";
                }
            }
        }
    }

    // Never leak internal detector tag names as user-visible description
    if is_internal_detector_tag(&out.description) {
        out.description.clear();
    }

    // Persist any newly discovered semantic metadata in session cache
    if (!out.label.is_empty() || !out.description.is_empty())
        && !target_exe.is_empty()
    {
        if let Ok(mut g) = cache().lock() {
            if g.entries.len() > 1024 {
                g.entries.clear();
            }
            g.entries
                .insert(ck, (out.label.clone(), out.description.clone()));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tooltip_class_classifier_matches_native_and_cad_tooltips() {
        assert!(is_tooltip_window_class("tooltips_class32"));
        assert!(is_tooltip_window_class("bcgptooltipctrl"));
        assert!(is_tooltip_window_class("afxtooltip42"));
        assert!(is_tooltip_window_class("netuitooltip"));
        assert!(!is_tooltip_window_class("button"));
        assert!(!is_tooltip_window_class("afx:toolbar:400000:8"));
    }

    #[test]
    fn enricher_preserves_existing_label_and_strips_internal_tags() {
        clear_session_cache();
        let rc = ScreenRect {
            x: 100,
            y: 50,
            w: 64,
            h: 48,
        };
        let res = enrich_box_semantics(
            HWND::default(),
            0,
            "sldworks.exe",
            rc,
            "拉伸凸台/基体",
            "omniparser_v2",
            "Button",
            None,
            false,
        );
        assert_eq!(res.label, "拉伸凸台/基体");
        assert!(res.description.is_empty());
        assert_eq!(res.role, "Button");
    }
}
