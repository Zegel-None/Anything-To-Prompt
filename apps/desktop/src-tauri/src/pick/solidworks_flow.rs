use super::*;

pub(super) fn commit_solidworks_v2_capsule(
    app: &AppHandle,
    shared: &AppShared,
    sw: &solidworks::SwTargetV2Payload,
    pick_rect: Option<ScreenRect>,
    raw_cap: Option<&RawScreenCapture>,
) {
    let cfg = shared.config.lock().map(|c| c.clone()).unwrap_or_default();
    let n = shared.allocate_pick_number();
    let badge = circled_badge(n);

    if let Some(rect) = pick_rect {
        if let Some(ov) = app.get_webview_window("overlay") {
            let _ = ov.emit("pen:picked", serde_json::json!({ "n": n, "rect": rect }));
        }
    }

    let screenshot_path = raw_cap.and_then(|raw| {
        save_annotated_capture(app, raw, pick_rect, &[], n, &cfg)
            .ok()
            .map(|p| p.to_string_lossy().to_string())
    });

    let (chip_title, chip_detail) = solidworks::build_solidworks_chip_title(&badge, sw);

    let mut lines = vec![format!("[DSB-PICK #{n} | kind=solidworks_target]")];
    lines.extend(solidworks::format_solidworks_capsule_lines(sw));
    if let Some(sp) = screenshot_path.as_deref() {
        lines.push(format!(
            "debug_screenshot_path: {sp} (仅当以上结构化信息不足或可疑时再打开查看)"
        ));
    }
    lines.push("[/DSB-PICK]".to_string());

    let body_preview = sw
        .targets
        .first()
        .map(|it| {
            let d = &it.details;
            if let Some(en) = d
                .get("entity_name")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                format!("{} · {en}", it.select_type_name)
            } else if !it.display_name.is_empty() {
                format!("{} · {}", it.select_type_name, it.display_name)
            } else {
                it.select_type_name.clone()
            }
        })
        .unwrap_or_else(|| format!("SolidWorks · {}", sw.doc_context.doc.title));

    let capsule = CapsulePayload {
        summary: CapsuleQuoteSummary {
            label: badge,
            title: truncate_chars(&chip_title, 44),
            detail: truncate_chars(&chip_detail, 44),
            body: truncate_chars(&body_preview, 110),
        },
        text: lines.join("\n"),
    };
    log::info!(
        "Committed SolidWorks v2 Pick #{n}: mode={} targets={} title='{}' screenshot={:?}",
        sw.pick_mode,
        sw.targets.len(),
        chip_title,
        screenshot_path
    );
    inject_capsule_into_bar(app, &capsule);
}

/// Auto-attaches SolidWorks pre-selected entities (features, faces, edges, components, etc.)
/// immediately when the user summons the Bar (`DoubleCtrlOpen`) or enters Pen Hold (`PenStart`),
/// strictly deduplicated per-entity against `attached_sw_entities` so individually picked items
/// are NEVER re-dumped as `等N项` when reopening the Bar.
pub fn maybe_auto_attach_solidworks_selection(app: &AppHandle, fg_hwnd_raw: isize) {
    if !adapter_plugins::is_builtin_adapter_enabled("solidworks") {
        return;
    }
    if fg_hwnd_raw == 0 {
        return;
    }
    let hwnd = HWND(fg_hwnd_raw as *mut std::ffi::c_void);
    let Some(hit) = solidworks::detect_solidworks_hwnd(hwnd) else {
        return;
    };
    let Some(shared) = app.try_state::<AppShared>() else {
        return;
    };

    let outcome = solidworks::pick_solidworks_v2(
        &shared.broker,
        None,
        solidworks::SwPickMode::CurrentSelectionOnly,
    );
    let solidworks::SwProbeOutcome::Ok(mut sw) = outcome else {
        return;
    };
    if sw.targets.is_empty() {
        return;
    }

    let doc_key = solidworks::payload_doc_key(&sw).to_string();
    let new_targets: Vec<solidworks::SwTargetV2Item> = {
        let Ok(mut attached) = shared.attached_sw_entities.lock() else {
            return;
        };
        let mut kept = Vec::new();
        for mut item in sw.targets.into_iter() {
            let ek = solidworks::target_entity_key(&doc_key, &item);
            if attached.insert(ek) {
                item.index = kept.len() + 1;
                kept.push(item);
            }
        }
        kept
    };
    if new_targets.is_empty() {
        return;
    }
    sw.targets = new_targets;

    let win_rc = get_window_screen_rect(hit.root_hwnd);
    let center_x = win_rc.x + win_rc.w / 2;
    let center_y = win_rc.y + win_rc.h / 2;
    let raw_cap = capture_target_window_bitblt(hit.root_hwnd, center_x, center_y).ok();
    commit_solidworks_v2_capsule(app, &shared, &sw, None, raw_cap.as_ref());
    emit_pick_toast(
        app,
        &format!("已加入 SolidWorks 当前选中的 {} 项", sw.targets.len()),
    );
}

