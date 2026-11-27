use super::*;

pub fn load_external_messages(harness: HarnessKind, session_id: &str) -> Vec<ExternalChatMessage> {
    if session_id.trim().is_empty() {
        return Vec::new();
    }
    match harness {
        HarnessKind::Dsh => Vec::new(),
        HarnessKind::Antigravity => load_antigravity_transcript(session_id),
        HarnessKind::Codex => load_codex_transcript(session_id),
        HarnessKind::ClaudeCode => load_claude_code_transcript(session_id),
    }
}

pub(super) fn strip_antigravity_user_wrappers(raw: &str) -> String {
    let mut text = raw.trim().to_string();
    if let (Some(start), Some(end)) = (text.find("<USER_REQUEST>"), text.find("</USER_REQUEST>")) {
        let inner = &text[start + "<USER_REQUEST>".len()..end];
        text = inner.trim().to_string();
    }
    text
}

pub fn parse_antigravity_transcript_lines(raw: &str) -> Vec<ExternalChatMessage> {
    let mut out = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        let step_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let content = v
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .trim();
        if content.is_empty() {
            continue;
        }
        match step_type {
            "USER_INPUT" => {
                let cleaned = strip_antigravity_user_wrappers(content);
                if !cleaned.is_empty() {
                    out.push(ExternalChatMessage {
                        id: format!("ag-u-{idx}"),
                        role: "user".to_string(),
                        text: cleaned,
                        streaming: false,
                    });
                }
            }
            "SYSTEM_MESSAGE" => {
                // Surface silent messages delivered by Anything-to-Prompt via official agentapi
                if content.contains("Anything-to-Prompt") || content.contains("[Message]") {
                    if let Some(idx_content) = content.find("content=") {
                        let body = content[idx_content + "content=".len()..]
                            .trim()
                            .trim_end_matches("</SYSTEM_MESSAGE>")
                            .trim();
                        if !body.is_empty()
                            && !body.starts_with("[BROWSER_HOST_FACTS")
                            && !body.starts_with("Task id ")
                        {
                            out.push(ExternalChatMessage {
                                id: format!("ag-su-{idx}"),
                                role: "user".to_string(),
                                text: body.to_string(),
                                streaming: false,
                            });
                        }
                    }
                }
            }
            "PLANNER_RESPONSE" => {
                out.push(ExternalChatMessage {
                    id: format!("ag-a-{idx}"),
                    role: "assistant".to_string(),
                    text: content.to_string(),
                    streaming: false,
                });
            }
            _ => {}
        }
    }
    if out.len() > 40 {
        out.drain(0..out.len() - 40);
    }
    out
}

pub(super) fn load_antigravity_transcript(session_id: &str) -> Vec<ExternalChatMessage> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let candidates = [
        home.join(".gemini")
            .join("antigravity")
            .join("brain")
            .join(session_id)
            .join(".system_generated")
            .join("logs")
            .join("transcript.jsonl"),
        home.join(".gemini")
            .join("antigravity-cli")
            .join("brain")
            .join(session_id)
            .join(".system_generated")
            .join("logs")
            .join("transcript.jsonl"),
    ];
    for path in candidates {
        if let Ok(raw) = fs::read_to_string(&path) {
            return parse_antigravity_transcript_lines(&raw);
        }
    }
    Vec::new()
}

pub fn parse_codex_rollout_lines(raw: &str) -> Vec<ExternalChatMessage> {
    let mut out = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("response_item") {
            continue;
        }
        let Some(payload) = v.get("payload") else {
            continue;
        };
        if payload.get("type").and_then(|t| t.as_str()) != Some("message") {
            continue;
        }
        let role = payload.get("role").and_then(|r| r.as_str()).unwrap_or("");
        if role != "user" && role != "assistant" {
            continue;
        }
        let mut text_parts = Vec::new();
        if let Some(arr) = payload.get("content").and_then(|c| c.as_array()) {
            for item in arr {
                if let Some(t) = item.get("text").and_then(|s| s.as_str()) {
                    if !t.trim().is_empty() {
                        text_parts.push(t.trim());
                    }
                }
            }
        }
        let text = text_parts.join("\n").trim().to_string();
        if text.is_empty() {
            continue;
        }
        let id = payload
            .get("id")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("cx-{role}-{idx}"));
        out.push(ExternalChatMessage {
            id,
            role: role.to_string(),
            text,
            streaming: false,
        });
    }
    if out.len() > 40 {
        out.drain(0..out.len() - 40);
    }
    out
}

