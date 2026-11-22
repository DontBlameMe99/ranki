//! Wayland-native review UI for Ranki.

use std::path::PathBuf;

pub(crate) mod app;
pub(crate) mod card_view;
pub(crate) mod theme;

pub use app::run;

/// Settings for the review window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    pub window_title: String,
    pub media_dir: Option<PathBuf>,
    pub light_theme: bool,
}
