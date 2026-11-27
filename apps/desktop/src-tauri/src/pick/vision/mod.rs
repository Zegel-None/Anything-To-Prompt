pub mod cloud_vlm;
pub mod ollama_vlm;
pub mod sidecar_client;

use crate::captures::ScreenRect;
use crate::config::{AppConfig, SegmentationBackend, VisionConfig, VisionHardwarePreset, VlmBackend};
use anyhow::{anyhow, Result};
use image::{Rgba, RgbaImage};
use serde::{Deserialize, Serialize};
use std::os::windows::process::CommandExt;
use std::process::Command;
use std::time::Instant;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisionSegmentItem {
    pub rect: ScreenRect,
    pub label: String,
    pub role: String,
    pub semantic_kind: String, // "ui_vlm_fixed" | "photo_object"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub polygon: Option<Vec<(i32, i32)>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiAuditResult {
    pub verdict: String, // "ok" | "fix_needed" | "switch_to_visual_mode"
    pub drop_ids: Vec<usize>,
    pub corrected_items: Vec<VisionSegmentItem>,
    pub switch_to_visual_mode: bool,
    pub backend_used: String,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotoSegmentResult {
    pub items: Vec<VisionSegmentItem>,
    pub backend_used: String,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareProbeReport {
    pub gpu_name: String,
    pub vram_total_mb: u64,
    pub vram_free_mb: u64,
    pub cpu_cores: usize,
    pub ram_total_gb: u64,
    pub ollama_running: bool,
    pub ollama_models: Vec<String>,
    pub sidecar_env_ready: bool,
    pub sidecar_script_ready: bool,
    pub sidecar_running: bool,
    pub downloaded_models: Vec<String>,
    pub recommended_preset: VisionHardwarePreset,
    pub recommendation_reason: String,
}

/// Pure pixel heuristic (< 2ms): distinguishes natural photographs (people, grass, mountains, rivers)
/// from flat solid-color desktop UI chrome / text documents.
pub fn is_natural_photo_region(img: &RgbaImage) -> bool {
    let w = img.width();
    let h = img.height();
    if w < 120 || h < 96 {
        return false;
    }
    let step_x = (w / 48).max(1);
    let step_y = (h / 36).max(1);
    let mut hist = [0u32; 512]; // 3-bit R x 3-bit G x 3-bit B = 512 bins
    let mut total = 0u32;
    let mut sat_pixels = 0u32;
    let mut grad_sum = 0u64;
    let mut grad_cnt = 0u64;

    let mut y = 0u32;
    while y < h {
        let mut x = 0u32;
        while x < w {
            let p = img.get_pixel(x, y).0;
            let bin = (((p[0] >> 5) as usize) << 6)
                | (((p[1] >> 5) as usize) << 3)
                | ((p[2] >> 5) as usize);
            hist[bin] += 1;
            total += 1;

            let maxc = p[0].max(p[1]).max(p[2]);
            let minc = p[0].min(p[1]).min(p[2]);
            if maxc.saturating_sub(minc) >= 24 {
                sat_pixels += 1;
            }
            if x + step_x < w {
                let p2 = img.get_pixel(x + step_x, y).0;
                let d = (p[0] as i32 - p2[0] as i32).abs()
                    + (p[1] as i32 - p2[1] as i32).abs()
                    + (p[2] as i32 - p2[2] as i32).abs();
                grad_sum += d as u64;
                grad_cnt += 1;
            }
            x += step_x;
        }
        y += step_y;
    }
    if total < 64 {
        return false;
    }
    let mut max_bin = 0u32;
    let mut active_bins = 0u32;
    let mut entropy = 0.0f64;
    let tf = total as f64;
    for &c in &hist {
        if c > 0 {
            active_bins += 1;
            if c > max_bin {
                max_bin = c;
            }
            let prob = (c as f64) / tf;
            entropy -= prob * prob.log2();
        }
    }
    let dominant_ratio = (max_bin as f64) / tf;
    let sat_ratio = (sat_pixels as f64) / tf;
    let mean_grad = if grad_cnt > 0 {
        (grad_sum as f64) / (grad_cnt as f64)
    } else {
        0.0
    };

    // UI chrome typically has dominant_ratio > 0.45 and entropy < 3.4;
    // Natural photos (portraits, grass, mountains, rivers) have rich color spread & continuous gradients.
    entropy >= 4.1 && dominant_ratio <= 0.34 && active_bins >= 26 && mean_grad >= 14.0 && sat_ratio >= 0.22
}

/// Converts a `[ymin, xmin, ymax, xmax]` norm-1000 box into absolute screen pixels inside crop `(ox, oy, cw, ch)`.
pub fn norm1000_to_screen_rect(
    bbox: [i32; 4],
    origin_x: i32,
    origin_y: i32,
    crop_w: i32,
    crop_h: i32,
) -> Option<ScreenRect> {
    let ymin = bbox[0].clamp(0, 1000);
    let xmin = bbox[1].clamp(0, 1000);
    let ymax = bbox[2].clamp(0, 1000);
    let xmax = bbox[3].clamp(0, 1000);
    if xmax <= xmin + 8 || ymax <= ymin + 8 {
        return None;
    }
    let x0 = origin_x + ((xmin as i64 * crop_w as i64) / 1000) as i32;
    let y0 = origin_y + ((ymin as i64 * crop_h as i64) / 1000) as i32;
    let x1 = origin_x + ((xmax as i64 * crop_w as i64) / 1000) as i32;
    let y1 = origin_y + ((ymax as i64 * crop_h as i64) / 1000) as i32;
    let w = (x1 - x0).max(0);
    let h = (y1 - y0).max(0);
    if w < 12 || h < 12 {
        return None;
    }
    Some(ScreenRect { x: x0, y: y0, w, h })
}

/// Renders bright cyan outlines onto a copy of `base` so the VLM can visually verify whether existing
/// blue boxes are aligned, shifted, stuck together, or missing elements.
pub fn draw_existing_boxes_for_audit(
    base: &RgbaImage,
    origin_x: i32,
    origin_y: i32,
    boxes: &[(usize, ScreenRect)],
) -> RgbaImage {
    let mut out = base.clone();
    let w = out.width() as i32;
    let h = out.height() as i32;
    let cyan = Rgba([56, 189, 248, 255]);
    let white = Rgba([255, 255, 255, 255]);
    for &(_id, r) in boxes {
        let x0 = (r.x - origin_x).clamp(0, w - 1);
        let y0 = (r.y - origin_y).clamp(0, h - 1);
        let x1 = (r.x + r.w - origin_x).clamp(0, w - 1);
        let y1 = (r.y + r.h - origin_y).clamp(0, h - 1);
        if x1 <= x0 + 2 || y1 <= y0 + 2 {
            continue;
        }
        for t in 0..2 {
            let yt = (y0 + t).min(h - 1);
            let yb = (y1 - t).max(0);
            for x in x0..=x1 {
                out.put_pixel(x as u32, yt as u32, cyan);
                out.put_pixel(x as u32, yb as u32, cyan);
            }
            let xl = (x0 + t).min(w - 1);
            let xr = (x1 - t).max(0);
            for y in y0..=y1 {
                out.put_pixel(xl as u32, y as u32, cyan);
                out.put_pixel(xr as u32, y as u32, cyan);
            }
        }
        // Small white corner marker so VLM sees box origin clearly
        for dy in 0..4 {
            for dx in 0..4 {
                let px = (x0 + dx).min(w - 1);
                let py = (y0 + dy).min(h - 1);
                out.put_pixel(px as u32, py as u32, white);
            }
        }
    }
    out
}

fn call_vlm_with_fallback(
    cfg: &VisionConfig,
    prompt: &str,
    images_b64: &[String],
) -> Result<(serde_json::Value, String)> {
    match cfg.vlm_backend {
        VlmBackend::CloudOpenAiCompat | VlmBackend::CloudGemini => {
            let v = cloud_vlm::call_cloud_vlm_json(cfg, prompt, images_b64)?;
            Ok((v, format!("cloud:{}", cfg.cloud_model)))
        }
        VlmBackend::LocalOllama => match ollama_vlm::call_ollama_vlm_json(cfg, prompt, images_b64) {
            Ok(v) => Ok((v, format!("ollama:{}", cfg.ollama_model))),
            Err(e) => {
                if !cfg.cloud_api_key.trim().is_empty() {
                    let v = cloud_vlm::call_cloud_vlm_json(cfg, prompt, images_b64)?;
                    Ok((v, format!("cloud_fallback:{}", cfg.cloud_model)))
                } else {
                    Err(e)
                }
            }
        },
        VlmBackend::SidecarFlorence2 | VlmBackend::Disabled => {
            if !cfg.cloud_api_key.trim().is_empty() {
                let v = cloud_vlm::call_cloud_vlm_json(cfg, prompt, images_b64)?;
                Ok((v, format!("cloud_fallback:{}", cfg.cloud_model)))
            } else {
                Err(anyhow!("未启用可用于视觉复核的 VLM 后端"))
            }
        }
    }
}

/// Consensus-preserving additive filter for Pass 3 UI audits:
/// 1. NEVER blindly drops verified ground-truth UIA boxes (labeled, compact controls);
/// 2. NEVER stacks a new VLM box on top of any surviving accurate box (overlap >= 35% rejected).
pub fn filter_audit_patch_additively(
    existing: &[(usize, ScreenRect, String, bool)], // (local_id, rect, label, is_uia_ground_truth)
    raw_drop_ids: Vec<usize>,
    raw_corrected: Vec<VisionSegmentItem>,
) -> (Vec<usize>, Vec<VisionSegmentItem>) {
    // 1. Protect verified ground-truth UIA controls from destructive VLM drops
    let safe_drop_ids: Vec<usize> = raw_drop_ids
        .into_iter()
        .filter(|&id| {
            existing
                .iter()
                .find(|(eid, _, _, _)| *eid == id)
                .is_some_and(|(_, r, lbl, is_uia)| {
                    // Only allow dropping non-UIA visual boxes OR unlabeled oversized boxes
                    !is_uia || (lbl.trim().is_empty() && (r.w > 320 || r.h > 140))
                })
        })
        .collect();

    let drop_set: std::collections::HashSet<usize> = safe_drop_ids.iter().copied().collect();
    let surviving_rects: Vec<ScreenRect> = existing
        .iter()
        .filter(|(id, _, _, _)| !drop_set.contains(id))
        .map(|(_, r, _, _)| *r)
        .collect();

    // 2. Additive-only acceptance: keep VLM items that genuinely fill a gap or replace a dropped bad box
    let mut kept_new: Vec<VisionSegmentItem> = Vec::new();
    for item in raw_corrected {
        let overlaps_surviving = surviving_rects.iter().any(|sr| rect_overlap_ratio(item.rect, *sr) >= 0.35);
        if overlaps_surviving {
            continue;
        }
        let overlaps_self = kept_new.iter().any(|kn| rect_overlap_ratio(item.rect, kn.rect) >= 0.38);
        if overlaps_self {
            continue;
        }
        kept_new.push(item);
    }

    (safe_drop_ids, kept_new)
}

fn rect_overlap_ratio(a: ScreenRect, b: ScreenRect) -> f64 {
    let ix0 = a.x.max(b.x);
    let iy0 = a.y.max(b.y);
    let ix1 = (a.x + a.w).min(b.x + b.w);
    let iy1 = (a.y + a.h).min(b.y + b.h);
    let iw = (ix1 - ix0).max(0) as f64;
    let ih = (iy1 - iy0).max(0) as f64;
    if iw <= 0.0 || ih <= 0.0 {
        return 0.0;
    }
    let inter = iw * ih;
    let min_area = ((a.w.max(1) as f64) * (a.h.max(1) as f64))
        .min((b.w.max(1) as f64) * (b.h.max(1) as f64))
        .max(1.0);
    inter / min_area
}

/// Pass 3A: Additive, consensus-preserving visual audit of UI boxes when user deliberately re-brushes.
/// IMPORTANT: `Florence-2 <OD>` is strictly reserved for natural photographs (`segment_natural_photo`),
/// NEVER for overwriting desktop UI chrome!
pub fn audit_ui_region(
    cfg: &VisionConfig,
    clean_crop: &RgbaImage,
    origin_x: i32,
    origin_y: i32,
    existing_boxes: &[(usize, ScreenRect, String, bool)],
) -> Result<UiAuditResult> {
    let t0 = Instant::now();
    let cw = clean_crop.width() as i32;
    let ch = clean_crop.height() as i32;

    // Never run Florence-2 natural-object detector on UI chrome (it would misclassify UI widgets!)
    let has_reasoning_vlm = matches!(
        cfg.vlm_backend,
        VlmBackend::CloudOpenAiCompat | VlmBackend::CloudGemini | VlmBackend::LocalOllama
    ) || !cfg.cloud_api_key.trim().is_empty();
    if !has_reasoning_vlm {
        return Ok(UiAuditResult {
            verdict: "ok".to_string(),
            drop_ids: Vec::new(),
            corrected_items: Vec::new(),
            switch_to_visual_mode: false,
            backend_used: "local_additive_guard".to_string(),
            latency_ms: t0.elapsed().as_millis() as u64,
        });
    }

    let clean_b64 = cloud_vlm::encode_rgba_to_jpeg_base64(clean_crop, 960)?;
    let boxes_only: Vec<(usize, ScreenRect)> = existing_boxes
        .iter()
        .map(|(id, r, _, _)| (*id, *r))
        .collect();
    let annotated = draw_existing_boxes_for_audit(clean_crop, origin_x, origin_y, &boxes_only);
    let ann_b64 = cloud_vlm::encode_rgba_to_jpeg_base64(&annotated, 960)?;

    let existing_desc: Vec<serde_json::Value> = existing_boxes
        .iter()
        .map(|(id, r, lbl, is_uia)| {
            let xmin = (((r.x - origin_x) as f64 / cw.max(1) as f64) * 1000.0).round() as i32;
            let ymin = (((r.y - origin_y) as f64 / ch.max(1) as f64) * 1000.0).round() as i32;
            let xmax = (((r.x + r.w - origin_x) as f64 / cw.max(1) as f64) * 1000.0).round() as i32;
            let ymax = (((r.y + r.h - origin_y) as f64 / ch.max(1) as f64) * 1000.0).round() as i32;
            serde_json::json!({
                "id": id,
                "label": lbl,
                "source": if *is_uia { "uia_dom" } else { "visual" },
                "bbox": [ymin.clamp(0,1000), xmin.clamp(0,1000), ymax.clamp(0,1000), xmax.clamp(0,1000)]
            })
        })
        .collect();

    let prompt = format!(
        "You are a conservative additive UI grounding verifier. Image 1 is the clean screenshot crop. Image 2 shows current cyan detection boxes: {}.\n\
         RULES:\n\
         1. Existing boxes are usually ALREADY CORRECT — DO NOT negate or drop accurate boxes! Prefer verdict=\"ok\" whenever existing boxes align with visible UI controls/text.\n\
         2. Only put an ID in drop_ids if that box frames completely empty space, is badly misaligned (>30% off), or merges 3+ separate buttons.\n\
         3. Only add genuinely MISSING controls/text in corrected_boxes that are NOT already covered by a surviving box.\n\
         Return STRICT JSON ONLY:\n\
         {{\"verdict\":\"ok|fix_needed|switch_to_visual_mode\",\"drop_ids\":[],\"corrected_boxes\":[{{\"label\":\"short name\",\"role\":\"Button|Text|Input|MenuItem|Image\",\"bbox\":[ymin,xmin,ymax,xmax]}}]}}\n\
         All bbox coordinates are integers in [0,1000] relative to Image 1.",
        serde_json::to_string(&existing_desc).unwrap_or_default()
    );

    let (val, backend_used) = call_vlm_with_fallback(cfg, &prompt, &[clean_b64, ann_b64])?;
    let verdict = val
        .get("verdict")
        .and_then(|v| v.as_str())
        .unwrap_or("ok")
        .to_string();
    let raw_drop_ids: Vec<usize> = val
        .get("drop_ids")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|n| n.as_u64().map(|u| u as usize))
                .collect()
        })
        .unwrap_or_default();

    let mut raw_corrected = Vec::new();
    if let Some(arr) = val.get("corrected_boxes").and_then(|v| v.as_array()) {
        for item in arr.iter().take(16) {
            let Some(b) = item.get("bbox").and_then(|v| v.as_array()) else {
                continue;
            };
            if b.len() != 4 {
                continue;
            }
            let raw_b = [
                b[0].as_i64().unwrap_or(0) as i32,
                b[1].as_i64().unwrap_or(0) as i32,
                b[2].as_i64().unwrap_or(0) as i32,
                b[3].as_i64().unwrap_or(0) as i32,
            ];
            if let Some(rect) = norm1000_to_screen_rect(raw_b, origin_x, origin_y, cw, ch) {
                raw_corrected.push(VisionSegmentItem {
                    rect,
                    label: item
                        .get("label")
                        .and_then(|v| v.as_str())
                        .unwrap_or("控件")
                        .to_string(),
                    role: item
                        .get("role")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Button")
                        .to_string(),
                    semantic_kind: "ui_vlm_fixed".to_string(),
                    polygon: None,
                });
            }
        }
    }

    let (drop_ids, corrected_items) =
        filter_audit_patch_additively(existing_boxes, raw_drop_ids, raw_corrected);
    let switch_to_visual_mode = verdict == "switch_to_visual_mode" && !corrected_items.is_empty();
    Ok(UiAuditResult {
        verdict,
        drop_ids,
        corrected_items,
        switch_to_visual_mode,
        backend_used,
        latency_ms: t0.elapsed().as_millis() as u64,
    })
}

