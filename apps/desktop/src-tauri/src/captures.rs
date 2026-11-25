use crate::config::AppConfig;
use crate::AppShared;
use anyhow::{anyhow, Context, Result};
use base64::Engine;
use image::{ImageFormat, Rgba, RgbaImage};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    GetMonitorInfoW, MonitorFromPoint, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, HDC, MONITORINFO, MONITOR_DEFAULTTONEAREST, SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowDisplayAffinity, GetWindowRect, GetWindowThreadProcessId, SetWindowDisplayAffinity,
    WDA_EXCLUDEFROMCAPTURE, WDA_NONE, WINDOW_DISPLAY_AFFINITY,
};

#[link(name = "user32")]
extern "system" {
    fn PrintWindow(hwnd: HWND, hdc_blt: HDC, n_flags: u32) -> i32;
}

const PW_RENDERFULLCONTENT: u32 = 0x0000_0002;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ScreenRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[derive(Debug, Clone)]
pub struct RawScreenCapture {
    pub origin_x: i32,
    pub origin_y: i32,
    pub image: RgbaImage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureRecord {
    pub path: String,
    pub created_at_secs: u64,
    pub attached_at_msg_seq: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CaptureIndex {
    pub total_sent_messages: u64,
    pub items: Vec<CaptureRecord>,
}

impl CaptureIndex {
    pub fn index_path(dir: &Path) -> PathBuf {
        dir.join("index.json")
    }

    pub fn load(dir: &Path) -> Self {
        let p = Self::index_path(dir);
        if let Ok(raw) = fs::read_to_string(p) {
            if let Ok(idx) = serde_json::from_str::<Self>(&raw) {
                return idx;
            }
        }
        Self::default()
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        fs::create_dir_all(dir)?;
        let raw = serde_json::to_string_pretty(self)?;
        fs::write(Self::index_path(dir), raw)?;
        Ok(())
    }

    pub fn record_new_capture(
        &mut self,
        path: String,
        now_secs: u64,
        keep_messages: usize,
        orphan_ttl_minutes: u64,
    ) -> Vec<String> {
        self.items.push(CaptureRecord {
            path,
            created_at_secs: now_secs,
            attached_at_msg_seq: None,
        });
        self.purge_expired(now_secs, keep_messages, orphan_ttl_minutes)
    }

    pub fn on_message_sent(
        &mut self,
        now_secs: u64,
        keep_messages: usize,
        orphan_ttl_minutes: u64,
    ) -> Vec<String> {
        self.total_sent_messages = self.total_sent_messages.saturating_add(1);
        let current_seq = self.total_sent_messages;
        for item in &mut self.items {
            if item.attached_at_msg_seq.is_none() {
                item.attached_at_msg_seq = Some(current_seq);
            }
        }
        self.purge_expired(now_secs, keep_messages, orphan_ttl_minutes)
    }

    pub fn purge_expired(
        &mut self,
        now_secs: u64,
        keep_messages: usize,
        orphan_ttl_minutes: u64,
    ) -> Vec<String> {
        let keep = (keep_messages.max(1)) as u64;
        let orphan_ttl_secs = orphan_ttl_minutes.max(1).saturating_mul(60);
        let current_seq = self.total_sent_messages;

        let mut removed = Vec::new();
        self.items.retain(|item| {
            let expired = match item.attached_at_msg_seq {
                Some(seq) => current_seq.saturating_sub(seq) >= keep,
                None => now_secs.saturating_sub(item.created_at_secs) >= orphan_ttl_secs,
            };
            if expired {
                removed.push(item.path.clone());
                false
            } else {
                true
            }
        });
        removed
    }
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn now_unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

pub fn register_saved_capture(path: &Path, cfg: &AppConfig) {
    let dir = AppConfig::captures_dir();
    let mut idx = CaptureIndex::load(&dir);
    let evicted = idx.record_new_capture(
        path.to_string_lossy().to_string(),
        now_unix_secs(),
        cfg.capture_keep_messages,
        cfg.capture_orphan_ttl_minutes,
    );
    for old in evicted {
        let _ = fs::remove_file(old);
    }
    let _ = idx.save(&dir);
}

pub fn rotate_captures_on_send(cfg: &AppConfig) {
    let dir = AppConfig::captures_dir();
    let mut idx = CaptureIndex::load(&dir);
    let evicted = idx.on_message_sent(
        now_unix_secs(),
        cfg.capture_keep_messages,
        cfg.capture_orphan_ttl_minutes,
    );
    for old in evicted {
        let _ = fs::remove_file(old);
    }
    let _ = idx.save(&dir);
}

pub fn capture_target_window_bitblt(
    target_hwnd: HWND,
    cursor_x: i32,
    cursor_y: i32,
) -> Result<RawScreenCapture> {
    // 1. Prefer DWM PrintWindow(PW_RENDERFULLCONTENT) on target_hwnd: works even when occluded or in restricted sessions, and never captures our own overlays
    if !target_hwnd.0.is_null() {
        if let Ok(cap) = capture_hwnd_via_print_window(target_hwnd) {
            return Ok(cap);
        }
    }

    unsafe {
        let pt = windows::Win32::Foundation::POINT {
            x: cursor_x,
            y: cursor_y,
        };
        let hmon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let mon_rc = if GetMonitorInfoW(hmon, &mut mi).as_bool() {
            mi.rcMonitor
        } else {
            RECT {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
            }
        };

        let mut win_rc = RECT::default();
        let got_dwm = !target_hwnd.0.is_null()
            && DwmGetWindowAttribute(
                target_hwnd,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                &mut win_rc as *mut _ as *mut _,
                std::mem::size_of::<RECT>() as u32,
            )
            .is_ok();

        if !got_dwm && (target_hwnd.0.is_null() || GetWindowRect(target_hwnd, &mut win_rc).is_err())
        {
            win_rc = mon_rc;
        }

        let left = win_rc.left.max(mon_rc.left);
        let top = win_rc.top.max(mon_rc.top);
        let right = win_rc.right.min(mon_rc.right);
        let bottom = win_rc.bottom.min(mon_rc.bottom);

        let (cap_l, cap_t, cap_w, cap_h) = if right - left >= 32 && bottom - top >= 32 {
            (left, top, right - left, bottom - top)
        } else {
            (
                mon_rc.left,
                mon_rc.top,
                (mon_rc.right - mon_rc.left).max(64),
                (mon_rc.bottom - mon_rc.top).max(64),
            )
        };

        capture_screen_rect_rgba(cap_l, cap_t, cap_w, cap_h)
    }
}

pub fn capture_hwnd_via_print_window(target_hwnd: HWND) -> Result<RawScreenCapture> {
    unsafe {
        let mut win_rc = RECT::default();
        GetWindowRect(target_hwnd, &mut win_rc).context("GetWindowRect failed")?;
        let width = (win_rc.right - win_rc.left).clamp(1, 7680);
        let height = (win_rc.bottom - win_rc.top).clamp(1, 4320);

        let screen_dc = GetDC(Some(target_hwnd));
        let fallback_dc = if screen_dc.0.is_null() {
            GetDC(None)
        } else {
            screen_dc
        };
        if fallback_dc.0.is_null() {
            return Err(anyhow!("GetDC failed for PrintWindow"));
        }
        let dc_owner = if screen_dc.0.is_null() {
            None
        } else {
            Some(target_hwnd)
        };

        let mem_dc = CreateCompatibleDC(Some(fallback_dc));
        if mem_dc.0.is_null() {
            let _ = ReleaseDC(dc_owner, fallback_dc);
            return Err(anyhow!("CreateCompatibleDC failed"));
        }

        let bmp = CreateCompatibleBitmap(fallback_dc, width, height);
        if bmp.0.is_null() {
            let _ = DeleteDC(mem_dc);
            let _ = ReleaseDC(dc_owner, fallback_dc);
            return Err(anyhow!("CreateCompatibleBitmap failed"));
        }

        // If capturing Anything-to-Prompt's own window while WDA_EXCLUDEFROMCAPTURE is active,
        // briefly relax display affinity for the 5ms PrintWindow call so the capture is crisp (never black).
        let mut restore_affinity: Option<WINDOW_DISPLAY_AFFINITY> = None;
        let mut target_pid = 0u32;
        GetWindowThreadProcessId(target_hwnd, Some(&mut target_pid));
        if target_pid == std::process::id() {
            let mut cur_aff: u32 = 0;
            if GetWindowDisplayAffinity(target_hwnd, &mut cur_aff).is_ok()
                && cur_aff == WDA_EXCLUDEFROMCAPTURE.0
            {
                let _ = SetWindowDisplayAffinity(target_hwnd, WDA_NONE);
                restore_affinity = Some(WDA_EXCLUDEFROMCAPTURE);
            }
        }

        let old_obj = SelectObject(mem_dc, bmp.into());
        let mut pw_ok = PrintWindow(target_hwnd, mem_dc, PW_RENDERFULLCONTENT) != 0;
        if !pw_ok {
            pw_ok = PrintWindow(target_hwnd, mem_dc, 0) != 0;
        }
        let _ = SelectObject(mem_dc, old_obj);
        if let Some(aff) = restore_affinity {
            let _ = SetWindowDisplayAffinity(target_hwnd, aff);
        }

        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: (width as u32) * (height as u32) * 4,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut bgra = vec![0u8; (width as usize) * (height as usize) * 4];
        let lines = if pw_ok {
            GetDIBits(
                fallback_dc,
                bmp,
                0,
                height as u32,
                Some(bgra.as_mut_ptr() as *mut _),
                &mut bmi,
                DIB_RGB_COLORS,
            )
        } else {
            0
        };

        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem_dc);
        let _ = ReleaseDC(dc_owner, fallback_dc);

        if !pw_ok || lines == 0 {
            return Err(anyhow!("PrintWindow failed (pw_ok={pw_ok}, lines={lines})"));
        }

        for px in bgra.as_chunks_mut::<4>().0 {
            px.swap(0, 2);
            px[3] = 255;
        }

        let img = RgbaImage::from_raw(width as u32, height as u32, bgra)
            .ok_or_else(|| anyhow!("failed to build RgbaImage"))?;

        Ok(RawScreenCapture {
            origin_x: win_rc.left,
            origin_y: win_rc.top,
            image: img,
        })
    }
}

pub fn capture_screen_rect_rgba(x: i32, y: i32, w: i32, h: i32) -> Result<RawScreenCapture> {
    let width = w.clamp(1, 7680);
    let height = h.clamp(1, 4320);

    unsafe {
        let screen_dc = GetDC(None);
        if screen_dc.0.is_null() {
            return Err(anyhow!("GetDC(None) returned null"));
        }

        let mem_dc = CreateCompatibleDC(Some(screen_dc));
        if mem_dc.0.is_null() {
            let _ = ReleaseDC(None, screen_dc);
            return Err(anyhow!("CreateCompatibleDC failed"));
        }

        let bmp = CreateCompatibleBitmap(screen_dc, width, height);
        if bmp.0.is_null() {
            let _ = DeleteDC(mem_dc);
            let _ = ReleaseDC(None, screen_dc);
            return Err(anyhow!("CreateCompatibleBitmap failed"));
        }

        let old_obj = SelectObject(mem_dc, bmp.into());
        let blt_ok = BitBlt(mem_dc, 0, 0, width, height, Some(screen_dc), x, y, SRCCOPY).is_ok();
        // Deselect bitmap from mem_dc BEFORE calling GetDIBits (required by Win32 GDI contract)
        let _ = SelectObject(mem_dc, old_obj);

        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height, // top-down DIB
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: (width as u32) * (height as u32) * 4,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut bgra = vec![0u8; (width as usize) * (height as usize) * 4];
        let lines = if blt_ok {
            GetDIBits(
                screen_dc,
                bmp,
                0,
                height as u32,
                Some(bgra.as_mut_ptr() as *mut _),
                &mut bmi,
                DIB_RGB_COLORS,
            )
        } else {
            0
        };

        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem_dc);
        let _ = ReleaseDC(None, screen_dc);

        if !blt_ok || lines == 0 {
            log::warn!(
                "capture_screen_rect_rgba failed: blt_ok={blt_ok} lines={lines} rect=({x},{y},{width}x{height})"
            );
            return Err(anyhow!("BitBlt / GetDIBits failed"));
        }

        // Convert BGRA -> RGBA
        for px in bgra.as_chunks_mut::<4>().0 {
            px.swap(0, 2);
            px[3] = 255;
        }

        let img = RgbaImage::from_raw(width as u32, height as u32, bgra)
            .ok_or_else(|| anyhow!("failed to construct RgbaImage"))?;

        Ok(RawScreenCapture {
            origin_x: x,
            origin_y: y,
            image: img,
        })
    }
}

const DIGIT_5X7: [[u8; 7]; 10] = [
    [
        0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
    ], // 0
    [
        0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
    ], // 1
    [
        0b01110, 0b10001, 0b00001, 0b00110, 0b01000, 0b10000, 0b11111,
    ], // 2
    [
        0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
    ], // 3
    [
        0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
    ], // 4
    [
        0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110,
    ], // 5
    [
        0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
    ], // 6
    [
        0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
    ], // 7
    [
        0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
    ], // 8
    [
        0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100,
    ], // 9
];

fn put_thick_pixel(img: &mut RgbaImage, x: i32, y: i32, radius: i32, color: Rgba<u8>) {
    let w = img.width() as i32;
    let h = img.height() as i32;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            if dx * dx + dy * dy <= radius * radius + 1 {
                let px = x + dx;
                let py = y + dy;
                if px >= 0 && px < w && py >= 0 && py < h {
                    img.put_pixel(px as u32, py as u32, color);
                }
            }
        }
    }
}

