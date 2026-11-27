use std::collections::HashMap;
use std::path::PathBuf;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Ioctl::{
    FSCTL_ENUM_USN_DATA, FSCTL_QUERY_USN_JOURNAL, FSCTL_READ_USN_JOURNAL, MFT_ENUM_DATA_V0,
    READ_USN_JOURNAL_DATA_V0, USN_JOURNAL_DATA_V0, USN_REASON_FILE_CREATE, USN_REASON_FILE_DELETE,
    USN_REASON_RENAME_NEW_NAME, USN_REASON_RENAME_OLD_NAME, USN_RECORD_V2,
};
use windows::Win32::System::IO::DeviceIoControl;

const DRIVE_FIXED_TYPE: u32 = 3;

const SKIP_NTFS_SEGMENTS: &[&str] = &[
    "windows",
    "winsxs",
    "system volume information",
    "$recycle.bin",
    "programdata",
    "appdata",
    "node_modules",
    ".git",
    ".hg",
    ".svn",
    "target",
    "__pycache__",
    ".venv",
    "venv",
    ".cache",
    ".cargo",
    ".rustup",
    ".npm",
    ".pnpm-store",
    ".yarn",
    ".gradle",
    ".m2",
    ".nuget",
    "windowsapps",
    "temp",
    "tmp",
    "prefetch",
    "servicing",
    "installer",
    "assembly",
    "driverstore",
    "recovery",
    "perflogs",
    "msocache",
    "_backups",
    "_sandbox",
    ".bun",
    ".dsh",
    ".gemini",
    ".codex",
    ".vscode",
    "site-packages",
    "dist-packages",
    "session_projcache",
    "wheels",
];

const SKIP_NTFS_EXTS: &[&str] = &[
    "tmp", "bak", "log", "pdb", "obj", "o", "d", "rmeta", "rlib", "pyc", "pyo", "dll", "sys",
    "mui", "cat", "man", "mum", "nls", "etl", "evtx", "dat", "bin", "idx", "pack", "lock",
    "manifest", "winmd", "tlb", "ocx", "cpl", "drv", "scr", "efi", "inf", "pnf", "cab",
];

#[derive(Debug, Clone)]
pub struct NtfsItem {
    pub is_dir: bool,
    pub name: String,
    pub path: PathBuf,
    pub priority: i32,
}

pub struct NtfsVolumeState {
    pub drive_letter: char,
    pub journal_id: u64,
    pub next_usn: i64,
    pub dirs: HashMap<u64, (u64, String)>,
}

#[derive(Debug, Default)]
pub struct NtfsIncrementalDelta {
    pub added: Vec<NtfsItem>,
    pub removed_lower: Vec<String>,
}

use crate::win32_util::to_wide as to_wide_null;

pub fn list_fixed_ntfs_drives() -> Vec<char> {
    let mut drives = Vec::new();
    let mask = unsafe { GetLogicalDrives() };
    for bit in 0..26u32 {
        if (mask & (1 << bit)) == 0 {
            continue;
        }
        let letter = (b'A' + bit as u8) as char;
        let root = format!("{letter}:\\");
        let root_w = to_wide_null(&root);
        let drive_type = unsafe { GetDriveTypeW(PCWSTR(root_w.as_ptr())) };
        if drive_type != DRIVE_FIXED_TYPE {
            continue;
        }
        let mut fs_buf = [0u16; 32];
        let ok = unsafe {
            GetVolumeInformationW(
                PCWSTR(root_w.as_ptr()),
                None,
                None,
                None,
                None,
                Some(&mut fs_buf),
            )
            .is_ok()
        };
        if !ok {
            continue;
        }
        let end = fs_buf.iter().position(|&c| c == 0).unwrap_or(fs_buf.len());
        let fs_name = String::from_utf16_lossy(&fs_buf[..end]);
        if fs_name.eq_ignore_ascii_case("NTFS") {
            drives.push(letter);
        }
    }
    drives
}

