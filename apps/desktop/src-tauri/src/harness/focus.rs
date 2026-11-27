use super::*;

pub(super) use crate::win32_util::to_wide as to_wide_null;

pub fn collect_pids_for_exe(exe_lower: &str) -> HashSet<u32> {
    let mut pids = HashSet::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return pids;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snap, &mut entry).is_ok() {
            loop {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..len]).to_ascii_lowercase();
                if name == exe_lower {
                    pids.insert(entry.th32ProcessID);
                }
                if Process32NextW(snap, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    pids
}

pub fn is_process_name_running(exe_lower: &str) -> bool {
    !collect_pids_for_exe(exe_lower).is_empty()
}

pub fn resolve_antigravity_desktop_exe() -> Option<PathBuf> {
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let p = PathBuf::from(local)
            .join("Programs")
            .join("antigravity")
            .join("Antigravity.exe");
        if p.exists() {
            return Some(p);
        }
    }
    if let Ok(pf) = std::env::var("ProgramFiles") {
        let p = PathBuf::from(pf).join("Antigravity").join("Antigravity.exe");
        if p.exists() {
            return Some(p);
        }
    }
    None
}

pub(super) fn query_exe_path_for_pid(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = vec![0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(handle);
        if ok && len > 0 {
            Some(String::from_utf16_lossy(&buf[..len as usize]))
        } else {
            None
        }
    }
}

pub(super) use crate::win32_util::window_title_text;

pub(super) fn find_main_window_for_pids(pids: HashSet<u32>, required_title_lower: Option<&str>) -> Option<HWND> {
    if pids.is_empty() {
        return None;
    }
    struct SearchCtx {
        pids: HashSet<u32>,
        required_title_lower: Option<String>,
        best_hwnd: Option<HWND>,
        best_score: i64,
    }
    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = &mut *(lparam.0 as *mut SearchCtx);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if !ctx.pids.contains(&pid) {
            return BOOL(1);
        }
        let iconic = IsIconic(hwnd).as_bool();
        let visible = IsWindowVisible(hwnd).as_bool();
        if !visible && !iconic {
            return BOOL(1);
        }
        if let Some(ref req) = ctx.required_title_lower {
            let title = window_title_text(hwnd).trim().to_ascii_lowercase();
            if title != *req && !title.starts_with(req) {
                return BOOL(1);
            }
        }
        let mut rc = RECT::default();
        let _ = GetWindowRect(hwnd, &mut rc);
        let w = (rc.right - rc.left) as i64;
        let h = (rc.bottom - rc.top) as i64;
        let score = if iconic {
            500_000
        } else if w >= 240 && h >= 160 {
            w * h
        } else {
            return BOOL(1);
        };
        if score > ctx.best_score {
            ctx.best_score = score;
            ctx.best_hwnd = Some(hwnd);
        }
        BOOL(1)
    }
    let mut ctx = SearchCtx {
        pids,
        required_title_lower: required_title_lower.map(|s| s.to_ascii_lowercase()),
        best_hwnd: None,
        best_score: 0,
    };
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut ctx as *mut SearchCtx as isize));
    }
    ctx.best_hwnd
}

pub(super) fn find_main_window_for_exe(exe_lower: &str) -> Option<HWND> {
    find_main_window_for_pids(collect_pids_for_exe(exe_lower), None)
}

/// Finds the real Codex Desktop GUI window (`WindowsApps\OpenAI.Codex_*\app\ChatGPT.exe`,
/// or `codex.exe` if a future build renames it), distinguishing it from standalone ChatGPT Desktop.
pub(super) fn find_codex_desktop_window() -> Option<HWND> {
    let mut codex_pids = collect_pids_for_exe("codex.exe");
    let mut fallback_chatgpt_pids = HashSet::new();
    for pid in collect_pids_for_exe("chatgpt.exe") {
        match query_exe_path_for_pid(pid) {
            Some(path) if path.to_ascii_lowercase().contains("openai.codex") => {
                codex_pids.insert(pid);
            }
            None => {
                fallback_chatgpt_pids.insert(pid);
            }
            _ => {}
        }
    }
    find_main_window_for_pids(codex_pids, None)
        .or_else(|| find_main_window_for_pids(fallback_chatgpt_pids, Some("codex")))
}

pub(super) fn find_harness_desktop_window(kind: HarnessKind) -> Option<HWND> {
    match kind {
        HarnessKind::Antigravity => find_main_window_for_exe("antigravity.exe"),
        HarnessKind::Codex => find_codex_desktop_window(),
        HarnessKind::ClaudeCode => find_main_window_for_exe("claude.exe"),
        HarnessKind::Dsh => None,
    }
}

