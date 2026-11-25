use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, GetDC, ReleaseDC, BITMAPINFO, BITMAPV5HEADER,
    BI_BITFIELDS, DIB_RGB_COLORS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CopyIcon, CreateIconIndirect, DestroyIcon, SetSystemCursor, SystemParametersInfoW, HCURSOR,
    HICON, ICONINFO, OCR_APPSTARTING, OCR_CROSS, OCR_HAND, OCR_IBEAM, OCR_NORMAL, OCR_SIZEALL,
    OCR_WAIT, SPI_SETCURSORS, SYSTEM_CURSOR_ID, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
};

static TECH_CURSOR_ACTIVE: AtomicBool = AtomicBool::new(false);
static IBEAM_OVERRIDE_ACTIVE: AtomicBool = AtomicBool::new(false);
static BREATH_ACTIVE: AtomicBool = AtomicBool::new(false);
static BREATH_GENERATION: AtomicU64 = AtomicU64::new(0);

const CURSOR_SIZE: usize = 64;
const HOTSPOT_X: u32 = 16;
const HOTSPOT_Y: u32 = 16;
const IBEAM_HOTSPOT_X: u32 = 32;
const IBEAM_HOTSPOT_Y: u32 = 32;

const BREATH_RISE_MS: u64 = 240;
const BREATH_HOLD_END_MS: u64 = 920;
const BREATH_TOTAL_MS: u64 = 1480;
const BREATH_FRAME_MS: u64 = 32;

