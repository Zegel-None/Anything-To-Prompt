use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    GetCurrentThread, GetCurrentThreadId, SetThreadPriority, THREAD_PRIORITY_HIGHEST,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_RCONTROL,
    VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetCursorPos, GetForegroundWindow, GetMessageW,
    GetWindowRect, PostThreadMessageW, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx,
    HHOOK, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, MSLLHOOKSTRUCT, WH_KEYBOARD_LL, WH_MOUSE_LL,
    WM_APP, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MOUSEMOVE,
    WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

const DUMMY_EXTRA_INFO: usize = 0xD5B0_00E8;
const WM_APP_REINSTALL_HOOKS: u32 = WM_APP + 1;

const VK_CONTROL_U32: u32 = 0x11;
const VK_LCONTROL_U32: u32 = 0xA2;
const VK_RCONTROL_U32: u32 = 0xA3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CtrlEvent {
    /// Clean single-tap Ctrl while Bar is active (Summon) -> close Bar immediately.
    SingleTapClose,
    /// Clean double-tap Ctrl while Bar is Hidden/Lip -> wake Bar & focus composer.
    DoubleTapOpen,
    /// Selection mode entered (double-Ctrl-and-hold from any state, OR single-Ctrl hold while Bar is active Summon).
    HoldStart,
    /// Selection mode exited on Ctrl release or cancellation by another key.
    HoldEnd { cancelled_by_other_key: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CtrlPhase {
    Idle,
    FirstDown {
        down_at: u64,
        tainted: bool,
        pen_open: bool,
    },
    WaitingSecond {
        first_up_at: u64,
    },
    SecondDown {
        down_at: u64,
        tainted: bool,
        pen_open: bool,
    },
}

/// Pure, deterministic unified Ctrl detector (zero Alt key usage):
/// - When Bar is Hidden / Lip (`bar_summoned == false`):
///   - Single Ctrl tap or hold -> ignored (zero conflict with other apps)
///   - Quick double-tap Ctrl -> `DoubleTapOpen` (wake Bar & focus cursor)
///   - Double-tap Ctrl & hold (一短一长) -> `HoldStart` (wake Bar, focus cursor & enter selection mode)
/// - When Bar is already active (`bar_summoned == true`):
///   - Single Ctrl tap (`< hold_ms`, no other key) -> `SingleTapClose` (close Bar immediately)
///   - Single Ctrl hold (`>= hold_ms`, or click outside) -> `HoldStart` (enter selection mode; release exits via `HoldEnd` without closing)
/// - Any chord (`Ctrl+C/V/A/Z/Backspace`, `Ctrl+Click`, `Ctrl+Alt` Chatterfly) taints Ctrl and passes through untouched.
#[derive(Debug, Clone)]
pub struct DoubleCtrlDetector {
    pub max_first_tap_ms: u64,
    pub double_tap_ms: u64,
    pub hold_ms: u64,
    phase: CtrlPhase,
    held_non_ctrl_keys: HashSet<u32>,
}

impl DoubleCtrlDetector {
    pub fn new(double_tap_ms: u64, hold_ms: u64) -> Self {
        Self {
            max_first_tap_ms: 300,
            double_tap_ms: double_tap_ms.clamp(150, 1000),
            hold_ms: hold_ms.clamp(150, 600),
            phase: CtrlPhase::Idle,
            held_non_ctrl_keys: HashSet::new(),
        }
    }

    pub fn is_ctrl_vk(vk: u32) -> bool {
        matches!(vk, VK_CONTROL_U32 | VK_LCONTROL_U32 | VK_RCONTROL_U32)
    }

    /// Prunes any non-Ctrl keys whose KeyUp may have been dropped by UAC/system hooks.
    pub fn prune_released_keys<F>(&mut self, is_down: F)
    where
        F: Fn(u32) -> bool,
    {
        self.held_non_ctrl_keys.retain(|&vk| is_down(vk));
    }

    pub fn on_key_down(
        &mut self,
        vk: u32,
        now_ms: u64,
        os_other_modifier_held: bool,
    ) -> Option<CtrlEvent> {
        if Self::is_ctrl_vk(vk) {
            let clean = self.held_non_ctrl_keys.is_empty() && !os_other_modifier_held;
            match self.phase {
                CtrlPhase::Idle => {
                    self.phase = CtrlPhase::FirstDown {
                        down_at: now_ms,
                        tainted: !clean,
                        pen_open: false,
                    };
                }
                CtrlPhase::FirstDown { .. } | CtrlPhase::SecondDown { .. } => {
                    // Ignore Windows auto-repeat WM_KEYDOWN while Ctrl is held down
                }
                CtrlPhase::WaitingSecond { first_up_at } => {
                    if clean && now_ms.saturating_sub(first_up_at) <= self.double_tap_ms {
                        self.phase = CtrlPhase::SecondDown {
                            down_at: now_ms,
                            tainted: false,
                            pen_open: false,
                        };
                    } else {
                        self.phase = CtrlPhase::FirstDown {
                            down_at: now_ms,
                            tainted: !clean,
                            pen_open: false,
                        };
                    }
                }
            }
            None
        } else {
            self.held_non_ctrl_keys.insert(vk);
            match &mut self.phase {
                CtrlPhase::Idle => None,
                CtrlPhase::WaitingSecond { .. } => {
                    self.phase = CtrlPhase::Idle;
                    None
                }
                CtrlPhase::FirstDown {
                    tainted, pen_open, ..
                }
                | CtrlPhase::SecondDown {
                    tainted, pen_open, ..
                } => {
                    *tainted = true;
                    if *pen_open {
                        *pen_open = false;
                        Some(CtrlEvent::HoldEnd {
                            cancelled_by_other_key: true,
                        })
                    } else {
                        None
                    }
                }
            }
        }
    }

    pub fn on_key_up(&mut self, vk: u32, now_ms: u64, bar_summoned: bool) -> Option<CtrlEvent> {
        if Self::is_ctrl_vk(vk) {
            let prev = std::mem::replace(&mut self.phase, CtrlPhase::Idle);
            match prev {
                CtrlPhase::FirstDown {
                    down_at,
                    tainted,
                    pen_open,
                } => {
                    if pen_open {
                        // Exiting selection mode on Ctrl release: stay in Summon (do NOT close!)
                        Some(CtrlEvent::HoldEnd {
                            cancelled_by_other_key: false,
                        })
                    } else if !tainted {
                        let dur = now_ms.saturating_sub(down_at);
                        if bar_summoned && dur < self.max_first_tap_ms {
                            // Active Bar + clean single tap Ctrl -> close immediately!
                            Some(CtrlEvent::SingleTapClose)
                        } else if !bar_summoned && dur < self.max_first_tap_ms {
                            self.phase = CtrlPhase::WaitingSecond {
                                first_up_at: now_ms,
                            };
                            None
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                CtrlPhase::SecondDown {
                    down_at,
                    tainted,
                    pen_open,
                } => {
                    if pen_open {
                        Some(CtrlEvent::HoldEnd {
                            cancelled_by_other_key: false,
                        })
                    } else if !tainted && now_ms.saturating_sub(down_at) < 500 {
                        if bar_summoned {
                            Some(CtrlEvent::SingleTapClose)
                        } else {
                            Some(CtrlEvent::DoubleTapOpen)
                        }
                    } else {
                        None
                    }
                }
                CtrlPhase::Idle | CtrlPhase::WaitingSecond { .. } => None,
            }
        } else {
            self.held_non_ctrl_keys.remove(&vk);
            None
        }
    }

    pub fn poll_hold(
        &mut self,
        now_ms: u64,
        bar_summoned: bool,
        os_other_modifier_held: bool,
    ) -> Option<CtrlEvent> {
        match &mut self.phase {
            CtrlPhase::FirstDown {
                down_at,
                tainted,
                pen_open,
            } => {
                if os_other_modifier_held || !self.held_non_ctrl_keys.is_empty() {
                    *tainted = true;
                    if *pen_open {
                        *pen_open = false;
                        return Some(CtrlEvent::HoldEnd {
                            cancelled_by_other_key: true,
                        });
                    }
                    return None;
                }
                if bar_summoned
                    && !*tainted
                    && !*pen_open
                    && now_ms.saturating_sub(*down_at) >= self.hold_ms
                {
                    *pen_open = true;
                    return Some(CtrlEvent::HoldStart);
                }
                None
            }
            CtrlPhase::SecondDown {
                down_at,
                tainted,
                pen_open,
            } => {
                if os_other_modifier_held || !self.held_non_ctrl_keys.is_empty() {
                    *tainted = true;
                    if *pen_open {
                        *pen_open = false;
                        return Some(CtrlEvent::HoldEnd {
                            cancelled_by_other_key: true,
                        });
                    }
                    return None;
                }
                if !*tainted && !*pen_open && now_ms.saturating_sub(*down_at) >= self.hold_ms {
                    *pen_open = true;
                    return Some(CtrlEvent::HoldStart);
                }
                None
            }
            CtrlPhase::WaitingSecond { first_up_at } => {
                if now_ms.saturating_sub(*first_up_at) > self.double_tap_ms {
                    self.phase = CtrlPhase::Idle;
                }
                None
            }
            CtrlPhase::Idle => None,
        }
    }

    /// Forces pen session open immediately when user clicks outside Bar while cleanly holding
    /// second Ctrl (any state) or first Ctrl (when Bar is already active Summon).
    pub fn force_open_on_click(&mut self, bar_summoned: bool) -> bool {
        match &mut self.phase {
            CtrlPhase::SecondDown {
                tainted, pen_open, ..
            } if !*tainted && !*pen_open => {
                *pen_open = true;
                true
            }
            CtrlPhase::FirstDown {
                tainted, pen_open, ..
            } if bar_summoned && !*tainted && !*pen_open => {
                *pen_open = true;
                true
            }
            _ => false,
        }
    }

    /// Non-pen mouse button click taints single-Ctrl holds (e.g. Ctrl+Click link) and breaks pending double-tap.
    pub fn on_non_pen_mouse_button(&mut self) {
        match &mut self.phase {
            CtrlPhase::FirstDown {
                tainted, pen_open, ..
            }
            | CtrlPhase::SecondDown {
                tainted, pen_open, ..
            } => {
                if !*pen_open {
                    *tainted = true;
                }
            }
            CtrlPhase::WaitingSecond { .. } => {
                self.phase = CtrlPhase::Idle;
            }
            CtrlPhase::Idle => {}
        }
    }

    pub fn is_pen_open(&self) -> bool {
        matches!(
            self.phase,
            CtrlPhase::FirstDown { pen_open: true, .. }
                | CtrlPhase::SecondDown { pen_open: true, .. }
        )
    }

    pub fn is_ctrl_held(&self) -> bool {
        matches!(
            self.phase,
            CtrlPhase::FirstDown { .. } | CtrlPhase::SecondDown { .. }
        )
    }

    pub fn is_second_down_clean(&self) -> bool {
        matches!(self.phase, CtrlPhase::SecondDown { tainted: false, .. })
    }

    pub fn held_info(&self, now_ms: u64) -> (u64, bool) {
        match self.phase {
            CtrlPhase::FirstDown {
                down_at, tainted, ..
            }
            | CtrlPhase::SecondDown {
                down_at, tainted, ..
            } => (now_ms.saturating_sub(down_at), tainted),
            _ => (0, false),
        }
    }

    pub fn phase_tag(&self) -> &'static str {
        match self.phase {
            CtrlPhase::Idle => "Idle",
            CtrlPhase::FirstDown {
                tainted: true,
                pen_open: false,
                ..
            } => "FirstDown(tainted)",
            CtrlPhase::FirstDown { pen_open: true, .. } => "FirstDown(pen)",
            CtrlPhase::FirstDown { .. } => "FirstDown",
            CtrlPhase::WaitingSecond { .. } => "WaitingSecond",
            CtrlPhase::SecondDown {
                tainted: true,
                pen_open: false,
                ..
            } => "SecondDown(tainted)",
            CtrlPhase::SecondDown { pen_open: true, .. } => "SecondDown(pen)",
            CtrlPhase::SecondDown { .. } => "SecondDown",
        }
    }

    /// Safety net: if OS reports Ctrl is no longer physically pressed (e.g. KeyUp dropped by UAC/other hook),
    /// immediately exit selection mode and reset phase to Idle.
    pub fn force_release_ctrl(&mut self) -> Option<CtrlEvent> {
        let was_pen = self.is_pen_open();
        self.phase = CtrlPhase::Idle;
        if was_pen {
            Some(CtrlEvent::HoldEnd {
                cancelled_by_other_key: false,
            })
        } else {
            None
        }
    }
}

fn is_vk_down_os(vk: u32) -> bool {
    unsafe { (GetAsyncKeyState(vk as i32) as u16 & 0x8000) != 0 }
}

fn is_ctrl_down_os() -> bool {
    let vks = [
        VK_CONTROL.0 as u32,
        VK_LCONTROL.0 as u32,
        VK_RCONTROL.0 as u32,
    ];
    vks.iter().any(|&vk| is_vk_down_os(vk))
}

fn is_any_non_ctrl_modifier_down_os() -> bool {
    let vks = [
        VK_MENU.0 as u32,
        VK_LMENU.0 as u32,
        VK_RMENU.0 as u32,
        VK_SHIFT.0 as u32,
        VK_LSHIFT.0 as u32,
        VK_RSHIFT.0 as u32,
        VK_LWIN.0 as u32,
        VK_RWIN.0 as u32,
    ];
    vks.iter().any(|&vk| is_vk_down_os(vk))
}

fn is_ignored_pseudo_vk(vk: u32) -> bool {
    matches!(vk, 0x00 | 0xE5 | 0xE7 | 0xE8 | 0xFF)
}

/// Checks OS physical state of common chord keys (modifiers, A-Z, 0-9, F1-F12,
/// editing/navigation keys, OEM punctuation like Ctrl+.) so Ctrl+C/V/A/Z/Alt/Space/.
/// never falsely fires Close or Pen even if WH_KEYBOARD_LL misses a key event.
fn is_any_chord_key_down_os() -> bool {
    if is_any_non_ctrl_modifier_down_os() {
        return true;
    }
    for vk in 0x41u32..=0x5Au32 {
        if is_vk_down_os(vk) {
            return true;
        }
    }
    for vk in 0x30u32..=0x39u32 {
        if is_vk_down_os(vk) {
            return true;
        }
    }
    for vk in 0x70u32..=0x7Bu32 {
        // F1-F12
        if is_vk_down_os(vk) {
            return true;
        }
    }
    for vk in 0x21u32..=0x28u32 {
        // PageUp/Down, End, Home, Left/Up/Right/Down
        if is_vk_down_os(vk) {
            return true;
        }
    }
    for vk in 0xBAu32..=0xC0u32 {
        // OEM punctuation (;, =, ,, -, ., /, `) — protects Ctrl+. IME punctuation toggle
        if is_vk_down_os(vk) {
            return true;
        }
    }
    for vk in 0xDBu32..=0xDFu32 {
        // OEM brackets / backslash / quotes
        if is_vk_down_os(vk) {
            return true;
        }
    }
    const EXTRA: &[u32] = &[0x08, 0x09, 0x0D, 0x1B, 0x20, 0x2D, 0x2E]; // Backspace, Tab, Enter, Esc, Space, Insert, Delete
    EXTRA.iter().any(|&vk| is_vk_down_os(vk))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PenButton {
    /// Left button in Ctrl selection mode: tap picks single element; drag draws BLUE freehand lasso (element group, never background).
    Left,
    /// Right button in Ctrl selection mode: drag draws RED doodle screenshot capsule; right-click context menu is suppressed.
    Right,
}

const SWALLOW_NONE: u8 = 0;
const SWALLOW_LEFT: u8 = 1;
const SWALLOW_RIGHT: u8 = 2;

#[derive(Debug, Clone)]
pub enum HookEvent {
    SingleCtrlClose,
    DoubleCtrlOpen { foreground_hwnd: isize },
    OutsideClick,
    PenStart { cursor_x: i32, cursor_y: i32 },
    PenHover { x: i32, y: i32 },
    PenMouseDown { x: i32, y: i32, button: PenButton },
    PenDrag { points: Vec<(i32, i32)>, button: PenButton },
    PenTap { x: i32, y: i32 },
    PenStroke { points: Vec<(i32, i32)>, button: PenButton },
    PenEnd { picked_any: bool },
}

pub struct HookSharedConfig {
    pub bar_visible: AtomicBool,
    pub vis_tier: AtomicU8,
    pub accept_injected: AtomicBool,
    pub double_tap_ms: AtomicU64,
    pub ctrl_hold_ms: AtomicU64,
    pub bar_hwnd_raw: AtomicU64,
    pen_overlay_active: AtomicBool,
    last_cursor_x: AtomicI32,
    last_cursor_y: AtomicI32,
}

impl HookSharedConfig {
    pub fn new(accept_injected: bool, double_tap_ms: u64, ctrl_hold_ms: u64) -> Arc<Self> {
        Arc::new(Self {
            bar_visible: AtomicBool::new(false),
            vis_tier: AtomicU8::new(0),
            accept_injected: AtomicBool::new(accept_injected),
            double_tap_ms: AtomicU64::new(double_tap_ms),
            ctrl_hold_ms: AtomicU64::new(ctrl_hold_ms),
            bar_hwnd_raw: AtomicU64::new(0),
            pen_overlay_active: AtomicBool::new(false),
            last_cursor_x: AtomicI32::new(0),
            last_cursor_y: AtomicI32::new(0),
        })
    }

    pub fn set_bar_hwnd(&self, hwnd: HWND) {
        self.bar_hwnd_raw
            .store(hwnd.0 as usize as u64, Ordering::Relaxed);
    }

    pub fn set_visibility_tier(&self, tier: u8) {
        let visible = tier > 0;
        self.vis_tier.store(tier, Ordering::Release);
        self.bar_visible.store(visible, Ordering::Release);
        if !visible {
            self.pen_overlay_active.store(false, Ordering::Release);
        }
    }

    pub fn is_bar_summoned(&self) -> bool {
        self.vis_tier.load(Ordering::Acquire) == 2
    }
}

#[derive(Debug)]
enum RawSignal {
    KeyDown {
        vk: u32,
        now_ms: u64,
    },
    KeyUp {
        vk: u32,
        now_ms: u64,
    },
    MouseDown {
        x: i32,
        y: i32,
        now_ms: u64,
        swallowed: bool,
        button: PenButton,
    },
    OutsideClick,
    MouseMove {
        x: i32,
        y: i32,
    },
    MouseUp {
        x: i32,
        y: i32,
        now_ms: u64,
        swallowed: bool,
        button: PenButton,
    },
}

struct GlobalHookBridge {
    shared: Arc<HookSharedConfig>,
    raw_tx: mpsc::Sender<RawSignal>,
    epoch: Instant,
    swallowing_button: AtomicU8,
    ctrl_down_fast: AtomicBool,
    first_ctrl_tainted: AtomicBool,
    second_ctrl_fast: AtomicBool,
    last_move_sent_ms: AtomicU64,
    hook_thread_id: AtomicU32,
    mouse_hook_events: AtomicU64,
    kb_hook_events: AtomicU64,
}

static HOOK_BRIDGE: std::sync::OnceLock<GlobalHookBridge> = std::sync::OnceLock::new();

fn is_pt_inside_bar_rect(pt: POINT, bar_raw: u64) -> bool {
    if bar_raw == 0 {
        return false;
    }
    let hwnd = HWND(bar_raw as usize as *mut std::ffi::c_void);
    unsafe {
        let mut rc = RECT::default();
        if GetWindowRect(hwnd, &mut rc).is_err() {
            return false;
        }
        pt.x >= rc.left && pt.x <= rc.right && pt.y >= rc.top && pt.y <= rc.bottom
    }
}

unsafe extern "system" fn low_level_keyboard_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code >= 0 {
        if let Some(bridge) = HOOK_BRIDGE.get() {
            bridge.kb_hook_events.fetch_add(1, Ordering::Relaxed);
            let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
            if kb.dwExtraInfo != DUMMY_EXTRA_INFO {
                let is_injected = (kb.flags.0 & LLKHF_INJECTED.0) != 0;
                if !is_injected || bridge.shared.accept_injected.load(Ordering::Relaxed) {
                    // Ctrl down/up transitions are driven solely by 8ms OS physical key-state
                    // polling (GetAsyncKeyState) in dsb-hook-worker so WH_KEYBOARD_LL (which fires
                    // BEFORE Windows updates GetAsyncKeyState) never races and splits a single Ctrl
                    // press into a phantom double-tap. Here we only forward real non-Ctrl keys so
                    // even ultra-fast chords (e.g. Ctrl+C/V/A) taint immediately.
                    if !DoubleCtrlDetector::is_ctrl_vk(kb.vkCode)
                        && !is_ignored_pseudo_vk(kb.vkCode)
                    {
                        let msg = wparam.0 as u32;
                        let now_ms = bridge.epoch.elapsed().as_millis() as u64;
                        match msg {
                            WM_KEYDOWN | WM_SYSKEYDOWN => {
                                bridge.first_ctrl_tainted.store(true, Ordering::Release);
                                bridge.second_ctrl_fast.store(false, Ordering::Release);
                                if bridge.ctrl_down_fast.load(Ordering::Acquire) {
                                    bridge
                                        .shared
                                        .pen_overlay_active
                                        .store(false, Ordering::Release);
                                }
                                let _ = bridge.raw_tx.send(RawSignal::KeyDown {
                                    vk: kb.vkCode,
                                    now_ms,
                                });
                            }
                            WM_KEYUP | WM_SYSKEYUP => {
                                let _ = bridge.raw_tx.send(RawSignal::KeyUp {
                                    vk: kb.vkCode,
                                    now_ms,
                                });
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

unsafe extern "system" fn low_level_mouse_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code >= 0 {
        if let Some(bridge) = HOOK_BRIDGE.get() {
            bridge.mouse_hook_events.fetch_add(1, Ordering::Relaxed);
            let ms = &*(lparam.0 as *const MSLLHOOKSTRUCT);
            // Only skip our own synthetic events (DUMMY_EXTRA_INFO). Do NOT block LLMHF_INJECTED:
            // Logitech Options+, Windows Precision Touchpads/Styluses, remote desktop tools, and
            // Computer-Use agents legitimately set LLMHF_INJECTED on real pointer moves/clicks.
            if ms.dwExtraInfo != DUMMY_EXTRA_INFO {
                let msg = wparam.0 as u32;
                let now_ms = bridge.epoch.elapsed().as_millis() as u64;
                let x = ms.pt.x;
                let y = ms.pt.y;
                bridge.shared.last_cursor_x.store(x, Ordering::Relaxed);
                bridge.shared.last_cursor_y.store(y, Ordering::Relaxed);

                match msg {
                    WM_LBUTTONDOWN => {
                        let bar_vis = bridge.shared.bar_visible.load(Ordering::Acquire);
                        let on_bar = bar_vis
                            && is_pt_inside_bar_rect(
                                ms.pt,
                                bridge.shared.bar_hwnd_raw.load(Ordering::Relaxed),
                            );
                        let os_ctrl = is_ctrl_down_os();
                        let pen_on = bridge.shared.pen_overlay_active.load(Ordering::Acquire);
                        let other_mod = is_any_non_ctrl_modifier_down_os();
                        let fast_second_hold = os_ctrl
                            && bridge.second_ctrl_fast.load(Ordering::Acquire)
                            && !other_mod;
                        let fast_first_hold = os_ctrl
                            && bridge.shared.is_bar_summoned()
                            && bridge.ctrl_down_fast.load(Ordering::Acquire)
                            && !bridge.first_ctrl_tainted.load(Ordering::Acquire)
                            && !other_mod;

                        // Once Selection Mode (pen_on) is active, clicks anywhere — including on Anything-to-Prompt
                        // itself — are swallowed so users can pick elements inside the floating bar too!
                        let swallow = os_ctrl
                            && (pen_on || (!on_bar && (fast_second_hold || fast_first_hold)));
                        if swallow {
                            if bridge.swallowing_button.load(Ordering::Acquire) == SWALLOW_RIGHT {
                                // Ignore stray left-down while right-button red doodle is active
                                return LRESULT(1);
                            }
                            bridge
                                .shared
                                .pen_overlay_active
                                .store(true, Ordering::Release);
                            bridge
                                .swallowing_button
                                .store(SWALLOW_LEFT, Ordering::Release);
                        } else {
                            bridge
                                .swallowing_button
                                .store(SWALLOW_NONE, Ordering::Release);
                            bridge.first_ctrl_tainted.store(true, Ordering::Release);
                            if bar_vis && !on_bar {
                                let _ = bridge.raw_tx.send(RawSignal::OutsideClick);
                            }
                        }

                        let _ = bridge.raw_tx.send(RawSignal::MouseDown {
                            x,
                            y,
                            now_ms,
                            swallowed: swallow,
                            button: PenButton::Left,
                        });
                        if swallow {
                            return LRESULT(1);
                        }
                    }
                    WM_RBUTTONDOWN => {
                        let bar_vis = bridge.shared.bar_visible.load(Ordering::Acquire);
                        let on_bar = bar_vis
                            && is_pt_inside_bar_rect(
                                ms.pt,
                                bridge.shared.bar_hwnd_raw.load(Ordering::Relaxed),
                            );
                        let os_ctrl = is_ctrl_down_os();
                        let pen_on = bridge.shared.pen_overlay_active.load(Ordering::Acquire);
                        let other_mod = is_any_non_ctrl_modifier_down_os();
                        let fast_second_hold = os_ctrl
                            && bridge.second_ctrl_fast.load(Ordering::Acquire)
                            && !other_mod;
                        let fast_first_hold = os_ctrl
                            && bridge.shared.is_bar_summoned()
                            && bridge.ctrl_down_fast.load(Ordering::Acquire)
                            && !bridge.first_ctrl_tainted.load(Ordering::Acquire)
                            && !other_mod;

                        let swallow = os_ctrl
                            && (pen_on || (!on_bar && (fast_second_hold || fast_first_hold)));
                        if swallow {
                            if bridge.swallowing_button.load(Ordering::Acquire) == SWALLOW_LEFT {
                                // Suppress context menu while left-button blue lasso is active
                                return LRESULT(1);
                            }
                            bridge
                                .shared
                                .pen_overlay_active
                                .store(true, Ordering::Release);
                            bridge
                                .swallowing_button
                                .store(SWALLOW_RIGHT, Ordering::Release);
                            let _ = bridge.raw_tx.send(RawSignal::MouseDown {
                                x,
                                y,
                                now_ms,
                                swallowed: true,
                                button: PenButton::Right,
                            });
                            return LRESULT(1);
                        }

                        bridge
                            .swallowing_button
                            .store(SWALLOW_NONE, Ordering::Release);
                        bridge.first_ctrl_tainted.store(true, Ordering::Release);
                        if bar_vis && !on_bar {
                            let _ = bridge.raw_tx.send(RawSignal::OutsideClick);
                        }
                    }
                    WM_MBUTTONDOWN => {
                        let bar_vis = bridge.shared.bar_visible.load(Ordering::Acquire);
                        let on_bar = bar_vis
                            && is_pt_inside_bar_rect(
                                ms.pt,
                                bridge.shared.bar_hwnd_raw.load(Ordering::Relaxed),
                            );
                        let os_ctrl = is_ctrl_down_os();
                        let pen_on =
                            os_ctrl && bridge.shared.pen_overlay_active.load(Ordering::Acquire);
                        let fast_second_hold =
                            os_ctrl && bridge.second_ctrl_fast.load(Ordering::Acquire);
                        let fast_first_hold = os_ctrl
                            && bridge.shared.is_bar_summoned()
                            && bridge.ctrl_down_fast.load(Ordering::Acquire);
                        bridge.first_ctrl_tainted.store(true, Ordering::Release);
                        if bar_vis && !on_bar && !pen_on && !fast_second_hold && !fast_first_hold {
                            let _ = bridge.raw_tx.send(RawSignal::OutsideClick);
                        }
                    }
                    WM_MOUSEMOVE => {
                        let dragging =
                            bridge.swallowing_button.load(Ordering::Acquire) != SWALLOW_NONE;
                        let pen_on = bridge.shared.pen_overlay_active.load(Ordering::Acquire);
                        if dragging || pen_on {
                            let min_gap = if dragging { 12 } else { 14 };
                            let prev = bridge.last_move_sent_ms.load(Ordering::Relaxed);
                            if now_ms.saturating_sub(prev) >= min_gap {
                                bridge.last_move_sent_ms.store(now_ms, Ordering::Relaxed);
                                let _ = bridge.raw_tx.send(RawSignal::MouseMove { x, y });
                            }
                        }
                    }
                    WM_LBUTTONUP => {
                        let was_left = bridge
                            .swallowing_button
                            .compare_exchange(
                                SWALLOW_LEFT,
                                SWALLOW_NONE,
                                Ordering::AcqRel,
                                Ordering::Acquire,
                            )
                            .is_ok();
                        let _ = bridge.raw_tx.send(RawSignal::MouseUp {
                            x,
                            y,
                            now_ms,
                            swallowed: was_left,
                            button: PenButton::Left,
                        });
                        if was_left {
                            return LRESULT(1);
                        }
                    }
                    WM_RBUTTONUP => {
                        let was_right = bridge
                            .swallowing_button
                            .compare_exchange(
                                SWALLOW_RIGHT,
                                SWALLOW_NONE,
                                Ordering::AcqRel,
                                Ordering::Acquire,
                            )
                            .is_ok();
                        let pen_still_on = bridge.shared.pen_overlay_active.load(Ordering::Acquire);
                        if was_right {
                            let _ = bridge.raw_tx.send(RawSignal::MouseUp {
                                x,
                                y,
                                now_ms,
                                swallowed: true,
                                button: PenButton::Right,
                            });
                            return LRESULT(1);
                        }
                        // Even if a stray WM_RBUTTONUP arrives while selection mode is active,
                        // swallow it so DefWindowProc never synthesizes WM_CONTEXTMENU.
                        if pen_still_on {
                            return LRESULT(1);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

pub fn start_input_hooks(shared: Arc<HookSharedConfig>, event_tx: mpsc::Sender<HookEvent>) {
    let (raw_tx, raw_rx) = mpsc::channel::<RawSignal>();
    let epoch = Instant::now();

    let mut cur_pt = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut cur_pt);
    }
    shared.last_cursor_x.store(cur_pt.x, Ordering::Relaxed);
    shared.last_cursor_y.store(cur_pt.y, Ordering::Relaxed);

    let _ = HOOK_BRIDGE.set(GlobalHookBridge {
        shared: shared.clone(),
        raw_tx,
        epoch,
        swallowing_button: AtomicU8::new(SWALLOW_NONE),
        ctrl_down_fast: AtomicBool::new(false),
        first_ctrl_tainted: AtomicBool::new(false),
        second_ctrl_fast: AtomicBool::new(false),
        last_move_sent_ms: AtomicU64::new(0),
        hook_thread_id: AtomicU32::new(0),
        mouse_hook_events: AtomicU64::new(0),
        kb_hook_events: AtomicU64::new(0),
    });

    std::thread::Builder::new()
        .name("dsb-win32-hooks".to_string())
        .spawn(move || unsafe {
            let tid = GetCurrentThreadId();
            if let Some(b) = HOOK_BRIDGE.get() {
                b.hook_thread_id.store(tid, Ordering::Release);
            }
            // Keep the low-level Win32 hook message pump above background NTFS/index threads so
            // startup CPU spikes can never stall WH_MOUSE_LL / WH_KEYBOARD_LL callbacks.
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST);
            let hmod: Option<HINSTANCE> = GetModuleHandleW(None).ok().map(HINSTANCE::from);

            let install_pair = || -> Option<(HHOOK, HHOOK)> {
                let kb = match SetWindowsHookExW(
                    WH_KEYBOARD_LL,
                    Some(low_level_keyboard_proc),
                    hmod,
                    0,
                ) {
                    Ok(h) => h,
                    Err(e) => {
                        log::error!("SetWindowsHookExW(WH_KEYBOARD_LL) failed: {e}");
                        return None;
                    }
                };
                let ms = match SetWindowsHookExW(WH_MOUSE_LL, Some(low_level_mouse_proc), hmod, 0) {
                    Ok(h) => h,
                    Err(e) => {
                        log::error!("SetWindowsHookExW(WH_MOUSE_LL) failed: {e}");
                        let _ = UnhookWindowsHookEx(kb);
                        return None;
                    }
                };
                Some((kb, ms))
            };

            let Some((mut kb_hook, mut ms_hook)) = install_pair() else {
                return;
            };

            log::info!("Installed WH_KEYBOARD_LL and WH_MOUSE_LL hooks successfully (tid={tid}).");
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, Some(HWND::default()), 0, 0).as_bool() {
                if msg.message == WM_APP_REINSTALL_HOOKS {
                    let _ = UnhookWindowsHookEx(ms_hook);
                    let _ = UnhookWindowsHookEx(kb_hook);
                    if let Some((new_kb, new_ms)) = install_pair() {
                        kb_hook = new_kb;
                        ms_hook = new_ms;
                        log::info!("[HOOK-WATCHDOG] Reinstalled WH_KEYBOARD_LL and WH_MOUSE_LL successfully (tid={tid}).");
                    } else {
                        log::error!("[HOOK-WATCHDOG] Failed to reinstall low-level hooks!");
                        return;
                    }
                    continue;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }

            log::warn!("dsb-win32-hooks message loop exited unexpectedly; unhooking.");
            let _ = UnhookWindowsHookEx(ms_hook);
            let _ = UnhookWindowsHookEx(kb_hook);
        })
        .expect("failed to spawn hook thread");

    std::thread::Builder::new()
        .name("dsb-hook-worker".to_string())
        .spawn(move || {
            let mut ctrl = DoubleCtrlDetector::new(
                shared.double_tap_ms.load(Ordering::Relaxed),
                shared.ctrl_hold_ms.load(Ordering::Relaxed),
            );
            let mut last_double_fg: isize = 0;
            let mut picked_in_session = false;

            let mut stroke_points: Vec<(i32, i32)> = Vec::new();
            let mut active_button: Option<PenButton> = None;
            let mut down_at_ms: Option<u64> = None;
            let mut max_drag_dist_sq: i64 = 0;
            let mut ctrl_physically_down = false;

            // Watchdog state: detects if Windows silently evicted WH_MOUSE_LL (e.g. after UAC/desktop switch)
            let mut wd_last_check_ms: u64 = 0;
            let mut wd_last_reinstall_ms: u64 = 0;
            let mut wd_prev_os_pt: (i32, i32) = (cur_pt.x, cur_pt.y);
            let mut wd_prev_mouse_events: u64 = 0;

            let handle_ctrl_event = |ev: CtrlEvent,
                                     picked: &mut bool,
                                     strokes: &mut Vec<(i32, i32)>,
                                     act_btn: &mut Option<PenButton>,
                                     down_ms: &mut Option<u64>,
                                     drag_dist_sq: &mut i64,
                                     fg_hwnd: isize| {
                match ev {
                    CtrlEvent::SingleTapClose => {
                        let _ = event_tx.send(HookEvent::SingleCtrlClose);
                    }
                    CtrlEvent::DoubleTapOpen => {
                        let _ = event_tx.send(HookEvent::DoubleCtrlOpen {
                            foreground_hwnd: fg_hwnd,
                        });
                    }
                    CtrlEvent::HoldStart => {
                        shared.pen_overlay_active.store(true, Ordering::Release);
                        *picked = false;
                        // Query OS cursor position fresh so PenStart never uses a stale startup coord
                        let mut live_pt = POINT::default();
                        let (cx, cy) = unsafe {
                            if GetCursorPos(&mut live_pt).is_ok() {
                                shared.last_cursor_x.store(live_pt.x, Ordering::Relaxed);
                                shared.last_cursor_y.store(live_pt.y, Ordering::Relaxed);
                                (live_pt.x, live_pt.y)
                            } else {
                                (
                                    shared.last_cursor_x.load(Ordering::Relaxed),
                                    shared.last_cursor_y.load(Ordering::Relaxed),
                                )
                            }
                        };
                        let _ = event_tx.send(HookEvent::PenStart {
                            cursor_x: cx,
                            cursor_y: cy,
                        });
                    }
                    CtrlEvent::HoldEnd {
                        cancelled_by_other_key,
                    } => {
                        shared.pen_overlay_active.store(false, Ordering::Release);
                        // If user released Ctrl slightly before releasing mouse button during a click-pick/drag,
                        // commit the in-progress tap/stroke instead of silently discarding it!
                        if !cancelled_by_other_key && !strokes.is_empty() {
                            let btn = act_btn.unwrap_or(PenButton::Left);
                            let is_tap = classify_tap_or_stroke(*drag_dist_sq, 0);
                            if is_tap {
                                // Only Left tap performs tap-pick; Right short-click stays suppressed no-op
                                if btn == PenButton::Left {
                                    *picked = true;
                                    if let Some(&(sx, sy)) = strokes.first() {
                                        let _ = event_tx.send(HookEvent::PenTap { x: sx, y: sy });
                                    }
                                }
                            } else {
                                *picked = true;
                                let pts = std::mem::take(strokes);
                                let _ = event_tx.send(HookEvent::PenStroke {
                                    points: pts,
                                    button: btn,
                                });
                            }
                        }
                        strokes.clear();
                        *act_btn = None;
                        *down_ms = None;
                        *drag_dist_sq = 0;
                        let _ = event_tx.send(HookEvent::PenEnd {
                            picked_any: *picked,
                        });
                        *picked = false;
                    }
                }
            };

            loop {
                ctrl.double_tap_ms = shared.double_tap_ms.load(Ordering::Relaxed);
                ctrl.hold_ms = shared.ctrl_hold_ms.load(Ordering::Relaxed).clamp(150, 600);

                // Wait up to 8ms (125 Hz) for non-Ctrl/mouse signals, drain all queued signals,
                // then read OS physical Ctrl state (single source of truth — zero LL-vs-OS races).
                let first_sig = match raw_rx.recv_timeout(Duration::from_millis(8)) {
                    Ok(sig) => Some(sig),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };

                let mut batch = Vec::new();
                if let Some(s) = first_sig {
                    batch.push(s);
                    while let Ok(more) = raw_rx.try_recv() {
                        batch.push(more);
                    }
                }

                for sig in batch {
                    match sig {
                        RawSignal::KeyDown { vk, now_ms } => {
                            if DoubleCtrlDetector::is_ctrl_vk(vk) || is_ignored_pseudo_vk(vk) {
                                continue;
                            }
                            let was_ctrl_held = ctrl.is_ctrl_held();
                            let was_tainted = ctrl.held_info(now_ms).1;
                            let chord_down = is_any_chord_key_down_os();
                            let ev = ctrl.on_key_down(vk, now_ms, chord_down);
                            if was_ctrl_held && !was_tainted {
                                log::info!(
                                    "[CTRL] tainted by non-ctrl vk=0x{vk:X} -> phase={} ev={:?}",
                                    ctrl.phase_tag(),
                                    ev
                                );
                            }
                            if let Some(ctrl_ev) = ev {
                                handle_ctrl_event(
                                    ctrl_ev,
                                    &mut picked_in_session,
                                    &mut stroke_points,
                                    &mut active_button,
                                    &mut down_at_ms,
                                    &mut max_drag_dist_sq,
                                    last_double_fg,
                                );
                            }
                        }

                        RawSignal::KeyUp { vk, now_ms } => {
                            if DoubleCtrlDetector::is_ctrl_vk(vk) || is_ignored_pseudo_vk(vk) {
                                continue;
                            }
                            let bar_summoned = shared.is_bar_summoned();
                            if let Some(ctrl_ev) = ctrl.on_key_up(vk, now_ms, bar_summoned) {
                                handle_ctrl_event(
                                    ctrl_ev,
                                    &mut picked_in_session,
                                    &mut stroke_points,
                                    &mut active_button,
                                    &mut down_at_ms,
                                    &mut max_drag_dist_sq,
                                    last_double_fg,
                                );
                            }
                        }

                        RawSignal::OutsideClick => {
                            ctrl.on_non_pen_mouse_button();
                            if !ctrl.is_pen_open() {
                                let _ = event_tx.send(HookEvent::OutsideClick);
                            }
                        }

                        RawSignal::MouseDown {
                            x,
                            y,
                            now_ms,
                            swallowed,
                            button,
                        } => {
                            if swallowed {
                                let bar_summoned = shared.is_bar_summoned();
                                if !ctrl.is_pen_open() && ctrl.force_open_on_click(bar_summoned) {
                                    picked_in_session = false;
                                    let _ = event_tx.send(HookEvent::PenStart {
                                        cursor_x: x,
                                        cursor_y: y,
                                    });
                                }
                                if ctrl.is_pen_open() {
                                    active_button = Some(button);
                                    down_at_ms = Some(now_ms);
                                    stroke_points.clear();
                                    stroke_points.push((x, y));
                                    max_drag_dist_sq = 0;
                                    let _ = event_tx.send(HookEvent::PenMouseDown { x, y, button });
                                }
                            } else {
                                ctrl.on_non_pen_mouse_button();
                            }
                        }

                        RawSignal::MouseMove { x, y } => {
                            if !ctrl.is_pen_open() {
                                continue;
                            }
                            if let Some((sx, sy)) = stroke_points.first().copied() {
                                let dx = (x - sx) as i64;
                                let dy = (y - sy) as i64;
                                max_drag_dist_sq = max_drag_dist_sq.max(dx * dx + dy * dy);
                                stroke_points.push((x, y));
                                if max_drag_dist_sq > 64 {
                                    let _ = event_tx.send(HookEvent::PenDrag {
                                        points: stroke_points.clone(),
                                        button: active_button.unwrap_or(PenButton::Left),
                                    });
                                }
                            } else {
                                let _ = event_tx.send(HookEvent::PenHover { x, y });
                            }
                        }

                        RawSignal::MouseUp {
                            x,
                            y,
                            now_ms,
                            swallowed,
                            button,
                        } => {
                            if swallowed && !stroke_points.is_empty() {
                                let btn = active_button.unwrap_or(button);
                                let dur =
                                    down_at_ms.map(|t0| now_ms.saturating_sub(t0)).unwrap_or(0);
                                let is_tap = classify_tap_or_stroke(max_drag_dist_sq, dur);
                                if is_tap {
                                    // Left tap -> single-element pick; Right short tap -> suppressed no-op (no context menu!)
                                    if btn == PenButton::Left {
                                        picked_in_session = true;
                                        let _ = event_tx.send(HookEvent::PenTap { x, y });
                                    }
                                } else {
                                    picked_in_session = true;
                                    let mut pts = std::mem::take(&mut stroke_points);
                                    pts.push((x, y));
                                    let _ = event_tx.send(HookEvent::PenStroke {
                                        points: pts,
                                        button: btn,
                                    });
                                }
                            }
                            stroke_points.clear();
                            active_button = None;
                            down_at_ms = None;
                            max_drag_dist_sq = 0;
                        }
                    }
                }

                let now_ms = epoch.elapsed().as_millis() as u64;
                let bar_summoned = shared.is_bar_summoned();
                let os_ctrl = is_ctrl_down_os();
                let chord_down = is_any_chord_key_down_os();

                // Single source of truth for Ctrl edges via 8ms OS physical key-state polling:
                // Never blocked by WebView2/UIPI/IME, and never races with WH_KEYBOARD_LL.
                if os_ctrl && !ctrl_physically_down {
                    ctrl_physically_down = true;
                    last_double_fg = unsafe { GetForegroundWindow().0 as isize };
                    ctrl.prune_released_keys(is_vk_down_os);
                    let ev = ctrl.on_key_down(VK_LCONTROL_U32, now_ms, chord_down);
                    if let Some(b) = HOOK_BRIDGE.get() {
                        b.ctrl_down_fast.store(true, Ordering::Release);
                        b.first_ctrl_tainted
                            .store(chord_down || ctrl.held_info(now_ms).1, Ordering::Release);
                        b.second_ctrl_fast
                            .store(ctrl.is_second_down_clean(), Ordering::Release);
                    }
                    log::info!(
                        "[CTRL] down summoned={} chord={} phase={}",
                        bar_summoned,
                        chord_down,
                        ctrl.phase_tag()
                    );
                    if let Some(ctrl_ev) = ev {
                        handle_ctrl_event(
                            ctrl_ev,
                            &mut picked_in_session,
                            &mut stroke_points,
                            &mut active_button,
                            &mut down_at_ms,
                            &mut max_drag_dist_sq,
                            last_double_fg,
                        );
                    }
                } else if !os_ctrl && (ctrl_physically_down || ctrl.is_ctrl_held()) {
                    ctrl_physically_down = false;
                    if let Some(b) = HOOK_BRIDGE.get() {
                        b.ctrl_down_fast.store(false, Ordering::Release);
                        b.second_ctrl_fast.store(false, Ordering::Release);
                        b.first_ctrl_tainted.store(false, Ordering::Release);
                    }
                    shared.pen_overlay_active.store(false, Ordering::Release);
                    let (dur_ms, was_tainted) = ctrl.held_info(now_ms);
                    let ev = ctrl.on_key_up(VK_LCONTROL_U32, now_ms, bar_summoned);
                    log::info!(
                        "[CTRL] up summoned={} dur={}ms tainted={} -> {:?} (next={})",
                        bar_summoned,
                        dur_ms,
                        was_tainted,
                        ev,
                        ctrl.phase_tag()
                    );
                    if let Some(ctrl_ev) = ev {
                        handle_ctrl_event(
                            ctrl_ev,
                            &mut picked_in_session,
                            &mut stroke_points,
                            &mut active_button,
                            &mut down_at_ms,
                            &mut max_drag_dist_sq,
                            last_double_fg,
                        );
                    }
                }

                if ctrl.is_ctrl_held() {
                    ctrl.prune_released_keys(is_vk_down_os);
                }

                if let Some(ctrl_ev) = ctrl.poll_hold(now_ms, bar_summoned, chord_down) {
                    log::info!(
                        "[CTRL] poll_hold summoned={} chord={} -> {:?}",
                        bar_summoned,
                        chord_down,
                        ctrl_ev
                    );
                    handle_ctrl_event(
                        ctrl_ev,
                        &mut picked_in_session,
                        &mut stroke_points,
                        &mut active_button,
                        &mut down_at_ms,
                        &mut max_drag_dist_sq,
                        last_double_fg,
                    );
                }
                if let Some(b) = HOOK_BRIDGE.get() {
                    if !ctrl.is_second_down_clean() {
                        b.second_ctrl_fast.store(false, Ordering::Release);
                    }
                    if ctrl.is_ctrl_held() && ctrl.held_info(now_ms).1 {
                        b.first_ctrl_tainted.store(true, Ordering::Release);
                    }
                    if now_ms.saturating_sub(wd_last_check_ms) >= 500 {
                        wd_last_check_ms = now_ms;
                        let mut os_pt = POINT::default();
                        if unsafe { GetCursorPos(&mut os_pt).is_ok() } {
                            let cur_events = b.mouse_hook_events.load(Ordering::Relaxed);
                            if should_trigger_hook_watchdog(
                                wd_prev_os_pt,
                                (os_pt.x, os_pt.y),
                                wd_prev_mouse_events,
                                cur_events,
                                now_ms,
                                wd_last_reinstall_ms,
                            ) {
                                let tid = b.hook_thread_id.load(Ordering::Acquire);
                                if tid != 0 {
                                    wd_last_reinstall_ms = now_ms;
                                    log::warn!(
                                        "[HOOK-WATCHDOG] OS cursor moved {:?} -> ({},{}) with 0 WH_MOUSE_LL events (total={cur_events}); requesting hook reinstall on tid={tid}",
                                        wd_prev_os_pt,
                                        os_pt.x,
                                        os_pt.y
                                    );
                                    unsafe {
                                        let _ = PostThreadMessageW(
                                            tid,
                                            WM_APP_REINSTALL_HOOKS,
                                            WPARAM(0),
                                            LPARAM(0),
                                        );
                                    }
                                }
                            }
                            wd_prev_os_pt = (os_pt.x, os_pt.y);
                            wd_prev_mouse_events = cur_events;
                        }
                    }
                }
            }
        })
        .expect("failed to spawn hook worker thread");
}

/// Pure helper deciding whether the hook watchdog should reinstall WH_KEYBOARD_LL/WH_MOUSE_LL:
/// fires when OS cursor physically moved >= 20px Manhattan distance while WH_MOUSE_LL delivered
/// zero callbacks, throttled by a 3-second cooldown so it can never spam.
pub fn should_trigger_hook_watchdog(
    prev_os_pt: (i32, i32),
    cur_os_pt: (i32, i32),
    prev_mouse_events: u64,
    cur_mouse_events: u64,
    now_ms: u64,
    last_reinstall_ms: u64,
) -> bool {
    let manhattan =
        (cur_os_pt.0 - prev_os_pt.0).abs() + (cur_os_pt.1 - prev_os_pt.1).abs();
    manhattan >= 20
        && cur_mouse_events == prev_mouse_events
        && (last_reinstall_ms == 0 || now_ms.saturating_sub(last_reinstall_ms) >= 3000)
}

/// Returns `true` if a pen gesture is a tap (click), `false` if freehand stroke.
/// Any click with <= 10px movement is always treated as a tap, even if held deliberately (>250ms).
pub fn classify_tap_or_stroke(max_drag_dist_sq: i64, _duration_ms: u64) -> bool {
    max_drag_dist_sq <= 100
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenario_unlaunched_vs_launched_ctrl_matrix() {
        let mut d = DoubleCtrlDetector::new(400, 220);

        // 1. Unlaunched (Hidden): single tap & single hold are ignored
        assert_eq!(d.on_key_down(VK_LCONTROL_U32, 100, false), None);
        assert_eq!(d.on_key_up(VK_LCONTROL_U32, 180, false), None);
        assert_eq!(d.poll_hold(700, false, false), None);
        assert_eq!(d.on_key_down(VK_LCONTROL_U32, 800, false), None);
        assert_eq!(d.poll_hold(1200, false, false), None);
        assert!(!d.is_pen_open());
        assert_eq!(d.on_key_up(VK_LCONTROL_U32, 1300, false), None);

        // 2. Unlaunched -> double-tap Ctrl opens Summon
        assert_eq!(d.on_key_down(VK_LCONTROL_U32, 2000, false), None);
        assert_eq!(d.on_key_up(VK_LCONTROL_U32, 2070, false), None);
        assert_eq!(d.on_key_down(VK_LCONTROL_U32, 2180, false), None);
        assert_eq!(
            d.on_key_up(VK_LCONTROL_U32, 2260, false),
            Some(CtrlEvent::DoubleTapOpen)
        );

        // 3. Launched (Summon) -> single tap Ctrl immediately closes!
        assert_eq!(d.on_key_down(VK_LCONTROL_U32, 3000, false), None);
        assert_eq!(
            d.on_key_up(VK_LCONTROL_U32, 3110, true),
            Some(CtrlEvent::SingleTapClose)
        );
    }

    #[test]
    fn scenario_double_tap_hold_pick_capsule_then_single_ctrl_exits_first_try() {
        let mut d = DoubleCtrlDetector::new(400, 220);

        // From Hidden: short tap + long hold enters selection mode
        assert_eq!(d.on_key_down(VK_LCONTROL_U32, 100, false), None);
        assert_eq!(d.on_key_up(VK_LCONTROL_U32, 170, false), None);
        assert_eq!(d.on_key_down(VK_LCONTROL_U32, 280, false), None);
        assert_eq!(d.poll_hold(500, false, false), Some(CtrlEvent::HoldStart));
        assert!(d.is_pen_open());

        // Windows auto-repeat while holding Ctrl does NOT cancel pen
        assert_eq!(d.on_key_down(VK_LCONTROL_U32, 800, false), None);
        assert!(d.is_pen_open());

        // User picked capsule and releases Ctrl (Bar is now Summon = true):
        // Release exits selection mode cleanly (HoldEnd) and stays in Summon!
        assert_eq!(
            d.on_key_up(VK_LCONTROL_U32, 1200, true),
            Some(CtrlEvent::HoldEnd {
                cancelled_by_other_key: false
            })
        );
        assert!(!d.is_pen_open());

        // User immediately single-taps Ctrl to exit -> closes on the VERY FIRST TAP!
        assert_eq!(d.on_key_down(VK_LCONTROL_U32, 1500, false), None);
        assert_eq!(
            d.on_key_up(VK_LCONTROL_U32, 1590, true),
            Some(CtrlEvent::SingleTapClose)
        );
    }

    #[test]
    fn scenario_typing_unsent_and_deleting_text_with_chords_then_single_ctrl_close_or_hold() {
        let mut d = DoubleCtrlDetector::new(400, 220);

        // Bar is Summon: user types "hi", uses Ctrl+A / Ctrl+C / Ctrl+V / Backspace / Delete
        for (idx, vk) in [0x48u32, 0x49u32, 0x08u32, 0x2Eu32].iter().enumerate() {
            let t = 100 + (idx as u64) * 50;
            assert_eq!(d.on_key_down(*vk, t, false), None);
            assert_eq!(d.on_key_up(*vk, t + 20, true), None);
        }

        // Ctrl+A (select all) + Delete -> MUST NOT close Bar or enter Pen!
        assert_eq!(d.on_key_down(VK_LCONTROL_U32, 500, false), None);
        assert_eq!(d.on_key_down(0x41, 530, false), None); // 'A'
        assert_eq!(d.on_key_up(0x41, 560, true), None);
        assert_eq!(d.on_key_up(VK_LCONTROL_U32, 600, true), None);
        assert_eq!(d.on_key_down(0x2E, 650, false), None); // Delete
        assert_eq!(d.on_key_up(0x2E, 670, true), None);

        // Now after deleting all text: long-press Ctrl enters selection mode, release keeps Bar open
        assert_eq!(d.on_key_down(VK_LCONTROL_U32, 1000, false), None);
        assert_eq!(d.poll_hold(1230, true, false), Some(CtrlEvent::HoldStart));
        assert!(d.is_pen_open());
        assert_eq!(
            d.on_key_up(VK_LCONTROL_U32, 1600, true),
            Some(CtrlEvent::HoldEnd {
                cancelled_by_other_key: false
            })
        );

        // And single-tap Ctrl closes immediately!
        assert_eq!(d.on_key_down(VK_LCONTROL_U32, 2000, false), None);
        assert_eq!(
            d.on_key_up(VK_LCONTROL_U32, 2090, true),
            Some(CtrlEvent::SingleTapClose)
        );
    }

    #[test]
    fn scenario_chatterfly_ctrl_alt_and_ctrl_click_never_trigger_close_or_pen() {
        let mut d = DoubleCtrlDetector::new(400, 220);

        // Chatterfly Ctrl+Alt while Bar is Summon -> NEVER closes Bar and NEVER enters Pen!
        assert_eq!(d.on_key_down(VK_LCONTROL_U32, 1000, false), None);
        assert_eq!(d.on_key_down(VK_LMENU.0 as u32, 1020, true), None);
        assert_eq!(d.poll_hold(1500, true, true), None);
        assert!(!d.is_pen_open());
        assert_eq!(d.on_key_up(VK_LMENU.0 as u32, 1600, true), None);
        assert_eq!(d.on_key_up(VK_LCONTROL_U32, 1620, true), None);

        // Lone Alt held forever -> completely ignored
        assert_eq!(d.on_key_down(VK_LMENU.0 as u32, 2000, true), None);
        assert_eq!(d.poll_hold(2600, true, true), None);
        assert!(!d.is_pen_open());
        assert_eq!(d.on_key_up(VK_LMENU.0 as u32, 2700, true), None);

        // Ctrl+Click when Bar is Hidden does not arm double-tap
        d.on_key_down(VK_LCONTROL_U32, 4000, false);
        d.on_non_pen_mouse_button();
        assert_eq!(d.on_key_up(VK_LCONTROL_U32, 4120, false), None);
        d.on_key_down(VK_LCONTROL_U32, 4200, false);
        assert_eq!(d.on_key_up(VK_LCONTROL_U32, 4260, false), None);
    }

    #[test]
    fn fast_click_outside_opens_pen_on_second_ctrl_or_summoned_first_ctrl() {
        let mut d = DoubleCtrlDetector::new(400, 220);
        // Second Ctrl fast click (Bar hidden)
        d.on_key_down(VK_LCONTROL_U32, 100, false);
        d.on_key_up(VK_LCONTROL_U32, 160, false);
        d.on_key_down(VK_LCONTROL_U32, 260, false);
        assert!(d.force_open_on_click(false));
        assert!(d.is_pen_open());
        assert_eq!(
            d.on_key_up(VK_LCONTROL_U32, 500, true),
            Some(CtrlEvent::HoldEnd {
                cancelled_by_other_key: false
            })
        );

        // First Ctrl fast click when Bar is Summon
        d.on_key_down(VK_LCONTROL_U32, 1000, false);
        assert!(d.force_open_on_click(true));
        assert!(d.is_pen_open());
        assert_eq!(
            d.on_key_up(VK_LCONTROL_U32, 1200, true),
            Some(CtrlEvent::HoldEnd {
                cancelled_by_other_key: false
            })
        );
    }

    #[test]
    fn force_release_ctrl_exits_pen_mode_if_keyup_was_dropped() {
        let mut d = DoubleCtrlDetector::new(400, 220);
        d.on_key_down(VK_LCONTROL_U32, 100, false);
        d.on_key_up(VK_LCONTROL_U32, 160, false);
        d.on_key_down(VK_LCONTROL_U32, 260, false);
        assert_eq!(d.poll_hold(500, false, false), Some(CtrlEvent::HoldStart));
        assert!(d.is_pen_open());

        assert_eq!(
            d.force_release_ctrl(),
            Some(CtrlEvent::HoldEnd {
                cancelled_by_other_key: false
            })
        );
        assert!(!d.is_pen_open());
    }

    #[test]
    fn classifies_pen_tap_vs_freehand_stroke() {
        assert!(classify_tap_or_stroke(16, 120));
        assert!(classify_tap_or_stroke(16, 350)); // deliberate slow click without dragging stays Tap
        assert!(!classify_tap_or_stroke(144, 120));
        assert!(!classify_tap_or_stroke(144, 300));
    }

    #[test]
    fn hook_watchdog_triggers_only_when_cursor_moves_with_zero_hook_events() {
        // Cursor stationary -> no reinstall
        assert!(!should_trigger_hook_watchdog((500, 500), (505, 504), 10, 10, 1000, 0));
        // Cursor moved AND WH_MOUSE_LL events incremented -> healthy, no reinstall
        assert!(!should_trigger_hook_watchdog((500, 500), (620, 540), 10, 14, 1000, 0));
        // Cursor moved >= 20px with 0 new WH_MOUSE_LL events -> trigger self-heal!
        assert!(should_trigger_hook_watchdog((500, 500), (620, 540), 10, 10, 1000, 0));
        // Inside 3000ms cooldown -> throttled
        assert!(!should_trigger_hook_watchdog((620, 540), (800, 600), 10, 10, 2500, 1000));
        // After 3000ms cooldown -> allowed again
        assert!(should_trigger_hook_watchdog((620, 540), (800, 600), 10, 10, 4000, 1000));
    }
}
