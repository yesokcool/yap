//! Screen sharing (PROTOCOL.md › Screen sharing): H.264 video (and the
//! computer's sound, stereo Opus) over the voice peer connections, only to
//! people who ask. `share` is our own share, `watch` the one we're watching,
//! `codec` OpenH264 both ways, `capture` the OS backends.

pub mod capture;
pub mod codec;
mod share;
#[cfg(target_os = "macos")]
mod vt;
mod watch;

pub use share::{EncodedAudio, EncodedVideo, Share, ShareEvent};
pub use watch::{Watch, receive};

use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// The web's presets (`SCREEN_QUALITY` in web/js/voice.js), same numbers:
/// per viewer, since the call is a mesh.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    /// 1080p, 60 fps, ~8 Mbps: games, video.
    #[default]
    Smooth,
    /// 1440p, 30 fps, ~6 Mbps: text, code.
    Sharp,
    /// Up to 4K, 60 fps, ~25 Mbps: one or two viewers.
    Max,
}

impl Quality {
    pub const ALL: [Quality; 3] = [Quality::Smooth, Quality::Sharp, Quality::Max];

    pub fn label(self) -> &'static str {
        match self {
            Quality::Smooth => "Smooth",
            Quality::Sharp => "Sharp",
            Quality::Max => "Max",
        }
    }

    /// What it's for (the share dialog, in the web's words).
    pub fn hint(self) -> &'static str {
        match self {
            Quality::Smooth => "1080p, 60 fps. Games and video. ~8 Mbps upload per person watching.",
            Quality::Sharp => "1440p, 30 fps. Text, code, slides. ~6 Mbps each.",
            Quality::Max => "Up to 4K, 60 fps. ~25 Mbps each: fast upload, one or two people watching.",
        }
    }

    pub fn max_size(self) -> (u32, u32) {
        match self {
            Quality::Smooth => (1920, 1080),
            Quality::Sharp => (2560, 1440),
            Quality::Max => (3840, 2160),
        }
    }

    pub fn fps(self) -> u32 {
        match self {
            Quality::Sharp => 30,
            Quality::Smooth | Quality::Max => 60,
        }
    }

    pub fn bitrate(self) -> u32 {
        match self {
            Quality::Smooth => 8_000_000,
            Quality::Sharp => 6_000_000,
            Quality::Max => 25_000_000,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|q| q.label().eq_ignore_ascii_case(s))
    }
}

/// Both sides of screen sharing, shared by the core, the voice engine and the UI.
pub struct Screen {
    pub share: Arc<Share>,
    pub watch: Arc<Watch>,
}

impl Screen {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { share: Share::new(), watch: Watch::new() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_match_the_web() {
        assert_eq!((Quality::Smooth.max_size(), Quality::Smooth.fps(), Quality::Smooth.bitrate()), ((1920, 1080), 60, 8_000_000));
        assert_eq!((Quality::Sharp.max_size(), Quality::Sharp.fps(), Quality::Sharp.bitrate()), ((2560, 1440), 30, 6_000_000));
        assert_eq!((Quality::Max.max_size(), Quality::Max.fps(), Quality::Max.bitrate()), ((3840, 2160), 60, 25_000_000));
        assert_eq!(Quality::parse("sharp"), Some(Quality::Sharp));
        assert_eq!(serde_json::to_string(&Quality::Max).unwrap(), r#""max""#);
    }
}
