//! Touch-first egui layout for the mobile app.

#![expect(
    clippy::arithmetic_side_effects,
    clippy::cast_precision_loss,
    clippy::excessive_nesting,
    clippy::shadow_reuse,
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "egui widget nesting and bounded layout math keep screen order obvious"
)]

use egui::{
    Align, Button, Color32, ComboBox, CornerRadius, FontId, Frame as EguiFrame,
    Layout, Margin, ProgressBar, RichText, ScrollArea, Stroke, TextEdit,
    TextStyle, Ui, vec2,
};
use ranki_model::{Card, CardType, Rating, ThemeChoice};
use std::time::Duration;

use crate::mobile::card::{render_side, sides};
use crate::mobile::state::{AppState, DeckRow, ReviewPhase, Screen, ToastKind};

const PAGE_MARGIN: i8 = 16;
const TAB_HEIGHT: f32 = 56.0;
const TAP_HEIGHT: f32 = 48.0;
const RATING_HEIGHT: f32 = 60.0;

/// egui app that owns the mobile state.
pub struct MobileApp {
    pub state: AppState,
}

impl MobileApp {
    #[must_use]
    pub const fn new(state: AppState) -> Self {
        Self { state }
    }
}

impl eframe::App for MobileApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        apply_style(ctx, self.state.settings.theme);
        if self.state.review.phase == ReviewPhase::Waiting {
            match self.state.pending_delay_secs() {
                Some(0) => self.state.poll_pending(),
                Some(delay) => ctx.request_repaint_after(Duration::from_secs(
                    u64::try_from(delay).unwrap_or(3_600).min(3_600),
                )),
                None => {}
            }
        }
        render(&mut self.state, ctx);
    }
}

struct Palette {
    bg: Color32,
    panel: Color32,
    panel_alt: Color32,
    text: Color32,
    muted: Color32,
    accent: Color32,
    accent_ink: Color32,
    border: Color32,
    danger: Color32,
    peach: Color32,
    success: Color32,
}

const fn palette(theme: ThemeChoice, system_dark: bool) -> Palette {
    let dark = match theme {
        ThemeChoice::Dark => true,
        ThemeChoice::Light => false,
        ThemeChoice::System => system_dark,
    };
    if dark {
        Palette {
            bg: Color32::from_rgb(24, 24, 37),
            panel: Color32::from_rgb(35, 36, 52),
            panel_alt: Color32::from_rgb(49, 50, 68),
            text: Color32::from_rgb(220, 224, 247),
            muted: Color32::from_rgb(166, 173, 200),
            accent: Color32::from_rgb(137, 180, 250),
            accent_ink: Color32::from_rgb(17, 17, 27),
            border: Color32::from_rgb(69, 71, 90),
            danger: Color32::from_rgb(243, 139, 168),
            peach: Color32::from_rgb(250, 179, 135),
            success: Color32::from_rgb(166, 227, 161),
        }
    } else {
        Palette {
            bg: Color32::from_rgb(239, 241, 245),
            panel: Color32::from_rgb(255, 255, 255),
            panel_alt: Color32::from_rgb(230, 233, 239),
            text: Color32::from_rgb(76, 79, 105),
            muted: Color32::from_rgb(108, 111, 133),
            accent: Color32::from_rgb(30, 102, 245),
            accent_ink: Color32::from_rgb(255, 255, 255),
            border: Color32::from_rgb(188, 192, 204),
            danger: Color32::from_rgb(210, 15, 57),
            peach: Color32::from_rgb(254, 100, 11),
            success: Color32::from_rgb(64, 160, 43),
        }
    }
}

