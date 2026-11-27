use crate::config::{VisionConfig, VlmBackend};
use anyhow::{anyhow, Context, Result};
use base64::Engine;
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, RgbaImage};
use std::time::Duration;

const CLOUD_TIMEOUT_SECS: u64 = 18;

/// Downscales (if larger than `max_dim`) and JPEG-encodes an RGBA crop into base64,
/// returning `(base64_jpeg, encoded_width, encoded_height)` so callers can accurately scale
/// inbound/outbound coordinates between original crop pixels and resized model pixels.
pub fn encode_rgba_to_jpeg_base64_with_size(
    img: &RgbaImage,
    max_dim: u32,
) -> Result<(String, u32, u32)> {
    let dyn_img = DynamicImage::ImageRgba8(img.clone());
    let resized = if img.width().max(img.height()) > max_dim {
        dyn_img.thumbnail(max_dim, max_dim)
    } else {
        dyn_img
    };
    let rgb = resized.to_rgb8();
    let (ew, eh) = (rgb.width(), rgb.height());
    let mut buf = Vec::with_capacity(32 * 1024);
    let mut enc = JpegEncoder::new_with_quality(&mut buf, 82);
    enc.encode(
        rgb.as_raw(),
        ew,
        eh,
        image::ExtendedColorType::Rgb8,
    )
    .context("jpeg encode failed")?;
    Ok((base64::engine::general_purpose::STANDARD.encode(buf), ew, eh))
}

/// Downscales (if larger than `max_dim`) and JPEG-encodes an RGBA crop into base64 for fast VLM calls.
pub fn encode_rgba_to_jpeg_base64(img: &RgbaImage, max_dim: u32) -> Result<String> {
    encode_rgba_to_jpeg_base64_with_size(img, max_dim).map(|(b64, _, _)| b64)
}

/// Extracts the first valid JSON object from a VLM response (tolerates ```json ... ``` wrappers).
pub fn parse_vlm_json_object(raw: &str) -> Result<serde_json::Value> {
    let trimmed = raw.trim();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return Ok(v);
    }
    // Strip markdown fences if present
    let stripped = trimmed
        .trim_start_matches("```json")
        .trim_start_matches("```JSON")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(stripped) {
        return Ok(v);
    }
    let start = trimmed
        .find('{')
        .ok_or_else(|| anyhow!("no JSON object found in VLM response"))?;
    let end = trimmed
        .rfind('}')
        .ok_or_else(|| anyhow!("unclosed JSON object in VLM response"))?;
    if end <= start {
        return Err(anyhow!("malformed JSON bounds in VLM response"));
    }
    serde_json::from_str::<serde_json::Value>(&trimmed[start..=end])
        .context("failed to parse extracted JSON from VLM response")
}

pub fn call_cloud_vlm_json(
    cfg: &VisionConfig,
    prompt: &str,
    images_b64: &[String],
) -> Result<serde_json::Value> {
    let api_key = cfg.cloud_api_key.trim();
    if api_key.is_empty() {
        return Err(anyhow!("云端 VLM API Key 未配置"));
    }
    match cfg.vlm_backend {
        VlmBackend::CloudGemini => call_gemini_json(cfg, api_key, prompt, images_b64),
        _ => call_openai_compat_json(cfg, api_key, prompt, images_b64),
    }
}

