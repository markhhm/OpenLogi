//! No native magnifier is supplied on these platforms.

/// Report the absence of a native Zoom implementation.
pub(super) fn status() -> super::ZoomStatus {
    // shortcut: magnifier is macOS-only, implement native state-checked zoom before enabling other platforms.
    super::ZoomStatus::Unavailable
}
/// No native events are available on this platform.
pub(super) fn toggle() {}
