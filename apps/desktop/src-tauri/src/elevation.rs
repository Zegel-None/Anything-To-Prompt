use anyhow::{anyhow, Context, Result};
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND};
use windows::Win32::Security::{
    DuplicateTokenEx, GetTokenInformation, SecurityImpersonation, TokenElevation, TokenPrimary,
    TOKEN_ACCESS_MASK, TOKEN_ADJUST_DEFAULT, TOKEN_ADJUST_SESSIONID, TOKEN_ASSIGN_PRIMARY,
    TOKEN_DUPLICATE, TOKEN_ELEVATION, TOKEN_QUERY,
};
use windows::Win32::System::Threading::{
    CreateProcessWithTokenW, OpenProcess, OpenProcessToken, CREATE_NO_WINDOW,
    CREATE_UNICODE_ENVIRONMENT, LOGON_WITH_PROFILE, PROCESS_INFORMATION,
    PROCESS_QUERY_LIMITED_INFORMATION, STARTUPINFOW,
};
use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW};
use windows::Win32::UI::WindowsAndMessaging::{
    GetShellWindow, GetWindowThreadProcessId, SW_HIDE, SW_SHOWNORMAL,
};

pub use crate::win32_util::to_wide;

pub fn is_elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        let current = windows::Win32::System::Threading::GetCurrentProcess();
        if OpenProcessToken(current, TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut ret_len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret_len,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}

fn quote_windows_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.chars().any(|c| c == ' ' || c == '\t' || c == '"') {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0usize;
    for ch in arg.chars() {
        if ch == '\\' {
            backslashes += 1;
        } else if ch == '"' {
            out.push_str(&"\\".repeat(backslashes * 2 + 1));
            out.push('"');
            backslashes = 0;
        } else {
            if backslashes > 0 {
                out.push_str(&"\\".repeat(backslashes));
                backslashes = 0;
            }
            out.push(ch);
        }
    }
    if backslashes > 0 {
        out.push_str(&"\\".repeat(backslashes * 2));
    }
    out.push('"');
    out
}

pub fn try_relaunch_elevated(extra_args: &[String]) -> bool {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            log::warn!("current_exe failed before elevation: {e}");
            return false;
        }
    };

    let mut params_vec = vec!["--elevated-relaunch".to_string()];
    for a in extra_args {
        if a != "--elevated-relaunch" {
            params_vec.push(a.clone());
        }
    }
    let joined = params_vec
        .iter()
        .map(|s| quote_windows_arg(s))
        .collect::<Vec<_>>()
        .join(" ");

    let verb_w = to_wide("runas");
    let file_w = to_wide(&exe.to_string_lossy());
    let params_w = to_wide(&joined);

    unsafe {
        let mut sei = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOASYNC,
            hwnd: HWND::default(),
            lpVerb: PCWSTR(verb_w.as_ptr()),
            lpFile: PCWSTR(file_w.as_ptr()),
            lpParameters: PCWSTR(params_w.as_ptr()),
            lpDirectory: PCWSTR::null(),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        match ShellExecuteExW(&mut sei) {
            Ok(()) => {
                log::info!("Relaunched elevated instance successfully; exiting medium bootstrap.");
                true
            }
            Err(err) => {
                log::warn!(
                    "UAC elevation declined or unavailable ({err}); continuing in medium integrity."
                );
                false
            }
        }
    }
}

/// Spawns a child process at medium integrity (desktop Explorer shell token when elevated,
/// or normal std::process::Command when already non-elevated).
/// Returns child PID on success.
pub fn spawn_medium(exe: &Path, args: &[String], cwd: Option<&Path>, hidden: bool) -> Result<u32> {
    if !is_elevated() {
        let mut cmd = Command::new(exe);
        cmd.args(args);
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        if hidden {
            cmd.creation_flags(CREATE_NO_WINDOW.0);
        }
        let child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn non-elevated {:?}", exe))?;
        return Ok(child.id());
    }

    unsafe {
        let shell_hwnd = GetShellWindow();
        if shell_hwnd.0.is_null() {
            return Err(anyhow!(
                "GetShellWindow returned null; Explorer shell not running"
            ));
        }
        let mut shell_pid = 0u32;
        GetWindowThreadProcessId(shell_hwnd, Some(&mut shell_pid));
        if shell_pid == 0 {
            return Err(anyhow!(
                "GetWindowThreadProcessId returned 0 for shell window"
            ));
        }

        let shell_proc = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, shell_pid)
            .context("OpenProcess on Explorer shell failed")?;

        let mut shell_token = HANDLE::default();
        let token_res = OpenProcessToken(shell_proc, TOKEN_DUPLICATE, &mut shell_token);
        let _ = CloseHandle(shell_proc);
        token_res.context("OpenProcessToken on Explorer shell failed")?;

        let desired_access: TOKEN_ACCESS_MASK = TOKEN_QUERY
            | TOKEN_ASSIGN_PRIMARY
            | TOKEN_DUPLICATE
            | TOKEN_ADJUST_DEFAULT
            | TOKEN_ADJUST_SESSIONID;

        let mut primary_token = HANDLE::default();
        let dup_res = DuplicateTokenEx(
            shell_token,
            desired_access,
            None,
            SecurityImpersonation,
            TokenPrimary,
            &mut primary_token,
        );
        let _ = CloseHandle(shell_token);
        dup_res.context("DuplicateTokenEx for medium token failed")?;

        let mut cmdline_parts = vec![quote_windows_arg(&exe.to_string_lossy())];
        for arg in args {
            cmdline_parts.push(quote_windows_arg(arg));
        }
        let cmdline_str = cmdline_parts.join(" ");
        let mut cmdline_w = to_wide(&cmdline_str);
        let cwd_w = cwd.map(|p| to_wide(&p.to_string_lossy()));

        let mut si = STARTUPINFOW {
            cb: std::mem::size_of::<STARTUPINFOW>() as u32,
            ..Default::default()
        };
        if hidden {
            si.dwFlags = windows::Win32::System::Threading::STARTF_USESHOWWINDOW;
            si.wShowWindow = SW_HIDE.0 as u16;
        }
        let mut pi = PROCESS_INFORMATION::default();
        let flags = CREATE_UNICODE_ENVIRONMENT
            | if hidden {
                CREATE_NO_WINDOW
            } else {
                windows::Win32::System::Threading::PROCESS_CREATION_FLAGS(0)
            };

        let create_res = CreateProcessWithTokenW(
            primary_token,
            LOGON_WITH_PROFILE,
            PCWSTR::null(),
            Some(PWSTR(cmdline_w.as_mut_ptr())),
            flags,
            None,
            cwd_w
                .as_ref()
                .map(|v| PCWSTR(v.as_ptr()))
                .unwrap_or(PCWSTR::null()),
            &si,
            &mut pi,
        );
        let _ = CloseHandle(primary_token);
        create_res.with_context(|| format!("CreateProcessWithTokenW failed for {:?}", exe))?;

        let pid = pi.dwProcessId;
        let _ = CloseHandle(pi.hThread);
        let _ = CloseHandle(pi.hProcess);
        Ok(pid)
    }
}
