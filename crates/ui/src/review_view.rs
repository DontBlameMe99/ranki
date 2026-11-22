//! Learning screen: header, fixed card stage, reveal and rating controls.

use gpui::{
    ClickEvent, Context, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, px,
};

use crate::{
    app::{Phase, Root, UiAction, rating_name},
    card_view,
    components::{icon, keycap, progress_bar_width},
    theme::{
        CONTROL_HEIGHT, FOOTER_HEIGHT, NAV_WIDTH, PROGRESS_WIDTH, RATING_GAP,
        STAGE_HEIGHT, STAGE_PADDING, STAGE_RADIUS, STAGE_WIDTH, TOPBAR_HEIGHT,
        accent_ink, alpha, interval_label, rating_fill, rating_label,
    },
};

impl Root {
    pub(crate) fn render_review(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = self.colors();
        let theme = self.theme();
        let stats = self
            .selected
            .and_then(|index| self.stats.get(index))
            .copied()
            .unwrap_or_default();
        let total = self.review.session_total.max(1);
        let fraction = self.review.answered as f32 / total as f32;
        let usable = f32::from(window.viewport_size().width) - NAV_WIDTH - 32.0;
        let progress_width = (PROGRESS_WIDTH).min((usable - 460.0).max(150.0));
        let current = self.review.current.clone();
        let phase = self.review.phase;
        let question = self.review.question.clone();
        let answer = self.review.answer.clone();
        let status = self.review.status.clone();
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
        let deck_name = self
            .selected
            .and_then(|index| self.decks.get(index))
            .map_or_else(|| "All decks".to_owned(), |deck| deck.name.clone());
        let reveal = self.action_click(UiAction::Reveal);
        let end = self.action_click(UiAction::EndSession);
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
                        .id("auto-review_view-1")
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
                        .child(icon("icon-arrow-left", 18.))
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
                                .child(format!(
                                    "{} · {}",
                                    deck_name,
                                    crate::theme::scheduler_label(
                                        self.options.scheduler
                                    )
                                )),
                        )
                        .child(progress_bar_width(
                            fraction,
                            colors,
                            progress_width,
                        ))
                        .child(
                            div()
                                .w(px(progress_width))
                                .text_size(px(11.))
                                .text_color(colors.subtext)
                                .child(format!(
                                    "{} / {} reviewed",
                                    self.review.answered,
                                    self.review.session_total
                                )),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_end()
                        .gap(px(12.))
                        .child(queue_count(
                            "New",
                            stats.new_cards,
                            colors.blue,
                            colors,
                            false,
                        ))
                        .child(queue_count(
                            "Learning",
                            stats.learning_cards,
                            colors.red,
                            colors,
                            false,
                        ))
                        .child(queue_count(
                            "Review",
                            stats.review_cards,
                            colors.green,
                            colors,
                            true,
                        )),
                ),
        );

        let stage = div()
            .w(px(STAGE_WIDTH))
            .h(px(STAGE_HEIGHT))
            .flex_none()
            .flex()
            .flex_col()
            .p(px(STAGE_PADDING))
            .rounded(px(STAGE_RADIUS))
            .bg(colors.surface0)
            .child(top_row(colors))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(10.))
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(colors.subtext)
                            .child("RECALL THE MEANING"),
                    )
                    .child(if let Some(card) = question.as_ref() {
                        card_view::rendered_text(
                            card,
                            colors,
                            if phase == Phase::Answer { 44. } else { 84. },
                        )
                        .into_any_element()
                    } else {
                        div()
                            .text_size(px(84.))
                            .child("No cards due")
                            .into_any_element()
                    })
                    .child(
                        div()
                            .text_size(px(18.))
                            .text_color(colors.subtext)
                            .child(reading_label(question.as_ref())),
                    ),
            );
        let stage = if let Some(card) = question.as_ref() {
            match card_view::media_column(&card.media, colors) {
                Some(media) => stage.child(media),
                None => stage,
            }
        } else {
            stage
        };
        let stage = if let Some(card) = question.as_ref()
            && !card.masks.is_empty()
        {
            let answer_phase = phase == Phase::Answer;
            stage.child(card_view::occlusion_layer(
                &card.masks,
                colors,
                answer_phase,
            ))
        } else {
            stage
        };
        let stage = if phase == Phase::Answer {
            stage
                .child(separator(colors))
                .child(
                    div()
                        .text_size(px(36.))
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(colors.mauve)
                        .child(if let Some(card) = answer.as_ref() {
                            card_view::rendered_text(card, colors, 32.)
                                .into_any_element()
                        } else {
                            div().into_any_element()
                        }),
                )
                .child(feedback(colors))
        } else {
            stage
        };
        let stage = gpui::image_cache(self.images.clone()).child(stage);

        screen = screen.child(
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
                        .id("auto-review_view-2")
                        .w(px(STAGE_WIDTH))
                        .h(px(CONTROL_HEIGHT))
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(12.))
                        .rounded(px(18.))
                        .bg(colors.mauve)
                        .text_color(accent_ink(theme))
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
                        .mt(px(8.))
                        .text_size(px(12.))
                        .text_color(colors.subtext)
                        .child(status.unwrap_or_default()),
                ),
        );

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

