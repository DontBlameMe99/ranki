//! Wayland-native review UI for Ranki.
//!
//! The binary supplies the shared settings. The review window owns only
//! session state.

use gpui::{
    App, AppContext as _, Application, Bounds, Context, IntoElement,
    ParentElement, Render, Styled, Window, WindowBounds, WindowOptions, div,
    px, rgb, size,
};
use ranki_model::Store;

pub use ranki_model::{SchedulerKind, Settings, ThemeChoice};

/// Settings for the review window, shared with the command line.
pub type Options = Settings;

const MOCHA_BASE: u32 = 0x001e_1e2e;
const MOCHA_TEXT: u32 = 0x00cd_d6f4;

struct Root {
    #[expect(dead_code, reason = "the next UI implementation owns the store")]
    store: Box<dyn Store>,
    options: Options,
}

impl Render for Root {
    fn render(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(MOCHA_BASE))
            .text_color(rgb(MOCHA_TEXT))
            .text_size(px(self.options.font_size * 2.0))
            .child("Ranki")
    }
}

/// Open the native review window and run the GPUI event loop.
pub fn run(store: Box<dyn Store>, options: Options) {
    Application::new().run(move |app: &mut App| {
        let bounds = Bounds::centered(None, size(px(960.), px(640.)), app);
        let window = app.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(640.), px(480.))),
                app_id: Some("ranki".into()),
                ..Default::default()
            },
            |_window, cx| cx.new(|_cx| Root { store, options }),
        );
        if let Err(error) = window {
            eprintln!("ranki: cannot open review window: {error}");
            return;
        }
        app.activate(true);
    });
}