pub(super) fn load_codex_transcript(session_id: &str) -> Vec<ExternalChatMessage> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let db_path = home.join(".codex").join("state_5.sqlite");
    let Some(conn) = open_sqlite_readonly(&db_path) else {
        return Vec::new();
    };
    let rollout_path: Option<String> = conn
        .query_row(
            "SELECT rollout_path FROM threads WHERE id = ?1 LIMIT 1",
            [session_id],
            |row| row.get(0),
        )
        .ok();
    let Some(path_str) = rollout_path else {
        return Vec::new();
    };
    let cleaned = clean_windows_path(&path_str);
    if let Ok(raw) = fs::read_to_string(&cleaned) {
        return parse_codex_rollout_lines(&raw);
    }
    Vec::new()
}

pub fn parse_claude_code_transcript_lines(raw: &str) -> Vec<ExternalChatMessage> {
    let mut out: Vec<ExternalChatMessage> = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        if v.get("isSidechain").and_then(|b| b.as_bool()) == Some(true)
            || v.get("isMeta").and_then(|b| b.as_bool()) == Some(true)
            || v.get("isApiErrorMessage").and_then(|b| b.as_bool()) == Some(true)
        {
            continue;
        }
        let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if kind != "user" && kind != "assistant" {
            continue;
        }
        let Some(msg) = v.get("message") else {
            continue;
        };
        let Some(content) = msg.get("content") else {
            continue;
        };
        let text = extract_claude_content_text(content);
        if text.is_empty() {
            continue;
        }
        if kind == "user" && is_claude_internal_user_text(&text) {
            continue;
        }
        // Merge consecutive assistant text blocks from the same assistant message id
        if kind == "assistant" {
            if let Some(last) = out.last_mut() {
                if last.role == "assistant" {
                    if let Some(msg_id) = msg.get("id").and_then(|s| s.as_str()) {
                        if !msg_id.is_empty() && last.id == format!("cc-{msg_id}") {
                            if !last.text.ends_with(&text) {
                                last.text.push_str("\n\n");
                                last.text.push_str(&text);
                            }
                            continue;
                        }
                    }
                }
            }
        }
        let id = msg
            .get("id")
            .and_then(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| format!("cc-{s}"))
            .or_else(|| {
                v.get("uuid")
                    .and_then(|s| s.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| format!("cc-{s}"))
            })
            .unwrap_or_else(|| format!("cc-{kind}-{idx}"));
        out.push(ExternalChatMessage {
            id,
            role: kind.to_string(),
            text,
            streaming: false,
        });
    }
    if out.len() > 40 {
        out.drain(0..out.len() - 40);
    }
    out
}

pub(super) fn load_claude_code_transcript(session_id: &str) -> Vec<ExternalChatMessage> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let projects_root = home.join(".claude").join("projects");
    let target_name = format!("{}.jsonl", session_id.trim());
    let Ok(proj_dirs) = fs::read_dir(&projects_root) else {
        return Vec::new();
    };
    for proj in proj_dirs.flatten() {
        let candidate = proj.path().join(&target_name);
        if candidate.is_file() {
            if let Ok(raw) = fs::read_to_string(&candidate) {
                return parse_claude_code_transcript_lines(&raw);
            }
        }
    }
    Vec::new()
}

pub(super) fn emit_stream_event(writer: &mut impl Write, evt: &HarnessStreamEvent) {
    if let Ok(line) = serde_json::to_string(evt) {
        let _ = writeln!(writer, "{line}");
        let _ = writer.flush();
    }
}