/// Launches a desktop Harness GUI silently in the background (minimized without activating),
/// completely decoupled from the caller's Job Object.
pub(super) fn launch_desktop_app_silently(exe: &Path, exe_lower_for_guard: &'static str) -> bool {
    let exe_str = exe.to_string_lossy().to_string();
    let exe_w = to_wide_null(&exe_str);
    let cmdline = format!("\"{exe_str}\"");
    let si = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        dwFlags: STARTF_USESHOWWINDOW,
        wShowWindow: SW_SHOWMINNOACTIVE.0 as u16,
        ..Default::default()
    };
    let mut cmd_w = to_wide_null(&cmdline);
    let mut pi = PROCESS_INFORMATION::default();
    let launched = unsafe {
        CreateProcessW(
            PCWSTR(exe_w.as_ptr()),
            Some(PWSTR(cmd_w.as_mut_ptr())),
            None,
            None,
            false,
            CREATE_BREAKAWAY_FROM_JOB | DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP,
            None,
            PCWSTR::null(),
            &si,
            &mut pi,
        )
        .is_ok()
    } || {
        let mut cmd_w2 = to_wide_null(&cmdline);
        unsafe {
            CreateProcessW(
                PCWSTR(exe_w.as_ptr()),
                Some(PWSTR(cmd_w2.as_mut_ptr())),
                None,
                None,
                false,
                DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP,
                None,
                PCWSTR::null(),
                &si,
                &mut pi,
            )
            .is_ok()
        }
    };
    if launched {
        unsafe {
            let _ = CloseHandle(pi.hThread);
            let _ = CloseHandle(pi.hProcess);
        }
        // Guard for 4.5s in case Electron ignores STARTF_USESHOWWINDOW and tries to pop up on first paint
        std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_millis(4500);
            let mut minimized_once = false;
            while Instant::now() < deadline {
                if let Some(hwnd) = find_main_window_for_exe(exe_lower_for_guard) {
                    unsafe {
                        if !IsIconic(hwnd).as_bool() {
                            let _ = ShowWindow(hwnd, SW_SHOWMINNOACTIVE);
                            minimized_once = true;
                        }
                    }
                    if minimized_once {
                        std::thread::sleep(Duration::from_millis(350));
                        // Re-check once more in case splash transitioned to main window
                        if let Some(hwnd2) = find_main_window_for_exe(exe_lower_for_guard) {
                            unsafe {
                                if !IsIconic(hwnd2).as_bool() {
                                    let _ = ShowWindow(hwnd2, SW_SHOWMINNOACTIVE);
                                }
                            }
                        }
                        break;
                    }
                }
                std::thread::sleep(Duration::from_millis(140));
            }
        });
    }
    launched
}

pub(super) fn title_matches_target(candidate_raw: &str, target_raw: &str) -> u8 {
    let cand = candidate_raw.trim();
    let target = target_raw.trim();
    if cand.is_empty() || target.is_empty() {
        return 0;
    }
    fn strip_dots(s: &str) -> &str {
        s.trim_end_matches(['…', '.', ' ']).trim()
    }
    let c_clean = strip_dots(cand);
    let t_clean = strip_dots(target);
    if c_clean.is_empty() || t_clean.is_empty() {
        return 0;
    }
    if c_clean.eq_ignore_ascii_case(t_clean) {
        return 3;
    }
    let c_low = c_clean.to_lowercase();
    let t_low = t_clean.to_lowercase();
    let c_chars = c_low.chars().count();
    let t_chars = t_low.chars().count();
    if c_chars >= 4 && t_chars >= 4 && (t_low.starts_with(&c_low) || c_low.starts_with(&t_low)) {
        return 2;
    }
    if t_chars >= 5 && c_low.contains(&t_low) {
        return 1;
    }
    0
}

