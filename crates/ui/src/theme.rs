//! Catppuccin palette, layout tokens, and semantic colors for the GPUI UI.

use gpui::Rgba;
use ranki_model::{SchedulerKind, ThemeChoice};

/// Dark Catppuccin Mocha, the palette used by the mockups.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    Dark,
    Light,
}

/// Resolves the configured choice against the current window appearance.
#[must_use]
pub const fn resolve_theme(system_dark: bool, choice: ThemeChoice) -> Theme {
    match choice {
        ThemeChoice::Dark => Theme::Dark,
        ThemeChoice::Light => Theme::Light,
        ThemeChoice::System => {
            if system_dark {
                Theme::Dark
            } else {
                Theme::Light
            }
        }
    }
}

/// A flat Catppuccin role set. Every view reads colors from here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub base: Rgba,
    pub mantle: Rgba,
    pub crust: Rgba,
    pub surface0: Rgba,
    pub surface1: Rgba,
    pub text: Rgba,
    pub subtext: Rgba,
    pub overlay: Rgba,
    pub mauve: Rgba,
    pub red: Rgba,
    pub peach: Rgba,
    pub green: Rgba,
    pub blue: Rgba,
    pub teal: Rgba,
}

const fn c(hex: u32) -> Rgba {
    let [_, r, g, b] = hex.to_be_bytes();
    Rgba {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: 1.0,
    }
}

pub const MOCHA: Palette = Palette {
    base: c(0x1e1e2e),
    mantle: c(0x181825),
    crust: c(0x11111b),
    surface0: c(0x313244),
    surface1: c(0x45475a),
    text: c(0xcdd6f4),
    subtext: c(0xa6adc8),
    overlay: c(0x7f849c),
    mauve: c(0xcba6f7),
    red: c(0xf38ba8),
    peach: c(0xfab387),
    green: c(0xa6e3a1),
    blue: c(0x89b4fa),
    teal: c(0x94e2d5),
};

pub const LATTE: Palette = Palette {
    base: c(0xeff1f5),
    mantle: c(0xe6e9ef),
    crust: c(0xdce0e8),
    surface0: c(0xccd0da),
    surface1: c(0xbcc0cc),
    text: c(0x4c4f69),
    subtext: c(0x6c6f85),
    overlay: c(0x8c8fa1),
    mauve: c(0x8839ef),
    red: c(0xd20f39),
    peach: c(0xfe640b),
    green: c(0x40a02b),
    blue: c(0x1e66f5),
    teal: c(0x179299),
};

#[must_use]
pub const fn palette(theme: Theme) -> Palette {
    match theme {
        Theme::Dark => MOCHA,
        Theme::Light => LATTE,
    }
}

/// Applies alpha to a role color without introducing a new hue.
#[must_use]
pub const fn alpha(color: Rgba, amount: f32) -> Rgba {
    let mut result = color;
    result.a = amount;
    result
}

#[must_use]
pub const fn accent_ink(theme: Theme) -> Rgba {
    match theme {
        Theme::Dark => MOCHA.crust,
        Theme::Light => c(0xffffff),
    }
}

/// Rating hue: Again red, Hard peach, Good green, Easy blue.
#[must_use]
pub const fn rating_fill(code: u8, theme: Theme) -> Rgba {
    let colors = palette(theme);
    match code {
        1 => colors.red,
        2 => colors.peach,
        3 => colors.green,
        4 => colors.blue,
        _ => colors.mauve,
    }
}

/// Solid label color. Latte keeps labels readable instead of tinting them.
#[must_use]
pub const fn rating_label(code: u8, theme: Theme) -> Rgba {
    match theme {
        Theme::Dark => rating_fill(code, theme),
        Theme::Light => LATTE.text,
    }
}

#[must_use]
pub const fn queue_color(code: u8, theme: Theme) -> Rgba {
    let colors = palette(theme);
    match code {
        0 => colors.blue,
        1 => colors.red,
        2 => colors.green,
        _ => colors.mauve,
    }
}

pub const FONT_FAMILY: &str = "CCL Nerd Font";

/// Picks the first installed family from the mockup and Ranki fallbacks.
#[must_use]
pub fn detected_font_family() -> &'static str {
    let candidates = font_candidates(None);
    for candidate in candidates {
        if let Ok(output) = std::process::Command::new("fc-match")
            .args(["-f", "%{family}", candidate])
            .output()
            && output.status.success()
        {
            let matched = String::from_utf8_lossy(&output.stdout);
            let family = matched.lines().next().unwrap_or("").trim();
            if !family.is_empty() && !family.eq_ignore_ascii_case(candidate) {
                continue;
            }
            if candidate.eq_ignore_ascii_case(FONT_FAMILY)
                || family.eq_ignore_ascii_case(candidate)
            {
                return if candidate == "Inter" {
                    "Inter"
                } else {
                    FONT_FAMILY
                };
            }
        }
    }
    FONT_FAMILY
}

/// Font candidates, with CCL Nerd Font first and a CJK fallback last.
#[must_use]
pub fn font_candidates(matched: Option<&str>) -> Vec<&str> {
    let mut candidates = vec![FONT_FAMILY];
    if let Some(name) = matched
        && !name.trim().is_empty()
    {
        let first = name.split(',').next().unwrap_or(name).trim();
        if !first.is_empty() && !first.eq_ignore_ascii_case(FONT_FAMILY) {
            candidates.push(first);
        }
    }
    if !candidates
        .iter()
        .any(|name| name.eq_ignore_ascii_case("noto sans jp"))
    {
        candidates.push("Noto Sans JP");
    }
    candidates
}