fn open_volume_handle(letter: char) -> Option<HANDLE> {
    let vol_path = format!(r"\\.\{letter}:");
    let vol_w = to_wide_null(&vol_path);
    let handle = unsafe {
        CreateFileW(
            PCWSTR(vol_w.as_ptr()),
            GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    }
    .ok()?;
    if handle == INVALID_HANDLE_VALUE || handle.0.is_null() {
        return None;
    }
    Some(handle)
}

fn query_usn_journal(handle: HANDLE) -> Option<USN_JOURNAL_DATA_V0> {
    let mut data = USN_JOURNAL_DATA_V0::default();
    let mut returned = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            handle,
            FSCTL_QUERY_USN_JOURNAL,
            None,
            0,
            Some(&mut data as *mut _ as *mut std::ffi::c_void),
            std::mem::size_of::<USN_JOURNAL_DATA_V0>() as u32,
            Some(&mut returned),
            None,
        )
        .is_ok()
    };
    if ok {
        Some(data)
    } else {
        None
    }
}

pub fn should_skip_ntfs_segment(name: &str) -> bool {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.starts_with('$') {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    SKIP_NTFS_SEGMENTS.contains(&lower.as_str())
}

pub fn should_skip_ntfs_file(name: &str) -> bool {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.starts_with('$') || trimmed.starts_with("~$") {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    if matches!(lower.as_str(), "desktop.ini" | "thumbs.db" | ".ds_store") {
        return true;
    }
    if let Some((_, ext)) = lower.rsplit_once('.') {
        if SKIP_NTFS_EXTS.contains(&ext) {
            return true;
        }
    }
    false
}

pub fn compute_ntfs_path_priority(path_str: &str) -> i32 {
    let lower = path_str.to_ascii_lowercase();
    if lower.contains(r"\program files") {
        -25
    } else if lower.starts_with(r"c:\users\") {
        55
    } else if !lower.starts_with(r"c:\") {
        45
    } else {
        15
    }
}

/// Resolves a directory `FileReferenceNumber` to `Some(path)` or `None` if it lies inside a skipped subtree.
fn resolve_dir_path(
    drive_letter: char,
    dirs: &HashMap<u64, (u64, String)>,
    cache: &mut HashMap<u64, Option<String>>,
    dir_ref: u64,
) -> Option<String> {
    if let Some(cached) = cache.get(&dir_ref) {
        return cached.clone();
    }

    let mut chain: Vec<(u64, String)> = Vec::with_capacity(16);
    let mut cur = dir_ref;
    let mut depth = 0usize;

    loop {
        if let Some(cached) = cache.get(&cur) {
            let mut result = cached.clone();
            for (node_ref, node_name) in chain.into_iter().rev() {
                result = match result {
                    Some(base) => {
                        if should_skip_ntfs_segment(&node_name) {
                            None
                        } else {
                            Some(format!("{base}\\{node_name}"))
                        }
                    }
                    None => None,
                };
                cache.insert(node_ref, result.clone());
            }
            return result;
        }

        depth += 1;
        if depth > 48 {
            cache.insert(dir_ref, None);
            return None;
        }

        let Some((parent_ref, name)) = dirs.get(&cur) else {
            // Reached volume root (root MFT ref is not in `dirs` or self-parented)
            let mut cur_path = Some(format!("{drive_letter}:"));
            for (node_ref, node_name) in chain.into_iter().rev() {
                cur_path = match cur_path {
                    Some(base) => {
                        if should_skip_ntfs_segment(&node_name) {
                            None
                        } else {
                            Some(format!("{base}\\{node_name}"))
                        }
                    }
                    None => None,
                };
                cache.insert(node_ref, cur_path.clone());
            }
            cache.entry(cur).or_insert_with(|| Some(format!("{drive_letter}:")));
            return cur_path;
        };

        if *parent_ref == cur {
            let mut cur_path = Some(format!("{drive_letter}:"));
            for (node_ref, node_name) in chain.into_iter().rev() {
                cur_path = match cur_path {
                    Some(base) => {
                        if should_skip_ntfs_segment(&node_name) {
                            None
                        } else {
                            Some(format!("{base}\\{node_name}"))
                        }
                    }
                    None => None,
                };
                cache.insert(node_ref, cur_path.clone());
            }
            cache.insert(cur, Some(format!("{drive_letter}:")));
            return cur_path;
        }

        chain.push((cur, name.clone()));
        cur = *parent_ref;
    }
}

fn parse_usn_record_v2(buf: &[u8], offset: usize) -> Option<(usize, u64, u64, u32, u32, String)> {
    let header_size = std::mem::size_of::<USN_RECORD_V2>();
    if offset + 8 > buf.len() {
        return None;
    }
    let rec_len = u32::from_le_bytes(buf[offset..offset + 4].try_into().ok()?) as usize;
    if rec_len < 60 || offset + rec_len > buf.len() {
        return None;
    }
    let major = u16::from_le_bytes(buf[offset + 4..offset + 6].try_into().ok()?);
    if major != 2 || offset + header_size > buf.len() {
        return Some((rec_len, 0, 0, 0, 0, String::new()));
    }

    let rec = unsafe { std::ptr::read_unaligned(buf.as_ptr().add(offset) as *const USN_RECORD_V2) };
    let fn_offset = rec.FileNameOffset as usize;
    let fn_len_bytes = rec.FileNameLength as usize;
    if fn_offset + fn_len_bytes > rec_len || fn_len_bytes < 2 || !fn_len_bytes.is_multiple_of(2) {
        return Some((rec_len, 0, 0, 0, 0, String::new()));
    }

    let name_bytes = &buf[offset + fn_offset..offset + fn_offset + fn_len_bytes];
    let u16_iter = name_bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&c| u16::from_le_bytes(c));
    let name: String = char::decode_utf16(u16_iter)
        .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect();

    Some((
        rec_len,
        rec.FileReferenceNumber,
        rec.ParentFileReferenceNumber,
        rec.FileAttributes,
        rec.Reason,
        name,
    ))
}

/// Enumerates all clean user/project files & folders on a single NTFS volume using `FSCTL_ENUM_USN_DATA`.
/// Returns `None` if the process lacks volume read permission (e.g., non-elevated).
pub fn scan_ntfs_volume(
    drive_letter: char,
    max_items: usize,
) -> Option<(Vec<NtfsItem>, NtfsVolumeState)> {
    let handle = match open_volume_handle(drive_letter) {
        Some(h) => h,
        None => {
            log::warn!(
                "[SEARCH] NTFS open_volume_handle({}:) failed: {:?}",
                drive_letter,
                windows::core::Error::from_thread()
            );
            return None;
        }
    };
    let journal = match query_usn_journal(handle) {
        Some(j) => j,
        None => {
            log::warn!(
                "[SEARCH] NTFS query_usn_journal({}:) failed: {:?}",
                drive_letter,
                windows::core::Error::from_thread()
            );
            unsafe {
                let _ = CloseHandle(handle);
            }
            return None;
        }
    };

    let mut dirs: HashMap<u64, (u64, String)> = HashMap::with_capacity(32_768);
    let mut raw_files: Vec<(u64, String)> = Vec::with_capacity(65_536);

    let mut med = MFT_ENUM_DATA_V0 {
        StartFileReferenceNumber: 0,
        LowUsn: 0,
        HighUsn: journal.NextUsn,
    };
    // 1 MiB aligned IOCTL buffer (16x fewer kernel transitions on multi-million MFT volumes)
    let mut buf_u64 = vec![0u64; 131_072];

    loop {
        let mut returned = 0u32;
        let ok = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_ENUM_USN_DATA,
                Some(&med as *const _ as *const std::ffi::c_void),
                std::mem::size_of::<MFT_ENUM_DATA_V0>() as u32,
                Some(buf_u64.as_mut_ptr() as *mut std::ffi::c_void),
                (buf_u64.len() * std::mem::size_of::<u64>()) as u32,
                Some(&mut returned),
                None,
            )
            .is_ok()
        };
        if !ok || (returned as usize) <= 8 {
            break;
        }

        let bytes = unsafe {
            std::slice::from_raw_parts(buf_u64.as_ptr() as *const u8, returned as usize)
        };
        let next_ref = u64::from_le_bytes(bytes[0..8].try_into().unwrap_or([0; 8]));
        let mut offset = 8usize;

        while offset < bytes.len() {
            let Some((rec_len, fref, pref, attrs, _reason, name)) = parse_usn_record_v2(bytes, offset)
            else {
                break;
            };
            offset += rec_len;
            if fref == 0 || name.is_empty() {
                continue;
            }
            let is_dir = (attrs & FILE_ATTRIBUTE_DIRECTORY.0) != 0;
            if is_dir {
                dirs.insert(fref, (pref, name));
            } else if !should_skip_ntfs_file(&name) {
                raw_files.push((pref, name));
            }
        }

        med.StartFileReferenceNumber = next_ref;
    }

    unsafe {
        let _ = CloseHandle(handle);
    }

    let mut dir_cache: HashMap<u64, Option<String>> = HashMap::with_capacity(dirs.len());
    let mut items = Vec::with_capacity(raw_files.len().min(max_items));

    // Emit clean directories first
    for (&dir_ref, (_pref, name)) in &dirs {
        if items.len() >= max_items {
            break;
        }
        if should_skip_ntfs_segment(name) {
            continue;
        }
        if let Some(full) = resolve_dir_path(drive_letter, &dirs, &mut dir_cache, dir_ref) {
            if full.len() > 3 {
                let prio = compute_ntfs_path_priority(&full);
                items.push(NtfsItem {
                    is_dir: true,
                    name: name.clone(),
                    path: PathBuf::from(full),
                    priority: prio,
                });
            }
        }
    }

    // Emit clean files
    for (pref, name) in raw_files {
        if items.len() >= max_items {
            break;
        }
        if let Some(parent_path) = resolve_dir_path(drive_letter, &dirs, &mut dir_cache, pref) {
            let full = format!("{parent_path}\\{name}");
            let prio = compute_ntfs_path_priority(&full);
            items.push(NtfsItem {
                is_dir: false,
                name,
                path: PathBuf::from(full),
                priority: prio,
            });
        }
    }

    // Prune system/node_modules/cache directories from retained USN watcher map to keep RAM tiny
    dirs.retain(|k, _| matches!(dir_cache.get(k), Some(Some(_))));
    dirs.shrink_to_fit();

    let state = NtfsVolumeState {
        drive_letter,
        journal_id: journal.UsnJournalID,
        next_usn: journal.NextUsn,
        dirs,
    };
    Some((items, state))
}

