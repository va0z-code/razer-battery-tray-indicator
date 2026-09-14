// Debounces "connected" / "disconnected" toasts. Plugging in the cable
// switches a mouse from its wireless PID to its wired PID (and back), which
// used to produce a Disconnected + Connected pair every time. Events are held
// for a short window; an opposite event for the same physical mouse inside
// that window cancels both.

use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Connected,
    Disconnected,
}

#[derive(Debug)]
struct Pending {
    kind: Kind,
    name: String,
    at: Instant,
}

#[derive(Debug)]
pub struct Debouncer {
    window: Duration,
    pending: Vec<Pending>,
}

/// "Razer DeathAdder V3 Pro (Wired)" → "Razer DeathAdder V3 Pro".
pub fn base_name(name: &str) -> &str {
    match name.rfind(" (") {
        Some(idx) if name.ends_with(')') => &name[..idx],
        _ => name,
    }
}

impl Debouncer {
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            pending: Vec::new(),
        }
    }

    pub fn push(&mut self, kind: Kind, name: &str, now: Instant) {
        let base = base_name(name);
        if let Some(idx) = self
            .pending
            .iter()
            .position(|p| p.kind != kind && base_name(&p.name) == base)
        {
            self.pending.remove(idx);
            return;
        }
        self.pending.push(Pending {
            kind,
            name: name.to_owned(),
            at: now,
        });
    }

    /// Returns events whose window has passed, oldest first.
    pub fn drain_due(&mut self, now: Instant) -> Vec<(Kind, String)> {
        let window = self.window;
        let (due, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|p| now.duration_since(p.at) >= window);
        self.pending = keep;
        due.into_iter().map(|p| (p.kind, p.name)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: Duration = Duration::from_secs(10);

    #[test]
    fn base_name_strips_connection_suffix() {
        assert_eq!(
            base_name("Razer DeathAdder V3 Pro (Wired)"),
            "Razer DeathAdder V3 Pro"
        );
        assert_eq!(base_name("Razer Orochi V2 (2.4 GHz)"), "Razer Orochi V2");
        assert_eq!(base_name("Plain"), "Plain");
    }

    #[test]
    fn emits_after_window() {
        let mut d = Debouncer::new(W);
        let t = Instant::now();
        d.push(Kind::Connected, "Mouse (Wireless)", t);
        assert!(d.drain_due(t + Duration::from_secs(9)).is_empty());
        assert_eq!(
            d.drain_due(t + W),
            vec![(Kind::Connected, "Mouse (Wireless)".to_owned())]
        );
        assert!(d.drain_due(t + W * 2).is_empty());
    }

    #[test]
    fn wired_wireless_swap_is_silent() {
        let mut d = Debouncer::new(W);
        let t = Instant::now();
        d.push(Kind::Disconnected, "Mouse (Wireless)", t);
        d.push(Kind::Connected, "Mouse (Wired)", t + Duration::from_secs(2));
        assert!(d.drain_due(t + W * 2).is_empty());
    }

    #[test]
    fn different_mice_do_not_cancel() {
        let mut d = Debouncer::new(W);
        let t = Instant::now();
        d.push(Kind::Disconnected, "Mouse A (Wireless)", t);
        d.push(Kind::Connected, "Mouse B (Wired)", t);
        assert_eq!(d.drain_due(t + W).len(), 2);
    }
}
