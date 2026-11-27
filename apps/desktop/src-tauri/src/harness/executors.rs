use super::*;

/// Runs strictly inside the medium-integrity broker process (or directly when app is already medium).
pub fn execute_harness_turn_in_medium(params: MediumTurnParams) {
    let full_pipe = if params.out_pipe.starts_with(r"\\.\pipe\") {
        params.out_pipe.clone()
    } else {
        format!(r"\\.\pipe\{}", params.out_pipe)
    };

    let Ok(mut pipe_writer) = fs::OpenOptions::new().write(true).open(&full_pipe) else {
        log::warn!("Medium harness worker failed to open stream pipe {full_pipe}");
        return;
    };

    if crate::acp::is_acp_harness_id(&params.harness) {
        let mode_choice = params
            .execution_mode
            .as_deref()
            .or(params.permission_mode.as_deref())
            .or(params.sandbox_mode.as_deref());
        let _ = crate::acp::run_acp_turn_in_medium(
            &params.turn_id,
            &params.harness,
            &params.exe,
            &params.acp_args,
            params.cwd.as_deref(),
            params.session_id.as_deref(),
            params.model.as_deref(),
            params.effort.as_deref(),
            mode_choice,
            &params.attachments,
            &params.prompt,
            &mut pipe_writer,
            true,
        );
        unregister_turn_pid(&params.turn_id);
        return;
    }

    match HarnessKind::parse(&params.harness) {
        HarnessKind::Antigravity => {
            run_antigravity_turn_in_medium(
                &params.turn_id,
                &params.exe,
                params.cwd.as_deref(),
                params.project_id.as_deref(),
                params.session_id.as_deref(),
                params.model.as_deref(),
                params.effort.as_deref(),
                params.execution_mode.as_deref(),
                params.plan_mode,
                params.sandbox_enabled,
                &params.attachments,
                &params.prompt,
                &mut pipe_writer,
            );
        }
        HarnessKind::Codex => {
            run_codex_app_server_turn_in_medium(
                &params.turn_id,
                &params.exe,
                params.cwd.as_deref(),
                params.session_id.as_deref(),
                params.model.as_deref(),
                params.effort.as_deref(),
                params.sandbox_mode.as_deref(),
                params.fast_speed,
                params.plan_mode,
                &params.enabled_plugins,
                &params.attachments,
                &params.prompt,
                &mut pipe_writer,
            );
        }
        HarnessKind::ClaudeCode => {
            run_claude_code_turn_in_medium(
                &params.turn_id,
                &params.exe,
                params.cwd.as_deref(),
                params.session_id.as_deref(),
                params.permission_mode.as_deref(),
                params.model.as_deref(),
                params.effort.as_deref(),
                params.plan_mode,
                params.chrome_enabled,
                &params.attachments,
                &params.prompt,
                &mut pipe_writer,
            );
        }
        HarnessKind::Dsh => {
            emit_stream_event(
                &mut pipe_writer,
                &HarnessStreamEvent {
                    turn_id: params.turn_id.clone(),
                    harness: "dsh".to_string(),
                    kind: "error".to_string(),
                    session_id: None,
                    text: None,
                    tool_name: None,
                    error: Some("DSH 使用原生 WebView 发送通道".to_string()),
                },
            );
        }
    }
    unregister_turn_pid(&params.turn_id);
}

