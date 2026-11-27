use super::*;

/// CAD-style automatic Box vs Lasso trajectory classifier.
/// Straight/monotonic diagonal drags classify as `Box`; curving, hooking, reversing,
/// or looping trajectories seamlessly switch to `Lasso`.
pub fn classify_blue_gesture(points: &[(i32, i32)]) -> BlueGestureKind {
    if points.len() < 3 {
        return BlueGestureKind::Box;
    }
    let start = points[0];
    let end = *points.last().unwrap_or(&start);
    let dx_total = (end.0 - start.0) as f64;
    let dy_total = (end.1 - start.1) as f64;
    let chord = dx_total.hypot(dy_total);

    let mut arc_len = 0.0_f64;
    let (mut min_x, mut max_x) = (start.0, start.0);
    let (mut min_y, mut max_y) = (start.1, start.1);
    for w in points.windows(2) {
        let seg_dx = (w[1].0 - w[0].0) as f64;
        let seg_dy = (w[1].1 - w[0].1) as f64;
        arc_len += seg_dx.hypot(seg_dy);
        min_x = min_x.min(w[1].0);
        max_x = max_x.max(w[1].0);
        min_y = min_y.min(w[1].1);
        max_y = max_y.max(w[1].1);
    }

    if arc_len < 36.0 {
        return BlueGestureKind::Box;
    }
    if chord < 24.0 {
        // Closed loop returning near start point
        return BlueGestureKind::Lasso;
    }

    let overshoot_x_pos = max_x - start.0.max(end.0);
    let overshoot_x_neg = start.0.min(end.0) - min_x;
    let overshoot_y_pos = max_y - start.1.max(end.1);
    let overshoot_y_neg = start.1.min(end.1) - min_y;
    let max_overshoot = overshoot_x_pos
        .max(overshoot_x_neg)
        .max(overshoot_y_pos)
        .max(overshoot_y_neg);
    if max_overshoot > 26 {
        return BlueGestureKind::Lasso;
    }

    let straightness = chord / arc_len.max(1.0);
    if straightness < 0.80 {
        return BlueGestureKind::Lasso;
    }

    let mut max_perp_dev = 0.0_f64;
    for &(px, py) in &points[1..points.len() - 1] {
        let cross = ((px - start.0) as f64 * dy_total - (py - start.1) as f64 * dx_total).abs();
        let perp = cross / chord;
        if perp > max_perp_dev {
            max_perp_dev = perp;
        }
    }
    if max_perp_dev > 34.0 && (max_perp_dev / chord) > 0.24 {
        return BlueGestureKind::Lasso;
    }

    BlueGestureKind::Box
}

pub fn point_in_polygon(pt: (i32, i32), poly: &[(i32, i32)]) -> bool {
    if poly.len() < 3 {
        return false;
    }
    let (px, py) = (pt.0 as f64, pt.1 as f64);
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (xi, yi) = (poly[i].0 as f64, poly[i].1 as f64);
        let (xj, yj) = (poly[j].0 as f64, poly[j].1 as f64);
        let intersects = ((yi > py) != (yj > py))
            && (px < (xj - xi) * (py - yi) / ((yj - yi) + 1e-12) + xi);
        if intersects {
            inside = !inside;
        }
        j = i;
    }
    inside
}

pub(super) fn rect_contains_pt(r: ScreenRect, pt: (i32, i32)) -> bool {
    pt.0 >= r.x && pt.0 <= r.x + r.w && pt.1 >= r.y && pt.1 <= r.y + r.h
}

pub fn rects_intersect(a: ScreenRect, b: ScreenRect) -> bool {
    a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y
}

pub(super) fn segments_intersect(a: (i32, i32), b: (i32, i32), c: (i32, i32), d: (i32, i32)) -> bool {
    fn orient(p: (i32, i32), q: (i32, i32), r: (i32, i32)) -> i64 {
        (q.0 - p.0) as i64 * (r.1 - p.1) as i64 - (q.1 - p.1) as i64 * (r.0 - p.0) as i64
    }
    fn on_seg(p: (i32, i32), q: (i32, i32), r: (i32, i32)) -> bool {
        q.0 >= p.0.min(r.0) && q.0 <= p.0.max(r.0) && q.1 >= p.1.min(r.1) && q.1 <= p.1.max(r.1)
    }
    let o1 = orient(a, b, c);
    let o2 = orient(a, b, d);
    let o3 = orient(c, d, a);
    let o4 = orient(c, d, b);

    if (o1 > 0 && o2 < 0 || o1 < 0 && o2 > 0) && (o3 > 0 && o4 < 0 || o3 < 0 && o4 > 0) {
        return true;
    }
    (o1 == 0 && on_seg(a, c, b))
        || (o2 == 0 && on_seg(a, d, b))
        || (o3 == 0 && on_seg(c, a, d))
        || (o4 == 0 && on_seg(c, b, d))
}

