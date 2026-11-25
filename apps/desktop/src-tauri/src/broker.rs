use crate::elevation::{is_elevated, spawn_medium, to_wide};
use anyhow::{anyhow, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::FromRawHandle;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::core::{PCWSTR, PSTR};
use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL, INVALID_HANDLE_VALUE};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidA, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{
    GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    TOKEN_USER,
};
use windows::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken, CREATE_NO_WINDOW};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum BrokerRequest {
    Ping,
    ShellSelection {
        hwnd: isize,
        class_name: String,
    },
    ExplorerFolder {
        hwnd: isize,
        class_name: String,
    },
    RunPs51 {
        script: String,
        timeout_ms: u64,
    },
    Spawn {
        exe: String,
        args: Vec<String>,
        cwd: Option<String>,
        hidden: bool,
    },
    StartHarnessTurn {
        turn_id: String,
        harness: String,
        exe: String,
        cwd: Option<String>,
        project_id: Option<String>,
        session_id: Option<String>,
        prompt: String,
        out_pipe: String,
    },
    CancelHarnessTurn {
        turn_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrokerResponse {
    pub ok: bool,
    pub paths: Vec<String>,
    pub text: String,
    pub pid: Option<u32>,
    pub error: Option<String>,
}

impl BrokerResponse {
    pub fn ok_text(text: String) -> Self {
        Self {
            ok: true,
            paths: Vec::new(),
            text,
            pid: None,
            error: None,
        }
    }

    pub fn ok_paths(paths: Vec<String>) -> Self {
        Self {
            ok: true,
            paths,
            text: String::new(),
            pid: None,
            error: None,
        }
    }

    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            ok: false,
            paths: Vec::new(),
            text: String::new(),
            pid: None,
            error: Some(msg.into()),
        }
    }
}

