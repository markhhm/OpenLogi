//! Serial, state-checked macOS Zoom requests; posted shortcuts settle asynchronously.

use std::io;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
#[path = "zoom/macos.rs"]
mod native;
#[cfg(not(target_os = "macos"))]
#[path = "zoom/unsupported.rs"]
mod native;

const SETTLE_TIMEOUT: Duration = Duration::from_secs(1);
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// A native Zoom state, or a platform without that capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ZoomStatus {
    On,
    Off,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    On,
    Off,
}
impl Target {
    fn matches(self, status: ZoomStatus) -> bool {
        matches!(
            (self, status),
            (Self::On, ZoomStatus::On) | (Self::Off, ZoomStatus::Off)
        )
    }
}
#[derive(Clone, Copy)]
enum Command {
    Show,
    Hide,
    Stop,
}
#[derive(Default, PartialEq, Eq)]
/// A lease for the effect, independent of the native state sampled for each toggle.
enum Phase {
    #[default]
    Idle,
    Checking,
    Active,
    Stopping,
    Stopped,
}
struct Pending {
    target: Target,
    until: Instant,
}
#[derive(Default)]
struct ZoomState {
    requested: Option<Target>,
    pending: Option<Pending>,
    phase: Phase,
}
#[derive(Debug, PartialEq, Eq)]
enum Step {
    Idle,
    Toggle,
    Failed,
    Unavailable,
    Done,
}

impl ZoomState {
    fn request(&mut self, command: Command) {
        match command {
            Command::Show => {
                if self.phase != Phase::Active {
                    self.phase = Phase::Checking;
                }
                self.requested = Some(Target::On);
            }
            Command::Hide if self.phase == Phase::Active => self.requested = Some(Target::Off),
            Command::Hide => {
                self.requested = None;
                self.phase = Phase::Idle;
            }
            Command::Stop => {
                if matches!(self.phase, Phase::Active | Phase::Stopping) {
                    self.phase = Phase::Stopping;
                    self.requested = Some(Target::Off);
                } else {
                    self.phase = Phase::Stopped;
                }
            }
        }
    }
    fn deadline(&self, now: Instant) -> Option<Instant> {
        self.pending
            .as_ref()
            .map(|pending| pending.until.min(now + POLL_INTERVAL))
    }
    fn step(&mut self, now: Instant, status: ZoomStatus) -> Step {
        if self.phase == Phase::Stopped {
            return Step::Done;
        }
        if self.requested.is_none() {
            return if self.phase == Phase::Stopping {
                Step::Done
            } else {
                Step::Idle
            };
        }
        if status == ZoomStatus::Unavailable {
            self.requested = None;
            self.pending = None;
            return Step::Unavailable;
        }
        if let Some(pending) = &self.pending {
            if pending.target.matches(status) {
                self.pending = None;
            } else if now < pending.until {
                return Step::Idle;
            } else {
                self.pending = None;
                self.requested = None;
                return Step::Failed;
            }
        }
        let Some(target) = self.requested else {
            return Step::Idle;
        };
        if target.matches(status) {
            self.requested = None;
            if self.phase == Phase::Checking
                || (target == Target::Off && self.phase == Phase::Active)
            {
                self.phase = Phase::Idle;
            }
            return if self.phase == Phase::Stopping {
                Step::Done
            } else {
                Step::Idle
            };
        }
        if target == Target::On {
            self.phase = Phase::Active;
        }
        self.pending = Some(Pending {
            target,
            until: now + SETTLE_TIMEOUT,
        });
        Step::Toggle
    }
}