pub fn parse_claude_session_stats_lines(session_id: &str, raw: &str) -> HarnessSessionStats {
    let mut model = String::new();
    let mut turns = 0usize;
    let mut input_tokens = 0u64;
    let mut output_tokens = 0u64;
    let mut cache_read_tokens = 0u64;
    let mut cache_creation_tokens = 0u64;
    let mut seen_msg_ids = HashSet::new();

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
            continue;
        }
        let Some(msg) = v.get("message") else {
            continue;
        };
        if let Some(m) = msg.get("model").and_then(|s| s.as_str()) {
            if !m.trim().is_empty() {
                model = m.trim().to_string();
            }
        }
        if let Some(mid) = msg.get("id").and_then(|s| s.as_str()) {
            if !mid.is_empty() && !seen_msg_ids.insert(mid.to_string()) {
                continue;
            }
        }
        turns += 1;
        if let Some(usage) = msg.get("usage") {
            input_tokens += usage.get("input_tokens").and_then(|n| n.as_u64()).unwrap_or(0);
            output_tokens += usage.get("output_tokens").and_then(|n| n.as_u64()).unwrap_or(0);
            cache_read_tokens += usage
                .get("cache_read_input_tokens")
                .and_then(|n| n.as_u64())
                .unwrap_or(0);
            cache_creation_tokens += usage
                .get("cache_creation_input_tokens")
                .and_then(|n| n.as_u64())
                .unwrap_or(0);
        }
    }

    let total_tokens = input_tokens + output_tokens + cache_read_tokens + cache_creation_tokens;
    let model_disp = if model.is_empty() {
        "Claude Code".to_string()
    } else {
        format_claude_model_label(&model)
    };
    let summary_markdown = format!(
        "### 📊 Claude Code 会话用量（`/cost`）\n\n\
        - **模型**：`{model_disp}`\n\
        - **对话轮次**：`{turns}` 轮\n\
        - **输入 / 输出 Tokens**：`{input_tokens}` in / `{output_tokens}` out\n\
        - **缓存读取 / 写入**：`{cache_read_tokens}` read / `{cache_creation_tokens}` write\n\
        - **合计 Tokens**：**`{total_tokens}`**"
    );

    HarnessSessionStats {
        harness: "claude_code".to_string(),
        session_id: session_id.to_string(),
        model: model_disp,
        turns,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_creation_tokens,
        total_tokens,
        summary_markdown,
    }
}

pub fn load_session_stats(kind: HarnessKind, session_id: &str) -> HarnessSessionStats {
    let sid = session_id.trim();
    let empty = |h: &str, msg: &str| HarnessSessionStats {
        harness: h.to_string(),
        session_id: sid.to_string(),
        model: String::new(),
        turns: 0,
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        cache_creation_tokens: 0,
        total_tokens: 0,
        summary_markdown: msg.to_string(),
    };
    if sid.is_empty() {
        return empty(
            kind.as_str(),
            "当前为全新对话，尚未产生会话 Token 消耗记录。",
        );
    }
    let Some(home) = dirs::home_dir() else {
        return empty(kind.as_str(), "无法定位本地会话存储目录。");
    };
    match kind {
        HarnessKind::ClaudeCode => {
            let projects_root = home.join(".claude").join("projects");
            let target_name = format!("{sid}.jsonl");
            if let Ok(proj_dirs) = fs::read_dir(&projects_root) {
                for proj in proj_dirs.flatten() {
                    let candidate = proj.path().join(&target_name);
                    if candidate.is_file() {
                        if let Ok(raw) = fs::read_to_string(&candidate) {
                            return parse_claude_session_stats_lines(sid, &raw);
                        }
                    }
                }
            }
            empty("claude_code", "未找到该 Claude Code 会话的本地 JSONL 用量日志。")
        }
        HarnessKind::Codex => {
            let db_path = home.join(".codex").join("state_5.sqlite");
            if let Some(conn) = open_sqlite_readonly(&db_path) {
                if let Ok((tokens, provider)) = conn.query_row(
                    "SELECT COALESCE(tokens_used, 0), COALESCE(model_provider, 'openai') FROM threads WHERE id = ?1 LIMIT 1",
                    [sid],
                    |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
                ) {
                    let total = tokens.max(0) as u64;
                    let turns = load_codex_transcript(sid)
                        .into_iter()
                        .filter(|m| m.role == "assistant")
                        .count();
                    return HarnessSessionStats {
                        harness: "codex".to_string(),
                        session_id: sid.to_string(),
                        model: provider.clone(),
                        turns,
                        input_tokens: 0,
                        output_tokens: 0,
                        cache_read_tokens: 0,
                        cache_creation_tokens: 0,
                        total_tokens: total,
                        summary_markdown: format!(
                            "### 📊 Codex 会话用量\n\n- **Provider**：`{provider}`\n- **回复轮次**：`{turns}` 轮\n- **累计 Tokens**：**`{total}`**"
                        ),
                    };
                }
            }
            empty("codex", "未找到该 Codex 会话的本地统计记录。")
        }
        HarnessKind::Antigravity => {
            let msgs = load_antigravity_transcript(sid);
            let turns = msgs.iter().filter(|m| m.role == "assistant").count();
            HarnessSessionStats {
                harness: "antigravity".to_string(),
                session_id: sid.to_string(),
                model: "Antigravity".to_string(),
                turns,
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                total_tokens: 0,
                summary_markdown: format!(
                    "### 📊 Antigravity 会话统计\n\n- **会话 ID**：`{sid}`\n- **回复轮次**：`{turns}` 轮"
                ),
            }
        }
        HarnessKind::Dsh => empty("dsh", "DeepSeek Harness 请直接在原对话界面查看统计。"),
    }
}

