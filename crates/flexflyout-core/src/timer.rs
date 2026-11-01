//! When the flyout is shown and how opaque it is: fade in, hold, fade out.
//!
//! The timer takes the current time as an argument instead of reading the
//! clock, so the host decides when to look and the tests can step through
//! time exactly.

use std::time::{Duration, Instant};

/// Durations of the three phases of one appearance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    /// From invisible to fully opaque.
    pub fade_in: Duration,
    /// How long the flyout stays fully opaque (the "display time").
    pub hold: Duration,
    /// From fully opaque to invisible.
    pub fade_out: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            fade_in: Duration::from_millis(150),
            hold: Duration::from_millis(3000),
            fade_out: Duration::from_millis(400),
        }
    }
}

/// Tracks one appearance of the flyout and restarts it on every trigger
/// (a volume key, a volume change, a new track).
#[derive(Clone, Debug)]
pub struct FlyoutTimer {
    timing: Timing,
    /// When the current appearance started fading in, or `None` while hidden.
    start: Option<Instant>,
}

impl FlyoutTimer {
    /// A hidden flyout with the given timing.
    pub fn new(timing: Timing) -> Self {
        Self {
            timing,
            start: None,
        }
    }

    /// Changes the timing (after the settings are reloaded). An appearance
    /// in progress continues with the new durations.
    pub fn set_timing(&mut self, timing: Timing) {
        self.timing = timing;
    }

    /// Shows the flyout, or keeps it shown for another full display time.
    ///
    /// If it is already visible, it does not blink: a flyout that is fading
    /// out fades back in from its current opacity, and a fully opaque one
    /// stays opaque.
    pub fn trigger(&mut self, now: Instant) {
        let opacity = self.opacity(now);
        // Move the start back by the part of the fade-in that is already "done".
        let done = self.timing.fade_in.mul_f32(opacity);
        self.start = Some(now.checked_sub(done).unwrap_or(now));
    }

    /// Hides the flyout at once (e.g. when the user clicks it away).
    pub fn cancel(&mut self) {
        self.start = None;
    }

    /// Opacity at `now`, from 0.0 (invisible) to 1.0 (fully opaque).
    pub fn opacity(&self, now: Instant) -> f32 {
        let Some(start) = self.start else {
            return 0.0;
        };
        let Timing {
            fade_in,
            hold,
            fade_out,
        } = self.timing;
        let t = now.saturating_duration_since(start);
        if t < fade_in {
            return t.as_secs_f32() / fade_in.as_secs_f32();
        }
        let t = t - fade_in;
        if t < hold {
            return 1.0;
        }
        let t = t - hold;
        if t < fade_out {
            return 1.0 - t.as_secs_f32() / fade_out.as_secs_f32();
        }
        0.0
    }

    /// Whether the appearance has not finished yet at `now`.
    pub fn is_active(&self, now: Instant) -> bool {
        self.start
            .is_some_and(|start| now.saturating_duration_since(start) < self.total())
    }

    /// How long the host may sleep before the picture changes: zero while
    /// fading (repaint every frame), the rest of the display time while fully
    /// opaque, and `None` once the appearance has finished.
    pub fn next_change(&self, now: Instant) -> Option<Duration> {
        let start = self.start?;
        let t = now.saturating_duration_since(start);
        let Timing { fade_in, hold, .. } = self.timing;
        if t >= self.total() {
            None
        } else if t >= fade_in && t < fade_in + hold {
            Some(fade_in + hold - t)
        } else {
            Some(Duration::ZERO)
        }
    }

    /// Length of one whole appearance.
    fn total(&self) -> Duration {
        self.timing.fade_in + self.timing.hold + self.timing.fade_out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    /// 100 ms in, 1000 ms hold, 200 ms out: easy numbers to check.
    fn timer() -> FlyoutTimer {
        FlyoutTimer::new(Timing {
            fade_in: 100 * MS,
            hold: 1000 * MS,
            fade_out: 200 * MS,
        })
    }

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn hidden_until_triggered() {
        let t0 = Instant::now();
        let timer = timer();
        assert_eq!(timer.opacity(t0), 0.0);
        assert!(!timer.is_active(t0));
        assert_eq!(timer.next_change(t0), None);
    }

    #[test]
    fn fades_in_holds_and_fades_out() {
        let t0 = Instant::now();
        let mut timer = timer();
        timer.trigger(t0);
        assert!(approx(timer.opacity(t0), 0.0));
        assert!(approx(timer.opacity(t0 + 50 * MS), 0.5));
        assert!(approx(timer.opacity(t0 + 100 * MS), 1.0));
        assert!(approx(timer.opacity(t0 + 1099 * MS), 1.0));
        assert!(approx(timer.opacity(t0 + 1200 * MS), 0.5));
        assert!(approx(timer.opacity(t0 + 1300 * MS), 0.0));
        assert!(timer.is_active(t0 + 1299 * MS));
        assert!(!timer.is_active(t0 + 1300 * MS));
    }

    #[test]
    fn next_change_sleeps_through_the_hold() {
        let t0 = Instant::now();
        let mut timer = timer();
        timer.trigger(t0);
        // Fading in: repaint every frame.
        assert_eq!(timer.next_change(t0 + 50 * MS), Some(Duration::ZERO));
        // Fully opaque: nothing changes until the fade-out starts.
        assert_eq!(timer.next_change(t0 + 300 * MS), Some(800 * MS));
        // Fading out: repaint every frame again.
        assert_eq!(timer.next_change(t0 + 1150 * MS), Some(Duration::ZERO));
        // Finished.
        assert_eq!(timer.next_change(t0 + 1300 * MS), None);
    }

    #[test]
    fn retrigger_while_opaque_restarts_the_hold_without_blinking() {
        let t0 = Instant::now();
        let mut timer = timer();
        timer.trigger(t0);
        let t1 = t0 + 900 * MS;
        timer.trigger(t1);
        assert!(approx(timer.opacity(t1), 1.0));
        // A full hold from t1 on.
        assert!(approx(timer.opacity(t1 + 999 * MS), 1.0));
        assert!(approx(timer.opacity(t1 + 1100 * MS), 0.5));
    }

    #[test]
    fn retrigger_while_fading_out_fades_back_in_from_there() {
        let t0 = Instant::now();
        let mut timer = timer();
        timer.trigger(t0);
        // Halfway through the fade-out.
        let t1 = t0 + 1200 * MS;
        timer.trigger(t1);
        assert!(approx(timer.opacity(t1), 0.5));
        assert!(approx(timer.opacity(t1 + 50 * MS), 1.0));
    }

    #[test]
    fn zero_durations_do_not_divide_by_zero() {
        let t0 = Instant::now();
        let mut timer = FlyoutTimer::new(Timing {
            fade_in: Duration::ZERO,
            hold: 1000 * MS,
            fade_out: Duration::ZERO,
        });
        timer.trigger(t0);
        assert_eq!(timer.opacity(t0), 1.0);
        assert_eq!(timer.opacity(t0 + 1000 * MS), 0.0);
    }

    #[test]
    fn cancel_hides_at_once() {
        let t0 = Instant::now();
        let mut timer = timer();
        timer.trigger(t0);
        timer.cancel();
        assert_eq!(timer.opacity(t0 + 500 * MS), 0.0);
        assert!(!timer.is_active(t0 + 500 * MS));
    }
}