/// A worker that serializes Zoom on/off requests and settles before shutdown.
/// It never changes Zoom until its first [`Self::show`] request.
/// The system Zoom style and level remain the user's settings.
/// Other platforms log unavailability once and post no input.
pub struct Zoom {
    commands: mpsc::Sender<Command>,
    worker: Option<JoinHandle<()>>,
}
impl Zoom {
    /// Start an idle worker; no native state is read and no input is posted here.
    pub fn spawn() -> io::Result<Self> {
        let (commands, receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("openlogi-zoom".into())
            .spawn(move || run(&receiver))?;
        Ok(Self {
            commands,
            worker: Some(worker),
        })
    }
    /// Enable Zoom only when its current state is off.
    pub fn show(&self) {
        let _ = self.commands.send(Command::Show);
    }
    /// Disable Zoom only if this hold enabled it and its current state is on.
    pub fn hide(&self) {
        let _ = self.commands.send(Command::Hide);
    }
}
impl Drop for Zoom {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run(commands: &mpsc::Receiver<Command>) {
    let mut state = ZoomState::default();
    let mut warned = false;
    loop {
        let command = match state.deadline(Instant::now()) {
            Some(deadline) => {
                commands.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            }
            None => commands
                .recv()
                .map_err(|_| mpsc::RecvTimeoutError::Disconnected),
        };
        match command {
            Ok(command) => state.request(command),
            Err(mpsc::RecvTimeoutError::Disconnected) => state.request(Command::Stop),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        loop {
            // Idle construction/shutdown never touches the user's native Zoom state.
            let status = if matches!(state.phase, Phase::Idle | Phase::Stopped) {
                ZoomStatus::Unavailable
            } else {
                native::status()
            };
            match state.step(Instant::now(), status) {
                Step::Toggle => {
                    native::toggle();
                    break;
                }
                Step::Idle => break,
                Step::Done => return,
                Step::Failed | Step::Unavailable => {
                    if !warned {
                        tracing::warn!(
                            "Zoom did not settle or is unavailable; on macOS enable Zoom keyboard shortcuts (Option+Command+8) and choose Picture-in-Picture"
                        );
                        warned = true;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_shutdown_leaves_a_manual_enable_after_release_untouched() {
        let mut state = ZoomState::default();
        let now = Instant::now();
        state.request(Command::Show);
        assert_eq!(state.step(now, ZoomStatus::Off), Step::Toggle);
        assert_eq!(state.step(now, ZoomStatus::On), Step::Idle);
        state.request(Command::Hide);
        assert_eq!(state.step(now, ZoomStatus::On), Step::Toggle);
        assert_eq!(state.step(now, ZoomStatus::Off), Step::Idle);
        state.request(Command::Stop);
        assert_eq!(state.step(now, ZoomStatus::On), Step::Done);
    }

    #[test]
    fn zoom_reads_state_before_each_toggle_and_honors_manual_changes() {
        for initially_on in [false, true] {
            for manually_off in [false, true] {
                let mut state = ZoomState::default();
                let now = Instant::now();
                state.request(Command::Show);
                let initial = if initially_on {
                    ZoomStatus::On
                } else {
                    ZoomStatus::Off
                };
                assert_eq!(
                    state.step(now, initial),
                    if initially_on {
                        Step::Idle
                    } else {
                        Step::Toggle
                    }
                );
                assert_eq!(state.step(now, ZoomStatus::On), Step::Idle);
                state.request(Command::Hide);
                let at_release = if manually_off {
                    ZoomStatus::Off
                } else {
                    ZoomStatus::On
                };
                let expected = if !initially_on && !manually_off {
                    Step::Toggle
                } else {
                    Step::Idle
                };
                assert_eq!(
                    state.step(now, at_release),
                    expected,
                    "initially_on={initially_on}, manually_off={manually_off}"
                );
                if expected == Step::Toggle {
                    assert_eq!(state.step(now, ZoomStatus::Off), Step::Idle);
                }
                // A subsequent hold must not inherit the previous hold's ownership.
                state.request(Command::Show);
                assert_eq!(state.step(now, ZoomStatus::On), Step::Idle);
                state.request(Command::Hide);
                assert_eq!(state.step(now, ZoomStatus::On), Step::Idle);
                state.request(Command::Stop);
                assert_eq!(state.step(now, ZoomStatus::On), Step::Done);
            }
        }
    }
    #[test]
    fn zoom_release_waits_for_a_posted_enable_before_disabling() {
        let mut state = ZoomState::default();
        let now = Instant::now();
        state.request(Command::Show);
        assert_eq!(state.step(now, ZoomStatus::Off), Step::Toggle);
        state.request(Command::Hide);
        assert_eq!(state.step(now, ZoomStatus::Off), Step::Idle);
        assert_eq!(
            state.step(now + POLL_INTERVAL, ZoomStatus::On),
            Step::Toggle
        );
        assert_eq!(state.step(now + POLL_INTERVAL, ZoomStatus::Off), Step::Idle);
        assert_eq!(state.deadline(now), None);
    }
    #[test]
    fn zoom_disabled_unavailable_and_shutdown_paths_are_bounded() {
        let now = Instant::now();
        let mut state = ZoomState::default();
        state.request(Command::Hide);
        assert_eq!(state.step(now, ZoomStatus::Unavailable), Step::Idle);
        state.request(Command::Show);
        assert_eq!(state.step(now, ZoomStatus::Off), Step::Toggle);
        assert_eq!(
            state.step(now + SETTLE_TIMEOUT, ZoomStatus::Off),
            Step::Failed
        );
        assert_eq!(state.deadline(now), None);
        state.request(Command::Show);
        assert_eq!(state.step(now, ZoomStatus::Unavailable), Step::Unavailable);
        state.request(Command::Stop);
        assert_eq!(state.step(now, ZoomStatus::Unavailable), Step::Unavailable);
        assert_eq!(state.step(now, ZoomStatus::Unavailable), Step::Done);
        let mut idle = ZoomState::default();
        idle.request(Command::Stop);
        assert_eq!(idle.step(now, ZoomStatus::Unavailable), Step::Done);
    }
}