pub(super) fn try_run_antigravity_via_desktop_sidecar(
    turn_id: &str,
    session_id: Option<&str>,
    model: Option<&str>,
    prompt: &str,
    pipe_writer: &mut impl Write,
) -> bool {
    let cancel_flag = register_sidecar_cancel(turn_id);
    let target_sid = session_id.map(|s| s.trim()).filter(|s| !s.is_empty());
    let home = dirs::home_dir();
    let baseline_assistant_count = match (&home, target_sid) {
        (Some(h), Some(sid)) => {
            let tp = h
                .join(".gemini")
                .join("antigravity")
                .join("brain")
                .join(sid)
                .join(".system_generated")
                .join("logs")
                .join("transcript.jsonl");
            fs::read_to_string(&tp)
                .ok()
                .map(|raw| {
                    parse_antigravity_transcript_lines(&raw)
                        .into_iter()
                        .filter(|m| m.role == "assistant")
                        .count()
                })
                .unwrap_or(0)
        }
        _ => 0,
    };

    let clean_prompt = prompt.replace('\0', "");
    let req = if let Some(sid) = target_sid {
        crate::harness_sync::AntigravitySidecarRequest {
            action: "send_message".to_string(),
            conversation_id: Some(sid.replace('\0', "")),
            prompt: Some(clean_prompt),
            title: Some("Anything-to-Prompt".to_string()),
            model: None,
        }
    } else {
        crate::harness_sync::AntigravitySidecarRequest {
            action: "new_conversation".to_string(),
            conversation_id: None,
            prompt: Some(clean_prompt),
            title: None,
            model: model
                .map(|s| s.trim().replace('\0', ""))
                .filter(|s| !s.is_empty()),
        }
    };

    let resp = match crate::harness_sync::call_antigravity_sidecar(&req) {
        Ok(r) => r,
        Err(e) if e.starts_with("SIDE_OFFLINE:") => {
            // Antigravity Desktop is closed or sidecar is still booting: silently launch it in the
            // background (minimized without activating!) and wait for the sidecar pipe to connect.
            let mut woken: Option<crate::harness_sync::AntigravitySidecarResponse> = None;
            if let Some(exe) = resolve_antigravity_desktop_exe() {
                emit_stream_event(
                    pipe_writer,
                    &HarnessStreamEvent {
                        turn_id: turn_id.to_string(),
                        harness: "antigravity".to_string(),
                        kind: "tool".to_string(),
                        session_id: target_sid.map(|s| s.to_string()),
                        text: None,
                        tool_name: Some("正在后台静默启动 Antigravity…".to_string()),
                        error: None,
                    },
                );
                if !is_process_name_running("antigravity.exe") {
                    let _ = launch_desktop_app_silently(&exe, "antigravity.exe");
                }
                let deadline = Instant::now() + Duration::from_secs(25);
                while Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(600));
                    match crate::harness_sync::call_antigravity_sidecar(&req) {
                        Ok(r) => {
                            woken = Some(r);
                            break;
                        }
                        Err(err2) if err2.starts_with("SIDE_OFFLINE:") => continue,
                        Err(err2) => {
                            emit_stream_event(
                                pipe_writer,
                                &HarnessStreamEvent {
                                    turn_id: turn_id.to_string(),
                                    harness: "antigravity".to_string(),
                                    kind: "error".to_string(),
                                    session_id: target_sid.map(|s| s.to_string()),
                                    text: None,
                                    tool_name: None,
                                    error: Some(format!("Antigravity Sidecar 通信失败: {err2}")),
                                },
                            );
                            return true;
                        }
                    }
                }
            }
            if let Some(r) = woken {
                r
            } else {
                emit_stream_event(
                    pipe_writer,
                    &HarnessStreamEvent {
                        turn_id: turn_id.to_string(),
                        harness: "antigravity".to_string(),
                        kind: "error".to_string(),
                        session_id: target_sid.map(|s| s.to_string()),
                        text: None,
                        tool_name: None,
                        error: Some(
                            "Antigravity 后台启动超时，Sidecar 尚未就绪：可点击抽屉右上角按钮拉起 Antigravity 桌面端后重试。"
                                .to_string(),
                        ),
                    },
                );
                return true;
            }
        }
        Err(e) => {
            emit_stream_event(
                pipe_writer,
                &HarnessStreamEvent {
                    turn_id: turn_id.to_string(),
                    harness: "antigravity".to_string(),
                    kind: "error".to_string(),
                    session_id: target_sid.map(|s| s.to_string()),
                    text: None,
                    tool_name: None,
                    error: Some(format!("Antigravity Sidecar 通信失败: {e}")),
                },
            );
            return true;
        }
    };

    if !resp.ok {
        emit_stream_event(
            pipe_writer,
            &HarnessStreamEvent {
                turn_id: turn_id.to_string(),
                harness: "antigravity".to_string(),
                kind: "error".to_string(),
                session_id: resp.conversation_id.or_else(|| target_sid.map(|s| s.to_string())),
                text: None,
                tool_name: None,
                error: Some(
                    resp.error
                        .unwrap_or_else(|| "Antigravity 桌面端投递失败".to_string()),
                ),
            },
        );
        return true;
    }

    let resolved_sid = resp
        .conversation_id
        .or_else(|| target_sid.map(|s| s.to_string()));
    if let Some(ref sid) = resolved_sid {
        emit_stream_event(
            pipe_writer,
            &HarnessStreamEvent {
                turn_id: turn_id.to_string(),
                harness: "antigravity".to_string(),
                kind: "session".to_string(),
                session_id: Some(sid.clone()),
                text: None,
                tool_name: None,
                error: None,
            },
        );
    }
    emit_stream_event(
        pipe_writer,
        &HarnessStreamEvent {
            turn_id: turn_id.to_string(),
            harness: "antigravity".to_string(),
            kind: "tool".to_string(),
            session_id: resolved_sid.clone(),
            text: None,
            tool_name: Some("已静默投递至 Antigravity 桌面端，等待回复".to_string()),
            error: None,
        },
    );

    // Tail transcript.jsonl silently in the background until Antigravity writes a new PLANNER_RESPONSE
    if let (Some(h), Some(ref sid)) = (home, &resolved_sid) {
        let tp = h
            .join(".gemini")
            .join("antigravity")
            .join("brain")
            .join(sid)
            .join(".system_generated")
            .join("logs")
            .join("transcript.jsonl");
        let start = std::time::Instant::now();
        let mut last_reply = String::new();
        let mut last_change_at: Option<std::time::Instant> = None;

        while start.elapsed() < std::time::Duration::from_secs(180) {
            if cancel_flag.load(Ordering::Acquire) {
                emit_stream_event(
                    pipe_writer,
                    &HarnessStreamEvent {
                        turn_id: turn_id.to_string(),
                        harness: "antigravity".to_string(),
                        kind: "error".to_string(),
                        session_id: Some(sid.clone()),
                        text: None,
                        tool_name: None,
                        error: Some("已取消本次发送".to_string()),
                    },
                );
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(350));
            if let Ok(raw) = fs::read_to_string(&tp) {
                let assts: Vec<ExternalChatMessage> = parse_antigravity_transcript_lines(&raw)
                    .into_iter()
                    .filter(|m| m.role == "assistant")
                    .collect();
                if assts.len() > baseline_assistant_count {
                    if let Some(newest) = assts.last() {
                        if newest.text != last_reply && !newest.text.trim().is_empty() {
                            last_reply = newest.text.clone();
                            last_change_at = Some(std::time::Instant::now());
                            emit_stream_event(
                                pipe_writer,
                                &HarnessStreamEvent {
                                    turn_id: turn_id.to_string(),
                                    harness: "antigravity".to_string(),
                                    kind: "replace".to_string(),
                                    session_id: Some(sid.clone()),
                                    text: Some(last_reply.clone()),
                                    tool_name: None,
                                    error: None,
                                },
                            );
                        }
                    }
                }
            }
            // Once a reply has arrived and remained stable for 1.5s, complete the turn cleanly
            if let Some(changed) = last_change_at {
                if changed.elapsed() >= std::time::Duration::from_millis(1500) {
                    emit_stream_event(
                        pipe_writer,
                        &HarnessStreamEvent {
                            turn_id: turn_id.to_string(),
                            harness: "antigravity".to_string(),
                            kind: "completed".to_string(),
                            session_id: Some(sid.clone()),
                            text: Some(last_reply),
                            tool_name: None,
                            error: None,
                        },
                    );
                    return true;
                }
            }
        }

        // Timed out waiting for transcript update — surface explicit error instead of silent success
        if last_reply.is_empty() {
            emit_stream_event(
                pipe_writer,
                &HarnessStreamEvent {
                    turn_id: turn_id.to_string(),
                    harness: "antigravity".to_string(),
                    kind: "error".to_string(),
                    session_id: Some(sid.clone()),
                    text: None,
                    tool_name: None,
                    error: Some("等待 Antigravity 回复超时，请切换到 Antigravity 窗口确认".to_string()),
                },
            );
            return true;
        }
    }

    emit_stream_event(
        pipe_writer,
        &HarnessStreamEvent {
            turn_id: turn_id.to_string(),
            harness: "antigravity".to_string(),
            kind: "completed".to_string(),
            session_id: resolved_sid,
            text: None,
            tool_name: None,
            error: None,
        },
    );
    true
}

