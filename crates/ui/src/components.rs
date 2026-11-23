//! Shared GPUI elements for the three views.

use gpui::{
    App, ClickEvent, Div, InteractiveElement, IntoElement, ParentElement, Rgba,
    SharedString, StatefulInteractiveElement, Styled, Svg, Window, div, px,
    svg,
};

use crate::theme::{Palette, Style, alpha};

pub type ClickHandler =
    Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// Lucide-style icon served by the embedded asset source.
///
/// GPUI paints an SVG mask in the element's own text color, so every call
/// must name its tint.
#[must_use]
pub fn icon(name: &'static str, size: f32, color: Rgba) -> Svg {
    svg()
        .path(name)
        .w(px(size))
        .h(px(size))
        .flex_none()
        .text_color(color)
}

/// 36x36 tinted tile holding a hue-colored icon.
#[must_use]
pub fn icon_tile(name: &'static str, hue: Rgba) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .w(px(36.))
        .h(px(36.))
        .flex_none()
        .rounded(px(10.))
        .bg(alpha(hue, 0.16))
        .child(icon(name, 18., hue))
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
    style: Style,
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
        .bg(style.colors.mauve)
        .text_color(style.ink())
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .cursor_pointer()
        .on_click(on_click)
        .child(label.into())
}

/// Small key badge. Mauve fills use crust ink on an 18% crust notch.
#[must_use]
pub fn keycap(
    label: impl Into<SharedString>,
    colors: Palette,
    on_accent: bool,
) -> Div {
    let (ink, fill) = if on_accent {
        (colors.crust, alpha(colors.crust, 0.18))
    } else {
        (colors.subtext, colors.surface1)
    };
    div()
        .flex_none()
        .px(px(6.))
        .py(px(2.))
        .rounded(px(6.))
        .bg(fill)
        .text_color(ink)
        .text_size(px(11.))
        .font_weight(gpui::FontWeight::BOLD)
        .child(label.into())
}

/// Rounded tag chip from a note tag.
#[must_use]
pub fn chip(label: impl Into<SharedString>, colors: Palette) -> Div {
    div()
        .flex_none()
        .px(px(10.))
        .py(px(5.))
        .rounded(px(10.))
        .bg(colors.surface1)
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .child(label.into())
}

/// Editable tag chip: the label plus a close button that owns the removal.
#[must_use]
pub fn removable_chip(
    label: impl Into<SharedString>,
    colors: Palette,
    on_remove: ClickHandler,
) -> impl IntoElement {
    div()
        .id("tag-chip")
        .flex_none()
        .flex()
        .items_center()
        .gap(px(6.))
        .py(px(7.))
        .pr(px(8.))
        .pl(px(12.))
        .rounded(px(12.))
        .bg(colors.surface0)
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .child(label.into())
        .child(
            div()
                .id("tag-chip-remove")
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .on_click(on_remove)
                .child(icon("icon-x", 14., colors.overlay)),
        )
}

/// Sidebar brand mark. The wordmark SVG is illegible at 32 px, so the tile
/// carries a bold monogram instead.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrandMark {
    pub label: &'static str,
    pub tile: f32,
    pub glyph: f32,
}

#[must_use]
pub const fn brand_mark() -> BrandMark {
    BrandMark {
        label: "R",
        tile: 32.0,
        glyph: 20.0,
    }
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

#[cfg(test)]
mod tests {
    use super::brand_mark;

    #[test]
    fn brand_mark_fills_the_tile_at_32_px() {
        let mark = brand_mark();
        assert_eq!(mark.label, "R");
        assert_eq!(mark.tile, 32.0);
        assert!(mark.glyph >= mark.tile * 0.55);
        assert!(mark.glyph <= mark.tile * 0.75);
    }
}
