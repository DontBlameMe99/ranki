use std::{fs, path::PathBuf, process::Command};

use egui::{
    Color32, FontData, FontDefinitions, FontFamily, FontId, TextStyle, Theme,
    ThemePreference, Visuals,
};

use crate::ThemeChoice;

/// Catppuccin Mocha, used by the dark theme.
pub const MOCHA: Palette = Palette {
    base: Color32::from_rgb(0x1e, 0x1e, 0x2e),
    mantle: Color32::from_rgb(0x18, 0x18, 0x25),
    crust: Color32::from_rgb(0x11, 0x11, 0x1b),
    surface0: Color32::from_rgb(0x31, 0x32, 0x44),
    surface1: Color32::from_rgb(0x45, 0x47, 0x5a),
    text: Color32::from_rgb(0xcd, 0xd6, 0xf4),
    subtext: Color32::from_rgb(0xa6, 0xad, 0xc8),
    mauve: Color32::from_rgb(0xcb, 0xa6, 0xf7),
    red: Color32::from_rgb(0xf3, 0x8b, 0xa8),
    yellow: Color32::from_rgb(0xf9, 0xe2, 0xaf),
    green: Color32::from_rgb(0xa6, 0xe3, 0xa1),
    blue: Color32::from_rgb(0x89, 0xb4, 0xfa),
    teal: Color32::from_rgb(0x94, 0xe2, 0xd5),
    peach: Color32::from_rgb(0xfa, 0xb3, 0x87),
};

/// Catppuccin Latte, used by the light theme.
pub const LATTE: Palette = Palette {
    base: Color32::from_rgb(0xef, 0xf1, 0xf5),
    mantle: Color32::from_rgb(0xe6, 0xe9, 0xef),
    crust: Color32::from_rgb(0xdc, 0xe0, 0xe8),
    surface0: Color32::from_rgb(0xcc, 0xd0, 0xda),
    surface1: Color32::from_rgb(0xbc, 0xc0, 0xcc),
    text: Color32::from_rgb(0x4c, 0x4f, 0x69),
    subtext: Color32::from_rgb(0x6c, 0x6f, 0x85),
    mauve: Color32::from_rgb(0x88, 0x39, 0xef),
    red: Color32::from_rgb(0xd2, 0x0f, 0x39),
    yellow: Color32::from_rgb(0xdf, 0x8e, 0x1d),
    green: Color32::from_rgb(0x40, 0xa0, 0x2b),
    blue: Color32::from_rgb(0x1e, 0x66, 0xf5),
    teal: Color32::from_rgb(0x17, 0x92, 0x99),
    peach: Color32::from_rgb(0xfe, 0x64, 0x0b),
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub base: Color32,
    pub mantle: Color32,
    pub crust: Color32,
    pub surface0: Color32,
    pub surface1: Color32,
    pub text: Color32,
    pub subtext: Color32,
    pub mauve: Color32,
    pub red: Color32,
    pub yellow: Color32,
    pub green: Color32,
    pub blue: Color32,
    pub teal: Color32,
    pub peach: Color32,
}

/// Resolves the palette for a frame without querying X11 or Wayland directly.
#[must_use]
pub const fn palette(theme: Theme) -> Palette {
    match theme {
        Theme::Dark => MOCHA,
        Theme::Light => LATTE,
    }
}

/// Resolves the effective theme for a frame using egui's system-theme input.
#[must_use]
pub const fn resolve_theme(
    system: Option<Theme>,
    choice: ThemeChoice,
) -> Theme {
    match choice {
        ThemeChoice::Dark => Theme::Dark,
        ThemeChoice::Light => Theme::Light,
        ThemeChoice::System => match system {
            Some(theme) => theme,
            None => Theme::Dark,
        },
    }
}

/// Returns the candidate font families, in priority order.
pub fn font_candidates(matched: Option<&str>) -> Vec<&str> {
    let mut candidates = Vec::with_capacity(2);
    match matched {
        Some(name) if !name.trim().is_empty() => {
            let first = name.split(',').next().unwrap_or(name).trim();
            if !first.is_empty() {
                candidates.push(first);
            }
        }
        _ => {}
    }
    if !candidates
        .iter()
        .any(|name| name.eq_ignore_ascii_case("ccl nerd font"))
    {
        candidates.push("CCL Nerd Font");
    }
    candidates
}