fn apply_style(ctx: &egui::Context, theme: ThemeChoice) {
    let system_dark = matches!(ctx.theme(), egui::Theme::Dark);
    let p = palette(theme, system_dark);
    let mut style = (*ctx.style()).clone();
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(18.0));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(17.0));
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::proportional(26.0));
    style.spacing.item_spacing = vec2(12.0, 12.0);
    style.spacing.button_padding = vec2(14.0, 10.0);
    style.spacing.interact_size = vec2(TAP_HEIGHT, TAP_HEIGHT);
    style.visuals.dark_mode = matches!(theme, ThemeChoice::Dark)
        || (theme == ThemeChoice::System && system_dark);
    style.visuals.override_text_color = Some(p.text);
    style.visuals.weak_text_color = Some(p.muted);
    style.visuals.panel_fill = p.bg;
    style.visuals.window_fill = p.bg;
    style.visuals.extreme_bg_color = p.panel;
    style.visuals.widgets.noninteractive.bg_fill = p.panel;
    style.visuals.widgets.inactive.bg_fill = p.panel_alt;
    style.visuals.widgets.hovered.bg_fill = p.panel_alt;
    style.visuals.widgets.active.bg_fill = p.panel_alt;
    style.visuals.widgets.inactive.fg_stroke.color = p.text;
    style.visuals.widgets.hovered.fg_stroke.color = p.text;
    style.visuals.widgets.active.fg_stroke.color = p.text;
    style.visuals.text_edit_bg_color = Some(p.panel_alt);
    style.visuals.widgets.inactive.weak_bg_fill = p.panel_alt;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, p.border);
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, p.border);
    style.visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, p.border);
    style.visuals.widgets.noninteractive.bg_stroke =
        Stroke::new(1.0_f32, p.border);
    style.visuals.widgets.noninteractive.corner_radius = CornerRadius::same(12);
    style.visuals.widgets.inactive.corner_radius = CornerRadius::same(12);
    style.visuals.widgets.hovered.corner_radius = CornerRadius::same(12);
    style.visuals.widgets.active.corner_radius = CornerRadius::same(12);
    ctx.set_style(style);
}

/// Draws the current screen and, outside review, the bottom tabs.
pub fn render(app: &mut AppState, ctx: &egui::Context) {
    let system_dark = matches!(ctx.theme(), egui::Theme::Dark);
    let p = palette(app.settings.theme, system_dark);
    egui::CentralPanel::default()
        .frame(
            EguiFrame::new()
                .fill(p.bg)
                .inner_margin(Margin::same(PAGE_MARGIN)),
        )
        .show(ctx, |ui| {
            show_toast(app, ui, &p);
            let size = ui.available_size();
            let show_tabs = app.screen != Screen::Review;
            let tab_space = if show_tabs {
                TAB_HEIGHT + ui.spacing().item_spacing.y
            } else {
                0.0
            };
            let content_height = (size.y - tab_space).max(1.0);
            ui.allocate_ui_with_layout(
                vec2(size.x, content_height),
                Layout::top_down(Align::Min),
                |ui| {
                    ScrollArea::vertical()
                        .id_salt((
                            app.screen,
                            app.review.current.as_ref().map(|card| card.id),
                        ))
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            match app.screen {
                                Screen::Decks => render_decks(app, ui, &p),
                                Screen::Review => render_review(app, ui, &p),
                                Screen::Add => render_add(app, ui, &p),
                                Screen::Browse => render_browse(app, ui, &p),
                                Screen::Settings => {
                                    render_settings(app, ui, &p);
                                }
                            }
                        });
                },
            );
            if show_tabs {
                ui.allocate_ui_with_layout(
                    vec2(size.x, TAB_HEIGHT),
                    Layout::top_down(Align::Min),
                    |ui| render_tabs(app, ui, &p),
                );
            }
        });
}

fn show_toast(app: &mut AppState, ui: &mut Ui, p: &Palette) {
    let Some(toast) = app.toast.clone() else {
        return;
    };
    let color = match toast.kind {
        ToastKind::Success => p.success,
        ToastKind::Error => p.danger,
    };
    EguiFrame::new()
        .fill(p.panel)
        .stroke(Stroke::new(1.0_f32, color))
        .inner_margin(Margin::same(12))
        .corner_radius(CornerRadius::same(12))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(toast.text).color(color));
                if ui
                    .add_sized(
                        [72.0, TAP_HEIGHT],
                        touch_button(
                            RichText::new("Dismiss").color(p.muted),
                            TAP_HEIGHT,
                            Color32::TRANSPARENT,
                            p.muted,
                        ),
                    )
                    .clicked()
                {
                    app.toast = None;
                }
            });
        });
    ui.add_space(10.0);
}

fn touch_button(
    text: RichText,
    height: f32,
    fill: Color32,
    ink: Color32,
) -> Button<'static> {
    Button::new(text.color(ink).strong())
        .min_size(vec2(0.0, height))
        .corner_radius(CornerRadius::same(12))
        .fill(fill)
}

