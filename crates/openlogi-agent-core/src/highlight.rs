//! A physical press owns the highlight until its matching terminal event.

use openlogi_ipc::{Generation, HighlightObservation, HighlightStyle, OBSERVE_HOLD};
use tokio::sync::watch;

use crate::runtime::PressToken;

#[derive(Clone)]
struct State {
    generation: Generation,
    owner: Option<PressToken>,
}

impl State {
    fn observation(&self) -> HighlightObservation {
        HighlightObservation::new(
            self.generation,
            self.owner.as_ref().map(|_| HighlightStyle::Spotlight),
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
            }),
        }
    }
}

impl Highlight {
    /// Publish this owner; repeated triggers from the same press are idempotent.
    pub(crate) fn show(&self, press: &PressToken) {
        self.published.send_if_modified(|state| {
            if state.owner.as_ref() == Some(press) {
                return false;
            }
            state.owner = Some(press.clone());
            state.generation = state.generation.wrapping_add(1);
            true
        });
    }

    /// Hide only if this press still owns the highlight.
    pub(crate) fn end(&self, press: &PressToken) {
        self.published.send_if_modified(|state| {
            if state.owner.as_ref() != Some(press) {
                return false;
            }
            state.owner = None;
            state.generation = state.generation.wrapping_add(1);
            true
        });
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
}
