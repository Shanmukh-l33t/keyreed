use crate::types::{ActivationMode, EngineState};

/// Events emitted by the global Push-to-Talk state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PttAction {
    None,
    StartRecording,
    StopAndProcess,
    CancelRecording { reason: &'static str },
}

/// Deterministic, lock-free-friendly Push-to-Talk controller.
/// Handles Hold-to-Talk, Toggle-to-Talk, Floating Mic click, independent PTT enable/disable,
/// OS auto-repeat deduplication, emergency Escape cancellation, and session/sleep resets.
#[derive(Debug, Clone)]
pub struct PttController {
    app_enabled: bool,
    ptt_enabled: bool,
    activation_mode: ActivationMode,
    shortcut_physically_down: bool,
    recording_started_ms: Option<u64>,
    /// Epoch of the hold that started the current recording. A late key-up from an older hold must not stop a newer session.
    recording_epoch: Option<u64>,
    /// Epoch of the physical hold that is still awaiting its release.
    open_hold: Option<u64>,
    hold_epoch: u64,
    min_hold_ms: u64,
    max_record_ms: u64,
    state: EngineState,
}

impl Default for PttController {
    fn default() -> Self {
        Self::new(ActivationMode::HoldToTalk, 110, 1_800_000)
    }
}

impl PttController {
    pub fn new(activation_mode: ActivationMode, min_hold_ms: u64, max_record_ms: u64) -> Self {
        Self {
            app_enabled: true,
            ptt_enabled: true,
            activation_mode,
            shortcut_physically_down: false,
            recording_started_ms: None,
            recording_epoch: None,
            open_hold: None,
            hold_epoch: 0,
            min_hold_ms,
            max_record_ms,
            state: EngineState::Idle,
        }
    }

    pub fn state(&self) -> &EngineState {
        &self.state
    }

    pub fn is_app_enabled(&self) -> bool {
        self.app_enabled
    }

    pub fn is_ptt_enabled(&self) -> bool {
        self.ptt_enabled
    }

    pub fn activation_mode(&self) -> ActivationMode {
        self.activation_mode
    }

    pub fn set_activation_mode(&mut self, mode: ActivationMode) {
        self.activation_mode = mode;
    }

    /// Enables or disables the entire application without exiting the tray process.
    pub fn set_app_enabled(&mut self, enabled: bool) -> PttAction {
        self.app_enabled = enabled;
        self.shortcut_physically_down = false;
        self.open_hold = None;
        if !enabled {
            let was_active = self.is_recording();
            self.recording_started_ms = None;
            self.recording_epoch = None;
            self.state = EngineState::Disabled;
            if was_active {
                return PttAction::CancelRecording {
                    reason: "application_disabled",
                };
            }
        } else if matches!(self.state, EngineState::Disabled) {
            self.state = EngineState::Idle;
        }
        PttAction::None
    }

    /// Independently switches global Push-to-Talk hotkey handling without closing the app
    /// or disabling floating-microphone click activation (Master Prompt Section 3).
    pub fn set_ptt_enabled(&mut self, enabled: bool) -> PttAction {
        self.ptt_enabled = enabled;
        self.shortcut_physically_down = false;
        self.open_hold = None;
        if !enabled && self.is_recording() {
            self.recording_started_ms = None;
            self.recording_epoch = None;
            self.state = EngineState::Idle;
            return PttAction::CancelRecording {
                reason: "ptt_disabled",
            };
        }
        PttAction::None
    }

    pub fn is_recording(&self) -> bool {
        matches!(self.state, EngineState::Listening)
    }

    pub fn is_shortcut_physically_down(&self) -> bool {
        self.shortcut_physically_down
    }

    pub fn recording_elapsed_ms(&self, now_ms: u64) -> u64 {
        self.recording_started_ms
            .map(|s| now_ms.saturating_sub(s))
            .unwrap_or(0)
    }

    pub fn is_busy_processing(&self) -> bool {
        matches!(self.state, EngineState::Processing | EngineState::Correcting)
    }

