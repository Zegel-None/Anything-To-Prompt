use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisibilityState {
    Hidden,
    Lip,
    Summon,
}

impl VisibilityState {
    pub fn tier_u8(self) -> u8 {
        match self {
            VisibilityState::Hidden => 0,
            VisibilityState::Lip => 1,
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
            last_session_id: None,
            last_assistant_id: String::new(),
            last_streaming: false,
        }
    }

    pub fn is_visible(&self) -> bool {
        !matches!(self.visibility, VisibilityState::Hidden)
    }

    pub fn apply(&mut self, event: StateEvent) -> TransitionEffects {
        match event {
            StateEvent::SingleCtrlClose => {
                if self.pen_active || self.modal || self.visibility != VisibilityState::Summon {
                    return TransitionEffects::none();
                }
                self.visibility = VisibilityState::Hidden;
                self.pinned = false;
                self.popup_open = false;
                self.composer_focused = false;
                TransitionEffects {
                    window_action: Some(WindowAction::Hide),
                    bridge_mode: Some(BridgeMode::Lip),
                    reset_pick_counter: false,
                    record_sent_message: false,
                }
            }

            StateEvent::DoubleCtrlOpen => {
                self.visibility = VisibilityState::Summon;
                TransitionEffects {
                    window_action: Some(WindowAction::ActivateAndFocus),
                    bridge_mode: Some(BridgeMode::Summon),
                    reset_pick_counter: false,
                    record_sent_message: false,
                }
            }

            StateEvent::SummonFromLip => {
                self.visibility = VisibilityState::Summon;
                TransitionEffects {
                    window_action: Some(WindowAction::ActivateAndFocus),
                    bridge_mode: Some(BridgeMode::Summon),
                    reset_pick_counter: false,
                    record_sent_message: false,
                }
            }

            StateEvent::OutsideClickHide => {
                if self.pen_active || self.modal || self.visibility == VisibilityState::Hidden {
                    return TransitionEffects::none();
                }
                self.visibility = VisibilityState::Hidden;
                self.pinned = false;
                self.popup_open = false;
                self.composer_focused = false;
                TransitionEffects {
                    window_action: Some(WindowAction::Hide),
                    bridge_mode: Some(BridgeMode::Lip),
                    reset_pick_counter: false,
                    record_sent_message: false,
                }
            }

            StateEvent::TrayToggle => match self.visibility {
                VisibilityState::Hidden => {
                    self.visibility = VisibilityState::Summon;
                    TransitionEffects {
                        window_action: Some(WindowAction::ActivateAndFocus),
                        bridge_mode: Some(BridgeMode::Summon),
                        reset_pick_counter: false,
                        record_sent_message: false,
                    }
                }
                _ => {
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
            },

            StateEvent::StepDown => {
                if self.pen_active || self.modal {
                    return TransitionEffects::none();
                }
                match self.visibility {
                    VisibilityState::Hidden => TransitionEffects::none(),
                    VisibilityState::Summon | VisibilityState::Lip => {
                        // Never leave a lingering Lip bubble on dismiss — Lip is exclusively
                        // reserved for incoming new-message notifications.
                        self.visibility = VisibilityState::Hidden;
                        self.pinned = false;
                        self.popup_open = false;
                        self.composer_focused = false;
                        TransitionEffects {
                            window_action: Some(WindowAction::Hide),
                            bridge_mode: Some(BridgeMode::Lip),
                            reset_pick_counter: false,
                            record_sent_message: false,
                        }
                    }
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
                    // Close cleanly after sending instead of leaving a lingering empty Lip bubble.
                    // The Lip bubble will pop up automatically when a real new AI reply arrives.
                    self.visibility = VisibilityState::Hidden;
                    self.popup_open = false;
                    self.composer_focused = false;
                    fx.window_action = Some(WindowAction::Hide);
                    fx.bridge_mode = Some(BridgeMode::Lip);
                }
                fx
            }

            StateEvent::AssistantActivity {
                session_id,
                assistant_id,
                streaming,
            } => {
                let same_session = self.last_session_id.as_deref() == Some(session_id.as_str());
                let had_hydrated_baseline = !self.last_assistant_id.is_empty();
                let is_new_activity = same_session
                    && ((had_hydrated_baseline
                        && !assistant_id.is_empty()
                        && assistant_id != self.last_assistant_id)
                        || (streaming && !self.last_streaming));

                self.last_session_id = Some(session_id);
                if !assistant_id.is_empty() || !same_session {
                    self.last_assistant_id = assistant_id;
                }
                self.last_streaming = streaming;

                if is_new_activity && self.visibility == VisibilityState::Hidden {
                    self.visibility = VisibilityState::Lip;
                    return TransitionEffects {
                        window_action: Some(WindowAction::ShowNoActivate),
                        bridge_mode: Some(BridgeMode::Lip),
                        reset_pick_counter: false,
                        record_sent_message: false,
                    };
                }
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
                TransitionEffects::none()
            }

            StateEvent::ComposerPointerDown => {
                if self.visibility == VisibilityState::Lip {
                    self.visibility = VisibilityState::Summon;
                    return TransitionEffects {
                        window_action: Some(WindowAction::ActivateAndFocus),
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
                TransitionEffects {
                    window_action: Some(WindowAction::ActivateAndFocus),
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
                        window_action: Some(WindowAction::ActivateAndFocus),
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
                    // Alt already released: activate Bar and place blinking caret in composer right after capsule insertion
                    TransitionEffects {
                        window_action: Some(WindowAction::ActivateAndFocus),
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
    fn assistant_activity_ignores_two_stage_async_hydration_and_wakes_on_new_reply() {
        let mut sm = BarStateMachine::new();
        // Stage 1: session selected before messages load (assistant_id = "")
        let fx0 = sm.apply(StateEvent::AssistantActivity {
            session_id: "s1".into(),
            assistant_id: String::new(),
            streaming: false,
        });
        assert_eq!(sm.visibility, VisibilityState::Hidden);
        assert!(fx0.window_action.is_none());

        // Stage 2: existing history finishes loading ("" -> "m1", not streaming) -> MUST stay Hidden!
        let fx = sm.apply(StateEvent::AssistantActivity {
            session_id: "s1".into(),
            assistant_id: "m1".into(),
            streaming: false,
        });
        assert_eq!(sm.visibility, VisibilityState::Hidden);
        assert!(fx.window_action.is_none());

        // Real new reply ("m1" -> "m2") -> wakes to Lip!
        let fx = sm.apply(StateEvent::AssistantActivity {
            session_id: "s1".into(),
            assistant_id: "m2".into(),
            streaming: true,
        });
        assert_eq!(sm.visibility, VisibilityState::Lip);
        assert_eq!(fx.window_action, Some(WindowAction::ShowNoActivate));
    }
}