fn draw_line_thick(
    img: &mut RgbaImage,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    radius: i32,
    color: Rgba<u8>,
) {
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let mut cx = x0;
    let mut cy = y0;
    loop {
        put_thick_pixel(img, cx, cy, radius, color);
        if cx == x1 && cy == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            cx += sx;
        }
        if e2 <= dx {
            err += dx;
            cy += sy;
        }
    }
}

fn draw_badge_in_rust(img: &mut RgbaImage, cx: i32, cy: i32, number: usize) {
    let red = Rgba([239, 68, 68, 255]);
    let white = Rgba([255, 255, 255, 255]);
    put_thick_pixel(img, cx, cy, 13, red);

    let digits: Vec<usize> = number
        .to_string()
        .chars()
        .filter_map(|c| c.to_digit(10).map(|d| d as usize))
        .collect();
    if digits.is_empty() {
        return;
    }

    let scale = if digits.len() == 1 { 2 } else { 1 };
    let glyph_w = 5 * scale;
    let glyph_h = 7 * scale;
    let gap = 2;
    let total_w = (digits.len() as i32) * glyph_w + ((digits.len() as i32 - 1).max(0)) * gap;
    let start_x = cx - total_w / 2;
    let start_y = cy - glyph_h / 2;

    for (idx, &d) in digits.iter().enumerate() {
        let gx = start_x + (idx as i32) * (glyph_w + gap);
        let bitmap = DIGIT_5X7[d.min(9)];
        for (row_idx, &row_bits) in bitmap.iter().enumerate() {
            for col_idx in 0..5 {
                if (row_bits >> (4 - col_idx)) & 1 == 1 {
                    for sy in 0..scale {
                        for sx in 0..scale {
                            let px = gx + col_idx * scale + sx;
                            let py = start_y + (row_idx as i32) * scale + sy;
                            if px >= 0
                                && px < img.width() as i32
                                && py >= 0
                                && py < img.height() as i32
                            {
                                img.put_pixel(px as u32, py as u32, white);
                            }
                        }
                    }
                }
            }
        }
    }
}