fn dist_to_segment(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let abx = bx - ax;
    let aby = by - ay;
    let apx = px - ax;
    let apy = py - ay;
    let ab_len_sq = abx * abx + aby * aby;
    let t = if ab_len_sq > 1e-6 {
        ((apx * abx + apy * aby) / ab_len_sq).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let cx = ax + abx * t;
    let cy = ay + aby * t;
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt()
}

/// Exact signed distance to the classic default Windows 11 arrow cursor (with stem),
/// tip anchored at `(16.0, 16.0)`.
fn default_arrow_sdf(px: f32, py: f32) -> f32 {
    const POLY: [(f32, f32); 7] = [
        (16.0, 16.0), // precision tip
        (16.0, 36.8), // left-bottom corner
        (20.9, 32.1), // inner left notch
        (24.0, 39.2), // stem bottom-left
        (26.7, 38.0), // stem bottom-right
        (23.6, 30.9), // inner right notch
        (30.4, 30.9), // right wing tip
    ];

    let mut min_dist = f32::MAX;
    let mut inside = false;
    let n = POLY.len();
    for i in 0..n {
        let (ax, ay) = POLY[i];
        let (bx, by) = POLY[(i + 1) % n];
        let d = dist_to_segment(px, py, ax, ay, bx, by);
        if d < min_dist {
            min_dist = d;
        }
        let cond = (ay > py) != (by > py);
        if cond {
            let intersect_x = ax + (py - ay) * (bx - ax) / (by - ay);
            if px < intersect_x {
                inside = !inside;
            }
        }
    }

    if inside {
        -min_dist
    } else {
        min_dist
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

/// Smooth 3-stage breathing envelope (`0.0 -> 1.0 -> hold ~1s -> 0.0`):
/// - `0..240ms`: gentle sine ease-out ignite
/// - `240..920ms`: bright hold with subtle breathing shimmer
/// - `920..1480ms`: smooth cosine ease-in-out fade back to normal cursor
/// - `>= 1480ms`: `None` (finished, restore native system cursors)
pub fn breath_envelope(elapsed_ms: u64) -> Option<f32> {
    if elapsed_ms >= BREATH_TOTAL_MS {
        return None;
    }
    if elapsed_ms < BREATH_RISE_MS {
        let p = elapsed_ms as f32 / BREATH_RISE_MS as f32;
        Some((p * std::f32::consts::FRAC_PI_2).sin().clamp(0.0, 1.0))
    } else if elapsed_ms < BREATH_HOLD_END_MS {
        let p = (elapsed_ms - BREATH_RISE_MS) as f32 / (BREATH_HOLD_END_MS - BREATH_RISE_MS) as f32;
        // Subtle organic breathing shimmer around peak brightness (0.95 .. 1.0)
        Some((0.975 + 0.025 * (p * std::f32::consts::TAU).cos()).clamp(0.0, 1.0))
    } else {
        let p = (elapsed_ms - BREATH_HOLD_END_MS) as f32 / (BREATH_TOTAL_MS - BREATH_HOLD_END_MS) as f32;
        Some((0.5 * (1.0 + (p * std::f32::consts::PI).cos())).clamp(0.0, 1.0))
    }
}

/// Renders a refined 64x64 32-bit BGRA default Windows arrow at full selection intensity (`1.0`).
pub fn render_tech_cursor_bgra(size: usize) -> Vec<u8> {
    render_tech_cursor_bgra_with_intensity(size, 1.0)
}

/// Renders a 64x64 32-bit BGRA arrow interpolated smoothly between:
/// - `intensity = 0.0`: classic Windows 11 crisp white arrow with fine dark outline (zero glow)
/// - `intensity = 1.0`: selection-mode translucent dark-glass core, pure white luminous rim,
///   and tight electric-blue outer Gaussian halo (`sdf > 0`, fading cleanly by ~6.5px)
pub fn render_tech_cursor_bgra_with_intensity(size: usize, intensity: f32) -> Vec<u8> {
    let t = intensity.clamp(0.0, 1.0);
    let mut buf = vec![0u8; size * size * 4];

    for y in 0..size {
        for x in 0..size {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let sdf = default_arrow_sdf(px, py) - 0.35;

            let (mut r, mut g, mut b, mut a) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);

            if sdf < 0.0 {
                let dist_in = -sdf;
                // Blend body from opaque white Windows arrow (t=0) to translucent dark-glass core (t=1)
                r = lerp(255.0, 16.0, t);
                g = lerp(255.0, 22.0, t);
                b = lerp(255.0, 34.0, t);
                a = lerp(1.0, 0.24, t);

                // Inner contour blends from crisp dark outline (t=0) to luminous pure-white rim (t=1)
                let rim_width = lerp(1.15, 1.65, t);
                let rim_cov = (rim_width - dist_in).clamp(0.0, 1.0);
                if rim_cov > 0.0 {
                    let rim_r = lerp(28.0, 255.0, t);
                    let rim_g = lerp(30.0, 255.0, t);
                    let rim_b = lerp(36.0, 255.0, t);
                    let out_a = rim_cov + a * (1.0 - rim_cov);
                    r = (rim_r * rim_cov + r * a * (1.0 - rim_cov)) / out_a;
                    g = (rim_g * rim_cov + g * a * (1.0 - rim_cov)) / out_a;
                    b = (rim_b * rim_cov + b * a * (1.0 - rim_cov)) / out_a;
                    a = out_a;
                }
            } else if sdf < 7.0 {
                // Outer edge anti-alias blends from subtle dark edge (t=0) to crisp white rim (t=1)
                let edge_cov = (1.0 - sdf / 0.85).clamp(0.0, 1.0);
                let edge_r = lerp(28.0, 255.0, t);
                let edge_g = lerp(30.0, 255.0, t);
                let edge_b = lerp(36.0, 255.0, t);

                // Tight electric-blue outer Gaussian halo scales smoothly with `t` (0 at rest, 0.72 at peak)
                let sigma = lerp(1.55, 2.05, t);
                let glow = (-(sdf * sdf) / (2.0 * sigma * sigma)).exp() * 0.72 * t;
                let color_t = (sdf / 3.2).clamp(0.0, 1.0);
                let gr = lerp(135.0, 0.0, color_t);
                let gg = lerp(228.0, 136.0, color_t);
                let gb = 255.0;

                let out_a = edge_cov + glow * (1.0 - edge_cov);
                if out_a > 1e-4 {
                    r = (edge_r * edge_cov + gr * glow * (1.0 - edge_cov)) / out_a;
                    g = (edge_g * edge_cov + gg * glow * (1.0 - edge_cov)) / out_a;
                    b = (edge_b * edge_cov + gb * glow * (1.0 - edge_cov)) / out_a;
                    a = out_a;
                }
            }

            let alpha_u8 = (a.clamp(0.0, 1.0) * 255.0).round() as u8;
            let pm = alpha_u8 as f32 / 255.0;
            let idx = (y * size + x) * 4;
            buf[idx] = (b.clamp(0.0, 255.0) * pm).round() as u8;
            buf[idx + 1] = (g.clamp(0.0, 255.0) * pm).round() as u8;
            buf[idx + 2] = (r.clamp(0.0, 255.0) * pm).round() as u8;
            buf[idx + 3] = alpha_u8;
        }
    }

    buf
}

/// Signed distance to a crisp 20px-tall text I-beam cursor centered at `(32.0, 32.0)`.
fn ibeam_sdf(px: f32, py: f32) -> f32 {
    let d_stem = dist_to_segment(px, py, 32.0, 22.0, 32.0, 42.0);
    let d_top = dist_to_segment(px, py, 28.0, 22.0, 36.0, 22.0);
    let d_bot = dist_to_segment(px, py, 28.0, 42.0, 36.0, 42.0);
    d_stem.min(d_top).min(d_bot) - 1.05
}

/// Renders a matching 64x64 32-bit BGRA text I-beam cursor with razor-crisp white core
/// and tight electric-blue outer glow for Word / WPS text selection mode.
pub fn render_tech_ibeam_bgra(size: usize) -> Vec<u8> {
    render_tech_ibeam_bgra_with_intensity(size, 1.0)
}

pub fn render_tech_ibeam_bgra_with_intensity(size: usize, intensity: f32) -> Vec<u8> {
    let t = intensity.clamp(0.0, 1.0);
    let mut buf = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let sdf = ibeam_sdf(px, py);
            let (mut r, mut g, mut b, mut a) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            if sdf < 0.0 {
                // Crisp dark-bordered white I-beam readable on both white Word pages and dark themes
                let dist_in = -sdf;
                r = lerp(24.0, 255.0, t);
                g = lerp(24.0, 255.0, t);
                b = lerp(28.0, 255.0, t);
                a = (dist_in * 1.4).clamp(0.88, 1.0);
            } else if sdf < 6.0 {
                let dark_rim = (1.0 - sdf / 1.15).clamp(0.0, 1.0) * lerp(0.55, 0.72, t);
                let glow = (-(sdf * sdf) / (2.0 * 2.0 * 2.0)).exp() * 0.78 * t;
                let color_t = (sdf / 3.0).clamp(0.0, 1.0);
                let gr = lerp(135.0, 0.0, color_t);
                let gg = lerp(228.0, 136.0, color_t);
                let gb = 255.0;
                a = (dark_rim + glow * (1.0 - dark_rim)).clamp(0.0, 1.0);
                if a > 0.001 {
                    let rim_c = lerp(245.0, 18.0, t);
                    r = (rim_c * dark_rim + gr * glow * (1.0 - dark_rim)) / a;
                    g = (lerp(245.0, 28.0, t) * dark_rim + gg * glow * (1.0 - dark_rim)) / a;
                    b = (lerp(245.0, 48.0, t) * dark_rim + gb * glow * (1.0 - dark_rim)) / a;
                }
            }
            if a > 0.001 {
                let idx = (y * size + x) * 4;
                buf[idx] = (b * a).round().clamp(0.0, 255.0) as u8;
                buf[idx + 1] = (g * a).round().clamp(0.0, 255.0) as u8;
                buf[idx + 2] = (r * a).round().clamp(0.0, 255.0) as u8;
                buf[idx + 3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    buf
}

fn create_hcursor_from_bgra(pixels: &[u8], hotspot_x: u32, hotspot_y: u32) -> Option<HCURSOR> {
    unsafe {
        let mut bi = BITMAPV5HEADER {
            bV5Size: std::mem::size_of::<BITMAPV5HEADER>() as u32,
            bV5Width: CURSOR_SIZE as i32,
            bV5Height: -(CURSOR_SIZE as i32),
            bV5Planes: 1,
            bV5BitCount: 32,
            bV5Compression: BI_BITFIELDS,
            bV5RedMask: 0x00FF_0000,
            bV5GreenMask: 0x0000_FF00,
            bV5BlueMask: 0x0000_00FF,
            bV5AlphaMask: 0xFF00_0000,
            ..Default::default()
        };

        let hdc = GetDC(None);
        let mut bits_ptr: *mut std::ffi::c_void = std::ptr::null_mut();
        let hbm_color = CreateDIBSection(
            Some(hdc),
            &mut bi as *mut _ as *const BITMAPINFO,
            DIB_RGB_COLORS,
            &mut bits_ptr,
            None,
            0,
        )
        .ok();
        let _ = ReleaseDC(None, hdc);

        let hbm_color = hbm_color?;
        if bits_ptr.is_null() {
            let _ = DeleteObject(hbm_color.into());
            return None;
        }

        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits_ptr as *mut u8, pixels.len());

        let hbm_mask = CreateBitmap(CURSOR_SIZE as i32, CURSOR_SIZE as i32, 1, 1, None);
        if hbm_mask.0.is_null() {
            let _ = DeleteObject(hbm_color.into());
            return None;
        }

        let icon_info = ICONINFO {
            fIcon: false.into(),
            xHotspot: hotspot_x,
            yHotspot: hotspot_y,
            hbmMask: hbm_mask,
            hbmColor: hbm_color,
        };

        let icon_res = CreateIconIndirect(&icon_info);
        let _ = DeleteObject(hbm_color.into());
        let _ = DeleteObject(hbm_mask.into());

        match icon_res {
            Ok(hicon) => Some(HCURSOR(hicon.0)),
            Err(e) => {
                log::warn!("CreateIconIndirect failed for tech cursor: {e}");
                None
            }
        }
    }
}

fn create_tech_hcursor_with_intensity(intensity: f32) -> Option<HCURSOR> {
    let pixels = render_tech_cursor_bgra_with_intensity(CURSOR_SIZE, intensity);
    create_hcursor_from_bgra(&pixels, HOTSPOT_X, HOTSPOT_Y)
}

fn create_tech_ibeam_hcursor_with_intensity(intensity: f32) -> Option<HCURSOR> {
    let pixels = render_tech_ibeam_bgra_with_intensity(CURSOR_SIZE, intensity);
    create_hcursor_from_bgra(&pixels, IBEAM_HOTSPOT_X, IBEAM_HOTSPOT_Y)
}

fn create_tech_hcursor() -> Option<HCURSOR> {
    create_tech_hcursor_with_intensity(1.0)
}

fn create_tech_ibeam_hcursor() -> Option<HCURSOR> {
    create_tech_ibeam_hcursor_with_intensity(1.0)
}

fn apply_breath_frame(intensity: f32, gen: u64) -> bool {
    if TECH_CURSOR_ACTIVE.load(Ordering::Acquire)
        || BREATH_GENERATION.load(Ordering::Acquire) != gen
    {
        return false;
    }
    let Some(arrow_cur) = create_tech_hcursor_with_intensity(intensity) else {
        return false;
    };
    let ibeam_cur = create_tech_ibeam_hcursor_with_intensity(intensity);

    if TECH_CURSOR_ACTIVE.load(Ordering::Acquire)
        || BREATH_GENERATION.load(Ordering::Acquire) != gen
    {
        unsafe {
            let _ = DestroyIcon(HICON(arrow_cur.0));
            if let Some(ib) = ibeam_cur {
                let _ = DestroyIcon(HICON(ib.0));
            }
        }
        return false;
    }

    const BREATH_ARROW_SLOTS: &[SYSTEM_CURSOR_ID] = &[OCR_NORMAL, OCR_HAND, OCR_APPSTARTING];
    unsafe {
        for &slot in BREATH_ARROW_SLOTS {
            if let Ok(copy) = CopyIcon(HICON(arrow_cur.0)) {
                let _ = SetSystemCursor(HCURSOR(copy.0), slot);
            }
        }
        if let Some(ib) = ibeam_cur {
            if let Ok(copy) = CopyIcon(HICON(ib.0)) {
                let _ = SetSystemCursor(HCURSOR(copy.0), OCR_IBEAM);
            }
            let _ = DestroyIcon(HICON(ib.0));
        }
        let _ = DestroyIcon(HICON(arrow_cur.0));
    }
    true
}

/// Plays a gentle ~1.5s "ready" breathing-light cursor pulse on startup/install completion:
/// smoothly illuminates the cursor into selection-mode electric blue for ~1s, then gracefully
/// fades back out to the native Windows cursor so the user immediately knows Ctrl is armed.
/// Automatically yields with zero interference if the user enters real selection mode mid-pulse.
pub fn play_ready_breath_pulse() {
    if TECH_CURSOR_ACTIVE.load(Ordering::Acquire) {
        return;
    }
    let gen = BREATH_GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    let _ = std::thread::Builder::new()
        .name("dsb-cursor-breath".to_string())
        .spawn(move || {
            // Brief settle pause so tray + low-level hooks are live before pulsing
            std::thread::sleep(Duration::from_millis(180));
            if TECH_CURSOR_ACTIVE.load(Ordering::Acquire)
                || BREATH_GENERATION.load(Ordering::Acquire) != gen
            {
                return;
            }
            BREATH_ACTIVE.store(true, Ordering::Release);
            let started = Instant::now();
            loop {
                let elapsed_ms = started.elapsed().as_millis() as u64;
                let Some(intensity) = breath_envelope(elapsed_ms) else {
                    break;
                };
                if !apply_breath_frame(intensity, gen) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(BREATH_FRAME_MS));
            }

            if !TECH_CURSOR_ACTIVE.load(Ordering::Acquire)
                && BREATH_GENERATION.load(Ordering::Acquire) == gen
                && BREATH_ACTIVE.swap(false, Ordering::AcqRel)
            {
                unsafe {
                    let _ = SystemParametersInfoW(
                        SPI_SETCURSORS,
                        0,
                        None,
                        SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
                    );
                }
            }
        });
}

pub fn enable_tech_cursor() {
    // Immediately cancel any in-flight startup breath pulse so selection mode takes ownership cleanly
    BREATH_GENERATION.fetch_add(1, Ordering::AcqRel);
    BREATH_ACTIVE.store(false, Ordering::Release);

    if TECH_CURSOR_ACTIVE.swap(true, Ordering::AcqRel) {
        return;
    }
    IBEAM_OVERRIDE_ACTIVE.store(false, Ordering::Release);
    let Some(base_cursor) = create_tech_hcursor() else {
        TECH_CURSOR_ACTIVE.store(false, Ordering::Release);
        return;
    };
    let ibeam_cursor = create_tech_ibeam_hcursor();

    const ARROW_SLOTS: &[SYSTEM_CURSOR_ID] = &[
        OCR_NORMAL,
        OCR_HAND,
        OCR_CROSS,
        OCR_APPSTARTING,
        OCR_WAIT,
        OCR_SIZEALL,
    ];

    unsafe {
        for &slot in ARROW_SLOTS {
            if let Ok(copy) = CopyIcon(HICON(base_cursor.0)) {
                let _ = SetSystemCursor(HCURSOR(copy.0), slot);
            }
        }
        if let Some(ib) = ibeam_cursor {
            if let Ok(copy) = CopyIcon(HICON(ib.0)) {
                let _ = SetSystemCursor(HCURSOR(copy.0), OCR_IBEAM);
            }
            let _ = DestroyIcon(HICON(ib.0));
        } else if let Ok(copy) = CopyIcon(HICON(base_cursor.0)) {
            let _ = SetSystemCursor(HCURSOR(copy.0), OCR_IBEAM);
        }
        let _ = DestroyIcon(HICON(base_cursor.0));
    }
}

/// Switches the primary selection cursor (`OCR_NORMAL`) to the glowing text I-beam while
/// dragging over Word / WPS text, and switches it back to the glass arrow once done.
pub fn set_ibeam_override(enable: bool) {
    if !TECH_CURSOR_ACTIVE.load(Ordering::Acquire) {
        return;
    }
    let prev = IBEAM_OVERRIDE_ACTIVE.swap(enable, Ordering::AcqRel);
    if prev == enable {
        return;
    }
    let next = if enable {
        create_tech_ibeam_hcursor()
    } else {
        create_tech_hcursor()
    };
    if let Some(cur) = next {
        unsafe {
            if let Ok(copy) = CopyIcon(HICON(cur.0)) {
                let _ = SetSystemCursor(HCURSOR(copy.0), OCR_NORMAL);
            }
            let _ = DestroyIcon(HICON(cur.0));
        }
    }
}

pub fn restore_system_cursors() {
    BREATH_GENERATION.fetch_add(1, Ordering::AcqRel);
    IBEAM_OVERRIDE_ACTIVE.store(false, Ordering::Release);
    let was_tech = TECH_CURSOR_ACTIVE.swap(false, Ordering::AcqRel);
    let was_breath = BREATH_ACTIVE.swap(false, Ordering::AcqRel);
    if !was_tech && !was_breath {
        return;
    }
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_SETCURSORS,
            0,
            None,
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_hollow_glass_arrow_with_tight_outer_blue_glow() {
        let bgra = render_tech_cursor_bgra(CURSOR_SIZE);
        assert_eq!(bgra.len(), CURSOR_SIZE * CURSOR_SIZE * 4);

        // Deep inside arrow body (19, 27) should be light translucent glass (NOT solid opaque white!)
        let core_idx = (27 * CURSOR_SIZE + 19) * 4;
        assert!(bgra[core_idx + 3] > 30 && bgra[core_idx + 3] < 100);

        // Right on the left edge (16, 25) should be crisp bright white rim
        let rim_idx = (25 * CURSOR_SIZE + 16) * 4;
        assert!(bgra[rim_idx + 3] > 210);

        // 2.5px outside left edge (13, 25) should have tight pure electric-blue outer bloom (B > R)
        let bloom_idx = (25 * CURSOR_SIZE + 13) * 4;
        assert!(bgra[bloom_idx + 3] > 20 && bgra[bloom_idx + 3] < 180);
        assert!(bgra[bloom_idx] > bgra[bloom_idx + 2]);

        // Far outside (8, 25, >7px away) must be cleanly transparent (tight restrained glow!)
        let far_idx = (25 * CURSOR_SIZE + 8) * 4;
        assert_eq!(bgra[far_idx + 3], 0);
    }

    #[test]
    fn breath_envelope_and_zero_intensity_cursor_fade_cleanly() {
        // Envelope rises from 0 -> peak (~1.0) -> holds ~1s -> fades smoothly to 0 -> None
        assert!((breath_envelope(0).unwrap() - 0.0).abs() < 0.01);
        assert!(breath_envelope(120).unwrap() > 0.65);
        assert!(breath_envelope(300).unwrap() > 0.94);
        assert!(breath_envelope(800).unwrap() > 0.94);
        let mid_fade = breath_envelope(1200).unwrap();
        assert!(mid_fade > 0.35 && mid_fade < 0.65);
        assert!(breath_envelope(1460).unwrap() < 0.02);
        assert!(breath_envelope(BREATH_TOTAL_MS).is_none());

        // At intensity 0.0, arrow interior is opaque white and outer bloom is completely extinguished
        let rest = render_tech_cursor_bgra_with_intensity(CURSOR_SIZE, 0.0);
        let core_idx = (27 * CURSOR_SIZE + 19) * 4;
        assert_eq!(rest[core_idx + 3], 255);
        assert_eq!(rest[core_idx], 255);
        assert_eq!(rest[core_idx + 2], 255);
        let bloom_idx = (25 * CURSOR_SIZE + 13) * 4;
        assert_eq!(rest[bloom_idx + 3], 0);
    }
}