pub(super) fn select_conversation_in_window_uia(hwnd: HWND, target_title: &str) -> bool {
    let target = target_title.trim();
    if target.is_empty() {
        return false;
    }
    let Ok(auto) = uiautomation::UIAutomation::new() else {
        return false;
    };
    let Ok(walker) = auto.get_control_view_walker() else {
        return false;
    };
    let Ok(root) = auto.element_from_handle(uiautomation::types::Handle::from(hwnd.0 as isize))
    else {
        return false;
    };

    let mut win_rc = RECT::default();
    unsafe {
        let _ = GetWindowRect(hwnd, &mut win_rc);
    }
    let sidebar_right = win_rc.left + 500;
    let titlebar_bottom = win_rc.top + 52;

    // Two-tier queue: scan left-sidebar subtrees FIRST so huge right-pane chat transcripts
    // never exhaust the UIA traversal budget before reaching the conversation list.
    let mut primary_queue: std::collections::VecDeque<(uiautomation::UIElement, usize)> =
        std::collections::VecDeque::new();
    let mut secondary_queue: std::collections::VecDeque<(uiautomation::UIElement, usize)> =
        std::collections::VecDeque::new();
    primary_queue.push_back((root, 0));
    let mut visited = 0usize;
    let mut best: Option<(u8, bool, String, uiautomation::UIElement)> = None;

    while let Some((el, depth)) = primary_queue.pop_front().or_else(|| {
        // Only fall back to right-pane elements if sidebar didn't find anything
        if best.as_ref().map(|(_, s, _, _)| *s).unwrap_or(false) {
            None
        } else {
            secondary_queue.pop_front()
        }
    }) {
        visited += 1;
        if visited > 5500 {
            break;
        }
        let el_rect = el.get_bounding_rectangle().ok();
        if let Ok(name) = el.get_name() {
            let score = title_matches_target(&name, target);
            if score > 0 {
                let rect_info = el_rect.as_ref().map(|r| {
                    let w = r.get_right() - r.get_left();
                    let h = r.get_bottom() - r.get_top();
                    let below_titlebar = r.get_top() > titlebar_bottom;
                    let valid_box = w > 12 && h > 10 && h < 120;
                    let in_sidebar = valid_box && below_titlebar && r.get_left() <= sidebar_right;
                    (valid_box && below_titlebar, in_sidebar)
                });
                if let Some((true, in_sidebar)) = rect_info {
                    let replace = match &best {
                        None => true,
                        Some((bs, b_side, _, _)) => {
                            (in_sidebar && !*b_side) || (in_sidebar == *b_side && score > *bs)
                        }
                    };
                    if replace {
                        best = Some((score, in_sidebar, name, el.clone()));
                        if score == 3 && in_sidebar {
                            break;
                        }
                    }
                }
            }
        }
        if depth < 24 {
            if let Ok(mut child) = walker.get_first_child(&el) {
                let mut sibs = 0usize;
                loop {
                    sibs += 1;
                    if sibs > 220 {
                        break;
                    }
                    let is_strictly_right_pane = depth >= 2
                        && child
                            .get_bounding_rectangle()
                            .ok()
                            .map(|r| {
                                let w = r.get_right() - r.get_left();
                                let h = r.get_bottom() - r.get_top();
                                w > 20 && h > 20 && r.get_left() > sidebar_right
                            })
                            .unwrap_or(false);
                    if is_strictly_right_pane {
                        secondary_queue.push_back((child.clone(), depth + 1));
                    } else {
                        primary_queue.push_back((child.clone(), depth + 1));
                    }
                    match walker.get_next_sibling(&child) {
                        Ok(next) => child = next,
                        Err(_) => break,
                    }
                }
            }
        }
    }

    let Some((score, in_sidebar, matched_name, chosen)) = best else {
        log::info!(
            "select_conversation_in_window_uia: no match for {:?} (visited={visited})",
            target
        );
        return false;
    };
    log::info!(
        "select_conversation_in_window_uia: matched {:?} (score={score}, in_sidebar={in_sidebar}, visited={visited})",
        matched_name
    );

    // Check chosen element and up to 3 ancestor levels for Invoke / SelectionItem patterns
    let mut cur = chosen.clone();
    for _ in 0..4 {
        if let Ok(inv) = cur.get_pattern::<uiautomation::patterns::UIInvokePattern>() {
            if inv.invoke().is_ok() {
                return true;
            }
        }
        if let Ok(sel) = cur.get_pattern::<uiautomation::patterns::UISelectionItemPattern>() {
            if sel.select().is_ok() {
                return true;
            }
        }
        match walker.get_parent(&cur) {
            Ok(parent) => cur = parent,
            Err(_) => break,
        }
    }
    chosen.click().is_ok()
}

