//! The highlight's native cursor, display geometry and click-through window.

use anyhow::{Context as _, Result};
use gpui::{Bounds, DisplayId, Window, point, px, size};
use objc2::MainThreadMarker;
use objc2_app_kit::NSScreen;

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
pub(super) fn place(window: &mut Window, frame: &Frame) -> Result<()> {
    let marker = MainThreadMarker::new().context("highlight placement requires the main thread")?;
    let native = crate::platform::presentation::raise(window)?;
    native.setIgnoresMouseEvents(true);

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
