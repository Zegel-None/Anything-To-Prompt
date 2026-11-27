//! Zero-UIA native Win32 control memory readers:
//! - `#32768` popup menus via `MN_GETHMENU` + `GetMenuItemRect` + `GetMenuItemInfoW` (exact rect, text, submenu flag)
//! - `ToolbarWindow32` (`TB_BUTTONCOUNT` + `TB_GETITEMRECT`) and `SysTabControl32` (`TCM_GETITEMCOUNT` + `TCM_GETITEMRECT`)
//!   via guarded cross-process `VirtualAllocEx` + `SendMessageTimeoutW(80ms)` + RAII `VirtualFreeEx`.
//! 100% immune to window move, resize, DPI change, and zero risk of SolidWorks UIA hangs!

use crate::captures::ScreenRect;
use crate::pick::hitmap::{HitMapEntry, HitMapSource};
use std::ffi::c_void;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows::Win32::System::Memory::{
    VirtualAllocEx, VirtualFreeEx, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_VM_OPERATION, PROCESS_VM_READ, PROCESS_VM_WRITE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetMenuItemCount, GetMenuItemInfoW, GetMenuItemRect, GetWindowThreadProcessId,
    SendMessageTimeoutW, HMENU, MENUITEMINFOW, MFT_SEPARATOR, MIIM_FTYPE, MIIM_ID, MIIM_STRING,
    MIIM_SUBMENU, SMTO_ABORTIFHUNG, SMTO_BLOCK,
};

const MN_GETHMENU: u32 = 0x01E1;
const TB_BUTTONCOUNT: u32 = 0x0418;
const TB_GETITEMRECT: u32 = 0x041D;
const TCM_GETITEMCOUNT: u32 = 0x1304;
const TCM_GETITEMRECT: u32 = 0x130A;
const MSG_TIMEOUT_MS: u32 = 80;
const MAX_READ_ITEMS: usize = 96;

/// Strips Win32 mnemonic `&` markers (preserving `&&` -> `&`) and trailing shortcut tabs (`\tCtrl+O`).
pub fn clean_win32_menu_label(raw: &str) -> String {
    let main_part = raw.split('\t').next().unwrap_or(raw).trim();
    let mut out = String::with_capacity(main_part.len());
    let mut chars = main_part.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '&' {
            if chars.peek() == Some(&'&') {
                out.push('&');
                chars.next();
            }
            continue;
        }
        out.push(ch);
    }
    out.trim().to_string()
}

fn send_msg_timeout(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> Option<usize> {
    let mut result: usize = 0;
    let ok = unsafe {
        SendMessageTimeoutW(
            hwnd,
            msg,
            WPARAM(wparam),
            LPARAM(lparam),
            SMTO_ABORTIFHUNG | SMTO_BLOCK,
            MSG_TIMEOUT_MS,
            Some(&mut result),
        )
    };
    if ok.0 != 0 {
        Some(result)
    } else {
        None
    }
}

/// Reads all visible items of a native Win32 `#32768` popup menu directly via `MN_GETHMENU`,
/// returning exact real-time rects, exact localized text labels (no OCR needed!), and exact
/// `is_expandable` submenu flags (`!hSubMenu.is_invalid()`).
pub fn read_win32_popup_menu_entries(popup_hwnd: HWND) -> Vec<HitMapEntry> {
    if popup_hwnd.0.is_null() {
        return Vec::new();
    }
    let Some(hmenu_raw) = send_msg_timeout(popup_hwnd, MN_GETHMENU, 0, 0) else {
        return Vec::new();
    };
    if hmenu_raw == 0 {
        return Vec::new();
    }
    let hmenu = HMENU(hmenu_raw as *mut c_void);
    let count = unsafe { GetMenuItemCount(Some(hmenu)) };
    if count <= 0 {
        return Vec::new();
    }

    let mut out = Vec::new();
    for i in 0..(count as usize).min(MAX_READ_ITEMS) {
        let mut rc = RECT::default();
        if unsafe { GetMenuItemRect(None, hmenu, i as u32, &mut rc) }.is_err() {
            continue;
        }
        let rect = ScreenRect {
            x: rc.left,
            y: rc.top,
            w: (rc.right - rc.left).max(0),
            h: (rc.bottom - rc.top).max(0),
        };
        if rect.w < 16 || rect.h < 12 {
            continue;
        }

        let mut buf = [0u16; 160];
        let mut mii = MENUITEMINFOW {
            cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_STRING | MIIM_SUBMENU | MIIM_FTYPE | MIIM_ID,
            dwTypeData: windows::core::PWSTR(buf.as_mut_ptr()),
            cch: (buf.len() - 1) as u32,
            ..Default::default()
        };
        if unsafe { GetMenuItemInfoW(hmenu, i as u32, true, &mut mii) }.is_err() {
            continue;
        }
        if (mii.fType & MFT_SEPARATOR) == MFT_SEPARATOR {
            continue;
        }
        let len = (mii.cch as usize).min(buf.len() - 1);
        let raw_label = String::from_utf16_lossy(&buf[..len]);
        let label = clean_win32_menu_label(&raw_label);
        let has_submenu = !mii.hSubMenu.0.is_null();
        let trigger_pt = if has_submenu {
            Some((rect.x + rect.w / 2, rect.y + rect.h / 2))
        } else {
            None
        };
        out.push(HitMapEntry {
            rect,
            label,
            description: format!("win32_menu_id={}", mii.wID),
            role: "MenuItem".to_string(),
            source: HitMapSource::MsaaSnapshot,
            is_expandable: has_submenu,
            trigger_pt,
        });
    }
    out
}

struct RemoteRectBuf {
    proc_handle: HANDLE,
    remote_ptr: *mut c_void,
}

impl RemoteRectBuf {
    fn alloc_for_hwnd(hwnd: HWND) -> Option<Self> {
        let mut pid = 0u32;
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
        }
        if pid == 0 {
            return None;
        }
        let proc_handle = unsafe {
            OpenProcess(
                PROCESS_VM_OPERATION | PROCESS_VM_READ | PROCESS_VM_WRITE,
                false,
                pid,
            )
            .ok()?
        };
        let remote_ptr = unsafe {
            VirtualAllocEx(
                proc_handle,
                None,
                std::mem::size_of::<RECT>(),
                MEM_COMMIT | MEM_RESERVE,
                PAGE_READWRITE,
            )
        };
        if remote_ptr.is_null() {
            unsafe {
                let _ = CloseHandle(proc_handle);
            }
            return None;
        }
        Some(Self {
            proc_handle,
            remote_ptr,
        })
    }

    fn read_rect(&self) -> Option<RECT> {
        let mut rc = RECT::default();
        let mut read_bytes = 0usize;
        let ok = unsafe {
            ReadProcessMemory(
                self.proc_handle,
                self.remote_ptr,
                &mut rc as *mut RECT as *mut c_void,
                std::mem::size_of::<RECT>(),
                Some(&mut read_bytes),
            )
        };
        if ok.is_ok() && read_bytes == std::mem::size_of::<RECT>() {
            Some(rc)
        } else {
            None
        }
    }
}