pub(super) fn read_file_tail_utf8(path: &Path, max_bytes: usize) -> Option<String> {
    use std::io::{Read as _, Seek as _, SeekFrom};
    let mut file = fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let take = (len as usize).min(max_bytes);
    if len > take as u64 {
        file.seek(SeekFrom::End(-(take as i64))).ok()?;
    }
    let mut buf = Vec::with_capacity(take);
    file.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf).into_owned();
    if len > take as u64 {
        if let Some(pos) = text.find('\n') {
            return Some(text[pos + 1..].to_string());
        }
    }
    Some(text)
}

pub fn parse_antigravity_model_and_effort(raw_model: &str) -> (Option<String>, Option<String>) {
    let trimmed = raw_model.trim();
    if trimmed.is_empty() {
        return (None, None);
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("gemini-pro-orchestrator") {
        return (Some(trimmed.to_string()), None);
    }
    if lower.starts_with("gpt-oss-") {
        let effort = if lower.ends_with("-low") {
            Some("low".to_string())
        } else if lower.ends_with("-high") {
            Some("high".to_string())
        } else {
            Some("medium".to_string())
        };
        return (Some(trimmed.to_string()), effort);
    }

    let (base, effort) = if let Some(b) = lower.strip_suffix("-xhigh") {
        (b, Some("xhigh".to_string()))
    } else if let Some(b) = lower.strip_suffix("-high") {
        (b, Some("high".to_string()))
    } else if let Some(b) = lower.strip_suffix("-medium") {
        (b, Some("medium".to_string()))
    } else if let Some(b) = lower.strip_suffix("-low") {
        (b, Some("low".to_string()))
    } else {
        (lower.as_str(), None)
    };

    // Map family base to canonical option ID used in harnesses/antigravity/ui.js
    let canonical = match base {
        "gemini-4-neon" => "gemini-4-neon-high".to_string(),
        "gemini-4-carbon" => "gemini-4-carbon-medium".to_string(),
        "gemini-4-argon" => "gemini-4-argon-medium".to_string(),
        "gemini-4-hydrogen" => "gemini-4-hydrogen-medium".to_string(),
        "gemini-3.8-flash" => "gemini-3.8-flash-medium".to_string(),
        "claude-opus-5-5" => "claude-opus-5-5-medium".to_string(),
        "claude-sonnet-5-5" => "claude-sonnet-5-5-medium".to_string(),
        _ => trimmed.to_string(),
    };
    (Some(canonical), effort)
}

pub fn parse_antigravity_permission_preset(pbtxt: &str) -> Option<String> {
    let pos = pbtxt.find("permission_preset:")?;
    let rest = pbtxt[pos + "permission_preset:".len()..].trim_start();
    let token: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    let upper = token.to_ascii_uppercase();
    if upper.is_empty() {
        return None;
    }
    if upper.contains("TURBO") {
        Some("turbo".to_string())
    } else if upper.contains("AUTO") {
        Some("auto".to_string())
    } else if upper.contains("REVIEW") || upper.contains("PLAN") {
        Some("request-review".to_string())
    } else if upper.contains("DEFAULT") || upper.contains("ASK") {
        Some("default".to_string())
    } else {
        None
    }
}

pub fn parse_antigravity_latest_model_from_transcript(raw_tail: &str) -> Option<String> {
    for line in raw_tail.lines().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() || !trimmed.contains("\"model\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        if let Some(m) = v.get("model").and_then(|s| s.as_str()) {
            let clean = m.trim();
            if !clean.is_empty() && clean != "<synthetic>" {
                return Some(clean.to_string());
            }
        }
    }
    None
}