/// Applies the palette, system font, and type scale.
pub fn apply(ctx: &egui::Context, choice: ThemeChoice, font_size: f32) {
    match choice {
        ThemeChoice::System => {
            ctx.set_theme(ThemePreference::System);
            ctx.set_visuals_of(Theme::Dark, visuals(Theme::Dark));
            ctx.set_visuals_of(Theme::Light, visuals(Theme::Light));
        }
        ThemeChoice::Dark | ThemeChoice::Light => {
            let theme = resolve_theme(None, choice);
            ctx.set_theme(theme);
            ctx.set_visuals_of(theme, visuals(theme));
        }
    }
    install_system_font(ctx);
    ctx.global_style_mut(|style| {
        style.text_styles.insert(
            TextStyle::Small,
            FontId::proportional(small_size(font_size)),
        );
        style.text_styles.insert(
            TextStyle::Body,
            FontId::proportional(body_size(font_size)),
        );
        style.text_styles.insert(
            TextStyle::Button,
            FontId::proportional(button_size(font_size)),
        );
        style.text_styles.insert(
            TextStyle::Heading,
            FontId::proportional(heading_size(font_size)),
        );
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(14.0, 9.0);
    });
}

/// Base size plus the design spec offsets, clamped to a usable range.
#[must_use]
pub const fn small_size(base: f32) -> f32 {
    (base - 4.0).clamp(8.0, 48.0)
}

#[must_use]
pub const fn body_size(base: f32) -> f32 {
    (base + 2.0).clamp(12.0, 52.0)
}

#[must_use]
pub const fn button_size(base: f32) -> f32 {
    (base - 1.0).clamp(10.0, 48.0)
}

#[must_use]
pub const fn heading_size(base: f32) -> f32 {
    (base + 5.0).clamp(16.0, 56.0)
}

#[must_use]
pub const fn question_size(base: f32) -> f32 {
    (base + 10.0).clamp(18.0, 32.0)
}

#[must_use]
pub const fn answer_size(base: f32) -> f32 {
    (base + 4.0).clamp(14.0, 24.0)
}

/// Mauve accent fill and readable ink for primary controls.
#[must_use]
pub const fn accent_fill(theme: Theme) -> Color32 {
    palette(theme).mauve
}

#[must_use]
pub const fn accent_ink(theme: Theme) -> Color32 {
    match theme {
        Theme::Dark => MOCHA.crust,
        Theme::Light => Color32::WHITE,
    }
}

/// Rating fills: Again red, Hard yellow, Good green, Easy blue.
#[must_use]
pub const fn rating_fill(code: u8, theme: Theme) -> Color32 {
    let colors = palette(theme);
    match code {
        1 => colors.red,
        2 => colors.yellow,
        3 => colors.green,
        4 => colors.blue,
        _ => colors.mauve,
    }
}

/// Dark ink on light fills, white ink on the saturated dark-theme fills.
#[must_use]
pub const fn rating_ink(code: u8, theme: Theme) -> Color32 {
    let colors = palette(theme);
    match theme {
        Theme::Dark => colors.crust,
        Theme::Light => {
            if code == 2 {
                colors.crust
            } else {
                Color32::WHITE
            }
        }
    }
}

fn visuals(theme: Theme) -> Visuals {
    let colors = palette(theme);
    let mut visuals = if theme == Theme::Dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    visuals.window_fill = colors.base;
    visuals.panel_fill = colors.mantle;
    visuals.faint_bg_color = colors.surface0;
    visuals.extreme_bg_color = colors.crust;
    visuals.override_text_color = Some(colors.text);
    visuals.weak_text_alpha = 0.82;
    visuals.hyperlink_color = colors.blue;
    visuals.selection.bg_fill = colors.mauve.gamma_multiply(0.45);
    visuals.widgets.active.bg_fill = colors.surface1;
    visuals.widgets.active.fg_stroke.color = colors.text;
    visuals.widgets.hovered.bg_fill = colors.surface0;
    visuals.widgets.hovered.fg_stroke.color = colors.text;
    visuals.widgets.inactive.bg_fill = colors.surface0;
    visuals.widgets.inactive.fg_stroke.color = colors.text;
    visuals.widgets.open.bg_fill = colors.surface1;
    visuals.widgets.open.fg_stroke.color = colors.text;
    visuals.widgets.noninteractive.bg_fill = colors.mantle;
    visuals.widgets.noninteractive.fg_stroke.color = colors.subtext;
    visuals
}

