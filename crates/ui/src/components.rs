//! Shared GPUI elements for the three views.

use gpui::{
    App, ClickEvent, Div, InteractiveElement, IntoElement, ParentElement, Rgba,
    SharedString, Stateful, StatefulInteractiveElement, Styled, Svg, Window,
    div, px, svg,
};

use crate::theme::{Palette, accent_ink, alpha};

pub type ClickHandler =
    Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// Lucide-style icon served by the embedded asset source.
#[must_use]
pub fn icon(name: &'static str, size: f32) -> Svg {
    svg().path(name).w(px(size)).h(px(size)).flex_none()
}

#[must_use]
pub fn primary_button(
    label: impl Into<SharedString>,
    colors: Palette,
    theme: crate::theme::Theme,
    on_click: ClickHandler,
) -> Stateful<Div> {
    div()
        .id("auto-components-1")
        .flex()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .h(px(44.))
        .px(px(16.))
        .rounded(px(12.))
        .bg(colors.mauve)
        .text_color(accent_ink(theme))
        .text_size(px(14.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .cursor_pointer()
        .on_click(on_click)
        .child(label.into())
}

#[must_use]
pub fn secondary_button(
    label: impl Into<SharedString>,
    colors: Palette,
    on_click: ClickHandler,
) -> impl IntoElement {
    div()
        .id("auto-components-2")
        .flex()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .h(px(44.))
        .px(px(16.))
        .rounded(px(12.))
        .border_1()
        .border_color(colors.surface1)
        .text_color(colors.text)
        .text_size(px(14.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .cursor_pointer()
        .on_click(on_click)
        .child(label.into())
}

#[must_use]
pub fn small_button(
    label: impl Into<SharedString>,
    colors: Palette,
    theme: crate::theme::Theme,
    on_click: ClickHandler,
) -> impl IntoElement {
    div()
        .id("auto-components-3")
        .flex()
        .items_center()
        .justify_center()
        .h(px(30.))
        .px(px(14.))
        .rounded(px(9.))
        .bg(colors.mauve)
        .text_color(accent_ink(theme))
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .cursor_pointer()
        .on_click(on_click)
        .child(label.into())
}

/// Small key badge, dark ink on mauve fills and muted ink elsewhere.
#[must_use]
pub fn keycap(
    label: impl Into<SharedString>,
    colors: Palette,
    on_accent: bool,
) -> Div {
    let ink = if on_accent {
        alpha(colors.crust, 0.18)
    } else {
        colors.subtext
    };
    div()
        .flex_none()
        .px(px(6.))
        .py(px(2.))
        .rounded(px(6.))
        .bg(if on_accent {
            alpha(colors.crust, 0.18)
        } else {
            colors.surface1
        })
        .text_color(ink)
        .text_size(px(11.))
        .font_weight(gpui::FontWeight::BOLD)
        .child(label.into())
}

#[must_use]
pub fn dot(color: Rgba, size: f32) -> Div {
    div()
        .w(px(size))
        .h(px(size))
        .rounded_full()
        .bg(color)
        .flex_none()
}
#[must_use]
pub fn progress_bar_width(fraction: f32, colors: Palette, width: f32) -> Div {
    let fill = fraction.clamp(0.0, 1.0) * width;
    div()
        .w(px(width))
        .h(px(crate::theme::PROGRESS_HEIGHT))
        .rounded(px(3.))
        .bg(colors.surface0)
        .child(
            div()
                .w(px(fill))
                .h(px(crate::theme::PROGRESS_HEIGHT))
                .rounded(px(3.))
                .bg(colors.mauve),
        )
}