pub(super) fn map_codex_permission_mode(
    approval: Option<&str>,
    sandbox: Option<&str>,
    reviewer: Option<&str>,
) -> Option<String> {
    let sb = sandbox.unwrap_or("").to_ascii_lowercase();
    let ap = approval.unwrap_or("").trim().to_ascii_lowercase();
    let rv = reviewer.unwrap_or("").trim().to_ascii_lowercase();
    if sb.contains("read-only") || sb.contains("readonly") {
        return Some("read-only".to_string());
    }
    if ap == "never"
        || sb.contains("danger-full-access")
        || sb.contains("dangerfullaccess")
        || sb.contains("\"disabled\"")
        || sb == "disabled"
    {
        return Some("danger-full-access".to_string());
    }
    if rv == "auto_review" || rv == "auto-review" {
        return Some("help-approve".to_string());
    }
    if ap == "on-request" || sb.contains("workspace-write") || sb.contains("workspacewrite") {
        return Some("request-approval".to_string());
    }
    None
}

pub fn parse_codex_session_state_from_turn_context(
    raw_tail: &str,
    db_model: Option<&str>,
    db_effort: Option<&str>,
    db_approval: Option<&str>,
    db_sandbox: Option<&str>,
) -> (Option<String>, Option<String>, Option<String>, Option<bool>) {
    let mut model = db_model
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let mut effort = db_effort
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty());
    let mut approval = db_approval
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let mut sandbox = db_sandbox
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let mut reviewer: Option<String> = None;
    let mut plan_mode: Option<bool> = None;

    for line in raw_tail.lines().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() || !trimmed.contains("\"turn_context\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("turn_context") {
            continue;
        }
        let Some(p) = v.get("payload") else {
            continue;
        };
        if let Some(m) = p.get("model").and_then(|s| s.as_str()).filter(|s| !s.trim().is_empty()) {
            model = Some(m.trim().to_string());
        }
        if let Some(e) = p
            .get("effort")
            .or_else(|| p.pointer("/collaboration_mode/settings/reasoning_effort"))
            .and_then(|s| s.as_str())
            .filter(|s| !s.trim().is_empty())
        {
            effort = Some(e.trim().to_ascii_lowercase());
        }
        if let Some(ap) = p.get("approval_policy").and_then(|s| s.as_str()).filter(|s| !s.trim().is_empty()) {
            approval = Some(ap.trim().to_string());
        }
        if let Some(rv) = p.get("approvals_reviewer").and_then(|s| s.as_str()).filter(|s| !s.trim().is_empty()) {
            reviewer = Some(rv.trim().to_string());
        }
        if let Some(sb_val) = p.get("sandbox_policy") {
            if let Some(s) = sb_val.as_str() {
                sandbox = Some(s.to_string());
            } else if let Some(t) = sb_val.get("type").and_then(|x| x.as_str()) {
                sandbox = Some(t.to_string());
            } else {
                sandbox = Some(sb_val.to_string());
            }
        }
        if let Some(cm) = p.pointer("/collaboration_mode/mode").and_then(|s| s.as_str()) {
            plan_mode = Some(cm.eq_ignore_ascii_case("plan"));
        }
        break;
    }

    let mode = map_codex_permission_mode(approval.as_deref(), sandbox.as_deref(), reviewer.as_deref());
    (model, effort, mode, plan_mode)
}

pub(super) fn normalize_claude_model_id(raw: &str) -> Option<String> {
    let clean = raw.split('[').next().unwrap_or(raw).trim();
    if clean.is_empty() || clean == "<synthetic>" {
        return None;
    }
    let parts: Vec<&str> = clean.split('-').filter(|p| !p.is_empty()).collect();
    if parts.len() >= 2 {
        if let Some(last) = parts.last() {
            if last.len() == 8 && last.chars().all(|c| c.is_ascii_digit()) {
                return Some(parts[..parts.len() - 1].join("-"));
            }
        }
    }
    Some(clean.to_string())
}

