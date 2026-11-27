use super::*;

pub(super) fn open_sqlite_readonly(path: &Path) -> Option<Connection> {
    if !path.is_file() {
        return None;
    }
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()
}

pub(super) fn parse_first_workspace_uri(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if let Ok(arr) = serde_json::from_str::<Vec<String>>(trimmed) {
        if let Some(first) = arr.first() {
            return clean_windows_path(first);
        }
    }
    clean_windows_path(trimmed)
}

pub(super) fn is_antigravity_archived(home: &Path, conv_id: &str) -> bool {
    let pbtxt = home
        .join(".gemini")
        .join("antigravity")
        .join("annotations")
        .join(format!("{conv_id}.pbtxt"));
    let Ok(raw) = fs::read_to_string(&pbtxt) else {
        return false;
    };
    raw.contains("archived:true") || raw.contains("archived: true")
}

pub fn load_antigravity_catalog() -> ExternalHarnessCatalog {
    let exe = resolve_agy_exe();
    let Some(home) = dirs::home_dir() else {
        return ExternalHarnessCatalog {
            harness: "antigravity".to_string(),
            available: exe.is_some(),
            exe_path: exe.map(|p| p.to_string_lossy().to_string()),
            groups: Vec::new(),
            error: Some("无法定位用户目录".to_string()),
        };
    };

    let gemini_dir = home.join(".gemini");
    // Use Antigravity Desktop's authoritative DB only so archived/deleted CLI probes never pollute the list
    let db_path = gemini_dir
        .join("antigravity")
        .join("conversation_summaries.db");

    // Map project_id -> (label, path, Vec<ExternalConversation>)
    // Do NOT unconditionally inject a phantom "CLI Project" — only include projects that
    // actually have active (non-killed, non-archived) conversations in Antigravity's DB,
    // so deleted workspaces vanish immediately on refresh.
    let mut by_project: HashMap<String, (String, String, Vec<ExternalConversation>)> =
        HashMap::new();
    let mut seen_conv_ids: HashMap<String, usize> = HashMap::new();

    if let Some(conn) = open_sqlite_readonly(&db_path) {
        if let Ok(mut stmt) = conn.prepare(
            "SELECT conversation_id, COALESCE(title, ''), COALESCE(preview, ''), \
             COALESCE(CAST(strftime('%s', substr(last_modified_time, 1, 19)) AS INTEGER) * 1000, \
                      CAST(last_modified_time AS INTEGER), 0), \
             COALESCE(workspace_uris, ''), COALESCE(source, ''), \
             COALESCE(project_id, ''), COALESCE(parent_conversation_id, '') \
             FROM conversation_summaries \
             WHERE COALESCE(killed, 0) = 0 \
             ORDER BY last_modified_time DESC LIMIT 120",
        ) {
            if let Ok(rows) = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            }) {
                for row in rows.flatten() {
                    let (conv_id, title, preview, updated_at, ws_uris, source, project_id, parent_id) =
                        row;
                    if conv_id.trim().is_empty() || !parent_id.trim().is_empty() {
                        continue;
                    }
                    if seen_conv_ids.contains_key(&conv_id) {
                        continue;
                    }
                    seen_conv_ids.insert(conv_id.clone(), 1);

                    // Match Antigravity Desktop sidebar: exclude archived conversations (stored in annotations/*.pbtxt)
                    if is_antigravity_archived(&home, &conv_id) {
                        continue;
                    }

                    // Exclude headless CLI one-shot/probe runs (empty title + CLI source) hidden by Antigravity Desktop
                    if source.eq_ignore_ascii_case("CORTEX_TRAJECTORY_SOURCE_CLI")
                        && title.trim().is_empty()
                    {
                        continue;
                    }

                    let ws_path = parse_first_workspace_uri(&ws_uris);
                    let normalized_pid = if project_id.trim().is_empty() {
                        "default-cli-project".to_string()
                    } else {
                        project_id.trim().to_string()
                    };

                    let label = if normalized_pid == "default-cli-project" {
                        "CLI Project".to_string()
                    } else if normalized_pid == "outside-of-project" {
                        "自由对话".to_string()
                    } else if !ws_path.is_empty() {
                        basename_of_path(&ws_path)
                    } else {
                        format!("项目 {}", &normalized_pid[..normalized_pid.len().min(8)])
                    };

                    let display_title = if !title.trim().is_empty() {
                        sanitize_title(&title, "新对话")
                    } else if !preview.trim().is_empty() {
                        sanitize_title(&preview, "新对话")
                    } else {
                        format!("对话 {}", &conv_id[..conv_id.len().min(8)])
                    };

                    let updated_ms = if updated_at > 0 && updated_at < 10_000_000_000 {
                        updated_at * 1000
                    } else {
                        updated_at
                    };

                    let entry = by_project
                        .entry(normalized_pid.clone())
                        .or_insert_with(|| (label.clone(), ws_path.clone(), Vec::new()));
                    if entry.1.is_empty() && !ws_path.is_empty() {
                        entry.1 = ws_path.clone();
                        if normalized_pid != "default-cli-project"
                            && normalized_pid != "outside-of-project"
                        {
                            entry.0 = basename_of_path(&ws_path);
                        }
                    }
                    entry.2.push(ExternalConversation {
                        id: conv_id,
                        title: display_title,
                        updated_at: updated_ms,
                        source,
                        cwd: ws_path,
                        rollout_path: None,
                    });
                }
            }
        }
    }

    let mut groups: Vec<ExternalWorkspaceGroup> = by_project
        .into_iter()
        .map(|(workspace_id, (label, path, mut convs))| {
            convs.sort_by_key(|c| std::cmp::Reverse(c.updated_at));
            ExternalWorkspaceGroup {
                workspace_id,
                label,
                path,
                convs,
            }
        })
        .collect();

    // Put real Desktop projects on top (newest first) and CLI Project at the bottom — matching Antigravity Desktop
    groups.sort_by(|a, b| {
        let a_cli = a.workspace_id == "default-cli-project";
        let b_cli = b.workspace_id == "default-cli-project";
        a_cli.cmp(&b_cli).then_with(|| {
            let a_ts = a.convs.first().map(|c| c.updated_at).unwrap_or(0);
            let b_ts = b.convs.first().map(|c| c.updated_at).unwrap_or(0);
            b_ts.cmp(&a_ts)
        })
    });

    ExternalHarnessCatalog {
        harness: "antigravity".to_string(),
        available: exe.is_some(),
        exe_path: exe.map(|p| p.to_string_lossy().to_string()),
        groups,
        error: None,
    }
}