pub(super) fn try_execute_solidworks_tap_pick(
    app: &AppHandle,
    shared: &AppShared,
    x: i32,
    y: i32,
    pre_capture: Option<&RawScreenCapture>,
) -> bool {
    if !adapter_plugins::is_builtin_adapter_enabled("solidworks") {
        return false;
    }
    let Some(hit) = solidworks::detect_solidworks_at(x, y) else {
        return false;
    };

    // P0-1: Zero UIA on SLDWORKS.exe — classify region strictly via Win32 WindowFromPoint + GetClassNameW.
    let region = solidworks::classify_solidworks_region_at(hit.root_hwnd, x, y);

    // For SolidWorks buttons, options, checkboxes, PropertyManager fields, CommandManager toolbars,
    // menus, and dialogs, pick via zero-UIA Win32 HWND metadata + annotated screenshot!
    if matches!(
        region,
        solidworks::SwWindowRegion::ToolbarOrMenu | solidworks::SwWindowRegion::Other
    ) {
        if let Some(mut ctrl) = solidworks::inspect_win32_ui_control_at(hit.root_hwnd, x, y, true) {
            // Do NOT invalidate HITMAP_CACHE on swallowed Selection Mode tap picks — clicks never reach
            // SolidWorks during Selection Mode, so keeping woken OmniParser boxes alive makes multi-pick instant!
            let enriched_fast = crate::pick::enricher::enrich_box_semantics(
                HWND(ctrl.hwnd_raw as *mut std::ffi::c_void),
                hit.pid,
                "sldworks.exe",
                ctrl.rect,
                &ctrl.text,
                &ctrl.description,
                &ctrl.role,
                None,
                false,
            );
            if !enriched_fast.label.is_empty() {
                ctrl.text = enriched_fast.label;
            }
            ctrl.description = enriched_fast.description;

            let cfg = shared.config.lock().map(|c| c.clone()).unwrap_or_default();
            let n = shared.allocate_pick_number();
            let badge = circled_badge(n);

            if let Some(ov) = app.get_webview_window("overlay") {
                let _ = ov.emit(
                    "pen:picked",
                    serde_json::json!({ "n": n, "rect": ctrl.rect }),
                );
            }

            let owned_cap;
            let raw_ref = match pre_capture {
                Some(r) => Some(r),
                None => {
                    owned_cap = capture_target_window_bitblt(hit.root_hwnd, x, y).ok();
                    owned_cap.as_ref()
                }
            };

            let screenshot_path = raw_ref.and_then(|raw| {
                save_annotated_capture(app, raw, Some(ctrl.rect), &[], n, &cfg)
                    .ok()
                    .map(|p| p.to_string_lossy().to_string())
            });

            let needs_progressive_ocr =
                solidworks::is_generic_sw_container_label(&ctrl.text) && raw_ref.is_some();
            let capsule_id = format!(
                "cap-sw-ui-{n}-{}",
                crate::pick::profile::now_unix_secs()
            );

            let initial_label = if needs_progressive_ocr {
                "按钮 · 识别中…".to_string()
            } else {
                ctrl.display_label()
            };
            let mut lines = vec![
                format!("[DSB-PICK #{n} | kind=solidworks_ui]"),
                format!(
                    "SolidWorks 界面控件: role={} label=\"{}\" class=\"{}\" ctrl_id={} rect=({},{},{}x{})",
                    ctrl.role,
                    initial_label,
                    ctrl.class_name,
                    ctrl.ctrl_id,
                    ctrl.rect.x,
                    ctrl.rect.y,
                    ctrl.rect.w,
                    ctrl.rect.h
                ),
            ];
            if !ctrl.description.is_empty() {
                lines.push(format!("description: \"{}\"", ctrl.description));
            }
            if let Some(chk) = ctrl.checked {
                lines.push(format!("checked: {chk}"));
            }
            if !ctrl.parent_chain.is_empty() {
                lines.push(format!("panel_chain: {}", ctrl.parent_chain.join(" > ")));
            }
            if !hit.win_title.trim().is_empty() {
                lines.push(format!("window: \"{}\"", hit.win_title.trim()));
            }
            if ctrl.is_composite {
                lines.push(format!(
                    "cursor_screen_px: ({x}, {y}) (复合工具栏/菜单区域，请结合截图确认具体图标或菜单项)"
                ));
            }
            if let Some(sp) = screenshot_path.as_deref() {
                lines.push(format!("screenshot_path: {sp}"));
            }
            lines.push("[/DSB-PICK]".to_string());

            let detail = ctrl
                .parent_chain
                .first()
                .map(|p| format!("SolidWorks · {p}"))
                .unwrap_or_else(|| format!("SolidWorks · {}", ctrl.role_zh()));
            let capsule = CapsulePayload {
                summary: CapsuleQuoteSummary {
                    label: badge.clone(),
                    title: truncate_chars(&format!("{badge} {initial_label}"), 44),
                    detail: truncate_chars(&detail, 44),
                    body: truncate_chars(&format!("{} ({})", initial_label, ctrl.class_name), 110),
                },
                text: lines.join("\n"),
            };
            log::info!(
                "Committed SolidWorks Win32 UI Pick #{n}: role={} class={} label='{}' progressive={} screenshot={:?}",
                ctrl.role,
                ctrl.class_name,
                initial_label,
                needs_progressive_ocr,
                screenshot_path
            );
            inject_capsule_into_bar_with_meta(
                app,
                &capsule,
                Some(&capsule_id),
                Some(serde_json::json!({
                    "exe": "sldworks.exe",
                    "containerClass": ctrl.class_name,
                })),
            );
            if needs_progressive_ocr {
                emit_pick_toast(app, "已抓取按钮 · 后台补全名称…");
                if let Some(raw_cloned) = raw_ref.cloned() {
                    let app_bg = app.clone();
                    let cap_id_bg = capsule_id.clone();
                    let ctrl_rect = ctrl.rect;
                    let is_exact = !ctrl.is_composite;
                    let badge_bg = badge.clone();
                    let ctrl_role = ctrl.role.clone();
                    let ctrl_class = ctrl.class_name.clone();
                    let ctrl_id = ctrl.ctrl_id;
                    let ctrl_checked = ctrl.checked;
                    let ctrl_parents = ctrl.parent_chain.clone();
                    let ctrl_role_zh = ctrl.role_zh().to_string();
                    let ctrl_desc = ctrl.description.clone();
                    let win_title_bg = hit.win_title.clone();
                    let sp_bg = screenshot_path.clone();
                    let root_raw = hit.root_hwnd.0 as isize;
                    let ctrl_hwnd_raw = ctrl.hwnd_raw;
                    let sw_pid = hit.pid;
                    let _ = std::thread::Builder::new()
                        .name(format!("atp-sw-ocr-{n}"))
                        .spawn(move || {
                            let enriched_bg = crate::pick::enricher::enrich_box_semantics(
                                windows::Win32::Foundation::HWND(
                                    ctrl_hwnd_raw as *mut std::ffi::c_void,
                                ),
                                sw_pid,
                                "sldworks.exe",
                                ctrl_rect,
                                "",
                                &ctrl_desc,
                                &ctrl_role,
                                Some(&raw_cloned),
                                true,
                            );
                            let ocr_opt = if !enriched_bg.label.is_empty() {
                                Some(enriched_bg.label)
                            } else {
                                solidworks::ocr_crop_from_raw_capture(
                                    &raw_cloned,
                                    ctrl_rect,
                                    (x, y),
                                    is_exact,
                                )
                            };
                            let desc = if enriched_bg.description.is_empty() {
                                solidworks::read_solidworks_status_bar_text(
                                    windows::Win32::Foundation::HWND(
                                        root_raw as *mut std::ffi::c_void,
                                    ),
                                )
                            } else {
                                enriched_bg.description
                            };
                            let final_label = ocr_opt
                                .filter(|s| !s.trim().is_empty())
                                .unwrap_or_else(|| format!("SolidWorks {ctrl_role_zh}"));

                            let mut plines = vec![
                                format!("[DSB-PICK #{n} | kind=solidworks_ui]"),
                                format!(
                                    "SolidWorks 界面控件: role={} label=\"{}\" class=\"{}\" ctrl_id={} rect=({},{},{}x{})",
                                    ctrl_role,
                                    final_label,
                                    ctrl_class,
                                    ctrl_id,
                                    ctrl_rect.x,
                                    ctrl_rect.y,
                                    ctrl_rect.w,
                                    ctrl_rect.h
                                ),
                            ];
                            if !desc.is_empty() {
                                plines.push(format!("description: \"{desc}\""));
                            }
                            if let Some(chk) = ctrl_checked {
                                plines.push(format!("checked: {chk}"));
                            }
                            if !ctrl_parents.is_empty() {
                                plines.push(format!("panel_chain: {}", ctrl_parents.join(" > ")));
                            }
                            if !win_title_bg.trim().is_empty() {
                                plines.push(format!("window: \"{}\"", win_title_bg.trim()));
                            }
                            if let Some(sp) = sp_bg.as_deref() {
                                plines.push(format!("screenshot_path: {sp}"));
                            }
                            plines.push("[/DSB-PICK]".to_string());

                            let pdetail = ctrl_parents
                                .first()
                                .map(|p| format!("SolidWorks · {p}"))
                                .unwrap_or_else(|| format!("SolidWorks · {ctrl_role_zh}"));
                            let patched = CapsulePayload {
                                summary: CapsuleQuoteSummary {
                                    label: badge_bg.clone(),
                                    title: truncate_chars(
                                        &format!("{badge_bg} {final_label}"),
                                        44,
                                    ),
                                    detail: truncate_chars(&pdetail, 44),
                                    body: truncate_chars(
                                        &format!("{} ({})", final_label, ctrl_class),
                                        110,
                                    ),
                                },
                                text: plines.join("\n"),
                            };
                            patch_capsule_in_bar(&app_bg, &cap_id_bg, &patched);
                        });
                }
            }
            return true;
        }
        return true;
    }

    // Because pre-existing selections are already auto-attached on DoubleCtrl / PenStart,
    // an explicit mouse click during Pen mode MUST pick what is actually under `(x, y)`:
    // - 3D viewport -> `ViewportRayOnly` (hits front face under cursor; ignores empty background)
    // - FeatureManager tree -> `FeatureTreeClick` (selects & reads the clicked tree node)
    let pick_mode = match region {
        solidworks::SwWindowRegion::Viewport3D => solidworks::SwPickMode::ViewportRayOnly,
        solidworks::SwWindowRegion::FeatureTree => solidworks::SwPickMode::FeatureTreeClick,
        solidworks::SwWindowRegion::Other | solidworks::SwWindowRegion::ToolbarOrMenu => {
            solidworks::SwPickMode::CurrentSelectionOnly
        }
    };
    let t_sw = std::time::Instant::now();
    let outcome = solidworks::pick_solidworks_v2(&shared.broker, Some((x, y)), pick_mode);
    let sw_elapsed_ms = t_sw.elapsed().as_millis();

    let cfg = shared.config.lock().map(|c| c.clone()).unwrap_or_default();
    let cursor_px = unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
            windows::Win32::UI::WindowsAndMessaging::SM_CXCURSOR,
        )
    }
    .max(24);
    let half_reticle = cursor_px / 2;
    let pick_rect = ScreenRect {
        x: x - half_reticle,
        y: y - half_reticle,
        w: cursor_px,
        h: cursor_px,
    };

    let sw = match outcome {
        solidworks::SwProbeOutcome::Ok(payload) => payload,
        solidworks::SwProbeOutcome::Busy => {
            log::warn!(
                "SolidWorks v2 Pick Busy: region={:?} mode={} pos=({},{}) elapsed={}ms",
                region,
                pick_mode.as_str(),
                x,
                y,
                sw_elapsed_ms
            );
            emit_pick_toast(app, "SolidWorks 正忙，请稍候再抓");
            return true;
        }
        solidworks::SwProbeOutcome::TimedOut => {
            log::warn!(
                "SolidWorks v2 Pick TimedOut: region={:?} mode={} pos=({},{}) elapsed={}ms",
                region,
                pick_mode.as_str(),
                x,
                y,
                sw_elapsed_ms
            );
            // FR-4: Hard timeout degrades gracefully to lightweight window screenshot capsule (zero UIA).
            let n = shared.allocate_pick_number();
            let badge = circled_badge(n);
            let owned_cap;
            let raw_ref = match pre_capture {
                Some(r) => Some(r),
                None => {
                    owned_cap = capture_target_window_bitblt(hit.root_hwnd, x, y).ok();
                    owned_cap.as_ref()
                }
            };
            let screenshot_path = raw_ref.and_then(|raw| {
                save_annotated_capture(app, raw, Some(pick_rect), &[], n, &cfg)
                    .ok()
                    .map(|p| p.to_string_lossy().to_string())
            });
            let mut lines = vec![
                format!("[DSB-PICK #{n} | kind=solidworks_target]"),
                format!(
                    "SolidWorks 探针超时降级截图: window=\"{}\"",
                    hit.win_title.trim()
                ),
            ];
            if let Some(sp) = screenshot_path.as_deref() {
                lines.push(format!("debug_screenshot_path: {sp}"));
            }
            lines.push("[/DSB-PICK]".to_string());
            let capsule = CapsulePayload {
                summary: CapsuleQuoteSummary {
                    label: badge.clone(),
                    title: format!("{badge} SolidWorks 视图截图"),
                    detail: "SolidWorks · 响应超时已降级".to_string(),
                    body: hit.win_title.trim().to_string(),
                },
                text: lines.join("\n"),
            };
            emit_pick_toast(app, "SolidWorks 响应超时，已降级为截图胶囊");
            inject_capsule_into_bar(app, &capsule);
            return true;
        }
        solidworks::SwProbeOutcome::Unavailable => {
            emit_pick_toast(app, "未检测到 SolidWorks 活动文档");
            return true;
        }
    };

    // Ignore empty background clicks in 3D viewport / FeatureTree (never re-add stale selections)
    if sw.targets.is_empty() {
        return true;
    }

    // Per-entity deduplication: never duplicate a face/edge/feature already in the Bar composer,
    // even if clicked at a slightly shifted 3D point on the same face!
    let mut sw = sw;
    let doc_key = solidworks::payload_doc_key(&sw).to_string();
    let new_targets: Vec<solidworks::SwTargetV2Item> = {
        let Ok(mut attached) = shared.attached_sw_entities.lock() else {
            return true;
        };
        let mut kept = Vec::new();
        for mut item in sw.targets.into_iter() {
            let ek = solidworks::target_entity_key(&doc_key, &item);
            if attached.insert(ek) {
                item.index = kept.len() + 1;
                kept.push(item);
            }
        }
        kept
    };
    if new_targets.is_empty() {
        emit_pick_toast(app, "该 SolidWorks 对象已在引用栏中");
        return true;
    }
    sw.targets = new_targets;

    let owned_cap;
    let raw_ref = match pre_capture {
        Some(r) => Some(r),
        None => {
            owned_cap = capture_target_window_bitblt(hit.root_hwnd, x, y).ok();
            owned_cap.as_ref()
        }
    };
    commit_solidworks_v2_capsule(app, shared, &sw, Some(pick_rect), raw_ref);
    true
}