pub fn parse_claude_session_state_from_jsonl(raw_tail: &str) -> (Option<String>, Option<String>) {
    let mut model: Option<String> = None;
    let mut mode: Option<String> = None;
    for line in raw_tail.lines().rev() {
        if model.is_some() && mode.is_some() {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        if v.get("isSidechain").and_then(|b| b.as_bool()) == Some(true) {
            continue;
        }
        let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if mode.is_none() && kind == "user" {
            if let Some(pm) = v.get("permissionMode").and_then(|s| s.as_str()) {
                let mapped = match pm.trim() {
                    "default" | "manual" => Some("manual"),
                    "auto" => Some("auto"),
                    "acceptEdits" => Some("acceptEdits"),
                    "plan" => Some("plan"),
                    "dontAsk" => Some("dontAsk"),
                    "bypassPermissions" => Some("bypassPermissions"),
                    _ => None,
                };
                if let Some(m) = mapped {
                    mode = Some(m.to_string());
                }
            }
        }
        if model.is_none() && kind == "assistant" {
            if let Some(raw_m) = v.pointer("/message/model").and_then(|s| s.as_str()) {
                model = normalize_claude_model_id(raw_m);
            }
        }
    }
    (model, mode)
}

/// Strictly read-only per-conversation state lookup for the bound external Harness session.
/// Never writes to any Harness config, SQLite DB, or client state.
pub fn load_session_state(kind: HarnessKind, session_id: &str) -> HarnessSessionState {
    let sid = session_id.trim();
    let mut out = HarnessSessionState {
        harness: kind.as_str().to_string(),
        session_id: sid.to_string(),
        ..Default::default()
    };
    if sid.is_empty() || kind == HarnessKind::Dsh {
        return out;
    }
    let Some(home) = dirs::home_dir() else {
        return out;
    };

    match kind {
        HarnessKind::Codex => {
            let db_path = home.join(".codex").join("state_5.sqlite");
            let mut db_model = None;
            let mut db_effort = None;
            let mut db_approval = None;
            let mut db_sandbox = None;
            let mut rollout_path = None;
            if let Some(conn) = open_sqlite_readonly(&db_path) {
                if let Ok((m, e, ap, sb, rp)) = conn.query_row(
                    "SELECT COALESCE(model, ''), COALESCE(reasoning_effort, ''), \
                     COALESCE(approval_mode, ''), COALESCE(sandbox_policy, ''), COALESCE(rollout_path, '') \
                     FROM threads WHERE id = ?1 LIMIT 1",
                    [sid],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, String>(3)?,
                            r.get::<_, String>(4)?,
                        ))
                    },
                ) {
                    if !m.trim().is_empty() {
                        db_model = Some(m);
                    }
                    if !e.trim().is_empty() {
                        db_effort = Some(e);
                    }
                    if !ap.trim().is_empty() {
                        db_approval = Some(ap);
                    }
                    if !sb.trim().is_empty() {
                        db_sandbox = Some(sb);
                    }
                    if !rp.trim().is_empty() {
                        rollout_path = Some(clean_windows_path(&rp));
                    }
                }
            }
            let raw_tail = rollout_path
                .as_deref()
                .and_then(|p| read_file_tail_utf8(Path::new(p), 131_072))
                .unwrap_or_default();
            let (model, effort, mode, plan_mode) = parse_codex_session_state_from_turn_context(
                &raw_tail,
                db_model.as_deref(),
                db_effort.as_deref(),
                db_approval.as_deref(),
                db_sandbox.as_deref(),
            );
            out.model = model;
            out.effort = effort;
            out.mode = mode;
            out.plan_mode = plan_mode;
            if out.model.is_some() || out.effort.is_some() || out.mode.is_some() {
                out.source_note = "已同步 Codex 当前对话（发消息时生效并同步回对话）".to_string();
            }
        }
        HarnessKind::ClaudeCode => {
            let projects_root = home.join(".claude").join("projects");
            let target_name = format!("{sid}.jsonl");
            if let Ok(proj_dirs) = fs::read_dir(&projects_root) {
                for proj in proj_dirs.flatten() {
                    let candidate = proj.path().join(&target_name);
                    if candidate.is_file() {
                        if let Some(raw_tail) = read_file_tail_utf8(&candidate, 131_072) {
                            let (model, mode) = parse_claude_session_state_from_jsonl(&raw_tail);
                            out.model = model;
                            out.mode = mode;
                        }
                        break;
                    }
                }
            }
            let claude_dir = home.join(".claude");
            for file_name in ["settings.local.json", "settings.json"] {
                let Ok(text) = fs::read_to_string(claude_dir.join(file_name)) else {
                    continue;
                };
                let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
                    continue;
                };
                if out.effort.is_none() {
                    if let Some(e) = v
                        .get("effortLevel")
                        .or_else(|| v.get("effort"))
                        .and_then(|s| s.as_str())
                        .filter(|s| !s.trim().is_empty())
                    {
                        out.effort = Some(e.trim().to_ascii_lowercase());
                    }
                }
                if out.model.is_none() {
                    if let Some(m) = v.get("model").and_then(|s| s.as_str()) {
                        out.model = normalize_claude_model_id(m);
                    }
                }
                if out.mode.is_none() {
                    if let Some(p) = v.pointer("/permissions/defaultMode").and_then(|s| s.as_str()) {
                        out.mode = match p.trim() {
                            "default" | "manual" => Some("manual".to_string()),
                            "auto" | "acceptEdits" | "plan" | "dontAsk" | "bypassPermissions" => {
                                Some(p.trim().to_string())
                            }
                            _ => None,
                        };
                    }
                }
            }
            if out.model.is_some() || out.effort.is_some() || out.mode.is_some() {
                out.source_note = "已同步 Claude Code 当前对话（发消息时生效并同步回对话）".to_string();
            }
        }
        HarnessKind::Antigravity => {
            // Official `agy acp` natively supports `session/resume` + `session/set_model` + `session/set_mode`
            // on ALL Antigravity conversations (both Desktop and CLI), so model & mode are fully editable!
            out.read_only_in_desktop = false;
            let anno_path = home
                .join(".gemini")
                .join("antigravity")
                .join("annotations")
                .join(format!("{sid}.pbtxt"));
            if let Ok(pbtxt) = fs::read_to_string(&anno_path) {
                out.mode = parse_antigravity_permission_preset(&pbtxt);
            }
            let transcript_candidates = [
                home.join(".gemini")
                    .join("antigravity")
                    .join("brain")
                    .join(sid)
                    .join(".system_generated")
                    .join("logs")
                    .join("transcript.jsonl"),
                home.join(".gemini")
                    .join("antigravity-cli")
                    .join("brain")
                    .join(sid)
                    .join(".system_generated")
                    .join("logs")
                    .join("transcript.jsonl"),
            ];
            for tp in transcript_candidates {
                if let Some(raw_tail) = read_file_tail_utf8(&tp, 131_072) {
                    if let Some(raw_model) = parse_antigravity_latest_model_from_transcript(&raw_tail) {
                        let (canon_model, eff) = parse_antigravity_model_and_effort(&raw_model);
                        out.model = canon_model;
                        out.effort = eff;
                        break;
                    }
                }
            }
            out.source_note =
                "已同步 Antigravity 当前对话（通过官方 agy acp 实时生效）".to_string();
        }
        HarnessKind::Dsh => {}
    }
    out
}