/// Polls incremental USN journal changes (creates, deletes, renames) since `state.next_usn`.
pub fn poll_ntfs_volume_changes(state: &mut NtfsVolumeState) -> NtfsIncrementalDelta {
    let mut delta = NtfsIncrementalDelta::default();
    let Some(handle) = open_volume_handle(state.drive_letter) else {
        return delta;
    };

    let reason_mask = USN_REASON_FILE_CREATE
        | USN_REASON_FILE_DELETE
        | USN_REASON_RENAME_NEW_NAME
        | USN_REASON_RENAME_OLD_NAME;

    let mut read_data = READ_USN_JOURNAL_DATA_V0 {
        StartUsn: state.next_usn,
        ReasonMask: reason_mask,
        ReturnOnlyOnClose: 0,
        Timeout: 0,
        BytesToWaitFor: 0,
        UsnJournalID: state.journal_id,
    };

    let mut buf_u64 = vec![0u64; 4096]; // 32 KiB
    let mut dir_cache: HashMap<u64, Option<String>> = HashMap::new();

    for _ in 0..16 {
        let mut returned = 0u32;
        let ok = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_READ_USN_JOURNAL,
                Some(&read_data as *const _ as *const std::ffi::c_void),
                std::mem::size_of::<READ_USN_JOURNAL_DATA_V0>() as u32,
                Some(buf_u64.as_mut_ptr() as *mut std::ffi::c_void),
                (buf_u64.len() * std::mem::size_of::<u64>()) as u32,
                Some(&mut returned),
                None,
            )
            .is_ok()
        };
        if !ok || (returned as usize) <= 8 {
            break;
        }

        let bytes = unsafe {
            std::slice::from_raw_parts(buf_u64.as_ptr() as *const u8, returned as usize)
        };
        let next_usn = i64::from_le_bytes(bytes[0..8].try_into().unwrap_or([0; 8]));
        let mut offset = 8usize;

        while offset < bytes.len() {
            let Some((rec_len, fref, pref, attrs, reason, name)) =
                parse_usn_record_v2(bytes, offset)
            else {
                break;
            };
            offset += rec_len;
            if fref == 0 || name.is_empty() {
                continue;
            }
            let is_dir = (attrs & FILE_ATTRIBUTE_DIRECTORY.0) != 0;

            if (reason & (USN_REASON_FILE_DELETE | USN_REASON_RENAME_OLD_NAME)) != 0 {
                if let Some(parent_path) =
                    resolve_dir_path(state.drive_letter, &state.dirs, &mut dir_cache, pref)
                {
                    delta
                        .removed_lower
                        .push(format!("{parent_path}\\{name}").to_ascii_lowercase());
                }
                if is_dir {
                    state.dirs.remove(&fref);
                    dir_cache.clear();
                }
            } else if (reason & (USN_REASON_FILE_CREATE | USN_REASON_RENAME_NEW_NAME)) != 0 {
                if is_dir {
                    state.dirs.insert(fref, (pref, name.clone()));
                    dir_cache.clear();
                    if !should_skip_ntfs_segment(&name) {
                        if let Some(full) =
                            resolve_dir_path(state.drive_letter, &state.dirs, &mut dir_cache, fref)
                        {
                            let prio = compute_ntfs_path_priority(&full);
                            delta.added.push(NtfsItem {
                                is_dir: true,
                                name,
                                path: PathBuf::from(full),
                                priority: prio,
                            });
                        }
                    }
                } else if !should_skip_ntfs_file(&name) {
                    if let Some(parent_path) =
                        resolve_dir_path(state.drive_letter, &state.dirs, &mut dir_cache, pref)
                    {
                        let full = format!("{parent_path}\\{name}");
                        let prio = compute_ntfs_path_priority(&full);
                        delta.added.push(NtfsItem {
                            is_dir: false,
                            name,
                            path: PathBuf::from(full),
                            priority: prio,
                        });
                    }
                }
            }
        }

        if next_usn == read_data.StartUsn {
            break;
        }
        read_data.StartUsn = next_usn;
        state.next_usn = next_usn;
    }

    unsafe {
        let _ = CloseHandle(handle);
    }
    delta
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_windows_system_noise_and_prioritizes_user_and_project_drives() {
        assert!(should_skip_ntfs_segment("Windows"));
        assert!(should_skip_ntfs_segment("node_modules"));
        assert!(should_skip_ntfs_segment("$Recycle.Bin"));
        assert!(!should_skip_ntfs_segment("DSH-Workspace"));

        assert!(should_skip_ntfs_file("kernel32.dll"));
        assert!(should_skip_ntfs_file("~$report.docx"));
        assert!(!should_skip_ntfs_file("支架.SLDPRT"));
        assert!(!should_skip_ntfs_file("报销发票.xlsx"));

        assert!(compute_ntfs_path_priority(r"C:\Users\Zegel\Desktop\a.pdf") > 0);
        assert!(compute_ntfs_path_priority(r"D:\Agent-Studio\foo.rs") > 0);
        assert!(compute_ntfs_path_priority(r"C:\Program Files\App\a.exe") < 0);
    }

    #[test]
    fn resolves_nested_dir_paths_and_prunes_skipped_ancestors() {
        let mut dirs = HashMap::new();
        dirs.insert(10, (5, "Agent-Studio".to_string()));
        dirs.insert(11, (10, "Solidworks".to_string()));
        dirs.insert(12, (11, "node_modules".to_string()));
        dirs.insert(13, (12, "pkg".to_string()));

        let mut cache = HashMap::new();
        assert_eq!(
            resolve_dir_path('D', &dirs, &mut cache, 11).as_deref(),
            Some(r"D:\Agent-Studio\Solidworks")
        );
        assert_eq!(resolve_dir_path('D', &dirs, &mut cache, 13), None);
    }

    #[test]
    #[ignore]
    fn live_ntfs_drive_enumeration_diagnostics() {
        let drives = list_fixed_ntfs_drives();
        eprintln!("[NTFS-LIVE] fixed NTFS drives: {:?}", drives);
        assert!(!drives.is_empty());
        for d in drives {
            let t0 = std::time::Instant::now();
            match scan_ntfs_volume(d, 100_000) {
                Some((items, st)) => {
                    eprintln!(
                        "[NTFS-LIVE] {}: scanned {} clean items (dirs={}) in {}ms",
                        d,
                        items.len(),
                        st.dirs.len(),
                        t0.elapsed().as_millis()
                    );
                }
                None => {
                    eprintln!("[NTFS-LIVE] {}: skipped (non-elevated or no journal)", d);
                }
            }
        }
    }
}
