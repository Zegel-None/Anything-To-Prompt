use crate::captures::ScreenRect;
use crate::config::{AppConfig, SegmentationBackend, VisionConfig, VisionHardwarePreset, VlmBackend};
use crate::pick::hitmap::VisualBoxProvider;
use crate::pick::vision::cloud_vlm::encode_rgba_to_jpeg_base64_with_size;
use anyhow::{anyhow, Context, Result};
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

static SIDECAR_CHILD: OnceLock<Mutex<Option<Child>>> = OnceLock::new();

fn child_slot() -> &'static Mutex<Option<Child>> {
    SIDECAR_CHILD.get_or_init(|| Mutex::new(None))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarHealth {
    pub ok: bool,
    pub device: String,
    pub cuda_available: bool,
    pub loaded_models: Vec<String>,
    pub available_backends: Vec<String>,
    pub vram_allocated_mb: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarSegmentOut {
    pub label: String,
    #[serde(default)]
    pub score: f32,
    /// Local crop pixel rectangle [x, y, w, h]
    pub bbox: [i32; 4],
    /// Local crop polygon vertices [[x, y], ...]
    #[serde(default)]
    pub polygon: Vec<[i32; 2]>,
}

pub fn resolve_sidecar_python(cfg: &VisionConfig) -> Option<PathBuf> {
    if let Some(custom) = cfg.sidecar_python_path.as_deref() {
        let p = PathBuf::from(custom.trim());
        if p.is_file() {
            return Some(p);
        }
    }
    let venv_py = AppConfig::vision_env_dir().join("Scripts").join("python.exe");
    if venv_py.is_file() {
        return Some(venv_py);
    }
    None
}

pub fn resolve_sidecar_script() -> Option<PathBuf> {
    let deployed = AppConfig::vision_sidecar_dir().join("server.py");
    if deployed.is_file() {
        return Some(deployed);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join("vision-sidecar").join("server.py");
            if sibling.is_file() {
                return Some(sibling);
            }
            // Dev workspace fallback: target/release/anything-to-prompt.exe -> apps/vision-sidecar/server.py
            for ancestor in dir.ancestors().take(6) {
                let candidate = ancestor.join("apps").join("vision-sidecar").join("server.py");
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

pub fn sidecar_base_url(cfg: &VisionConfig) -> String {
    let port = if cfg.sidecar_port < 1024 {
        18765
    } else {
        cfg.sidecar_port
    };
    format!("http://127.0.0.1:{port}")
}

pub fn health(cfg: &VisionConfig) -> Result<SidecarHealth> {
    let url = format!("{}/health", sidecar_base_url(cfg));
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_millis(1200))
        .build();
    let resp = agent
        .get(&url)
        .call()
        .map_err(|e| anyhow!("Vision Sidecar 未运行: {e}"))?;
    resp.into_json::<SidecarHealth>()
        .context("Sidecar /health JSON 解析失败")
}

pub fn ensure_sidecar_running(cfg: &VisionConfig) -> Result<SidecarHealth> {
    if let Ok(h) = health(cfg) {
        if h.ok {
            return Ok(h);
        }
    }
    let py = resolve_sidecar_python(cfg)
        .ok_or_else(|| anyhow!("未找到本地视觉虚拟环境 (%LOCALAPPDATA%\\AnythingToPrompt\\vision-env)"))?;
    let script = resolve_sidecar_script()
        .ok_or_else(|| anyhow!("未找到本地视觉服务脚本 (vision-sidecar/server.py)"))?;
    let models_dir = AppConfig::vision_models_dir();
    let _ = std::fs::create_dir_all(&models_dir);

    let port = if cfg.sidecar_port < 1024 {
        18765
    } else {
        cfg.sidecar_port
    };

    if let Ok(mut guard) = child_slot().lock() {
        if let Some(ch) = guard.as_mut() {
            if matches!(ch.try_wait(), Ok(Some(_))) {
                *guard = None;
            }
        }
        if guard.is_none() {
            let child = Command::new(&py)
                .arg(&script)
                .arg("--port")
                .arg(port.to_string())
                .arg("--models-dir")
                .arg(&models_dir)
                .arg("--idle-unload")
                .arg(cfg.idle_unload_seconds.to_string())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
                .with_context(|| format!("启动 Vision Sidecar 失败: {}", py.display()))?;
            *guard = Some(child);
        }
    }

    let deadline = Instant::now() + Duration::from_secs(12);
    while Instant::now() < deadline {
        if let Ok(h) = health(cfg) {
            if h.ok {
                return Ok(h);
            }
        }
        std::thread::sleep(Duration::from_millis(180));
    }
    Err(anyhow!("等待本地 Vision Sidecar 启动超时 (12s)"))
}

pub fn backend_str(b: SegmentationBackend) -> &'static str {
    match b {
        SegmentationBackend::Disabled => "disabled",
        SegmentationBackend::OnnxMobileSamCpu => "onnx_mobilesam_cpu",
        SegmentationBackend::GpuSam2Tiny => "gpu_sam2_tiny",
        SegmentationBackend::GpuSam2Small => "gpu_sam2_small",
    }
}

pub fn segment_boxes_to_polygons(
    cfg: &VisionConfig,
    image_b64: &str,
    boxes: &[(String, [i32; 4])],
    brush_points: &[[i32; 2]],
) -> Result<Vec<SidecarSegmentOut>> {
    let _ = ensure_sidecar_running(cfg)?;
    let url = format!("{}/segment/boxes", sidecar_base_url(cfg));
    let body = serde_json::json!({
        "backend": backend_str(cfg.segmentation_backend),
        "imageB64": image_b64,
        "boxes": boxes.iter().map(|(label, b)| serde_json::json!({
            "label": label,
            "bbox": b,
        })).collect::<Vec<_>>(),
        "points": brush_points,
    });
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .build();
    let resp = agent
        .post(&url)
        .set("Content-Type", "application/json")
        .send_json(body)
        .map_err(|e| anyhow!("Sidecar /segment/boxes 失败: {e}"))?;
    let json: serde_json::Value = resp.into_json().context("Sidecar 响应解析失败")?;
    let items: Vec<SidecarSegmentOut> = serde_json::from_value(
        json.get("segments")
            .cloned()
            .unwrap_or_else(|| serde_json::json!([])),
    )
    .unwrap_or_default();
    Ok(items)
}

pub fn auto_semantic_segment(
    cfg: &VisionConfig,
    image_b64: &str,
    brush_points: &[[i32; 2]],
    task_mode: &str, // "photo" | "ui"
) -> Result<Vec<SidecarSegmentOut>> {
    let _ = ensure_sidecar_running(cfg)?;
    let url = format!("{}/segment/auto_semantic", sidecar_base_url(cfg));
    let body = serde_json::json!({
        "backend": backend_str(cfg.segmentation_backend),
        "imageB64": image_b64,
        "points": brush_points,
        "mode": task_mode,
    });
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(45))
        .build();
    let resp = agent
        .post(&url)
        .set("Content-Type", "application/json")
        .send_json(body)
        .map_err(|e| anyhow!("Sidecar /segment/auto_semantic 失败: {e}"))?;
    let json: serde_json::Value = resp.into_json().context("Sidecar 语义分割响应解析失败")?;
    let items: Vec<SidecarSegmentOut> = serde_json::from_value(
        json.get("segments")
            .cloned()
            .unwrap_or_else(|| serde_json::json!([])),
    )
    .unwrap_or_default();
    Ok(items)
}

pub fn detect_ui_icons(
    cfg: &VisionConfig,
    image_b64: &str,
    origin_x: i32,
    origin_y: i32,
    scale_x: f64,
    scale_y: f64,
    conf: f32,
) -> Result<Vec<ScreenRect>> {
    let _ = ensure_sidecar_running(cfg)?;
    let url = format!("{}/detect/ui_icons", sidecar_base_url(cfg));
    let body = serde_json::json!({
        "imageB64": image_b64,
        "conf": conf,
        "iou": 0.45,
    });
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .build();
    let resp = agent
        .post(&url)
        .set("Content-Type", "application/json")
        .send_json(body)
        .map_err(|e| anyhow!("Sidecar /detect/ui_icons 失败: {e}"))?;
    let json: serde_json::Value = resp.into_json().context("OmniParser 响应解析失败")?;
    let mut out = Vec::new();
    if let Some(arr) = json.get("boxes").and_then(|v| v.as_array()) {
        for item in arr {
            let Some(b) = item.get("bbox").and_then(|v| v.as_array()) else {
                continue;
            };
            if b.len() != 4 {
                continue;
            }
            let r = ScreenRect {
                x: origin_x + ((b[0].as_f64().unwrap_or(0.0) * scale_x).round() as i32),
                y: origin_y + ((b[1].as_f64().unwrap_or(0.0) * scale_y).round() as i32),
                w: (b[2].as_f64().unwrap_or(0.0).max(0.0) * scale_x).round() as i32,
                h: (b[3].as_f64().unwrap_or(0.0).max(0.0) * scale_y).round() as i32,
            };
            if r.w >= 12 && r.h >= 12 {
                out.push(r);
            }
        }
    }
    Ok(out)
}

/// Microsoft OmniParser v2 (`icon_detect.pt`) implementation of `VisualBoxProvider`.
/// Runs via the lazy-started local Vision Sidecar (~20-40ms on GPU, ~120ms CPU) with zero hardcoded coordinates.
pub struct OmniParserBoxProvider;

impl VisualBoxProvider for OmniParserBoxProvider {
    fn name(&self) -> &'static str {
        "OmniParser-v2"
    }

    fn detect_boxes(&self, image: &RgbaImage, origin_x: i32, origin_y: i32) -> Vec<ScreenRect> {
        let weight = AppConfig::vision_models_dir()
            .join("omniparser")
            .join("icon_detect.pt");
        if !weight.is_file() {
            return Vec::new();
        }
        let cfg = AppConfig::load_or_create().vision;
        if cfg.segmentation_backend == SegmentationBackend::Disabled
            && cfg.vlm_backend == VlmBackend::Disabled
        {
            return Vec::new();
        }
        let Ok((b64, sent_w, sent_h)) = encode_rgba_to_jpeg_base64_with_size(image, 1280) else {
            return Vec::new();
        };
        let scale_x = (image.width() as f64) / (sent_w.max(1) as f64);
        let scale_y = (image.height() as f64) / (sent_h.max(1) as f64);
        detect_ui_icons(&cfg, &b64, origin_x, origin_y, scale_x, scale_y, 0.14).unwrap_or_default()
    }
}

