//! Wayland-native review UI for Ranki.
//!
//! The binary supplies the shared settings. The review window owns only
//! session state.

pub(crate) mod app;
pub(crate) mod card_view;
pub(crate) mod theme;

pub use app::{BenchApp, run};
pub use ranki_model::{SchedulerKind, Settings, ThemeChoice};

/// Settings for the review window, shared with the command line.
pub type Options = Settings;
