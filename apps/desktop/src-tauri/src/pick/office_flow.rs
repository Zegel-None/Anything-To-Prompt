use super::*;

pub(super) fn try_execute_office_tap_pick(
    app: &AppHandle,
    shared: &AppShared,
    x: i32,
    y: i32,
    pre_capture: Option<&RawScreenCapture>,
) -> bool {
    if !adapter_plugins::is_builtin_adapter_enabled("office") {
        return false;
    }
    let Some(hit) = office::detect_office_canvas_at(x, y) else {
        return false;
    };
    if !hit.in_canvas {
        return false;
    }

    match hit.kind {
        OfficeAppKind::Excel => {
            let Some(ex) = office::pick_excel_cell_com(
                &shared.broker,
                hit.root_hwnd,
                x,
                y,
                hit.dpi_scale,
            ) else {
                return false;
            };
            let cfg = shared.config.lock().map(|c| c.clone()).unwrap_or_default();
            let n = shared.allocate_pick_number();
            let badge = circled_badge(n);
            let rect = ex.screen_rect.unwrap_or(ScreenRect {
                x: x - 42,
                y: y - 13,
                w: 84,
                h: 26,
            });
            if let Some(ov) = app.get_webview_window("overlay") {
                let _ = ov.emit(
                    "pen:picked",
                    serde_json::json!({ "n": n, "rect": rect, "mode": "excel_cell" }),
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
                save_annotated_capture(app, raw, Some(rect), &[], n, &cfg)
                    .ok()
                    .map(|p| p.to_string_lossy().to_string())
            });
            let proc_meta = inspect_process(hit.pid, &cfg.app_source_map);
            let app_label = office_app_display_name(&proc_meta.exe_name, OfficeAppKind::Excel);
            let cell_ref = if ex.sheet.is_empty() {
                ex.address.clone()
            } else {
                format!("{}!{}", ex.sheet, ex.address)
            };
            let disp_short = truncate_chars(ex.display_text.trim(), 16);
            let chip_title = if disp_short.is_empty() {
                format!("{badge} {cell_ref}")
            } else {
                format!("{badge} {cell_ref} · {disp_short}")
            };
            let wb_badge = if !ex.workbook_name.is_empty() {
                ex.workbook_name.as_str()
            } else {
                hit.win_title.trim()
            };
            let chip_detail = format!("{app_label} · {}", truncate_chars(wb_badge, 26));

            let mut lines = Vec::new();
            lines.push(format!(
                "[DSB-PICK #{n} badge={badge} kind=office_excel_cell app=\"{}\"]",
                proc_meta.exe_name
            ));
            lines.push(format!("office_app: {app_label}"));
            if !ex.prog_id.is_empty() {
                lines.push(format!("office_prog_id: {}", ex.prog_id));
            }
            lines.push(format!("window_hwnd: {}", hit.root_hwnd.0 as isize));
            if !ex.full_name.is_empty() {
                lines.push(format!("office_document_path: {}", ex.full_name));
            }
            lines.push(format!("office_workbook: {}", ex.workbook_name));
            lines.push(format!("excel_sheet: {}", ex.sheet));
            lines.push(format!("excel_address: {}", ex.address));
            if !ex.col_header.is_empty() {
                lines.push(format!("excel_col_header: {}", ex.col_header));
            }
            if !ex.row_header.is_empty() {
                lines.push(format!("excel_row_header: {}", ex.row_header));
            }
            if !ex.display_text.is_empty() {
                lines.push(format!("excel_display_text: {}", ex.display_text));
            }
            if !ex.value.is_empty() && ex.value != ex.display_text {
                lines.push(format!("excel_raw_value: {}", ex.value));
            }
            if !ex.formula.is_empty() {
                lines.push(format!("excel_formula: {}", ex.formula));
            }
            let target_locator = if ex.full_name.is_empty() {
                format!("{} | {cell_ref}", ex.workbook_name)
            } else {
                format!("{} | {cell_ref}", ex.full_name)
            };
            lines.extend(office::format_live_edit_guidance(
                "excel",
                is_wps_exe(&proc_meta.exe_name),
                &target_locator,
            ));
            let win_rect = get_window_screen_rect(hit.root_hwnd);
            let mon = MonitorSpace::for_point(x, y, hit.root_hwnd);
            lines.extend(grounding::format_tap_grounding(
                hit.root_hwnd,
                win_rect,
                &mon,
                (x, y),
                rect,
                "ExcelCell",
                None,
            ));
            if let Some(sp) = screenshot_path.as_deref() {
                lines.push(format!(
                    "debug_screenshot_path: {sp} (仅当以上信息不足或可疑时再打开查看)"
                ));
            }
            lines.push("[/DSB-PICK]".to_string());

            let capsule = CapsulePayload {
                summary: CapsuleQuoteSummary {
                    label: badge,
                    title: chip_title,
                    detail: chip_detail,
                    body: format!("{cell_ref} = {}", ex.display_text),
                },
                text: lines.join("\n"),
            };
            log::info!(
                "Committed Excel Cell Pick #{n}: ref='{cell_ref}' val='{}' screenshot={:?}",
                truncate_chars(&ex.display_text, 40),
                screenshot_path
            );
            inject_capsule_into_bar(app, &capsule);
            true
        }
        OfficeAppKind::PowerPoint => {
            let Some(ppt) = office::pick_ppt_element_com(
                &shared.broker,
                hit.root_hwnd,
                x,
                y,
                hit.dpi_scale,
            ) else {
                return false;
            };
            let cfg = shared.config.lock().map(|c| c.clone()).unwrap_or_default();
            let n = shared.allocate_pick_number();
            let badge = circled_badge(n);
            let rect = ppt.screen_rect.unwrap_or(ScreenRect {
                x: x - 48,
                y: y - 24,
                w: 96,
                h: 48,
            });
            if let Some(ov) = app.get_webview_window("overlay") {
                let _ = ov.emit(
                    "pen:picked",
                    serde_json::json!({ "n": n, "rect": rect, "mode": "ppt_element" }),
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
                save_annotated_capture(app, raw, Some(rect), &[], n, &cfg)
                    .ok()
                    .map(|p| p.to_string_lossy().to_string())
            });
            let proc_meta = inspect_process(hit.pid, &cfg.app_source_map);
            let app_label = office_app_display_name(&proc_meta.exe_name, OfficeAppKind::PowerPoint);
            let first_line = ppt
                .text
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("");
            let elem_label = if !first_line.is_empty() {
                truncate_chars(first_line, 20)
            } else if !ppt.shape_name.is_empty() {
                truncate_chars(&ppt.shape_name, 22)
            } else {
                ppt.shape_type.clone()
            };
            let chip_title = format!("{badge} Slide {} · {elem_label}", ppt.slide_index);
            let pres_badge = if !ppt.pres_name.is_empty() {
                ppt.pres_name.as_str()
            } else {
                hit.win_title.trim()
            };
            let chip_detail = format!(
                "{app_label} · {} ({})",
                truncate_chars(pres_badge, 22),
                ppt.shape_type
            );

            let mut lines = Vec::new();
            lines.push(format!(
                "[DSB-PICK #{n} badge={badge} kind=office_ppt_element app=\"{}\"]",
                proc_meta.exe_name
            ));
            lines.push(format!("office_app: {app_label}"));
            if !ppt.prog_id.is_empty() {
                lines.push(format!("office_prog_id: {}", ppt.prog_id));
            }
            lines.push(format!("window_hwnd: {}", hit.root_hwnd.0 as isize));
            if !ppt.full_name.is_empty() {
                lines.push(format!("office_document_path: {}", ppt.full_name));
            }
            lines.push(format!("ppt_presentation: {}", ppt.pres_name));
            lines.push(format!(
                "ppt_slide: {} / {}",
                ppt.slide_index, ppt.slide_count
            ));
            lines.push(format!(
                "ppt_shape: id={} name=\"{}\" type={}",
                ppt.shape_id, ppt.shape_name, ppt.shape_type
            ));
            if let Some(gid) = ppt.parent_group_id {
                lines.push(format!(
                    "ppt_parent_group: id={gid} name=\"{}\"",
                    ppt.parent_group_name
                ));
            }
            if !ppt.geometry.is_empty() {
                lines.push(format!("ppt_geometry: {}", ppt.geometry));
            }
            if !ppt.style.is_empty() {
                lines.push(format!("ppt_style: {}", ppt.style));
            }
            if !ppt.text.is_empty() {
                lines.push(format!("ppt_shape_text:\n{}", ppt.text));
            }
            let mut target_locator = format!(
                "{} | slide={} shape_id={} shape_name=\"{}\"",
                if ppt.full_name.is_empty() {
                    ppt.pres_name.as_str()
                } else {
                    ppt.full_name.as_str()
                },
                ppt.slide_index,
                ppt.shape_id,
                ppt.shape_name
            );
            if let Some(gid) = ppt.parent_group_id {
                target_locator.push_str(&format!(" parent_group_id={gid}"));
            }
            lines.extend(office::format_live_edit_guidance(
                "ppt",
                is_wps_exe(&proc_meta.exe_name),
                &target_locator,
            ));
            let win_rect = get_window_screen_rect(hit.root_hwnd);
            let mon = MonitorSpace::for_point(x, y, hit.root_hwnd);
            lines.extend(grounding::format_tap_grounding(
                hit.root_hwnd,
                win_rect,
                &mon,
                (x, y),
                rect,
                &ppt.shape_type,
                None,
            ));
            if let Some(sp) = screenshot_path.as_deref() {
                lines.push(format!(
                    "debug_screenshot_path: {sp} (仅当以上信息不足或可疑时再打开查看)"
                ));
            }
            lines.push("[/DSB-PICK]".to_string());

            let capsule = CapsulePayload {
                summary: CapsuleQuoteSummary {
                    label: badge,
                    title: chip_title,
                    detail: chip_detail,
                    body: if ppt.text.is_empty() {
                        format!("{} ({})", ppt.shape_name, ppt.shape_type)
                    } else {
                        truncate_chars(&ppt.text, 120)
                    },
                },
                text: lines.join("\n"),
            };
            log::info!(
                "Committed PPT Element Pick #{n}: slide={} shape='{}' (id={}) screenshot={:?}",
                ppt.slide_index,
                ppt.shape_name,
                ppt.shape_id,
                screenshot_path
            );
            inject_capsule_into_bar(app, &capsule);
            true
        }
        OfficeAppKind::Word => false,
    }
}