pub fn segment_intersects_rect(a: (i32, i32), b: (i32, i32), r: ScreenRect) -> bool {
    if rect_contains_pt(r, a) || rect_contains_pt(r, b) {
        return true;
    }
    let seg_min_x = a.0.min(b.0);
    let seg_max_x = a.0.max(b.0);
    let seg_min_y = a.1.min(b.1);
    let seg_max_y = a.1.max(b.1);
    if seg_max_x < r.x || seg_min_x > r.x + r.w || seg_max_y < r.y || seg_min_y > r.y + r.h {
        return false;
    }
    let tl = (r.x, r.y);
    let tr = (r.x + r.w, r.y);
    let br = (r.x + r.w, r.y + r.h);
    let bl = (r.x, r.y + r.h);
    segments_intersect(a, b, tl, tr)
        || segments_intersect(a, b, tr, br)
        || segments_intersect(a, b, br, bl)
        || segments_intersect(a, b, bl, tl)
}

/// True if the freehand blue polyline touches/crosses `r` OR encloses `r` (center or any corner inside polygon).
pub fn polyline_touches_or_encloses_rect(points: &[(i32, i32)], r: ScreenRect) -> bool {
    if points.is_empty() || r.w <= 0 || r.h <= 0 {
        return false;
    }
    for w in points.windows(2) {
        if segment_intersects_rect(w[0], w[1], r) {
            return true;
        }
    }
    if points.len() >= 3 {
        let cx = r.x + r.w / 2;
        let cy = r.y + r.h / 2;
        let probe_pts = [
            (cx, cy),
            (r.x + 2, r.y + 2),
            (r.x + r.w - 2, r.y + 2),
            (r.x + r.w - 2, r.y + r.h - 2),
            (r.x + 2, r.y + r.h - 2),
        ];
        if probe_pts.iter().any(|&p| point_in_polygon(p, points)) {
            return true;
        }
    }
    false
}

pub(super) fn is_strict_foreground_for_blue(
    info: &UiaElementInfo,
    win_rect: ScreenRect,
    win_class: &str,
) -> bool {
    if !is_valid_tap_element(info) {
        return false;
    }
    // Desktop: ONLY allow actual desktop icons (ListItem), never wallpaper/shell list/pane
    if is_desktop_window_class(win_class) {
        return info.control_type == "ListItem";
    }
    if classify_hover_target(info, win_rect, win_class) != HoverTargetKind::Element {
        return false;
    }
    // Never treat giant window-filling container panels as selected foreground items
    let win_area = (win_rect.w.max(1) as i64) * (win_rect.h.max(1) as i64);
    let el_area = (info.rect.w.max(0) as i64) * (info.rect.h.max(0) as i64);
    let is_container = matches!(
        info.control_type.as_str(),
        "Pane" | "Group" | "Window" | "Document" | "Custom" | "List" | "Table" | "Tree"
    );
    if is_container && el_area * 100 >= win_area * 55 {
        return false;
    }
    true
}

pub(super) struct BlueTextRangeHit {
    text: String,
    start_pt: (i32, i32),
    end_pt: (i32, i32),
    bbox: ScreenRect,
    container_info: UiaElementInfo,
}

pub(super) fn point_distance_to_rect(pt: (i32, i32), r: ScreenRect) -> i32 {
    let dx = if pt.0 < r.x {
        r.x - pt.0
    } else if pt.0 > r.x + r.w {
        pt.0 - (r.x + r.w)
    } else {
        0
    };
    let dy = if pt.1 < r.y {
        r.y - pt.1
    } else if pt.1 > r.y + r.h {
        pt.1 - (r.y + r.h)
    } else {
        0
    };
    ((dx as f64).hypot(dy as f64)) as i32
}

pub(super) fn find_text_pattern_ancestor(
    el: &UIElement,
    walker: &UITreeWalker,
) -> Option<(UIElement, UITextPattern)> {
    let mut cur = el.clone();
    for _ in 0..5 {
        if let Ok(tp) = cur.get_pattern::<UITextPattern>() {
            return Some((cur, tp));
        }
        match walker.get_parent(&cur) {
            Ok(p) => cur = p,
            Err(_) => break,
        }
    }
    None
}

pub(super) fn range_char_distance_to_pt(
    tp: &UITextPattern,
    pt: (i32, i32),
) -> Option<i32> {
    let probe = tp.get_range_from_point(UiaPoint::new(pt.0, pt.1)).ok()?;
    let enclosing = probe.get_enclosing_element().ok()?;
    let r = enclosing.get_bounding_rectangle().ok()?;
    let sr = ScreenRect {
        x: r.get_left(),
        y: r.get_top(),
        w: (r.get_right() - r.get_left()).max(1),
        h: (r.get_bottom() - r.get_top()).max(1),
    };
    Some(point_distance_to_rect(pt, sr))
}