/// Pass 3B: Natural photo semantic segmentation / matting ("人 / 人 / 草 / 山 / 河").
pub fn segment_natural_photo(
    cfg: &VisionConfig,
    clean_crop: &RgbaImage,
    origin_x: i32,
    origin_y: i32,
    brush_pts_local: &[[i32; 2]],
) -> Result<PhotoSegmentResult> {
    let t0 = Instant::now();
    let cw = clean_crop.width() as i32;
    let ch = clean_crop.height() as i32;
    let (clean_b64, sent_w, sent_h) = cloud_vlm::encode_rgba_to_jpeg_base64_with_size(clean_crop, 960)?;
    let to_sent_x = (sent_w.max(1) as f64) / (cw.max(1) as f64);
    let to_sent_y = (sent_h.max(1) as f64) / (ch.max(1) as f64);
    let back_scale_x = (cw.max(1) as f64) / (sent_w.max(1) as f64);
    let back_scale_y = (ch.max(1) as f64) / (sent_h.max(1) as f64);
    let scaled_pts: Vec<[i32; 2]> = brush_pts_local
        .iter()
        .map(|p| {
            [
                ((p[0] as f64) * to_sent_x).round() as i32,
                ((p[1] as f64) * to_sent_y).round() as i32,
            ]
        })
        .collect();

    // Route 1: Local Sidecar Florence-2 + SAM 2.1 / MobileSAM all-in-one offline pipeline
    if cfg.vlm_backend == VlmBackend::SidecarFlorence2
        || (cfg.vlm_backend == VlmBackend::Disabled
            && cfg.segmentation_backend != SegmentationBackend::Disabled)
    {
        let segs = sidecar_client::auto_semantic_segment(cfg, &clean_b64, &scaled_pts, "photo")?;
        let items = sidecar_segments_to_screen(segs, origin_x, origin_y, back_scale_x, back_scale_y);
        return Ok(PhotoSegmentResult {
            items,
            backend_used: format!("sidecar:{}", sidecar_client::backend_str(cfg.segmentation_backend)),
            latency_ms: t0.elapsed().as_millis() as u64,
        });
    }

    // Route 2: VLM semantic detection (Ollama or Cloud VLM) -> optional SAM polygon refinement
    let prompt = "Segment this photograph into distinct meaningful subjects and landscape regions \
        (such as each individual person, animal, grass/lawn, mountain, river/water, sky, tree, building, vehicle, object). \
        Return STRICT JSON ONLY:\n\
        {\"objects\":[{\"label\":\"concise Chinese semantic name e.g. 左侧人物 / 右侧人物 / 草地 / 远山 / 河流\",\"bbox\":[ymin,xmin,ymax,xmax]}]}\n\
        Coordinates are integers in [0,1000]. Include up to 16 clearest non-overlapping regions.";

    let (val, vlm_name) = call_vlm_with_fallback(cfg, prompt, std::slice::from_ref(&clean_b64))?;
    let mut local_boxes_sent: Vec<(String, [i32; 4])> = Vec::new();
    let mut fallback_items: Vec<VisionSegmentItem> = Vec::new();

    if let Some(arr) = val.get("objects").and_then(|v| v.as_array()) {
        for obj in arr.iter().take(16) {
            let Some(b) = obj.get("bbox").and_then(|v| v.as_array()) else {
                continue;
            };
            if b.len() != 4 {
                continue;
            }
            let raw_b = [
                b[0].as_i64().unwrap_or(0) as i32,
                b[1].as_i64().unwrap_or(0) as i32,
                b[2].as_i64().unwrap_or(0) as i32,
                b[3].as_i64().unwrap_or(0) as i32,
            ];
            let label = obj
                .get("label")
                .and_then(|v| v.as_str())
                .unwrap_or("物体")
                .to_string();
            if let Some(sr) = norm1000_to_screen_rect(raw_b, origin_x, origin_y, cw, ch) {
                local_boxes_sent.push((
                    label.clone(),
                    [
                        (((sr.x - origin_x) as f64) * to_sent_x).round() as i32,
                        (((sr.y - origin_y) as f64) * to_sent_y).round() as i32,
                        ((sr.w as f64) * to_sent_x).round().max(1.0) as i32,
                        ((sr.h as f64) * to_sent_y).round().max(1.0) as i32,
                    ],
                ));
                fallback_items.push(VisionSegmentItem {
                    rect: sr,
                    label,
                    role: "PhotoObject".to_string(),
                    semantic_kind: "photo_object".to_string(),
                    polygon: None,
                });
            }
        }
    }

    // Refine VLM boxes into organic pixel-tight polygons if any SAM segmentation backend is enabled!
    if !local_boxes_sent.is_empty() && cfg.segmentation_backend != SegmentationBackend::Disabled {
        if let Ok(segs) =
            sidecar_client::segment_boxes_to_polygons(cfg, &clean_b64, &local_boxes_sent, &scaled_pts)
        {
            let refined =
                sidecar_segments_to_screen(segs, origin_x, origin_y, back_scale_x, back_scale_y);
            if !refined.is_empty() {
                return Ok(PhotoSegmentResult {
                    items: refined,
                    backend_used: format!(
                        "{vlm_name}+{}",
                        sidecar_client::backend_str(cfg.segmentation_backend)
                    ),
                    latency_ms: t0.elapsed().as_millis() as u64,
                });
            }
        }
    }

    Ok(PhotoSegmentResult {
        items: fallback_items,
        backend_used: vlm_name,
        latency_ms: t0.elapsed().as_millis() as u64,
    })
}