impl Drop for RemoteRectBuf {
    fn drop(&mut self) {
        unsafe {
            let _ = VirtualFreeEx(self.proc_handle, self.remote_ptr, 0, MEM_RELEASE);
            let _ = CloseHandle(self.proc_handle);
        }
    }
}

fn client_rect_to_screen(hwnd: HWND, rc: RECT) -> Option<ScreenRect> {
    let mut tl = POINT {
        x: rc.left,
        y: rc.top,
    };
    let mut br = POINT {
        x: rc.right,
        y: rc.bottom,
    };
    unsafe {
        if !ClientToScreen(hwnd, &mut tl).as_bool() || !ClientToScreen(hwnd, &mut br).as_bool() {
            return None;
        }
    }
    let w = (br.x - tl.x).max(0);
    let h = (br.y - tl.y).max(0);
    if w < 12 || h < 12 {
        return None;
    }
    Some(ScreenRect {
        x: tl.x,
        y: tl.y,
        w,
        h,
    })
}

/// Reads exact real-time item rectangles for standard Win32 `ToolbarWindow32` or `SysTabControl32`
/// controls using native Win32 messages (zero UIA, immune to window resize/move).
pub fn read_win32_toolbar_or_tab_entries(hwnd: HWND, class_lower: &str) -> Vec<HitMapEntry> {
    if hwnd.0.is_null() {
        return Vec::new();
    }
    let (count_msg, rect_msg, role) = if class_lower.contains("toolbarwindow32") {
        (TB_BUTTONCOUNT, TB_GETITEMRECT, "Button")
    } else if class_lower.contains("systabcontrol32") {
        (TCM_GETITEMCOUNT, TCM_GETITEMRECT, "TabItem")
    } else {
        return Vec::new();
    };

    let Some(raw_cnt) = send_msg_timeout(hwnd, count_msg, 0, 0) else {
        return Vec::new();
    };
    if raw_cnt == 0 || raw_cnt > 256 {
        return Vec::new();
    }
    let Some(buf) = RemoteRectBuf::alloc_for_hwnd(hwnd) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for i in 0..raw_cnt.min(MAX_READ_ITEMS) {
        let Some(ok) = send_msg_timeout(hwnd, rect_msg, i, buf.remote_ptr as isize) else {
            break;
        };
        if ok == 0 {
            continue;
        }
        let Some(rc) = buf.read_rect() else {
            continue;
        };
        if let Some(rect) = client_rect_to_screen(hwnd, rc) {
            out.push(HitMapEntry {
                rect,
                label: String::new(),
                description: format!("win32_{role}_{i}"),
                role: role.to_string(),
                source: HitMapSource::MsaaSnapshot,
                is_expandable: false,
                trigger_pt: None,
            });
        }
    }
    out
}

/// Unified entry point: dispatches `#32768` menus, `ToolbarWindow32`, and `SysTabControl32`.
pub fn read_native_win32_control_entries(hwnd: HWND, class_name: &str) -> Vec<HitMapEntry> {
    let low = class_name.to_ascii_lowercase();
    if low == "#32768" {
        return read_win32_popup_menu_entries(hwnd);
    }
    if low.contains("toolbarwindow32") || low.contains("systabcontrol32") {
        return read_win32_toolbar_or_tab_entries(hwnd, &low);
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_win32_menu_mnemonics_and_shortcut_tabs() {
        assert_eq!(clean_win32_menu_label("&Open...\tCtrl+O"), "Open...");
        assert_eq!(clean_win32_menu_label("文件(&F)"), "文件(F)");
        assert_eq!(clean_win32_menu_label("Save && &Exit\tAlt+F4"), "Save & Exit");
    }
}
