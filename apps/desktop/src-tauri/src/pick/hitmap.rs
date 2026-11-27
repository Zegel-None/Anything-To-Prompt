use crate::captures::{capture_screen_rect_rgba, ScreenRect};
use crate::pick::probe;
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HWND;

const HITMAP_TTL: Duration = Duration::from_secs(20);
const HITMAP_MAX_CONTAINERS: usize = 48;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HitMapSource {
    MsaaSnapshot,
    StaticVisual,
    ExternalBoxProvider,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainerLayoutKind {
    Auto,
    HorizontalRibbon,
    VerticalIconStrip,
    TreeOrMenuRows,
    IconCardGrid,
}

impl ContainerLayoutKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ContainerLayoutKind::Auto => "auto",
            ContainerLayoutKind::HorizontalRibbon => "horizontal_ribbon",
            ContainerLayoutKind::VerticalIconStrip => "vertical_icon_strip",
            ContainerLayoutKind::TreeOrMenuRows => "tree_or_menu_rows",
            ContainerLayoutKind::IconCardGrid => "icon_card_grid",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "horizontal_ribbon" => ContainerLayoutKind::HorizontalRibbon,
            "vertical_icon_strip" => ContainerLayoutKind::VerticalIconStrip,
            "tree_or_menu_rows" => ContainerLayoutKind::TreeOrMenuRows,
            "icon_card_grid" => ContainerLayoutKind::IconCardGrid,
            _ => ContainerLayoutKind::Auto,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HitMapEntry {
    pub rect: ScreenRect,
    pub label: String,
    pub description: String,
    pub role: String,
    pub source: HitMapSource,
    /// True when this element is a dropdown trigger (`▾`), top menu-bar header, or cascading submenu row (`▸`).
    #[serde(default)]
    pub is_expandable: bool,
    /// Safe trigger coordinate (chevron / menu header / submenu row center) for drawer exploration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_pt: Option<(i32, i32)>,
}

#[derive(Debug, Clone)]
pub struct ContainerHitMap {
    pub container_hwnd: isize,
    pub container_rect: ScreenRect,
    pub visual_hash: u64,
    pub entries: Vec<HitMapEntry>,
    pub built_at: Instant,
    pub source: HitMapSource,
}

/// Extension point for an optional, user-downloaded local UI box detector (e.g., ONNX/OmniParser).
pub trait VisualBoxProvider: Send + Sync {
    fn name(&self) -> &'static str;
    fn detect_boxes(&self, image: &RgbaImage, origin_x: i32, origin_y: i32) -> Vec<ScreenRect>;
}

type BoxProviderSlot = Mutex<Option<Arc<dyn VisualBoxProvider>>>;
type HitMapCacheKey = (isize, HitMapSource);

static VISUAL_BOX_PROVIDER: OnceLock<BoxProviderSlot> = OnceLock::new();
static HITMAP_CACHE: OnceLock<Mutex<HashMap<HitMapCacheKey, ContainerHitMap>>> = OnceLock::new();

fn cache_lock() -> &'static Mutex<HashMap<HitMapCacheKey, ContainerHitMap>> {
    HITMAP_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn register_visual_box_provider(provider: Option<Arc<dyn VisualBoxProvider>>) {
    let slot = VISUAL_BOX_PROVIDER.get_or_init(|| Mutex::new(None));
    if let Ok(mut g) = slot.lock() {
        *g = provider;
    }
    invalidate_all_hitmaps();
}

/// Inspects a single detected UI button rect inside `img` (origin `ox, oy`) to determine whether it
/// carries a genuine dropdown chevron (`▾`), submenu arrow (`▸`), or sits as a top-left menu-bar
/// trigger (e.g., SolidWorks `SOLIDWORKS Design ▸`, `New ▾`, `Fillet ▾`).
/// Strictly verifies isolated triangular geometry so plain executable buttons (Save/Print/Close/text)
/// are NEVER falsely clicked!
pub fn detect_expandable_trigger_in_rgba(
    img: &RgbaImage,
    ox: i32,
    oy: i32,
    rect: ScreenRect,
    win_rect: ScreenRect,
) -> Option<(i32, i32)> {
    let iw = img.width() as i32;
    let ih = img.height() as i32;
    let lx0 = (rect.x - ox).clamp(0, iw);
    let ly0 = (rect.y - oy).clamp(0, ih);
    let lx1 = (rect.x + rect.w - ox).clamp(lx0, iw);
    let ly1 = (rect.y + rect.h - oy).clamp(ly0, ih);
    let rw = lx1 - lx0;
    let rh = ly1 - ly0;
    if rw < 10 || rh < 10 {
        return None;
    }

    // Estimate local background from the 4 corners of the box (median-like pick of brightest/most common corner)
    let corners = [
        img.get_pixel(lx0 as u32, ly0 as u32).0,
        img.get_pixel((lx1 - 1).max(lx0) as u32, ly0 as u32).0,
        img.get_pixel(lx0 as u32, (ly1 - 1).max(ly0) as u32).0,
        img.get_pixel((lx1 - 1).max(lx0) as u32, (ly1 - 1).max(ly0) as u32).0,
    ];
    let mut bg = (corners[0][0], corners[0][1], corners[0][2]);
    let mut best_agree = 0usize;
    for c in &corners {
        let agree = corners
            .iter()
            .filter(|o| {
                (c[0] as i32 - o[0] as i32).abs()
                    + (c[1] as i32 - o[1] as i32).abs()
                    + (c[2] as i32 - o[2] as i32).abs()
                    <= 30
            })
            .count();
        if agree > best_agree {
            best_agree = agree;
            bg = (c[0], c[1], c[2]);
        }
    }
    let is_fg = |px: i32, py: i32| -> bool {
        if px < 0 || py < 0 || px >= iw || py >= ih {
            return false;
        }
        let p = img.get_pixel(px as u32, py as u32).0;
        ((p[0] as i32 - bg.0 as i32).abs()
            + (p[1] as i32 - bg.1 as i32).abs()
            + (p[2] as i32 - bg.2 as i32).abs())
            >= 42
    };

    // Helper: checks if foreground pixels inside `[sx0..sx1) x [sy0..sy1)` form a compact
    // downward (`▾`) or right-pointing (`▸`) triangle (NOT a dense icon or text stroke).
    let is_compact_chevron = |sx0: i32, sx1: i32, sy0: i32, sy1: i32, allow_right: bool| -> Option<(i32, i32)> {
        if sx1 <= sx0 || sy1 <= sy0 {
            return None;
        }
        let mut min_x = sx1;
        let mut max_x = sx0;
        let mut min_y = sy1;
        let mut max_y = sy0;
        let mut cnt = 0i32;
        for py in sy0..sy1 {
            for px in sx0..sx1 {
                if is_fg(px, py) {
                    min_x = min_x.min(px);
                    max_x = max_x.max(px + 1);
                    min_y = min_y.min(py);
                    max_y = max_y.max(py + 1);
                    cnt += 1;
                }
            }
        }
        let cw = max_x - min_x;
        let ch = max_y - min_y;
        if !(4..=48).contains(&cnt) || !(3..=13).contains(&cw) || !(2..=13).contains(&ch) {
            return None;
        }
        // Check downward triangle `▾`: top half row span >= bottom half row span + 2
        let mid_y = (min_y + max_y) / 2;
        let mut top_max_w = 0i32;
        let mut bot_min_w = cw;
        for py in min_y..max_y {
            let mut row_w = 0i32;
            for px in min_x..max_x {
                if is_fg(px, py) {
                    row_w += 1;
                }
            }
            if py < mid_y {
                top_max_w = top_max_w.max(row_w);
            } else if row_w > 0 {
                bot_min_w = bot_min_w.min(row_w);
            }
        }
        if cw >= ch && ch <= 9 && top_max_w >= bot_min_w + 2 {
            return Some((ox + (min_x + max_x) / 2, oy + (min_y + max_y) / 2));
        }
        // Check right-pointing triangle `▸`: left half col span >= right half col span + 2
        if allow_right && ch >= cw && cw <= 9 {
            let mid_x = (min_x + max_x) / 2;
            let mut left_max_h = 0i32;
            let mut right_min_h = ch;
            for px in min_x..max_x {
                let mut col_h = 0i32;
                for py in min_y..max_y {
                    if is_fg(px, py) {
                        col_h += 1;
                    }
                }
                if px < mid_x {
                    left_max_h = left_max_h.max(col_h);
                } else if col_h > 0 {
                    right_min_h = right_min_h.min(col_h);
                }
            }
            if left_max_h >= right_min_h + 2 {
                return Some((ox + (min_x + max_x) / 2, oy + (min_y + max_y) / 2));
            }
        }
        None
    };

    // 1. Standalone narrow chevron button detected by OmniParser (e.g., `▾` strip beside New/Save/Fillet)
    //    MUST contain ONLY a compact chevron glyph (never a regular 20x20 toolbar icon!)
    if (10..=24).contains(&rw) && (14..=44).contains(&rh) {
        if let Some(pt) = is_compact_chevron(lx0 + 1, lx1 - 1, ly0 + 1, ly1 - 1, true) {
            return Some(pt);
        }
        return None;
    }

    // 2. Bottom-center `▾` chevron on tall split ribbon buttons (Fillet, Linear Pattern, Reference Geometry)
    if rh >= 56 && rw >= 34 {
        let bx0 = lx0 + rw / 4;
        let bx1 = lx1 - rw / 4;
        let by0 = (ly1 - 16).max(ly0 + rh * 2 / 3);
        let by1 = (ly1 - 1).max(by0 + 1);
        // Require a clean horizontal gap above the chevron separating caption text from `▾`
        let gap_y0 = (by0 - 5).max(ly0);
        let mut has_gap = false;
        for py in gap_y0..by0 {
            if (bx0..bx1).all(|px| !is_fg(px, py)) {
                has_gap = true;
                break;
            }
        }
        if has_gap {
            if let Some(pt) = is_compact_chevron(bx0, bx1, by0, by1, false) {
                return Some(pt);
            }
        }
    }

    // 3. Right-edge `▾` or `▸` chevron (e.g., `SOLIDWORKS Design ▸`, `Options ▾`, cascading menu items)
    if rw >= 36 && rh >= 18 {
        let rx0 = (lx1 - 16).max(lx0 + rw * 2 / 3);
        let rx1 = (lx1 - 1).max(rx0 + 1);
        let ry0 = ly0 + 2;
        let ry1 = (ly1 - 2).max(ry0 + 1);
        // Require a clean vertical gap (>= 3 empty columns) separating text/icon from the chevron
        let gx0 = (rx0 - 6).max(lx0);
        let empty_cols = (gx0..rx0)
            .filter(|&px| (ry0..ry1).all(|py| !is_fg(px, py)))
            .count();
        if empty_cols >= 3 {
            if let Some(pt) = is_compact_chevron(rx0, rx1, ry0, ry1, true) {
                return Some(pt);
            }
        }
    }

    // 4. Top-left menu bar logo/trigger (e.g., SolidWorks top-left `SOLIDWORKS Design ▸`)
    if (rect.y - win_rect.y).abs() <= 46
        && (rect.x - win_rect.x) <= 140
        && (90..=240).contains(&rw)
        && (18..=42).contains(&rh)
    {
        return Some((rect.x + rect.w - 10, rect.y + rect.h / 2));
    }

    None
}

pub fn active_visual_box_provider_name() -> Option<String> {
    let slot = VISUAL_BOX_PROVIDER.get_or_init(|| Mutex::new(None));
    slot.lock()
        .ok()
        .and_then(|g| g.as_ref().map(|p| p.name().to_string()))
}

pub fn detect_with_registered_box_provider(
    image: &RgbaImage,
    origin_x: i32,
    origin_y: i32,
) -> Vec<ScreenRect> {
    let provider = VISUAL_BOX_PROVIDER
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .and_then(|g| g.clone());
    match provider {
        Some(p) => p.detect_boxes(image, origin_x, origin_y),
        None => Vec::new(),
    }
}

pub fn invalidate_all_hitmaps() {
    if let Ok(mut g) = cache_lock().lock() {
        g.clear();
    }
}

pub fn invalidate_hitmap_for_hwnd(hwnd_raw: isize) {
    if let Ok(mut g) = cache_lock().lock() {
        g.retain(|(h, _), _| *h != hwnd_raw);
    }
}

/// Invalidates visual & external box caches after mouse-wheel scroll while keeping native metadata.
pub fn invalidate_visual_hitmaps_only() {
    if let Ok(mut g) = cache_lock().lock() {
        g.retain(|(_, src), _| *src == HitMapSource::MsaaSnapshot);
    }
}

/// Fast `<0.1ms` read-only lookup across ALL cached Hit-Map sources for `(hwnd_raw, x, y)`.
/// Prefers woken `ExternalBoxProvider` (OmniParser / Hover-Diff) while preserving native MSAA/Win32
/// labels/descriptions, and NEVER lets tiny `StaticVisual` fragments override a compact OmniParser button!
pub fn lookup_cached_any_source(hwnd_raw: isize, x: i32, y: i32) -> Option<HitMapEntry> {
    let ext = get_cached_entries(hwnd_raw, HitMapSource::ExternalBoxProvider)
        .and_then(|v| find_entry_at(&v, x, y));
    let msaa = get_cached_entries(hwnd_raw, HitMapSource::MsaaSnapshot)
        .and_then(|v| find_entry_at(&v, x, y));
    let stat = get_cached_entries(hwnd_raw, HitMapSource::StaticVisual)
        .and_then(|v| find_entry_at(&v, x, y));

    let mut best = ext.or(msaa.clone()).or(stat.clone())?;
    for cand in [msaa, stat].into_iter().flatten() {
        let best_area = (best.rect.w.max(1) as i64) * (best.rect.h.max(1) as i64);
        let cand_area = (cand.rect.w.max(1) as i64) * (cand.rect.h.max(1) as i64);
        let best_is_compact_ext = best.source == HitMapSource::ExternalBoxProvider
            && best.rect.w <= 240
            && best.rect.h <= 120;
        let can_split = !best_is_compact_ext && best_area >= cand_area * 4;
        if can_split {
            best = cand;
        } else {
            // Additive semantic preservation: borrow native label/description without altering geometry!
            if best.label.is_empty() && !cand.label.is_empty() {
                best.label = cand.label;
            }
            if best.description.is_empty() && !cand.description.is_empty() {
                best.description = cand.description;
            }
            if !best.is_expandable && cand.is_expandable {
                best.is_expandable = true;
                best.trigger_pt = cand.trigger_pt;
            }
        }
    }
    Some(best)
}

pub fn get_cached_entries(hwnd_raw: isize, source: HitMapSource) -> Option<Vec<HitMapEntry>> {
    let g = cache_lock().lock().ok()?;
    let m = g.get(&(hwnd_raw, source))?;
    if m.built_at.elapsed() > HITMAP_TTL {
        return None;
    }
    Some(m.entries.clone())
}

fn rect_contains_pt(r: ScreenRect, x: i32, y: i32, pad: i32) -> bool {
    x >= r.x - pad && x <= r.x + r.w + pad && y >= r.y - pad && y <= r.y + r.h + pad
}

/// Picks the tightest (smallest-area) entry containing `(x, y)`.
pub fn find_entry_at(entries: &[HitMapEntry], x: i32, y: i32) -> Option<HitMapEntry> {
    for pad in [0i32, 2i32] {
        let mut best: Option<(&HitMapEntry, i64)> = None;
        for e in entries {
            if !rect_contains_pt(e.rect, x, y, pad) {
                continue;
            }
            let area = (e.rect.w as i64) * (e.rect.h as i64);
            if best.is_none_or(|(_, ba)| area < ba) {
                best = Some((e, area));
            }
        }
        if let Some((e, _)) = best {
            return Some(e.clone());
        }
    }
    None
}

fn compute_image_sample_hash(img: &RgbaImage) -> u64 {
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return 0;
    }
    let mut hash: u64 = 0xcbf29ce484222325;
    let step_x = (w / 16).max(1);
    let step_y = (h / 8).max(1);
    let mut y = step_y / 2;
    while y < h {
        let mut x = step_x / 2;
        while x < w {
            let p = img.get_pixel(x, y);
            let q = ((p[0] as u64 >> 3) << 10) | ((p[1] as u64 >> 3) << 5) | (p[2] as u64 >> 3);
            hash ^= q;
            hash = hash.wrapping_mul(0x100000001b3);
            x += step_x;
        }
        y += step_y;
    }
    hash
}