fn secondary_button(
    text: RichText,
    height: f32,
    p: &Palette,
) -> Button<'static> {
    Button::new(text.color(p.text).size(17.0).strong())
        .min_size(vec2(0.0, height))
        .corner_radius(CornerRadius::same(12))
        .fill(p.panel_alt)
        .stroke(Stroke::new(1.0_f32, p.border))
}

fn title(ui: &mut Ui, text: &str, p: &Palette) {
    ui.label(RichText::new(text).size(26.0).strong().color(p.text));
    ui.add_space(10.0);
}

fn render_tabs(app: &mut AppState, ui: &mut Ui, p: &Palette) {
    let tabs = [
        (Screen::Decks, "Decks"),
        (Screen::Add, "Add"),
        (Screen::Browse, "Browse"),
        (Screen::Settings, "Settings"),
    ];
    let width = (ui.available_width() - ((tabs.len() as f32) - 1.0) * 8.0)
        / (tabs.len() as f32);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.spacing_mut().button_padding = vec2(4.0, 8.0);
        ui.style_mut()
            .text_styles
            .insert(TextStyle::Button, FontId::proportional(15.0));
        ui.horizontal(|ui| {
            for (screen, label) in tabs {
                let selected = app.screen == screen;
                let button = touch_button(
                    RichText::new(label),
                    TAB_HEIGHT,
                    if selected { p.accent } else { p.panel },
                    if selected { p.accent_ink } else { p.text },
                )
                .stroke(Stroke::new(1.0_f32, p.border));
                if ui.add_sized([width, TAB_HEIGHT], button).clicked() {
                    let _ = app.set_screen(screen);
                }
            }
        });
    });
}

fn render_decks(app: &mut AppState, ui: &mut Ui, p: &Palette) {
    title(ui, "Decks", p);

    let mut query = app.deck_query.clone();
    let search = TextEdit::singleline(&mut query)
        .hint_text("Search decks")
        .desired_width(f32::MAX)
        .min_size(vec2(0.0, TAP_HEIGHT));
    if ui
        .add_sized([ui.available_width(), TAP_HEIGHT], search)
        .changed()
    {
        app.set_deck_query(query);
    }
    ui.add_space(12.0);

    let study_all = touch_button(
        RichText::new("Study all"),
        TAP_HEIGHT,
        p.accent,
        p.accent_ink,
    )
    .min_size(vec2(ui.available_width(), TAP_HEIGHT));
    if ui.add(study_all).clicked() {
        let _ = app.start_review(None);
    }
    ui.add_space(16.0);

    let filter = app.deck_query.trim().to_lowercase();
    let rows: Vec<DeckRow> = app
        .decks
        .iter()
        .filter(|row| {
            filter.is_empty() || row.name.to_lowercase().contains(&filter)
        })
        .cloned()
        .collect();
    if rows.is_empty() {
        if app.deck_query.trim().is_empty() {
            ui.label(RichText::new("No decks yet.").color(p.muted));
        } else {
            ui.label(
                RichText::new("No decks match this search.").color(p.muted),
            );
            ui.add_space(8.0);
            let clear =
                secondary_button(RichText::new("Clear search"), TAP_HEIGHT, p);
            if ui
                .add_sized([ui.available_width(), TAP_HEIGHT], clear)
                .clicked()
            {
                app.set_deck_query(String::new());
            }
        }
    }
    for row in rows {
        EguiFrame::new()
            .fill(p.panel)
            .stroke(Stroke::new(1.0_f32, p.border))
            .inner_margin(Margin::same(12))
            .corner_radius(CornerRadius::same(12))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(RichText::new(row.name).size(20.0).strong());
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    stat(ui, "New", row.stats.new_cards, p);
                    stat(ui, "Learning", row.stats.learning_cards, p);
                    stat(ui, "Review", row.stats.review_cards, p);
                });
                ui.add_space(10.0);
                let label = if row.due == 0 {
                    String::from("No cards due")
                } else {
                    format!("Study ({} due)", row.due)
                };
                let button = touch_button(
                    RichText::new(label),
                    TAP_HEIGHT,
                    p.panel_alt,
                    p.text,
                )
                .min_size(vec2(ui.available_width(), TAP_HEIGHT));
                if ui.add(button).clicked() {
                    let _ = app.start_review(row.id);
                }
            });
        ui.add_space(10.0);
    }
}