/// Returns `Some(OmniParserBoxProvider)` if `icon_detect.pt` and the local vision sidecar are installed
/// and the user has not disabled local vision models.
pub fn maybe_build_omniparser_provider() -> Option<Arc<dyn VisualBoxProvider>> {
    let weight = AppConfig::vision_models_dir()
        .join("omniparser")
        .join("icon_detect.pt");
    let cfg = AppConfig::load_or_create().vision;
    let enabled = !(cfg.segmentation_backend == SegmentationBackend::Disabled
        && cfg.vlm_backend == VlmBackend::Disabled);
    if enabled
        && weight.is_file()
        && resolve_sidecar_python(&cfg).is_some()
        && resolve_sidecar_script().is_some()
    {
        Some(Arc::new(OmniParserBoxProvider))
    } else {
        None
    }
}

/// Returns `true` when Scheme B automatic background OmniParser pre-warming is permitted
/// (requires a GPU-capable vision preset, not `LiteOffice` CPU-only mode or `Disabled`).
pub fn is_gpu_auto_wake_permitted(cfg: &VisionConfig) -> bool {
    cfg.preset != VisionHardwarePreset::LiteOffice
        && !(cfg.segmentation_backend == SegmentationBackend::Disabled
            && cfg.vlm_backend == VlmBackend::Disabled)
        && matches!(
            cfg.segmentation_backend,
            SegmentationBackend::GpuSam2Tiny | SegmentationBackend::GpuSam2Small
        )
}

pub fn unload_models(cfg: &VisionConfig) -> Result<()> {
    let url = format!("{}/unload", sidecar_base_url(cfg));
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(4))
        .build();
    let _ = agent.post(&url).send_json(serde_json::json!({}));
    Ok(())
}

pub fn shutdown_sidecar() {
    if let Ok(mut guard) = child_slot().lock() {
        if let Some(mut ch) = guard.take() {
            let _ = ch.kill();
            let _ = ch.wait();
        }
    }
}
