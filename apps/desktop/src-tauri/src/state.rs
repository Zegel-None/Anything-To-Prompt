use crate::config::DockEdge;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisibilityState {
    Hidden,
    Lip,
    DockedCollapsed,
    Summon,
}

impl VisibilityState {
    pub fn tier_u8(self) -> u8 {
        match self {
            VisibilityState::Hidden => 0,
            VisibilityState::Lip | VisibilityState::DockedCollapsed => 1,
            VisibilityState::Summon => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeMode {
    Lip,
    Summon,
    Expanded,
}

impl BridgeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            BridgeMode::Lip => "lip",
            BridgeMode::Summon => "summon",
            BridgeMode::Expanded => "expanded",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowAction {
    Hide,
    ShowNoActivate,
    ActivateAndFocus,
    DockCollapse,
    DockExpandNoActivate,
    DockExpandAndFocus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionEffects {
    pub window_action: Option<WindowAction>,
    pub bridge_mode: Option<BridgeMode>,
    pub reset_pick_counter: bool,
    pub record_sent_message: bool,
}

impl TransitionEffects {
    pub fn none() -> Self {
        Self {
            window_action: None,
            bridge_mode: None,
            reset_pick_counter: false,
            record_sent_message: false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum StateEvent {
    SingleCtrlClose,
    DoubleCtrlOpen,
    OutsideClickHide,
    StepDown,
    Submitted,
    AssistantActivity {
        session_id: String,
        assistant_id: String,
        streaming: bool,
    },
    /// Background reply from another conversation while Bar is Hidden -> wake Lip notification bubble only.
    IncomingBubbleNotice,
    UiUpdated {
        hovered: bool,
        pinned: bool,
        popup_open: bool,
        modal: bool,
        composer_focused: bool,
    },
    SummonFromLip,
    ComposerPointerDown,
    PenStarted,
    PenEnded {
        picked_any: bool,
        cancelled_by_other_key: bool,
    },
    PickInserted,
    TrayToggle,
    DevForceState(DevTargetState),
    SetDockEdge {
        edge: DockEdge,
        start_collapsed: bool,
    },
    SetDockEdgeFromDrag {
        edge: DockEdge,
    },
    DockEdgePeekEnter,
    DockEdgeClickExpand,
    DockAutoCollapseIfUnlocked,
    FullscreenChanged {
        active: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DevTargetState {
    Hidden,
    Lip,
    Summon,
    Expanded,
}

impl DevTargetState {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "hidden" => Some(Self::Hidden),
            "lip" => Some(Self::Lip),
            "summon" => Some(Self::Summon),
            "expanded" => Some(Self::Expanded),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BarStateMachine {
    pub visibility: VisibilityState,
    pub pinned: bool,
    pub popup_open: bool,
    pub modal: bool,
    pub composer_focused: bool,
    pub pen_active: bool,
    pub dock_edge: DockEdge,
    pub dock_locked_open: bool,
    pub fullscreen_suppressed: bool,
    last_session_id: Option<String>,
    last_assistant_id: String,
    last_streaming: bool,
}

impl Default for BarStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl BarStateMachine {
    pub fn new() -> Self {
        Self {
            visibility: VisibilityState::Hidden,
            pinned: false,
            popup_open: false,
            modal: false,
            composer_focused: false,
            pen_active: false,
            dock_edge: DockEdge::None,
            dock_locked_open: false,
            fullscreen_suppressed: false,
            last_session_id: None,
            last_assistant_id: String::new(),
            last_streaming: false,
        }
    }

    pub fn is_visible(&self) -> bool {
        !matches!(self.visibility, VisibilityState::Hidden)
    }

    fn collapse_or_hide(&mut self) -> TransitionEffects {
        self.pinned = false;
        self.popup_open = false;
        self.composer_focused = false;
        self.dock_locked_open = false;
        if self.dock_edge.is_docked() {
            if self.fullscreen_suppressed {
                self.visibility = VisibilityState::Hidden;
                return TransitionEffects {
                    window_action: Some(WindowAction::Hide),
                    bridge_mode: None,
                    reset_pick_counter: false,
                    record_sent_message: false,
                };
            }
            self.visibility = VisibilityState::DockedCollapsed;
            TransitionEffects {
                window_action: Some(WindowAction::DockCollapse),
                bridge_mode: None,
                reset_pick_counter: false,
                record_sent_message: false,
            }
        } else {
            self.visibility = VisibilityState::Hidden;
            TransitionEffects {
                window_action: Some(WindowAction::Hide),
                bridge_mode: None,
                reset_pick_counter: false,
                record_sent_message: false,
            }
        }
    }

    pub fn apply(&mut self, event: StateEvent) -> TransitionEffects {
        match event {
            StateEvent::SingleCtrlClose => {
                if self.pen_active
                    || self.modal
                    || self.pinned
                    || self.visibility != VisibilityState::Summon
                {
                    return TransitionEffects::none();
                }
                self.collapse_or_hide()
            }

            StateEvent::DoubleCtrlOpen => {
                self.visibility = VisibilityState::Summon;
                self.dock_locked_open = true;
                TransitionEffects {
                    window_action: Some(if self.dock_edge.is_docked() {
                        WindowAction::DockExpandAndFocus
                    } else {
                        WindowAction::ActivateAndFocus
                    }),
                    bridge_mode: Some(BridgeMode::Summon),
                    reset_pick_counter: false,
                    record_sent_message: false,
                }
            }

            StateEvent::SummonFromLip => {
                self.visibility = VisibilityState::Summon;
                self.dock_locked_open = true;
                TransitionEffects {
                    window_action: Some(if self.dock_edge.is_docked() {
                        WindowAction::DockExpandAndFocus
                    } else {
                        WindowAction::ActivateAndFocus
                    }),
                    bridge_mode: Some(BridgeMode::Summon),
                    reset_pick_counter: false,
                    record_sent_message: false,
                }
            }

            StateEvent::OutsideClickHide => {
                if self.pen_active
                    || self.modal
                    || self.pinned
                    || self.visibility == VisibilityState::Hidden
                    || self.visibility == VisibilityState::DockedCollapsed
                {
                    return TransitionEffects::none();
                }
                self.collapse_or_hide()
            }

            StateEvent::TrayToggle => match self.visibility {
                VisibilityState::Hidden | VisibilityState::DockedCollapsed => {
                    self.visibility = VisibilityState::Summon;
                    self.dock_locked_open = true;
                    TransitionEffects {
                        window_action: Some(if self.dock_edge.is_docked() {
                            WindowAction::DockExpandAndFocus
                        } else {
                            WindowAction::ActivateAndFocus
                        }),
                        bridge_mode: Some(BridgeMode::Summon),
                        reset_pick_counter: false,
                        record_sent_message: false,
                    }
                }
                _ => self.collapse_or_hide(),
            },

            StateEvent::StepDown => {
                if self.pen_active || self.modal || self.pinned {
                    return TransitionEffects::none();
                }
                match self.visibility {
                    VisibilityState::Hidden | VisibilityState::DockedCollapsed => {
                        TransitionEffects::none()
                    }
                    VisibilityState::Summon | VisibilityState::Lip => self.collapse_or_hide(),
                }
            }

            StateEvent::Submitted => {
                let mut fx = TransitionEffects {
                    window_action: None,
                    bridge_mode: None,
                    reset_pick_counter: true,
                    record_sent_message: true,
                };
                if self.visibility == VisibilityState::Summon && !self.pinned {
                    let sub = self.collapse_or_hide();
                    fx.window_action = sub.window_action;
                    fx.bridge_mode = sub.bridge_mode;
                }
                fx
            }

            StateEvent::AssistantActivity {
                session_id,
                assistant_id,
                streaming,
            } => {
                let same_session = self.last_session_id.as_deref() == Some(session_id.as_str());
                self.last_session_id = Some(session_id);
                if !assistant_id.is_empty() || !same_session {
                    self.last_assistant_id = assistant_id;
                }
                self.last_streaming = streaming;
                // Scheme B: never ambush the user with the obsolete 56px Lip-only pill;
                // unread replies surface cleanly as the top unread dot + detached toast on summon.
                TransitionEffects::none()
            }

            StateEvent::IncomingBubbleNotice => {
                // Scheme B: never wake Hidden Bar into the obsolete composer-less 56px Lip pill.
                TransitionEffects::none()
            }

            StateEvent::UiUpdated {
                hovered: _,
                pinned,
                popup_open,
                modal,
                composer_focused,
            } => {
                self.pinned = pinned;
                self.popup_open = popup_open;
                self.modal = modal;
                self.composer_focused = composer_focused;
                if composer_focused || popup_open || pinned {
                    self.dock_locked_open = true;
                }
                TransitionEffects::none()
            }

            StateEvent::ComposerPointerDown => {
                self.dock_locked_open = true;
                if self.visibility == VisibilityState::Lip
                    || self.visibility == VisibilityState::DockedCollapsed
                {
                    self.visibility = VisibilityState::Summon;
                    return TransitionEffects {
                        window_action: Some(if self.dock_edge.is_docked() {
                            WindowAction::DockExpandAndFocus
                        } else {
                            WindowAction::ActivateAndFocus
                        }),
                        bridge_mode: Some(BridgeMode::Summon),
                        reset_pick_counter: false,
                        record_sent_message: false,
                    };
                }
                TransitionEffects::none()
            }

            StateEvent::PenStarted => {
                // Double-tap Ctrl & hold: wake Bar to Summon, focus composer cursor, and enter selection mode
                self.visibility = VisibilityState::Summon;
                self.pen_active = true;
                self.dock_locked_open = true;
                TransitionEffects {
                    window_action: Some(if self.dock_edge.is_docked() {
                        WindowAction::DockExpandAndFocus
                    } else {
                        WindowAction::ActivateAndFocus
                    }),
                    bridge_mode: Some(BridgeMode::Summon),
                    reset_pick_counter: false,
                    record_sent_message: false,
                }
            }

            StateEvent::PenEnded {
                picked_any,
                cancelled_by_other_key,
            } => {
                self.pen_active = false;
                if cancelled_by_other_key {
                    // Cancelled because another key was pressed (e.g. Ctrl+Alt for Chatterfly voice input):
                    // silently dismiss Selection Mode without stealing foreground focus or re-activating Bar!
                    return TransitionEffects::none();
                }
                if picked_any || self.visibility == VisibilityState::Summon {
                    self.visibility = VisibilityState::Summon;
                    return TransitionEffects {
                        window_action: Some(if self.dock_edge.is_docked() {
                            WindowAction::DockExpandAndFocus
                        } else {
                            WindowAction::ActivateAndFocus
                        }),
                        bridge_mode: Some(BridgeMode::Summon),
                        reset_pick_counter: false,
                        record_sent_message: false,
                    };
                }
                TransitionEffects::none()
            }

            StateEvent::PickInserted => {
                let was_summon = self.visibility == VisibilityState::Summon;
                self.visibility = VisibilityState::Summon;
                if !self.pen_active {
                    // Ctrl already released: activate Bar and place blinking caret in composer right after capsule insertion
                    TransitionEffects {
                        window_action: Some(if self.dock_edge.is_docked() {
                            WindowAction::DockExpandAndFocus
                        } else {
                            WindowAction::ActivateAndFocus
                        }),
                        bridge_mode: if was_summon {
                            None
                        } else {
                            Some(BridgeMode::Summon)
                        },
                        reset_pick_counter: false,
                        record_sent_message: false,
                    }
                } else if !was_summon {
                    TransitionEffects {
                        window_action: Some(WindowAction::ShowNoActivate),
                        bridge_mode: Some(BridgeMode::Summon),
                        reset_pick_counter: false,
                        record_sent_message: false,
                    }
                } else {
                    TransitionEffects::none()
                }
            }

            StateEvent::DevForceState(target) => match target {
                DevTargetState::Hidden => {
                    self.visibility = VisibilityState::Hidden;
                    self.pinned = false;
                    self.popup_open = false;
                    TransitionEffects {
                        window_action: Some(WindowAction::Hide),
                        bridge_mode: Some(BridgeMode::Lip),
                        reset_pick_counter: false,
                        record_sent_message: false,
                    }
                }
                DevTargetState::Lip => {
                    self.visibility = VisibilityState::Lip;
                    self.pinned = false;
                    self.popup_open = false;
                    TransitionEffects {
                        window_action: Some(WindowAction::ShowNoActivate),
                        bridge_mode: Some(BridgeMode::Lip),
                        reset_pick_counter: false,
                        record_sent_message: false,
                    }
                }
                DevTargetState::Summon => {
                    self.visibility = VisibilityState::Summon;
                    TransitionEffects {
                        window_action: Some(WindowAction::ActivateAndFocus),
                        bridge_mode: Some(BridgeMode::Summon),
                        reset_pick_counter: false,
                        record_sent_message: false,
                    }
                }
                DevTargetState::Expanded => {
                    self.visibility = VisibilityState::Summon;
                    TransitionEffects {
                        window_action: Some(WindowAction::ActivateAndFocus),
                        bridge_mode: Some(BridgeMode::Expanded),
                        reset_pick_counter: false,
                        record_sent_message: false,
                    }
                }
            },

            StateEvent::SetDockEdge {
                edge,
                start_collapsed,
            } => {
                self.dock_edge = edge;
                if !edge.is_docked() {
                    self.dock_locked_open = false;
                    if self.visibility == VisibilityState::DockedCollapsed {
                        self.visibility = VisibilityState::Summon;
                        return TransitionEffects {
                            window_action: Some(WindowAction::ActivateAndFocus),
                            bridge_mode: Some(BridgeMode::Summon),
                            reset_pick_counter: false,
                            record_sent_message: false,
                        };
                    }
                    return TransitionEffects::none();
                }
                if start_collapsed {
                    self.dock_locked_open = false;
                    if self.fullscreen_suppressed {
                        self.visibility = VisibilityState::Hidden;
                        return TransitionEffects {
                            window_action: Some(WindowAction::Hide),
                            bridge_mode: Some(BridgeMode::Summon),
                            reset_pick_counter: false,
                            record_sent_message: false,
                        };
                    }
                    self.visibility = VisibilityState::DockedCollapsed;
                    TransitionEffects {
                        window_action: Some(WindowAction::DockCollapse),
                        bridge_mode: Some(BridgeMode::Summon),
                        reset_pick_counter: false,
                        record_sent_message: false,
                    }
                } else {
                    self.visibility = VisibilityState::Summon;
                    self.dock_locked_open = true;
                    TransitionEffects {
                        window_action: Some(WindowAction::DockExpandAndFocus),
                        bridge_mode: Some(BridgeMode::Summon),
                        reset_pick_counter: false,
                        record_sent_message: false,
                    }
                }
            }

            StateEvent::SetDockEdgeFromDrag { edge } => {
                self.dock_edge = edge;
                self.dock_locked_open = false;
                self.composer_focused = false;
                if !edge.is_docked() {
                    return TransitionEffects::none();
                }
                self.visibility = VisibilityState::Summon;
                TransitionEffects {
                    window_action: Some(WindowAction::DockExpandNoActivate),
                    bridge_mode: Some(BridgeMode::Summon),
                    reset_pick_counter: false,
                    record_sent_message: false,
                }
            }

            StateEvent::DockEdgePeekEnter => {
                if !self.dock_edge.is_docked()
                    || self.visibility != VisibilityState::DockedCollapsed
                    || self.fullscreen_suppressed
                {
                    return TransitionEffects::none();
                }
                self.visibility = VisibilityState::Summon;
                self.dock_locked_open = false;
                TransitionEffects {
                    window_action: Some(WindowAction::DockExpandNoActivate),
                    bridge_mode: Some(BridgeMode::Summon),
                    reset_pick_counter: false,
                    record_sent_message: false,
                }
            }

            StateEvent::DockEdgeClickExpand => {
                if !self.dock_edge.is_docked() {
                    return TransitionEffects::none();
                }
                self.visibility = VisibilityState::Summon;
                self.dock_locked_open = true;
                TransitionEffects {
                    window_action: Some(WindowAction::DockExpandAndFocus),
                    bridge_mode: Some(BridgeMode::Summon),
                    reset_pick_counter: false,
                    record_sent_message: false,
                }
            }

            StateEvent::DockAutoCollapseIfUnlocked => {
                if !self.dock_edge.is_docked()
                    || self.visibility != VisibilityState::Summon
                    || self.dock_locked_open
                    || self.pinned
                    || self.popup_open
                    || self.modal
                    || self.composer_focused
                    || self.pen_active
                {
                    return TransitionEffects::none();
                }
                self.collapse_or_hide()
            }

            StateEvent::FullscreenChanged { active } => {
                if self.fullscreen_suppressed == active {
                    return TransitionEffects::none();
                }
                self.fullscreen_suppressed = active;
                if !self.dock_edge.is_docked() {
                    return TransitionEffects::none();
                }
                if active && self.visibility == VisibilityState::DockedCollapsed {
                    self.visibility = VisibilityState::Hidden;
                    return TransitionEffects {
                        window_action: Some(WindowAction::Hide),
                        bridge_mode: None,
                        reset_pick_counter: false,
                        record_sent_message: false,
                    };
                }
                if !active && self.visibility == VisibilityState::Hidden {
                    self.visibility = VisibilityState::DockedCollapsed;
                    return TransitionEffects {
                        window_action: Some(WindowAction::DockCollapse),
                        bridge_mode: None,
                        reset_pick_counter: false,
                        record_sent_message: false,
                    };
                }
                TransitionEffects::none()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_ctrl_opens_and_single_ctrl_or_outside_click_closes_immediately() {
        let mut sm = BarStateMachine::new();
        assert_eq!(sm.visibility, VisibilityState::Hidden);

        // Single Ctrl when Hidden does nothing
        assert!(sm
            .apply(StateEvent::SingleCtrlClose)
            .window_action
            .is_none());

        // Double Ctrl opens Summon + focuses composer
        let fx = sm.apply(StateEvent::DoubleCtrlOpen);
        assert_eq!(sm.visibility, VisibilityState::Summon);
        assert_eq!(fx.window_action, Some(WindowAction::ActivateAndFocus));

        // Single Ctrl immediately closes Summon -> Hidden
        let fx = sm.apply(StateEvent::SingleCtrlClose);
        assert_eq!(sm.visibility, VisibilityState::Hidden);
        assert_eq!(fx.window_action, Some(WindowAction::Hide));

        // Open again, then outside click immediately hides all
        sm.apply(StateEvent::DoubleCtrlOpen);
        assert_eq!(sm.visibility, VisibilityState::Summon);
        let fx = sm.apply(StateEvent::OutsideClickHide);
        assert_eq!(sm.visibility, VisibilityState::Hidden);
        assert_eq!(fx.window_action, Some(WindowAction::Hide));
    }

    #[test]
    fn pen_started_wakes_hidden_bar_and_single_ctrl_closes_after_pick() {
        let mut sm = BarStateMachine::new();
        assert_eq!(sm.visibility, VisibilityState::Hidden);

        // Double-tap Ctrl & hold directly from Hidden wakes Summon + focuses composer + activates pen
        let fx_start = sm.apply(StateEvent::PenStarted);
        assert_eq!(sm.visibility, VisibilityState::Summon);
        assert!(sm.pen_active);
        assert_eq!(fx_start.window_action, Some(WindowAction::ActivateAndFocus));

        // While pen is active, outside click and single Ctrl close are blocked
        assert!(sm
            .apply(StateEvent::OutsideClickHide)
            .window_action
            .is_none());
        assert!(sm
            .apply(StateEvent::SingleCtrlClose)
            .window_action
            .is_none());

        // Releasing Ctrl keeps Summon focused even if nothing was clicked
        let fx_end = sm.apply(StateEvent::PenEnded {
            picked_any: true,
            cancelled_by_other_key: false,
        });
        assert!(!sm.pen_active);
        assert_eq!(fx_end.window_action, Some(WindowAction::ActivateAndFocus));

        // Late PickInserted arriving after Ctrl release still focuses composer
        let fx_ins = sm.apply(StateEvent::PickInserted);
        assert_eq!(fx_ins.window_action, Some(WindowAction::ActivateAndFocus));

        // Now single Ctrl tap immediately closes Summon -> Hidden on first try!
        let fx_close = sm.apply(StateEvent::SingleCtrlClose);
        assert_eq!(sm.visibility, VisibilityState::Hidden);
        assert_eq!(fx_close.window_action, Some(WindowAction::Hide));
    }

    #[test]
    fn pen_ended_cancelled_by_other_key_does_not_steal_focus() {
        let mut sm = BarStateMachine::new();
        sm.apply(StateEvent::DoubleCtrlOpen);
        assert_eq!(sm.visibility, VisibilityState::Summon);

        // User held Ctrl slightly >= hold_ms before pressing Alt (Chatterfly Ctrl+Alt)
        sm.apply(StateEvent::PenStarted);
        assert!(sm.pen_active);

        // Alt cancels Pen -> MUST NOT emit ActivateAndFocus or hide Bar!
        let fx = sm.apply(StateEvent::PenEnded {
            picked_any: false,
            cancelled_by_other_key: true,
        });
        assert!(!sm.pen_active);
        assert_eq!(sm.visibility, VisibilityState::Summon);
        assert!(fx.window_action.is_none());
        assert!(fx.bridge_mode.is_none());
    }

    #[test]
    fn submitted_hides_summon_directly_without_lingering_lip_and_resets_counter() {
        let mut sm = BarStateMachine::new();
        sm.apply(StateEvent::DoubleCtrlOpen);

        let fx = sm.apply(StateEvent::Submitted);
        assert_eq!(sm.visibility, VisibilityState::Hidden);
        assert_eq!(fx.window_action, Some(WindowAction::Hide));
        assert!(fx.reset_pick_counter);
        assert!(fx.record_sent_message);

        // Esc (StepDown) from Summon also hides cleanly without leaving a Lip bubble
        sm.apply(StateEvent::DoubleCtrlOpen);
        let fx_esc = sm.apply(StateEvent::StepDown);
        assert_eq!(sm.visibility, VisibilityState::Hidden);
        assert_eq!(fx_esc.window_action, Some(WindowAction::Hide));
    }

    #[test]
    fn assistant_activity_never_pops_obsolete_lip_pill_while_hidden() {
        let mut sm = BarStateMachine::new();
        // Stage 1: session selected before messages load (assistant_id = "")
        let fx0 = sm.apply(StateEvent::AssistantActivity {
            session_id: "s1".into(),
            assistant_id: String::new(),
            streaming: false,
        });
        assert_eq!(sm.visibility, VisibilityState::Hidden);
        assert!(fx0.window_action.is_none());

        // Stage 2: existing history finishes loading -> MUST stay Hidden
        let fx = sm.apply(StateEvent::AssistantActivity {
            session_id: "s1".into(),
            assistant_id: "m1".into(),
            streaming: false,
        });
        assert_eq!(sm.visibility, VisibilityState::Hidden);
        assert!(fx.window_action.is_none());

        // Real new reply ("m1" -> "m2") -> Scheme B never pops up a composer-less 56px Lip pill
        let fx = sm.apply(StateEvent::AssistantActivity {
            session_id: "s1".into(),
            assistant_id: "m2".into(),
            streaming: true,
        });
        assert_eq!(sm.visibility, VisibilityState::Hidden);
        assert!(fx.window_action.is_none());
        assert!(fx.bridge_mode.is_none());
    }

    #[test]
    fn pinned_bar_stays_open_on_outside_click_single_ctrl_esc_and_submit() {
        let mut sm = BarStateMachine::new();
        sm.apply(StateEvent::DoubleCtrlOpen);
        sm.apply(StateEvent::UiUpdated {
            hovered: true,
            pinned: true,
            popup_open: false,
            modal: false,
            composer_focused: true,
        });
        assert!(sm.pinned);

        assert!(sm.apply(StateEvent::OutsideClickHide).window_action.is_none());
        assert_eq!(sm.visibility, VisibilityState::Summon);

        assert!(sm.apply(StateEvent::SingleCtrlClose).window_action.is_none());
        assert_eq!(sm.visibility, VisibilityState::Summon);

        assert!(sm.apply(StateEvent::StepDown).window_action.is_none());
        assert_eq!(sm.visibility, VisibilityState::Summon);

        let fx_sub = sm.apply(StateEvent::Submitted);
        assert!(fx_sub.window_action.is_none());
        assert_eq!(sm.visibility, VisibilityState::Summon);

        // Unpin -> outside click closes normally again without switching to Lip mode
        sm.apply(StateEvent::UiUpdated {
            hovered: false,
            pinned: false,
            popup_open: false,
            modal: false,
            composer_focused: false,
        });
        let fx_hide = sm.apply(StateEvent::OutsideClickHide);
        assert_eq!(sm.visibility, VisibilityState::Hidden);
        assert_eq!(fx_hide.window_action, Some(WindowAction::Hide));
        assert!(fx_hide.bridge_mode.is_none());
    }

    #[test]
    fn incoming_bubble_notice_never_pops_obsolete_lip_pill() {
        let mut sm = BarStateMachine::new();
        assert_eq!(sm.visibility, VisibilityState::Hidden);

        let fx = sm.apply(StateEvent::IncomingBubbleNotice);
        assert_eq!(sm.visibility, VisibilityState::Hidden);
        assert!(fx.window_action.is_none());
        assert!(fx.bridge_mode.is_none());

        // Double-Ctrl still summons directly into Summon
        let fx2 = sm.apply(StateEvent::DoubleCtrlOpen);
        assert_eq!(sm.visibility, VisibilityState::Summon);
        assert_eq!(fx2.window_action, Some(WindowAction::ActivateAndFocus));
        assert_eq!(fx2.bridge_mode, Some(BridgeMode::Summon));

        // While Summon, IncomingBubbleNotice must NOT alter visibility
        let fx3 = sm.apply(StateEvent::IncomingBubbleNotice);
        assert_eq!(sm.visibility, VisibilityState::Summon);
        assert!(fx3.window_action.is_none());
    }

    #[test]
    fn docked_bar_collapses_to_edge_on_outside_click_esc_ctrl_and_submit() {
        let mut sm = BarStateMachine::new();
        let fx = sm.apply(StateEvent::SetDockEdge {
            edge: DockEdge::Right,
            start_collapsed: false,
        });
        assert_eq!(sm.dock_edge, DockEdge::Right);
        assert_eq!(sm.visibility, VisibilityState::Summon);
        assert!(sm.dock_locked_open);
        assert_eq!(fx.window_action, Some(WindowAction::DockExpandAndFocus));

        // Outside click collapses into DockedCollapsed (never Hidden!)
        let fx_out = sm.apply(StateEvent::OutsideClickHide);
        assert_eq!(sm.visibility, VisibilityState::DockedCollapsed);
        assert_eq!(fx_out.window_action, Some(WindowAction::DockCollapse));

        // Double-Ctrl expands & locks open
        let fx_open = sm.apply(StateEvent::DoubleCtrlOpen);
        assert_eq!(sm.visibility, VisibilityState::Summon);
        assert!(sm.dock_locked_open);
        assert_eq!(fx_open.window_action, Some(WindowAction::DockExpandAndFocus));

        // Esc (StepDown) collapses to DockedCollapsed
        let fx_esc = sm.apply(StateEvent::StepDown);
        assert_eq!(sm.visibility, VisibilityState::DockedCollapsed);
        assert_eq!(fx_esc.window_action, Some(WindowAction::DockCollapse));

        // Single-Ctrl while Summon collapses to DockedCollapsed
        sm.apply(StateEvent::DockEdgeClickExpand);
        let fx_ctrl = sm.apply(StateEvent::SingleCtrlClose);
        assert_eq!(sm.visibility, VisibilityState::DockedCollapsed);
        assert_eq!(fx_ctrl.window_action, Some(WindowAction::DockCollapse));

        // Submitted while Summon collapses to DockedCollapsed
        sm.apply(StateEvent::DockEdgeClickExpand);
        let fx_sub = sm.apply(StateEvent::Submitted);
        assert_eq!(sm.visibility, VisibilityState::DockedCollapsed);
        assert_eq!(fx_sub.window_action, Some(WindowAction::DockCollapse));
        assert!(fx_sub.reset_pick_counter);
        assert!(fx_sub.record_sent_message);
    }

    #[test]
    fn docked_hover_peek_auto_collapses_until_locked_by_click_or_focus() {
        let mut sm = BarStateMachine::new();
        sm.apply(StateEvent::SetDockEdge {
            edge: DockEdge::Bottom,
            start_collapsed: true,
        });
        assert_eq!(sm.visibility, VisibilityState::DockedCollapsed);

        // Hover edge -> slides out unlocked without stealing focus
        let fx_peek = sm.apply(StateEvent::DockEdgePeekEnter);
        assert_eq!(sm.visibility, VisibilityState::Summon);
        assert!(!sm.dock_locked_open);
        assert_eq!(fx_peek.window_action, Some(WindowAction::DockExpandNoActivate));

        // Mouse leaves while unlocked -> auto-collapses back into 10px edge
        let fx_auto = sm.apply(StateEvent::DockAutoCollapseIfUnlocked);
        assert_eq!(sm.visibility, VisibilityState::DockedCollapsed);
        assert_eq!(fx_auto.window_action, Some(WindowAction::DockCollapse));

        // Hover edge again, then user clicks composer -> locks open
        sm.apply(StateEvent::DockEdgePeekEnter);
        sm.apply(StateEvent::ComposerPointerDown);
        assert!(sm.dock_locked_open);

        // Now mouse leaving must NOT auto-collapse
        let fx_stay = sm.apply(StateEvent::DockAutoCollapseIfUnlocked);
        assert_eq!(sm.visibility, VisibilityState::Summon);
        assert!(fx_stay.window_action.is_none());

        // Fullscreen app covers dock monitor while collapsed -> hides 10px strip, then restores on exit
        sm.apply(StateEvent::OutsideClickHide);
        assert_eq!(sm.visibility, VisibilityState::DockedCollapsed);
        let fx_fs_on = sm.apply(StateEvent::FullscreenChanged { active: true });
        assert_eq!(sm.visibility, VisibilityState::Hidden);
        assert_eq!(fx_fs_on.window_action, Some(WindowAction::Hide));

        let fx_fs_off = sm.apply(StateEvent::FullscreenChanged { active: false });
        assert_eq!(sm.visibility, VisibilityState::DockedCollapsed);
        assert_eq!(fx_fs_off.window_action, Some(WindowAction::DockCollapse));
    }
}
