use crate::config::VisionConfig;
use crate::pick::vision::cloud_vlm::parse_vlm_json_object;
use anyhow::{anyhow, Context, Result};
use std::time::Duration;

const OLLAMA_PROBE_TIMEOUT_MS: u64 = 1800;
const OLLAMA_INFER_TIMEOUT_SECS: u64 = 35;

pub fn list_ollama_models(base_url: &str) -> Result<Vec<String>> {
    let root = base_url.trim().trim_end_matches('/');
    let url = if root.is_empty() {
        "http://127.0.0.1:11434/api/tags".to_string()
    } else {
        format!("{root}/api/tags")
    };
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_millis(OLLAMA_PROBE_TIMEOUT_MS))
        .build();
    let resp = agent
        .get(&url)
        .call()
        .map_err(|e| anyhow!("Ollama 未响应 ({url}): {e}"))?;
    let json: serde_json::Value = resp.into_json().context("Ollama /api/tags 解析失败")?;
    let models = json
        .get("models")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("name").and_then(|n| n.as_str()).map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(models)
}

pub fn call_ollama_vlm_json(
    cfg: &VisionConfig,
    prompt: &str,
    images_b64: &[String],
) -> Result<serde_json::Value> {
    let root = cfg.ollama_base_url.trim().trim_end_matches('/');
    let url = if root.is_empty() {
        "http://127.0.0.1:11434/api/chat".to_string()
    } else {
        format!("{root}/api/chat")
    };
    let model = if cfg.ollama_model.trim().is_empty() {
        "qwen2.5vl:7b"
    } else {
        cfg.ollama_model.trim()
    };

    let body = serde_json::json!({
        "model": model,
        "stream": false,
        "format": "json",
        "keep_alive": format!("{}s", cfg.idle_unload_seconds.clamp(30, 1800)),
        "options": {
            "temperature": 0.1
        },
        "messages": [
            {
                "role": "user",
                "content": prompt,
                "images": images_b64
            }
        ]
    });

    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(OLLAMA_INFER_TIMEOUT_SECS))
        .build();
    let resp = agent
        .post(&url)
        .set("Content-Type", "application/json")
        .send_json(body)
        .map_err(|e| anyhow!("本机 Ollama VLM 推理失败 ({model}): {e}"))?;
    let json: serde_json::Value = resp.into_json().context("Ollama /api/chat 响应无效")?;
    let text = json
        .pointer("/message/content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Ollama 响应缺少 message.content"))?;
    parse_vlm_json_object(text)
}

pub fn unload_ollama_model(cfg: &VisionConfig) -> Result<()> {
    let root = cfg.ollama_base_url.trim().trim_end_matches('/');
    let url = if root.is_empty() {
        "http://127.0.0.1:11434/api/chat".to_string()
    } else {
        format!("{root}/api/chat")
    };
    let model = cfg.ollama_model.trim();
    if model.is_empty() {
        return Ok(());
    }
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(3))
        .build();
    let _ = agent.post(&url).send_json(serde_json::json!({
        "model": model,
        "keep_alive": 0,
        "messages": []
    }));
    Ok(())
}