/// Read-only lookup of an Antigravity conversation's authoritative metadata
/// `(workspace_id, workspace_path, workspace_label, title)` from `conversation_summaries.db`.
/// Prevents real-time hook / JSONL tail updates from overwriting the real conversation title
/// (e.g. "DeepSeek Bar UI Bug Fix") with a raw user message snippet.
pub fn lookup_antigravity_conversation_meta(
    conv_id: &str,
) -> Option<(String, String, String, String)> {
    let cid = conv_id.trim();
    if cid.is_empty() {
        return None;
    }
    let home = dirs::home_dir()?;
    let gemini_dir = home.join(".gemini");
    let dbs = [
        gemini_dir
            .join("antigravity")
            .join("conversation_summaries.db"),
        gemini_dir
            .join("antigravity-cli")
            .join("conversation_summaries.db"),
    ];

    for db_path in dbs {
        let Some(conn) = open_sqlite_readonly(&db_path) else {
            continue;
        };
        let row: rusqlite::Result<(String, String, String, String)> = conn.query_row(
            "SELECT COALESCE(title, ''), COALESCE(preview, ''), \
             COALESCE(workspace_uris, ''), COALESCE(project_id, '') \
             FROM conversation_summaries WHERE conversation_id = ?1 LIMIT 1",
            [cid],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        );
        if let Ok((title, preview, ws_uris, project_id)) = row {
            let ws_path = parse_first_workspace_uri(&ws_uris);
            let normalized_pid = if project_id.trim().is_empty() {
                "default-cli-project".to_string()
            } else {
                project_id.trim().to_string()
            };
            let label = if normalized_pid == "default-cli-project" {
                "CLI Project (桌面端同步)".to_string()
            } else if normalized_pid == "outside-of-project" {
                "自由对话".to_string()
            } else if !ws_path.is_empty() {
                basename_of_path(&ws_path)
            } else {
                format!("项目 {}", &normalized_pid[..normalized_pid.len().min(8)])
            };
            let display_title = if !title.trim().is_empty() {
                sanitize_title(&title, "新对话")
            } else if !preview.trim().is_empty() {
                sanitize_title(&preview, "新对话")
            } else {
                String::new()
            };
            if !display_title.is_empty() || !ws_path.is_empty() {
                return Some((normalized_pid, ws_path, label, display_title));
            }
        }
    }
    None
}

