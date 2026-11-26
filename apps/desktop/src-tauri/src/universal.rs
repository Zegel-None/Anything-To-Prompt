use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_CONTROL,
    VK_RETURN, VK_V,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, GetClassNameW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow,
    IsWindowVisible, SetForegroundWindow, ShowWindow, SW_RESTORE,
};

const CF_UNICODETEXT_U32: u32 = 13;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetWindowInfo {
    pub hwnd: isize,
    pub title: String,
    pub process_name: String,
    pub display_label: String,
    pub valid: bool,
    pub updated_at_ms: i64,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn get_window_title(hwnd: HWND) -> String {
    let mut buf = [0u16; 512];
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if len <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..len as usize]).trim().to_string()
}

fn get_window_class(hwnd: HWND) -> String {
    let mut buf = [0u16; 128];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    if len <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..len as usize]).trim().to_string()
}

fn get_process_basename(pid: u32) -> String {
    if pid == 0 {
        return String::new();
    }
    unsafe {
        let Ok(proc_handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return String::new();
        };
        let mut buf = [0u16; 512];
        let mut size = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            proc_handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut size,
        )
        .is_ok();
        let _ = CloseHandle(proc_handle);
        if !ok || size == 0 {
            return String::new();
        }
        let full = String::from_utf16_lossy(&buf[..size as usize]);
        Path::new(&full)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string()
    }
}

/// Captures metadata for a non-ATP foreground window so Universal Drop-In knows exactly
/// where to restore focus and paste the user's structured prompt.
pub fn capture_target_window(raw_hwnd: isize) -> Option<TargetWindowInfo> {
    if raw_hwnd == 0 {
        return None;
    }
    let hwnd = HWND(raw_hwnd as *mut core::ffi::c_void);
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() || !IsWindowVisible(hwnd).as_bool() {
            return None;
        }
    }
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    if pid == 0 || pid == std::process::id() {
        return None;
    }
    let class_name = get_window_class(hwnd);
    if matches!(
        class_name.as_str(),
        "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "Progman" | "WorkerW" | "NotifyIconOverflowWindow"
    ) {
        return None;
    }
    let title = get_window_title(hwnd);
    let process_name = get_process_basename(pid);
    if title.is_empty() && process_name.is_empty() {
        return None;
    }
    let display_label = format_target_label(&process_name, &title);
    Some(TargetWindowInfo {
        hwnd: raw_hwnd,
        title,
        process_name,
        display_label,
        valid: true,
        updated_at_ms: now_ms(),
    })
}

pub fn format_target_label(process_name: &str, title: &str) -> String {
    let p = process_name.trim();
    let t = title.trim();
    if t.is_empty() {
        return if p.is_empty() {
            "当前窗口".to_string()
        } else {
            p.to_string()
        };
    }
    let short_title: String = t.chars().take(28).collect();
    let ellip = if t.chars().count() > 28 { "…" } else { "" };
    if p.is_empty() || t.to_ascii_lowercase().contains(&p.to_ascii_lowercase()) {
        format!("{short_title}{ellip}")
    } else {
        format!("{p} · {short_title}{ellip}")
    }
}

pub fn is_target_hwnd_still_valid(raw_hwnd: isize) -> bool {
    if raw_hwnd == 0 {
        return false;
    }
    let hwnd = HWND(raw_hwnd as *mut core::ffi::c_void);
    unsafe { IsWindow(Some(hwnd)).as_bool() }
}

fn read_clipboard_unicode_text() -> Option<String> {
    unsafe {
        if OpenClipboard(None).is_err() {
            return None;
        }
        let mut out = None;
        if let Ok(handle) = GetClipboardData(CF_UNICODETEXT_U32) {
            let hglobal = HGLOBAL(handle.0);
            let ptr = GlobalLock(hglobal) as *const u16;
            if !ptr.is_null() {
                let mut len = 0usize;
                while *ptr.add(len) != 0 && len < 2_000_000 {
                    len += 1;
                }
                let slice = std::slice::from_raw_parts(ptr, len);
                out = Some(String::from_utf16_lossy(slice));
                let _ = GlobalUnlock(hglobal);
            }
        }
        let _ = CloseClipboard();
        out
    }
}