pub fn annotate_rgba_in_rust(
    raw: &RawScreenCapture,
    rect: Option<ScreenRect>,
    strokes: &[(i32, i32)],
    number: usize,
) -> RgbaImage {
    let mut img = raw.image.clone();
    let red = Rgba([239, 68, 68, 255]);
    let ox = raw.origin_x;
    let oy = raw.origin_y;
    let mut badge_x = 24;
    let mut badge_y = 24;

    if strokes.len() > 1 {
        for pair in strokes.windows(2) {
            let (x0, y0) = (pair[0].0 - ox, pair[0].1 - oy);
            let (x1, y1) = (pair[1].0 - ox, pair[1].1 - oy);
            draw_line_thick(&mut img, x0, y0, x1, y1, 2, red);
        }
        badge_x = (strokes[0].0 - ox + 14).clamp(16, (img.width() as i32 - 16).max(16));
        badge_y = (strokes[0].1 - oy - 14).clamp(16, (img.height() as i32 - 16).max(16));
    } else if let Some(r) = rect {
        let x0 = r.x - ox;
        let y0 = r.y - oy;
        let x1 = x0 + r.w.max(2);
        let y1 = y0 + r.h.max(2);
        draw_line_thick(&mut img, x0, y0, x1, y0, 2, red);
        draw_line_thick(&mut img, x1, y0, x1, y1, 2, red);
        draw_line_thick(&mut img, x1, y1, x0, y1, 2, red);
        draw_line_thick(&mut img, x0, y1, x0, y0, 2, red);
        badge_x = (x1 + 10).clamp(16, (img.width() as i32 - 16).max(16));
        badge_y = (y0 - 8).clamp(16, (img.height() as i32 - 16).max(16));
    }

    draw_badge_in_rust(&mut img, badge_x, badge_y, number);
    img
}

