//! Platforms without the native click-through highlight boundary yet.

use super::Frame;

pub(super) fn capture() -> Option<Frame> {
    // shortcut: highlight is macOS-only, enable when native click-through and global display tracking are implemented here.
    None
}

pub(super) fn place(_window: &mut gpui::Window, _frame: &Frame) -> anyhow::Result<()> {
    anyhow::bail!("pointer highlighting is not supported on this platform")
}
