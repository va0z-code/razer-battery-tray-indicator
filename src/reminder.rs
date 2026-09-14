// Low-battery reminder schedule. Pure logic (no clock, no Win32) so it can be
// unit-tested: the caller passes `now` and whether a game is running.
//
// Low (<= LOW_LEVEL): first alert immediately, then repeats after
// 5, 10, 15, 20, 25, 30 minutes, then every 30 minutes.
// Critical (<= CRITICAL_LEVEL): repeats every 5 minutes.
// A reminder that falls due while a game is running is held back and shown
// once, right after the game ends.

use std::time::{Duration, Instant};

pub const LOW_LEVEL: i32 = 30;
pub const CRITICAL_LEVEL: i32 = 5;

/// Minutes to wait after the n-th alert (1-based) in the low schedule.
const LOW_SCHEDULE: [u32; 6] = [5, 10, 15, 20, 25, 30];
const CRITICAL_INTERVAL: u32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Alert {
    /// 1 for the first alert, 2 for the first repeat, ...
    pub count: u32,
    pub critical: bool,
}

#[derive(Debug)]
struct Active {
    sent: u32,
    next_due: Instant,
    critical: bool,
}

#[derive(Debug, Default)]
pub struct Reminder {
    active: Option<Active>,
}

impl Reminder {
    /// `unit` is the length of one schedule "minute" (60 s normally, 1 s with
    /// `--fast-reminders`).
    pub fn update(
        &mut self,
        now: Instant,
        level: i32,
        charging: bool,
        game_running: bool,
        unit: Duration,
    ) -> Option<Alert> {
        if level < 0 || charging || level > LOW_LEVEL {
            self.active = None;
            return None;
        }

        let critical = level <= CRITICAL_LEVEL;
        let active = self.active.get_or_insert(Active {
            sent: 0,
            next_due: now,
            critical,
        });

        // Dropping into critical skips the rest of the low wait.
        if critical && !active.critical {
            active.next_due = now;
        }
        active.critical = critical;

        if now < active.next_due || game_running {
            return None;
        }

        active.sent += 1;
        let minutes = if critical {
            CRITICAL_INTERVAL
        } else {
            let idx = (active.sent as usize - 1).min(LOW_SCHEDULE.len() - 1);
            LOW_SCHEDULE[idx]
        };
        active.next_due = now + unit * minutes;

        Some(Alert {
            count: active.sent,
            critical,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Duration = Duration::from_secs(60);

    fn at(start: Instant, minutes: u64) -> Instant {
        start + Duration::from_secs(minutes * 60)
    }

    #[test]
    fn no_alert_above_low_level_or_while_charging() {
        let mut r = Reminder::default();
        let t = Instant::now();
        assert_eq!(r.update(t, 31, false, false, MIN), None);
        assert_eq!(r.update(t, 10, true, false, MIN), None);
        assert_eq!(r.update(t, -1, false, false, MIN), None);
    }

    #[test]
    fn alerts_immediately_including_at_startup() {
        let mut r = Reminder::default();
        let t = Instant::now();
        let alert = r.update(t, 23, false, false, MIN);
        assert_eq!(
            alert,
            Some(Alert {
                count: 1,
                critical: false
            })
        );
    }

    #[test]
    fn follows_escalating_schedule_then_every_30_minutes() {
        let mut r = Reminder::default();
        let t = Instant::now();
        // Alerts at 0, +5, +10, +15, +20, +25, +30, +30 → cumulative minutes.
        let expected = [0u64, 5, 15, 30, 50, 75, 105, 135, 165];

        let mut fired = Vec::new();
        for m in 0..=170 {
            if r.update(at(t, m), 25, false, false, MIN).is_some() {
                fired.push(m);
            }
        }
        assert_eq!(fired, expected);
    }

    #[test]
    fn resets_when_charging() {
        let mut r = Reminder::default();
        let t = Instant::now();
        assert!(r.update(t, 25, false, false, MIN).is_some());
        assert!(r.update(at(t, 1), 25, true, false, MIN).is_none());
        // Unplugged again: schedule starts over with an immediate alert #1.
        let alert = r.update(at(t, 2), 25, false, false, MIN).unwrap();
        assert_eq!(alert.count, 1);
    }

    #[test]
    fn critical_repeats_every_5_minutes() {
        let mut r = Reminder::default();
        let t = Instant::now();
        let mut fired = Vec::new();
        for m in 0..=20 {
            if r.update(at(t, m), 4, false, false, MIN).is_some() {
                fired.push(m);
            }
        }
        assert_eq!(fired, [0, 5, 10, 15, 20]);
    }

    #[test]
    fn dropping_to_critical_alerts_right_away() {
        let mut r = Reminder::default();
        let t = Instant::now();
        assert!(r.update(t, 20, false, false, MIN).is_some());
        // Next low reminder would be at +5; critical interrupts at +2.
        let alert = r.update(at(t, 2), 5, false, false, MIN).unwrap();
        assert!(alert.critical);
        assert_eq!(alert.count, 2);
    }

    #[test]
    fn held_during_game_and_shown_once_after() {
        let mut r = Reminder::default();
        let t = Instant::now();
        assert!(r.update(t, 25, false, false, MIN).is_some());

        // Game from minute 3 to minute 40: nothing shown.
        for m in 3..=40 {
            assert!(r.update(at(t, m), 25, false, true, MIN).is_none());
        }
        // Game over: the held reminder fires once...
        assert_eq!(r.update(at(t, 41), 25, false, false, MIN).unwrap().count, 2);
        // ...and the schedule continues from there (+10 min).
        assert!(r.update(at(t, 42), 25, false, false, MIN).is_none());
        assert!(r.update(at(t, 51), 25, false, false, MIN).is_some());
    }

    #[test]
    fn unit_scales_the_schedule() {
        let mut r = Reminder::default();
        let t = Instant::now();
        let sec = Duration::from_secs(1);
        assert!(r.update(t, 25, false, false, sec).is_some());
        assert!(r
            .update(t + Duration::from_secs(4), 25, false, false, sec)
            .is_none());
        assert!(r
            .update(t + Duration::from_secs(5), 25, false, false, sec)
            .is_some());
    }
}
