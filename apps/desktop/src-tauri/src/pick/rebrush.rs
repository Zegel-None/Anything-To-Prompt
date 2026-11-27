use crate::captures::ScreenRect;
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub const REBRUSH_CELL_PX: i32 = 96;
const REENTER_MIN_MS: u64 = 420;
const REVERSAL_MIN_DIST_PX: f64 = 38.0;
const PASS2_COOLDOWN_MS: u64 = 1500;
const PASS3_COOLDOWN_MS: u64 = 6500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ScanTier {
    FastPass1 = 1,
    DeepLocalPass2 = 2,
    VlmArbitratePass3 = 3,
}

impl ScanTier {
    pub fn level(self) -> u8 {
        self as u8
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RebrushCellKey {
    pub gx: i32,
    pub gy: i32,
}

impl RebrushCellKey {
    pub fn from_point(x: i32, y: i32) -> Self {
        Self {
            gx: x.div_euclid(REBRUSH_CELL_PX),
            gy: y.div_euclid(REBRUSH_CELL_PX),
        }
    }

    pub fn screen_rect(self) -> ScreenRect {
        ScreenRect {
            x: self.gx * REBRUSH_CELL_PX,
            y: self.gy * REBRUSH_CELL_PX,
            w: REBRUSH_CELL_PX,
            h: REBRUSH_CELL_PX,
        }
    }
}

#[derive(Debug, Clone)]
struct CellState {
    passes: u8,
    last_entered_at: Instant,
    last_exited_at: Option<Instant>,
    last_pass2_at: Option<Instant>,
    last_pass3_at: Option<Instant>,
    verified_locked: bool,
    anchor_pt: (i32, i32),
    peak_dist_px: f64,
    prev_dir: Option<(f64, f64)>,
}

#[derive(Debug, Clone)]
pub struct RebrushDecision {
    pub cell: RebrushCellKey,
    pub cell_rect: ScreenRect,
    pub pass_count: u8,
    pub tier: ScanTier,
    /// True only when this point newly triggered an escalation (Pass 2 or Pass 3) outside cooldown.
    pub escalated_now: bool,
}

/// Tracks 96x96 spatial heat-cells along a middle-button green-brush session.
/// Distinguishes a normal single sweep (always Pass 1) from deliberate scrubbing / re-brushing
/// (re-entering a cell after >= 280ms OR reversing stroke direction inside the cell).
#[derive(Debug, Default)]
pub struct RebrushTracker {
    cells: HashMap<RebrushCellKey, CellState>,
    current_cell: Option<RebrushCellKey>,
    last_pt: Option<(i32, i32)>,
    pass2_threshold: u8,
    pass3_threshold: u8,
}

impl RebrushTracker {
    pub fn new(pass2_threshold: u8, pass3_threshold: u8) -> Self {
        let p2 = pass2_threshold.clamp(2, 5);
        let p3 = pass3_threshold.max(p2 + 1).clamp(3, 7);
        Self {
            cells: HashMap::new(),
            current_cell: None,
            last_pt: None,
            pass2_threshold: p2,
            pass3_threshold: p3,
        }
    }

    pub fn clear(&mut self) {
        self.cells.clear();
        self.current_cell = None;
        self.last_pt = None;
    }

    pub fn reset_thresholds(&mut self, pass2_threshold: u8, pass3_threshold: u8) {
        self.pass2_threshold = pass2_threshold.clamp(2, 5);
        self.pass3_threshold = pass3_threshold.max(self.pass2_threshold + 1).clamp(3, 7);
    }

    /// Locks a cell after Pass 3 verification succeeds so continued wide-brush movement
    /// NEVER oscillates or overwrites already-verified boxes.
    pub fn mark_cell_verified(&mut self, cell: RebrushCellKey) {
        if let Some(st) = self.cells.get_mut(&cell) {
            st.verified_locked = true;
        }
    }

    /// Call on mouse-up of a brush stroke so starting a second stroke over the same cell
    /// immediately counts as a clean re-entry.
    pub fn end_stroke(&mut self, now: Instant) {
        if let Some(prev) = self.current_cell.take() {
            if let Some(st) = self.cells.get_mut(&prev) {
                st.last_exited_at = Some(now);
            }
        }
        self.last_pt = None;
    }

    pub fn observe(&mut self, x: i32, y: i32, now: Instant) -> RebrushDecision {
        let p2_th = if self.pass2_threshold == 0 {
            2
        } else {
            self.pass2_threshold
        };
        let p3_th = if self.pass3_threshold <= p2_th {
            p2_th + 1
        } else {
            self.pass3_threshold
        };

        let key = RebrushCellKey::from_point(x, y);
        let prev_cell = self.current_cell;
        let prev_pt = self.last_pt;
        self.current_cell = Some(key);
        self.last_pt = Some((x, y));

        if let Some(old_key) = prev_cell {
            if old_key != key {
                if let Some(old_st) = self.cells.get_mut(&old_key) {
                    old_st.last_exited_at = Some(now);
                }
            }
        }

        let st = self.cells.entry(key).or_insert_with(|| CellState {
            passes: 1,
            last_entered_at: now,
            last_exited_at: None,
            last_pass2_at: None,
            last_pass3_at: None,
            verified_locked: false,
            anchor_pt: (x, y),
            peak_dist_px: 0.0,
            prev_dir: None,
        });

        let mut bumped = false;
        if st.verified_locked {
            return RebrushDecision {
                cell: key,
                cell_rect: key.screen_rect(),
                pass_count: st.passes,
                tier: ScanTier::FastPass1,
                escalated_now: false,
            };
        }
        if prev_cell != Some(key) {
            if let Some(exited_at) = st.last_exited_at {
                if now.saturating_duration_since(exited_at) >= Duration::from_millis(REENTER_MIN_MS)
                {
                    st.passes = st.passes.saturating_add(1);
                    st.last_entered_at = now;
                    st.anchor_pt = (x, y);
                    st.peak_dist_px = 0.0;
                    st.prev_dir = None;
                    bumped = true;
                }
            }
        } else if let Some((lx, ly)) = prev_pt {
            let dx = (x - lx) as f64;
            let dy = (y - ly) as f64;
            let step_len = (dx * dx + dy * dy).sqrt();
            if step_len >= 10.0 {
                let dir = (dx / step_len, dy / step_len);
                let adx = (x - st.anchor_pt.0) as f64;
                let ady = (y - st.anchor_pt.1) as f64;
                let dist_from_anchor = (adx * adx + ady * ady).sqrt();
                st.peak_dist_px = st.peak_dist_px.max(dist_from_anchor);
                if let Some((pdx, pdy)) = st.prev_dir {
                    let dot = pdx * dir.0 + pdy * dir.1;
                    if dot < -0.35 && st.peak_dist_px >= REVERSAL_MIN_DIST_PX {
                        st.passes = st.passes.saturating_add(1);
                        st.anchor_pt = (x, y);
                        st.peak_dist_px = 0.0;
                        st.prev_dir = Some(dir);
                        bumped = true;
                    }
                } else {
                    st.prev_dir = Some(dir);
                }
            }
        }

        let mut tier = ScanTier::FastPass1;
        let mut escalated_now = false;

        if st.passes >= p3_th {
            let cool_ok = st.last_pass3_at.is_none_or(|t| {
                now.saturating_duration_since(t) >= Duration::from_millis(PASS3_COOLDOWN_MS)
            });
            if cool_ok && (bumped || st.last_pass3_at.is_none()) {
                st.last_pass3_at = Some(now);
                tier = ScanTier::VlmArbitratePass3;
                escalated_now = true;
            }
        } else if st.passes >= p2_th {
            let cool_ok = st.last_pass2_at.is_none_or(|t| {
                now.saturating_duration_since(t) >= Duration::from_millis(PASS2_COOLDOWN_MS)
            });
            if cool_ok && (bumped || st.last_pass2_at.is_none()) {
                st.last_pass2_at = Some(now);
                tier = ScanTier::DeepLocalPass2;
                escalated_now = true;
            }
        }

        RebrushDecision {
            cell: key,
            cell_rect: key.screen_rect(),
            pass_count: st.passes,
            tier,
            escalated_now,
        }
    }
}

/// Interpolates intermediate cursor points along a fast brush stroke so sparse timer ticks
/// never skip narrow text lines or toolbar buttons between `from` and `to`.
pub fn interpolate_brush_segment(
    from: Option<(i32, i32)>,
    to: (i32, i32),
    step_px: i32,
) -> Vec<(i32, i32)> {
    let Some((x0, y0)) = from else {
        return vec![to];
    };
    let dx = (to.0 - x0) as f64;
    let dy = (to.1 - y0) as f64;
    let dist = (dx * dx + dy * dy).sqrt();
    let step = step_px.max(12) as f64;
    if dist < step {
        return vec![to];
    }
    let count = ((dist / step).ceil() as usize).clamp(1, 18);
    let mut pts = Vec::with_capacity(count);
    for i in 1..=count {
        let t = (i as f64) / (count as f64);
        pts.push((
            (x0 as f64 + dx * t).round() as i32,
            (y0 as f64 + dy * t).round() as i32,
        ));
    }
    pts
}

// =============================================================================
// Shake-to-Wake Dose Detector (Normal Selection Mode — zero MMB required!)
// =============================================================================
// - Stationary / calm aiming on a blue box: NEVER triggers.
// - Deliberate shake (>= 2 direction reversals within 650ms inside a ~140px local cluster,
//   total travel >= 75px): wakes Dose 1 (OmniParser v2 + Hover-Diff + Deep UIA).
// - Once Dose 1 results have landed, continuing to shake on the same region escalates to
//   Dose 2 (SAM 2.1 photo matting / multi-level menu drawer expansion / VLM arbitration).

pub const DOSE_CELL_PX: i32 = 128;
const SHAKE_WINDOW_MS: u64 = 650;
const SHAKE_MIN_LEG_PX: f64 = 18.0;
const SHAKE_MIN_PATH_PX: f64 = 75.0;
const SHAKE_MAX_RADIUS_PX: f64 = 145.0;
const DOSE2_COOLDOWN_MS: u64 = 3800;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DoseCellKey {
    pub gx: i32,
    pub gy: i32,
}

impl DoseCellKey {
    pub fn from_point(x: i32, y: i32) -> Self {
        Self {
            gx: x.div_euclid(DOSE_CELL_PX),
            gy: y.div_euclid(DOSE_CELL_PX),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoseEscalation {
    /// Dose 1: Wake OmniParser v2 + Hover-Diff + Deep UIA on owning container/tile.
    Dose1Wake { cell: DoseCellKey, x: i32, y: i32 },
    /// Dose 2: Dose 1 already delivered & user keeps shaking -> Photo matting / Drawer expand / VLM boost.
    Dose2Boost { cell: DoseCellKey, x: i32, y: i32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CellDosePhase {
    Idle,
    Dose1InFlight,
    Dose1Done,
    Dose2InFlight,
    Dose2Done { finished_at: Instant },
}

#[derive(Debug, Default)]
pub struct ShakeDoseDetector {
    samples: Vec<(i32, i32, Instant)>,
    cells: HashMap<DoseCellKey, CellDosePhase>,
    last_trigger_at: Option<Instant>,
}

impl ShakeDoseDetector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.samples.clear();
        self.cells.clear();
        self.last_trigger_at = None;
    }

    pub fn mark_dose1_complete(&mut self, cell: DoseCellKey, now: Instant) {
        self.cells.insert(cell, CellDosePhase::Dose1Done);
        // Clear pre-completion samples so ONLY fresh shaking AFTER Dose 1 results land
        // can escalate to Dose 2!
        self.samples.clear();
        self.last_trigger_at = Some(now);
    }

    pub fn mark_dose2_complete(&mut self, cell: DoseCellKey, now: Instant) {
        self.cells
            .insert(cell, CellDosePhase::Dose2Done { finished_at: now });
        self.samples.clear();
        self.last_trigger_at = Some(now);
    }

    /// Rolls back an `*InFlight` cell phase when the global `dose_inflight` lock is busy or
    /// worker spawn fails — prevents permanent cell deadlock!
    pub fn revert_dose_inflight(&mut self, cell: DoseCellKey) {
        match self.cells.get(&cell).copied() {
            Some(CellDosePhase::Dose1InFlight) => {
                self.cells.insert(cell, CellDosePhase::Idle);
            }
            Some(CellDosePhase::Dose2InFlight) => {
                self.cells.insert(cell, CellDosePhase::Dose1Done);
            }
            _ => {}
        }
    }

    /// Observes a cursor point `(x, y)` during normal Selection Mode hover.
    /// Returns `Some(DoseEscalation)` ONLY when the user actively shakes (`>= 2` reversals in `650ms`).
    pub fn observe(&mut self, x: i32, y: i32, now: Instant) -> Option<DoseEscalation> {
        // Drop samples older than SHAKE_WINDOW_MS
        let window = Duration::from_millis(SHAKE_WINDOW_MS);
        self.samples
            .retain(|&(_, _, ts)| now.saturating_duration_since(ts) <= window);

        // Ignore micro-jitter (< 4px) so stationary hand tremor NEVER accumulates shake energy
        if let Some(&(lx, ly, _)) = self.samples.last() {
            let d2 = ((x - lx) as i64).pow(2) + ((y - ly) as i64).pow(2);
            if d2 < 16 {
                return None;
            }
        }
        self.samples.push((x, y, now));
        if self.samples.len() > 64 {
            let excess = self.samples.len() - 64;
            self.samples.drain(..excess);
        }
        if self.samples.len() < 4 {
            return None;
        }

        // Brief debounce after a trigger so one shake doesn't double-fire on the very next sample
        if let Some(lt) = self.last_trigger_at {
            if now.saturating_duration_since(lt) < Duration::from_millis(220) {
                return None;
            }
        }

        // Analyze trajectory inside the 650ms window:
        // 1. Must stay roughly local around the shake centroid (not a huge cross-screen sweep)
        let n = self.samples.len() as f64;
        let cx = self.samples.iter().map(|s| s.0 as f64).sum::<f64>() / n;
        let cy = self.samples.iter().map(|s| s.1 as f64).sum::<f64>() / n;
        let max_r = self
            .samples
            .iter()
            .map(|s| ((s.0 as f64 - cx).powi(2) + (s.1 as f64 - cy).powi(2)).sqrt())
            .fold(0.0f64, f64::max);
        if max_r > SHAKE_MAX_RADIUS_PX {
            return None;
        }

        // 2. Count distinct reversals where each leg moved >= SHAKE_MIN_LEG_PX and turned >= 110 deg (dot < -0.35)
        let mut total_path = 0.0f64;
        let mut reversals = 0u32;
        let mut leg_origin = (self.samples[0].0, self.samples[0].1);
        let mut prev_leg_dir: Option<(f64, f64)> = None;
        let mut peak_leg_len = 0.0f64;

        for w in self.samples.windows(2) {
            let (x0, y0, _) = w[0];
            let (x1, y1, _) = w[1];
            let sdx = (x1 - x0) as f64;
            let sdy = (y1 - y0) as f64;
            let step = (sdx * sdx + sdy * sdy).sqrt();
            if step < 4.0 {
                continue;
            }
            total_path += step;

            let ldx = (x1 - leg_origin.0) as f64;
            let ldy = (y1 - leg_origin.1) as f64;
            let leg_dist = (ldx * ldx + ldy * ldy).sqrt();
            peak_leg_len = peak_leg_len.max(leg_dist);

            let step_dir = (sdx / step, sdy / step);
            if let Some((pdx, pdy)) = prev_leg_dir {
                let dot = pdx * step_dir.0 + pdy * step_dir.1;
                if dot < -0.35 && peak_leg_len >= SHAKE_MIN_LEG_PX {
                    reversals += 1;
                    leg_origin = (x0, y0);
                    peak_leg_len = step;
                    prev_leg_dir = Some(step_dir);
                }
            } else if leg_dist >= SHAKE_MIN_LEG_PX {
                prev_leg_dir = Some((ldx / leg_dist, ldy / leg_dist));
            }
        }

        if reversals < 2 || total_path < SHAKE_MIN_PATH_PX {
            return None;
        }

        let cell = DoseCellKey::from_point(cx.round() as i32, cy.round() as i32);
        let phase = self.cells.get(&cell).copied().unwrap_or(CellDosePhase::Idle);
        match phase {
            CellDosePhase::Idle => {
                self.cells.insert(cell, CellDosePhase::Dose1InFlight);
                self.samples.clear();
                self.last_trigger_at = Some(now);
                Some(DoseEscalation::Dose1Wake { cell, x, y })
            }
            CellDosePhase::Dose1Done => {
                self.cells.insert(cell, CellDosePhase::Dose2InFlight);
                self.samples.clear();
                self.last_trigger_at = Some(now);
                Some(DoseEscalation::Dose2Boost { cell, x, y })
            }
            CellDosePhase::Dose2Done { finished_at }
                if now.saturating_duration_since(finished_at)
                    >= Duration::from_millis(DOSE2_COOLDOWN_MS) =>
            {
                self.cells.insert(cell, CellDosePhase::Dose2InFlight);
                self.samples.clear();
                self.last_trigger_at = Some(now);
                Some(DoseEscalation::Dose2Boost { cell, x, y })
            }
            CellDosePhase::Dose1InFlight
            | CellDosePhase::Dose2InFlight
            | CellDosePhase::Dose2Done { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shake_dose_stationary_and_single_aim_nudge_never_trigger() {
        let mut det = ShakeDoseDetector::new();
        let t0 = Instant::now();
        // Stationary tremor (+/- 2px)
        for i in 0..20 {
            let dx = if i % 2 == 0 { 2 } else { -2 };
            assert!(det.observe(300 + dx, 200, t0 + Duration::from_millis(i * 25)).is_none());
        }
        // Single reversal (aiming between two adjacent buttons: right 28px then left 24px -> only 1 reversal!)
        let pts = [(300, 200), (315, 200), (328, 200), (316, 200), (304, 200)];
        for (i, (x, y)) in pts.into_iter().enumerate() {
            assert!(
                det.observe(x, y, t0 + Duration::from_millis(600 + i as u64 * 40))
                    .is_none(),
                "1 reversal aiming adjustment must NOT trigger shake wake"
            );
        }
    }

    #[test]
    fn shake_dose_two_reversals_wakes_dose1_then_continued_shake_escalates_dose2() {
        let mut det = ShakeDoseDetector::new();
        let t0 = Instant::now();
        // Shake 1: Right -> Left (rev 1) -> Right (rev 2) within 300ms
        let shake1 = [
            (400, 200),
            (422, 200),
            (440, 202),
            (418, 201),
            (395, 200),
            (420, 200),
            (442, 200),
        ];
        let mut got1 = None;
        for (i, (x, y)) in shake1.into_iter().enumerate() {
            if let Some(ev) = det.observe(x, y, t0 + Duration::from_millis(i as u64 * 40)) {
                got1 = Some(ev);
                break;
            }
        }
        let cell = match got1 {
            Some(DoseEscalation::Dose1Wake { cell, .. }) => cell,
            other => panic!("expected Dose1Wake, got {other:?}"),
        };

        // Shaking WHILE Dose 1 is still in-flight must NOT double-trigger
        let mut dup = None;
        for (i, (x, y)) in shake1.into_iter().enumerate() {
            if let Some(ev) = det.observe(x, y, t0 + Duration::from_millis(400 + i as u64 * 35)) {
                dup = Some(ev);
            }
        }
        assert!(dup.is_none(), "in-flight Dose 1 must suppress duplicate shakes");

        // Dose 1 completes -> user keeps shaking -> escalates to Dose 2!
        det.mark_dose1_complete(cell, t0 + Duration::from_millis(800));
        let mut got2 = None;
        for (i, (x, y)) in shake1.into_iter().enumerate() {
            if let Some(ev) = det.observe(x, y, t0 + Duration::from_millis(1050 + i as u64 * 40)) {
                got2 = Some(ev);
                break;
            }
        }
        assert!(
            matches!(got2, Some(DoseEscalation::Dose2Boost { .. })),
            "continued shake after Dose 1 completion must escalate to Dose2Boost, got {got2:?}"
        );
    }

    #[test]
    fn shake_dose_revert_prevents_deadlock_on_inflight_contention() {
        let mut det = ShakeDoseDetector::new();
        let t0 = Instant::now();
        let shake = [
            (400, 200),
            (422, 200),
            (440, 202),
            (418, 201),
            (395, 200),
            (420, 200),
            (442, 200),
        ];
        let mut cell = None;
        for (i, (x, y)) in shake.into_iter().enumerate() {
            if let Some(DoseEscalation::Dose1Wake { cell: c, .. }) =
                det.observe(x, y, t0 + Duration::from_millis(i as u64 * 40))
            {
                cell = Some(c);
                break;
            }
        }
        let cell = cell.expect("first shake must trigger Dose1Wake");
        // Simulate global dose_inflight lock contention -> revert!
        det.revert_dose_inflight(cell);
        // Subsequent shake after 300ms MUST trigger Dose1Wake again (not deadlocked!)
        let mut retry = None;
        for (i, (x, y)) in shake.into_iter().enumerate() {
            if let Some(ev) = det.observe(x, y, t0 + Duration::from_millis(550 + i as u64 * 40)) {
                retry = Some(ev);
                break;
            }
        }
        assert!(
            matches!(retry, Some(DoseEscalation::Dose1Wake { .. })),
            "reverted cell must wake again without deadlock, got {retry:?}"
        );
    }

    #[test]
    fn single_smooth_sweep_never_triggers_rebrush_escalation() {
        let mut tr = RebrushTracker::new(2, 3);
        let t0 = Instant::now();
        for i in 0..12 {
            let d = tr.observe(100 + i * 16, 200, t0 + Duration::from_millis(i as u64 * 25));
            assert_eq!(d.tier, ScanTier::FastPass1);
            assert!(!d.escalated_now);
        }
    }

    #[test]
    fn reentering_cell_after_pause_escalates_pass2_then_pass3() {
        let mut tr = RebrushTracker::new(2, 3);
        let t0 = Instant::now();
        let d1 = tr.observe(120, 120, t0);
        assert_eq!(d1.tier, ScanTier::FastPass1);

        // Move to adjacent cell then come back after 500ms -> Pass 2
        let _ = tr.observe(320, 120, t0 + Duration::from_millis(100));
        let d2 = tr.observe(125, 125, t0 + Duration::from_millis(600));
        assert_eq!(d2.tier, ScanTier::DeepLocalPass2);
        assert!(d2.escalated_now);
        assert_eq!(d2.pass_count, 2);

        // Leave and re-enter again after 500ms -> Pass 3 (VLM audit)
        let _ = tr.observe(320, 120, t0 + Duration::from_millis(700));
        let d3 = tr.observe(122, 122, t0 + Duration::from_millis(1200));
        assert_eq!(d3.tier, ScanTier::VlmArbitratePass3);
        assert!(d3.escalated_now);
        assert_eq!(d3.pass_count, 3);

        // Once verified & locked, continued brushing NEVER oscillates back into Pass 3
        tr.mark_cell_verified(d3.cell);
        let _ = tr.observe(320, 120, t0 + Duration::from_millis(8000));
        let d4 = tr.observe(122, 122, t0 + Duration::from_millis(9000));
        assert!(!d4.escalated_now);
    }

    #[test]
    fn scrubbing_back_and_forth_inside_same_cell_escalates() {
        let mut tr = RebrushTracker::new(2, 3);
        let t0 = Instant::now();
        let _ = tr.observe(100, 100, t0);
        let _ = tr.observe(145, 100, t0 + Duration::from_millis(40));
        let d2 = tr.observe(102, 100, t0 + Duration::from_millis(80));
        assert_eq!(d2.tier, ScanTier::DeepLocalPass2);
        assert!(d2.escalated_now);
    }

    #[test]
    fn brush_segment_interpolation_fills_fast_swipe_gaps() {
        let pts = interpolate_brush_segment(Some((100, 100)), (100, 240), 28);
        assert_eq!(pts.len(), 5);
        assert_eq!(pts.last().copied(), Some((100, 240)));
    }
}