/// Returns the most recently active Antigravity Desktop project_id (excluding CLI & free chats),
/// used to pre-populate the official Plugin Sidecar's default `projectId` if unset.
pub fn latest_antigravity_desktop_project_id() -> Option<String> {
    let home = dirs::home_dir()?;
    let db_path = home
        .join(".gemini")
        .join("antigravity")
        .join("conversation_summaries.db");
    let conn = open_sqlite_readonly(&db_path)?;
    conn.query_row(
        "SELECT project_id FROM conversation_summaries \
         WHERE COALESCE(killed, 0) = 0 \
           AND COALESCE(parent_conversation_id, '') = '' \
           AND COALESCE(project_id, '') != '' \
           AND project_id != 'default-cli-project' \
           AND project_id != 'outside-of-project' \
         ORDER BY last_modified_time DESC LIMIT 1",
        [],
        |r| r.get::<_, String>(0),
    )
    .ok()
    .filter(|s| !s.trim().is_empty())
}

pub(super) fn is_antigravity_cli_conversation(session_id: &str) -> bool {
    let cid = session_id.trim();
    if cid.is_empty() {
        return false;
    }
    let Some(home) = dirs::home_dir() else {
        return false;
    };
    if home
        .join(".gemini")
        .join("antigravity-cli")
        .join("brain")
        .join(cid)
        .is_dir()
    {
        return true;
    }
    let db_path = home
        .join(".gemini")
        .join("antigravity")
        .join("conversation_summaries.db");
    if let Some(conn) = open_sqlite_readonly(&db_path) {
        if let Ok((src, pid)) = conn.query_row(
            "SELECT COALESCE(source, ''), COALESCE(project_id, '') \
             FROM conversation_summaries WHERE conversation_id = ?1 LIMIT 1",
            [cid],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        ) {
            if src.to_ascii_uppercase().contains("CLI") || pid == "default-cli-project" {
                return true;
            }
        }
    }
    false
}

pub(super) fn load_codex_session_index_names(home: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let idx_path = home.join(".codex").join("session_index.jsonl");
    let Ok(raw) = fs::read_to_string(&idx_path) else {
        return map;
    };
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        let id = v.get("id").and_then(|s| s.as_str()).unwrap_or("").trim();
        let name = v
            .get("thread_name")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .trim();
        if !id.is_empty() && !name.is_empty() {
            map.insert(id.to_string(), name.to_string());
        }
    }
    map
}

/// Read-only authoritative title lookup for a Codex thread (`threads.name` -> `session_index.jsonl`).
pub fn lookup_codex_thread_name(thread_id: &str) -> Option<String> {
    let tid = thread_id.trim();
    if tid.is_empty() {
        return None;
    }
    let home = dirs::home_dir()?;
    let db_path = home.join(".codex").join("state_5.sqlite");
    if let Some(conn) = open_sqlite_readonly(&db_path) {
        if let Ok(name) = conn.query_row(
            "SELECT COALESCE(name, '') FROM threads WHERE id = ?1 LIMIT 1",
            [tid],
            |r| r.get::<_, String>(0),
        ) {
            if !name.trim().is_empty() {
                return Some(sanitize_title(&name, "Codex 会话"));
            }
        }
    }
    let idx = load_codex_session_index_names(&home);
    idx.get(tid).map(|n| sanitize_title(n, "Codex 会话"))
}

pub(super) fn normalize_win_path_key(p: &str) -> String {
    clean_windows_path(p)
        .trim()
        .trim_end_matches(['\\', '/'])
        .replace('/', "\\")
        .to_ascii_lowercase()
}

