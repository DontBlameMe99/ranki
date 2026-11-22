use egui::{Color32, FontId, TextStyle, Theme, Visuals};

/// The preference is explicit: light when set, dark otherwise.
pub const fn use_dark_theme(light_theme: bool) -> bool {
    !light_theme
}

pub fn apply(ctx: &egui::Context, light_theme: bool) {
    let theme = if use_dark_theme(light_theme) {
        Theme::Dark
    } else {
        Theme::Light
    };
    ctx.set_theme(theme);
    ctx.set_visuals_of(theme, visuals(theme));
    ctx.global_style_mut(|style| {
        style
            .text_styles
            .insert(TextStyle::Heading, FontId::proportional(24.0));
        style
            .text_styles
            .insert(TextStyle::Body, FontId::proportional(16.0));
        style
            .text_styles
            .insert(TextStyle::Button, FontId::proportional(15.0));
        style.spacing.item_spacing = egui::vec2(10.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 7.0);
    });
}

fn visuals(theme: Theme) -> Visuals {
    let dark = theme == Theme::Dark;
    let mut visuals = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    visuals.window_fill = if dark {
        Color32::from_rgb(18, 21, 28)
    } else {
        Color32::from_rgb(250, 251, 253)
    };
    visuals.panel_fill = visuals.window_fill;
    visuals.faint_bg_color = if dark {
        Color32::from_rgb(30, 35, 45)
    } else {
        Color32::from_rgb(239, 242, 247)
    };
    visuals.extreme_bg_color = if dark {
        Color32::from_rgb(12, 15, 20)
    } else {
        Color32::from_rgb(255, 255, 255)
    };
    visuals.override_text_color = Some(if dark {
        Color32::from_rgb(238, 242, 248)
    } else {
        Color32::from_rgb(25, 31, 41)
    });
    visuals.weak_text_alpha = 0.78;
    visuals
}

#[cfg(test)]
mod tests {
    use super::use_dark_theme;

    #[test]
    fn theme_choice_follows_the_preference() {
        assert!(use_dark_theme(false));
        assert!(!use_dark_theme(true));
    }
}