fn sidecar_segments_to_screen(
    segs: Vec<sidecar_client::SidecarSegmentOut>,
    origin_x: i32,
    origin_y: i32,
    scale_x: f64,
    scale_y: f64,
) -> Vec<VisionSegmentItem> {
    segs.into_iter()
        .filter_map(|s| {
            let rect = ScreenRect {
                x: origin_x + ((s.bbox[0] as f64) * scale_x).round() as i32,
                y: origin_y + ((s.bbox[1] as f64) * scale_y).round() as i32,
                w: ((s.bbox[2].max(0) as f64) * scale_x).round() as i32,
                h: ((s.bbox[3].max(0) as f64) * scale_y).round() as i32,
            };
            if rect.w < 14 || rect.h < 14 {
                return None;
            }
            let polygon = if s.polygon.len() >= 3 {
                Some(
                    s.polygon
                        .into_iter()
                        .map(|p| {
                            (
                                origin_x + ((p[0] as f64) * scale_x).round() as i32,
                                origin_y + ((p[1] as f64) * scale_y).round() as i32,
                            )
                        })
                        .collect(),
                )
            } else {
                None
            };
            Some(VisionSegmentItem {
                rect,
                label: if s.label.trim().is_empty() {
                    "图像主体".to_string()
                } else {
                    s.label
                },
                role: "PhotoObject".to_string(),
                semantic_kind: "photo_object".to_string(),
                polygon,
            })
        })
        .collect()
}