pub fn load_codex_catalog() -> ExternalHarnessCatalog {
    let exe = resolve_codex_exe();
    let Some(home) = dirs::home_dir() else {
        return ExternalHarnessCatalog {
            harness: "codex".to_string(),
            available: exe.is_some(),
            exe_path: exe.map(|p| p.to_string_lossy().to_string()),
            groups: Vec::new(),
            error: Some("无法定位用户目录".to_string()),
        };
    };

    let db_path = home.join(".codex").join("state_5.sqlite");
    let Some(conn) = open_sqlite_readonly(&db_path) else {
        return ExternalHarnessCatalog {
            harness: "codex".to_string(),
            available: exe.is_some(),
            exe_path: exe.map(|p| p.to_string_lossy().to_string()),
            groups: vec![ExternalWorkspaceGroup {
                workspace_id: home.to_string_lossy().to_string(),
                label: basename_of_path(&home.to_string_lossy()),
                path: home.to_string_lossy().to_string(),
                convs: Vec::new(),
            }],
            error: None,
        };
    };

    // 1. Read Codex Desktop's `.codex-global-state.json` (read-only) for exact Projects / Assignments / Projectless IDs
    struct CodexProjectDef {
        id: String,
        name: String,
        root_paths: Vec<String>,
        root_keys: Vec<String>,
    }
    let mut local_projects: Vec<CodexProjectDef> = Vec::new();
    let mut thread_proj_assign: HashMap<String, String> = HashMap::new();
    let mut projectless_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    let gs_path = home.join(".codex").join(".codex-global-state.json");
    if let Ok(raw_gs) = fs::read_to_string(&gs_path) {
        if let Ok(gs) = serde_json::from_str::<serde_json::Value>(&raw_gs) {
            let order: Vec<String> = gs
                .get("project-order")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();

            if let Some(proj_obj) = gs.get("local-projects").and_then(|v| v.as_object()) {
                let mut seen_pids = std::collections::HashSet::new();
                let mut ordered_keys: Vec<String> = Vec::new();
                for pid in order {
                    if proj_obj.contains_key(&pid) && seen_pids.insert(pid.clone()) {
                        ordered_keys.push(pid);
                    }
                }
                for pid in proj_obj.keys() {
                    if seen_pids.insert(pid.clone()) {
                        ordered_keys.push(pid.clone());
                    }
                }
                for pid in ordered_keys {
                    if let Some(entry) = proj_obj.get(&pid).and_then(|v| v.as_object()) {
                        let name = entry
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .trim()
                            .to_string();
                        let roots: Vec<String> = entry
                            .get("rootPaths")
                            .and_then(|v| v.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|x| x.as_str().map(clean_windows_path))
                                    .filter(|s| !s.is_empty())
                                    .collect()
                            })
                            .unwrap_or_default();
                        let label = if !name.is_empty() {
                            name
                        } else if let Some(first) = roots.first() {
                            basename_of_path(first)
                        } else {
                            continue;
                        };
                        let root_keys: Vec<String> =
                            roots.iter().map(|r| normalize_win_path_key(r)).collect();
                        local_projects.push(CodexProjectDef {
                            id: pid,
                            name: label,
                            root_paths: roots,
                            root_keys,
                        });
                    }
                }
            }

            if let Some(assigns) = gs
                .get("thread-project-assignments")
                .and_then(|v| v.as_object())
            {
                for (tid, val) in assigns {
                    if let Some(pid) = val.get("projectId").and_then(|x| x.as_str()) {
                        if !pid.trim().is_empty() {
                            thread_proj_assign.insert(tid.clone(), pid.trim().to_string());
                        }
                    }
                }
            }

            if let Some(pless) = gs.get("projectless-thread-ids").and_then(|v| v.as_array()) {
                for v in pless {
                    if let Some(tid) = v.as_str() {
                        projectless_ids.insert(tid.to_string());
                    }
                }
            }
        }
    }

    let index_names = load_codex_session_index_names(&home);
    let mut all_threads: Vec<(ExternalConversation, String)> = Vec::new(); // (conv, db_project_id)

    if let Ok(mut stmt) = conn.prepare(
        "SELECT id, COALESCE(source, ''), COALESCE(cwd, ''), COALESCE(name, ''), \
         COALESCE(title, ''), COALESCE(first_user_message, ''), \
         COALESCE(updated_at_ms, updated_at * 1000, 0), COALESCE(rollout_path, ''), \
         COALESCE(tokens_used, 0), COALESCE(project_id, '') \
         FROM threads \
         WHERE COALESCE(archived, 0) = 0 \
           AND source NOT LIKE '{%' \
           AND source != 'exec' \
         ORDER BY COALESCE(updated_at_ms, updated_at * 1000, 0) DESC LIMIT 120",
    ) {
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, String>(9)?,
            ))
        }) {
            for row in rows.flatten() {
                let (id, source, raw_cwd, name, title, first_msg, updated_ms, rollout, tokens_used, db_pid) =
                    row;
                // Skip empty / unstarted ghost threads (0 tokens & no user content) hidden by Codex Desktop
                if tokens_used == 0 && title.trim().is_empty() && first_msg.trim().is_empty() {
                    continue;
                }
                let cwd = clean_windows_path(&raw_cwd);
                let display_title = if !name.trim().is_empty() {
                    sanitize_title(&name, "Codex 会话")
                } else if let Some(idx_name) = index_names.get(&id) {
                    sanitize_title(idx_name, "Codex 会话")
                } else if !title.trim().is_empty() {
                    sanitize_title(&title, "Codex 会话")
                } else if !first_msg.trim().is_empty() {
                    sanitize_title(&first_msg, "Codex 会话")
                } else {
                    format!("会话 {}", &id[..id.len().min(8)])
                };
                all_threads.push((
                    ExternalConversation {
                        id,
                        title: display_title,
                        updated_at: updated_ms,
                        source,
                        cwd,
                        rollout_path: if rollout.trim().is_empty() {
                            None
                        } else {
                            Some(clean_windows_path(&rollout))
                        },
                    },
                    db_pid,
                ));
            }
        }
    }

    let mut groups: Vec<ExternalWorkspaceGroup> = Vec::new();
    if !local_projects.is_empty() {
        // Match Codex Desktop 1:1: show exact `local-projects` first, then `最近` (all recent threads)
        for proj in &local_projects {
            let mut convs: Vec<ExternalConversation> = Vec::new();
            for (conv, db_pid) in &all_threads {
                if projectless_ids.contains(&conv.id) {
                    continue;
                }
                let assigned = thread_proj_assign
                    .get(&conv.id)
                    .map(|p| p == &proj.id)
                    .unwrap_or(false)
                    || db_pid == &proj.id;
                let cwd_key = normalize_win_path_key(&conv.cwd);
                let path_match = !cwd_key.is_empty()
                    && proj
                        .root_keys
                        .iter()
                        .any(|rk| cwd_key == *rk || cwd_key.starts_with(&format!("{rk}\\")));
                if assigned || path_match {
                    convs.push(conv.clone());
                }
            }
            convs.sort_by_key(|c| std::cmp::Reverse(c.updated_at));
            let primary_path = proj.root_paths.first().cloned().unwrap_or_default();
            groups.push(ExternalWorkspaceGroup {
                workspace_id: if primary_path.is_empty() {
                    proj.id.clone()
                } else {
                    primary_path.clone()
                },
                label: proj.name.clone(),
                path: primary_path,
                convs,
            });
        }

        if !all_threads.is_empty() {
            let mut recent_convs: Vec<ExternalConversation> =
                all_threads.into_iter().map(|(c, _)| c).collect();
            recent_convs.sort_by_key(|c| std::cmp::Reverse(c.updated_at));
            groups.push(ExternalWorkspaceGroup {
                workspace_id: "__codex_recent__".to_string(),
                label: "最近".to_string(),
                path: String::new(),
                convs: recent_convs,
            });
        }
    } else {
        // Fallback for CLI-only Codex installations without Codex Desktop global state
        let mut by_cwd: BTreeMap<String, Vec<ExternalConversation>> = BTreeMap::new();
        for (conv, _) in all_threads {
            let key = if conv.cwd.is_empty() {
                home.to_string_lossy().to_string()
            } else {
                conv.cwd.clone()
            };
            by_cwd.entry(key).or_default().push(conv);
        }
        groups = by_cwd
            .into_iter()
            .map(|(path, mut convs)| {
                convs.sort_by_key(|c| std::cmp::Reverse(c.updated_at));
                ExternalWorkspaceGroup {
                    workspace_id: path.clone(),
                    label: basename_of_path(&path),
                    path,
                    convs,
                }
            })
            .collect();
        groups.sort_by(|a, b| {
            let a_ts = a.convs.first().map(|c| c.updated_at).unwrap_or(0);
            let b_ts = b.convs.first().map(|c| c.updated_at).unwrap_or(0);
            b_ts.cmp(&a_ts)
        });
    }

    if groups.is_empty() {
        let default_dir = home.to_string_lossy().to_string();
        groups.push(ExternalWorkspaceGroup {
            workspace_id: default_dir.clone(),
            label: basename_of_path(&default_dir),
            path: default_dir,
            convs: Vec::new(),
        });
    }

    ExternalHarnessCatalog {
        harness: "codex".to_string(),
        available: exe.is_some(),
        exe_path: exe.map(|p| p.to_string_lossy().to_string()),
        groups,
        error: None,
    }
}

