//! Learning screen: header, fixed card stage, reveal and rating controls.

use gpui::{
    ClickEvent, Context, InteractiveElement, IntoElement, ParentElement, Rgba,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use ranki_model::{CardType, Note, NoteModel};

use crate::{
    app::{Phase, Root, UiAction, leaf_deck_totals, rating_name},
    card_view,
    components::{chip, icon, keycap, progress_bar_width},
    theme::{
        CONTROL_HEIGHT, FOOTER_HEIGHT, PROGRESS_WIDTH, RATING_GAP,
        STAGE_HEIGHT, STAGE_PADDING, STAGE_RADIUS, STAGE_WIDTH, TOPBAR_HEIGHT,
        alpha, interval_label, rating_fill, rating_label,
    },
};

impl Root {
    pub(crate) fn render_review(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = self.colors();
        let style = self.style();
        let stats = if self.review.scope_all {
            leaf_deck_totals(&self.decks, &self.stats)
        } else {
            self.selected
                .and_then(|index| self.stats.get(index))
                .copied()
                .unwrap_or_default()
        };
        let total = self.review.session_total;
        let fraction = if total == 0 {
            0.0
        } else {
            self.review.answered as f32 / total as f32
        };
        let current = self.review.current.clone();
        let phase = self.review.phase;
        let question = self.review.question.clone();
        let answer = self.review.answer.clone();
        let note = self.review.note.clone();
        let status = self.review.status.clone();
        let active_queue = current.as_ref().map(|card| card.card_type);
        let collection_start = self.store.collection_created().unwrap_or(0);
        let now = Self::now();
        let previews = if let Some(card) = current.as_ref() {
            [
                self.scheduler.preview(
                    card,
                    ranki_model::Rating::Again,
                    now,
                    collection_start,
                ),
                self.scheduler.preview(
                    card,
                    ranki_model::Rating::Hard,
                    now,
                    collection_start,
                ),
                self.scheduler.preview(
                    card,
                    ranki_model::Rating::Good,
                    now,
                    collection_start,
                ),
                self.scheduler.preview(
                    card,
                    ranki_model::Rating::Easy,
                    now,
                    collection_start,
                ),
            ]
        } else {
            [0; 4]
        };
        let deck_name = if self.review.scope_all {
            String::from("All decks")
        } else {
            self.selected
                .and_then(|index| self.decks.get(index))
                .map_or_else(
                    || String::from("All decks"),
                    |deck| deck.name.replace("::", " › "),
                )
        };
        let tags = note
            .as_ref()
            .map(|note| note.tags.clone())
            .unwrap_or_default();
        let reading = reading_label(&self.models, note.as_ref());
        let reveal = self.action_click(UiAction::Reveal);
        let end = self.action_click(UiAction::EndSession);
        let queue_count = |name: &str,
                           value: usize,
                           color: Rgba,
                           ink: Rgba,
                           active: bool| {
            let mut wrapper = div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(2.))
                .pb(px(4.))
                .border_b_2()
                .border_color(if active { color } else { alpha(color, 0.0) });
            if active {
                wrapper = wrapper.child(
                    div()
                        .text_size(px(27.))
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(ink)
                        .child(format!("{value}")),
                );
            } else {
                wrapper = wrapper.child(
                    div()
                        .text_size(px(11.))
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(ink)
                        .child(format!("{value}")),
                );
            }
            wrapper.child(
                div()
                    .text_size(px(13.))
                    .text_color(colors.subtext)
                    .child(name.to_owned()),
            )
        };
        let mut screen = div().size_full().flex().flex_col();

        screen = screen.child(
            div()
                .h(px(TOPBAR_HEIGHT))
                .flex_none()
                .flex()
                .items_center()
                .justify_between()
                .px(px(32.))
                .child(
                    div()
                        .id("review-end")
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .h(px(40.))
                        .px(px(14.))
                        .rounded(px(12.))
                        .bg(colors.surface0)
                        .text_size(px(14.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .cursor_pointer()
                        .on_click(end)
                        .child(icon("icon-arrow-left", 18., colors.text))
                        .child("End session")
                        .child(keycap("Esc", colors, false)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_size(px(14.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(deck_name),
                        )
                        .children((total > 0).then(|| {
                            progress_bar_width(fraction, colors, PROGRESS_WIDTH)
                        })),
                )
                .child(
                    div()
                        .flex()
                        .items_end()
                        .gap(px(22.))
                        .child(queue_count(
                            "New",
                            stats.new_cards,
                            colors.blue,
                            crate::theme::queue_text(0, self.theme()),
                            active_queue == Some(CardType::New),
                        ))
                        .child(queue_count(
                            "Learning",
                            stats.learning_cards,
                            colors.red,
                            crate::theme::queue_text(1, self.theme()),
                            matches!(
                                active_queue,
                                Some(CardType::Learning | CardType::Relearning)
                            ),
                        ))
                        .child(queue_count(
                            "Review",
                            stats.review_cards,
                            colors.green,
                            crate::theme::queue_text(2, self.theme()),
                            active_queue == Some(CardType::Review),
                        )),
                ),
        );

        let body = if current.is_none() {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(8.))
                        .w(px(STAGE_WIDTH))
                        .h(px(240.))
                        .rounded(px(STAGE_RADIUS))
                        .bg(colors.surface0)
                        .child(
                            div()
                                .text_size(px(32.))
                                .font_weight(gpui::FontWeight::BOLD)
                                .child("All caught up"),
                        )
                        .child(
                            div()
                                .text_size(px(14.))
                                .text_color(colors.subtext)
                                .child("No cards are due right now"),
                        ),
                )
                .into_any_element()
        } else {
            let mut stage = div()
                .w(px(STAGE_WIDTH))
                .h(px(STAGE_HEIGHT))
                .flex_none()
                .flex()
                .flex_col()
                .p(px(STAGE_PADDING))
                .rounded(px(STAGE_RADIUS))
                .bg(colors.surface0)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(div().flex().gap(px(6.)).children(
                            tags.iter().map(|tag| chip(tag.clone(), colors)),
                        ))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .h(px(40.))
                                .px(px(12.))
                                .rounded(px(20.))
                                .bg(alpha(colors.mauve, 0.16))
                                .text_color(colors.mauve)
                                .child(icon("icon-volume", 20., colors.mauve))
                                .child(keycap("R", colors, false)),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(10.))
                        .children((phase == Phase::Question).then(|| {
                            div()
                                .text_size(px(11.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.subtext)
                                .child("RECALL THE MEANING")
                        }))
                        .child(if let Some(card) = question.as_ref() {
                            card_view::rendered_text(card, colors.text, 84.)
                                .into_any_element()
                        } else {
                            div()
                                .text_size(px(84.))
                                .child("No cards due")
                                .into_any_element()
                        })
                        .children(reading.as_ref().map(|text| {
                            div()
                                .text_size(px(18.))
                                .text_color(colors.subtext)
                                .child(text.clone())
                        }))
                        .children(
                            (phase == Phase::Answer).then(|| separator(colors)),
                        )
                        .children((phase == Phase::Answer).then(|| {
                            div()
                                .text_size(px(36.))
                                .font_weight(gpui::FontWeight::BOLD)
                                .text_color(colors.mauve)
                                .child(if let Some(card) = answer.as_ref() {
                                    card_view::rendered_text(
                                        card,
                                        colors.mauve,
                                        32.,
                                    )
                                    .into_any_element()
                                } else {
                                    div().into_any_element()
                                })
                        })),
                );
            if let Some(card) = question.as_ref()
                && let Some(media) =
                    card_view::media_column(&card.media, colors)
            {
                stage = stage.child(media);
            }
            if let Some(card) = question.as_ref()
                && !card.masks.is_empty()
            {
                stage = stage.child(card_view::occlusion_layer(
                    &card.masks,
                    colors,
                    phase == Phase::Answer,
                ));
            }
            let stage = gpui::image_cache(self.images.clone()).child(stage);

            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(24.))
                .child(stage)
                .child(if phase == Phase::Question {
                    div()
                        .id("review-reveal")
                        .w(px(STAGE_WIDTH))
                        .h(px(CONTROL_HEIGHT))
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(12.))
                        .rounded(px(18.))
                        .bg(colors.mauve)
                        .text_color(style.ink())
                        .text_size(px(17.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .cursor_pointer()
                        .on_click(reveal)
                        .child("Show answer")
                        .child(keycap("Space", colors, true))
                        .into_any_element()
                } else {
                    rating_row(colors, &previews, cx).into_any_element()
                })
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(colors.subtext)
                        .child(status.unwrap_or_default()),
                )
                .into_any_element()
        };
        screen = screen.child(body);

        screen.child(
            div()
                .h(px(FOOTER_HEIGHT))
                .flex_none()
                .flex()
                .items_center()
                .justify_between()
                .px(px(32.))
                .child(
                    div()
                        .flex()
                        .gap(px(4.))
                        .child(footer_action("Undo", "Z", colors))
                        .child(footer_action("Flag", "F", colors)),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(4.))
                        .child(footer_action("Edit", "E", colors))
                        .child(footer_action("Bury", "B", colors)),
                ),
        )
    }
}