pub(super) fn try_execute_office_drag_pick(
    app: &AppHandle,
    shared: &AppShared,
    points: &[(i32, i32)],
    pre_capture: Option<&RawScreenCapture>,
) -> bool {
    if points.len() < 2 {
        return false;
    }
    if !adapter_plugins::is_builtin_adapter_enabled("office") {
        return false;
    }
    let start = points[0];
    let end = *points.last().unwrap_or(&start);
    let Some(hit) = office::detect_office_canvas_at(start.0, start.1) else {
        return false;
    };
    if !hit.in_canvas {
        return false;
    }

    let bbox = ScreenRect {
        x: start.0.min(end.0),
        y: start.1.min(end.1),
        w: (start.0 - end.0).abs().max(24),
        h: (start.1 - end.1).abs().max(18),
    };

    match hit.kind {
        OfficeAppKind::PowerPoint => {
            // PowerPoint / WPS Presentation drag MUST keep drawing the red freehand stroke unchanged!
            false
        }
        OfficeAppKind::Word => {
            let uia_text = office::select_word_text_uia(hit.root_hwnd, start, end, true);
            let com_res = office::pick_word_text_com(
                &shared.broker,
                start.0,
                start.1,
                end.0,
                end.1,
                hit.dpi_scale,
            );
            let selected_text = com_res
                .as_ref()
                .map(|r| r.selected_text.trim())
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .or(uia_text)
                .unwrap_or_default();

            let cfg = shared.config.lock().map(|c| c.clone()).unwrap_or_default();
            let n = shared.allocate_pick_number();
            let badge = circled_badge(n);
            if let Some(ov) = app.get_webview_window("overlay") {
                let _ = ov.emit(
                    "pen:picked",
                    serde_json::json!({ "n": n, "rect": bbox, "mode": "word_text" }),
                );
            }
            let owned_cap;
            let raw_ref = match pre_capture {
                Some(r) => Some(r),
                None => {
                    owned_cap = capture_target_window_bitblt(
                        hit.root_hwnd,
                        bbox.x + bbox.w / 2,
                        bbox.y + bbox.h / 2,
                    )
                    .ok();
                    owned_cap.as_ref()
                }
            };
            // Save clean rectangular box screenshot (NO red freehand squiggle!)
            let screenshot_path = raw_ref.and_then(|raw| {
                save_annotated_capture(app, raw, Some(bbox), &[], n, &cfg)
                    .ok()
                    .map(|p| p.to_string_lossy().to_string())
            });
            let proc_meta = inspect_process(hit.pid, &cfg.app_source_map);
            let app_label = office_app_display_name(&proc_meta.exe_name, OfficeAppKind::Word);

            let (prog_id, full_name, doc_name, page_num, para_text, range_start, range_end) =
                match com_res {
                    Some(r) => (
                        r.prog_id,
                        r.full_name,
                        r.doc_name,
                        r.page_number,
                        r.paragraph_text,
                        r.range_start,
                        r.range_end,
                    ),
                    None => (
                        String::new(),
                        String::new(),
                        hit.win_title.trim().to_string(),
                        String::new(),
                        String::new(),
                        None,
                        None,
                    ),
                };

            let single_line = selected_text
                .replace(['\r', '\n'], " ")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let chip_title = if !single_line.is_empty() {
                format!("{badge} 「{}」", truncate_chars(&single_line, 20))
            } else {
                format!("{badge} {} 选区", truncate_chars(&doc_name, 18))
            };
            let doc_badge = if !doc_name.is_empty() {
                doc_name.as_str()
            } else {
                hit.win_title.trim()
            };
            let chip_detail = if !page_num.is_empty() {
                format!(
                    "{app_label} · {} · P{page_num}",
                    truncate_chars(doc_badge, 20)
                )
            } else {
                format!("{app_label} · {}", truncate_chars(doc_badge, 24))
            };

            let mut lines = Vec::new();
            lines.push(format!(
                "[DSB-PICK #{n} badge={badge} kind=office_word_text app=\"{}\"]",
                proc_meta.exe_name
            ));
            lines.push(format!("office_app: {app_label}"));
            if !prog_id.is_empty() {
                lines.push(format!("office_prog_id: {prog_id}"));
            }
            lines.push(format!("window_hwnd: {}", hit.root_hwnd.0 as isize));
            if !full_name.is_empty() {
                lines.push(format!("office_document_path: {full_name}"));
            }
            lines.push(format!("word_document: {doc_name}"));
            if !page_num.is_empty() {
                lines.push(format!("word_page: {page_num}"));
            }
            if let (Some(rs), Some(re)) = (range_start, range_end) {
                lines.push(format!("word_range_offsets: {rs}..{re}"));
            }
            if !selected_text.is_empty() {
                lines.push(format!("word_selected_text:\n{selected_text}"));
            } else {
                lines.push(
                    "word_selected_text: (未读取到文本字符，可能划选在空白/图片区域，请参考截图)"
                        .to_string(),
                );
            }
            if !para_text.is_empty() && para_text != selected_text {
                lines.push(format!("word_paragraph_context:\n{para_text}"));
            }
            let doc_id = if full_name.is_empty() {
                doc_name.as_str()
            } else {
                full_name.as_str()
            };
            let target_locator = match (range_start, range_end) {
                (Some(rs), Some(re)) => format!("{doc_id} | range={rs}..{re}"),
                _ => doc_id.to_string(),
            };
            lines.extend(office::format_live_edit_guidance(
                "word",
                is_wps_exe(&proc_meta.exe_name),
                &target_locator,
            ));
            let win_rect = get_window_screen_rect(hit.root_hwnd);
            let mon = MonitorSpace::for_point(start.0, start.1, hit.root_hwnd);
            lines.extend(grounding::format_drag_grounding(
                hit.root_hwnd,
                win_rect,
                &mon,
                start,
                end,
                bbox,
            ));
            if let Some(sp) = screenshot_path.as_deref() {
                lines.push(format!(
                    "debug_screenshot_path: {sp} (仅当以上信息不足或可疑时再打开查看)"
                ));
            }
            lines.push("[/DSB-PICK]".to_string());

            let capsule = CapsulePayload {
                summary: CapsuleQuoteSummary {
                    label: badge,
                    title: chip_title,
                    detail: chip_detail,
                    body: if !single_line.is_empty() {
                        truncate_chars(&single_line, 120)
                    } else {
                        format!("{app_label} — {}", truncate_chars(doc_badge, 80))
                    },
                },
                text: lines.join("\n"),
            };
            log::info!(
                "Committed Word Text Pick #{n}: chars={} range={:?}..{:?} screenshot={:?}",
                selected_text.chars().count(),
                range_start,
                range_end,
                screenshot_path
            );
            inject_capsule_into_bar(app, &capsule);
            true
        }
        OfficeAppKind::Excel => {
            let Some(ex) = office::pick_excel_range_com(
                &shared.broker,
                hit.root_hwnd,
                start.0,
                start.1,
                end.0,
                end.1,
                hit.dpi_scale,
            ) else {
                return false;
            };
            let cfg = shared.config.lock().map(|c| c.clone()).unwrap_or_default();
            let n = shared.allocate_pick_number();
            let badge = circled_badge(n);
            let rect = ex.screen_rect.unwrap_or(bbox);
            if let Some(ov) = app.get_webview_window("overlay") {
                let _ = ov.emit(
                    "pen:picked",
                    serde_json::json!({ "n": n, "rect": rect, "mode": "excel_box" }),
                );
            }
            let owned_cap;
            let raw_ref = match pre_capture {
                Some(r) => Some(r),
                None => {
                    owned_cap = capture_target_window_bitblt(
                        hit.root_hwnd,
                        rect.x + rect.w / 2,
                        rect.y + rect.h / 2,
                    )
                    .ok();
                    owned_cap.as_ref()
                }
            };
            // Save clean rectangular box screenshot (NO red freehand squiggle!)
            let screenshot_path = raw_ref.and_then(|raw| {
                save_annotated_capture(app, raw, Some(rect), &[], n, &cfg)
                    .ok()
                    .map(|p| p.to_string_lossy().to_string())
            });
            let proc_meta = inspect_process(hit.pid, &cfg.app_source_map);
            let app_label = office_app_display_name(&proc_meta.exe_name, OfficeAppKind::Excel);
            let range_ref = if ex.sheet.is_empty() {
                ex.address.clone()
            } else {
                format!("{}!{}", ex.sheet, ex.address)
            };
            let chip_title = if ex.is_range {
                format!("{badge} {range_ref} ({}×{})", ex.rows, ex.cols)
            } else {
                format!("{badge} {range_ref}")
            };
            let wb_badge = if !ex.workbook_name.is_empty() {
                ex.workbook_name.as_str()
            } else {
                hit.win_title.trim()
            };
            let chip_detail = format!("{app_label} · {}", truncate_chars(wb_badge, 26));

            let mut lines = Vec::new();
            lines.push(format!(
                "[DSB-PICK #{n} badge={badge} kind=office_excel_range app=\"{}\"]",
                proc_meta.exe_name
            ));
            lines.push(format!("office_app: {app_label}"));
            if !ex.prog_id.is_empty() {
                lines.push(format!("office_prog_id: {}", ex.prog_id));
            }
            lines.push(format!("window_hwnd: {}", hit.root_hwnd.0 as isize));
            if !ex.full_name.is_empty() {
                lines.push(format!("office_document_path: {}", ex.full_name));
            }
            lines.push(format!("office_workbook: {}", ex.workbook_name));
            lines.push(format!("excel_sheet: {}", ex.sheet));
            lines.push(format!(
                "excel_address: {} ({} rows × {} cols)",
                ex.address, ex.rows, ex.cols
            ));
            if !ex.table_preview.is_empty() {
                lines.push(format!("excel_table_preview:\n{}", ex.table_preview));
            }
            if !ex.formulas_preview.is_empty() {
                lines.push(format!("excel_formulas: {}", ex.formulas_preview));
            }
            let target_locator = if ex.full_name.is_empty() {
                format!("{} | {range_ref}", ex.workbook_name)
            } else {
                format!("{} | {range_ref}", ex.full_name)
            };
            lines.extend(office::format_live_edit_guidance(
                "excel",
                is_wps_exe(&proc_meta.exe_name),
                &target_locator,
            ));
            let win_rect = get_window_screen_rect(hit.root_hwnd);
            let mon = MonitorSpace::for_point(start.0, start.1, hit.root_hwnd);
            lines.extend(grounding::format_drag_grounding(
                hit.root_hwnd,
                win_rect,
                &mon,
                start,
                end,
                rect,
            ));
            if let Some(sp) = screenshot_path.as_deref() {
                lines.push(format!(
                    "debug_screenshot_path: {sp} (仅当以上信息不足或可疑时再打开查看)"
                ));
            }
            lines.push("[/DSB-PICK]".to_string());

            let capsule = CapsulePayload {
                summary: CapsuleQuoteSummary {
                    label: badge,
                    title: chip_title,
                    detail: chip_detail,
                    body: format!("{range_ref} ({}×{})", ex.rows, ex.cols),
                },
                text: lines.join("\n"),
            };
            log::info!(
                "Committed Excel Range Pick #{n}: ref='{range_ref}' ({}x{}) screenshot={:?}",
                ex.rows,
                ex.cols,
                screenshot_path
            );
            inject_capsule_into_bar(app, &capsule);
            true
        }
    }
}

