use std::sync::atomic::{AtomicBool, Ordering};
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

const CURSOR_SIZE: usize = 64;
const HOTSPOT_X: u32 = 16;
const HOTSPOT_Y: u32 = 16;

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

/// Renders a refined 64x64 32-bit BGRA default Windows arrow:
/// - Clean translucent dark-glass hollow interior (never filled with a thick glowing blob!)
/// - Razor-crisp 1.25px pure-white luminous contour right on the arrow border
/// - Tight, delicate electric-blue outer-only Gaussian halo (`sdf > 0`, fading cleanly by ~6.5px)
pub fn render_tech_cursor_bgra(size: usize) -> Vec<u8> {
    let mut buf = vec![0u8; size * size * 4];

    for y in 0..size {
        for x in 0..size {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let sdf = default_arrow_sdf(px, py) - 0.35;

            let (mut r, mut g, mut b, mut a) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);

            if sdf < 0.0 {
                // Inside the arrow: translucent dark-glass core + razor-crisp white rim ONLY at the edge
                let dist_in = -sdf;
                // Subtle translucent slate glass center (0.24 alpha) so arrow stays readable on pure white pages yet looks hollow/glassy like the Apple reference
                r = 16.0;
                g = 22.0;
                b = 34.0;
                a = 0.24;

                // Crisp 1.5px white rim along the inner contour
                let rim_cov = (1.65 - dist_in).clamp(0.0, 1.0);
                if rim_cov > 0.0 {
                    let out_a = rim_cov + a * (1.0 - rim_cov);
                    r = (255.0 * rim_cov + r * a * (1.0 - rim_cov)) / out_a;
                    g = (255.0 * rim_cov + g * a * (1.0 - rim_cov)) / out_a;
                    b = (255.0 * rim_cov + b * a * (1.0 - rim_cov)) / out_a;
                    a = out_a;
                }
            } else if sdf < 7.0 {
                // Strictly OUTSIDE the arrow (`sdf >= 0`): crisp outer edge anti-alias + tight Apple-blue outer bloom ONLY
                let edge_white = (1.0 - sdf / 0.85).clamp(0.0, 1.0);

                // Tight, restrained electric-blue outer Gaussian halo (sigma = 2.05px, zero inward bleed!)
                let glow = (-(sdf * sdf) / (2.0 * 2.05 * 2.05)).exp() * 0.72;
                let color_t = (sdf / 3.2).clamp(0.0, 1.0);
                let gr = lerp(135.0, 0.0, color_t);
                let gg = lerp(228.0, 136.0, color_t);
                let gb = 255.0;

                // Composite crisp white edge over tight blue outer glow
                let out_a = edge_white + glow * (1.0 - edge_white);
                if out_a > 1e-4 {
                    r = (255.0 * edge_white + gr * glow * (1.0 - edge_white)) / out_a;
                    g = (255.0 * edge_white + gg * glow * (1.0 - edge_white)) / out_a;
                    b = (255.0 * edge_white + gb * glow * (1.0 - edge_white)) / out_a;
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

fn create_tech_hcursor() -> Option<HCURSOR> {
    let pixels = render_tech_cursor_bgra(CURSOR_SIZE);
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
            xHotspot: HOTSPOT_X,
            yHotspot: HOTSPOT_Y,
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

pub fn enable_tech_cursor() {
    if TECH_CURSOR_ACTIVE.swap(true, Ordering::AcqRel) {
        return;
    }
    let Some(base_cursor) = create_tech_hcursor() else {
        TECH_CURSOR_ACTIVE.store(false, Ordering::Release);
        return;
    };

    const SLOTS: &[SYSTEM_CURSOR_ID] = &[
        OCR_NORMAL,
        OCR_IBEAM,
        OCR_HAND,
        OCR_CROSS,
        OCR_APPSTARTING,
        OCR_WAIT,
        OCR_SIZEALL,
    ];

    unsafe {
        for &slot in SLOTS {
            if let Ok(copy) = CopyIcon(HICON(base_cursor.0)) {
                let _ = SetSystemCursor(HCURSOR(copy.0), slot);
            }
        }
        let _ = DestroyIcon(HICON(base_cursor.0));
    }
}

pub fn restore_system_cursors() {
    if !TECH_CURSOR_ACTIVE.swap(false, Ordering::AcqRel) {
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
}