pub(super) fn try_extract_win32_edit_char_range(
    el: &UIElement,
    tp: Option<&UITextPattern>,
    start_pt: (i32, i32),
    end_pt: (i32, i32),
) -> Option<String> {
    use uiautomation::patterns::UIValuePattern;
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::Graphics::Gdi::ScreenToClient;

    const EM_CHARFROMPOS: u32 = 0x00D7;
    // Standard Win32 Edit / WinForms TextBox passes packed MAKELPARAM(x, y) by value (safe cross-process).
    // RichEdit expects a pointer in LPARAM which is NOT marshaled cross-process, so skip it here.
    let class_lower = el
        .get_classname()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if class_lower.contains("richedit") || !class_lower.contains("edit") {
        return None;
    }
    let raw_hwnd: isize = el.get_native_window_handle().ok()?.into();
    if raw_hwnd == 0 {
        return None;
    }
    let hwnd = HWND(raw_hwnd as *mut std::ffi::c_void);
    let mut c1 = POINT {
        x: start_pt.0,
        y: start_pt.1,
    };
    let mut c2 = POINT {
        x: end_pt.0,
        y: end_pt.1,
    };
    unsafe {
        if !ScreenToClient(hwnd, &mut c1).as_bool() || !ScreenToClient(hwnd, &mut c2).as_bool() {
            return None;
        }
    }

    let char_index_at = |pt: &POINT| -> Option<usize> {
        let lparam = ((pt.x & 0xFFFF) | ((pt.y & 0xFFFF) << 16)) as isize;
        let mut res = 0usize;
        let ok = unsafe {
            windows::Win32::UI::WindowsAndMessaging::SendMessageTimeoutW(
                hwnd,
                EM_CHARFROMPOS,
                WPARAM(0),
                LPARAM(lparam),
                windows::Win32::UI::WindowsAndMessaging::SMTO_ABORTIFHUNG
                    | windows::Win32::UI::WindowsAndMessaging::SMTO_NORMAL,
                100,
                Some(&mut res),
            )
        };
        if ok.0 == 0 {
            None
        } else {
            Some(res & 0xFFFF)
        }
    };

    let idx1 = char_index_at(&c1)?;
    let idx2 = char_index_at(&c2)?;
    let lo = idx1.min(idx2);
    let hi = idx1.max(idx2);
    if hi <= lo {
        return None;
    }

    let full = tp
        .and_then(|p| p.get_document_range().ok())
        .and_then(|d| d.get_text(32_000).ok())
        .or_else(|| {
            el.get_pattern::<UIValuePattern>()
                .ok()
                .and_then(|vp| vp.get_value().ok())
        })?;
    let utf16: Vec<u16> = full.encode_utf16().collect();
    if lo >= utf16.len() {
        return None;
    }
    let end_excl = (hi + 1).min(utf16.len());
    let sliced = String::from_utf16_lossy(&utf16[lo..end_excl])
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .trim()
        .to_string();
    if sliced.chars().count() >= 2 {
        Some(sliced)
    } else {
        None
    }
}