fn stat(ui: &mut Ui, label: &str, value: usize, p: &Palette) {
    EguiFrame::new()
        .fill(p.panel_alt)
        .inner_margin(Margin::symmetric(10, 8))
        .corner_radius(CornerRadius::same(10))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(format!("{value}")).size(18.0).strong());
                ui.label(RichText::new(label).size(13.0).color(p.muted));
            });
        });
}

fn render_review(app: &mut AppState, ui: &mut Ui, p: &Palette) {
    title(ui, "Review", p);
    let review = app.review.clone();
    let remaining = review.queue.len()
        + review.pending.len()
        + usize::from(review.current.is_some());
    let total = review.answered + remaining;
    let fraction = if total == 0 {
        1.0
    } else {
        (review.answered as f32) / (total as f32)
    };
    ui.add(
        ProgressBar::new(fraction)
            .desired_width(ui.available_width())
            .desired_height(8.0)
            .fill(p.accent),
    );
    ui.add_space(8.0);
    ui.label(
        RichText::new(format!("{} / {} reviewed", review.answered, total))
            .color(p.muted),
    );
    let counts =
        review_counts(&review.queue, &review.pending, review.current.as_ref());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        stat(ui, "New", counts[0], p);
        stat(ui, "Learning", counts[1], p);
        stat(ui, "Review", counts[2], p);
    });
    ui.add_space(12.0);

    if matches!(review.phase, ReviewPhase::Summary | ReviewPhase::Waiting) {
        let waiting = review.phase == ReviewPhase::Waiting;
        EguiFrame::new()
            .fill(p.panel)
            .stroke(Stroke::new(1.0_f32, p.border))
            .inner_margin(Margin::same(16))
            .corner_radius(CornerRadius::same(12))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                let heading = if waiting {
                    "Next card pending"
                } else {
                    "Session complete"
                };
                ui.label(RichText::new(heading).size(22.0).strong());
                ui.add_space(8.0);
                ui.label(&review.summary);
            });
        ui.add_space(12.0);
        if review.last.is_some() {
            let undo = secondary_button(
                RichText::new("Undo last grade"),
                TAP_HEIGHT,
                p,
            );
            if ui
                .add_sized([ui.available_width(), TAP_HEIGHT], undo)
                .clicked()
            {
                let _ = app.undo();
            }
            ui.add_space(8.0);
        }
        if !waiting {
            let again = touch_button(
                RichText::new("Study again"),
                TAP_HEIGHT,
                p.accent,
                p.accent_ink,
            );
            if ui
                .add_sized([ui.available_width(), TAP_HEIGHT], again)
                .clicked()
            {
                let _ = app.start_review(app.review_scope);
            }
            ui.add_space(8.0);
        }
        let done =
            secondary_button(RichText::new("Back to decks"), TAP_HEIGHT, p);
        if ui
            .add_sized([ui.available_width(), TAP_HEIGHT], done)
            .clicked()
        {
            app.end_review();
        }
        return;
    }

    let media_dir = app.settings.paths.media_dir.clone();
    if let (Some(card), Some(note), Some(model)) = (
        review.current.as_ref(),
        review.note.as_ref(),
        review.model.as_ref(),
    ) {
        let (question, answer) = sides(model, note, card.ordinal);
        EguiFrame::new()
            .fill(p.panel)
            .stroke(Stroke::new(1.0_f32, p.border))
            .inner_margin(Margin::same(16))
            .corner_radius(CornerRadius::same(12))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(RichText::new("Question").size(14.0).color(p.muted));
                ui.add_space(6.0);
                render_side(ui, &question, &media_dir);
                if review.phase == ReviewPhase::Answer {
                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(12.0);
                    ui.label(RichText::new("Answer").size(14.0).color(p.muted));
                    ui.add_space(6.0);
                    render_side(ui, &answer, &media_dir);
                }
            });
    } else {
        ui.label(RichText::new("No card loaded.").color(p.muted));
    }
    ui.add_space(12.0);

    if review.phase == ReviewPhase::Question {
        let reveal = touch_button(
            RichText::new("Show answer"),
            TAP_HEIGHT,
            p.accent,
            p.accent_ink,
        );
        if ui
            .add_sized([ui.available_width(), TAP_HEIGHT], reveal)
            .clicked()
        {
            app.reveal();
        }
    } else {
        let ratings = [
            (Rating::Again, "Again", p.danger),
            (Rating::Hard, "Hard", p.peach),
            (Rating::Good, "Good", p.accent),
            (Rating::Easy, "Easy", p.success),
        ];
        let width = (ui.available_width() - 24.0).max(0.0) / 4.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for (index, (rating, label, fill)) in ratings.iter().enumerate() {
                let ink = if matches!(rating, Rating::Hard | Rating::Easy) {
                    Color32::from_rgb(24, 24, 37)
                } else {
                    p.accent_ink
                };
                let interval =
                    interval_label(*review.intervals.get(index).unwrap_or(&0));
                let text = format!("{label}\n{interval}");
                let button = touch_button(
                    RichText::new(text),
                    RATING_HEIGHT,
                    *fill,
                    ink,
                );
                if ui.add_sized([width, RATING_HEIGHT], button).clicked() {
                    let _ = app.rate(*rating);
                }
            }
        });
    }

    ui.add_space(10.0);
    if review.last.is_some() {
        let undo = secondary_button(RichText::new("Undo"), TAP_HEIGHT, p);
        if ui
            .add_sized([ui.available_width(), TAP_HEIGHT], undo)
            .clicked()
        {
            let _ = app.undo();
        }
        ui.add_space(8.0);
    }
    let end = secondary_button(RichText::new("End session"), TAP_HEIGHT, p);
    if ui
        .add_sized([ui.available_width(), TAP_HEIGHT], end)
        .clicked()
    {
        app.end_review();
    }
}