pub(super) fn run_antigravity_turn_in_medium(
    turn_id: &str,
    exe: &str,
    cwd: Option<&str>,
    project_id: Option<&str>,
    session_id: Option<&str>,
    model: Option<&str>,
    effort: Option<&str>,
    execution_mode: Option<&str>,
    plan_mode: Option<bool>,
    sandbox_enabled: Option<bool>,
    attachments: &[TurnAttachment],
    prompt: &str,
    pipe_writer: &mut impl Write,
) {
    let full_prompt = build_prompt_with_file_attachments(prompt, attachments, true).replace('\0', "");
    let resolved_model = model
        .map(|m| resolve_antigravity_model_for_effort(m, effort))
        .filter(|s| !s.is_empty());
    // Determine whether target is an Antigravity CLI conversation vs Antigravity Desktop conversation.
    // CRITICAL: Desktop conversations live in `~/.gemini/antigravity` and are unknown to `agy acp`
    // (`Session not found` after ~12s, which would otherwise fall back to `session/new` and hijack the
    // message into an orphan CLI session!). Always route Desktop conversations directly to the official
    // `agentapi` Desktop sidecar FIRST (~0.3s silent delivery into the live Desktop conversation).
    let is_cli_target = match (session_id, project_id) {
        (Some(sid), _) if !sid.trim().is_empty() => is_antigravity_cli_conversation(sid),
        (_, Some("default-cli-project")) => true,
        _ => false,
    };
    let effective_mode = if plan_mode == Some(true) {
        Some("plan")
    } else {
        execution_mode
    };
    if !is_cli_target
        && try_run_antigravity_via_desktop_sidecar(
            turn_id,
            session_id,
            resolved_model.as_deref(),
            &full_prompt,
            pipe_writer,
        )
    {
        return;
    }
    // Primary path for Antigravity CLI conversations (or fallback when Desktop sidecar is unavailable):
    // official `agy acp` (Antigravity built-in ACP v1 server).
    if exe.trim().is_empty() || !Path::new(exe).is_file() {
        emit_stream_event(
            pipe_writer,
            &HarnessStreamEvent {
                turn_id: turn_id.to_string(),
                harness: "antigravity".to_string(),
                kind: "error".to_string(),
                session_id: None,
                text: None,
                tool_name: None,
                error: Some("未找到 Antigravity CLI (agy.exe)，请确认已安装或先启动 Antigravity 桌面端".to_string()),
            },
        );
        return;
    }
    let acp_args = vec!["acp".to_string()];
    if crate::acp::run_acp_turn_in_medium(
        turn_id,
        "antigravity",
        exe,
        &acp_args,
        cwd,
        session_id,
        resolved_model.as_deref(),
        effort,
        effective_mode,
        attachments,
        prompt,
        pipe_writer,
        false,
    ) {
        return;
    }

    let mut cmd = Command::new(exe);
    cmd.args(["--output-format", "stream-json"]);
    if let Some(ref m) = resolved_model {
        cmd.args(["--model", m]);
    }
    if let Some(e) = effort.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        cmd.args(["--effort", e]);
    }
    let mut added_sandbox = false;
    if plan_mode == Some(true) {
        cmd.args(["--mode", "plan"]);
    } else if let Some(mode) = execution_mode.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        match mode {
            "turbo" => {
                cmd.arg("--dangerously-skip-permissions");
                cmd.args(["--mode", "auto"]);
            }
            "auto" => {
                cmd.args(["--mode", "auto"]);
            }
            "default" | "accept-edits" => {
                cmd.args(["--mode", "accept-edits"]);
            }
            "request-review" => {
                cmd.arg("--sandbox");
                added_sandbox = true;
                cmd.args(["--mode", "plan"]);
            }
            "plan" => {
                cmd.args(["--mode", "plan"]);
            }
            _ => {}
        }
    }
    if sandbox_enabled == Some(true) && !added_sandbox {
        cmd.arg("--sandbox");
    }
    for att in attachments {
        if att.kind == "folder" && !att.path.trim().is_empty() && Path::new(att.path.trim()).is_dir() {
            cmd.args(["--add-dir", att.path.trim()]);
        }
    }
    if let Some(sid) = session_id.filter(|s| !s.trim().is_empty()) {
        cmd.args(["--conversation", sid.trim()]);
    } else if let Some(pid) =
        project_id.filter(|s| !s.trim().is_empty() && *s != "outside-of-project")
    {
        if pid != "default-cli-project" {
            cmd.args(["--project", pid.trim()]);
        }
    }
    let bounded_print = crate::harness_sync::bound_cli_prompt_arg(&full_prompt);
    cmd.arg(format!("--print={bounded_print}"));
    if let Some(dir) = resolve_effective_workspace_dir(cwd) {
        cmd.current_dir(&dir);
    }
    cmd.creation_flags(CREATE_NO_WINDOW.0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            emit_stream_event(
                pipe_writer,
                &HarnessStreamEvent {
                    turn_id: turn_id.to_string(),
                    harness: "antigravity".to_string(),
                    kind: "error".to_string(),
                    session_id: None,
                    text: None,
                    tool_name: None,
                    error: Some(format!("启动 agy.exe 失败: {e}")),
                },
            );
            return;
        }
    };

    register_turn_pid(turn_id, child.id());

    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        return;
    };

    let reader = BufReader::new(stdout);
    let mut resolved_session_id = session_id.map(|s| s.to_string());
    let mut emitted_any_delta = false;
    let mut completed = false;

    for line in reader.lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        let event_name = v
            .get("event")
            .or_else(|| v.get("type"))
            .and_then(|s| s.as_str())
            .unwrap_or("");

        match event_name {
            "init" => {
                if let Some(cid) = v.get("conversation_id").and_then(|s| s.as_str()) {
                    if !cid.trim().is_empty() {
                        resolved_session_id = Some(cid.to_string());
                        emit_stream_event(
                            pipe_writer,
                            &HarnessStreamEvent {
                                turn_id: turn_id.to_string(),
                                harness: "antigravity".to_string(),
                                kind: "session".to_string(),
                                session_id: Some(cid.to_string()),
                                text: None,
                                tool_name: None,
                                error: None,
                            },
                        );
                    }
                }
            }
            "step_update" => {
                let step_type = v.get("step_type").and_then(|s| s.as_str()).unwrap_or("");
                if let Some(delta) = v.get("text_delta").and_then(|s| s.as_str()) {
                    if !delta.is_empty() {
                        emitted_any_delta = true;
                        emit_stream_event(
                            pipe_writer,
                            &HarnessStreamEvent {
                                turn_id: turn_id.to_string(),
                                harness: "antigravity".to_string(),
                                kind: "delta".to_string(),
                                session_id: resolved_session_id.clone(),
                                text: Some(delta.to_string()),
                                tool_name: None,
                                error: None,
                            },
                        );
                    }
                } else if step_type == "tool" {
                    if let Some(tool) = v.get("tool_name").and_then(|s| s.as_str()) {
                        emit_stream_event(
                            pipe_writer,
                            &HarnessStreamEvent {
                                turn_id: turn_id.to_string(),
                                harness: "antigravity".to_string(),
                                kind: "tool".to_string(),
                                session_id: resolved_session_id.clone(),
                                text: None,
                                tool_name: Some(tool.to_string()),
                                error: None,
                            },
                        );
                    }
                }
            }
            "result" => {
                completed = true;
                let res_obj = v.get("result").unwrap_or(&v);
                let status = res_obj
                    .get("status")
                    .and_then(|s| s.as_str())
                    .unwrap_or("SUCCESS");
                if status.eq_ignore_ascii_case("ERROR") || status.eq_ignore_ascii_case("FAILURE") {
                    let err_msg = res_obj
                        .get("error")
                        .and_then(|s| s.as_str())
                        .unwrap_or("Antigravity 执行失败")
                        .to_string();
                    emit_stream_event(
                        pipe_writer,
                        &HarnessStreamEvent {
                            turn_id: turn_id.to_string(),
                            harness: "antigravity".to_string(),
                            kind: "error".to_string(),
                            session_id: resolved_session_id.clone(),
                            text: None,
                            tool_name: None,
                            error: Some(err_msg),
                        },
                    );
                } else {
                    let final_text = res_obj
                        .get("response")
                        .and_then(|s| s.as_str())
                        .map(|s| s.to_string());
                    if !emitted_any_delta {
                        if let Some(ref text) = final_text {
                            if !text.is_empty() {
                                emit_stream_event(
                                    pipe_writer,
                                    &HarnessStreamEvent {
                                        turn_id: turn_id.to_string(),
                                        harness: "antigravity".to_string(),
                                        kind: "replace".to_string(),
                                        session_id: resolved_session_id.clone(),
                                        text: Some(text.clone()),
                                        tool_name: None,
                                        error: None,
                                    },
                                );
                            }
                        }
                    }
                    emit_stream_event(
                        pipe_writer,
                        &HarnessStreamEvent {
                            turn_id: turn_id.to_string(),
                            harness: "antigravity".to_string(),
                            kind: "completed".to_string(),
                            session_id: resolved_session_id.clone(),
                            text: final_text,
                            tool_name: None,
                            error: None,
                        },
                    );
                }
            }
            _ => {}
        }
    }

    let status = child.wait();
    if !completed {
        let mut stderr_text = String::new();
        if let Some(mut se) = child.stderr.take() {
            let _ = std::io::Read::read_to_string(&mut se, &mut stderr_text);
        }
        let ok = status.map(|s| s.success()).unwrap_or(false);
        if ok {
            emit_stream_event(
                pipe_writer,
                &HarnessStreamEvent {
                    turn_id: turn_id.to_string(),
                    harness: "antigravity".to_string(),
                    kind: "completed".to_string(),
                    session_id: resolved_session_id,
                    text: None,
                    tool_name: None,
                    error: None,
                },
            );
        } else {
            let err = if stderr_text.trim().is_empty() {
                "Antigravity 进程异常退出".to_string()
            } else {
                stderr_text.trim().chars().take(280).collect()
            };
            emit_stream_event(
                pipe_writer,
                &HarnessStreamEvent {
                    turn_id: turn_id.to_string(),
                    harness: "antigravity".to_string(),
                    kind: "error".to_string(),
                    session_id: resolved_session_id,
                    text: None,
                    tool_name: None,
                    error: Some(err),
                },
            );
        }
    }
}