pub fn save_annotated_capture(
    app: &AppHandle,
    raw: &RawScreenCapture,
    rect: Option<ScreenRect>,
    strokes: &[(i32, i32)],
    number: usize,
    cfg: &AppConfig,
) -> Result<PathBuf> {
    let captures_dir = AppConfig::captures_dir();
    fs::create_dir_all(&captures_dir).context("failed to create captures dir")?;
    let filename = format!("pick-{}-{}.png", now_unix_millis(), number);
    let out_path = captures_dir.join(filename);

    // Attempt crisp HTML5 canvas composition via overlay webview first (400ms budget)
    let mut composed_bytes: Option<Vec<u8>> = None;
    if let (Some(ov), Some(shared)) = (
        app.get_webview_window("overlay"),
        app.try_state::<AppShared>(),
    ) {
        let mut png_buf = Vec::new();
        if raw
            .image
            .write_to(&mut Cursor::new(&mut png_buf), ImageFormat::Png)
            .is_ok()
        {
            let b64 = base64::engine::general_purpose::STANDARD.encode(&png_buf);
            let req_id = format!("cmp-{}-{}", now_unix_millis(), number);
            let (tx, rx) = mpsc::channel::<String>();
            if let Ok(mut map) = shared.compose_waiters.lock() {
                map.insert(req_id.clone(), tx);
            }

            let stroke_json: Vec<serde_json::Value> = strokes
                .iter()
                .map(|&(x, y)| serde_json::json!({ "x": x, "y": y }))
                .collect();

            let _ = ov.emit(
                "pen:compose",
                serde_json::json!({
                    "id": req_id,
                    "pngBase64": b64,
                    "originX": raw.origin_x,
                    "originY": raw.origin_y,
                    "rect": rect,
                    "strokes": stroke_json,
                    "n": number,
                }),
            );

            if let Ok(data_url) = rx.recv_timeout(Duration::from_millis(400)) {
                if let Some(stripped) = data_url.strip_prefix("data:image/png;base64,") {
                    if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(stripped)
                    {
                        if !decoded.is_empty() {
                            composed_bytes = Some(decoded);
                        }
                    }
                }
            } else if let Ok(mut map) = shared.compose_waiters.lock() {
                map.remove(&req_id);
            }
        }
    }

    if let Some(bytes) = composed_bytes {
        fs::write(&out_path, bytes).context("failed to write overlay-composed PNG")?;
    } else {
        let fallback_img = annotate_rgba_in_rust(raw, rect, strokes, number);
        fallback_img
            .save_with_format(&out_path, ImageFormat::Png)
            .context("failed to save rust-annotated PNG")?;
    }

    register_saved_capture(&out_path, cfg);
    Ok(out_path)
}

