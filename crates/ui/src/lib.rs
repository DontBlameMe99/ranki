//! Wayland-native review UI for Ranki.
//!
//! The binary supplies the paths, theme, and scheduler settings. The review
//! window owns only session state.

use std::path::PathBuf;

pub(crate) mod app;
pub(crate) mod card_view;
pub(crate) mod theme;

pub use app::run;

/// Which visual theme the review window should use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeChoice {
    Dark,
    Light,
    #[default]
    System,
}

impl ThemeChoice {
    /// Keeps the older boolean preference readable at call sites.
    #[must_use]
    pub const fn from_light_theme(light: bool) -> Self {
        if light { Self::Light } else { Self::Dark }
    }
}

/// Scheduler available for a review session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SchedulerKind {
    #[default]
    Fsrs,
    Sm2,
}

impl SchedulerKind {
    /// The other scheduler, used by the session choice shortcut.
    #[must_use]
    pub const fn other(self) -> Self {
        match self {
            Self::Fsrs => Self::Sm2,
            Self::Sm2 => Self::Fsrs,
        }
    }

    /// Short name shown in the sidebar and review header.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Fsrs => "FSRS",
            Self::Sm2 => "SM-2",
        }
    }
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "the older boolean theme flag is kept for compatibility"
)]
/// Settings for the review window.
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub window_title: String,
    pub media_dir: Option<PathBuf>,
    pub light_theme: bool,
    pub theme: ThemeChoice,
    pub font_size: f32,
    pub review_batch_size: usize,
    pub show_keyboard_hints: bool,
    pub scheduler: SchedulerKind,
    pub desired_retention: f32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            window_title: String::from("Ranki"),
            media_dir: None,
            light_theme: false,
            theme: ThemeChoice::System,
            font_size: 16.0,
            review_batch_size: 20,
            show_keyboard_hints: true,
            scheduler: SchedulerKind::Fsrs,
            desired_retention: 0.9,
        }
    }
}
