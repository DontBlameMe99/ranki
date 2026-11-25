//! Wayland-native Ranki UI built with GPUI.
//!
//! The binary supplies the shared settings and a store. This crate owns the
//! window, its view state, and the card renderer.
#![expect(
    clippy::absolute_paths,
    clippy::arithmetic_side_effects,
    clippy::cast_precision_loss,
    clippy::clone_on_ref_ptr,
    clippy::inconsistent_struct_constructor,
    clippy::indexing_slicing,
    clippy::integer_division,
    clippy::match_same_arms,
    clippy::missing_const_for_fn,
    clippy::needless_pass_by_ref_mut,
    clippy::needless_pass_by_value,
    clippy::option_if_let_else,
    clippy::shadow_reuse,
    clippy::too_many_lines,
    clippy::type_complexity,
    clippy::unreadable_literal,
    clippy::struct_excessive_bools,
    clippy::too_many_arguments,
    clippy::excessive_nesting,
    reason = "GPUI view builders and pure helper models favor direct element trees and index math over extra abstraction"
)]

mod add_card_view;
mod app;
mod assets;
mod bench;
mod browser_view;
mod card_view;
mod components;
mod decks_view;
mod nav;
pub mod occlusion;
mod review_view;
mod settings_view;
mod statistics_view;
mod sync_view;
mod text_field;
mod theme;

use gpui::{
    App, AppContext, Application, Bounds, WindowBounds, WindowOptions, px, size,
};
use ranki_model::Store;
use std::sync::Arc;

pub use bench::{BenchApp, BenchKey};
pub use ranki_model::{SchedulerKind, Settings, ThemeChoice};

/// Writes settings changed in the window back to the shared config file.
pub type SettingsSaver =
    Arc<dyn Fn(&Settings) -> Result<(), String> + Send + Sync>;

/// Settings for the review window, shared with the command line.
pub type Options = Settings;

/// Opens the native window and runs the GPUI event loop.
pub fn run(
    store: Box<dyn Store>,
    options: Options,
    save_settings: SettingsSaver,
) {
    let media_dir = options.media_dir.clone();
    Application::new()
        .with_assets(assets::Assets { media_dir })
        .run(move |app: &mut App| {
            app.bind_keys(app::key_bindings());
            let bounds = Bounds::centered(None, size(px(1280.), px(800.)), app);
            let window = app.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(900.), px(600.))),
                    app_id: Some("ranki".into()),
                    ..Default::default()
                },
                |window, cx| {
                    cx.new(|cx| {
                        let root = app::Root::new(
                            app::Launch {
                                store,
                                options,
                                save_settings,
                            },
                            window,
                            cx,
                        );
                        app::Root::observe_appearance(window, cx);
                        root
                    })
                },
            );
            if let Err(error) = window {
                eprintln!("ranki: cannot open review window: {error}");
                app.quit();
                return;
            }
            app.activate(true);
        });
}