pub fn resolve_claude_desktop_exe() -> Option<PathBuf> {
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        for rel in [
            r"AnthropicClaude\Claude.exe",
            r"Programs\claude-desktop\Claude.exe",
            r"Programs\Claude\Claude.exe",
        ] {
            let p = PathBuf::from(&local).join(rel);
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}

pub(super) fn find_msix_app_launch_uri(prefix_lower: &str) -> Option<String> {
    let local = std::env::var("LOCALAPPDATA").ok()?;
    let packages_dir = PathBuf::from(local).join("Packages");
    let entries = fs::read_dir(packages_dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.to_ascii_lowercase().starts_with(prefix_lower) {
            return Some(format!(r"shell:AppsFolder\{name}!App"));
        }
    }
    None
}

pub(super) fn launch_uri_or_exe_foreground(target: &str) {
    let verb_w = to_wide_null("open");
    let target_w = to_wide_null(target);
    unsafe {
        let _ = ShellExecuteW(
            None,
            PCWSTR(verb_w.as_ptr()),
            PCWSTR(target_w.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOW,
        );
    }
}

pub(super) fn bring_hwnd_to_front_force(target_hwnd: HWND) {
    unsafe {
        if IsIconic(target_hwnd).as_bool() {
            let _ = ShowWindow(target_hwnd, SW_RESTORE);
        } else {
            let _ = ShowWindow(target_hwnd, SW_SHOW);
        }
        crate::window::focus_hwnd_without_sendinput(target_hwnd);
        let _ = BringWindowToTop(target_hwnd);
        // Temporary TOPMOST -> NOTOPMOST pulse guarantees the desktop app rises to the very front
        let _ = SetWindowPos(
            target_hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE,
        );
        let _ = SetWindowPos(
            target_hwnd,
            Some(HWND_NOTOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE,
        );
        let _ = SetForegroundWindow(target_hwnd);
    }
}

/// Builds an authenticated DSH WebUI URL preserving `?token=...` across DSH's 302 auth redirect
/// and passing the target session via URL fragment (`#atp_session=<sid>`) so DSH's SPA opens that
/// exact conversation without hitting a 404 route.
pub fn build_dsh_focus_url(launch_url: &str, session_id: Option<&str>) -> String {
    let trimmed_url = launch_url.trim();
    let sid = session_id.map(|s| s.trim()).filter(|s| !s.is_empty());
    let Ok(mut parsed) = url::Url::parse(trimmed_url) else {
        return match sid {
            Some(id) => format!("{trimmed_url}#atp_session={id}"),
            None => trimmed_url.to_string(),
        };
    };
    let kept_pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(k, _)| k != "dsh_bar_clone" && k != "dsb_mode")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if kept_pairs.is_empty() {
        parsed.set_query(None);
    } else {
        let mut qp = parsed.query_pairs_mut();
        qp.clear();
        for (k, v) in kept_pairs {
            qp.append_pair(&k, &v);
        }
    }
    if let Some(id) = sid {
        let encoded: String = url::form_urlencoded::byte_serialize(id.as_bytes())
            .collect::<String>()
            .replace('+', "%20");
        parsed.set_fragment(Some(&format!("atp_session={encoded}")));
    } else {
        parsed.set_fragment(None);
    }
    parsed.to_string()
}

pub fn focus_harness_desktop(
    app: AppHandle,
    kind: HarnessKind,
    session_id: Option<String>,
    title: Option<String>,
    workspace_path: Option<String>,
    config: crate::config::AppConfig,
    broker: Arc<BrokerClient>,
    dsh_conn: Arc<Mutex<crate::dsh::DshConnectionState>>,
) -> Result<String, String> {
    if kind == HarnessKind::Dsh {
        std::thread::spawn(move || {
            let _ = crate::plugin_sync::ensure_dsh_plugin_synced();
            let outcome = crate::dsh::discover_or_start_dsh(&config, &broker, true);
            match outcome {
                crate::dsh::DiscoveryOutcome::Connected { port, launch_url } => {
                    let needs_bar_nav = dsh_conn
                        .lock()
                        .map(|st| st.active_port != Some(port))
                        .unwrap_or(false);
                    if needs_bar_nav {
                        crate::dsh::apply_discovery_to_bar(
                            &app,
                            &dsh_conn,
                            crate::dsh::DiscoveryOutcome::Connected {
                                port,
                                launch_url: launch_url.clone(),
                            },
                        );
                    }
                    let url = build_dsh_focus_url(&launch_url, session_id.as_deref());
                    log::info!(
                        "focus_harness_desktop(dsh): opening authenticated WebUI on port {port} (session={:?})",
                        session_id
                    );
                    launch_uri_or_exe_foreground(&url);
                }
                crate::dsh::DiscoveryOutcome::MissingPlugin { port } => {
                    let url = build_dsh_focus_url(
                        &format!("http://127.0.0.1:{port}/"),
                        session_id.as_deref(),
                    );
                    log::warn!(
                        "focus_harness_desktop(dsh): plugin missing on port {port}, opening fallback WebUI"
                    );
                    launch_uri_or_exe_foreground(&url);
                }
                crate::dsh::DiscoveryOutcome::Unavailable { reason } => {
                    log::warn!("focus_harness_desktop(dsh): DSH unavailable: {reason}");
                }
            }
        });
        return Ok("正在启动并打开 DeepSeek Harness 对话".to_string());
    }

    let kind_label = kind.as_str();
    let resolved_title = title
        .as_deref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            let sid = session_id.as_deref()?.trim();
            if sid.is_empty() {
                return None;
            }
            match kind {
                HarnessKind::Antigravity => {
                    lookup_antigravity_conversation_meta(sid).map(|(_, _, _, t)| t)
                }
                HarnessKind::Codex => lookup_codex_thread_name(sid),
                _ => None,
            }
        });

    std::thread::spawn(move || {
        let mut freshly_launched = false;
        let mut hwnd = find_harness_desktop_window(kind);

        if hwnd.is_none() {
            match kind {
                HarnessKind::Antigravity => {
                    if let Some(exe) = resolve_antigravity_desktop_exe() {
                        launch_uri_or_exe_foreground(&exe.to_string_lossy());
                        freshly_launched = true;
                    }
                }
                HarnessKind::Codex => {
                    if let Some(apps_uri) = find_msix_app_launch_uri("openai.codex_") {
                        let verb_w = to_wide_null("open");
                        let exp_w = to_wide_null("explorer.exe");
                        let arg_w = to_wide_null(&apps_uri);
                        unsafe {
                            let _ = ShellExecuteW(
                                None,
                                PCWSTR(verb_w.as_ptr()),
                                PCWSTR(exp_w.as_ptr()),
                                PCWSTR(arg_w.as_ptr()),
                                PCWSTR::null(),
                                SW_SHOW,
                            );
                        }
                        freshly_launched = true;
                    }
                }
                HarnessKind::ClaudeCode => {
                    if let Some(exe) = resolve_claude_desktop_exe() {
                        launch_uri_or_exe_foreground(&exe.to_string_lossy());
                        freshly_launched = true;
                    } else if let Some(apps_uri) = find_msix_app_launch_uri("claude_") {
                        let verb_w = to_wide_null("open");
                        let exp_w = to_wide_null("explorer.exe");
                        let arg_w = to_wide_null(&apps_uri);
                        unsafe {
                            let _ = ShellExecuteW(
                                None,
                                PCWSTR(verb_w.as_ptr()),
                                PCWSTR(exp_w.as_ptr()),
                                PCWSTR(arg_w.as_ptr()),
                                PCWSTR::null(),
                                SW_SHOW,
                            );
                        }
                        freshly_launched = true;
                    } else if let Some(claude_cli) = resolve_claude_exe() {
                        // Fallback when only Claude Code CLI is installed: open an interactive terminal resuming that session
                        let sid = session_id.as_deref().map(|s| s.trim()).unwrap_or("");
                        let cli_cmd = if !sid.is_empty() && !sid.ends_with(":new") {
                            format!("\"{}\" --resume \"{}\"", claude_cli.to_string_lossy(), sid)
                        } else {
                            format!("\"{}\"", claude_cli.to_string_lossy())
                        };
                        let mut cmd = Command::new("cmd.exe");
                        cmd.args(["/K", &cli_cmd]);
                        if let Some(cwd) = workspace_path
                            .as_deref()
                            .map(clean_windows_path)
                            .filter(|p| !p.is_empty() && Path::new(p).exists())
                        {
                            cmd.current_dir(cwd);
                        }
                        let _ = cmd.spawn();
                        return;
                    }
                }
                HarnessKind::Dsh => unreachable!(),
            }

            if freshly_launched {
                let deadline = Instant::now() + Duration::from_secs(10);
                while Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(250));
                    if let Some(found) = find_harness_desktop_window(kind) {
                        hwnd = Some(found);
                        break;
                    }
                }
            }
        }

        let Some(target_hwnd) = hwnd else {
            log::warn!("focus_harness_desktop: no window found for {kind_label}");
            return;
        };

        bring_hwnd_to_front_force(target_hwnd);

        if let Some(ref conv_title) = resolved_title {
            std::thread::sleep(Duration::from_millis(if freshly_launched { 1600 } else { 220 }));
            let ok = select_conversation_in_window_uia(target_hwnd, conv_title);
            log::info!(
                "focus_harness_desktop({kind_label}): foregrounded window (title={:?}), select_conversation({:?})={ok}",
                window_title_text(target_hwnd),
                conv_title
            );
        }
    });

    Ok(format!("正在切换到 {}", kind.as_str()))
}