#[derive(Debug, Clone)]
pub struct MediumTurnParams {
    pub turn_id: String,
    pub harness: String,
    pub exe: String,
    pub acp_args: Vec<String>,
    pub cwd: Option<String>,
    pub project_id: Option<String>,
    pub session_id: Option<String>,
    pub permission_mode: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub sandbox_mode: Option<String>,
    pub execution_mode: Option<String>,
    pub fast_speed: Option<bool>,
    pub plan_mode: Option<bool>,
    pub sandbox_enabled: Option<bool>,
    pub chrome_enabled: Option<bool>,
    pub enabled_plugins: Vec<String>,
    pub attachments: Vec<TurnAttachment>,
    pub prompt: String,
    pub out_pipe: String,
}

pub(crate) fn build_prompt_with_file_attachments(
    prompt: &str,
    attachments: &[TurnAttachment],
    include_images_as_paths: bool,
) -> String {
    let mut lines = Vec::new();
    for att in attachments {
        if att.path.trim().is_empty() {
            continue;
        }
        if att.kind == "image" && !include_images_as_paths {
            continue;
        }
        let label = match att.kind.as_str() {
            "image" => "附加图片",
            "folder" => "附加文件夹",
            _ => "附加文件",
        };
        lines.push(format!("[{label}: {}]", att.path.trim()));
    }
    if lines.is_empty() {
        prompt.to_string()
    } else if prompt.trim().is_empty() {
        lines.join("\n")
    } else {
        format!("{}\n\n{}", lines.join("\n"), prompt)
    }
}