    /// Called when the global shortcut transitions down (may also be called repeatedly by OS auto-repeat).
    pub fn on_shortcut_down(&mut self, now_ms: u64) -> PttAction {
        if !self.app_enabled || !self.ptt_enabled {
            return PttAction::None;
        }
        // Ignore OS key-repeat while the key is already held.
        if self.shortcut_physically_down {
            return PttAction::None;
        }
        self.shortcut_physically_down = true;
        self.hold_epoch = self.hold_epoch.saturating_add(1);
        let epoch = self.hold_epoch;
        self.open_hold = Some(epoch);

        if self.is_busy_processing() {
            return PttAction::None;
        }

        match self.activation_mode {
            ActivationMode::HoldToTalk => {
                if !self.is_recording() {
                    self.recording_started_ms = Some(now_ms);
                    self.recording_epoch = Some(epoch);
                    self.state = EngineState::Listening;
                    PttAction::StartRecording
                } else {
                    PttAction::None
                }
            }
            ActivationMode::ToggleToTalk => {
                if self.is_recording() {
                    self.recording_started_ms = None;
                    self.recording_epoch = None;
                    self.state = EngineState::Processing;
                    PttAction::StopAndProcess
                } else {
                    self.recording_started_ms = Some(now_ms);
                    self.recording_epoch = Some(epoch);
                    self.state = EngineState::Listening;
                    PttAction::StartRecording
                }
            }
        }
    }

    /// Called when the global shortcut is released.
    pub fn on_shortcut_up(&mut self, now_ms: u64) -> PttAction {
        if !self.shortcut_physically_down {
            return PttAction::None;
        }
        let epoch = self.open_hold.take();
        self.shortcut_physically_down = false;

        if self.activation_mode == ActivationMode::HoldToTalk && self.is_recording() {
            if self.recording_epoch != epoch {
                return PttAction::None;
            }
            let started = self.recording_started_ms.take().unwrap_or(now_ms);
            self.recording_epoch = None;
            let held_ms = now_ms.saturating_sub(started);
            if held_ms < self.min_hold_ms {
                self.state = EngineState::Idle;
                return PttAction::CancelRecording {
                    reason: "tap_too_short",
                };
            }
            self.state = EngineState::Processing;
            return PttAction::StopAndProcess;
        }
        PttAction::None
    }

    /// Floating microphone button click: works even when keyboard PTT is paused,
    /// as long as the application itself is enabled.
    pub fn on_floating_mic_click(&mut self, now_ms: u64) -> PttAction {
        if !self.app_enabled || self.is_busy_processing() {
            return PttAction::None;
        }
        if self.is_recording() {
            self.recording_started_ms = None;
            self.recording_epoch = Some(self.hold_epoch);
            self.state = EngineState::Processing;
            PttAction::StopAndProcess
        } else {
            self.hold_epoch = self.hold_epoch.saturating_add(1);
            self.recording_started_ms = Some(now_ms);
            self.recording_epoch = Some(self.hold_epoch);
            self.state = EngineState::Listening;
            PttAction::StartRecording
        }
    }

    /// Emergency cancellation shortcut (e.g. Esc) or session lock / sleep transition.
    pub fn on_emergency_cancel(&mut self, reason: &'static str) -> PttAction {
        self.shortcut_physically_down = false;
        self.open_hold = None;
        self.recording_started_ms = None;
        self.recording_epoch = None;
        if self.is_recording() || self.is_busy_processing() {
            self.state = EngineState::Idle;
            return PttAction::CancelRecording { reason };
        }
        PttAction::None
    }

    /// Periodic watchdog check so a stuck toggle or forgotten mic never records indefinitely.
    /// At the cap the recording is finished and processed, never thrown away. The hold latch is
    /// kept so the eventual key release (or its repeats) cannot start a new session by accident.
    pub fn on_tick(&mut self, now_ms: u64) -> PttAction {
        if self.is_recording() {
            if let Some(started) = self.recording_started_ms {
                if now_ms.saturating_sub(started) >= self.max_record_ms {
                    self.recording_started_ms = None;
                    self.recording_epoch = None;
                    self.state = EngineState::Processing;
                    return PttAction::StopAndProcess;
                }
            }
        }
        PttAction::None
    }

    pub fn transition_to_correcting(&mut self) {
        if matches!(self.state, EngineState::Processing) {
            self.state = EngineState::Correcting;
        }
    }

    pub fn transition_to_ready(&mut self) {
        if self.app_enabled {
            self.state = EngineState::ReadyToPaste;
        }
    }

    pub fn transition_to_idle(&mut self) {
        if self.app_enabled {
            self.state = EngineState::Idle;
        }
    }