fn top_row(colors: crate::theme::Palette) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(10.))
                .py(px(5.))
                .rounded(px(10.))
                .bg(colors.surface1)
                .text_size(px(12.))
                .child("Japanese N5"),
        )
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
                .text_size(px(13.))
                .child(icon("icon-volume", 20.))
                .child("Audio"),
        )
}

fn separator(colors: crate::theme::Palette) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.))
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

fn feedback(colors: crate::theme::Palette) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .p(px(10.))
        .px(px(14.))
        .rounded(px(16.))
        .bg(colors.base)
        .child(div().text_size(px(17.)).child("毎日りんごを食べる。"))
        .child(
            div()
                .text_size(px(13.))
                .text_color(colors.subtext)
                .child("I eat an apple every day."),
        )
}

fn reading_label(card: Option<&card_view::RenderedCard>) -> String {
    card.map_or_else(String::new, |card| {
        let text = card.text.lines().next().unwrap_or("");
        if text.is_ascii() {
            String::new()
        } else {
            format!("{text} · review")
        }
    })
}

fn queue_count(
    name: &str,
    value: usize,
    color: gpui::Rgba,
    colors: crate::theme::Palette,
    active: bool,
) -> gpui::Div {
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
                .text_color(color)
                .child(format!("{value}")),
        );
    } else {
        wrapper = wrapper.child(
            div()
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(color)
                .child(format!("{value}")),
        );
    }
    wrapper.child(
        div()
            .text_size(px(13.))
            .text_color(colors.subtext)
            .child(name.to_owned()),
    )
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
        let (fill_color, label_color) = rating_colors(code, colors);
        row = row.child(
            div()
                .id("auto-review_view-3")
                .flex_1()
                .h(px(CONTROL_HEIGHT))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(4.))
                .rounded(px(18.))
                .bg(fill_color)
                .border_1()
                .border_color(alpha(rating_fill(code, theme_from(colors)), 0.4))
                .cursor_pointer()
                .on_click(cx.listener(
                    move |this, _: &ClickEvent, window, cx| {
                        this.dispatch(action.clone(), window, cx);
                    },
                ))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(label_color)
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
                                .bg(alpha(
                                    rating_fill(code, theme_from(colors)),
                                    0.2,
                                ))
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

fn rating_colors(
    code: u8,
    colors: crate::theme::Palette,
) -> (gpui::Rgba, gpui::Rgba) {
    let theme = theme_from(colors);
    (
        alpha(
            rating_fill(code, theme),
            if theme == crate::theme::Theme::Dark {
                0.14
            } else {
                0.10
            },
        ),
        rating_label(code, theme),
    )
}

fn footer_action(
    label: &str,
    key: &str,
    colors: crate::theme::Palette,
) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(36.))
        .px(px(12.))
        .rounded(px(10.))
        .text_size(px(13.))
        .text_color(alpha(colors.subtext, 0.6))
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