pub fn write_clipboard_unicode_text(text: &str) -> Result<(), String> {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let byte_len = wide.len() * 2;
    unsafe {
        let mut opened = false;
        for _ in 0..6 {
            if OpenClipboard(None).is_ok() {
                opened = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        if !opened {
            return Err("无法打开 Windows 剪贴板（可能正被其他应用占用）".to_string());
        }
        let _ = EmptyClipboard();
        let hglobal = match GlobalAlloc(GMEM_MOVEABLE, byte_len) {
            Ok(h) => h,
            Err(e) => {
                let _ = CloseClipboard();
                return Err(format!("分配剪贴板内存失败: {e}"));
            }
        };
        let dst = GlobalLock(hglobal) as *mut u16;
        if dst.is_null() {
            let _ = GlobalFree(Some(hglobal));
            let _ = CloseClipboard();
            return Err("锁定剪贴板内存失败".to_string());
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), dst, wide.len());
        let _ = GlobalUnlock(hglobal);
        if SetClipboardData(CF_UNICODETEXT_U32, Some(HANDLE(hglobal.0))).is_err() {
            let _ = GlobalFree(Some(hglobal));
            let _ = CloseClipboard();
            return Err("写入剪贴板失败".to_string());
        }
        let _ = CloseClipboard();
        Ok(())
    }
}

fn make_key_input(vk: VIRTUAL_KEY, key_up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if key_up {
                    KEYEVENTF_KEYUP
                } else {
                    Default::default()
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send_ctrl_v() {
    let inputs = [
        make_key_input(VK_CONTROL, false),
        make_key_input(VK_V, false),
        make_key_input(VK_V, true),
        make_key_input(VK_CONTROL, true),
    ];
    unsafe {
        let _ = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

fn send_enter() {
    let inputs = [
        make_key_input(VK_RETURN, false),
        make_key_input(VK_RETURN, true),
    ];
    unsafe {
        let _ = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

/// Safely delivers `text` to the previously focused non-ATP window via clipboard + Ctrl+V
/// (and optional Enter), restoring the user's previous clipboard contents afterward.
pub fn deliver_to_target_window(
    target_hwnd: isize,
    text: &str,
    mode: &str, // "paste_only" | "paste_and_enter" | "clipboard_only"
) -> Result<String, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("发送内容为空".to_string());
    }
    if mode == "clipboard_only" {
        write_clipboard_unicode_text(trimmed)?;
        return Ok("已复制 Prompt 到剪贴板".to_string());
    }

    if !is_target_hwnd_still_valid(target_hwnd) {
        write_clipboard_unicode_text(trimmed)?;
        return Ok("原目标窗口已关闭，Prompt 已复制到剪贴板，请直接 Ctrl+V 粘贴".to_string());
    }

    let previous_clipboard = read_clipboard_unicode_text();
    write_clipboard_unicode_text(trimmed)?;

    let hwnd = HWND(target_hwnd as *mut core::ffi::c_void);
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        crate::window::focus_hwnd_without_sendinput(hwnd);
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
    }

    std::thread::sleep(Duration::from_millis(95));
    send_ctrl_v();

    if mode == "paste_and_enter" {
        std::thread::sleep(Duration::from_millis(95));
        send_enter();
    }

    // Give target app (Electron / Chrome / WebView) time to read CF_UNICODETEXT before restoring
    if let Some(prev) = previous_clipboard {
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(320));
            let _ = write_clipboard_unicode_text(&prev);
        });
    }

    let summary = if mode == "paste_and_enter" {
        "已投递并发送到目标窗口"
    } else {
        "已粘贴到目标窗口输入框"
    };
    Ok(summary.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_target_window_labels_cleanly() {
        assert_eq!(format_target_label("chrome", "ChatGPT - Google Chrome"), "ChatGPT - Google Chrome");
        assert_eq!(format_target_label("WeChat", "产品讨论群"), "WeChat · 产品讨论群");
        assert_eq!(format_target_label("", ""), "当前窗口");
        let long = "A".repeat(40);
        let formatted = format_target_label("Cursor", &long);
        assert!(formatted.starts_with("Cursor · "));
        assert!(formatted.ends_with('…'));
    }
}