    pub fn transition_to_error(&mut self, msg: impl Into<String>) {
        if self.app_enabled {
            self.state = EngineState::Error(msg.into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hold_to_talk_ignores_autorepeat_and_processes_on_release() {
        let mut ptt = PttController::default();
        assert_eq!(ptt.on_shortcut_down(1_000), PttAction::StartRecording);
        assert_eq!(ptt.state(), &EngineState::Listening);
        // OS key repeat must be ignored
        assert_eq!(ptt.on_shortcut_down(1_050), PttAction::None);
        assert_eq!(ptt.on_shortcut_down(1_100), PttAction::None);
        // Release after 1.5s
        assert_eq!(ptt.on_shortcut_up(2_500), PttAction::StopAndProcess);
        assert_eq!(ptt.state(), &EngineState::Processing);
    }

    #[test]
    fn hold_to_talk_cancels_accidental_micro_tap() {
        let mut ptt = PttController::new(ActivationMode::HoldToTalk, 120, 60_000);
        assert_eq!(ptt.on_shortcut_down(1_000), PttAction::StartRecording);
        assert_eq!(
            ptt.on_shortcut_up(1_040),
            PttAction::CancelRecording {
                reason: "tap_too_short"
            }
        );
        assert_eq!(ptt.state(), &EngineState::Idle);
    }

    #[test]
    fn toggle_to_talk_starts_and_stops_on_distinct_presses() {
        let mut ptt = PttController::new(ActivationMode::ToggleToTalk, 100, 60_000);
        assert_eq!(ptt.on_shortcut_down(1_000), PttAction::StartRecording);
        assert_eq!(ptt.on_shortcut_up(1_080), PttAction::None);
        assert_eq!(ptt.state(), &EngineState::Listening);
        assert_eq!(ptt.on_shortcut_down(4_000), PttAction::StopAndProcess);
        assert_eq!(ptt.on_shortcut_up(4_060), PttAction::None);
    }

    #[test]
    fn independent_ptt_disable_keeps_floating_mic_usable() {
        let mut ptt = PttController::default();
        ptt.set_ptt_enabled(false);
        assert_eq!(ptt.on_shortcut_down(1_000), PttAction::None);
        // Floating mic still works independently!
        assert_eq!(ptt.on_floating_mic_click(1_500), PttAction::StartRecording);
        assert_eq!(ptt.on_floating_mic_click(3_000), PttAction::StopAndProcess);
    }

    #[test]
    fn emergency_cancel_aborts_active_recording() {
        let mut ptt = PttController::default();
        assert_eq!(ptt.on_shortcut_down(1_000), PttAction::StartRecording);
        assert_eq!(
            ptt.on_emergency_cancel("escape_pressed"),
            PttAction::CancelRecording {
                reason: "escape_pressed"
            }
        );
        assert_eq!(ptt.state(), &EngineState::Idle);
        // Subsequent release of shortcut must be a safe no-op
        assert_eq!(ptt.on_shortcut_up(1_500), PttAction::None);
    }

    #[test]
    fn max_duration_stops_and_processes_instead_of_discarding() {
        let mut ptt = PttController::new(ActivationMode::HoldToTalk, 110, 1_000);
        assert_eq!(ptt.on_shortcut_down(0), PttAction::StartRecording);
        assert_eq!(ptt.on_tick(1_000), PttAction::StopAndProcess);
        assert_eq!(ptt.state(), &EngineState::Processing);
        // Cap fires once; the late key release is a safe no-op.
        assert_eq!(ptt.on_tick(1_025), PttAction::None);
        assert_eq!(ptt.on_shortcut_up(1_100), PttAction::None);
        ptt.transition_to_idle();
        assert_eq!(ptt.on_shortcut_down(1_200), PttAction::StartRecording);
    }

    #[test]
    fn toggle_cap_also_processes() {
        let mut ptt = PttController::new(ActivationMode::ToggleToTalk, 110, 500);
        assert_eq!(ptt.on_shortcut_down(0), PttAction::StartRecording);
        assert_eq!(ptt.on_shortcut_up(50), PttAction::None);
        assert_eq!(ptt.on_tick(500), PttAction::StopAndProcess);
        assert_eq!(ptt.on_tick(600), PttAction::None);
    }

    #[test]
    fn stale_keyup_does_not_stop_a_newer_floating_mic_session() {
        let mut ptt = PttController::new(ActivationMode::HoldToTalk, 110, 1_000);
        assert_eq!(ptt.on_shortcut_down(0), PttAction::StartRecording);
        assert_eq!(ptt.on_tick(1_000), PttAction::StopAndProcess);
        ptt.transition_to_idle();
        assert_eq!(ptt.on_floating_mic_click(1_100), PttAction::StartRecording);
        assert_eq!(ptt.on_shortcut_up(1_200), PttAction::None);
        assert_eq!(ptt.state(), &EngineState::Listening);
        assert_eq!(ptt.on_floating_mic_click(2_000), PttAction::StopAndProcess);
    }

    #[test]
    fn duplicate_keydown_does_not_start_a_second_session() {
        let mut ptt = PttController::default();
        assert_eq!(ptt.on_shortcut_down(0), PttAction::StartRecording);
        assert_eq!(ptt.on_shortcut_down(10), PttAction::None);
        assert_eq!(ptt.on_shortcut_down(20), PttAction::None);
        assert_eq!(ptt.state(), &EngineState::Listening);
    }
}
