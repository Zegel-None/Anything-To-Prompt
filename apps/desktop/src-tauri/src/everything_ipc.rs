use std::cell::RefCell;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, FindWindowW, RegisterClassW,
    SendMessageTimeoutW, HWND_MESSAGE, SMTO_ABORTIFHUNG, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_COPYDATA, WNDCLASSW,
};

const EVERYTHING_IPC_COPYDATAQUERYW: usize = 2;
const EVERYTHING_IPC_REPLY_ID: u32 = 0x4154_5031; // 'ATP1'
const EVERYTHING_IPC_FOLDER_FLAG: u32 = 0x0000_0001;

#[derive(Debug, Clone)]
pub struct EverythingHit {
    pub is_dir: bool,
    pub name: String,
    pub path: PathBuf,
}

thread_local! {
    static REPLY_BUFFER: RefCell<Option<Vec<EverythingHit>>> = const { RefCell::new(None) };
}

fn to_wide_null(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Fast (<0.05ms) check for a live Everything 1.4 or 1.5a IPC notification window.
pub fn find_everything_ipc_hwnd() -> Option<HWND> {
    for cls in [
        "EVERYTHING_TASKBAR_NOTIFICATION",
        "EVERYTHING_TASKBAR_NOTIFICATION_(1.5a)",
    ] {
        let w = to_wide_null(cls);
        let hwnd = unsafe { FindWindowW(PCWSTR(w.as_ptr()), None) }.ok()?;
        if !hwnd.0.is_null() {
            return Some(hwnd);
        }
    }
    None
}

pub fn is_everything_running() -> bool {
    find_everything_ipc_hwnd().is_some()
}

fn read_utf16_z(buf: &[u8], byte_offset: usize) -> Option<String> {
    if byte_offset >= buf.len() || !byte_offset.is_multiple_of(2) {
        return None;
    }
    let mut u16s = Vec::with_capacity(64);
    let mut i = byte_offset;
    while i + 1 < buf.len() {
        let code = u16::from_le_bytes([buf[i], buf[i + 1]]);
        if code == 0 {
            break;
        }
        u16s.push(code);
        i += 2;
    }
    Some(String::from_utf16_lossy(&u16s))
}

pub fn parse_everything_list_w(buf: &[u8]) -> Vec<EverythingHit> {
    // EVERYTHING_IPC_LISTW header: 7 x u32 = 28 bytes
    if buf.len() < 28 {
        return Vec::new();
    }
    let num_items = u32::from_le_bytes(buf[20..24].try_into().unwrap_or([0; 4])) as usize;
    let items_cap = num_items.min(64);
    let mut out = Vec::with_capacity(items_cap);

    for idx in 0..items_cap {
        let item_off = 28 + idx * 12;
        if item_off + 12 > buf.len() {
            break;
        }
        let flags = u32::from_le_bytes(buf[item_off..item_off + 4].try_into().unwrap_or([0; 4]));
        let fn_off =
            u32::from_le_bytes(buf[item_off + 4..item_off + 8].try_into().unwrap_or([0; 4]))
                as usize;
        let path_off =
            u32::from_le_bytes(buf[item_off + 8..item_off + 12].try_into().unwrap_or([0; 4]))
                as usize;

        let Some(name) = read_utf16_z(buf, fn_off) else {
            continue;
        };
        let dir_path = read_utf16_z(buf, path_off).unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let full = if dir_path.is_empty() {
            PathBuf::from(&name)
        } else {
            PathBuf::from(dir_path).join(&name)
        };
        out.push(EverythingHit {
            is_dir: (flags & EVERYTHING_IPC_FOLDER_FLAG) != 0,
            name,
            path: full,
        });
    }
    out
}

unsafe extern "system" fn reply_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_COPYDATA && lparam.0 != 0 {
        let cds = &*(lparam.0 as *const COPYDATASTRUCT);
        if cds.dwData == EVERYTHING_IPC_REPLY_ID as usize
            && !cds.lpData.is_null()
            && cds.cbData >= 28
        {
            let slice =
                std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize);
            let parsed = parse_everything_list_w(slice);
            REPLY_BUFFER.with(|cell| {
                *cell.borrow_mut() = Some(parsed);
            });
            return LRESULT(1);
        }
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// Zero-DLL live query against voidtools Everything via `WM_COPYDATA`.
/// Returns `None` immediately if Everything is not running or busy (>120ms).
pub fn query_everything_ipc(query: &str, max_results: u32) -> Option<Vec<EverythingHit>> {
    let target_hwnd = find_everything_ipc_hwnd()?;
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Some(Vec::new());
    }

    let cls_w = to_wide_null("AnythingToPromptEverythingIpcReply");
    unsafe {
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSW {
            lpfnWndProc: Some(reply_wnd_proc),
            hInstance: hinst.into(),
            lpszClassName: PCWSTR(cls_w.as_ptr()),
            ..Default::default()
        };
        let _ = RegisterClassW(&wc);

        let reply_hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(cls_w.as_ptr()),
            PCWSTR::null(),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(hinst.into()),
            None,
        )
        .ok()?;
        if reply_hwnd.0.is_null() {
            return None;
        }

        // Build EVERYTHING_IPC_QUERYW payload:
        // reply_hwnd (u32), reply_copydata_message (u32), search_flags (u32), offset (u32), max_results (u32), query UTF-16Z
        let query_w = to_wide_null(trimmed);
        let mut payload = Vec::with_capacity(20 + query_w.len() * 2);
        payload.extend_from_slice(&(reply_hwnd.0 as usize as u32).to_le_bytes());
        payload.extend_from_slice(&EVERYTHING_IPC_REPLY_ID.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes()); // search_flags
        payload.extend_from_slice(&0u32.to_le_bytes()); // offset
        payload.extend_from_slice(&max_results.clamp(1, 32).to_le_bytes());
        for ch in query_w {
            payload.extend_from_slice(&ch.to_le_bytes());
        }

        let cds = COPYDATASTRUCT {
            dwData: EVERYTHING_IPC_COPYDATAQUERYW,
            cbData: payload.len() as u32,
            lpData: payload.as_mut_ptr() as *mut std::ffi::c_void,
        };

        REPLY_BUFFER.with(|cell| {
            *cell.borrow_mut() = None;
        });

        let mut _res = 0usize;
        let sent = SendMessageTimeoutW(
            target_hwnd,
            WM_COPYDATA,
            WPARAM(reply_hwnd.0 as usize),
            LPARAM(&cds as *const _ as isize),
            SMTO_ABORTIFHUNG,
            120,
            Some(&mut _res),
        );

        let _ = DestroyWindow(reply_hwnd);

        if sent.0 == 0 {
            return None;
        }

        REPLY_BUFFER.with(|cell| cell.borrow_mut().take())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_everything_ipc_list_w_binary_reply() {
        // Construct a valid synthetic EVERYTHING_IPC_LISTW with 1 file item
        let mut buf = Vec::new();
        // Header: 7 x u32 = 28 bytes (totfolders=0, totfiles=1, totitems=1, numfolders=0, numfiles=1, numitems=1, offset=0)
        for v in [0u32, 1, 1, 0, 1, 1, 0] {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        // Item 0 (12 bytes): flags=0, fn_off=40, path_off=60
        let fn_off = 40u32;
        let name_u16 = to_wide_null("支架.SLDPRT");
        let path_off = fn_off + (name_u16.len() * 2) as u32;
        let path_u16 = to_wide_null(r"D:\CAD");

        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&fn_off.to_le_bytes());
        buf.extend_from_slice(&path_off.to_le_bytes());
        for c in name_u16 {
            buf.extend_from_slice(&c.to_le_bytes());
        }
        for c in path_u16 {
            buf.extend_from_slice(&c.to_le_bytes());
        }

        let hits = parse_everything_list_w(&buf);
        assert_eq!(hits.len(), 1);
        assert!(!hits[0].is_dir);
        assert_eq!(hits[0].name, "支架.SLDPRT");
        assert_eq!(hits[0].path, PathBuf::from(r"D:\CAD\支架.SLDPRT"));
    }
}