fn call_openai_compat_json(
    cfg: &VisionConfig,
    api_key: &str,
    prompt: &str,
    images_b64: &[String],
) -> Result<serde_json::Value> {
    let base = cfg.cloud_base_url.trim().trim_end_matches('/');
    let url = if base.ends_with("/chat/completions") {
        base.to_string()
    } else if base.is_empty() {
        "https://api.openai.com/v1/chat/completions".to_string()
    } else {
        format!("{base}/chat/completions")
    };
    let model = if cfg.cloud_model.trim().is_empty() {
        "gpt-4o-mini"
    } else {
        cfg.cloud_model.trim()
    };

    let mut content = vec![serde_json::json!({ "type": "text", "text": prompt })];
    for b64 in images_b64 {
        content.push(serde_json::json!({
            "type": "image_url",
            "image_url": {
                "url": format!("data:image/jpeg;base64,{b64}"),
                "detail": "high"
            }
        }));
    }

    let body = serde_json::json!({
        "model": model,
        "temperature": 0.1,
        "max_tokens": 1400,
        "messages": [
            {
                "role": "user",
                "content": content
            }
        ]
    });

    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(CLOUD_TIMEOUT_SECS))
        .build();
    let resp = agent
        .post(&url)
        .set("Authorization", &format!("Bearer {api_key}"))
        .set("Content-Type", "application/json")
        .send_json(body)
        .map_err(|e| anyhow!("云端 OpenAI 兼容 VLM 请求失败: {e}"))?;

    let json: serde_json::Value = resp
        .into_json()
        .context("云端 VLM 返回体不是有效 JSON")?;
    let text = json
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("云端 VLM 响应缺少 choices[0].message.content"))?;
    parse_vlm_json_object(text)
}

fn call_gemini_json(
    cfg: &VisionConfig,
    api_key: &str,
    prompt: &str,
    images_b64: &[String],
) -> Result<serde_json::Value> {
    let base = cfg.cloud_base_url.trim().trim_end_matches('/');
    let api_root = if base.is_empty() || base.contains("openai.com") {
        "https://generativelanguage.googleapis.com/v1beta"
    } else {
        base
    };
    let model = if cfg.cloud_model.trim().is_empty() || cfg.cloud_model.contains("gpt-") {
        "gemini-2.5-flash"
    } else {
        cfg.cloud_model.trim()
    };
    let url = format!("{api_root}/models/{model}:generateContent?key={api_key}");

    let mut parts = vec![serde_json::json!({ "text": prompt })];
    for b64 in images_b64 {
        parts.push(serde_json::json!({
            "inline_data": {
                "mime_type": "image/jpeg",
                "data": b64
            }
        }));
    }

    let body = serde_json::json!({
        "contents": [{ "parts": parts }],
        "generationConfig": {
            "temperature": 0.1,
            "responseMimeType": "application/json"
        }
    });

    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(CLOUD_TIMEOUT_SECS))
        .build();
    let resp = agent
        .post(&url)
        .set("Content-Type", "application/json")
        .send_json(body)
        .map_err(|e| anyhow!("Gemini VLM 请求失败: {e}"))?;

    let json: serde_json::Value = resp.into_json().context("Gemini 返回体不是有效 JSON")?;
    let text = json
        .pointer("/candidates/0/content/parts/0/text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Gemini 响应缺少 candidates[0].content.parts[0].text"))?;
    parse_vlm_json_object(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fenced_and_prose_wrapped_vlm_json() {
        let fenced = "```json\n{\"verdict\":\"fix_needed\",\"drop_ids\":[1]}\n```";
        let v = parse_vlm_json_object(fenced).unwrap();
        assert_eq!(v["verdict"], "fix_needed");
        assert_eq!(v["drop_ids"][0], 1);

        let prose = "Here is the analysis:\n{\"objects\":[{\"label\":\"草地\",\"bbox\":[600,0,980,1000]}]}\nDone.";
        let v2 = parse_vlm_json_object(prose).unwrap();
        assert_eq!(v2["objects"][0]["label"], "草地");
    }

    #[test]
    fn encode_with_size_preserves_exact_downscale_ratio_for_wide_ribbons() {
        let img = RgbaImage::new(2560, 400);
        let (b64, sw, sh) = encode_rgba_to_jpeg_base64_with_size(&img, 1280).expect("encode");
        assert!(!b64.is_empty());
        assert_eq!(sw, 1280);
        assert_eq!(sh, 200);
        let scale_x = (img.width() as f64) / (sw as f64);
        let scale_y = (img.height() as f64) / (sh as f64);
        assert!((scale_x - 2.0).abs() < 1e-6);
        assert!((scale_y - 2.0).abs() < 1e-6);
    }
}
