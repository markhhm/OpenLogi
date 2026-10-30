//! Platforms without the native click-through highlight boundary yet.

use super::Frame;

/// Unavailable until this platform supplies native pointer and display tracking.
pub(super) fn capture() -> Option<Frame> {
    // shortcut: highlight is macOS-only, enable when native click-through and global display tracking are implemented here.
    None
}

/// Reject placement without a native click-through highlight window.
pub(super) fn place(_window: &mut gpui::Window, _frame: &Frame) -> anyhow::Result<()> {
    anyhow::bail!("pointer highlighting is not supported on this platform")
}
