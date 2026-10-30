//! A physical press owns the highlight until its matching terminal event.

use openlogi_core::binding::PresenterMode;
use openlogi_ipc::{Generation, HighlightObservation, HighlightStyle, OBSERVE_HOLD};
use tokio::sync::watch;

use crate::runtime::PressToken;

#[derive(Clone)]
struct State {
    generation: Generation,
    owner: Option<(PressToken, PresenterMode)>,
    // shortcut: mode is per agent run; persist when presenter settings are introduced.
    mode: PresenterMode,
}

impl State {
    fn observation(&self) -> HighlightObservation {
        HighlightObservation::new(
            self.generation,
            self.owner.as_ref().and_then(|(_, mode)| match mode {
                PresenterMode::Spotlight => Some(HighlightStyle::Spotlight),
                PresenterMode::Laser => Some(HighlightStyle::Laser),
                PresenterMode::Magnifier => None,
            }),
        )
    }
}

/// Agent-owned highlight state, shared by the button runtime and IPC server.
#[derive(Clone)]
pub struct Highlight {
    published: watch::Sender<State>,
}

impl Default for Highlight {
    fn default() -> Self {
        Self {
            published: watch::Sender::new(State {
                generation: 1,
                owner: None,
                mode: PresenterMode::default(),
            }),
        }
    }
}

/// A native magnifier transition derived from the single highlight owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MagnifierChange {
    /// Enable Zoom if it is currently off.
    Show,
    /// Disable Zoom if it is currently on.
    Hide,
}

impl State {
    fn replace_owner(
        &mut self,
        owner: Option<(PressToken, PresenterMode)>,
    ) -> Option<MagnifierChange> {
        let before = self
            .owner
            .as_ref()
            .is_some_and(|(_, mode)| *mode == PresenterMode::Magnifier);
        let after = owner
            .as_ref()
            .is_some_and(|(_, mode)| *mode == PresenterMode::Magnifier);
        self.owner = owner;
        self.generation = self.generation.wrapping_add(1);
        match (before, after) {
            (false, true) => Some(MagnifierChange::Show),
            (true, false) => Some(MagnifierChange::Hide),
            _ => None,
        }
    }
}

impl Highlight {
    /// Select the next mode for future holds; an existing press retains its mode.
    pub(crate) fn cycle(&self) -> PresenterMode {
        let mut mode = PresenterMode::default();
        self.published.send_modify(|state| {
            state.mode = state.mode.next();
            mode = state.mode;
        });
        mode
    }

    /// Publish this owner; repeated triggers from the same press are idempotent.
    pub(crate) fn show(&self, press: &PressToken) -> Option<MagnifierChange> {
        let mut change = None;
        self.published.send_if_modified(|state| {
            if state
                .owner
                .as_ref()
                .is_some_and(|(owner, _)| owner == press)
            {
                return false;
            }
            change = state.replace_owner(Some((press.clone(), state.mode)));
            true
        });
        change
    }

    /// Hide only if this press still owns the highlight.
    pub(crate) fn end(&self, press: &PressToken) -> Option<MagnifierChange> {
        let mut change = None;
        self.published.send_if_modified(|state| {
            if !state
                .owner
                .as_ref()
                .is_some_and(|(owner, _)| owner == press)
            {
                return false;
            }
            change = state.replace_owner(None);
            true
        });
        change
    }