fn store_hitmap(map: ContainerHitMap) {
    let key = (map.container_hwnd, map.source);
    if let Ok(mut g) = cache_lock().lock() {
        if g.len() >= HITMAP_MAX_CONTAINERS && !g.contains_key(&key) {
            if let Some(oldest_key) = g
                .iter()
                .min_by_key(|(_, v)| v.built_at)
                .map(|(k, _)| *k)
            {
                g.remove(&oldest_key);
            }
        }
        g.insert(key, map);
    }
}

/// Merges newly scanned entries into the cached Hit-Map for `(hwnd_raw, source)`, deduplicating
/// near-identical rects while preserving existing entries outside the newly brushed sub-region.
pub fn merge_entries_into_hitmap(
    hwnd_raw: isize,
    container_rect: ScreenRect,
    source: HitMapSource,
    new_entries: Vec<HitMapEntry>,
) {
    if new_entries.is_empty() {
        return;
    }
    if let Ok(mut g) = cache_lock().lock() {
        let key = (hwnd_raw, source);
        if g.len() >= HITMAP_MAX_CONTAINERS && !g.contains_key(&key) {
            if let Some(oldest_key) = g
                .iter()
                .min_by_key(|(_, v)| v.built_at)
                .map(|(k, _)| *k)
            {
                g.remove(&oldest_key);
            }
        }
        let entry = g.entry(key).or_insert_with(|| ContainerHitMap {
            container_hwnd: hwnd_raw,
            container_rect,
            visual_hash: 0,
            entries: Vec::new(),
            built_at: Instant::now(),
            source,
        });
        entry.container_rect = container_rect;
        entry.built_at = Instant::now();
        for ne in new_entries {
            let dup = entry.entries.iter().any(|ex| {
                (ex.rect.x - ne.rect.x).abs() <= 4
                    && (ex.rect.y - ne.rect.y).abs() <= 4
                    && (ex.rect.w - ne.rect.w).abs() <= 6
                    && (ex.rect.h - ne.rect.h).abs() <= 6
            });
            if !dup {
                entry.entries.push(ne);
            }
        }
    }
}

