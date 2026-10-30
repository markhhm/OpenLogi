//! Public Universal Access state getter; the existing injector posts the shortcut.

const ZOOM_SHORTCUT: &str = "Alt+Cmd+8";

/// Read Zoom's actual state, including the SDK's zoomed-fully-out off state.
#[expect(
    unsafe_code,
    reason = "read-only, thread-safe Universal Access Zoom getter"
)]
pub(super) fn status() -> super::ZoomStatus {
    // SAFETY: the SDK marks UAZoomEnabled thread-safe, without arguments or preconditions.
    if unsafe { objc2_application_services::UAZoomEnabled() } {
        super::ZoomStatus::On
    } else {
        super::ZoomStatus::Off
    }
}

/// Post the standard Zoom shortcut without enabling or rewriting system hotkeys.
pub(super) fn toggle() {
    use openlogi_core::binding::{HeldKeys, KeyCombo};
    // The plan deliberately uses the standard shortcut; never enable or rewrite system hotkeys.
    match ZOOM_SHORTCUT.parse::<KeyCombo>() {
        // WindowServer shortcuts need modifier edges, including without the virtual keyboard.
        Ok(combo) => drop(crate::press_hold(&combo, HeldKeys::ModifiersOnly)),
        Err(error) => tracing::error!(%error, "invalid built-in Zoom shortcut"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_shortcut_is_option_command_eight() {
        let combo = ZOOM_SHORTCUT
            .parse::<openlogi_core::binding::KeyCombo>()
            .unwrap();
        assert!(combo.has_option() && combo.has_command());
        assert!(!combo.has_control() && !combo.has_shift() && !combo.has_super());
        assert_eq!(combo.key().ascii_char(), Some('8'));
    }
}
