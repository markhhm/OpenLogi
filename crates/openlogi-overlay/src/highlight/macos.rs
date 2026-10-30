//! The highlight's native cursor, display geometry and click-through window.

use anyhow::{Context as _, Result};
use core_graphics::display::CGShieldingWindowLevel;
use core_graphics::window::CGWindowLevel;
use gpui::{Bounds, DisplayId, Window, point, px, size};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSScreen, NSView, NSWindowCollectionBehavior, NSWindowLevel};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::Frame;

/// Capture the pointer and its containing display in global logical points.
#[expect(
    clippy::cast_possible_truncation,
    reason = "screen-sized logical coordinates fit GPUI's f32 pixels"
)]
pub(super) fn capture() -> Option<Frame> {
    let cursor = openlogi_hook::cursor_position()?;
    let display = crate::platform::display_containing(cursor.x, cursor.y)?;
    Some(Frame {
        display_id: DisplayId::from(display.id),
        bounds: Bounds::new(
            point(px(display.origin.0 as f32), px(display.origin.1 as f32)),
            size(px(display.size.0 as f32), px(display.size.1 as f32)),
        ),
        pointer: point(px(cursor.x as f32), px(cursor.y as f32)),
    })
}

/// Place the click-through popup above captured presentations without taking focus.
#[expect(
    unsafe_code,
    reason = "borrow GPUI's main-thread NSView and read the CoreGraphics shielding level"
)]
pub(super) fn place(window: &mut Window, frame: &Frame) -> Result<()> {
    let marker = MainThreadMarker::new().context("highlight placement requires the main thread")?;
    let handle = HasWindowHandle::window_handle(window)
        .map_err(|error| anyhow::anyhow!("get highlight AppKit handle: {error}"))?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        anyhow::bail!("highlight window has no AppKit handle");
    };
    // SAFETY: the borrowed handle guarantees a live NSView; `window` outlives this
    // borrow, and the marker proves we are on AppKit's owning main thread.
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let native = view.window().context("highlight view has no window")?;
    native.setIgnoresMouseEvents(true);
    native.setHasShadow(false);
    // SAFETY: this read-only CoreGraphics getter has no arguments or preconditions.
    let shielding_level = unsafe { CGShieldingWindowLevel() };
    let (behavior, level) = presentation_policy(shielding_level)?;
    native.setCollectionBehavior(behavior);
    native.setLevel(level);

    // CG global points start at the primary display's top-left; AppKit starts at its bottom-left.
    let primary = NSScreen::screens(marker)
        .firstObject()
        .context("no primary display")?;
    let mut rect = native.frame();
    rect.origin.x = f64::from(f32::from(frame.bounds.origin.x));
    rect.origin.y = primary.frame().size.height
        - f64::from(f32::from(frame.bounds.origin.y + frame.bounds.size.height));
    rect.size.width = f64::from(f32::from(frame.bounds.size.width));
    rect.size.height = f64::from(f32::from(frame.bounds.size.height));
    native.setFrame_display(rect, true);
    Ok(())
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