/// Optimized diagonal/horizontal blue text sweep:
/// 1) Uses UIA `UITextPattern::get_range_from_point(start..end)` to capture exact characters
///    from the first character touched to the last character touched across multiple lines.
/// 2) Falls back to reading-order sibling `Text` sweep when both endpoints hit sibling text rows.
pub(super) fn try_extract_blue_text_range(
    points: &[(i32, i32)],
    bbox: ScreenRect,
    root_hwnd: HWND,
    expected_pid: u32,
    win_class: &str,
) -> Option<BlueTextRangeHit> {
    if points.len() < 2 || is_desktop_window_class(win_class) {
        return None;
    }
    let start_pt = points[0];
    let end_pt = *points.last()?;
    let chord = ((end_pt.0 - start_pt.0) as f64).hypot((end_pt.1 - start_pt.1) as f64);
    if chord < 18.0 {
        return None;
    }
    // Skip closed lasso loops around icons/controls; diagonal/horizontal text sweeps are open strokes
    let mut arc_len = 0.0_f64;
    for w in points.windows(2) {
        arc_len += ((w[1].0 - w[0].0) as f64).hypot((w[1].1 - w[0].1) as f64);
    }
    if chord / arc_len.max(1.0) < 0.65 {
        return None;
    }
    if expected_pid != 0 && solidworks::is_solidworks_exe(&office::fast_exe_basename_for_pid(expected_pid)) {
        return None;
    }

    let auto = UIAutomation::new().ok()?;
    let walker = auto.get_control_view_walker().ok()?;
    let el_start = find_target_uia_element(&auto, &walker, root_hwnd, expected_pid, start_pt.0, start_pt.1)?;
    let el_end = find_target_uia_element(&auto, &walker, root_hwnd, expected_pid, end_pt.0, end_pt.1)?;

    // Reject if user clearly started/ended on an interactive non-text widget (button, checkbox, icon, etc.)
    let start_ct = format!("{:?}", el_start.get_control_type().unwrap_or(ControlType::Custom));
    let end_ct = format!("{:?}", el_end.get_control_type().unwrap_or(ControlType::Custom));
    let is_widget = |ct: &str| {
        matches!(
            ct,
            "Button"
                | "CheckBox"
                | "RadioButton"
                | "ComboBox"
                | "MenuItem"
                | "TabItem"
                | "ListItem"
                | "TreeItem"
                | "Slider"
                | "ScrollBar"
                | "Image"
        )
    };
    if is_widget(&start_ct) || is_widget(&end_ct) {
        return None;
    }

    // Tier 1: Exact character-level UIA TextPattern RangeFromPoint
    if let Some((tp_el, tp)) = find_text_pattern_ancestor(&el_start, &walker) {
        let start_near = range_char_distance_to_pt(&tp, start_pt)
            .map(|d| d <= 52)
            .unwrap_or(true);
        let end_near = range_char_distance_to_pt(&tp, end_pt)
            .map(|d| d <= 52)
            .unwrap_or(true);

        if start_near && end_near {
            let r_start_res = tp.get_range_from_point(UiaPoint::new(start_pt.0, start_pt.1));
            let r_end_res = tp.get_range_from_point(UiaPoint::new(end_pt.0, end_pt.1));
            match (r_start_res, r_end_res) {
                (Ok(r_start), Ok(r_end)) => {
                    let cmp = r_start
                        .compare_endpoints(
                            TextPatternRangeEndpoint::Start,
                            &r_end,
                            TextPatternRangeEndpoint::Start,
                        )
                        .unwrap_or(0);
                    let (first_r, last_r) = if cmp <= 0 {
                        (r_start, r_end)
                    } else {
                        (r_end, r_start)
                    };

                    let _ = first_r.move_endpoint_by_range(
                        TextPatternRangeEndpoint::End,
                        &last_r,
                        TextPatternRangeEndpoint::End,
                    );
                    // Include the final character under the end cursor tip
                    let _ = first_r.move_endpoint_by_unit(
                        TextPatternRangeEndpoint::End,
                        TextUnit::Character,
                        1,
                    );
                    if let Ok(raw_text) = first_r.get_text(8000) {
                        let cleaned = raw_text.replace("\r\n", "\n").trim().to_string();
                        if cleaned.chars().count() >= 2 {
                            let container_info = extract_uia_element_info(&tp_el, &walker, false);
                            return Some(BlueTextRangeHit {
                                text: cleaned,
                                start_pt,
                                end_pt,
                                bbox,
                                container_info,
                            });
                        }
                    }
                }
                (e1, e2) => {
                    log::debug!(
                        "[BLUE-TEXT] RangeFromPoint failed: start={:?} end={:?}",
                        e1.err(),
                        e2.err()
                    );
                }
            }
        }

        // Tier 1b (inside TextPattern provider): Win32 Edit fallback
        if let Some(text) = try_extract_win32_edit_char_range(&tp_el, Some(&tp), start_pt, end_pt) {
            let container_info = extract_uia_element_info(&tp_el, &walker, false);
            return Some(BlueTextRangeHit {
                text,
                start_pt,
                end_pt,
                bbox,
                container_info,
            });
        }
    }

    // Tier 1c: Classic Win32 Edit / WinForms TextBox (exposes ValuePattern instead of TextPattern)
    if let Some(text) = try_extract_win32_edit_char_range(&el_start, None, start_pt, end_pt) {
        let container_info = extract_uia_element_info(&el_start, &walker, false);
        return Some(BlueTextRangeHit {
            text,
            start_pt,
            end_pt,
            bbox,
            container_info,
        });
    }

    // Tier 2: Multi-line sibling `Text` sweep (for WinForms/WPF/Qt/chat UIs without TextPattern)
    if start_ct == "Text" && end_ct == "Text" {
        if let Ok(parent) = walker.get_parent(&el_start) {
            let y_min = start_pt.1.min(end_pt.1) - 6;
            let y_max = start_pt.1.max(end_pt.1) + 6;
            let mut rows: Vec<(i32, i32, String)> = Vec::new();
            if let Ok(mut child) = walker.get_first_child(&parent) {
                for _ in 0..100 {
                    let ct = format!("{:?}", child.get_control_type().unwrap_or(ControlType::Custom));
                    if ct == "Text" {
                        if let Ok(rc) = child.get_bounding_rectangle() {
                            let cy = (rc.get_top() + rc.get_bottom()) / 2;
                            if cy >= y_min && cy <= y_max {
                                let name = child.get_name().unwrap_or_default().trim().to_string();
                                if !name.is_empty() {
                                    rows.push((rc.get_top(), rc.get_left(), name));
                                }
                            }
                        }
                    }
                    match walker.get_next_sibling(&child) {
                        Ok(next) => child = next,
                        Err(_) => break,
                    }
                }
            }
            if rows.len() >= 2 {
                rows.sort_by_key(|(y, x, _)| (*y / 14, *x));
                let joined = rows
                    .into_iter()
                    .map(|(_, _, s)| s)
                    .collect::<Vec<_>>()
                    .join("\n");
                if joined.chars().count() >= 2 {
                    let container_info = extract_uia_element_info(&el_start, &walker, false);
                    return Some(BlueTextRangeHit {
                        text: joined,
                        start_pt,
                        end_pt,
                        bbox,
                        container_info,
                    });
                }
            }
        }
    }

    None
}