pub(super) fn send_jsonrpc_line(stdin: &mut impl Write, val: &serde_json::Value) -> std::io::Result<()> {
    let line = serde_json::to_string(val)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    writeln!(stdin, "{line}")?;
    stdin.flush()
}

pub fn build_codex_turn_start_params(
    tid: &str,
    model: Option<&str>,
    effort: Option<&str>,
    sandbox_mode: Option<&str>,
    fast_speed: Option<bool>,
    plan_mode: Option<bool>,
    enabled_plugins: &[String],
    attachments: &[TurnAttachment],
    prompt: &str,
) -> serde_json::Map<String, serde_json::Value> {
    let mut input_items: Vec<serde_json::Value> = Vec::new();
    for att in attachments {
        if att.kind == "image" && !att.path.trim().is_empty() {
            input_items.push(serde_json::json!({
                "type": "localImage",
                "path": att.path.trim()
            }));
        }
    }
    for raw_plg in enabled_plugins {
        let plg_id = raw_plg.trim();
        if plg_id.is_empty() {
            continue;
        }
        let short_name = plg_id.split('@').next().unwrap_or(plg_id).trim();
        let uri = if plg_id.starts_with("plugin://") {
            plg_id.to_string()
        } else {
            format!("plugin://{plg_id}")
        };
        input_items.push(serde_json::json!({
            "type": "mention",
            "name": short_name,
            "path": uri
        }));
    }
    let mut final_text = build_prompt_with_file_attachments(prompt, attachments, false);
    if plan_mode == Some(true) && !final_text.trim_start().starts_with("/plan") {
        final_text = format!("[Plan Mode: 只读分析与制定分步执行计划，不要直接修改代码或执行写入操作]\n\n{final_text}");
    }
    if !final_text.trim().is_empty() || input_items.is_empty() {
        input_items.push(serde_json::json!({
            "type": "text",
            "text": final_text
        }));
    }
    let mut turn_params = serde_json::Map::new();
    turn_params.insert("threadId".to_string(), serde_json::json!(tid));
    turn_params.insert("input".to_string(), serde_json::Value::Array(input_items));
    if let Some(m) = model.map(|s| s.trim()).filter(|s| !s.is_empty() && *s != "default") {
        turn_params.insert("model".to_string(), serde_json::json!(m));
    }
    if let Some(e) = effort.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        let mapped_effort = if e == "max" { "xhigh" } else { e };
        turn_params.insert("effort".to_string(), serde_json::json!(mapped_effort));
    }
    if let Some(fast) = fast_speed {
        turn_params.insert(
            "serviceTier".to_string(),
            serde_json::json!(if fast { "fast" } else { "default" }),
        );
    }
    if plan_mode == Some(true) {
        turn_params.insert("approvalPolicy".to_string(), serde_json::json!("on-request"));
        turn_params.insert("sandboxPolicy".to_string(), serde_json::json!({ "type": "readOnly" }));
    } else if let Some(sb) = sandbox_mode.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        match sb {
            "danger-full-access" => {
                turn_params.insert("approvalPolicy".to_string(), serde_json::json!("never"));
                turn_params.insert("sandboxPolicy".to_string(), serde_json::json!({ "type": "dangerFullAccess" }));
            }
            "help-approve" => {
                // Valid Codex AskForApproval + ApprovalsReviewer per TurnStartParams.json schema
                turn_params.insert("approvalPolicy".to_string(), serde_json::json!("on-request"));
                turn_params.insert("approvalsReviewer".to_string(), serde_json::json!("auto_review"));
                turn_params.insert("sandboxPolicy".to_string(), serde_json::json!({ "type": "workspaceWrite" }));
            }
            "request-approval" | "workspace-write" => {
                turn_params.insert("approvalPolicy".to_string(), serde_json::json!("on-request"));
                turn_params.insert("approvalsReviewer".to_string(), serde_json::json!("user"));
                turn_params.insert("sandboxPolicy".to_string(), serde_json::json!({ "type": "workspaceWrite" }));
            }
            "read-only" => {
                turn_params.insert("approvalPolicy".to_string(), serde_json::json!("on-request"));
                turn_params.insert("sandboxPolicy".to_string(), serde_json::json!({ "type": "readOnly" }));
            }
            _ => {}
        }
    }
    turn_params
}

