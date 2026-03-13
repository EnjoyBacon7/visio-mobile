//! Bandwidth degradation controller.
//!
//! Tracks connection quality over time and decides when to degrade
//! or restore video quality using a state machine with hysteresis.

use std::time::{Duration, Instant};

use crate::events::ConnectionQuality;

/// Bandwidth degradation mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BandwidthMode {
    /// All video at full quality.
    Full,
    /// Video downgraded to low quality.
    ReducedVideo,
    /// All video disabled, audio only.
    AudioOnly,
}

/// Duration of continuous `Poor` quality before escalating to `AudioOnly`.
const POOR_ESCALATION: Duration = Duration::from_secs(3);

/// Duration of improved quality required before upgrading mode (hysteresis).
const UPGRADE_HYSTERESIS: Duration = Duration::from_secs(5);

/// State machine that tracks connection quality and decides bandwidth mode.
pub struct BandwidthController {
    mode: BandwidthMode,
    /// When we first entered `Poor` quality (for escalation timing).
    poor_since: Option<Instant>,
    /// When we first saw an improvement that could trigger an upgrade.
    upgrade_candidate: Option<(BandwidthMode, Instant)>,
}

impl BandwidthController {
    pub fn new() -> Self {
        Self {
            mode: BandwidthMode::Full,
            poor_since: None,
            upgrade_candidate: None,
        }
    }

    /// Returns the current bandwidth mode.
    pub fn current_mode(&self) -> BandwidthMode {
        self.mode
    }

    /// Reset to initial state (call on disconnect).
    pub fn reset(&mut self) {
        self.mode = BandwidthMode::Full;
        self.poor_since = None;
        self.upgrade_candidate = None;
    }

    /// Convenience wrapper that uses `Instant::now()`.
    pub fn update(&mut self, quality: ConnectionQuality) -> Option<BandwidthMode> {
        self.update_with_time(quality, Instant::now())
    }

    /// Update the controller with a connection quality sample.
    /// Returns `Some(mode)` if the mode changed, `None` otherwise.
    pub fn update_with_time(
        &mut self,
        quality: ConnectionQuality,
        now: Instant,
    ) -> Option<BandwidthMode> {
        let target = self.compute_target(&quality, now);

        if target < self.mode {
            // Downgrade is immediate.
            self.set_mode(target);
            self.upgrade_candidate = None;
            return Some(self.mode);
        }

        if target > self.mode {
            // Upgrade requires hysteresis.
            match &self.upgrade_candidate {
                Some((candidate_mode, since)) if *candidate_mode == target => {
                    if now.duration_since(*since) >= UPGRADE_HYSTERESIS {
                        self.set_mode(target);
                        self.upgrade_candidate = None;
                        return Some(self.mode);
                    }
                    // Still waiting.
                    None
                }
                _ => {
                    // Start or restart hysteresis timer.
                    self.upgrade_candidate = Some((target, now));
                    None
                }
            }
        } else {
            // Same mode — no change. Clear upgrade candidate if any.
            self.upgrade_candidate = None;
            None
        }
    }

    /// Compute the target mode based on quality and timing.
    fn compute_target(&mut self, quality: &ConnectionQuality, now: Instant) -> BandwidthMode {
        match quality {
            ConnectionQuality::Excellent | ConnectionQuality::Good => {
                self.poor_since = None;
                BandwidthMode::Full
            }
            ConnectionQuality::Poor => {
                let poor_since = *self.poor_since.get_or_insert(now);
                if now.duration_since(poor_since) >= POOR_ESCALATION {
                    BandwidthMode::AudioOnly
                } else {
                    BandwidthMode::ReducedVideo
                }
            }
            ConnectionQuality::Lost => {
                self.poor_since = None;
                BandwidthMode::AudioOnly
            }
        }
    }

    fn set_mode(&mut self, mode: BandwidthMode) {
        self.mode = mode;
    }
}

impl Default for BandwidthController {
    fn default() -> Self {
        Self::new()
    }
}

// Ordering for comparison: Full > ReducedVideo > AudioOnly
// Higher = better quality.
impl PartialOrd for BandwidthMode {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BandwidthMode {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.rank().cmp(&other.rank())
    }
}