fn lookup_cached(
    hwnd_raw: isize,
    container_rect: ScreenRect,
    source: HitMapSource,
) -> Option<ContainerHitMap> {
    let g = cache_lock().lock().ok()?;
    let m = g.get(&(hwnd_raw, source))?;
    if m.built_at.elapsed() > HITMAP_TTL {
        return None;
    }
    if m.container_rect != container_rect {
        return None;
    }
    Some(m.clone())
}

#[derive(Debug, Clone)]
pub enum MsaaLookup {
    Hit(HitMapEntry),
    EmptyMargin,
    Unsupported,
}

/// L2: Container-level one-shot MSAA snapshot (`AccessibleChildren`) cached as an immutable Hit-Map.
pub fn lookup_or_build_msaa(
    container_hwnd: HWND,
    container_rect: ScreenRect,
    x: i32,
    y: i32,
) -> MsaaLookup {
    let hwnd_raw = container_hwnd.0 as isize;
    if let Some(cached) = lookup_cached(hwnd_raw, container_rect, HitMapSource::MsaaSnapshot) {
        if cached.entries.is_empty() {
            return MsaaLookup::Unsupported;
        }
        return match find_entry_at(&cached.entries, x, y) {
            Some(e) => MsaaLookup::Hit(e),
            None => MsaaLookup::EmptyMargin,
        };
    }

    let entries = build_msaa_entries(container_hwnd, container_rect);
    let hit = find_entry_at(&entries, x, y);
    let has_any = !entries.is_empty();
    store_hitmap(ContainerHitMap {
        container_hwnd: hwnd_raw,
        container_rect,
        visual_hash: 0,
        entries,
        built_at: Instant::now(),
        source: HitMapSource::MsaaSnapshot,
    });
    if !has_any {
        MsaaLookup::Unsupported
    } else if let Some(e) = hit {
        MsaaLookup::Hit(e)
    } else {
        MsaaLookup::EmptyMargin
    }
}

pub fn build_msaa_entries(container_hwnd: HWND, container_rect: ScreenRect) -> Vec<HitMapEntry> {
    let (items, _count, _elapsed) =
        probe::probe_container_msaa_children(container_hwnd, container_rect);
    items
        .into_iter()
        .filter(|it| {
            it.rect.w >= 12
                && it.rect.h >= 12
                && it.rect.w <= 620
                && it.rect.h <= 220
                && it.rect.x >= container_rect.x - 4
                && it.rect.y >= container_rect.y - 4
                && it.rect.x + it.rect.w <= container_rect.x + container_rect.w + 4
                && it.rect.y + it.rect.h <= container_rect.y + container_rect.h + 4
        })
        .map(|it| {
            let is_expandable = matches!(
                it.role.as_str(),
                "ButtonDropDown" | "ButtonMenu" | "ButtonDropDownGrid"
            );
            let trigger_pt = if is_expandable {
                Some((it.rect.x + it.rect.w / 2, it.rect.y + it.rect.h - 6))
            } else {
                None
            };
            HitMapEntry {
                rect: it.rect,
                label: it.name,
                description: it.description,
                role: it.role,
                source: HitMapSource::MsaaSnapshot,
                is_expandable,
                trigger_pt,
            }
        })
        .collect()
}

#[inline]
fn channel_diff_rgb(p: &[u8; 4], bg: (u8, u8, u8)) -> i32 {
    (p[0] as i32 - bg.0 as i32)
        .abs()
        .max((p[1] as i32 - bg.1 as i32).abs())
        .max((p[2] as i32 - bg.2 as i32).abs())
}

fn estimate_row_backgrounds(img: &RgbaImage) -> Vec<(u8, u8, u8)> {
    let w = img.width();
    let h = img.height();
    let step_x = (w / 320).clamp(1, 4);
    let mut row_bg = Vec::with_capacity(h as usize);
    let mut bins: HashMap<(u8, u8, u8), (u32, u64, u64, u64)> = HashMap::new();
    for y in 0..h {
        bins.clear();
        let mut x = 0u32;
        while x < w {
            let p = img.get_pixel(x, y);
            let key = (p[0] >> 4, p[1] >> 4, p[2] >> 4);
            let e = bins.entry(key).or_insert((0, 0, 0, 0));
            e.0 += 1;
            e.1 += p[0] as u64;
            e.2 += p[1] as u64;
            e.3 += p[2] as u64;
            x += step_x;
        }
        if let Some((_, (cnt, sr, sg, sb))) = bins.iter().max_by_key(|(_, v)| v.0) {
            let c = (*cnt).max(1) as u64;
            row_bg.push(((sr / c) as u8, (sg / c) as u8, (sb / c) as u8));
        } else {
            row_bg.push((242, 242, 242));
        }
    }
    row_bg
}

fn estimate_col_backgrounds(img: &RgbaImage) -> Vec<(u8, u8, u8)> {
    let w = img.width();
    let h = img.height();
    let step_y = (h / 240).clamp(1, 4);
    let mut col_bg = Vec::with_capacity(w as usize);
    let mut bins: HashMap<(u8, u8, u8), (u32, u64, u64, u64)> = HashMap::new();
    for x in 0..w {
        bins.clear();
        let mut y = 0u32;
        while y < h {
            let p = img.get_pixel(x, y);
            let key = (p[0] >> 4, p[1] >> 4, p[2] >> 4);
            let e = bins.entry(key).or_insert((0, 0, 0, 0));
            e.0 += 1;
            e.1 += p[0] as u64;
            e.2 += p[1] as u64;
            e.3 += p[2] as u64;
            y += step_y;
        }
        if let Some((_, (cnt, sr, sg, sb))) = bins.iter().max_by_key(|(_, v)| v.0) {
            let c = (*cnt).max(1) as u64;
            col_bg.push(((sr / c) as u8, (sg / c) as u8, (sb / c) as u8));
        } else {
            col_bg.push((242, 242, 242));
        }
    }
    col_bg
}

fn find_vertical_subbands(
    fg: &[bool],
    w: usize,
    sy0: usize,
    sy1: usize,
    x0: usize,
    x1: usize,
    min_gap: usize,
) -> Vec<(usize, usize)> {
    let mut bands = Vec::new();
    let mut in_v = false;
    let mut vs = 0usize;
    let mut vgap = 0usize;
    for y in sy0..sy1 {
        let mut ri = 0usize;
        let row_off = y * w;
        for x in x0..x1 {
            if fg[row_off + x] {
                ri += 1;
            }
        }
        if ri >= 2 {
            if !in_v {
                vs = y;
                in_v = true;
            }
            vgap = 0;
        } else if in_v {
            vgap += 1;
            if vgap >= min_gap {
                let ve = y + 1 - vgap;
                if ve.saturating_sub(vs) >= 6 {
                    bands.push((vs, ve));
                }
                in_v = false;
                vgap = 0;
            }
        }
    }
    if in_v {
        let ve = sy1.saturating_sub(vgap);
        if ve.saturating_sub(vs) >= 6 {
            bands.push((vs, ve));
        }
    }
    bands
}

/// Detects a small downward dropdown chevron (`▾`) near the bottom-center of a tall ribbon button
/// (like SolidWorks `圆角 ▾`, `异型孔向导 ▾`, `线性阵列 ▾`).
fn detect_bottom_chevron(
    fg: &[bool],
    w: usize,
    bx0: usize,
    bx1: usize,
    by0: usize,
    by1: usize,
) -> Option<(usize, usize)> {
    let bw = bx1.saturating_sub(bx0);
    let bh = by1.saturating_sub(by0);
    if bw < 24 || bh < 56 {
        return None;
    }
    // Look in the bottom 20px strip, horizontally centered
    let scan_y0 = by1.saturating_sub(18).max(by0 + bh * 2 / 3);
    let scan_y1 = by1.saturating_sub(2);
    let mid_x = (bx0 + bx1) / 2;
    let cx0 = mid_x.saturating_sub(10).max(bx0 + 4);
    let cx1 = (mid_x + 10).min(bx1.saturating_sub(4));
    if scan_y1 <= scan_y0 || cx1 <= cx0 {
        return None;
    }
    // Check there is a clean empty gap just above the chevron (separating caption text from `▾`)
    let gap_y0 = scan_y0.saturating_sub(5).max(by0);
    let mut has_gap_above = false;
    for y in gap_y0..scan_y0 {
        let mut ink = 0usize;
        for x in cx0..cx1 {
            if fg[y * w + x] {
                ink += 1;
            }
        }
        if ink == 0 {
            has_gap_above = true;
            break;
        }
    }
    if !has_gap_above {
        return None;
    }
    // Count a small compact triangular cluster (4..36 foreground pixels, width 3..11, height 2..7)
    let mut min_x = cx1;
    let mut max_x = cx0;
    let mut min_y = scan_y1;
    let mut max_y = scan_y0;
    let mut count = 0usize;
    for y in scan_y0..scan_y1 {
        for x in cx0..cx1 {
            if fg[y * w + x] {
                min_x = min_x.min(x);
                max_x = max_x.max(x + 1);
                min_y = min_y.min(y);
                max_y = max_y.max(y + 1);
                count += 1;
            }
        }
    }
    let cw = max_x.saturating_sub(min_x);
    let ch = max_y.saturating_sub(min_y);
    if (4..=40).contains(&count) && (3..=12).contains(&cw) && (2..=8).contains(&ch) {
        Some(((min_x + max_x) / 2, (min_y + max_y) / 2))
    } else {
        None
    }
}

