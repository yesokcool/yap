//! The v0.36 extras' UI (PROTOCOL.md › Extras):
//! polls, server-rolled dice, "▶ Watch together" under YouTube links, away
//! auto-replies, the "New since …" divider and its jump pill, Catch up,
//! the quick switcher, the soundboard, hangs and their settings.
//!
//! Every piece draws through `theme::*`.

use super::{Act, App, dialogs, icons, theme, theme::Bevel, theme::Pal};
use crate::core::{Cmd, Model};
use crate::protocol::{Message, Poll, Roll};
use eframe::egui::{self, Align2, Color32, Rect, Sense, Stroke, pos2, vec2};

mod poll;
mod unread;
mod doodles;
mod hangs;
mod sounds;
mod settings_groups;
mod switcher;

pub use poll::*;
pub use unread::*;
pub use doodles::*;
pub use hangs::*;
pub use sounds::*;
pub use settings_groups::*;
pub use switcher::*;