impl BandwidthMode {
    fn rank(self) -> u8 {
        match self {
            BandwidthMode::AudioOnly => 0,
            BandwidthMode::ReducedVideo => 1,
            BandwidthMode::Full => 2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_time() -> Instant {
        Instant::now()
    }

    #[test]
    fn starts_in_full_mode() {
        let ctrl = BandwidthController::new();
        assert_eq!(ctrl.current_mode(), BandwidthMode::Full);
    }

    #[test]
    fn poor_immediately_reduces_video() {
        let mut ctrl = BandwidthController::new();
        let t = base_time();
        let result = ctrl.update_with_time(ConnectionQuality::Poor, t);
        assert_eq!(result, Some(BandwidthMode::ReducedVideo));
        assert_eq!(ctrl.current_mode(), BandwidthMode::ReducedVideo);
    }

    #[test]
    fn poor_escalates_to_audio_only_after_3s() {
        let mut ctrl = BandwidthController::new();
        let t0 = base_time();

        ctrl.update_with_time(ConnectionQuality::Poor, t0);
        assert_eq!(ctrl.current_mode(), BandwidthMode::ReducedVideo);

        // Still Poor at 2s — no escalation yet.
        let result = ctrl.update_with_time(ConnectionQuality::Poor, t0 + Duration::from_secs(2));
        assert_eq!(result, None);
        assert_eq!(ctrl.current_mode(), BandwidthMode::ReducedVideo);

        // Poor at 3s — escalate.
        let result = ctrl.update_with_time(ConnectionQuality::Poor, t0 + Duration::from_secs(3));
        assert_eq!(result, Some(BandwidthMode::AudioOnly));
        assert_eq!(ctrl.current_mode(), BandwidthMode::AudioOnly);
    }

    #[test]
    fn lost_immediately_goes_audio_only() {
        let mut ctrl = BandwidthController::new();
        let t = base_time();
        let result = ctrl.update_with_time(ConnectionQuality::Lost, t);
        assert_eq!(result, Some(BandwidthMode::AudioOnly));
        assert_eq!(ctrl.current_mode(), BandwidthMode::AudioOnly);
    }

    #[test]
    fn good_does_not_restore_before_hysteresis() {
        let mut ctrl = BandwidthController::new();
        let t0 = base_time();

        // Go to ReducedVideo.
        ctrl.update_with_time(ConnectionQuality::Poor, t0);
        assert_eq!(ctrl.current_mode(), BandwidthMode::ReducedVideo);

        // Good at +1s — not enough time.
        let result = ctrl.update_with_time(ConnectionQuality::Good, t0 + Duration::from_secs(1));
        assert_eq!(result, None);
        assert_eq!(ctrl.current_mode(), BandwidthMode::ReducedVideo);

        // Good at +4s — still not enough (need 5s from first Good).
        let result = ctrl.update_with_time(ConnectionQuality::Good, t0 + Duration::from_secs(4));
        assert_eq!(result, None);
        assert_eq!(ctrl.current_mode(), BandwidthMode::ReducedVideo);
    }

    #[test]
    fn good_restores_after_hysteresis() {
        let mut ctrl = BandwidthController::new();
        let t0 = base_time();

        // Go to ReducedVideo.
        ctrl.update_with_time(ConnectionQuality::Poor, t0);
        assert_eq!(ctrl.current_mode(), BandwidthMode::ReducedVideo);

        // First Good sample starts hysteresis.
        let t_good = t0 + Duration::from_secs(1);
        ctrl.update_with_time(ConnectionQuality::Good, t_good);

        // Good at +6s (5s after first Good) — should restore.
        let result =
            ctrl.update_with_time(ConnectionQuality::Good, t_good + Duration::from_secs(5));
        assert_eq!(result, Some(BandwidthMode::Full));
        assert_eq!(ctrl.current_mode(), BandwidthMode::Full);
    }

    #[test]
    fn hysteresis_resets_on_quality_drop() {
        let mut ctrl = BandwidthController::new();
        let t0 = base_time();

        // Go to ReducedVideo.
        ctrl.update_with_time(ConnectionQuality::Poor, t0);

        // Start hysteresis with Good.
        let t1 = t0 + Duration::from_secs(1);
        ctrl.update_with_time(ConnectionQuality::Good, t1);

        // Drop back to Poor at +3s — resets hysteresis.
        let t2 = t0 + Duration::from_secs(3);
        ctrl.update_with_time(ConnectionQuality::Poor, t2);
        assert_eq!(ctrl.current_mode(), BandwidthMode::ReducedVideo);

        // Good again at +4s — hysteresis restarts from here.
        let t3 = t0 + Duration::from_secs(4);
        ctrl.update_with_time(ConnectionQuality::Good, t3);

        // +8s (4s after second Good) — not enough.
        let result = ctrl.update_with_time(ConnectionQuality::Good, t0 + Duration::from_secs(8));
        assert_eq!(result, None);

        // +9s (5s after second Good) — should restore.
        let result = ctrl.update_with_time(ConnectionQuality::Good, t3 + Duration::from_secs(5));
        assert_eq!(result, Some(BandwidthMode::Full));
    }

    #[test]
    fn excellent_and_good_are_equivalent_for_full() {
        let mut ctrl = BandwidthController::new();
        let t0 = base_time();

        // Go to ReducedVideo.
        ctrl.update_with_time(ConnectionQuality::Poor, t0);

        // Start hysteresis with Excellent.
        let t1 = t0 + Duration::from_secs(1);
        ctrl.update_with_time(ConnectionQuality::Excellent, t1);

        // Continue with Good — should keep the same hysteresis.
        let result = ctrl.update_with_time(ConnectionQuality::Good, t1 + Duration::from_secs(5));
        assert_eq!(result, Some(BandwidthMode::Full));
    }

    #[test]
    fn no_change_emitted_when_mode_stays_same() {
        let mut ctrl = BandwidthController::new();
        let t0 = base_time();

        // Already Full, Good should return None.
        let result = ctrl.update_with_time(ConnectionQuality::Good, t0);
        assert_eq!(result, None);

        // Excellent also returns None.
        let result =
            ctrl.update_with_time(ConnectionQuality::Excellent, t0 + Duration::from_secs(1));
        assert_eq!(result, None);
    }
}