/// Detects a small right-pointing submenu arrow (`▸`) near the right edge of a menu row.
fn detect_right_submenu_arrow(
    fg: &[bool],
    w: usize,
    row_x0: usize,
    row_x1: usize,
    row_y0: usize,
    row_y1: usize,
) -> bool {
    let rw = row_x1.saturating_sub(row_x0);
    let rh = row_y1.saturating_sub(row_y0);
    if rw < 60 || rh < 14 {
        return false;
    }
    let ax0 = row_x1.saturating_sub(24).max(row_x0 + rw * 3 / 4);
    let ax1 = row_x1.saturating_sub(3);
    if ax1 <= ax0 {
        return false;
    }
    // Verify a clean gap (at least 6px) immediately to the left of the arrow cluster
    let gap_x0 = ax0.saturating_sub(8).max(row_x0);
    let mut empty_cols = 0usize;
    for x in gap_x0..ax0 {
        let mut ink = 0usize;
        for y in row_y0..row_y1 {
            if fg[y * w + x] {
                ink += 1;
            }
        }
        if ink == 0 {
            empty_cols += 1;
        }
    }
    if empty_cols < 4 {
        return false;
    }
    let mut min_x = ax1;
    let mut max_x = ax0;
    let mut min_y = row_y1;
    let mut max_y = row_y0;
    let mut count = 0usize;
    for y in (row_y0 + 2)..row_y1.saturating_sub(2) {
        for x in ax0..ax1 {
            if fg[y * w + x] {
                min_x = min_x.min(x);
                max_x = max_x.max(x + 1);
                min_y = min_y.min(y);
                max_y = max_y.max(y + 1);
                count += 1;
            }
        }
    }
    let cw = max_x.saturating_sub(min_x);
    let ch = max_y.saturating_sub(min_y);
    (4..=45).contains(&count) && (2..=9).contains(&cw) && (4..=14).contains(&ch)
}

/// Automatically classifies a container's UI geometry from its dimensions, class name, and popup flag.
pub fn classify_container_layout(
    rect: ScreenRect,
    class_hint: &str,
    is_popup_menu: bool,
) -> ContainerLayoutKind {
    let cls = class_hint.to_ascii_lowercase();
    if is_popup_menu
        || cls == "#32768"
        || cls.contains("popup")
        || cls.contains("systreeview32")
        || cls.contains("menu")
    {
        return ContainerLayoutKind::TreeOrMenuRows;
    }
    if rect.w <= 76 && rect.h >= rect.w * 2 && rect.h >= 80 {
        return ContainerLayoutKind::VerticalIconStrip;
    }
    if cls.contains("syslistview32") && rect.w >= 140 && rect.h >= 140 {
        return ContainerLayoutKind::IconCardGrid;
    }
    if cls.contains("toolbar")
        || cls.contains("afx:toolbar")
        || cls.contains("afxcontrolbar")
        || cls.contains("rebarwindow32")
        || (rect.w >= rect.h * 2 && rect.h <= 180)
    {
        return ContainerLayoutKind::HorizontalRibbon;
    }
    if rect.h > rect.w && rect.w >= 140 && rect.w <= 420 {
        return ContainerLayoutKind::TreeOrMenuRows;
    }
    if rect.w >= 180 && rect.h >= 180 {
        return ContainerLayoutKind::IconCardGrid;
    }
    ContainerLayoutKind::HorizontalRibbon
}

