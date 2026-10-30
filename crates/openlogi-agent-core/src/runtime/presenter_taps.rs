//! Sans-I/O top-button tap disambiguation; the button worker owns its deadline.

use std::time::{Duration, Instant};

/// A single release waits this long for a second release.
const DOUBLE_TAP_WINDOW: Duration = Duration::from_millis(300);

/// Output admitted by the top-button tap state machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TapEffect {
    /// Replay one complete left click through the injector.
    Click,
    /// Select the next hold effect without sending any click.
    CyclePresenterMode,
}

/// At most one released tap awaiting disambiguation for a capture source.
#[derive(Default)]
pub(super) struct PresenterTaps {
    deadline: Option<Instant>,
}

impl PresenterTaps {
    /// Accept a release; a late second tap owes the first click and starts anew.
    pub(super) fn release(&mut self, now: Instant) -> Option<TapEffect> {
        match self.deadline.take() {
            Some(deadline) if now < deadline => Some(TapEffect::CyclePresenterMode),
            previous => {
                self.deadline = Some(now + DOUBLE_TAP_WINDOW);
                previous.map(|_| TapEffect::Click)
            }
        }
    }

    /// Emit the pending click only once its window has ended.
    pub(super) fn expire(&mut self, now: Instant) -> Option<TapEffect> {
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            self.deadline = None;
            Some(TapEffect::Click)
        } else {
            None
        }
    }

    /// Cancel on hold, source loss or runtime invalidation, without replay.
    pub(super) fn cancel(&mut self) {
        self.deadline = None;
    }

    /// The worker's next wakeup, absent when no click is owed.
    pub(super) fn deadline(&self) -> Option<Instant> {
        self.deadline
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presenter_tap_transition_table() {
        // Exhaust all reachable state / deadline boundary / input combinations.
        let start = Instant::now();
        for pending in [false, true] {
            for elapsed in [
                Duration::ZERO,
                DOUBLE_TAP_WINDOW
                    .checked_sub(Duration::from_nanos(1))
                    .unwrap(),
                DOUBLE_TAP_WINDOW,
                DOUBLE_TAP_WINDOW + Duration::from_nanos(1),
            ] {
                for input in ["release", "expire", "cancel"] {
                    let mut taps = PresenterTaps::default();
                    if pending {
                        assert_eq!(taps.release(start), None);
                    }
                    let now = start + elapsed;
                    let actual = match input {
                        "release" => taps.release(now),
                        "expire" => taps.expire(now),
                        _ => {
                            taps.cancel();
                            None
                        }
                    };
                    let expired = elapsed >= DOUBLE_TAP_WINDOW;
                    let (expected, next) = match (pending, expired, input) {
                        (true, false, "release") => (Some(TapEffect::CyclePresenterMode), None),
                        (true, true, "release") => {
                            (Some(TapEffect::Click), Some(now + DOUBLE_TAP_WINDOW))
                        }
                        (false, _, "release") => (None, Some(now + DOUBLE_TAP_WINDOW)),
                        (true, true, "expire") => (Some(TapEffect::Click), None),
                        (true, false, "expire") => (None, Some(start + DOUBLE_TAP_WINDOW)),
                        _ => (None, None),
                    };
                    assert_eq!(
                        (actual, taps.deadline()),
                        (expected, next),
                        "pending={pending}, elapsed={elapsed:?}, input={input}"
                    );
                    taps.cancel();
                    assert_eq!(taps.expire(now + DOUBLE_TAP_WINDOW), None);
                }
            }
        }
    }
}