/// Probes GPU/VRAM, CPU/RAM, Ollama, and local Vision Sidecar readiness to recommend the best hardware preset.
pub fn probe_local_hardware(cfg: &VisionConfig) -> HardwareProbeReport {
    let mut gpu_name = "未检测到独立 NVIDIA GPU".to_string();
    let mut vram_total_mb = 0u64;
    let mut vram_free_mb = 0u64;

    if let Ok(out) = Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,memory.total,memory.free",
            "--format=csv,noheader,nounits",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            if let Some(first) = text.lines().next() {
                let parts: Vec<&str> = first.split(',').map(|s| s.trim()).collect();
                if parts.len() >= 3 {
                    gpu_name = parts[0].to_string();
                    vram_total_mb = parts[1].parse::<u64>().unwrap_or(0);
                    vram_free_mb = parts[2].parse::<u64>().unwrap_or(0);
                }
            }
        }
    }

    let ram_total_gb = if let Ok(mut sys) = sysinfo::System::new() {
        sys.refresh_memory();
        (sys.total_memory() / (1024 * 1024 * 1024)).max(1)
    } else {
        16
    };
    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    let (ollama_running, ollama_models) = match ollama_vlm::list_ollama_models(&cfg.ollama_base_url)
    {
        Ok(m) => (true, m),
        Err(_) => (false, Vec::new()),
    };

    let sidecar_env_ready = sidecar_client::resolve_sidecar_python(cfg).is_some();
    let sidecar_script_ready = sidecar_client::resolve_sidecar_script().is_some();
    let sidecar_running = sidecar_client::health(cfg).is_ok();

    let mut downloaded_models = Vec::new();
    let models_dir = AppConfig::vision_models_dir();
    if let Ok(rd) = std::fs::read_dir(&models_dir) {
        for ent in rd.flatten() {
            downloaded_models.push(ent.file_name().to_string_lossy().to_string());
        }
    }
    downloaded_models.sort();

    let (recommended_preset, recommendation_reason) = if vram_total_mb >= 12000 {
        (
            VisionHardwarePreset::ProAiWorkstation,
            format!(
                "检测到 {gpu_name} ({:.1} GB 显存)，支持旗舰离线 SAM 2.1 Small + Florence-2 / Qwen2.5-VL 全链路方案",
                vram_total_mb as f64 / 1024.0
            ),
        )
    } else if vram_total_mb >= 5500 {
        (
            VisionHardwarePreset::BalancedUniversal,
            format!(
                "检测到 {gpu_name} ({:.1} GB 显存)，推荐中度万金油档（SAM 2.1 Tiny + Florence-2-base）",
                vram_total_mb as f64 / 1024.0
            ),
        )
    } else {
        (
            VisionHardwarePreset::LiteOffice,
            "无高显存独显或纯核显环境，推荐极速办公本档（0 显存，云端 VLM + 可选 CPU 轻量抠图）".to_string(),
        )
    };

    HardwareProbeReport {
        gpu_name,
        vram_total_mb,
        vram_free_mb,
        cpu_cores,
        ram_total_gb,
        ollama_running,
        ollama_models,
        sidecar_env_ready,
        sidecar_script_ready,
        sidecar_running,
        downloaded_models,
        recommended_preset,
        recommendation_reason,
    }
}