fn review_counts(
    queue: &[Card],
    pending: &[Card],
    current: Option<&Card>,
) -> [usize; 3] {
    let mut counts = [0_usize; 3];
    for card in current
        .into_iter()
        .chain(queue.iter())
        .chain(pending.iter())
    {
        if let Some(slot) = counts.get_mut(card_bucket(card)) {
            *slot = (*slot).saturating_add(1);
        }
    }
    counts
}

const fn card_bucket(card: &Card) -> usize {
    match card.card_type {
        CardType::New => 0,
        CardType::Learning | CardType::Relearning => 1,
        CardType::Review => 2,
    }
}

fn interval_label(interval: i32) -> String {
    if interval >= 0 {
        return format!("{interval}d");
    }
    let seconds = interval.saturating_abs();
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3_600 {
        format!("{}m", ceil_div(seconds, 60))
    } else if seconds < 86_400 {
        format!("{}h", ceil_div(seconds, 3_600))
    } else {
        format!("{}d", ceil_div(seconds, 86_400))
    }
}

const fn ceil_div(value: i32, divisor: i32) -> i32 {
    value
        .saturating_add(divisor.saturating_sub(1))
        .saturating_div(divisor)
}

fn render_add(app: &mut AppState, ui: &mut Ui, p: &Palette) {
    title(ui, "Add note", p);

    let deck_names: Vec<String> =
        app.add.decks.iter().map(|deck| deck.name.clone()).collect();
    let mut deck_index = app.add.deck_index;
    ui.label(RichText::new("Deck").color(p.muted));
    ComboBox::from_id_salt("mobile-add-deck")
        .selected_text(
            deck_names
                .get(deck_index)
                .cloned()
                .unwrap_or_else(|| String::from("Choose a deck")),
        )
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for (index, name) in deck_names.iter().enumerate() {
                ui.selectable_value(&mut deck_index, index, name.clone());
            }
        });
    if deck_index != app.add.deck_index {
        app.set_add_deck(deck_index);
    }
    ui.add_space(8.0);

    let model_names: Vec<String> = app
        .add
        .models
        .iter()
        .map(|model| model.name.clone())
        .collect();
    let mut model_index = app.add.model_index;
    ui.label(RichText::new("Note type").color(p.muted));
    ComboBox::from_id_salt("mobile-add-note-type")
        .selected_text(
            model_names
                .get(model_index)
                .cloned()
                .unwrap_or_else(|| String::from("Choose a note type")),
        )
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for (index, name) in model_names.iter().enumerate() {
                ui.selectable_value(&mut model_index, index, name.clone());
            }
        });
    if model_index != app.add.model_index {
        app.set_add_model(model_index);
    }
    ui.add_space(12.0);

    let field_names: Vec<String> = app
        .add
        .models
        .get(app.add.model_index)
        .map_or_else(Vec::new, |model| model.fields.clone());
    for index in 0..app.add.fields.len() {
        let label = field_names
            .get(index)
            .cloned()
            .unwrap_or_else(|| format!("Field {}", index + 1));
        ui.label(RichText::new(label).color(p.muted));
        let mut value = app.add.fields.get(index).cloned().unwrap_or_default();
        let field = TextEdit::multiline(&mut value)
            .desired_rows(2)
            .desired_width(ui.available_width())
            .min_size(vec2(0.0, TAP_HEIGHT));
        if ui.add(field).changed() {
            app.set_add_field(index, value);
        }
        ui.add_space(8.0);
    }

    ui.label(RichText::new("Tags").color(p.muted));
    let tags = TextEdit::singleline(&mut app.add.tags)
        .hint_text("Separate tags with spaces")
        .desired_width(ui.available_width())
        .min_size(vec2(0.0, TAP_HEIGHT));
    ui.add(tags);
    ui.add_space(12.0);

    title(ui, "Preview", p);
    EguiFrame::new()
        .fill(p.panel)
        .stroke(Stroke::new(1.0_f32, p.border))
        .inner_margin(Margin::same(12))
        .corner_radius(CornerRadius::same(12))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            if app.add.preview.is_empty() {
                ui.label(
                    RichText::new("The card preview appears here.")
                        .color(p.muted),
                );
            } else {
                ui.label(&app.add.preview);
            }
        });
    ui.add_space(12.0);

    let save = touch_button(
        RichText::new("Save note"),
        TAP_HEIGHT,
        p.accent,
        p.accent_ink,
    );
    if ui
        .add_sized([ui.available_width(), TAP_HEIGHT], save)
        .clicked()
    {
        let _ = app.add_note();
    }
}

