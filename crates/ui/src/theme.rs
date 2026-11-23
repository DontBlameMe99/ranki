//! Catppuccin palette, layout tokens, and semantic colors for the GPUI UI.

use gpui::Rgba;
use ranki_model::ThemeChoice;

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
/// Palette plus resolved theme, passed by value to shared element helpers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub colors: Palette,
    pub theme: Theme,
}

impl Style {
    #[must_use]
    pub const fn new(colors: Palette, theme: Theme) -> Self {
        Self { colors, theme }
    }

    /// Ink used on mauve fills.
    #[must_use]
    pub const fn ink(self) -> Rgba {
        accent_ink(self.theme)
    }
}

/// Field placeholder ink. Latte `subtext` reaches only 3.2:1 on a `surface0`
/// input, so light placeholders use a darker ink that clears 4.5:1 on both
/// input roles.
#[must_use]
pub const fn placeholder_color(theme: Theme) -> Rgba {
    match theme {
        Theme::Dark => MOCHA.subtext,
        Theme::Light => c(0x545869),
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

/// Text-safe queue color. Latte green fails 4.5:1 on raised surfaces.
#[must_use]
pub const fn queue_text(code: u8, theme: Theme) -> Rgba {
    match (theme, code) {
        (Theme::Light, 2) => LATTE.text,
        _ => queue_color(code, theme),
    }
}

pub const FONT_FAMILY: &str = "Liberation Sans";
pub const CJK_FAMILY: &str = "Noto Sans CJK JP";
/// Last-resort family that still covers Latin, kana, and Nerd icons.
pub const FONT_FALLBACK: &str = "CCL Nerd Font";

/// Picks the first installed family from the mockup and Ranki fallbacks.
#[must_use]
pub fn detected_font_family() -> &'static str {
    font_candidates(None)
        .into_iter()
        .find(|family| family_installed(family))
        .unwrap_or(FONT_FALLBACK)
}

/// Resolved UI family, cached after the first fontconfig probe.
pub fn ui_font_family() -> &'static str {
    static UI_FONT: std::sync::OnceLock<&'static str> =
        std::sync::OnceLock::new();
    UI_FONT.get_or_init(detected_font_family)
}

/// Japanese family for glyph runs that need a real CJK face.
pub fn cjk_font_family() -> &'static str {
    static CJK_FONT: std::sync::OnceLock<&'static str> =
        std::sync::OnceLock::new();
    CJK_FONT.get_or_init(|| {
        [CJK_FAMILY, "Noto Sans JP"]
            .into_iter()
            .find(|family| family_installed(family))
            .unwrap_or_else(ui_font_family)
    })
}

/// True when the text needs the CJK face. Unifont is the wrong fallback.
#[must_use]
pub fn contains_cjk(text: &str) -> bool {
    text.chars().any(|character| {
        matches!(
            u32::from(character),
            0x3040..=0x30ff
                | 0x3400..=0x4dbf
                | 0x4e00..=0x9fff
                | 0xf900..=0xfaff
                | 0xff66..=0xff9f
        )
    })
}

/// Family for one text block. Mixed strings favor the CJK face.
#[must_use]
pub fn font_for_text(text: &str) -> &'static str {
    if contains_cjk(text) {
        cjk_font_family()
    } else {
        ui_font_family()
    }
}

fn family_installed(family: &str) -> bool {
    let Ok(output) = std::process::Command::new("fc-match")
        .args(["-f", "%{family}", family])
        .output()
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    String::from_utf8_lossy(&output.stdout)
        .split(',')
        .any(|matched| matched.trim().eq_ignore_ascii_case(family))
}

/// Font candidates: clean sans and real CJK first, CCL Nerd Font last.
#[must_use]
pub fn font_candidates(matched: Option<&str>) -> Vec<&str> {
    let mut candidates =
        vec![FONT_FAMILY, "Inter", "Noto Sans JP", "DejaVu Sans"];
    if let Some(name) = matched {
        let first = name.split(',').next().unwrap_or(name).trim();
        if !first.is_empty()
            && !candidates
                .iter()
                .any(|family| family.eq_ignore_ascii_case(first))
        {
            candidates.push(first);
        }
    }
    candidates.push(FONT_FALLBACK);
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
/// Fixed width of one queue-count column in the deck table.
pub const COUNT_COLUMN_WIDTH: f32 = 72.0;
/// Reserved width of the deck table action column, so rows never shift.
pub const ACTION_COLUMN_WIDTH: f32 = 120.0;
/// Deck table widths: deck name flexes, three counts, then the action cell.
pub const TABLE_COLUMNS: [f32; 5] = [
    1.0,
    COUNT_COLUMN_WIDTH,
    COUNT_COLUMN_WIDTH,
    COUNT_COLUMN_WIDTH,
    ACTION_COLUMN_WIDTH,
];
/// Form column cap for the add-card screen.
pub const ADD_FORM_WIDTH: f32 = 720.0;
/// Preview rail width on the add-card screen.
pub const PREVIEW_WIDTH: f32 = 380.0;
/// Gap between the add-card form and its preview rail.
pub const ADD_GAP: f32 = 32.0;
/// Full add-card content width: form plus gap plus preview rail.
pub const ADD_CONTENT_WIDTH: f32 = ADD_FORM_WIDTH + ADD_GAP + PREVIEW_WIDTH;

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

#[cfg(test)]
mod tests {
    use gpui::rgb;

    use super::{
        LATTE, MOCHA, PROGRESS_HEIGHT, PROGRESS_WIDTH, STAGE_HEIGHT,
        STAGE_RADIUS, STAGE_WIDTH, Theme, alpha, font_candidates,
        interval_label, palette, placeholder_color, rating_fill, rating_label,
        resolve_theme,
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
    fn fonts_put_clean_sans_first_and_keep_ccl_fallback() {
        let candidates = font_candidates(Some("Inter, sans-serif"));
        assert_eq!(candidates[0], "Liberation Sans");
        assert_eq!(candidates.last(), Some(&"CCL Nerd Font"));
        assert!(candidates.contains(&"Noto Sans JP"));
        assert_eq!(font_candidates(None).last(), Some(&"CCL Nerd Font"));
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
    /// WCAG 2.1 relative luminance ratio for two opaque roles.
    fn contrast(a: gpui::Rgba, b: gpui::Rgba) -> f32 {
        let lum = |color: gpui::Rgba| {
            let channel = |value: f32| {
                if value <= 0.040_45 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            };
            0.212_6 * channel(color.r)
                + 0.715_2 * channel(color.g)
                + 0.072_2 * channel(color.b)
        };
        let (high, low) = {
            let (a, b) = (lum(a), lum(b));
            if a >= b { (a, b) } else { (b, a) }
        };
        (high + 0.05) / (low + 0.05)
    }

    #[test]
    fn placeholders_clear_4_5_to_1_on_both_input_roles() {
        for theme in [Theme::Dark, Theme::Light] {
            let ink = placeholder_color(theme);
            let input = palette(theme).base;
            let raised = palette(theme).surface0;
            assert!(contrast(ink, input) >= 4.5, "{theme:?} on base");
            assert!(contrast(ink, raised) >= 4.5, "{theme:?} on surface0");
        }
    }
}