/// Live end-to-end connectivity & latency smoke test invoked from Settings UI.
pub fn test_active_pipeline(cfg: &VisionConfig) -> serde_json::Value {
    let t0 = Instant::now();
    let mut seg_status = serde_json::json!({ "enabled": false, "ok": true, "detail": "未启用抠图引擎" });
    let mut vlm_status = serde_json::json!({ "enabled": false, "ok": true, "detail": "未启用 VLM 校验引擎" });

    if cfg.segmentation_backend != SegmentationBackend::Disabled {
        match sidecar_client::ensure_sidecar_running(cfg) {
            Ok(h) => {
                seg_status = serde_json::json!({
                    "enabled": true,
                    "ok": true,
                    "device": h.device,
                    "availableBackends": h.available_backends,
                    "detail": format!("Sidecar 就绪 ({}, 已加载 {:?})", h.device, h.loaded_models),
                });
            }
            Err(e) => {
                seg_status = serde_json::json!({
                    "enabled": true,
                    "ok": false,
                    "detail": e.to_string(),
                });
            }
        }
    }

    match cfg.vlm_backend {
        VlmBackend::Disabled => {}
        VlmBackend::SidecarFlorence2 => match sidecar_client::ensure_sidecar_running(cfg) {
            Ok(h) => {
                vlm_status = serde_json::json!({
                    "enabled": true,
                    "ok": true,
                    "detail": format!("本地 Sidecar Florence-2 就绪 ({})", h.device),
                });
            }
            Err(e) => {
                vlm_status = serde_json::json!({ "enabled": true, "ok": false, "detail": e.to_string() });
            }
        },
        VlmBackend::LocalOllama => match ollama_vlm::list_ollama_models(&cfg.ollama_base_url) {
            Ok(models) => {
                let target = cfg.ollama_model.trim();
                let found = models.iter().any(|m| m.starts_with(target));
                vlm_status = serde_json::json!({
                    "enabled": true,
                    "ok": found,
                    "models": models,
                    "detail": if found {
                        format!("Ollama 在线，目标模型 {target} 已就绪")
                    } else {
                        format!("Ollama 在线，但未找到 {target}（已有 {} 个模型）", models.len())
                    },
                });
            }
            Err(e) => {
                vlm_status = serde_json::json!({ "enabled": true, "ok": false, "detail": e.to_string() });
            }
        },
        VlmBackend::CloudOpenAiCompat | VlmBackend::CloudGemini => {
            // Tiny 64x64 test image to verify real key + endpoint round-trip
            let img = RgbaImage::from_pixel(64, 64, Rgba([40, 140, 220, 255]));
            match cloud_vlm::encode_rgba_to_jpeg_base64(&img, 64).and_then(|b64| {
                cloud_vlm::call_cloud_vlm_json(
                    cfg,
                    "Reply with strict JSON: {\"ok\": true, \"color\": \"blue\"}",
                    &[b64],
                )
            }) {
                Ok(_) => {
                    vlm_status = serde_json::json!({
                        "enabled": true,
                        "ok": true,
                        "detail": format!("云端 VLM ({}) 连通成功", cfg.cloud_model),
                    });
                }
                Err(e) => {
                    vlm_status = serde_json::json!({ "enabled": true, "ok": false, "detail": e.to_string() });
                }
            }
        }
    }

    let ok = seg_status["ok"].as_bool().unwrap_or(false) && vlm_status["ok"].as_bool().unwrap_or(false);
    serde_json::json!({
        "ok": ok,
        "latencyMs": t0.elapsed().as_millis() as u64,
        "segmentation": seg_status,
        "vlm": vlm_status,
    })
}