fn render_browse(app: &mut AppState, ui: &mut Ui, p: &Palette) {
    title(ui, "Browse", p);
    let mut query = app.browse.query.clone();
    let search = TextEdit::singleline(&mut query)
        .hint_text("Search cards, tags, or decks")
        .desired_width(ui.available_width())
        .min_size(vec2(0.0, TAP_HEIGHT));
    if ui.add(search).changed() {
        let _ = app.set_browse_query(query);
    }
    ui.add_space(10.0);
    let count = if app.browse.total == 1 {
        String::from("1 card")
    } else {
        format!("{} cards", app.browse.total)
    };
    ui.label(RichText::new(count).color(p.muted));
    if app.browse.rows.len() < app.browse.total {
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!(
                "Showing first {} of {}",
                app.browse.rows.len(),
                app.browse.total
            ))
            .color(p.muted),
        );
    }
    ui.add_space(6.0);

    if app.browse.rows.is_empty() {
        let empty = if app.browse.query.trim().is_empty() {
            "No cards yet."
        } else {
            "No cards match this search."
        };
        ui.label(RichText::new(empty).color(p.muted));
    }
    for row in &app.browse.rows {
        EguiFrame::new()
            .fill(p.panel)
            .stroke(Stroke::new(1.0_f32, p.border))
            .inner_margin(Margin::same(12))
            .corner_radius(CornerRadius::same(12))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(RichText::new(row.front.clone()).size(18.0).strong());
                ui.add_space(4.0);
                ui.label(RichText::new(row.back.clone()).color(p.muted));
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!("{} / {}", row.deck, row.meta))
                        .size(13.0)
                        .color(p.muted),
                );
            });
        ui.add_space(8.0);
    }
}

