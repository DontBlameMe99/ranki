//! Settings screen for theme and optional `AnkiWeb` sync.

use gpui::{
    FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px,
};

use crate::{
    ThemeChoice,
    app::{Root, UiAction},
    components::{Interaction, icon, panel},
    theme::{
        PAGE_X, PAGE_Y, SPACE_INLINE, SPACE_RELATED, SPACE_SECTION, alpha,
    },
};

impl Root {
    pub(crate) fn render_settings(&self) -> impl IntoElement {
        let colors = self.colors();
        let chosen = self.options.theme;
        let theme_text_size = px(13.);

        let mut segments = div()
            .flex()
            .flex_wrap()
            .items_center()
            .min_w(px(0.))
            .gap(px(SPACE_INLINE))
            .p(px(SPACE_INLINE))
            .rounded(px(12.))
            .bg(colors.surface0);
        for (choice, label) in [
            (ThemeChoice::Light, "Light"),
            (ThemeChoice::Dark, "Dark"),
            (ThemeChoice::System, "System"),
        ] {
            let selected = chosen == choice;
            let ink = if selected {
                colors.mauve
            } else {
                colors.subtext
            };
            let (hover, press) = if selected {
                (alpha(colors.mauve, 0.32), alpha(colors.mauve, 0.42))
            } else {
                (alpha(colors.mauve, 0.12), alpha(colors.mauve, 0.28))
            };
            let fill = if selected {
                alpha(colors.mauve, 0.22)
            } else {
                colors.surface0
            };
            segments = segments.child(
                Interaction::new(
                    if selected {
                        colors.mauve
                    } else {
                        alpha(colors.text, 0.0)
                    },
                    hover,
                    press,
                    if selected { colors.text } else { colors.mauve },
                )
                .apply(
                    div()
                        .id(SharedString::from(format!("theme-{choice:?}")))
                        .flex()
                        .flex_col()
                        .gap(px(SPACE_INLINE))
                        .flex_1()
                        .min_w(px(80.))
                        .h(px(44.))
                        .items_center()
                        .justify_center()
                        .rounded(px(9.))
                        .bg(fill)
                        .text_color(ink)
                        .text_size(theme_text_size)
                        .font_weight(FontWeight::SEMIBOLD)
                        .cursor_pointer(),
                )
                .on_click(self.action_click(UiAction::SetTheme(choice)))
                .child(label),
            );
        }

        div()
            .id("settings-scroll")
            .track_focus(&self.settings_focus)
            .key_context("ranki_settings")
            .size_full()
            .flex()
            .flex_col()
            .gap(px(SPACE_SECTION))
            .overflow_scroll()
            .track_scroll(&self.settings_scroll)
            .px(px(PAGE_X))
            .py(px(PAGE_Y))
            .child(
                div()
                    .text_size(px(32.))
                    .font_weight(FontWeight::BOLD)
                    .child("Settings"),
            )
            .child(
                panel(colors)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(SPACE_RELATED))
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
                            .flex_wrap()
                            .items_center()
                            .justify_between()
                            .gap(px(SPACE_SECTION))
                            .child(
                                div()
                                    .text_size(theme_text_size)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Theme"),
                            )
                            .child(segments),
                    ),
            )
            .child(self.render_sync())
    }
}