pub fn unload_all_local_models(cfg: &VisionConfig) -> serde_json::Value {
    let sidecar_ok = sidecar_client::unload_models(cfg).is_ok();
    let ollama_ok = ollama_vlm::unload_ollama_model(cfg).is_ok();
    serde_json::json!({
        "ok": sidecar_ok || ollama_ok,
        "sidecarUnloaded": sidecar_ok,
        "ollamaUnloaded": ollama_ok,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_photo_detector_distinguishes_flat_ui_from_colorful_scene() {
        // Flat dark UI panel -> NOT a photo
        let mut ui = RgbaImage::from_pixel(240, 180, Rgba([28, 30, 34, 255]));
        for y in 40..64 {
            for x in 20..180 {
                ui.put_pixel(x, y, Rgba([230, 232, 236, 255]));
            }
        }
        assert!(!is_natural_photo_region(&ui));

        // Synthetic natural landscape (sky gradient + mountain + grass + river + colorful subjects) -> IS a photo
        let mut photo = RgbaImage::new(240, 180);
        for y in 0..180u32 {
            for x in 0..240u32 {
                let r = ((x * 3 + y * 5) % 240) as u8;
                let g = ((x * 7 + y * 2 + 60) % 235) as u8;
                let b = ((x * 2 + y * 9 + 110) % 245) as u8;
                photo.put_pixel(x, y, Rgba([r, g, b, 255]));
            }
        }
        assert!(is_natural_photo_region(&photo));
    }

    #[test]
    fn norm1000_maps_accurately_to_screen_pixels() {
        let r = norm1000_to_screen_rect([250, 100, 750, 600], 500, 200, 400, 200).unwrap();
        assert_eq!(r, ScreenRect { x: 540, y: 250, w: 200, h: 100 });
    }

    #[test]
    fn hardware_presets_apply_clean_combinations() {
        let mut cfg = VisionConfig::default();
        cfg.apply_preset(VisionHardwarePreset::BalancedUniversal);
        assert_eq!(cfg.segmentation_backend, SegmentationBackend::GpuSam2Tiny);
        assert_eq!(cfg.vlm_backend, VlmBackend::SidecarFlorence2);

        cfg.apply_preset(VisionHardwarePreset::ProAiWorkstation);
        assert_eq!(cfg.segmentation_backend, SegmentationBackend::GpuSam2Small);
        assert_eq!(cfg.vlm_backend, VlmBackend::LocalOllama);
    }

    #[test]
    fn additive_audit_protects_good_uia_boxes_and_only_adds_missing_gaps() {
        let good_uia = ScreenRect { x: 100, y: 100, w: 120, h: 28 };
        let bad_visual = ScreenRect { x: 100, y: 200, w: 80, h: 24 };
        let existing = vec![
            (0usize, good_uia, "新建会话".to_string(), true),
            (1usize, bad_visual, String::new(), false),
        ];
        // VLM mistakenly tries to drop BOTH id=0 (good UIA) and id=1 (bad visual),
        // and proposes an overlapping replacement for id=0 plus a genuinely missing button at y=300
        let proposed = vec![
            VisionSegmentItem {
                rect: ScreenRect { x: 104, y: 102, w: 118, h: 28 },
                label: "误改的框".to_string(),
                role: "Button".to_string(),
                semantic_kind: "ui_vlm_fixed".to_string(),
                polygon: None,
            },
            VisionSegmentItem {
                rect: ScreenRect { x: 100, y: 300, w: 90, h: 26 },
                label: "漏掉的设置按钮".to_string(),
                role: "Button".to_string(),
                semantic_kind: "ui_vlm_fixed".to_string(),
                polygon: None,
            },
        ];
        let (safe_drops, safe_adds) = filter_audit_patch_additively(&existing, vec![0, 1], proposed);
        assert_eq!(safe_drops, vec![1], "verified UIA box #0 must NEVER be dropped");
        assert_eq!(safe_adds.len(), 1, "overlapping replacement over protected UIA box must be rejected");
        assert_eq!(safe_adds[0].label, "漏掉的设置按钮");
    }

    #[test]
    fn florence2_photo_detector_never_overwrites_ui_boxes_during_ui_audit() {
        let mut cfg = VisionConfig::default();
        cfg.apply_preset(VisionHardwarePreset::BalancedUniversal);
        assert_eq!(cfg.vlm_backend, VlmBackend::SidecarFlorence2);
        let ui_img = RgbaImage::from_pixel(180, 140, Rgba([28, 30, 34, 255]));
        let existing = vec![(
            0usize,
            ScreenRect { x: 10, y: 10, w: 80, h: 24 },
            "保存".to_string(),
            true,
        )];
        let res = audit_ui_region(&cfg, &ui_img, 0, 0, &existing).unwrap();
        assert_eq!(res.verdict, "ok");
        assert!(res.drop_ids.is_empty());
        assert!(res.corrected_items.is_empty());
        assert_eq!(res.backend_used, "local_additive_guard");
    }
}