fn render_settings(app: &mut AppState, ui: &mut Ui, p: &Palette) {
    title(ui, "Settings", p);

    EguiFrame::new()
        .fill(p.panel)
        .stroke(Stroke::new(1.0_f32, p.border))
        .inner_margin(Margin::same(12))
        .corner_radius(CornerRadius::same(12))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(RichText::new("Appearance").size(20.0).strong());
            ui.add_space(8.0);
            let mut theme = app.settings.theme;
            ui.label(RichText::new("Theme").color(p.muted));
            ComboBox::from_id_salt("mobile-theme")
                .selected_text(theme_name(theme))
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut theme, ThemeChoice::Dark, "Dark");
                    ui.selectable_value(
                        &mut theme,
                        ThemeChoice::Light,
                        "Light",
                    );
                    ui.selectable_value(
                        &mut theme,
                        ThemeChoice::System,
                        "System",
                    );
                });
            if theme != app.settings.theme {
                app.settings.theme = theme;
                app.say(format!("Theme set to {}", theme_name(theme)));
            }
        });
    ui.add_space(14.0);

    EguiFrame::new()
        .fill(p.panel)
        .stroke(Stroke::new(1.0_f32, p.border))
        .inner_margin(Margin::same(12))
        .corner_radius(CornerRadius::same(12))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(RichText::new("AnkiWeb sync").size(20.0).strong());
            ui.add_space(8.0);
            if !app.sync.status.is_empty() {
                ui.label(RichText::new(app.sync.status.clone()).color(p.muted));
                ui.add_space(8.0);
            }

            if app.sync.signed_in {
                ui.label(RichText::new("Signed in").color(p.success));
                ui.add_space(8.0);
                if !app.sync.username.is_empty() {
                    ui.label(
                        RichText::new(app.sync.username.clone()).color(p.text),
                    );
                    ui.add_space(8.0);
                }
                let sync_now =
                    secondary_button(RichText::new("Sync now"), TAP_HEIGHT, p)
                        .min_size(vec2(ui.available_width(), TAP_HEIGHT));
                if ui.add_enabled(!app.sync.busy, sync_now).clicked() {
                    let _ = app.sync_now();
                }
                ui.add_space(8.0);
                let sign_out =
                    secondary_button(RichText::new("Sign out"), TAP_HEIGHT, p)
                        .min_size(vec2(ui.available_width(), TAP_HEIGHT));
                if ui.add_enabled(!app.sync.busy, sign_out).clicked() {
                    let _ = app.sign_out();
                }
            } else {
                ui.label(RichText::new("Not signed in").color(p.muted));
                ui.add_space(8.0);
                let username = TextEdit::singleline(&mut app.sync.username)
                    .hint_text("Username")
                    .desired_width(ui.available_width())
                    .min_size(vec2(0.0, TAP_HEIGHT));
                ui.add(username);
                ui.add_space(8.0);
                let password = TextEdit::singleline(&mut app.sync.password)
                    .hint_text("Password")
                    .password(true)
                    .desired_width(ui.available_width())
                    .min_size(vec2(0.0, TAP_HEIGHT));
                ui.add(password);
                ui.add_space(8.0);
                let sign_in = touch_button(
                    RichText::new("Sign in and sync"),
                    TAP_HEIGHT,
                    p.accent,
                    p.accent_ink,
                )
                .min_size(vec2(ui.available_width(), TAP_HEIGHT));
                if ui.add_enabled(!app.sync.busy, sign_in).clicked() {
                    let _ = app.sign_in();
                }
            }

            if app.sync.conflict {
                ui.add_space(12.0);
                ui.label(
                    RichText::new("Sync stopped on conflicting changes.")
                        .color(p.danger),
                );
                ui.add_space(8.0);
                let keep_local = secondary_button(
                    RichText::new("Keep local"),
                    TAP_HEIGHT,
                    p,
                )
                .min_size(vec2(ui.available_width(), TAP_HEIGHT));
                if ui.add_enabled(!app.sync.busy, keep_local).clicked() {
                    let _ = app.resolve_conflict(false);
                }
                ui.add_space(8.0);
                let keep_remote = secondary_button(
                    RichText::new("Keep remote"),
                    TAP_HEIGHT,
                    p,
                )
                .min_size(vec2(ui.available_width(), TAP_HEIGHT));
                if ui.add_enabled(!app.sync.busy, keep_remote).clicked() {
                    let _ = app.resolve_conflict(true);
                }
            }
        });
}

const fn theme_name(theme: ThemeChoice) -> &'static str {
    match theme {
        ThemeChoice::Dark => "Dark",
        ThemeChoice::Light => "Light",
        ThemeChoice::System => "System",
    }
}

#[cfg(test)]
mod tests {
    use super::interval_label;

    #[test]
    fn interval_labels_use_readable_time_units() {
        assert_eq!(interval_label(-30), "30s");
        assert_eq!(interval_label(-119), "2m");
        assert_eq!(interval_label(-7_200), "2h");
        assert_eq!(interval_label(-86_400), "1d");
        assert_eq!(interval_label(3), "3d");
    }
}