pub const NAV_WIDTH: f32 = 232.0;
pub const TOPBAR_HEIGHT: f32 = 72.0;
pub const FOOTER_HEIGHT: f32 = 64.0;
pub const STAGE_WIDTH: f32 = 760.0;
pub const STAGE_HEIGHT: f32 = 440.0;
pub const STAGE_RADIUS: f32 = 32.0;
pub const STAGE_PADDING: f32 = 28.0;
pub const CONTROL_HEIGHT: f32 = 76.0;
pub const RATING_GAP: f32 = 14.0;
pub const PROGRESS_WIDTH: f32 = 420.0;
pub const PROGRESS_HEIGHT: f32 = 6.0;
pub const PAGE_X: f32 = 40.0;
pub const PAGE_Y: f32 = 32.0;
pub const TABLE_COLUMNS: [f32; 5] = [1.0, 72.0, 72.0, 72.0, 108.0];

/// Converts an interval in days or negative seconds into an Anki-style label.
#[must_use]
pub fn interval_label(interval: i32) -> String {
    if interval < 0 {
        let seconds = interval.unsigned_abs();
        if seconds < 60 {
            format!("{seconds}s")
        } else {
            format!("{}m", seconds / 60)
        }
    } else if interval < 30 {
        format!("{interval}d")
    } else if interval < 365 {
        format!("{}mo", interval / 30)
    } else {
        format!("{}y", interval / 365)
    }
}

/// Short label used by the scheduler controls.
#[must_use]
pub const fn scheduler_label(kind: SchedulerKind) -> &'static str {
    kind.label()
}

#[cfg(test)]
mod tests {
    use gpui::rgb;

    use super::{
        LATTE, MOCHA, PROGRESS_HEIGHT, PROGRESS_WIDTH, STAGE_HEIGHT,
        STAGE_RADIUS, STAGE_WIDTH, Theme, alpha, font_candidates,
        interval_label, palette, rating_fill, rating_label, resolve_theme,
    };
    use crate::ThemeChoice;

    #[test]
    fn mocha_matches_the_mockup_exactly() {
        assert_eq!(MOCHA.base, rgb(0x1e1e2e));
        assert_eq!(MOCHA.surface0, rgb(0x313244));
        assert_eq!(MOCHA.mauve, rgb(0xcba6f7));
        assert_eq!(palette(Theme::Dark), MOCHA);
    }

    #[test]
    fn latte_is_the_light_mapping() {
        assert_eq!(LATTE.base, rgb(0xeff1f5));
        assert_eq!(LATTE.mauve, rgb(0x8839ef));
        assert_eq!(palette(Theme::Light), LATTE);
    }

    #[test]
    fn explicit_theme_beats_system_appearance() {
        assert_eq!(resolve_theme(false, ThemeChoice::Dark), Theme::Dark);
        assert_eq!(resolve_theme(true, ThemeChoice::Light), Theme::Light);
        assert_eq!(resolve_theme(true, ThemeChoice::System), Theme::Dark);
        assert_eq!(resolve_theme(false, ThemeChoice::System), Theme::Light);
    }

    #[test]
    fn rating_colors_match_the_learning_mockup() {
        assert_eq!(rating_fill(1, Theme::Dark), MOCHA.red);
        assert_eq!(rating_fill(2, Theme::Dark), MOCHA.peach);
        assert_eq!(rating_fill(3, Theme::Light), LATTE.green);
        assert_eq!(rating_fill(4, Theme::Light), LATTE.blue);
    }

    #[test]
    fn latte_rating_labels_use_text_contrast() {
        for code in 1..=4 {
            assert_eq!(rating_label(code, Theme::Light), LATTE.text);
            assert_eq!(
                rating_label(code, Theme::Dark),
                rating_fill(code, Theme::Dark)
            );
        }
    }

    #[test]
    fn alpha_only_changes_opacity() {
        let faded = alpha(MOCHA.mauve, 0.16);
        assert_eq!(faded.r, MOCHA.mauve.r);
        assert_eq!(faded.a, 0.16);
    }

    #[test]
    fn fonts_put_ccl_first_and_keep_cjk_fallback() {
        assert_eq!(
            font_candidates(Some("Inter, sans-serif")),
            vec!["CCL Nerd Font", "Inter", "Noto Sans JP"]
        );
        assert_eq!(font_candidates(None)[0], "CCL Nerd Font");
    }

    #[test]
    fn layout_constants_match_the_artboard() {
        assert_eq!(STAGE_WIDTH, 760.0);
        assert_eq!(STAGE_HEIGHT, 440.0);
        assert_eq!(STAGE_RADIUS, 32.0);
        assert_eq!(PROGRESS_WIDTH, 420.0);
        assert_eq!(PROGRESS_HEIGHT, 6.0);
    }

    #[test]
    fn interval_labels_cover_seconds_days_and_years() {
        assert_eq!(interval_label(-10), "10s");
        assert_eq!(interval_label(-600), "10m");
        assert_eq!(interval_label(6), "6d");
        assert_eq!(interval_label(60), "2mo");
        assert_eq!(interval_label(365), "1y");
    }
}