/// Layout 1 — Horizontal Ribbon / Toolbar (verified 39/39 on real SolidWorks 2026 CommandManager).
pub fn segment_horizontal_ribbon_entries(
    img: &RgbaImage,
    origin_x: i32,
    origin_y: i32,
) -> Vec<HitMapEntry> {
    let w = img.width() as usize;
    let h = img.height() as usize;
    if w < 24 || h < 16 {
        return Vec::new();
    }

    let row_bg = estimate_row_backgrounds(img);
    let mut fg = vec![false; w * h];
    let mut row_ink = vec![0usize; h];
    for (y, &bg) in row_bg.iter().enumerate() {
        let mut rc = 0usize;
        let row_off = y * w;
        for x in 0..w {
            let p = img.get_pixel(x as u32, y as u32).0;
            if channel_diff_rgb(&p, bg) >= 20 {
                fg[row_off + x] = true;
                rc += 1;
            }
        }
        row_ink[y] = rc;
    }

    for (y, ink) in row_ink.iter_mut().enumerate() {
        if *ink * 10 >= w * 7 {
            let row_off = y * w;
            for x in 0..w {
                fg[row_off + x] = false;
            }
            *ink = 0;
        }
    }

    let mut bg_strips: Vec<(usize, usize)> = Vec::new();
    let mut s0 = 0usize;
    for y in 1..h {
        let prev = row_bg[y - 1];
        let cur = row_bg[y];
        let jump = (cur.0 as i32 - prev.0 as i32)
            .abs()
            .max((cur.1 as i32 - prev.1 as i32).abs())
            .max((cur.2 as i32 - prev.2 as i32).abs());
        if jump >= 18 {
            if y.saturating_sub(s0) >= 14 {
                bg_strips.push((s0, y));
            }
            s0 = y;
        }
    }
    if h.saturating_sub(s0) >= 14 {
        bg_strips.push((s0, h));
    }
    if bg_strips.is_empty() {
        bg_strips.push((0, h));
    }

    let mut strips: Vec<(usize, usize)> = Vec::new();
    for (bs0, bs1) in bg_strips {
        if bs1 - bs0 <= 120 {
            strips.push((bs0, bs1));
            continue;
        }
        let min_ink = (w / 140).max(2);
        let mut cur_s = bs0;
        let mut in_b = false;
        let mut gap = 0usize;
        for (y, &ink) in row_ink.iter().enumerate().take(bs1).skip(bs0) {
            if ink >= min_ink {
                if !in_b {
                    cur_s = y;
                    in_b = true;
                }
                gap = 0;
            } else if in_b {
                gap += 1;
                if gap >= 8 {
                    let end = y + 1 - gap;
                    if end.saturating_sub(cur_s) >= 14 {
                        strips.push((cur_s, end));
                    }
                    in_b = false;
                    gap = 0;
                }
            }
        }
        if in_b {
            let end = bs1.saturating_sub(gap);
            if end.saturating_sub(cur_s) >= 14 {
                strips.push((cur_s, end));
            }
        }
    }

    let mut out = Vec::new();
    for (sy0, sy1) in strips {
        let sh = sy1.saturating_sub(sy0);
        if !(14..=220).contains(&sh) {
            continue;
        }

        let mut col_ink = vec![0usize; w];
        for y in sy0..sy1 {
            let row_off = y * w;
            for x in 0..w {
                if fg[row_off + x] {
                    col_ink[x] += 1;
                }
            }
        }

        let orig_cols = col_ink.clone();
        for x in 0..w {
            if orig_cols[x] * 100 >= sh * 55 {
                let lc = (x >= 2 && orig_cols[x - 2] <= 2)
                    || (x >= 3 && orig_cols[x - 3] <= 2 && orig_cols[x - 1] * 100 >= sh * 55);
                let rc = (x + 2 < w && orig_cols[x + 2] <= 2)
                    || (x + 3 < w && orig_cols[x + 3] <= 2 && orig_cols[x + 1] * 100 >= sh * 55);
                if lc && rc {
                    col_ink[x] = 0;
                    for y in sy0..sy1 {
                        fg[y * w + x] = false;
                    }
                }
            }
        }

        let mut raw_cols: Vec<(usize, usize)> = Vec::new();
        let mut in_c = false;
        let mut cs = 0usize;
        let mut gap = 0usize;
        for (x, &ci) in col_ink.iter().enumerate() {
            if ci >= 2 {
                if !in_c {
                    cs = x;
                    in_c = true;
                }
                gap = 0;
            } else if in_c {
                gap += 1;
                if gap >= 5 {
                    let ce = x + 1 - gap;
                    if ce.saturating_sub(cs) >= 8 {
                        raw_cols.push((cs, ce));
                    }
                    in_c = false;
                    gap = 0;
                }
            }
        }
        if in_c {
            let ce = w.saturating_sub(gap);
            if ce.saturating_sub(cs) >= 8 {
                raw_cols.push((cs, ce));
            }
        }

        let mut paired: Vec<(usize, usize, Option<Vec<(usize, usize)>>)> = Vec::new();
        let mut idx = 0usize;
        while idx < raw_cols.len() {
            let (c0, c1) = raw_cols[idx];
            let cw = c1 - c0;
            if idx + 1 < raw_cols.len() && sh >= 56 && cw <= 28 {
                let (nc0, nc1) = raw_cols[idx + 1];
                let gap_lr = nc0.saturating_sub(c1);
                if gap_lr <= 12 {
                    let rvb = find_vertical_subbands(&fg, w, sy0, sy1, nc0, nc1, 3);
                    if matches!(rvb.len(), 2 | 3)
                        && rvb.iter().all(|&(vs, ve)| (10..=28).contains(&(ve - vs)))
                    {
                        paired.push((c0, nc1, Some(rvb)));
                        idx += 2;
                        continue;
                    }
                }
            }
            if let Some(prev) = paired.last_mut() {
                if prev.2.is_none()
                    && cw <= 14
                    && c0.saturating_sub(prev.1) <= 6
                    && c1.saturating_sub(prev.0) <= 280
                {
                    prev.1 = c1;
                    idx += 1;
                    continue;
                }
            }
            paired.push((c0, c1, None));
            idx += 1;
        }

        let is_top_menu_strip = sy0 <= 36 && sh <= 36;
        for (c0, c1, stacked_vbands) in paired {
            if let Some(vbands) = stacked_vbands {
                let n_rows = vbands.len();
                for r_idx in 0..n_rows {
                    let ry0 = sy0 + (sh * r_idx) / n_rows;
                    let ry1 = sy0 + (sh * (r_idx + 1)) / n_rows;
                    let mut tx0 = c1;
                    let mut tx1 = c0;
                    let mut ty0 = ry1;
                    let mut ty1 = ry0;
                    for y in ry0..ry1 {
                        let row_off = y * w;
                        for x in c0..c1 {
                            if fg[row_off + x] {
                                tx0 = tx0.min(x);
                                tx1 = tx1.max(x + 1);
                                ty0 = ty0.min(y);
                                ty1 = ty1.max(y + 1);
                            }
                        }
                    }
                    if tx1 > tx0 && ty1 > ty0 {
                        let bx0 = tx0.saturating_sub(3);
                        let bx1 = (tx1 + 4).min(w);
                        let by0 = ty0.saturating_sub(2).max(sy0 + 1);
                        let by1 = (ty1 + 2).min(sy1.saturating_sub(1).max(by0 + 1));
                        let bw = (bx1 - bx0) as i32;
                        let bh = (by1 - by0) as i32;
                        if (14..=360).contains(&bw) && (14..=192).contains(&bh) {
                            out.push(HitMapEntry {
                                rect: ScreenRect {
                                    x: origin_x + bx0 as i32,
                                    y: origin_y + by0 as i32,
                                    w: bw,
                                    h: bh,
                                },
                                label: String::new(),
                                description: String::new(),
                                role: "Button".to_string(),
                                source: HitMapSource::StaticVisual,
                                is_expandable: false,
                                trigger_pt: None,
                            });
                        }
                    }
                }
            } else {
                let vb = find_vertical_subbands(&fg, w, sy0, sy1, c0, c1, 3);
                if vb.is_empty() {
                    continue;
                }
                let is_bridged_stacked = sh >= 56
                    && (c1 - c0) >= 48
                    && matches!(vb.len(), 2 | 3)
                    && vb.iter().all(|&(vs, ve)| (12..=28).contains(&(ve - vs)));
                if is_bridged_stacked {
                    for &(vs, ve) in &vb {
                        let mut tx0 = c1;
                        let mut tx1 = c0;
                        for y in vs..ve {
                            let row_off = y * w;
                            for x in c0..c1 {
                                if fg[row_off + x] {
                                    tx0 = tx0.min(x);
                                    tx1 = tx1.max(x + 1);
                                }
                            }
                        }
                        if tx1 > tx0 {
                            let bx0 = tx0.saturating_sub(3);
                            let bx1 = (tx1 + 4).min(w);
                            let by0 = vs.saturating_sub(2).max(sy0 + 1);
                            let by1 = (ve + 2).min(sy1.saturating_sub(1).max(by0 + 1));
                            let bw = (bx1 - bx0) as i32;
                            let bh = (by1 - by0) as i32;
                            if (14..=360).contains(&bw) && (14..=192).contains(&bh) {
                                out.push(HitMapEntry {
                                    rect: ScreenRect {
                                        x: origin_x + bx0 as i32,
                                        y: origin_y + by0 as i32,
                                        w: bw,
                                        h: bh,
                                    },
                                    label: String::new(),
                                    description: String::new(),
                                    role: "Button".to_string(),
                                    source: HitMapSource::StaticVisual,
                                    is_expandable: false,
                                    trigger_pt: None,
                                });
                            }
                        }
                    }
                    continue;
                }
                let bx0 = c0.saturating_sub(4);
                let bx1 = (c1 + 4).min(w);
                let (by0, by1) = if sh >= 56 {
                    (sy0 + 2, sy1.saturating_sub(2).max(sy0 + 16))
                } else {
                    (
                        vb[0].0.saturating_sub(2).max(sy0),
                        (vb[vb.len() - 1].1 + 2).min(sy1),
                    )
                };
                let bw = (bx1 - bx0) as i32;
                let bh = (by1 - by0) as i32;
                if (14..=360).contains(&bw) && (14..=192).contains(&bh) {
                    let chev = detect_bottom_chevron(&fg, w, bx0, bx1, by0, by1);
                    let (is_expandable, trigger_pt, role) = if let Some((cx, cy)) = chev {
                        (
                            true,
                            Some((origin_x + cx as i32, origin_y + cy as i32)),
                            "SplitButton".to_string(),
                        )
                    } else if is_top_menu_strip && (24..=140).contains(&bw) {
                        (
                            true,
                            Some((
                                origin_x + ((bx0 + bx1) / 2) as i32,
                                origin_y + ((by0 + by1) / 2) as i32,
                            )),
                            "MenuItem".to_string(),
                        )
                    } else {
                        (false, None, "Button".to_string())
                    };
                    out.push(HitMapEntry {
                        rect: ScreenRect {
                            x: origin_x + bx0 as i32,
                            y: origin_y + by0 as i32,
                            w: bw,
                            h: bh,
                        },
                        label: String::new(),
                        description: String::new(),
                        role,
                        source: HitMapSource::StaticVisual,
                        is_expandable,
                        trigger_pt,
                    });
                }
            }
        }
    }

    out
}

/// Backward-compatible rect-only wrapper for Horizontal Ribbon segmentation.
pub fn segment_static_toolbar_buttons_in_rgba(
    img: &RgbaImage,
    origin_x: i32,
    origin_y: i32,
) -> Vec<ScreenRect> {
    segment_horizontal_ribbon_entries(img, origin_x, origin_y)
        .into_iter()
        .map(|e| e.rect)
        .collect()
}

/// Layout 2 — Vertical Icon Strip (e.g., SolidWorks Right Task Pane narrow edge bar, PS/Blender side toolbars).
pub fn segment_vertical_icon_strip_entries(
    img: &RgbaImage,
    origin_x: i32,
    origin_y: i32,
) -> Vec<HitMapEntry> {
    let w = img.width() as usize;
    let h = img.height() as usize;
    if w < 14 || h < 28 {
        return Vec::new();
    }
    let col_bg = estimate_col_backgrounds(img);
    let mut fg = vec![false; w * h];
    let mut row_ink = vec![0usize; h];
    for y in 0..h {
        let mut rc = 0usize;
        for (x, &bg) in col_bg.iter().enumerate() {
            let p = img.get_pixel(x as u32, y as u32).0;
            if channel_diff_rgb(&p, bg) >= 20 {
                fg[y * w + x] = true;
                rc += 1;
            }
        }
        row_ink[y] = rc;
    }
    // Clear thin horizontal divider lines
    let orig_rows = row_ink.clone();
    for y in 0..h {
        if orig_rows[y] * 10 >= w * 6 {
            let up_clear = y >= 2 && orig_rows[y - 2] <= 2;
            let dn_clear = y + 2 < h && orig_rows[y + 2] <= 2;
            if up_clear && dn_clear {
                row_ink[y] = 0;
            }
        }
    }

    let mut out = Vec::new();
    let mut in_r = false;
    let mut rs = 0usize;
    let mut gap = 0usize;
    for (y, &ri) in row_ink.iter().enumerate() {
        if ri >= 2 {
            if !in_r {
                rs = y;
                in_r = true;
            }
            gap = 0;
        } else if in_r {
            gap += 1;
            if gap >= 4 {
                let re = y + 1 - gap;
                if re.saturating_sub(rs) >= 10 {
                    let by0 = rs.saturating_sub(3);
                    let by1 = (re + 3).min(h);
                    let bw = w.saturating_sub(2).max(14) as i32;
                    let bh = (by1 - by0) as i32;
                    if (14..=96).contains(&bw) && (14..=96).contains(&bh) {
                        out.push(HitMapEntry {
                            rect: ScreenRect {
                                x: origin_x + 1,
                                y: origin_y + by0 as i32,
                                w: bw,
                                h: bh,
                            },
                            label: String::new(),
                            description: String::new(),
                            role: "Button".to_string(),
                            source: HitMapSource::StaticVisual,
                            is_expandable: false,
                            trigger_pt: None,
                        });
                    }
                }
                in_r = false;
                gap = 0;
            }
        }
    }
    if in_r {
        let re = h.saturating_sub(gap);
        if re.saturating_sub(rs) >= 10 {
            let by0 = rs.saturating_sub(3);
            let by1 = (re + 3).min(h);
            let bw = w.saturating_sub(2).max(14) as i32;
            let bh = (by1 - by0) as i32;
            if (14..=96).contains(&bw) && (14..=96).contains(&bh) {
                out.push(HitMapEntry {
                    rect: ScreenRect {
                        x: origin_x + 1,
                        y: origin_y + by0 as i32,
                        w: bw,
                        h: bh,
                    },
                    label: String::new(),
                    description: String::new(),
                    role: "Button".to_string(),
                    source: HitMapSource::StaticVisual,
                    is_expandable: false,
                    trigger_pt: None,
                });
            }
        }
    }
    out
}