    /// Answer on a new generation, or unchanged after [`OBSERVE_HOLD`].
    pub async fn observe(&self, since: Generation) -> HighlightObservation {
        let mut receiver = self.published.subscribe();
        let changed = receiver.wait_for(|state| state.generation != since);
        match tokio::time::timeout(OBSERVE_HOLD, changed).await {
            Ok(Ok(state)) => state.observation(),
            Ok(Err(_)) | Err(_) => self.published.borrow().observation(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openlogi_core::binding::ButtonId;

    #[tokio::test(start_paused = true)]
    async fn highlight_ownership_rejects_a_stale_release() {
        let highlight = Highlight::default();
        assert_eq!(
            highlight.observe(0).await,
            HighlightObservation::new(1, None)
        );
        let first = PressToken::hook_for_test(1, ButtonId::Back);
        let second = PressToken::hook_for_test(2, ButtonId::Back);
        highlight.show(&first);
        assert_eq!(
            highlight.observe(1).await.shown,
            Some(HighlightStyle::Spotlight)
        );
        highlight.show(&first);
        assert_eq!(
            highlight.observe(0).await.generation,
            2,
            "a repeated trigger from the same press must not republish"
        );
        highlight.show(&second);
        highlight.end(&first);
        assert_eq!(
            highlight.observe(0).await,
            HighlightObservation::new(3, Some(HighlightStyle::Spotlight))
        );
        highlight.end(&second);
        assert_eq!(
            highlight.observe(3).await,
            HighlightObservation::new(4, None)
        );
        highlight.end(&second);
        assert_eq!(highlight.observe(4).await.generation, 4);
    }

    #[tokio::test(start_paused = true)]
    async fn highlight_has_no_timer_and_holds_unchanged_observations() {
        let highlight = Highlight::default();
        highlight.show(&PressToken::hook_for_test(1, ButtonId::Back));
        let before = tokio::time::Instant::now();
        let observation = highlight.observe(2).await;
        assert_eq!(tokio::time::Instant::now() - before, OBSERVE_HOLD);
        assert_eq!(observation.generation, 2);
        assert_eq!(observation.shown, Some(HighlightStyle::Spotlight));
    }

    #[tokio::test]
    async fn highlight_wakes_an_observer_on_press() {
        let highlight = Highlight::default();
        let observed = highlight.observe(1);
        tokio::pin!(observed);
        assert!(
            futures_lite::future::poll_once(&mut observed)
                .await
                .is_none()
        );
        highlight.show(&PressToken::hook_for_test(1, ButtonId::Back));
        assert_eq!(observed.await.shown, Some(HighlightStyle::Spotlight));
    }
    #[tokio::test]
    async fn modes_are_captured_by_the_press_and_zoom_cleanup_rejects_stale_owners() {
        let highlight = Highlight::default();
        let first = PressToken::hook_for_test(1, ButtonId::Back);
        let second = PressToken::hook_for_test(2, ButtonId::Back);
        assert_eq!(highlight.cycle(), PresenterMode::Laser);
        assert_eq!(highlight.show(&first), None);
        assert_eq!(
            highlight.observe(0).await.shown,
            Some(HighlightStyle::Laser)
        );
        assert_eq!(highlight.cycle(), PresenterMode::Magnifier);
        assert_eq!(
            highlight.show(&first),
            None,
            "cycling cannot change an existing press"
        );
        assert_eq!(
            highlight.observe(0).await.shown,
            Some(HighlightStyle::Laser)
        );
        assert_eq!(highlight.show(&second), Some(MagnifierChange::Show));
        assert_eq!(highlight.observe(0).await.shown, None, "Zoom is agent-side");
        assert_eq!(highlight.end(&first), None);
        assert_eq!(highlight.end(&second), Some(MagnifierChange::Hide));
        assert_eq!(highlight.end(&second), None);
        assert_eq!(highlight.show(&first), Some(MagnifierChange::Show));
        assert_eq!(
            highlight.show(&second),
            None,
            "successive magnifier owners keep Zoom on"
        );
        assert_eq!(highlight.cycle(), PresenterMode::Spotlight);
        assert_eq!(highlight.show(&first), Some(MagnifierChange::Hide));
        assert_eq!(
            highlight.observe(0).await.shown,
            Some(HighlightStyle::Spotlight)
        );
        assert_eq!(highlight.end(&second), None);
        assert_eq!(highlight.end(&first), None);
    }
}
