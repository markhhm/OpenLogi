//! The presenter hold effect selected for this agent run.

use serde::{Deserialize, Serialize};

/// Presenter hold modes, append-only when carried by an IPC notice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresenterMode {
    /// Dim the display around the pointer.
    #[default]
    Spotlight,
    /// Draw a red pointer dot without dimming.
    Laser,
    /// Hold macOS's configured Zoom lens open.
    Magnifier,
}

impl PresenterMode {
    /// The next mode selected by a top-button double-click.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Spotlight => Self::Laser,
            Self::Laser => Self::Magnifier,
            Self::Magnifier => Self::Spotlight,
        }
    }

    /// The shared locale key naming this effect.
    #[must_use]
    pub const fn translation_key(self) -> &'static str {
        match self {
            Self::Spotlight => "spotlight.mode_spotlight",
            Self::Laser => "spotlight.mode_laser",
            Self::Magnifier => "spotlight.mode_magnifier",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presenter_modes_cycle_back_to_the_default() {
        let mut mode = PresenterMode::default();
        for expected in [
            PresenterMode::Laser,
            PresenterMode::Magnifier,
            PresenterMode::Spotlight,
        ] {
            mode = mode.next();
            assert_eq!(mode, expected);
        }
    }
}