pub(super) fn extract_claude_content_text(content: &serde_json::Value) -> String {
    if let Some(s) = content.as_str() {
        return s.trim().to_string();
    }
    let Some(arr) = content.as_array() else {
        return String::new();
    };
    let mut parts: Vec<String> = Vec::new();
    for item in arr {
        if item.get("type").and_then(|t| t.as_str()) != Some("text") {
            continue;
        }
        if let Some(txt) = item.get("text").and_then(|t| t.as_str()) {
            let trimmed = txt.trim();
            if !trimmed.is_empty() {
                parts.push(trimmed.to_string());
            }
        }
    }
    parts.join("\n")
}

pub(super) fn is_claude_internal_user_text(text: &str) -> bool {
    let t = text.trim();
    t.is_empty()
        || t.starts_with("<local-command-")
        || t.starts_with("<command-name>")
        || t.starts_with("<command-message>")
        || t.starts_with("<system-reminder>")
}

pub(super) fn file_mtime_ms(path: &Path) -> i64 {
    fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub(super) fn decode_claude_project_dir_fallback(dir_name: &str) -> String {
    // e.g. "D--AI-Ollama-Claude" -> "D:\AI-Ollama-Claude" (fallback only if jsonl lacks cwd)
    let bytes = dir_name.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b'-'
        && bytes[2] == b'-'
    {
        let drive = (bytes[0] as char).to_ascii_uppercase();
        let rest = &dir_name[3..];
        return format!(r"{drive}:\{rest}");
    }
    dir_name.to_string()
}

pub(super) fn inspect_claude_session_file(path: &Path, fallback_cwd: &str) -> Option<ExternalConversation> {
    let stem = path.file_stem()?.to_string_lossy().to_string();
    if stem.is_empty() || stem.starts_with("agent-") {
        return None;
    }

    let file = fs::File::open(path).ok()?;
    let reader = BufReader::new(file);

    let mut session_id = stem.clone();
    let mut cwd = String::new();
    let mut custom_title = String::new();
    let mut summary_title = String::new();
    let mut first_user_text = String::new();

    // Read up to 240 lines to extract metadata quickly without blocking on giant transcripts
    for (idx, line) in reader.lines().map_while(Result::ok).enumerate() {
        if idx >= 240 {
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
        if cwd.is_empty() {
            if let Some(c) = v.get("cwd").and_then(|s| s.as_str()) {
                if !c.trim().is_empty() {
                    cwd = clean_windows_path(c);
                }
            }
        }
        if let Some(sid) = v.get("sessionId").and_then(|s| s.as_str()) {
            if !sid.trim().is_empty() {
                session_id = sid.trim().to_string();
            }
        }
        let kind = v.get("type").and_then(|s| s.as_str()).unwrap_or("");
        match kind {
            "custom-title" => {
                if let Some(t) = v.get("customTitle").and_then(|s| s.as_str()) {
                    if !t.trim().is_empty() {
                        custom_title = t.trim().to_string();
                    }
                }
            }
            "summary" => {
                if summary_title.is_empty() {
                    if let Some(s) = v.get("summary").and_then(|s| s.as_str()) {
                        if !s.trim().is_empty() {
                            summary_title = s.trim().to_string();
                        }
                    }
                }
            }
            "user" if first_user_text.is_empty() => {
                if v.get("isMeta").and_then(|b| b.as_bool()) == Some(true) {
                    continue;
                }
                if let Some(content) = v.get("message").and_then(|m| m.get("content")) {
                    let txt = extract_claude_content_text(content);
                    if !is_claude_internal_user_text(&txt) {
                        first_user_text = txt;
                    }
                }
            }
            _ => {}
        }
    }

    if first_user_text.is_empty() && custom_title.is_empty() && summary_title.is_empty() {
        return None;
    }

    let final_cwd = if cwd.is_empty() {
        fallback_cwd.to_string()
    } else {
        cwd
    };
    let display_title = if !custom_title.is_empty() {
        sanitize_title(&custom_title, "Claude 会话")
    } else if !summary_title.is_empty() {
        sanitize_title(&summary_title, "Claude 会话")
    } else {
        sanitize_title(&first_user_text, "Claude 会话")
    };

    Some(ExternalConversation {
        id: session_id,
        title: display_title,
        updated_at: file_mtime_ms(path),
        source: "claude_code".to_string(),
        cwd: final_cwd,
        rollout_path: Some(path.to_string_lossy().to_string()),
    })
}

pub fn load_claude_code_catalog() -> ExternalHarnessCatalog {
    let exe = resolve_claude_exe();
    let Some(home) = dirs::home_dir() else {
        return ExternalHarnessCatalog {
            harness: "claude_code".to_string(),
            available: exe.is_some(),
            exe_path: exe.map(|p| p.to_string_lossy().to_string()),
            groups: Vec::new(),
            error: Some("无法定位用户目录".to_string()),
        };
    };

    let projects_root = home.join(".claude").join("projects");
    let mut by_cwd: BTreeMap<String, Vec<ExternalConversation>> = BTreeMap::new();

    if let Ok(proj_dirs) = fs::read_dir(&projects_root) {
        let mut session_files: Vec<(i64, PathBuf, String)> = Vec::new();
        for proj in proj_dirs.flatten() {
            let proj_path = proj.path();
            if !proj_path.is_dir() {
                continue;
            }
            let dir_name = proj.file_name().to_string_lossy().to_string();
            let dir_lower = dir_name.to_ascii_lowercase();
            // Skip ephemeral OS temp scratch directories (e.g. AppData\Local\Temp\...)
            if dir_lower.contains("appdata-local-temp-") || dir_lower.contains("windows-temp-") {
                continue;
            }
            let fallback_cwd = decode_claude_project_dir_fallback(&dir_name);
            let Ok(files) = fs::read_dir(&proj_path) else {
                continue;
            };
            for f in files.flatten() {
                let fp = f.path();
                if !fp.is_file() {
                    continue;
                }
                if fp.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                    continue;
                }
                session_files.push((file_mtime_ms(&fp), fp, fallback_cwd.clone()));
            }
        }

        // Only inspect the most recent 160 sessions for snappy popup open
        session_files.sort_by_key(|(ts, _, _)| std::cmp::Reverse(*ts));
        session_files.truncate(160);

        for (_, fp, fallback_cwd) in session_files {
            if let Some(conv) = inspect_claude_session_file(&fp, &fallback_cwd) {
                let key = if conv.cwd.is_empty() {
                    home.to_string_lossy().to_string()
                } else {
                    conv.cwd.clone()
                };
                by_cwd.entry(key).or_default().push(conv);
            }
        }
    }

    let mut groups: Vec<ExternalWorkspaceGroup> = by_cwd
        .into_iter()
        .map(|(path, mut convs)| {
            convs.sort_by_key(|c| std::cmp::Reverse(c.updated_at));
            ExternalWorkspaceGroup {
                workspace_id: path.clone(),
                label: basename_of_path(&path),
                path,
                convs,
            }
        })
        .collect();

    groups.sort_by(|a, b| {
        let a_ts = a.convs.first().map(|c| c.updated_at).unwrap_or(0);
        let b_ts = b.convs.first().map(|c| c.updated_at).unwrap_or(0);
        b_ts.cmp(&a_ts)
    });

    if groups.is_empty() {
        let default_dir = home.to_string_lossy().to_string();
        groups.push(ExternalWorkspaceGroup {
            workspace_id: default_dir.clone(),
            label: basename_of_path(&default_dir),
            path: default_dir,
            convs: Vec::new(),
        });
    }

    ExternalHarnessCatalog {
        harness: "claude_code".to_string(),
        available: exe.is_some(),
        exe_path: exe.map(|p| p.to_string_lossy().to_string()),
        groups,
        error: None,
    }
}