pub(super) fn sample_blue_foreground_elements(
    points: &[(i32, i32)],
    bbox: ScreenRect,
    gesture: BlueGestureKind,
    expected_pid: u32,
    cap: usize,
) -> Vec<UiaElementInfo> {
    let mut out: Vec<UiaElementInfo> = Vec::new();
    if expected_pid != 0 && solidworks::is_solidworks_exe(&office::fast_exe_basename_for_pid(expected_pid)) {
        return out;
    }
    let Ok(auto) = UIAutomation::new() else {
        return out;
    };
    let Ok(walker) = auto.get_control_view_walker() else {
        return out;
    };

    let mut probe_points: Vec<(i32, i32)> = Vec::with_capacity(64);
    match gesture {
        BlueGestureKind::Box => {
            let cols = 6usize;
            let rows = 6usize;
            for r in 0..rows {
                for c in 0..cols {
                    let px = bbox.x + ((bbox.w as usize) * (2 * c + 1) / (2 * cols)) as i32;
                    let py = bbox.y + ((bbox.h as usize) * (2 * r + 1) / (2 * rows)) as i32;
                    probe_points.push((px, py));
                }
            }
            probe_points.push((bbox.x + 4, bbox.y + 4));
            probe_points.push((bbox.x + bbox.w - 4, bbox.y + bbox.h - 4));
        }
        BlueGestureKind::Lasso => {
            // 1. Sample along the blue stroke polyline (every ~20px, up to 34 pts) -> elements TOUCHED
            let mut accum = 0.0_f64;
            let step = 20.0_f64;
            if let Some(&first) = points.first() {
                probe_points.push(first);
            }
            for w in points.windows(2) {
                let d = ((w[1].0 - w[0].0) as f64).hypot((w[1].1 - w[0].1) as f64);
                accum += d;
                if accum >= step {
                    probe_points.push(w[1]);
                    accum = 0.0;
                    if probe_points.len() >= 34 {
                        break;
                    }
                }
            }
            if let Some(&last) = points.last() {
                probe_points.push(last);
            }

            // 2. Sample interior points enclosed inside the blue polygon -> elements ENCLOSED
            let grid = 8usize;
            let mut interior_added = 0usize;
            for r in 0..grid {
                for c in 0..grid {
                    if interior_added >= 24 {
                        break;
                    }
                    let px = bbox.x + ((bbox.w as usize) * (2 * c + 1) / (2 * grid)) as i32;
                    let py = bbox.y + ((bbox.h as usize) * (2 * r + 1) / (2 * grid)) as i32;
                    if point_in_polygon((px, py), points) {
                        probe_points.push((px, py));
                        interior_added += 1;
                    }
                }
            }
        }
    }

    let mut seen_els: HashSet<String> = HashSet::new();
    let mut swept_parents: HashSet<String> = HashSet::new();
    let started = std::time::Instant::now();

    let geom_matches = |rect: ScreenRect| -> bool {
        match gesture {
            BlueGestureKind::Box => rects_intersect(bbox, rect),
            BlueGestureKind::Lasso => polyline_touches_or_encloses_rect(points, rect),
        }
    };

    for (px, py) in probe_points {
        if out.len() >= cap || started.elapsed().as_millis() > 480 {
            break;
        }
        let (rhwnd, pid, _, wclass) = resolve_root_window_at(px, py);
        if pid == 0 || (expected_pid != 0 && pid != expected_pid && !is_desktop_window_class(&wclass)) {
            continue;
        }
        let win_rect = get_window_screen_rect(rhwnd);
        let Some(el) = find_target_uia_element(&auto, &walker, rhwnd, pid, px, py) else {
            continue;
        };
        let info = extract_uia_element_info(&el, &walker, false);
        if is_strict_foreground_for_blue(&info, win_rect, &wclass) && geom_matches(info.rect) {
            let key = format!(
                "{}:{}:{}:{}:{}",
                info.control_type_id, info.rect.x, info.rect.y, info.name, info.automation_id
            );
            if seen_els.insert(key) {
                out.push(info.clone());
            }

            // Sibling container sweep: catches 100% of touched/enclosed ListItems, icons, buttons, tabs, etc.
            let is_sweepable = matches!(
                info.control_type.as_str(),
                "ListItem"
                    | "TreeItem"
                    | "DataItem"
                    | "TabItem"
                    | "Button"
                    | "CheckBox"
                    | "RadioButton"
                    | "MenuItem"
                    | "Hyperlink"
                    | "Text"
                    | "Edit"
            );
            if is_sweepable {
                if let Ok(parent) = walker.get_parent(&el) {
                    if let Ok(prc) = parent.get_bounding_rectangle() {
                        let pkey = format!(
                            "{}:{}:{}:{}",
                            pid,
                            prc.get_left(),
                            prc.get_top(),
                            prc.get_right() - prc.get_left()
                        );
                        if swept_parents.insert(pkey) {
                            if let Ok(mut child) = walker.get_first_child(&parent) {
                                for _ in 0..120 {
                                    if out.len() >= cap || started.elapsed().as_millis() > 480 {
                                        break;
                                    }
                                    if let Ok(crc) = child.get_bounding_rectangle() {
                                        let sr = ScreenRect {
                                            x: crc.get_left(),
                                            y: crc.get_top(),
                                            w: (crc.get_right() - crc.get_left()).max(0),
                                            h: (crc.get_bottom() - crc.get_top()).max(0),
                                        };
                                        if sr.w > 2 && sr.h > 2 && geom_matches(sr) {
                                            let cinfo = extract_uia_element_info(&child, &walker, false);
                                            if is_strict_foreground_for_blue(&cinfo, win_rect, &wclass) {
                                                let ckey = format!(
                                                    "{}:{}:{}:{}:{}",
                                                    cinfo.control_type_id,
                                                    cinfo.rect.x,
                                                    cinfo.rect.y,
                                                    cinfo.name,
                                                    cinfo.automation_id
                                                );
                                                if seen_els.insert(ckey) {
                                                    out.push(cinfo);
                                                }
                                            }
                                        }
                                    }
                                    match walker.get_next_sibling(&child) {
                                        Ok(next) => child = next,
                                        Err(_) => break,
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Natural top-to-bottom, left-to-right reading order
    out.sort_by_key(|e| (e.rect.y / 18, e.rect.x));
    out.truncate(cap);
    out
}


/// Left-button BLUE drag:
/// 1) Word / Excel smart selection if inside Office canvas
/// 2) Diagonal/horizontal text range extraction (UIA TextPattern RangeFromPoint)
/// 3) CAD-style Auto-Box / Auto-Lasso foreground element group (`元素组`, strictly excluding background)
pub(super) fn execute_blue_selection_pick(
    app: &AppHandle,
    shared: &AppShared,
    points: Vec<(i32, i32)>,
    pre_capture: Option<RawScreenCapture>,
) {
    // Keep Word text drag & Excel cell range box on Left drag inside Office
    if try_execute_office_drag_pick(app, shared, &points, pre_capture.as_ref()) {
        return;
    }

    let (mut min_x, mut min_y) = points[0];
    let (mut max_x, mut max_y) = points[0];
    for &(px, py) in &points[1..] {
        min_x = min_x.min(px);
        min_y = min_y.min(py);
        max_x = max_x.max(px);
        max_y = max_y.max(py);
    }
    let bbox = ScreenRect {
        x: min_x,
        y: min_y,
        w: (max_x - min_x).max(12),
        h: (max_y - min_y).max(12),
    };
    let gesture = classify_blue_gesture(&points);
    let start_pt = points[0];
    let end_pt = *points.last().unwrap_or(&start_pt);
    let center_x = bbox.x + bbox.w / 2;
    let center_y = bbox.y + bbox.h / 2;

    let (root_hwnd, target_pid, win_title, win_class) = resolve_root_window_at(start_pt.0, start_pt.1);
    if target_pid == 0 {
        if let Some(ov) = app.get_webview_window("overlay") {
            let _ = ov.emit("pen:clear_stroke", serde_json::json!({}));
        }
        return;
    }

    let cfg = shared.config.lock().map(|c| c.clone()).unwrap_or_default();
    let proc_meta = inspect_process(target_pid, &cfg.app_source_map);
    let app_display = if !proc_meta.product_name.is_empty() {
        proc_meta.product_name.as_str()
    } else if !proc_meta.exe_name.is_empty() {
        proc_meta.exe_name.as_str()
    } else {
        "App"
    };
    let win_rect = get_window_screen_rect(root_hwnd);
    let mon = MonitorSpace::for_point(center_x, center_y, root_hwnd);

    // Priority 1: Diagonal / horizontal blue text selection
    if let Some(text_hit) =
        try_extract_blue_text_range(&points, bbox, root_hwnd, target_pid, &win_class)
    {
        let n = shared.allocate_pick_number();
        let badge = circled_badge(n);
        if let Some(ov) = app.get_webview_window("overlay") {
            let pts_json: Vec<serde_json::Value> = points
                .iter()
                .map(|&(px, py)| serde_json::json!({ "x": px, "y": py }))
                .collect();
            let _ = ov.emit(
                "pen:picked",
                serde_json::json!({
                    "n": n,
                    "mode": "blue_text",
                    "color": "blue",
                    "rect": text_hit.bbox,
                    "strokes": pts_json,
                }),
            );
        }

        let grounding_lines = grounding::format_drag_grounding(
            root_hwnd,
            win_rect,
            &mon,
            text_hit.start_pt,
            text_hit.end_pt,
            text_hit.bbox,
        );
        let one_line_preview = text_hit
            .text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let short_preview = truncate_chars(&one_line_preview, 22);
        let char_count = text_hit.text.chars().count();

        let mut lines = Vec::new();
        lines.push(format!(
            "[DSB-PICK #{n} badge={badge} kind=blue_text_select chars={char_count} app=\"{}\"]",
            proc_meta.exe_name
        ));
        lines.push(format!("window_title: {}", win_title.trim()));
        lines.push(format!("window_class: {win_class}"));
        lines.push(format!(
            "process: pid={} exe=\"{}\" framework=\"{}\"",
            proc_meta.pid, proc_meta.exe_path, proc_meta.framework_guess
        ));
        lines.extend(grounding_lines);
        lines.push(format!(
            "source_control: type={} name=\"{}\"",
            text_hit.container_info.control_type,
            truncate_chars(&text_hit.container_info.name, 60)
        ));
        lines.push("selected_text:".to_string());
        lines.push(truncate_chars(&text_hit.text, 6000));
        lines.push("[/DSB-PICK]".to_string());

        let capsule = CapsulePayload {
            summary: CapsuleQuoteSummary {
                label: badge.clone(),
                title: format!("{badge} 选字 · {short_preview}"),
                detail: format!("{app_display} · {char_count} 字"),
                body: truncate_chars(&one_line_preview, 120),
            },
            text: lines.join("\n"),
        };

        log::info!(
            "Committed Stroke Pick #{n}: mode=blue_text_select chars={char_count} bbox=({},{},{}x{})",
            bbox.x,
            bbox.y,
            bbox.w,
            bbox.h
        );
        inject_capsule_into_bar(app, &capsule);
        return;
    }

    // Priority 2: CAD-style Blue Box / Freehand Lasso foreground element selection (NEVER background!)
    let raw_sampled = sample_blue_foreground_elements(&points, bbox, gesture, target_pid, 80);
    let sampled = coalesce_inline_text_runs(raw_sampled);
    if sampled.is_empty() {
        log::info!(
            "Blue {:?} found 0 foreground elements (background excluded) in bbox=({},{},{}x{})",
            gesture,
            bbox.x,
            bbox.y,
            bbox.w,
            bbox.h
        );
        if let Some(ov) = app.get_webview_window("overlay") {
            let _ = ov.emit("pen:clear_stroke", serde_json::json!({}));
        }
        emit_pick_toast(app, "蓝线范围内未检测到前景元素（已自动排除背景）");
        return;
    }

    let n = shared.allocate_pick_number();

    if let Some(ov) = app.get_webview_window("overlay") {
        let pts_json: Vec<serde_json::Value> = points
            .iter()
            .map(|&(px, py)| serde_json::json!({ "x": px, "y": py }))
            .collect();
        let item_rects: Vec<ScreenRect> = sampled.iter().map(|e| e.rect).collect();
        let _ = ov.emit(
            "pen:picked",
            serde_json::json!({
                "n": n,
                "mode": if gesture == BlueGestureKind::Lasso { "blue_lasso" } else { "blue_box" },
                "color": "blue",
                "strokes": pts_json,
                "rect": bbox,
                "itemRects": item_rects,
            }),
        );
    }

    let adapter_outputs = collect_deep_adapter_outputs(
        AdapterContext {
            hwnd: root_hwnd.0 as isize,
            window_title: win_title.clone(),
            window_class: win_class.clone(),
            proc_meta: proc_meta.clone(),
            primary_element: sampled.first().cloned(),
            tap_point: Some((center_x, center_y)),
            broker: shared.broker.clone(),
        },
        1200,
    );

    let grounding_lines =
        grounding::format_drag_grounding(root_hwnd, win_rect, &mon, start_pt, end_pt, bbox);

    let capsule = if sampled.len() == 1 {
        build_capsule_payload(
            n,
            "blue_select",
            &win_title,
            &win_class,
            &proc_meta,
            sampled.first(),
            &sampled,
            &adapter_outputs,
            &grounding_lines,
            Some((&mon, win_rect)),
            None,
        )
    } else {
        build_blue_element_group_capsule(
            n,
            gesture,
            &win_title,
            &win_class,
            &proc_meta,
            &sampled,
            &adapter_outputs,
            &grounding_lines,
        )
    };

    log::info!(
        "Committed Stroke Pick #{n}: mode={} bbox=({},{},{}x{}) sampled={}",
        if gesture == BlueGestureKind::Lasso {
            "blue_lasso"
        } else {
            "blue_box"
        },
        bbox.x,
        bbox.y,
        bbox.w,
        bbox.h,
        sampled.len()
    );
    inject_capsule_into_bar(app, &capsule);
}

/// Coalesce adjacent inline `type=Text` UIA leaf fragments on the same visual line
/// (common in Electron / streaming Markdown DOMs where every CJK word or inline code
/// token is a separate span) into a single continuous text item with a union bbox.
/// Non-Text interactive controls (Button, Edit, etc.) and far-apart columns are untouched.
pub(super) fn coalesce_inline_text_runs(mut elements: Vec<UiaElementInfo>) -> Vec<UiaElementInfo> {
    if elements.len() <= 1 {
        return elements;
    }

    // Sort in natural reading order (same visual line left-to-right, then top-to-bottom)
    // so adjacent streaming text fragments are contiguous before merging.
    elements.sort_by(|a, b| {
        let a_cy = a.rect.y + a.rect.h / 2;
        let b_cy = b.rect.y + b.rect.h / 2;
        let line_tol = (a.rect.h.min(b.rect.h).max(12) / 2).clamp(6, 14);
        if (a_cy - b_cy).abs() <= line_tol {
            a.rect.x.cmp(&b.rect.x)
        } else {
            a.rect.y.cmp(&b.rect.y)
        }
    });

    fn is_mergeable_text(el: &UiaElementInfo) -> bool {
        el.control_type == "Text"
            && el.automation_id.trim().is_empty()
            && !el.name.trim().is_empty()
    }

    fn needs_space_between(prev: &str, next: &str, gap_px: i32) -> bool {
        let Some(last_ch) = prev.chars().last() else {
            return false;
        };
        let Some(first_ch) = next.chars().next() else {
            return false;
        };
        if last_ch.is_whitespace() || first_ch.is_whitespace() {
            return false;
        }
        if matches!(last_ch, '（' | '(' | '「' | '『' | '【' | '[')
            || matches!(
                first_ch,
                '）' | ')' | '，' | '。' | '！' | '？' | '、' | '：' | '；' | '」' | '』' | '】' | ']' | ',' | '.' | '!' | '?' | ':' | ';'
            )
        {
            return false;
        }
        // Inline code pills / spaced runs have >=3px visual gap; also keep space between two ASCII word tokens.
        gap_px >= 3 || (last_ch.is_ascii_alphanumeric() && first_ch.is_ascii_alphanumeric())
    }

    let mut merged: Vec<UiaElementInfo> = Vec::with_capacity(elements.len());
    for el in elements {
        if let Some(prev) = merged.last_mut() {
            if is_mergeable_text(prev) && is_mergeable_text(&el) {
                let prev_cy = prev.rect.y + prev.rect.h / 2;
                let cur_cy = el.rect.y + el.rect.h / 2;
                let max_h = prev.rect.h.max(el.rect.h).max(16);
                let same_line =
                    (prev.rect.y - el.rect.y).abs() <= 8 && (prev_cy - cur_cy).abs() <= max_h / 2;
                let gap = el.rect.x - (prev.rect.x + prev.rect.w);
                if same_line && (-6..=20).contains(&gap) {
                    if needs_space_between(&prev.name, &el.name, gap) {
                        prev.name.push(' ');
                    }
                    prev.name.push_str(&el.name);
                    let min_x = prev.rect.x.min(el.rect.x);
                    let min_y = prev.rect.y.min(el.rect.y);
                    let max_r = (prev.rect.x + prev.rect.w).max(el.rect.x + el.rect.w);
                    let max_b = (prev.rect.y + prev.rect.h).max(el.rect.y + el.rect.h);
                    prev.rect = ScreenRect {
                        x: min_x,
                        y: min_y,
                        w: (max_r - min_x).max(0),
                        h: (max_b - min_y).max(0),
                    };
                    if !el.text_snippet.is_empty() && prev.text_snippet != prev.name {
                        prev.text_snippet = prev.name.clone();
                    }
                    continue;
                }
            }
        }
        merged.push(el);
    }
    merged
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_blue_element_group_capsule(
    n: usize,
    gesture: BlueGestureKind,
    win_title: &str,
    win_class: &str,
    proc_meta: &process::ProcessMetadata,
    elements: &[UiaElementInfo],
    adapter_outputs: &[adapters::AdapterOutput],
    grounding_lines: &[String],
) -> CapsulePayload {
    let badge = circled_badge(n);
    let count = elements.len();
    let gesture_tag = match gesture {
        BlueGestureKind::Box => "box",
        BlueGestureKind::Lasso => "lasso",
    };
    let gesture_zh = match gesture {
        BlueGestureKind::Box => "蓝线框选",
        BlueGestureKind::Lasso => "蓝线圈选",
    };
    let app_display = if !proc_meta.product_name.is_empty() {
        proc_meta.product_name.as_str()
    } else if !proc_meta.exe_name.is_empty() {
        proc_meta.exe_name.as_str()
    } else {
        "App"
    };

    let preview_names: Vec<String> = elements
        .iter()
        .take(4)
        .map(|e| {
            if !e.name.is_empty() {
                truncate_chars(&e.name, 16)
            } else if !e.value.is_empty() && e.value != "[PASSWORD_REDACTED]" {
                truncate_chars(&e.value, 16)
            } else {
                e.control_type.clone()
            }
        })
        .collect();
    let body_preview = if count > 4 {
        format!("{} 等 {} 项前景元素", preview_names.join("、"), count)
    } else {
        preview_names.join("、")
    };

    let mut lines = Vec::new();
    lines.push(format!(
        "[DSB-PICK #{n} badge={badge} kind=element_group gesture={gesture_tag} count={count} app=\"{}\"]",
        proc_meta.exe_name
    ));
    lines.push(format!("window_title: {}", win_title.trim()));
    lines.push(format!("window_class: {win_class}"));
    lines.push(format!(
        "process: pid={} exe=\"{}\" framework=\"{}\"",
        proc_meta.pid, proc_meta.exe_path, proc_meta.framework_guess
    ));
    lines.extend(grounding_lines.iter().cloned());
    lines.push(format!("element_group ({count} foreground items):"));
    for (idx, el) in elements.iter().enumerate() {
        let mut desc = format!(
            "  [{}] type={} rect=({},{},{}x{})",
            idx + 1,
            el.control_type,
            el.rect.x,
            el.rect.y,
            el.rect.w,
            el.rect.h
        );
        if !el.name.is_empty() {
            desc.push_str(&format!(" name=\"{}\"", truncate_chars(&el.name, 80)));
        }
        if !el.value.is_empty() && el.value != "[PASSWORD_REDACTED]" && el.value != el.name {
            desc.push_str(&format!(" value=\"{}\"", truncate_chars(&el.value, 80)));
        }
        if !el.automation_id.is_empty() {
            desc.push_str(&format!(" id=\"{}\"", el.automation_id));
        }
        lines.push(desc);
    }

    for out in adapter_outputs {
        for detail in &out.lines {
            lines.push(detail.clone());
        }
        for p in out.paths.iter().take(12) {
            if let Some(meta) = inspect_path_metadata(Path::new(p), 600) {
                append_fs_meta_lines(&mut lines, &meta);
            }
        }
    }
    lines.push("[/DSB-PICK]".to_string());

    CapsulePayload {
        summary: CapsuleQuoteSummary {
            label: badge.clone(),
            title: format!("{badge} 元素组 ({count}项)"),
            detail: format!("{app_display} · {gesture_zh}"),
            body: body_preview,
        },
        text: lines.join("\n"),
    }
}

pub(super) fn is_wps_exe(exe_name: &str) -> bool {
    matches!(exe_name, "wps.exe" | "wpsoffice.exe" | "et.exe" | "wpp.exe")
}

pub(super) fn office_app_display_name(exe_name: &str, kind: OfficeAppKind) -> &'static str {
    let is_wps = is_wps_exe(exe_name);
    match (is_wps, kind) {
        (true, OfficeAppKind::Word) => "WPS 文字",
        (true, OfficeAppKind::Excel) => "WPS 表格",
        (true, OfficeAppKind::PowerPoint) => "WPS 演示",
        (false, OfficeAppKind::Word) => "Word",
        (false, OfficeAppKind::Excel) => "Excel",
        (false, OfficeAppKind::PowerPoint) => "PowerPoint",
    }
}

