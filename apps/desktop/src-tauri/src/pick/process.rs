use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::c_void;
use std::path::Path;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HMODULE};
use windows::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, MIB_TCP6TABLE_OWNER_PID, MIB_TCPTABLE_OWNER_PID, MIB_TCP_STATE_LISTEN,
    TCP_TABLE_OWNER_PID_LISTENER,
};
use windows::Win32::Networking::WinSock::{AF_INET, AF_INET6};
use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
};
use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT, PROCESS_QUERY_INFORMATION,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProcessMetadata {
    pub pid: u32,
    pub exe_path: String,
    pub exe_name: String,
    pub install_dir: String,
    pub cmdline: String,
    pub cwd: Option<String>,
    pub product_name: String,
    pub file_description: String,
    pub file_version: String,
    pub company_name: String,
    pub framework_guess: String,
    pub mapped_source_dir: Option<String>,
}

pub fn inspect_process(pid: u32, app_source_map: &HashMap<String, String>) -> ProcessMetadata {
    if pid == 0 {
        return ProcessMetadata::default();
    }

    let exe_path = query_exe_path(pid).unwrap_or_default();
    let path_obj = Path::new(&exe_path);
    let exe_name = path_obj
        .file_name()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let install_dir = path_obj
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    let cmdline = query_wmi_cmdline(pid).unwrap_or_default();
    let cwd = read_process_cwd(pid);
    let ver = query_version_strings(&exe_path);
    let framework_guess = guess_framework(&exe_path, &exe_name, &install_dir);

    let mapped_source_dir = lookup_source_map(app_source_map, &exe_name, &exe_path);

    ProcessMetadata {
        pid,
        exe_path,
        exe_name,
        install_dir,
        cmdline,
        cwd,
        product_name: ver.product_name,
        file_description: ver.file_description,
        file_version: ver.file_version,
        company_name: ver.company_name,
        framework_guess,
        mapped_source_dir,
    }
}

fn lookup_source_map(
    map: &HashMap<String, String>,
    exe_name: &str,
    exe_path: &str,
) -> Option<String> {
    let lower_path = exe_path.to_ascii_lowercase();
    for (k, v) in map {
        let lk = k.trim().to_ascii_lowercase();
        if !lk.is_empty() && (lk == exe_name || lk == lower_path || lower_path.ends_with(&lk)) {
            return Some(v.clone());
        }
    }
    None
}

pub fn query_exe_path(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = vec![0u16; 1024];
        let mut size = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT(0),
            PWSTR(buf.as_mut_ptr()),
            &mut size,
        )
        .is_ok();
        let _ = CloseHandle(handle);
        if ok && size > 0 {
            Some(String::from_utf16_lossy(&buf[..size as usize]))
        } else {
            None
        }
    }
}

pub fn query_wmi_cmdline(pid: u32) -> Option<String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct Win32ProcRow {
        command_line: Option<String>,
    }

    let conn = wmi::WMIConnection::new().ok()?;
    let query = format!("SELECT CommandLine FROM Win32_Process WHERE ProcessId = {pid}");
    let rows: Vec<Win32ProcRow> = conn.raw_query(&query).ok()?;
    rows.into_iter()
        .find_map(|r| r.command_line)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[derive(Default)]
struct VersionStrings {
    product_name: String,
    file_description: String,
    file_version: String,
    company_name: String,
}