pub(super) fn run_codex_app_server_turn_in_medium(
    turn_id: &str,
    exe: &str,
    cwd: Option<&str>,
    session_id: Option<&str>,
    model: Option<&str>,
    effort: Option<&str>,
    sandbox_mode: Option<&str>,
    fast_speed: Option<bool>,
    plan_mode: Option<bool>,
    enabled_plugins: &[String],
    attachments: &[TurnAttachment],
    prompt: &str,
    pipe_writer: &mut impl Write,
) {
    let mut cmd = if exe.to_ascii_lowercase().ends_with(".cmd") {
        let mut c = Command::new("cmd.exe");
        c.args(["/C", exe, "app-server"]);
        c
    } else {
        let mut c = Command::new(exe);
        c.arg("app-server");
        c
    };

    let cleaned_cwd = resolve_effective_workspace_dir(cwd);

    if let Some(ref dir) = cleaned_cwd {
        cmd.current_dir(dir);
    }

    cmd.creation_flags(CREATE_NO_WINDOW.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            emit_stream_event(
                pipe_writer,
                &HarnessStreamEvent {
                    turn_id: turn_id.to_string(),
                    harness: "codex".to_string(),
                    kind: "error".to_string(),
                    session_id: None,
                    text: None,
                    tool_name: None,
                    error: Some(format!("启动 codex app-server 失败: {e}")),
                },
            );
            return;
        }
    };

    register_turn_pid(turn_id, child.id());

    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = child.kill();
        return;
    };

    // Step 1: initialize
    let init_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "clientInfo": {
                "name": "anything-to-prompt",
                "title": "Anything-to-Prompt",
                "version": "0.1.0"
            }
        }
    });
    if let Err(e) = send_jsonrpc_line(&mut stdin, &init_req) {
        emit_stream_event(
            pipe_writer,
            &HarnessStreamEvent {
                turn_id: turn_id.to_string(),
                harness: "codex".to_string(),
                kind: "error".to_string(),
                session_id: None,
                text: None,
                tool_name: None,
                error: Some(format!("Codex initialize 写入失败: {e}")),
            },
        );
        let _ = child.kill();
        return;
    }

    let reader = BufReader::new(stdout);
    let mut stage = 1u8; // 1=wait init, 2=wait thread/start|resume, 3=streaming turn
    let mut resolved_thread_id = session_id
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string());
    let mut emitted_delta = false;
    let mut finished = false;

    for line in reader.lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };

        // Handle JSON-RPC responses by id
        if let Some(id_num) = v.get("id").and_then(|i| i.as_i64()) {
            if let Some(err) = v.get("error") {
                let msg = err
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("Codex app-server 返回错误")
                    .to_string();
                emit_stream_event(
                    pipe_writer,
                    &HarnessStreamEvent {
                        turn_id: turn_id.to_string(),
                        harness: "codex".to_string(),
                        kind: "error".to_string(),
                        session_id: resolved_thread_id.clone(),
                        text: None,
                        tool_name: None,
                        error: Some(msg),
                    },
                );
                finished = true;
                break;
            }

            if id_num == 1 && stage == 1 {
                // Send initialized notification + thread/resume or thread/start
                let _ = send_jsonrpc_line(
                    &mut stdin,
                    &serde_json::json!({ "jsonrpc": "2.0", "method": "initialized" }),
                );
                let thread_req = if let Some(ref tid) = resolved_thread_id {
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": 2,
                        "method": "thread/resume",
                        "params": { "threadId": tid }
                    })
                } else {
                    let mut start_params = serde_json::Map::new();
                    start_params.insert("cwd".to_string(), serde_json::json!(cleaned_cwd.clone().unwrap_or_default()));
                    if let Some(fast) = fast_speed {
                        start_params.insert(
                            "serviceTier".to_string(),
                            serde_json::json!(if fast { "fast" } else { "default" }),
                        );
                    }
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": 2,
                        "method": "thread/start",
                        "params": serde_json::Value::Object(start_params)
                    })
                };
                if let Err(e) = send_jsonrpc_line(&mut stdin, &thread_req) {
                    emit_stream_event(
                        pipe_writer,
                        &HarnessStreamEvent {
                            turn_id: turn_id.to_string(),
                            harness: "codex".to_string(),
                            kind: "error".to_string(),
                            session_id: None,
                            text: None,
                            tool_name: None,
                            error: Some(format!("Codex 创建/恢复会话失败: {e}")),
                        },
                    );
                    finished = true;
                    break;
                }
                stage = 2;
                continue;
            }

            if id_num == 2 && stage == 2 {
                if let Some(tid) = v
                    .get("result")
                    .and_then(|r| r.get("thread"))
                    .and_then(|t| t.get("id"))
                    .and_then(|s| s.as_str())
                {
                    resolved_thread_id = Some(tid.to_string());
                    emit_stream_event(
                        pipe_writer,
                        &HarnessStreamEvent {
                            turn_id: turn_id.to_string(),
                            harness: "codex".to_string(),
                            kind: "session".to_string(),
                            session_id: Some(tid.to_string()),
                            text: None,
                            tool_name: None,
                            error: None,
                        },
                    );
                }
                let Some(ref tid) = resolved_thread_id else {
                    emit_stream_event(
                        pipe_writer,
                        &HarnessStreamEvent {
                            turn_id: turn_id.to_string(),
                            harness: "codex".to_string(),
                            kind: "error".to_string(),
                            session_id: None,
                            text: None,
                            tool_name: None,
                            error: Some("Codex 未返回有效 threadId".to_string()),
                        },
                    );
                    finished = true;
                    break;
                };
                let turn_params = build_codex_turn_start_params(
                    tid,
                    model,
                    effort,
                    sandbox_mode,
                    fast_speed,
                    plan_mode,
                    enabled_plugins,
                    attachments,
                    prompt,
                );
                let turn_req = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 3,
                    "method": "turn/start",
                    "params": serde_json::Value::Object(turn_params)
                });
                if let Err(e) = send_jsonrpc_line(&mut stdin, &turn_req) {
                    emit_stream_event(
                        pipe_writer,
                        &HarnessStreamEvent {
                            turn_id: turn_id.to_string(),
                            harness: "codex".to_string(),
                            kind: "error".to_string(),
                            session_id: resolved_thread_id.clone(),
                            text: None,
                            tool_name: None,
                            error: Some(format!("Codex 发送消息失败: {e}")),
                        },
                    );
                    finished = true;
                    break;
                }
                stage = 3;
                continue;
            }

            // Server-initiated request (e.g., requestApproval) — answer politely so we never deadlock
            if let Some(method) = v.get("method").and_then(|m| m.as_str()) {
                if method.contains("requestApproval") {
                    let reply = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": v.get("id").cloned().unwrap_or(serde_json::json!(0)),
                        "result": { "decision": "decline" }
                    });
                    let _ = send_jsonrpc_line(&mut stdin, &reply);
                    emit_stream_event(
                        pipe_writer,
                        &HarnessStreamEvent {
                            turn_id: turn_id.to_string(),
                            harness: "codex".to_string(),
                            kind: "tool".to_string(),
                            session_id: resolved_thread_id.clone(),
                            text: None,
                            tool_name: Some("需在 Codex 桌面端确认敏感操作".to_string()),
                            error: None,
                        },
                    );
                }
            }
            continue;
        }

        // Handle JSON-RPC notifications
        let Some(method) = v.get("method").and_then(|m| m.as_str()) else {
            continue;
        };
        let params = v.get("params");

        match method {
            "item/agentMessage/delta" => {
                if let Some(delta) = params.and_then(|p| p.get("delta")).and_then(|d| d.as_str()) {
                    if !delta.is_empty() {
                        emitted_delta = true;
                        emit_stream_event(
                            pipe_writer,
                            &HarnessStreamEvent {
                                turn_id: turn_id.to_string(),
                                harness: "codex".to_string(),
                                kind: "delta".to_string(),
                                session_id: resolved_thread_id.clone(),
                                text: Some(delta.to_string()),
                                tool_name: None,
                                error: None,
                            },
                        );
                    }
                }
            }
            "item/started" => {
                if let Some(item_type) = params
                    .and_then(|p| p.get("item"))
                    .and_then(|i| i.get("type"))
                    .and_then(|t| t.as_str())
                {
                    if item_type != "agentMessage" && item_type != "userMessage" {
                        emit_stream_event(
                            pipe_writer,
                            &HarnessStreamEvent {
                                turn_id: turn_id.to_string(),
                                harness: "codex".to_string(),
                                kind: "tool".to_string(),
                                session_id: resolved_thread_id.clone(),
                                text: None,
                                tool_name: Some(item_type.to_string()),
                                error: None,
                            },
                        );
                    }
                }
            }
            "item/completed" => {
                if !emitted_delta {
                    if let Some(item) = params.and_then(|p| p.get("item")) {
                        let ty = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        if ty.eq_ignore_ascii_case("agentMessage") {
                            if let Some(txt) = item.get("text").and_then(|t| t.as_str()) {
                                if !txt.is_empty() {
                                    emitted_delta = true;
                                    emit_stream_event(
                                        pipe_writer,
                                        &HarnessStreamEvent {
                                            turn_id: turn_id.to_string(),
                                            harness: "codex".to_string(),
                                            kind: "replace".to_string(),
                                            session_id: resolved_thread_id.clone(),
                                            text: Some(txt.to_string()),
                                            tool_name: None,
                                            error: None,
                                        },
                                    );
                                }
                            }
                        }
                    }
                }
            }
            "turn/completed" => {
                let turn_err = params
                    .and_then(|p| p.get("turn"))
                    .and_then(|t| t.get("error"))
                    .and_then(|e| {
                        e.get("message")
                            .and_then(|m| m.as_str())
                            .or_else(|| e.as_str())
                    })
                    .map(|s| s.to_string());
                if let Some(err_msg) = turn_err {
                    emit_stream_event(
                        pipe_writer,
                        &HarnessStreamEvent {
                            turn_id: turn_id.to_string(),
                            harness: "codex".to_string(),
                            kind: "error".to_string(),
                            session_id: resolved_thread_id.clone(),
                            text: None,
                            tool_name: None,
                            error: Some(err_msg),
                        },
                    );
                } else {
                    emit_stream_event(
                        pipe_writer,
                        &HarnessStreamEvent {
                            turn_id: turn_id.to_string(),
                            harness: "codex".to_string(),
                            kind: "completed".to_string(),
                            session_id: resolved_thread_id.clone(),
                            text: None,
                            tool_name: None,
                            error: None,
                        },
                    );
                }
                finished = true;
                break;
            }
            _ => {}
        }
    }

    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();

    if !finished {
        emit_stream_event(
            pipe_writer,
            &HarnessStreamEvent {
                turn_id: turn_id.to_string(),
                harness: "codex".to_string(),
                kind: "error".to_string(),
                session_id: resolved_thread_id,
                text: None,
                tool_name: None,
                error: Some("Codex app-server 连接提前结束".to_string()),
            },
        );
    }
}

