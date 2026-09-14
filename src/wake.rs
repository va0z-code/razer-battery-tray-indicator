// Retry schedule for a mouse that stopped answering (asleep, out of range,
// or not yet paired after boot/resume). Pure logic, like reminder.rs.
//
// A sleeping wireless mouse wakes when it is moved, so a retry is only worth
// it after new user input. Failed retries back off (5 s, 10 s, 20 s, ... up
// to RETRY_GAP_MAX) so a mouse that is switched off isn't hammered with HID
// requests while the user keeps typing. Coming back after a long idle resets
// the backoff: that is exactly when the mouse is most likely waking up.

use std::time::{Duration, Instant};

const RETRY_GAP_MIN: Duration = Duration::from_secs(5);
const RETRY_GAP_MAX: Duration = Duration::from_secs(120);
/// Input after at least this much idle time counts as "the user is back".
const RETURN_IDLE: Duration = Duration::from_secs(60);

#[derive(Debug, Default)]
pub struct WakeRetry {
    failures: u32,
    last_attempt: Option<Instant>,
}

impl WakeRetry {
    /// True when a stale device should be read again now.
    pub fn due(&self, now: Instant, last_input: Option<Instant>) -> bool {
        let Some(last) = self.last_attempt else {
            return true;
        };
        now.duration_since(last) >= self.gap() && last_input.is_some_and(|t| t > last)
    }

    pub fn record(&mut self, now: Instant, success: bool) {
        if success {
            *self = Self::default();
        } else {
            self.failures = self.failures.saturating_add(1);
            self.last_attempt = Some(now);
        }
    }

    /// Forget the backoff but keep the attempt time, so a retry still waits
    /// for fresh input.
    pub fn reset_backoff(&mut self) {
        self.failures = 0;
    }

    fn gap(&self) -> Duration {
        let shift = self.failures.saturating_sub(1).min(16);
        (RETRY_GAP_MIN * (1 << shift)).min(RETRY_GAP_MAX)
    }
}

/// Turns periodic idle-time samples into input events.
#[derive(Debug, Default)]
pub struct Activity {
    last_idle: Option<Duration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputSample {
    /// When the last keyboard/mouse input happened.
    pub last_input: Option<Instant>,
    /// New input arrived after a long idle period.
    pub returned: bool,
}

impl Activity {
    pub fn sample(&mut self, now: Instant, idle: Option<Duration>) -> InputSample {
        let returned = match (self.last_idle, idle) {
            (Some(prev), Some(cur)) => prev >= RETURN_IDLE && cur < prev,
            _ => false,
        };
        if idle.is_some() {
            self.last_idle = idle;
        }
        InputSample {
            last_input: idle.and_then(|idle| now.checked_sub(idle)),
            returned,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn retries_only_after_input() {
        let t = Instant::now();
        let mut r = WakeRetry::default();
        r.record(t, false);
        assert!(!r.due(t + secs(30), Some(t - secs(1))));
        assert!(r.due(t + secs(30), Some(t + secs(29))));
    }

    #[test]
    fn backs_off_and_caps() {
        let t = Instant::now();
        let mut r = WakeRetry::default();
        let mut gaps = Vec::new();
        for _ in 0..8 {
            r.record(t, false);
            gaps.push(r.gap().as_secs());
        }
        assert_eq!(gaps, vec![5, 10, 20, 40, 80, 120, 120, 120]);
        assert!(!r.due(t + secs(119), Some(t + secs(100))));
        assert!(r.due(t + secs(120), Some(t + secs(100))));
    }

    #[test]
    fn success_and_return_reset() {
        let t = Instant::now();
        let mut r = WakeRetry::default();
        for _ in 0..6 {
            r.record(t, false);
        }
        r.reset_backoff();
        assert!(r.due(t + secs(5), Some(t + secs(4))));
        r.record(t, true);
        assert!(r.due(t, None));
    }

    #[test]
    fn detects_return_from_idle() {
        let t = Instant::now();
        let mut a = Activity::default();
        assert!(!a.sample(t, Some(secs(10))).returned);
        assert!(!a.sample(t, Some(secs(3))).returned);
        assert!(!a.sample(t, Some(secs(90))).returned);
        let s = a.sample(t + secs(5), Some(secs(1)));
        assert!(s.returned);
        assert_eq!(s.last_input, Some(t + secs(4)));
    }
}
