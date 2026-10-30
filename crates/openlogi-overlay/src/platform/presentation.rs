//! Shared native window eligibility and ordering above captured presentations.

use anyhow::{Context as _, Result};
use core_graphics::display::CGShieldingWindowLevel;
use core_graphics::window::CGWindowLevel;
use objc2::{MainThreadMarker, rc::Retained};
use objc2_app_kit::{NSPanel, NSView, NSWindow, NSWindowCollectionBehavior, NSWindowLevel};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// Raise a live GPUI popup without taking keyboard focus or replacing GPUI's style mask.
#[expect(
    unsafe_code,
    reason = "borrow the live main-thread NSView and read the capture shielding level"
)]
pub(crate) fn raise(window: &gpui::Window) -> Result<Retained<NSWindow>> {
    let _marker =
        MainThreadMarker::new().context("presentation window requires the main thread")?;
    let handle = HasWindowHandle::window_handle(window)
        .map_err(|error| anyhow::anyhow!("get presentation AppKit handle: {error}"))?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        anyhow::bail!("presentation window has no AppKit handle");
    };
    // SAFETY: the borrowed handle is a live NSView, window outlives this borrow,
    // and the marker proves execution on AppKit's owning main thread.
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let native = view.window().context("presentation view has no window")?;
    // SAFETY: this read-only CoreGraphics getter has no arguments or preconditions.
    let shielding_level = unsafe { CGShieldingWindowLevel() };
    let (behavior, level) = presentation_policy(shielding_level)?;
    native.setCollectionBehavior(behavior);
    native.setLevel(level);
    native.setHasShadow(false);
    if let Some(panel) = native.downcast_ref::<NSPanel>() {
        panel.setBecomesKeyOnlyIfNeeded(true);
    }
    Ok(native)
}

fn presentation_policy(
    shielding_level: CGWindowLevel,
) -> Result<(NSWindowCollectionBehavior, NSWindowLevel)> {
    // Captured-display slides can cover screen-saver windows; stay just above their shield.
    let level = NSWindowLevel::try_from(shielding_level)? + 1;
    Ok((
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::IgnoresCycle,
        level,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn synchronous_geometry_callbacks_require_an_unborrowed_app(cx: &mut gpui::TestAppContext) {
        let window = cx.add_window(|_, _| gpui::Empty);
        let mut callback_cx = cx.update(|cx| cx.to_async());
        // GPUI's macOS resize/move delegates call WindowHandle::update through AsyncApp.
        let error = cx.update(|_| window.update(&mut callback_cx, |_, _, _| ()).unwrap_err());
        assert!(error.is::<std::cell::BorrowMutError>());
        assert_eq!(error.to_string(), "RefCell already borrowed");
        window.update(&mut callback_cx, |_, _, _| ()).unwrap();
    }

    #[test]
    fn presentation_policy_places_highlight_above_the_display_capture_shield() {
        for shielding in [101, 1000, i32::MAX - 16, i32::MAX] {
            let (_, level) = presentation_policy(shielding).unwrap();
            let shielding = isize::try_from(shielding).unwrap();
            assert!(
                level > shielding,
                "highlight level {level} is below shielding {shielding}"
            );
            assert_eq!(
                level - shielding,
                1,
                "use the nearest level above the shield"
            );
        }
    }

    #[test]
    fn presentation_policy_keeps_fullscreen_spaces_without_switching_or_cycling_focus() {
        use NSWindowCollectionBehavior as Behavior;
        let (behavior, _) = presentation_policy(1000).unwrap();
        for required in [
            Behavior::CanJoinAllSpaces,
            Behavior::FullScreenAuxiliary,
            Behavior::Stationary,
            Behavior::IgnoresCycle,
        ] {
            assert!(behavior.contains(required), "missing {required:?}");
        }
        assert!(!behavior.intersects(
            Behavior::MoveToActiveSpace
                | Behavior::Managed
                | Behavior::Transient
                | Behavior::ParticipatesInCycle
                | Behavior::FullScreenPrimary
                | Behavior::FullScreenNone
        ));
    }
}