pub(super) fn sanitize_claude_permission_mode(raw: Option<&str>) -> &'static str {
    match raw.map(|s| s.trim()).unwrap_or("auto") {
        "acceptEdits" => "acceptEdits",
        "plan" => "plan",
        "dontAsk" => "dontAsk",
        "bypassPermissions" => "bypassPermissions",
        "manual" => "manual",
        _ => "auto",
    }
}

#[derive(Default)]
pub(super) struct ClaudeStreamParseState {
    session_id: Option<String>,
    emitted_any_delta: bool,
    emitted_text_in_current_msg: bool,
    completed: bool,
}

pub(super) fn parse_claude_stream_json_line(
    turn_id: &str,
    line: &str,
    state: &mut ClaudeStreamParseState,
) -> Vec<HarnessStreamEvent> {
    let mut events = Vec::new();
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return events;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return events;
    };

    if let Some(sid) = v.get("session_id").and_then(|s| s.as_str()) {
        let clean_sid = sid.trim();
        if !clean_sid.is_empty() && state.session_id.as_deref() != Some(clean_sid) {
            state.session_id = Some(clean_sid.to_string());
            events.push(HarnessStreamEvent {
                turn_id: turn_id.to_string(),
                harness: "claude_code".to_string(),
                kind: "session".to_string(),
                session_id: state.session_id.clone(),
                text: None,
                tool_name: None,
                error: None,
            });
        }
    }

    let event_type = v.get("type").and_then(|s| s.as_str()).unwrap_or("");
    match event_type {
        "stream_event" => {
            let Some(ev) = v.get("event") else {
                return events;
            };
            let ev_type = ev.get("type").and_then(|s| s.as_str()).unwrap_or("");
            match ev_type {
                "message_start" => {
                    if state.emitted_text_in_current_msg {
                        // Separate multi-step assistant turns cleanly
                        events.push(HarnessStreamEvent {
                            turn_id: turn_id.to_string(),
                            harness: "claude_code".to_string(),
                            kind: "delta".to_string(),
                            session_id: state.session_id.clone(),
                            text: Some("\n\n".to_string()),
                            tool_name: None,
                            error: None,
                        });
                    }
                    state.emitted_text_in_current_msg = false;
                }
                "content_block_start" => {
                    if let Some(cb) = ev.get("content_block") {
                        if cb.get("type").and_then(|s| s.as_str()) == Some("tool_use") {
                            if let Some(name) = cb.get("name").and_then(|s| s.as_str()) {
                                events.push(HarnessStreamEvent {
                                    turn_id: turn_id.to_string(),
                                    harness: "claude_code".to_string(),
                                    kind: "tool".to_string(),
                                    session_id: state.session_id.clone(),
                                    text: None,
                                    tool_name: Some(name.to_string()),
                                    error: None,
                                });
                            }
                        }
                    }
                }
                "content_block_delta" => {
                    if let Some(delta) = ev.get("delta") {
                        if delta.get("type").and_then(|s| s.as_str()) == Some("text_delta") {
                            if let Some(txt) = delta.get("text").and_then(|s| s.as_str()) {
                                if !txt.is_empty() {
                                    state.emitted_any_delta = true;
                                    state.emitted_text_in_current_msg = true;
                                    events.push(HarnessStreamEvent {
                                        turn_id: turn_id.to_string(),
                                        harness: "claude_code".to_string(),
                                        kind: "delta".to_string(),
                                        session_id: state.session_id.clone(),
                                        text: Some(txt.to_string()),
                                        tool_name: None,
                                        error: None,
                                    });
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        "assistant" => {
            if v.get("is_api_error_message").and_then(|b| b.as_bool()) == Some(true) {
                return events;
            }
            if let Some(arr) = v
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_array())
            {
                for item in arr {
                    if item.get("type").and_then(|s| s.as_str()) == Some("tool_use") {
                        if let Some(name) = item.get("name").and_then(|s| s.as_str()) {
                            events.push(HarnessStreamEvent {
                                turn_id: turn_id.to_string(),
                                harness: "claude_code".to_string(),
                                kind: "tool".to_string(),
                                session_id: state.session_id.clone(),
                                text: None,
                                tool_name: Some(name.to_string()),
                                error: None,
                            });
                        }
                    }
                }
                if !state.emitted_any_delta {
                    if let Some(content) = v.get("message").and_then(|m| m.get("content")) {
                        let txt = extract_claude_content_text(content);
                        if !txt.is_empty() {
                            state.emitted_any_delta = true;
                            state.emitted_text_in_current_msg = true;
                            events.push(HarnessStreamEvent {
                                turn_id: turn_id.to_string(),
                                harness: "claude_code".to_string(),
                                kind: "replace".to_string(),
                                session_id: state.session_id.clone(),
                                text: Some(txt),
                                tool_name: None,
                                error: None,
                            });
                        }
                    }
                }
            }
        }
        "result" => {
            state.completed = true;
            let is_err = v.get("is_error").and_then(|b| b.as_bool()).unwrap_or(false)
                || v.get("subtype")
                    .and_then(|s| s.as_str())
                    .map(|s| s.starts_with("error"))
                    .unwrap_or(false);
            let result_text = v
                .get("result")
                .and_then(|s| s.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            if is_err {
                let err_msg = result_text.unwrap_or_else(|| "Claude Code 执行失败".to_string());
                events.push(HarnessStreamEvent {
                    turn_id: turn_id.to_string(),
                    harness: "claude_code".to_string(),
                    kind: "error".to_string(),
                    session_id: state.session_id.clone(),
                    text: None,
                    tool_name: None,
                    error: Some(err_msg),
                });
            } else {
                if !state.emitted_any_delta {
                    if let Some(ref txt) = result_text {
                        events.push(HarnessStreamEvent {
                            turn_id: turn_id.to_string(),
                            harness: "claude_code".to_string(),
                            kind: "replace".to_string(),
                            session_id: state.session_id.clone(),
                            text: Some(txt.clone()),
                            tool_name: None,
                            error: None,
                        });
                    }
                }
                events.push(HarnessStreamEvent {
                    turn_id: turn_id.to_string(),
                    harness: "claude_code".to_string(),
                    kind: "completed".to_string(),
                    session_id: state.session_id.clone(),
                    text: result_text,
                    tool_name: None,
                    error: None,
                });
            }
        }
        _ => {}
    }
    events
}

pub(super) fn run_claude_code_turn_in_medium(
    turn_id: &str,
    exe: &str,
    cwd: Option<&str>,
    session_id: Option<&str>,
    permission_mode: Option<&str>,
    model: Option<&str>,
    effort: Option<&str>,
    plan_mode: Option<bool>,
    chrome_enabled: Option<bool>,
    attachments: &[TurnAttachment],
    prompt: &str,
    pipe_writer: &mut impl Write,
) {
    let perm = if plan_mode == Some(true) {
        "plan"
    } else {
        sanitize_claude_permission_mode(permission_mode)
    };
    let full_prompt = build_prompt_with_file_attachments(prompt, attachments, true).replace('\0', "");
    let mut cmd = if exe.to_ascii_lowercase().ends_with(".cmd") {
        let mut c = Command::new("cmd.exe");
        c.args(["/C", exe]);
        c
    } else {
        Command::new(exe)
    };
    cmd.args([
        "-p",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--permission-mode",
        perm,
    ]);
    if chrome_enabled == Some(true) {
        cmd.arg("--chrome");
    }
    if let Some(m) = model.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        cmd.args(["--model", m]);
    }
    if let Some(e) = effort.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        cmd.args(["--effort", e]);
    }
    for att in attachments {
        if att.kind == "folder" && !att.path.trim().is_empty() && Path::new(att.path.trim()).is_dir() {
            cmd.args(["--add-dir", att.path.trim()]);
        }
    }
    if let Some(sid) = session_id.filter(|s| !s.trim().is_empty()) {
        cmd.args(["--resume", sid.trim()]);
    }

    let cleaned_cwd = resolve_effective_workspace_dir(cwd);
    if let Some(ref dir) = cleaned_cwd {
        cmd.current_dir(dir);
    }

    cmd.creation_flags(CREATE_NO_WINDOW.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            emit_stream_event(
                pipe_writer,
                &HarnessStreamEvent {
                    turn_id: turn_id.to_string(),
                    harness: "claude_code".to_string(),
                    kind: "error".to_string(),
                    session_id: None,
                    text: None,
                    tool_name: None,
                    error: Some(format!("启动 claude.exe 失败: {e}")),
                },
            );
            return;
        }
    };

    register_turn_pid(turn_id, child.id());

    // Write prompt to stdin and drain stderr on dedicated threads so neither pipe can ever deadlock
    // on large prompts or verbose stderr output!
    let stdin_writer = child.stdin.take().map(|mut stdin| {
        let prompt_bytes = full_prompt.into_bytes();
        std::thread::spawn(move || {
            let _ = stdin.write_all(&prompt_bytes);
            let _ = stdin.flush();
            drop(stdin);
        })
    });
    let stderr_reader = child.stderr.take().map(|mut se| {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = std::io::Read::read_to_string(&mut se, &mut s);
            s
        })
    });

    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        if let Some(t) = stdin_writer {
            let _ = t.join();
        }
        if let Some(t) = stderr_reader {
            let _ = t.join();
        }
        return;
    };

    let reader = BufReader::new(stdout);
    let mut state = ClaudeStreamParseState {
        session_id: session_id
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_string()),
        ..Default::default()
    };

    for line in reader.lines().map_while(Result::ok) {
        let evts = parse_claude_stream_json_line(turn_id, &line, &mut state);
        for evt in evts {
            emit_stream_event(pipe_writer, &evt);
        }
    }

    if let Some(t) = stdin_writer {
        let _ = t.join();
    }
    let status = child.wait();
    let stderr_text = stderr_reader
        .and_then(|t| t.join().ok())
        .unwrap_or_default();
    if !state.completed {
        let ok = status.map(|s| s.success()).unwrap_or(false);
        if ok {
            emit_stream_event(
                pipe_writer,
                &HarnessStreamEvent {
                    turn_id: turn_id.to_string(),
                    harness: "claude_code".to_string(),
                    kind: "completed".to_string(),
                    session_id: state.session_id,
                    text: None,
                    tool_name: None,
                    error: None,
                },
            );
        } else {
            let err = if stderr_text.trim().is_empty() {
                "Claude Code 进程异常退出".to_string()
            } else {
                stderr_text.trim().chars().take(280).collect()
            };
            emit_stream_event(
                pipe_writer,
                &HarnessStreamEvent {
                    turn_id: turn_id.to_string(),
                    harness: "claude_code".to_string(),
                    kind: "error".to_string(),
                    session_id: state.session_id,
                    text: None,
                    tool_name: None,
                    error: Some(err),
                },
            );
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExternalTurnRequest {
    pub harness: HarnessKind,
    pub turn_id: String,
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
}

/// Dispatches an external harness turn via the medium-integrity broker and streams NDJSON events
/// straight into the Tauri `bar` WebView via `window.__DSB_HARNESS_EVENT__(evt)`.
pub fn start_external_harness_turn(
    app: AppHandle,
    broker: std::sync::Arc<BrokerClient>,
    req_in: ExternalTurnRequest,
) -> Result<(), String> {
    let harness = req_in.harness;
    let turn_id = req_in.turn_id;
    let exe = match harness {
        HarnessKind::Antigravity => {
            // Desktop conversations deliver via official `agentapi` sidecar even if `agy.exe` CLI is absent.
            match resolve_agy_exe() {
                Some(p) => p,
                None if resolve_antigravity_desktop_exe().is_some() => PathBuf::new(),
                None => return Err("未检测到本机 Antigravity 桌面端或 CLI (agy.exe)".to_string()),
            }
        }
        HarnessKind::Codex => {
            resolve_codex_exe().ok_or_else(|| "未检测到本机 Codex (codex.exe)".to_string())?
        }
        HarnessKind::ClaudeCode => {
            resolve_claude_exe().ok_or_else(|| "未检测到本机 Claude Code CLI (claude.exe)".to_string())?
        }
        HarnessKind::Dsh => return Err("DSH 请使用原生 WebView 发送".to_string()),
    };

    let (pipe_short, server_pipe) = broker
        .create_oneshot_read_pipe(&turn_id)
        .map_err(|e| format!("创建流式管道失败: {e}"))?;

    let req = BrokerRequest::StartHarnessTurn {
        turn_id: turn_id.clone(),
        harness: harness.as_str().to_string(),
        exe: exe.to_string_lossy().to_string(),
        acp_args: Vec::new(),
        cwd: req_in.cwd,
        project_id: req_in.project_id,
        session_id: req_in.session_id,
        permission_mode: req_in.permission_mode,
        model: req_in.model,
        effort: req_in.effort,
        sandbox_mode: req_in.sandbox_mode,
        execution_mode: req_in.execution_mode,
        fast_speed: req_in.fast_speed,
        plan_mode: req_in.plan_mode,
        sandbox_enabled: req_in.sandbox_enabled,
        chrome_enabled: req_in.chrome_enabled,
        enabled_plugins: req_in.enabled_plugins,
        attachments: req_in.attachments,
        prompt: req_in.prompt,
        out_pipe: pipe_short,
    };

    let resp = broker.call(req);
    if !resp.ok {
        return Err(resp
            .error
            .unwrap_or_else(|| "启动普通权限 Harness 子进程失败".to_string()));
    }

    std::thread::Builder::new()
        .name(format!("harness-stream-{}", harness.as_str()))
        .spawn(move || {
            let Ok(reader) = server_pipe.accept_and_read() else {
                let err_evt = HarnessStreamEvent {
                    turn_id: turn_id.clone(),
                    harness: harness.as_str().to_string(),
                    kind: "error".to_string(),
                    session_id: None,
                    text: None,
                    tool_name: None,
                    error: Some("流式管道未能连接".to_string()),
                };
                push_harness_event_to_bar(&app, &err_evt);
                return;
            };
            for line in reader.lines().map_while(Result::ok) {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Ok(evt) = serde_json::from_str::<HarnessStreamEvent>(trimmed) {
                    if matches!(evt.kind.as_str(), "error" | "completed" | "tool" | "session") {
                        log::info!(
                            "[HARNESS_EVT] harness={} turn={} kind={} session={:?} tool={:?} err={:?}",
                            evt.harness,
                            evt.turn_id,
                            evt.kind,
                            evt.session_id,
                            evt.tool_name,
                            evt.error
                        );
                    }
                    push_harness_event_to_bar(&app, &evt);
                }
            }
        })
        .map_err(|e| format!("启动流式读取线程失败: {e}"))?;

    Ok(())
}

pub fn start_acp_harness_turn(
    app: AppHandle,
    broker: std::sync::Arc<BrokerClient>,
    harness_id: String,
    spec: crate::acp::AcpAgentSpec,
    turn_id: String,
    cwd: Option<String>,
    session_id: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    mode: Option<String>,
    attachments: Vec<TurnAttachment>,
    prompt: String,
) -> Result<(), String> {
    if !spec.available {
        return Err(format!("未找到 ACP 命令：{}", spec.command));
    }
    let (pipe_short, server_pipe) = broker
        .create_oneshot_read_pipe(&turn_id)
        .map_err(|e| format!("创建流式管道失败: {e}"))?;

    let req = BrokerRequest::StartHarnessTurn {
        turn_id: turn_id.clone(),
        harness: harness_id.clone(),
        exe: spec.command,
        acp_args: spec.args,
        cwd: cwd.or(spec.default_cwd),
        project_id: None,
        session_id,
        permission_mode: mode.clone(),
        model,
        effort,
        sandbox_mode: None,
        execution_mode: mode,
        fast_speed: None,
        plan_mode: None,
        sandbox_enabled: None,
        chrome_enabled: None,
        enabled_plugins: Vec::new(),
        attachments,
        prompt,
        out_pipe: pipe_short,
    };

    let resp = broker.call(req);
    if !resp.ok {
        return Err(resp
            .error
            .unwrap_or_else(|| "启动 ACP Agent 进程失败".to_string()));
    }

    std::thread::Builder::new()
        .name(format!("acp-stream-{}", crate::acp::normalize_acp_slug(&harness_id)))
        .spawn(move || {
            let Ok(reader) = server_pipe.accept_and_read() else {
                let err_evt = HarnessStreamEvent {
                    turn_id: turn_id.clone(),
                    harness: harness_id.clone(),
                    kind: "error".to_string(),
                    session_id: None,
                    text: None,
                    tool_name: None,
                    error: Some("ACP 流式管道未能连接".to_string()),
                };
                push_harness_event_to_bar(&app, &err_evt);
                return;
            };
            for line in reader.lines().map_while(Result::ok) {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Ok(evt) = serde_json::from_str::<HarnessStreamEvent>(trimmed) {
                    if matches!(evt.kind.as_str(), "error" | "completed" | "tool" | "session") {
                        log::info!(
                            "[HARNESS_EVT] harness={} turn={} kind={} session={:?} tool={:?} err={:?}",
                            evt.harness,
                            evt.turn_id,
                            evt.kind,
                            evt.session_id,
                            evt.tool_name,
                            evt.error
                        );
                    }
                    push_harness_event_to_bar(&app, &evt);
                }
            }
        })
        .map_err(|e| format!("启动 ACP 流式读取线程失败: {e}"))?;

    Ok(())
}