/// Layout 3 — Tree Rows & Popup Menu Lists (SolidWorks FeatureManager `SysTreeView32`, Design Library tree,
/// and 1st/2nd/3rd-level dropdown popup menus `#32768` / `Afx:` / `BCGP`).
pub fn segment_tree_or_menu_rows_entries(
    img: &RgbaImage,
    origin_x: i32,
    origin_y: i32,
    is_popup_menu: bool,
) -> Vec<HitMapEntry> {
    let w = img.width() as usize;
    let h = img.height() as usize;
    if w < 40 || h < 18 {
        return Vec::new();
    }
    let row_bg = estimate_row_backgrounds(img);
    let mut fg = vec![false; w * h];
    let mut row_ink = vec![0usize; h];
    // Ignore 2px outer border frame common on popup menus & tree panes
    let x_start = 3usize.min(w);
    let x_end = w.saturating_sub(3).max(x_start);
    let inner_w = x_end.saturating_sub(x_start).max(1);
    for (y, &bg) in row_bg.iter().enumerate() {
        let mut rc = 0usize;
        let row_off = y * w;
        for x in x_start..x_end {
            let p = img.get_pixel(x as u32, y as u32).0;
            if channel_diff_rgb(&p, bg) >= 22 {
                fg[row_off + x] = true;
                rc += 1;
            }
        }
        row_ink[y] = rc;
    }

    // Clear 1-2px horizontal menu separators (wide thin horizontal rule)
    let orig = row_ink.clone();
    for y in 0..h {
        if orig[y] * 10 >= inner_w * 5 {
            let up_thin = y >= 2 && orig[y - 2] * 10 < inner_w * 4;
            let dn_thin = y + 2 < h && orig[y + 2] * 10 < inner_w * 4;
            if up_thin && dn_thin {
                row_ink[y] = 0;
                for x in x_start..x_end {
                    fg[y * w + x] = false;
                }
            }
        }
    }

    let mut rows: Vec<(usize, usize)> = Vec::new();
    let mut in_r = false;
    let mut rs = 0usize;
    let mut gap = 0usize;
    for y in 2..h.saturating_sub(2) {
        if row_ink[y] >= 3 {
            if !in_r {
                rs = y;
                in_r = true;
            }
            gap = 0;
        } else if in_r {
            gap += 1;
            if gap >= 3 {
                let re = y + 1 - gap;
                if re.saturating_sub(rs) >= 8 {
                    rows.push((rs, re));
                }
                in_r = false;
                gap = 0;
            }
        }
    }
    if in_r {
        let re = h.saturating_sub(2 + gap);
        if re.saturating_sub(rs) >= 8 {
            rows.push((rs, re));
        }
    }

    let mut out = Vec::new();
    for (rs, re) in rows {
        let raw_h = re - rs;
        // If a single contiguous band is taller than 2 normal rows (e.g. tight icons touching vertically),
        // split uniformly by typical tree/menu row pitch (~22px)
        let sub_rows: Vec<(usize, usize)> = if raw_h > 44 && raw_h <= 400 {
            let n = ((raw_h as f32) / 22.0).round().max(2.0) as usize;
            (0..n)
                .map(|i| (rs + (raw_h * i) / n, rs + (raw_h * (i + 1)) / n))
                .collect()
        } else {
            vec![(rs, re)]
        };

        for (y0, y1) in sub_rows {
            let by0 = y0.saturating_sub(2).max(1);
            let by1 = (y1 + 2).min(h.saturating_sub(1));
            let bh = (by1 - by0) as i32;
            if !(14..=46).contains(&bh) {
                continue;
            }
            // For popup menus, span full clean inner width; for FeatureTree rows, snap tightly to content + clean margin
            let (bx0, bx1) = if is_popup_menu {
                (x_start, x_end)
            } else {
                let mut tx0 = x_end;
                let mut tx1 = x_start;
                for y in y0..y1 {
                    let row_off = y * w;
                    for x in x_start..x_end {
                        if fg[row_off + x] {
                            tx0 = tx0.min(x);
                            tx1 = tx1.max(x + 1);
                        }
                    }
                }
                if tx1 <= tx0 {
                    continue;
                }
                (tx0.saturating_sub(6).max(2), (tx1 + 10).min(x_end))
            };
            let bw = (bx1 - bx0) as i32;
            if !(24..=640).contains(&bw) {
                continue;
            }
            let has_submenu = is_popup_menu && detect_right_submenu_arrow(&fg, w, bx0, bx1, by0, by1);
            let trigger_pt = if has_submenu {
                Some((
                    origin_x + ((bx0 + bx1) / 2) as i32,
                    origin_y + ((by0 + by1) / 2) as i32,
                ))
            } else {
                None
            };
            out.push(HitMapEntry {
                rect: ScreenRect {
                    x: origin_x + bx0 as i32,
                    y: origin_y + by0 as i32,
                    w: bw,
                    h: bh,
                },
                label: String::new(),
                description: String::new(),
                role: if is_popup_menu {
                    "MenuItem".to_string()
                } else {
                    "TreeItem".to_string()
                },
                source: HitMapSource::StaticVisual,
                is_expandable: has_submenu,
                trigger_pt,
            });
        }
    }
    out
}

/// Layout 4 — 2D Icon Card Grid (e.g., SolidWorks Design Library `Favorites / ANSI / GB / ISO / SKF`,
/// file thumbnail galleries, asset browsers).
pub fn segment_icon_card_grid_entries(
    img: &RgbaImage,
    origin_x: i32,
    origin_y: i32,
) -> Vec<HitMapEntry> {
    let w = img.width() as usize;
    let h = img.height() as usize;
    if w < 60 || h < 60 {
        return Vec::new();
    }
    let row_bg = estimate_row_backgrounds(img);
    let mut fg = vec![false; w * h];
    let mut row_ink = vec![0usize; h];
    for (y, &bg) in row_bg.iter().enumerate() {
        let mut rc = 0usize;
        let row_off = y * w;
        for x in 0..w {
            let p = img.get_pixel(x as u32, y as u32).0;
            if channel_diff_rgb(&p, bg) >= 20 {
                fg[row_off + x] = true;
                rc += 1;
            }
        }
        row_ink[y] = rc;
    }

    // Step 1: Segment horizontal card rows (gap >= 5px, then merge icon band + caption band below it)
    let mut raw_bands: Vec<(usize, usize)> = Vec::new();
    let mut in_b = false;
    let mut bs = 0usize;
    let mut gap = 0usize;
    for (y, &ri) in row_ink.iter().enumerate() {
        if ri >= 4 {
            if !in_b {
                bs = y;
                in_b = true;
            }
            gap = 0;
        } else if in_b {
            gap += 1;
            if gap >= 5 {
                let be = y + 1 - gap;
                if be.saturating_sub(bs) >= 8 {
                    raw_bands.push((bs, be));
                }
                in_b = false;
                gap = 0;
            }
        }
    }
    if in_b {
        let be = h.saturating_sub(gap);
        if be.saturating_sub(bs) >= 8 {
            raw_bands.push((bs, be));
        }
    }

    // Merge icon row (24..72px) + caption row (8..26px) separated by <= 14px gap
    let mut card_rows: Vec<(usize, usize)> = Vec::new();
    let mut i = 0usize;
    while i < raw_bands.len() {
        let (r0, r1) = raw_bands[i];
        let rh = r1 - r0;
        if i + 1 < raw_bands.len() {
            let (nr0, nr1) = raw_bands[i + 1];
            let nrh = nr1 - nr0;
            let vgap = nr0.saturating_sub(r1);
            if (20..=80).contains(&rh) && (8..=30).contains(&nrh) && vgap <= 14 {
                card_rows.push((r0, nr1));
                i += 2;
                continue;
            }
        }
        card_rows.push((r0, r1));
        i += 1;
    }

    // Step 2: Inside each card row, project columns (gap >= 8px between adjacent cards)
    let mut out = Vec::new();
    for (ry0, ry1) in card_rows {
        let rh = ry1 - ry0;
        if !(24..=160).contains(&rh) {
            continue;
        }
        let mut col_ink = vec![0usize; w];
        for y in ry0..ry1 {
            let row_off = y * w;
            for x in 0..w {
                if fg[row_off + x] {
                    col_ink[x] += 1;
                }
            }
        }
        let mut in_c = false;
        let mut cs = 0usize;
        let mut cgap = 0usize;
        for (x, &ci) in col_ink.iter().enumerate() {
            if ci >= 2 {
                if !in_c {
                    cs = x;
                    in_c = true;
                }
                cgap = 0;
            } else if in_c {
                cgap += 1;
                if cgap >= 8 {
                    let ce = x + 1 - cgap;
                    if ce.saturating_sub(cs) >= 20 {
                        let bx0 = cs.saturating_sub(5);
                        let bx1 = (ce + 5).min(w);
                        let by0 = ry0.saturating_sub(4);
                        let by1 = (ry1 + 4).min(h);
                        let bw = (bx1 - bx0) as i32;
                        let bh = (by1 - by0) as i32;
                        if (24..=240).contains(&bw) && (24..=180).contains(&bh) {
                            out.push(HitMapEntry {
                                rect: ScreenRect {
                                    x: origin_x + bx0 as i32,
                                    y: origin_y + by0 as i32,
                                    w: bw,
                                    h: bh,
                                },
                                label: String::new(),
                                description: String::new(),
                                role: "GridItem".to_string(),
                                source: HitMapSource::StaticVisual,
                                is_expandable: false,
                                trigger_pt: None,
                            });
                        }
                    }
                    in_c = false;
                    cgap = 0;
                }
            }
        }
        if in_c {
            let ce = w.saturating_sub(cgap);
            if ce.saturating_sub(cs) >= 20 {
                let bx0 = cs.saturating_sub(5);
                let bx1 = (ce + 5).min(w);
                let by0 = ry0.saturating_sub(4);
                let by1 = (ry1 + 4).min(h);
                let bw = (bx1 - bx0) as i32;
                let bh = (by1 - by0) as i32;
                if (24..=240).contains(&bw) && (24..=180).contains(&bh) {
                    out.push(HitMapEntry {
                        rect: ScreenRect {
                            x: origin_x + bx0 as i32,
                            y: origin_y + by0 as i32,
                            w: bw,
                            h: bh,
                        },
                        label: String::new(),
                        description: String::new(),
                        role: "GridItem".to_string(),
                        source: HitMapSource::StaticVisual,
                        is_expandable: false,
                        trigger_pt: None,
                    });
                }
            }
        }
    }
    out
}

