//! Shared Win32 utility helpers — single canonical owner for UTF-16 conversion,
//! NUL-safe window title reads, and current-user SID lookup.

use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, HWND, LocalFree};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::WindowsAndMessaging::{GetWindowTextLengthW, GetWindowTextW};

/// Encodes a Rust `&str` as a NUL-terminated UTF-16 `Vec<u16>` (stripping any embedded NULs).
pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16()
        .filter(|&u| u != 0)
        .chain(std::iter::once(0))
        .collect()
}

/// Reads a Win32 window's title text safely, truncating at the first embedded NUL.
pub fn window_title_text(hwnd: HWND) -> String {
    if hwnd.0.is_null() {
        return String::new();
    }
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len <= 0 {
        return String::new();
    }
    let cap = (len as usize).min(512) + 1;
    let mut buf = vec![0u16; cap];
    let got = unsafe { GetWindowTextW(hwnd, &mut buf) }.max(0) as usize;
    let slice = &buf[..got.min(buf.len())];
    let nul_pos = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
    String::from_utf16_lossy(&slice[..nul_pos])
}

/// Returns the current Windows user's SID string (e.g. `S-1-5-21-...`) for per-user pipe ACLs.
pub fn current_user_sid_string() -> Option<String> {
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
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut sid_ptr = PWSTR::null();
        if ConvertSidToStringSidW(user.User.Sid, &mut sid_ptr).is_err() || sid_ptr.0.is_null() {
            return None;
        }
        let mut len = 0usize;
        while *sid_ptr.0.add(len) != 0 {
            len += 1;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(sid_ptr.0, len));
        let _ = LocalFree(Some(HLOCAL(sid_ptr.0 as *mut _)));
        Some(s)
    }
}