fn separator(colors: crate::theme::Palette) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .w_full()
        .my(px(10.))
        .child(div().h(px(1.)).flex_1().bg(colors.surface1))
        .child(
            div()
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.subtext)
                .child("ANSWER"),
        )
        .child(div().h(px(1.)).flex_1().bg(colors.surface1))
}

/// A field named Reading, Kana, or Pronunciation, when the note has one.
fn reading_label(models: &[NoteModel], note: Option<&Note>) -> Option<String> {
    let note = note?;
    let model = models.iter().find(|model| model.id == note.model_id)?;
    let index = model.fields.iter().position(|field| {
        field.eq_ignore_ascii_case("reading")
            || field.eq_ignore_ascii_case("kana")
            || field.eq_ignore_ascii_case("pronunciation")
    })?;
    let text = note.fields.get(index)?.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

fn rating_row(
    colors: crate::theme::Palette,
    previews: &[i32; 4],
    cx: &Context<Root>,
) -> gpui::Div {
    let mut row = div()
        .w(px(STAGE_WIDTH))
        .h(px(CONTROL_HEIGHT))
        .flex()
        .gap(px(RATING_GAP));
    for (index, (rating, interval)) in [
        (ranki_model::Rating::Again, previews[0]),
        (ranki_model::Rating::Hard, previews[1]),
        (ranki_model::Rating::Good, previews[2]),
        (ranki_model::Rating::Easy, previews[3]),
    ]
    .into_iter()
    .enumerate()
    {
        let code = rating.code();
        let action = match rating {
            ranki_model::Rating::Again => {
                UiAction::Grade(ranki_model::Rating::Again)
            }
            ranki_model::Rating::Hard => {
                UiAction::Grade(ranki_model::Rating::Hard)
            }
            ranki_model::Rating::Good => {
                UiAction::Grade(ranki_model::Rating::Good)
            }
            ranki_model::Rating::Easy => {
                UiAction::Grade(ranki_model::Rating::Easy)
            }
        };
        let theme = theme_from(colors);
        let hue = rating_fill(code, theme);
        let label_color = rating_label(code, theme);
        row = row.child(
            div()
                .id("review-rating")
                .flex_1()
                .h(px(CONTROL_HEIGHT))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(4.))
                .rounded(px(18.))
                .bg(alpha(hue, 0.14))
                .border_1()
                .border_color(alpha(hue, 0.4))
                .cursor_pointer()
                .on_click(cx.listener(
                    move |this, _: &ClickEvent, window, cx| {
                        this.dispatch(action.clone(), window, cx);
                    },
                ))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(colors.text)
                        .child(interval_label(interval)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .text_size(px(16.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(label_color)
                        .child(rating_name(rating))
                        .child(
                            div()
                                .px(px(6.))
                                .py(px(2.))
                                .rounded(px(6.))
                                .bg(alpha(hue, 0.2))
                                .text_size(px(11.))
                                .child(format!("{}", index + 1)),
                        ),
                ),
        );
    }
    row
}

fn theme_from(colors: crate::theme::Palette) -> crate::theme::Theme {
    if colors.base == crate::theme::MOCHA.base {
        crate::theme::Theme::Dark
    } else {
        crate::theme::Theme::Light
    }
}

fn footer_action(
    label: &str,
    key: &str,
    colors: crate::theme::Palette,
) -> gpui::Div {
    let icon_name = match label {
        "Undo" => "icon-undo",
        "Flag" => "icon-flag",
        "Edit" => "icon-more",
        _ => "icon-x",
    };
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(36.))
        .px(px(12.))
        .rounded(px(10.))
        .text_size(px(13.))
        .text_color(colors.subtext)
        .child(icon(icon_name, 16., colors.subtext))
        .child(label.to_owned())
        .child(keycap(key.to_owned(), colors, false))
}

#[cfg(test)]
mod tests {
    use crate::{
        app::Phase,
        theme::{Theme, rating_fill},
    };

    #[test]
    fn question_phase_hides_answer_controls() {
        assert_ne!(Phase::Question, Phase::Answer);
    }

    #[test]
    fn rating_palette_uses_peach_for_hard() {
        assert_eq!(rating_fill(2, Theme::Dark), crate::theme::MOCHA.peach);
    }
}