fn query_version_strings(exe_path: &str) -> VersionStrings {
    if exe_path.is_empty() {
        return VersionStrings::default();
    }
    let path_w = crate::elevation::to_wide(exe_path);
    unsafe {
        let mut dummy = 0u32;
        let size = GetFileVersionInfoSizeW(PCWSTR(path_w.as_ptr()), Some(&mut dummy));
        if size == 0 {
            return VersionStrings::default();
        }
        let mut data = vec![0u8; size as usize];
        if GetFileVersionInfoW(
            PCWSTR(path_w.as_ptr()),
            Some(0),
            size,
            data.as_mut_ptr() as *mut c_void,
        )
        .is_err()
        {
            return VersionStrings::default();
        }

        // Read VarFileInfo\Translation if present
        let mut lang_codes = Vec::new();
        let trans_key = crate::elevation::to_wide(r"\VarFileInfo\Translation");
        let mut trans_ptr: *mut c_void = std::ptr::null_mut();
        let mut trans_len = 0u32;
        if VerQueryValueW(
            data.as_ptr() as *const c_void,
            PCWSTR(trans_key.as_ptr()),
            &mut trans_ptr,
            &mut trans_len,
        )
        .as_bool()
            && !trans_ptr.is_null()
            && trans_len >= 4
        {
            let pair = *(trans_ptr as *const [u16; 2]);
            lang_codes.push(format!("{:04x}{:04x}", pair[0], pair[1]));
        }
        lang_codes.push("040904b0".to_string());
        lang_codes.push("080404b0".to_string());
        lang_codes.push("040904e4".to_string());

        let read_field = |field: &str| -> String {
            for lc in &lang_codes {
                let sub = format!(r"\StringFileInfo\{lc}\{field}");
                let sub_w = crate::elevation::to_wide(&sub);
                let mut val_ptr: *mut c_void = std::ptr::null_mut();
                let mut val_len = 0u32;
                if VerQueryValueW(
                    data.as_ptr() as *const c_void,
                    PCWSTR(sub_w.as_ptr()),
                    &mut val_ptr,
                    &mut val_len,
                )
                .as_bool()
                    && !val_ptr.is_null()
                    && val_len > 0
                {
                    let slice = std::slice::from_raw_parts(val_ptr as *const u16, val_len as usize);
                    let end = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
                    let s = String::from_utf16_lossy(&slice[..end]).trim().to_string();
                    if !s.is_empty() {
                        return s;
                    }
                }
            }
            String::new()
        };

        VersionStrings {
            product_name: read_field("ProductName"),
            file_description: read_field("FileDescription"),
            file_version: read_field("FileVersion"),
            company_name: read_field("CompanyName"),
        }
    }
}

fn guess_framework(exe_path: &str, exe_name: &str, install_dir: &str) -> String {
    if install_dir.is_empty() {
        return "Unknown".to_string();
    }
    let dir = Path::new(install_dir);
    if dir.join("resources").join("app.asar").exists() || dir.join("resources").join("app").is_dir()
    {
        return "Electron".to_string();
    }
    if dir.join("WebView2Loader.dll").exists() {
        return "WebView2 / Tauri".to_string();
    }
    if dir.join("Qt6Core.dll").exists() || dir.join("Qt5Core.dll").exists() {
        return "Qt".to_string();
    }
    if dir.join("chrome_elf.dll").exists() {
        return "Chromium".to_string();
    }
    if exe_name.starts_with("python") || dir.join("python3.dll").exists() {
        return "Python".to_string();
    }
    if dir.join("jbr").is_dir() || exe_name.contains("java") || exe_name.contains("idea64") {
        return "JVM / JetBrains".to_string();
    }
    if let Some(stem) = Path::new(exe_path).file_stem().and_then(|s| s.to_str()) {
        if dir.join(format!("{stem}.runtimeconfig.json")).exists()
            || dir.join(format!("{stem}.exe.config")).exists()
        {
            return ".NET (WPF / WinForms)".to_string();
        }
    }
    "Win32 / Native".to_string()
}

/// Reads a 64-bit target process's current working directory from its PEB.
#[cfg(target_pointer_width = "64")]
pub fn read_process_cwd(pid: u32) -> Option<String> {
    #[repr(C)]
    #[derive(Default)]
    struct ProcessBasicInformation {
        reserved1: *mut c_void,
        peb_base_address: usize,
        reserved2: [*mut c_void; 2],
        unique_process_id: usize,
        reserved3: *mut c_void,
    }

    #[repr(C)]
    #[derive(Default, Copy, Clone)]
    struct UnicodeString64 {
        length: u16,
        maximum_length: u16,
        _pad: u32,
        buffer: usize,
    }

    type NtQueryInfoProc =
        unsafe extern "system" fn(HANDLE, u32, *mut c_void, u32, *mut u32) -> i32;

    unsafe {
        let ntdll_w = crate::elevation::to_wide("ntdll.dll");
        let ntdll: HMODULE = GetModuleHandleW(PCWSTR(ntdll_w.as_ptr())).ok()?;
        let proc_addr = GetProcAddress(ntdll, windows::core::s!("NtQueryInformationProcess"))?;
        let nt_query: NtQueryInfoProc = std::mem::transmute(proc_addr);

        let proc_handle =
            OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid).ok()?;

        let mut pbi = ProcessBasicInformation::default();
        let mut ret_len = 0u32;
        let status = nt_query(
            proc_handle,
            0, // ProcessBasicInformation
            &mut pbi as *mut _ as *mut c_void,
            std::mem::size_of::<ProcessBasicInformation>() as u32,
            &mut ret_len,
        );
        if status != 0 || pbi.peb_base_address == 0 {
            let _ = CloseHandle(proc_handle);
            return None;
        }

        // On x64 Windows, PEB + 0x20 is ProcessParameters pointer
        let mut params_ptr: usize = 0;
        if ReadProcessMemory(
            proc_handle,
            (pbi.peb_base_address + 0x20) as *const c_void,
            &mut params_ptr as *mut _ as *mut c_void,
            std::mem::size_of::<usize>(),
            None,
        )
        .is_err()
            || params_ptr == 0
        {
            let _ = CloseHandle(proc_handle);
            return None;
        }

        // On x64 Windows, RTL_USER_PROCESS_PARAMETERS + 0x38 is CurrentDirectory.DosPath (UNICODE_STRING)
        let mut dos_path = UnicodeString64::default();
        if ReadProcessMemory(
            proc_handle,
            (params_ptr + 0x38) as *const c_void,
            &mut dos_path as *mut _ as *mut c_void,
            std::mem::size_of::<UnicodeString64>(),
            None,
        )
        .is_err()
            || dos_path.length == 0
            || dos_path.length > 2048
            || dos_path.buffer == 0
        {
            let _ = CloseHandle(proc_handle);
            return None;
        }

        let u16_count = (dos_path.length as usize) / 2;
        let mut str_buf = vec![0u16; u16_count];
        let ok = ReadProcessMemory(
            proc_handle,
            dos_path.buffer as *const c_void,
            str_buf.as_mut_ptr() as *mut c_void,
            dos_path.length as usize,
            None,
        )
        .is_ok();
        let _ = CloseHandle(proc_handle);

        if ok {
            let s = String::from_utf16_lossy(&str_buf)
                .trim_end_matches('\\')
                .to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
        None
    }
}