/// Saves BOTH the annotated red-doodle PNG and the unoccluded clean reference PNG in a single
/// PNG-encode pass, writing the clean PNG while the overlay canvas composites the red stroke in parallel.
pub fn save_red_dual_captures(
    app: &AppHandle,
    raw: &RawScreenCapture,
    rect: Option<ScreenRect>,
    strokes: &[(i32, i32)],
    number: usize,
    cfg: &AppConfig,
) -> (Option<PathBuf>, Option<PathBuf>) {
    let captures_dir = AppConfig::captures_dir();
    if fs::create_dir_all(&captures_dir).is_err() {
        return (None, None);
    }
    let ts = now_unix_millis();
    let annot_path = captures_dir.join(format!("pick-{ts}-{number}.png"));
    let clean_path = captures_dir.join(format!("pick-{ts}-{number}-clean.png"));

    let mut png_buf = Vec::new();
    let encoded_ok = raw
        .image
        .write_to(&mut Cursor::new(&mut png_buf), ImageFormat::Png)
        .is_ok();

    // Kick off overlay canvas composition first so it overlaps disk write of the clean PNG!
    let mut compose_rx: Option<(String, mpsc::Receiver<String>)> = None;
    if encoded_ok {
        if let (Some(ov), Some(shared)) = (
            app.get_webview_window("overlay"),
            app.try_state::<AppShared>(),
        ) {
            let b64 = base64::engine::general_purpose::STANDARD.encode(&png_buf);
            let req_id = format!("cmp-{ts}-{number}");
            let (tx, rx) = mpsc::channel::<String>();
            if let Ok(mut map) = shared.compose_waiters.lock() {
                map.insert(req_id.clone(), tx);
            }
            let stroke_json: Vec<serde_json::Value> = strokes
                .iter()
                .map(|&(x, y)| serde_json::json!({ "x": x, "y": y }))
                .collect();
            let _ = ov.emit(
                "pen:compose",
                serde_json::json!({
                    "id": req_id,
                    "pngBase64": b64,
                    "originX": raw.origin_x,
                    "originY": raw.origin_y,
                    "rect": rect,
                    "strokes": stroke_json,
                    "n": number,
                }),
            );
            compose_rx = Some((req_id, rx));
        }
    }

    // Write clean reference PNG directly from the already-encoded bytes (zero second PNG encode!)
    let clean_written = (encoded_ok && fs::write(&clean_path, &png_buf).is_ok())
        || raw.image.save_with_format(&clean_path, ImageFormat::Png).is_ok();
    let saved_clean = if clean_written {
        register_saved_capture(&clean_path, cfg);
        Some(clean_path)
    } else {
        None
    };

    let mut composed_bytes: Option<Vec<u8>> = None;
    if let Some((req_id, rx)) = compose_rx {
        if let Ok(data_url) = rx.recv_timeout(Duration::from_millis(400)) {
            if let Some(stripped) = data_url.strip_prefix("data:image/png;base64,") {
                if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(stripped) {
                    if !decoded.is_empty() {
                        composed_bytes = Some(decoded);
                    }
                }
            }
        } else if let Some(shared) = app.try_state::<AppShared>() {
            if let Ok(mut map) = shared.compose_waiters.lock() {
                map.remove(&req_id);
            }
        }
    }

    let saved_annot = if let Some(bytes) = composed_bytes {
        if fs::write(&annot_path, bytes).is_ok() {
            register_saved_capture(&annot_path, cfg);
            Some(annot_path)
        } else {
            None
        }
    } else {
        let fallback_img = annotate_rgba_in_rust(raw, rect, strokes, number);
        if fallback_img
            .save_with_format(&annot_path, ImageFormat::Png)
            .is_ok()
        {
            register_saved_capture(&annot_path, cfg);
            Some(annot_path)
        } else {
            None
        }
    };

    (saved_annot, saved_clean)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn purges_after_20_subsequent_sent_messages() {
        let mut idx = CaptureIndex::default();
        idx.record_new_capture("cap1.png".into(), 1000, 20, 60);
        assert_eq!(idx.items.len(), 1);

        // First send attaches cap1 at seq=1
        let evicted = idx.on_message_sent(1010, 20, 60);
        assert!(evicted.is_empty());
        assert_eq!(idx.items[0].attached_at_msg_seq, Some(1));

        // Next 19 sends (seq 2..=20): diff is 19 < 20, still kept
        for step in 0..19 {
            let ev = idx.on_message_sent(1020 + step, 20, 60);
            assert!(ev.is_empty());
        }
        assert_eq!(idx.items.len(), 1);

        // 20th subsequent send (seq=21, 21 - 1 == 20): evicted!
        let ev = idx.on_message_sent(1100, 20, 60);
        assert_eq!(ev, vec!["cap1.png".to_string()]);
        assert!(idx.items.is_empty());
    }

    #[test]
    fn purges_unsent_orphans_after_60_minutes() {
        let mut idx = CaptureIndex::default();
        idx.record_new_capture("orphan.png".into(), 1000, 20, 60);

        // At +59 minutes (3540s), still alive
        let ev1 = idx.purge_expired(1000 + 3540, 20, 60);
        assert!(ev1.is_empty());
        assert_eq!(idx.items.len(), 1);

        // At +60 minutes (3600s), purged!
        let ev2 = idx.purge_expired(1000 + 3600, 20, 60);
        assert_eq!(ev2, vec!["orphan.png".to_string()]);
        assert!(idx.items.is_empty());
    }
}