fn install_system_font(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    if let Some(path) = system_font_path()
        && let Ok(bytes) = fs::read(&path)
    {
        fonts.font_data.insert(
            String::from("system-ui"),
            FontData::from_owned(bytes).into(),
        );
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .insert(0, String::from("system-ui"));
        }
    }
    ctx.set_fonts(fonts);
}

fn system_font_path() -> Option<PathBuf> {
    let matched = fc_output(&["-f", "%{family}", "sans"]);
    font_candidates(matched.as_deref())
        .into_iter()
        .find_map(|family| {
            let path = PathBuf::from(fc_output(&["-f", "%{file}", family])?);
            path.is_file().then_some(path)
        })
}

fn fc_output(args: &[&str]) -> Option<String> {
    let output = Command::new("fc-match").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let value = text.lines().next().unwrap_or_default().trim().to_owned();
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use egui::{Color32, Theme};

    use super::{
        LATTE, MOCHA, body_size, font_candidates, palette, rating_fill,
        rating_ink, resolve_theme,
    };
    use crate::ThemeChoice;

    #[test]
    fn palettes_are_exact_catppuccin_colors() {
        assert_eq!(MOCHA.base, Color32::from_rgb(0x1e, 0x1e, 0x2e));
        assert_eq!(MOCHA.mauve, Color32::from_rgb(0xcb, 0xa6, 0xf7));
        assert_eq!(LATTE.base, Color32::from_rgb(0xef, 0xf1, 0xf5));
        assert_eq!(LATTE.mauve, Color32::from_rgb(0x88, 0x39, 0xef));
        assert_eq!(palette(Theme::Dark).text, MOCHA.text);
        assert_eq!(palette(Theme::Light).text, LATTE.text);
    }

    #[test]
    fn rating_colors_follow_the_semantic_roles() {
        assert_eq!(rating_fill(1, Theme::Dark), MOCHA.red);
        assert_eq!(rating_fill(2, Theme::Light), LATTE.yellow);
        assert_eq!(rating_fill(3, Theme::Dark), MOCHA.green);
        assert_eq!(rating_fill(4, Theme::Light), LATTE.blue);
        assert_eq!(rating_ink(2, Theme::Dark), MOCHA.crust);
        assert_eq!(rating_ink(3, Theme::Light), Color32::WHITE);
    }

    #[test]
    fn font_candidates_prefer_matched_family_then_ccl() {
        assert_eq!(
            font_candidates(Some("Inter, sans-serif")),
            vec!["Inter", "CCL Nerd Font"]
        );
        assert_eq!(font_candidates(None), vec!["CCL Nerd Font"]);
        assert_eq!(
            font_candidates(Some("CCL Nerd Font")),
            vec!["CCL Nerd Font"]
        );
    }

    #[test]
    fn typography_scales_from_the_base_size() {
        assert_eq!(body_size(16.0), 18.0);
        assert_eq!(body_size(20.0), 22.0);
    }

    #[test]
    fn explicit_theme_wins_over_system() {
        assert_eq!(
            resolve_theme(Some(Theme::Light), ThemeChoice::Dark),
            Theme::Dark
        );
        assert_eq!(
            resolve_theme(Some(Theme::Dark), ThemeChoice::Light),
            Theme::Light
        );
        assert_eq!(
            resolve_theme(Some(Theme::Light), ThemeChoice::System),
            Theme::Light
        );
        assert_eq!(resolve_theme(None, ThemeChoice::System), Theme::Dark);
    }
}