#[cfg(not(target_pointer_width = "64"))]
pub fn read_process_cwd(_pid: u32) -> Option<String> {
    None
}

/// Resolves a listening localhost TCP port (IPv4 or IPv6) to its owning PID.
pub fn find_listening_pid_for_tcp_port(target_port: u16) -> Option<u32> {
    unsafe {
        // 1. IPv4 TCP table
        let mut size = 0u32;
        let _ = GetExtendedTcpTable(
            None,
            &mut size,
            false,
            AF_INET.0 as u32,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        );
        if size > 0 {
            let mut buf = vec![0u8; size as usize];
            if GetExtendedTcpTable(
                Some(buf.as_mut_ptr() as *mut c_void),
                &mut size,
                false,
                AF_INET.0 as u32,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            ) == 0
            {
                let table = &*(buf.as_ptr() as *const MIB_TCPTABLE_OWNER_PID);
                let rows =
                    std::slice::from_raw_parts(table.table.as_ptr(), table.dwNumEntries as usize);
                for row in rows {
                    if row.dwState == MIB_TCP_STATE_LISTEN.0 as u32 {
                        let port = u16::from_be((row.dwLocalPort & 0xFFFF) as u16);
                        if port == target_port && row.dwOwningPid != 0 {
                            return Some(row.dwOwningPid);
                        }
                    }
                }
            }
        }

        // 2. IPv6 TCP table
        let mut size6 = 0u32;
        let _ = GetExtendedTcpTable(
            None,
            &mut size6,
            false,
            AF_INET6.0 as u32,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        );
        if size6 > 0 {
            let mut buf6 = vec![0u8; size6 as usize];
            if GetExtendedTcpTable(
                Some(buf6.as_mut_ptr() as *mut c_void),
                &mut size6,
                false,
                AF_INET6.0 as u32,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            ) == 0
            {
                let table6 = &*(buf6.as_ptr() as *const MIB_TCP6TABLE_OWNER_PID);
                let rows6 =
                    std::slice::from_raw_parts(table6.table.as_ptr(), table6.dwNumEntries as usize);
                for row in rows6 {
                    if row.dwState == MIB_TCP_STATE_LISTEN.0 as u32 {
                        let port = u16::from_be((row.dwLocalPort & 0xFFFF) as u16);
                        if port == target_port && row.dwOwningPid != 0 {
                            return Some(row.dwOwningPid);
                        }
                    }
                }
            }
        }
    }
    None
}

/// Lists direct child processes `(pid, exe_name)` of `parent_pid`.
pub fn list_child_processes(parent_pid: u32) -> Vec<(u32, String)> {
    let mut children = Vec::new();
    if parent_pid == 0 {
        return children;
    }
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return children;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snap, &mut entry).is_ok() {
            loop {
                if entry.th32ParentProcessID == parent_pid && entry.th32ProcessID != parent_pid {
                    let end = entry
                        .szExeFile
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(entry.szExeFile.len());
                    let name = String::from_utf16_lossy(&entry.szExeFile[..end]);
                    children.push((entry.th32ProcessID, name));
                }
                if Process32NextW(snap, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    children
}