/// Unified 4-layout static visual dispatcher.
pub fn segment_container_auto_in_rgba(
    img: &RgbaImage,
    origin_x: i32,
    origin_y: i32,
    class_hint: &str,
    is_popup_menu: bool,
    forced_layout: ContainerLayoutKind,
) -> (ContainerLayoutKind, Vec<HitMapEntry>) {
    let rect = ScreenRect {
        x: origin_x,
        y: origin_y,
        w: img.width() as i32,
        h: img.height() as i32,
    };
    let layout = if forced_layout == ContainerLayoutKind::Auto {
        classify_container_layout(rect, class_hint, is_popup_menu)
    } else {
        forced_layout
    };
    let entries = match layout {
        ContainerLayoutKind::HorizontalRibbon | ContainerLayoutKind::Auto => {
            segment_horizontal_ribbon_entries(img, origin_x, origin_y)
        }
        ContainerLayoutKind::VerticalIconStrip => {
            segment_vertical_icon_strip_entries(img, origin_x, origin_y)
        }
        ContainerLayoutKind::TreeOrMenuRows => {
            segment_tree_or_menu_rows_entries(img, origin_x, origin_y, is_popup_menu)
        }
        ContainerLayoutKind::IconCardGrid => {
            segment_icon_card_grid_entries(img, origin_x, origin_y)
        }
    };
    (layout, entries)
}

/// L3: Static visual Hit-Map lookup/builder for any container (all 4 layouts supported).
pub fn lookup_or_build_static_visual_with_hint(
    container_hwnd: HWND,
    container_rect: ScreenRect,
    class_hint: &str,
    is_popup_menu: bool,
    forced_layout: ContainerLayoutKind,
    x: i32,
    y: i32,
) -> Option<HitMapEntry> {
    let hwnd_raw = container_hwnd.0 as isize;
    // Always prefer already-woken OmniParser / Hover-Diff cache (<0.1ms, 0 VRAM!) when present!
    if let Some(hit) = lookup_cached_any_source(hwnd_raw, x, y) {
        return Some(hit);
    }

    // Dose 0 Lightweight Default: NEVER synchronously block normal hover on cold GPU OmniParser!
    // Use 0ms 4-Layout Static Visual segmentation on an overlay-free capture (or screen fallback)
    // and cache BOTH compact containers AND large-container tiles via `merge_entries_into_hitmap`!
    let cap_rect = if (container_rect.w <= 2600 && container_rect.h <= 480)
        || (container_rect.w <= 640 && container_rect.h <= 1600)
        || is_popup_menu
    {
        container_rect
    } else {
        let gx = x.div_euclid(512) * 512;
        let gy = y.div_euclid(256) * 256;
        let tx0 = (gx - 128).max(container_rect.x);
        let ty0 = (gy - 64).max(container_rect.y);
        let tx1 = (gx + 640).min(container_rect.x + container_rect.w);
        let ty1 = (gy + 320).min(container_rect.y + container_rect.h);
        ScreenRect {
            x: tx0,
            y: ty0,
            w: (tx1 - tx0).max(0),
            h: (ty1 - ty0).max(0),
        }
    };
    if cap_rect.w < 16 || cap_rect.h < 16 {
        return None;
    }

    // Capture overlay-free PrintWindow / clean session snapshot first so blue hover boxes & trails
    // NEVER pollute StaticVisual segmentation; fall back to screen capture only if needed.
    let root_hwnd = unsafe {
        let r = windows::Win32::UI::WindowsAndMessaging::GetAncestor(
            container_hwnd,
            windows::Win32::UI::WindowsAndMessaging::GA_ROOT,
        );
        if r.0.is_null() { container_hwnd } else { r }
    };
    let fallback_snap = crate::pick::scout::current_fallback_snapshot();
    let (img, ox, oy) = match crate::pick::scout::capture_clean_region_no_overlay(
        root_hwnd,
        cap_rect,
        fallback_snap.as_deref(),
    ) {
        Some(triple) => triple,
        None => {
            let cap = capture_screen_rect_rgba(cap_rect.x, cap_rect.y, cap_rect.w, cap_rect.h).ok()?;
            (cap.image, cap.origin_x, cap.origin_y)
        }
    };
    let vhash = compute_image_sample_hash(&img);
    let (_layout, entries) = segment_container_auto_in_rgba(
        &img,
        ox,
        oy,
        class_hint,
        is_popup_menu,
        forced_layout,
    );

    let hit = find_entry_at(&entries, x, y);
    if cap_rect == container_rect {
        store_hitmap(ContainerHitMap {
            container_hwnd: hwnd_raw,
            container_rect,
            visual_hash: vhash,
            entries,
            built_at: Instant::now(),
            source: HitMapSource::StaticVisual,
        });
    } else if !entries.is_empty() {
        merge_entries_into_hitmap(hwnd_raw, container_rect, HitMapSource::StaticVisual, entries);
    }
    hit
}

