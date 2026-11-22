//! Wayland-native Ranki UI built with GPUI.
//!
//! The binary supplies the shared settings and a store. This crate owns the
//! window, its view state, and the card renderer.
#![expect(
    clippy::absolute_paths,
    clippy::arithmetic_side_effects,
    clippy::cast_precision_loss,
    clippy::clone_on_ref_ptr,
    clippy::default_trait_access,
    clippy::field_reassign_with_default,
    clippy::inconsistent_struct_constructor,
    clippy::indexing_slicing,
    clippy::integer_division,
    clippy::items_after_statements,
    clippy::map_unwrap_or,
    clippy::match_same_arms,
    clippy::missing_const_for_fn,
    clippy::needless_pass_by_ref_mut,
    clippy::needless_pass_by_value,
    clippy::option_if_let_else,
    clippy::redundant_closure,
    clippy::redundant_pub_crate,
    clippy::semicolon_if_nothing_returned,
    clippy::shadow_reuse,
    clippy::similar_names,
    clippy::too_many_lines,
    clippy::type_complexity,
    clippy::uninlined_format_args,
    clippy::unreadable_literal,
    clippy::struct_excessive_bools,
    clippy::use_self,
    clippy::excessive_nesting,
    reason = "GPUI view builders and pure helper models favor direct element trees and index math over extra abstraction"
)]

mod add_card_view;
mod app;
mod assets;
mod card_view;
mod components;
mod decks_view;
mod nav;
mod review_view;
mod text_field;
mod theme;

use gpui::{
    App, AppContext, Application, Bounds, WindowBounds, WindowOptions, px, size,
};
use ranki_model::Store;

pub use ranki_model::{SchedulerKind, Settings, ThemeChoice};

/// Settings for the review window, shared with the command line.
pub type Options = Settings;

/// Opens the native window and runs the GPUI event loop.
pub fn run(store: Box<dyn Store>, options: Options) {
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
                        let root = app::Root::new(store, options, window, cx);
                        root.observe_appearance(window, cx);
                        root
                    })
                },
            );
            if let Err(error) = window {
                eprintln!("ranki: cannot open review window: {error}");
                return;
            }
            app.activate(true);
        });
}
