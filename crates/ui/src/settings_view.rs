//! Settings screen. Theme choice is the only setting exposed in the window.

use gpui::{
    FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px,
};

use crate::{
    ThemeChoice,
    app::{Root, UiAction},
    components::{icon, keycap},
    theme::{PAGE_X, PAGE_Y, alpha},
};

impl Root {
    pub(crate) fn render_settings(&self) -> impl IntoElement {
        let colors = self.colors();
        let chosen = self.options.theme;

        let mut segments = div()
            .flex()
            .items_center()
            .gap(px(4.))
            .p(px(4.))
            .rounded(px(12.))
            .bg(colors.surface0);
        for (choice, label, key) in [
            (ThemeChoice::Light, "Light", "1"),
            (ThemeChoice::Dark, "Dark", "2"),
            (ThemeChoice::System, "System", "3"),
        ] {
            let selected = chosen == choice;
            let ink = if selected {
                colors.mauve
            } else {
                colors.subtext
            };
            let hover = alpha(colors.mauve, 0.12);
            let fill = if selected {
                alpha(colors.mauve, 0.22)
            } else {
                colors.surface0
            };
            segments = segments.child(
                div()
                    .id(SharedString::from(format!("theme-{choice:?}")))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .w(px(112.))
                    .h(px(44.))
                    .items_center()
                    .justify_center()
                    .rounded(px(9.))
                    .bg(fill)
                    .text_color(ink)
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .cursor_pointer()
                    .hover(move |element| element.bg(hover))
                    .on_click(self.action_click(UiAction::SetTheme(choice)))
                    .child(label)
                    .child(keycap(key, colors, false)),
            );
        }

        div()
            .id("settings-scroll")
            .track_focus(&self.settings_focus)
            .key_context("ranki_settings")
            .size_full()
            .flex()
            .flex_col()
            .gap(px(24.))
            .overflow_scroll()
            .px(px(PAGE_X))
            .py(px(PAGE_Y))
            .child(
                div()
                    .flex()
                    .items_end()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(32.))
                            .font_weight(FontWeight::BOLD)
                            .child("Settings"),
                    )
                    .child(keycap(",", colors, false)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(18.))
                    .p(px(24.))
                    .rounded(px(24.))
                    .bg(colors.mantle)
                    .border_1()
                    .border_color(colors.surface0)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .child(icon("icon-settings", 18., colors.mauve))
                            .child(
                                div()
                                    .text_size(px(18.))
                                    .font_weight(FontWeight::BOLD)
                                    .child("Appearance"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(24.))
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Theme"),
                            )
                            .child(segments),
                    ),
            )
    }
}