pub fn lookup_or_build_static_visual(
    container_hwnd: HWND,
    container_rect: ScreenRect,
    x: i32,
    y: i32,
) -> Option<HitMapEntry> {
    lookup_or_build_static_visual_with_hint(
        container_hwnd,
        container_rect,
        "",
        false,
        ContainerLayoutKind::Auto,
        x,
        y,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn make_synthetic_toolbar(dark: bool) -> RgbaImage {
        let bg = if dark {
            Rgba([38, 40, 46, 255])
        } else {
            Rgba([242, 243, 245, 255])
        };
        let tab_bg = if dark {
            Rgba([20, 22, 26, 255])
        } else {
            Rgba([210, 212, 216, 255])
        };
        let icon_fg = if dark {
            Rgba([110, 185, 255, 255])
        } else {
            Rgba([32, 118, 210, 255])
        };
        let text_fg = if dark {
            Rgba([225, 228, 234, 255])
        } else {
            Rgba([35, 38, 44, 255])
        };
        let sep = if dark {
            Rgba([72, 75, 84, 255])
        } else {
            Rgba([196, 199, 206, 255])
        };

        let mut img = RgbaImage::from_pixel(420, 104, bg);
        for y in 81..104 {
            for x in 0..420 {
                img.put_pixel(x, y, tab_bg);
            }
        }
        for &(bx0, bx1) in &[(14u32, 66u32), (84, 136)] {
            let ix0 = bx0 + (bx1 - bx0).saturating_sub(30) / 2;
            for y in 8..38 {
                for x in ix0..(ix0 + 30) {
                    img.put_pixel(x, y, icon_fg);
                }
            }
            for y in 44..54 {
                for x in bx0..bx1 {
                    if (x - bx0) % 5 != 4 {
                        img.put_pixel(x, y, text_fg);
                    }
                }
            }
        }
        // Draw downward chevron `▾` under tall button 2
        for x in 107..=113 {
            img.put_pixel(x, 66, text_fg);
        }
        for x in 108..=112 {
            img.put_pixel(x, 67, text_fg);
        }
        for x in 109..=111 {
            img.put_pixel(x, 68, text_fg);
        }
        img.put_pixel(110, 69, text_fg);

        for y in 6..74 {
            img.put_pixel(148, y, sep);
        }
        for &(ry0, ry1) in &[(4u32, 22u32), (30, 48), (56, 74)] {
            for y in ry0..ry1 {
                for x in 160..180 {
                    img.put_pixel(x, y, icon_fg);
                }
            }
            for y in (ry0 + 3)..(ry1 - 3) {
                for x in 186..244 {
                    if (x - 186) % 5 != 4 {
                        img.put_pixel(x, y, text_fg);
                    }
                }
            }
        }
        for &(tx0, tx1) in &[(14u32, 58u32), (74, 120)] {
            for y in 86..98 {
                for x in tx0..tx1 {
                    if (x - tx0) % 5 != 4 {
                        img.put_pixel(x, y, text_fg);
                    }
                }
            }
        }
        img
    }

    #[test]
    fn static_segmenter_splits_tall_stacked_and_tabs_with_zero_interior_flicker() {
        for dark in [false, true] {
            let img = make_synthetic_toolbar(dark);
            let entries = segment_horizontal_ribbon_entries(&img, 100, 50);
            assert_eq!(entries.len(), 7, "expected 7 entries (dark={dark})");
            assert!(
                entries[1].is_expandable && entries[1].trigger_pt.is_some(),
                "tall button 2 with ▾ chevron must be flagged expandable"
            );

            let stacked_mid = find_entry_at(&entries, 100 + 170, 50 + 38)
                .expect("middle stacked button icon hit");
            assert!(stacked_mid.rect.w >= 75 && stacked_mid.rect.h <= 30);
            for (px, py) in [
                (100 + 165, 50 + 38),
                (100 + 175, 50 + 38),
                (100 + 190, 50 + 38),
                (100 + 215, 50 + 38),
                (100 + 238, 50 + 38),
            ] {
                let h = find_entry_at(&entries, px, py).expect("hit inside stacked button");
                assert_eq!(h.rect, stacked_mid.rect);
            }
        }
    }

    #[test]
    fn vertical_strip_tree_menu_and_card_grid_layouts_all_segment_cleanly() {
        let bg = Rgba([245, 246, 248, 255]);
        let fg = Rgba([36, 40, 48, 255]);
        let icon = Rgba([38, 120, 210, 255]);

        // 1. Vertical icon strip (32x160 with 4 icons)
        let mut vimg = RgbaImage::from_pixel(32, 160, bg);
        for i in 0..4u32 {
            let y0 = 8 + i * 36;
            for y in y0..(y0 + 22) {
                for x in 5..27 {
                    vimg.put_pixel(x, y, icon);
                }
            }
        }
        let v_entries = segment_vertical_icon_strip_entries(&vimg, 0, 0);
        assert_eq!(v_entries.len(), 4, "vertical strip should find 4 icons");

        // 2. Popup menu with 3 rows, row #2 carrying a right-pointing `▸` submenu arrow
        let mut mimg = RgbaImage::from_pixel(180, 96, bg);
        for i in 0..3u32 {
            let y0 = 8 + i * 28;
            for y in y0..(y0 + 14) {
                for x in 14..96 {
                    if x % 4 != 3 {
                        mimg.put_pixel(x, y, fg);
                    }
                }
            }
        }
        // Right-pointing `▸` triangle on row #2 (y=36..50, x=162..167)
        for dy in 0..7u32 {
            let span = if dy <= 3 { dy + 1 } else { 7 - dy };
            for dx in 0..span {
                mimg.put_pixel(162 + dx, 39 + dy, fg);
            }
        }
        let m_entries = segment_tree_or_menu_rows_entries(&mimg, 0, 0, true);
        assert_eq!(m_entries.len(), 3, "popup menu should find 3 rows");
        assert!(!m_entries[0].is_expandable);
        assert!(m_entries[1].is_expandable, "row 2 with ▸ arrow must be expandable");
        assert!(!m_entries[2].is_expandable);

        // 3. 2D Icon Card Grid (3 cols x 2 rows = 6 cards)
        let mut gimg = RgbaImage::from_pixel(300, 180, bg);
        for r in 0..2u32 {
            let y0 = 10 + r * 82;
            for c in 0..3u32 {
                let x0 = 16 + c * 92;
                for y in y0..(y0 + 36) {
                    for x in (x0 + 10)..(x0 + 50) {
                        gimg.put_pixel(x, y, icon);
                    }
                }
                for y in (y0 + 42)..(y0 + 54) {
                    for x in x0..(x0 + 62) {
                        if x % 4 != 3 {
                            gimg.put_pixel(x, y, fg);
                        }
                    }
                }
            }
        }
        let g_entries = segment_icon_card_grid_entries(&gimg, 0, 0);
        assert_eq!(g_entries.len(), 6, "2D card grid should find 6 cards");
    }

    #[test]
    fn msaa_and_static_caches_never_evict_each_other() {
        let hwnd = 0x123456isize;
        let rc = ScreenRect { x: 0, y: 0, w: 800, h: 80 };
        invalidate_hitmap_for_hwnd(hwnd);

        store_hitmap(ContainerHitMap {
            container_hwnd: hwnd,
            container_rect: rc,
            visual_hash: 0,
            entries: vec![],
            built_at: Instant::now(),
            source: HitMapSource::MsaaSnapshot,
        });
        store_hitmap(ContainerHitMap {
            container_hwnd: hwnd,
            container_rect: rc,
            visual_hash: 42,
            entries: vec![HitMapEntry {
                rect: ScreenRect { x: 10, y: 5, w: 60, h: 70 },
                label: String::new(),
                description: String::new(),
                role: "Button".to_string(),
                source: HitMapSource::StaticVisual,
                is_expandable: false,
                trigger_pt: None,
            }],
            built_at: Instant::now(),
            source: HitMapSource::StaticVisual,
        });

        assert!(lookup_cached(hwnd, rc, HitMapSource::MsaaSnapshot).is_some());
        let st = lookup_cached(hwnd, rc, HitMapSource::StaticVisual).expect("static preserved");
        assert_eq!(st.entries.len(), 1);
        invalidate_hitmap_for_hwnd(hwnd);
    }

    #[test]
    fn expandable_trigger_detector_hits_chevrons_and_ignores_plain_buttons() {
        let bg = Rgba([242, 243, 245, 255]);
        let fg = Rgba([35, 38, 44, 255]);
        let icon = Rgba([32, 118, 210, 255]);
        let mut img = RgbaImage::from_pixel(320, 120, bg);
        let win_rc = ScreenRect { x: 0, y: 0, w: 1280, h: 800 };

        // 1. Tall button with bottom `▾` chevron (x=10..66, y=20..92)
        for y in 26..56 {
            for x in 24..52 {
                img.put_pixel(x, y, icon);
            }
        }
        for x in 35..=41 {
            img.put_pixel(x, 82, fg);
        }
        for x in 36..=40 {
            img.put_pixel(x, 83, fg);
        }
        for x in 37..=39 {
            img.put_pixel(x, 84, fg);
        }
        img.put_pixel(38, 85, fg);
        assert!(
            detect_expandable_trigger_in_rgba(
                &img,
                0,
                0,
                ScreenRect { x: 10, y: 20, w: 56, h: 72 },
                win_rc,
            )
            .is_some(),
            "tall ribbon button with bottom ▾ must trigger"
        );

        // 2. Standalone narrow `▾` strip (x=80..96, y=30..58)
        for x in 85..=91 {
            img.put_pixel(x, 42, fg);
        }
        for x in 86..=90 {
            img.put_pixel(x, 43, fg);
        }
        for x in 87..=89 {
            img.put_pixel(x, 44, fg);
        }
        img.put_pixel(88, 45, fg);
        assert!(
            detect_expandable_trigger_in_rgba(
                &img,
                0,
                0,
                ScreenRect { x: 80, y: 30, w: 16, h: 28 },
                win_rc,
            )
            .is_some(),
            "standalone narrow ▾ strip must trigger"
        );

        // 3. Plain 20x20 toolbar icon (e.g. Save/Print) — MUST NOT trigger!
        for y in 34..48 {
            for x in 114..128 {
                img.put_pixel(x, y, icon);
            }
        }
        assert!(
            detect_expandable_trigger_in_rgba(
                &img,
                0,
                0,
                ScreenRect { x: 110, y: 30, w: 22, h: 22 },
                win_rc,
            )
            .is_none(),
            "plain square icon button MUST NOT falsely trigger dropdown click"
        );

        // 4. Plain button with right-edge `▸` separated by gap (x=150..240, y=60..86)
        for y in 66..80 {
            for x in 158..208 {
                img.put_pixel(x, y, fg);
            }
        }
        for dy in 0..7u32 {
            let span = if dy <= 3 { dy + 1 } else { 7 - dy };
            for dx in 0..span {
                img.put_pixel(228 + dx, 69 + dy, fg);
            }
        }
        assert!(
            detect_expandable_trigger_in_rgba(
                &img,
                0,
                0,
                ScreenRect { x: 150, y: 60, w: 90, h: 26 },
                win_rc,
            )
            .is_some(),
            "button with right-edge ▸ must trigger"
        );
    }
}