fn encode_powershell_utf16le(script: &str) -> String {
    let utf16: Vec<u16> = std::ffi::OsStr::new(script).encode_wide().collect();
    let mut bytes = Vec::with_capacity(utf16.len() * 2);
    for u in utf16 {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

pub fn execute_ps51_local(script: &str, timeout_ms: u64) -> Result<String> {
    let wrapped = format!(
        "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; $ErrorActionPreference = 'Stop'; {}",
        script
    );
    let encoded = encode_powershell_utf16le(&wrapped);
    let mut child = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-EncodedCommand",
            &encoded,
        ])
        .creation_flags(CREATE_NO_WINDOW.0)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to spawn powershell.exe")?;

    let deadline = Instant::now() + Duration::from_millis(timeout_ms.max(250));
    loop {
        match child.try_wait()? {
            Some(_status) => {
                let output = child.wait_with_output()?;
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !stdout.is_empty() || output.status.success() {
                    return Ok(stdout);
                }
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                return Err(anyhow!("powershell error: {stderr}"));
            }
            None => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(anyhow!("powershell script timed out after {timeout_ms}ms"));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

fn handle_request_in_medium(req: BrokerRequest) -> BrokerResponse {
    match req {
        BrokerRequest::Ping => BrokerResponse::ok_text("pong".to_string()),

        BrokerRequest::ShellSelection { hwnd, class_name } => {
            let is_desktop = matches!(class_name.as_str(), "Progman" | "WorkerW");
            let script = if is_desktop {
                r#"
$sh = New-Object -ComObject Shell.Application
$loc = [ref]$null
$win = $sh.Windows().FindWindowSW(0, 0, 8, $loc, 1)
if ($null -ne $win -and $null -ne $win.Document) {
  $items = @($win.Document.SelectedItems() | ForEach-Object { $_.Path } | Where-Object { $_ })
  ConvertTo-Json -InputObject $items -Compress
} else {
  '[]'
}
"#
                .to_string()
            } else {
                format!(
                    r#"
$targetHwnd = [int64]{hwnd}
$sh = New-Object -ComObject Shell.Application
$out = @()
foreach ($w in $sh.Windows()) {{
  try {{
    if ([int64]$w.HWND -eq $targetHwnd -and $null -ne $w.Document) {{
      $out = @($w.Document.SelectedItems() | ForEach-Object {{ $_.Path }} | Where-Object {{ $_ }})
      break
    }}
  }} catch {{}}
}}
ConvertTo-Json -InputObject $out -Compress
"#
                )
            };

            match execute_ps51_local(&script, 1400) {
                Ok(raw) => {
                    let paths = parse_json_string_list(&raw);
                    BrokerResponse::ok_paths(paths)
                }
                Err(e) => BrokerResponse::err(e.to_string()),
            }
        }

        BrokerRequest::ExplorerFolder { hwnd, class_name } => {
            if matches!(class_name.as_str(), "Progman" | "WorkerW") {
                if let Some(desktop) = dirs::desktop_dir() {
                    return BrokerResponse::ok_text(desktop.to_string_lossy().to_string());
                }
            }
            let script = format!(
                r#"
$targetHwnd = [int64]{hwnd}
$sh = New-Object -ComObject Shell.Application
foreach ($w in $sh.Windows()) {{
  try {{
    if ([int64]$w.HWND -eq $targetHwnd -and $null -ne $w.Document -and $null -ne $w.Document.Folder) {{
      Write-Output $w.Document.Folder.Self.Path
      break
    }}
  }} catch {{}}
}}
"#
            );
            match execute_ps51_local(&script, 1200) {
                Ok(text) => BrokerResponse::ok_text(text),
                Err(e) => BrokerResponse::err(e.to_string()),
            }
        }

        BrokerRequest::RunPs51 { script, timeout_ms } => {
            match execute_ps51_local(&script, timeout_ms) {
                Ok(text) => BrokerResponse::ok_text(text),
                Err(e) => BrokerResponse::err(e.to_string()),
            }
        }

        BrokerRequest::Spawn {
            exe,
            args,
            cwd,
            hidden,
        } => {
            let cwd_path = cwd.as_deref().map(Path::new);
            match spawn_medium(Path::new(&exe), &args, cwd_path, hidden) {
                Ok(pid) => BrokerResponse {
                    ok: true,
                    paths: Vec::new(),
                    text: String::new(),
                    pid: Some(pid),
                    error: None,
                },
                Err(e) => BrokerResponse::err(e.to_string()),
            }
        }

        BrokerRequest::StartHarnessTurn {
            turn_id,
            harness,
            exe,
            cwd,
            project_id,
            session_id,
            prompt,
            out_pipe,
        } => {
            match std::thread::Builder::new()
                .name(format!("med-harness-{harness}"))
                .spawn(move || {
                    crate::harness::execute_harness_turn_in_medium(
                        crate::harness::MediumTurnParams {
                            turn_id,
                            harness,
                            exe,
                            cwd,
                            project_id,
                            session_id,
                            prompt,
                            out_pipe,
                        },
                    );
                }) {
                Ok(_) => BrokerResponse::ok_text("started".to_string()),
                Err(e) => BrokerResponse::err(format!("failed to spawn harness worker: {e}")),
            }
        }

        BrokerRequest::CancelHarnessTurn { turn_id } => {
            let killed = crate::harness::kill_turn_pid_in_medium(&turn_id);
            BrokerResponse::ok_text(if killed { "killed" } else { "not_found" }.to_string())
        }
    }
}

fn parse_json_string_list(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "null" {
        return Vec::new();
    }
    if let Ok(vec) = serde_json::from_str::<Vec<String>>(trimmed) {
        return vec;
    }
    if let Ok(single) = serde_json::from_str::<String>(trimmed) {
        if !single.is_empty() {
            return vec![single];
        }
    }
    Vec::new()
}

/// Entry point when invoked as `anything-to-prompt.exe --broker <pipe_name>` at medium integrity.
pub fn run_broker_client(pipe_name: &str) -> Result<()> {
    let full_pipe = if pipe_name.starts_with(r"\\.\pipe\") {
        pipe_name.to_string()
    } else {
        format!(r"\\.\pipe\{pipe_name}")
    };

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(0)
        .open(&full_pipe)
        .with_context(|| format!("failed to connect broker pipe {full_pipe}"))?;

    let writer_file = file.try_clone().context("failed to clone pipe handle")?;
    let mut reader = BufReader::new(file);
    let mut writer = writer_file;
    let mut line = String::new();

    loop {
        line.clear();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let resp = match serde_json::from_str::<BrokerRequest>(trimmed) {
            Ok(req) => handle_request_in_medium(req),
            Err(e) => BrokerResponse::err(format!("invalid broker json: {e}")),
        };
        let encoded = serde_json::to_string(&resp)? + "\n";
        writer.write_all(encoded.as_bytes())?;
        writer.flush()?;
    }
    Ok(())
}

fn current_user_sid_string() -> Option<String> {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return None;
        }
        let mut needed = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
        if needed == 0 {
            let _ = CloseHandle(token);
            return None;
        }
        let mut buf = vec![0u8; needed as usize];
        let ok = GetTokenInformation(
            token,
            TokenUser,
            Some(buf.as_mut_ptr() as *mut _),
            needed,
            &mut needed,
        )
        .is_ok();
        let _ = CloseHandle(token);
        if !ok {
            return None;
        }
        let token_user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut str_sid = PSTR::null();
        if ConvertSidToStringSidA(token_user.User.Sid, &mut str_sid).is_err() || str_sid.is_null() {
            return None;
        }
        let cstr = std::ffi::CStr::from_ptr(str_sid.0 as *const i8);
        let res = cstr.to_string_lossy().into_owned();
        let _ = LocalFree(Some(HLOCAL(str_sid.0 as *mut std::ffi::c_void)));
        Some(res)
    }
}

struct ActivePipeConn {
    reader: BufReader<std::fs::File>,
    writer: std::fs::File,
    raw_handle: HANDLE,
}

impl Drop for ActivePipeConn {
    fn drop(&mut self) {
        unsafe {
            let _ = DisconnectNamedPipe(self.raw_handle);
        }
    }
}

pub struct OneshotReadPipe {
    raw_handle: HANDLE,
}

unsafe impl Send for OneshotReadPipe {}

impl OneshotReadPipe {
    pub fn accept_and_read(self) -> Result<BufReader<std::fs::File>> {
        let handle = self.raw_handle;
        std::mem::forget(self);
        unsafe {
            if let Err(e) = ConnectNamedPipe(handle, None) {
                let win_err = windows::core::Error::from_thread();
                if win_err.code().0 as u32 != 0x80070217 && e.code().0 as u32 != 0x80070217 {
                    let _ = CloseHandle(handle);
                    return Err(anyhow!("ConnectNamedPipe(stream) failed: {e}"));
                }
            }
            let file = std::fs::File::from_raw_handle(handle.0);
            Ok(BufReader::new(file))
        }
    }
}

impl Drop for OneshotReadPipe {
    fn drop(&mut self) {
        if self.raw_handle != INVALID_HANDLE_VALUE && !self.raw_handle.0.is_null() {
            unsafe {
                let _ = CloseHandle(self.raw_handle);
            }
        }
    }
}

pub struct BrokerClient {
    elevated: bool,
    conn: Mutex<Option<ActivePipeConn>>,
}

unsafe impl Send for BrokerClient {}
unsafe impl Sync for BrokerClient {}

impl Default for BrokerClient {
    fn default() -> Self {
        Self::new()
    }
}

impl BrokerClient {
    pub fn new() -> Self {
        Self {
            elevated: is_elevated(),
            conn: Mutex::new(None),
        }
    }

    pub fn create_oneshot_read_pipe(&self, tag: &str) -> Result<(String, OneshotReadPipe)> {
        let safe_tag: String = tag
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .take(32)
            .collect();
        let pipe_short = format!(
            "anything-to-prompt-stream-{}-{}-{}",
            std::process::id(),
            safe_tag,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        );
        let full_pipe = format!(r"\\.\pipe\{pipe_short}");
        let pipe_w = to_wide(&full_pipe);
        let trustee = current_user_sid_string().unwrap_or_else(|| "AU".to_string());
        let sddl = format!("D:(A;;GA;;;{trustee})S:(ML;;NW;;;ME)");
        let sddl_w = to_wide(&sddl);

        unsafe {
            let mut sd = PSECURITY_DESCRIPTOR::default();
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl_w.as_ptr()),
                SDDL_REVISION_1,
                &mut sd,
                None,
            )
            .context("ConvertStringSecurityDescriptorToSecurityDescriptorW failed for stream pipe")?;

            let sa = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: sd.0,
                bInheritHandle: false.into(),
            };

            let pipe_handle = CreateNamedPipeW(
                PCWSTR(pipe_w.as_ptr()),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                65536,
                65536,
                5000,
                Some(&sa),
            );
            let _ = LocalFree(Some(HLOCAL(sd.0)));

            if pipe_handle == INVALID_HANDLE_VALUE || pipe_handle.0.is_null() {
                return Err(anyhow!("CreateNamedPipeW failed for {full_pipe}"));
            }

            Ok((
                pipe_short,
                OneshotReadPipe {
                    raw_handle: pipe_handle,
                },
            ))
        }
    }

    pub fn call(&self, req: BrokerRequest) -> BrokerResponse {
        if !self.elevated {
            return handle_request_in_medium(req);
        }

        let mut guard = match self.conn.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };

        for attempt in 0..2 {
            if guard.is_none() {
                match Self::spawn_and_connect_pipe() {
                    Ok(c) => *guard = Some(c),
                    Err(e) => {
                        log::warn!("Failed to start medium broker (attempt {attempt}): {e}");
                        if attempt == 1 {
                            return BrokerResponse::err(e.to_string());
                        }
                        continue;
                    }
                }
            }

            if let Some(conn) = guard.as_mut() {
                match Self::roundtrip(conn, &req) {
                    Ok(resp) => return resp,
                    Err(e) => {
                        log::warn!("Broker pipe roundtrip failed (restarting): {e}");
                        *guard = None;
                    }
                }
            }
        }

        BrokerResponse::err("medium broker unavailable")
    }

    fn roundtrip(conn: &mut ActivePipeConn, req: &BrokerRequest) -> Result<BrokerResponse> {
        let line_out = serde_json::to_string(req)? + "\n";
        conn.writer.write_all(line_out.as_bytes())?;
        conn.writer.flush()?;

        let mut line_in = String::new();
        let n = conn.reader.read_line(&mut line_in)?;
        if n == 0 {
            return Err(anyhow!("broker pipe closed EOF"));
        }
        let resp = serde_json::from_str::<BrokerResponse>(line_in.trim())?;
        Ok(resp)
    }

    fn spawn_and_connect_pipe() -> Result<ActivePipeConn> {
        let pipe_short = format!(
            "anything-to-prompt-broker-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        );
        let full_pipe = format!(r"\\.\pipe\{pipe_short}");
        let pipe_w = to_wide(&full_pipe);

        // Build SDDL granting current user SID (or Authenticated Users) + Medium Mandatory Label
        let trustee = current_user_sid_string().unwrap_or_else(|| "AU".to_string());
        let sddl = format!("D:(A;;GA;;;{trustee})S:(ML;;NW;;;ME)");
        let sddl_w = to_wide(&sddl);

        unsafe {
            let mut sd = PSECURITY_DESCRIPTOR::default();
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl_w.as_ptr()),
                SDDL_REVISION_1,
                &mut sd,
                None,
            )
            .context("ConvertStringSecurityDescriptorToSecurityDescriptorW failed")?;

            let sa = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: sd.0,
                bInheritHandle: false.into(),
            };

            let pipe_handle = CreateNamedPipeW(
                PCWSTR(pipe_w.as_ptr()),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                65536,
                65536,
                3000,
                Some(&sa),
            );
            let _ = LocalFree(Some(windows::Win32::Foundation::HLOCAL(sd.0)));

            if pipe_handle == INVALID_HANDLE_VALUE || pipe_handle.0.is_null() {
                return Err(anyhow!("CreateNamedPipeW failed for {full_pipe}"));
            }

            let exe = std::env::current_exe().context("current_exe failed for broker")?;
            let args = vec!["--broker".to_string(), pipe_short];
            if let Err(e) = spawn_medium(&exe, &args, None, true) {
                let _ = CloseHandle(pipe_handle);
                return Err(e);
            }

            if let Err(e) = ConnectNamedPipe(pipe_handle, None) {
                // ERROR_PIPE_CONNECTED (535 -> 0x80070217) is success when client connected before ConnectNamedPipe
                let win_err = windows::core::Error::from_thread();
                if win_err.code().0 as u32 != 0x80070217 && e.code().0 as u32 != 0x80070217 {
                    let _ = CloseHandle(pipe_handle);
                    return Err(anyhow!("ConnectNamedPipe failed: {e}"));
                }
            }

            let file = std::fs::File::from_raw_handle(pipe_handle.0);
            let writer = file
                .try_clone()
                .context("failed to clone server pipe file")?;
            let reader = BufReader::new(file);
            Ok(ActivePipeConn {
                reader,
                writer,
                raw_handle: pipe_handle,
            })
        }
    }
}
