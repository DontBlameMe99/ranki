//! Learning screen: header, fixed card stage, reveal and rating controls.

use gpui::{
    ClickEvent, Context, InteractiveElement, IntoElement, ParentElement, Rgba,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use ranki_model::{CardType, Note, NoteModel};

use crate::{
    app::{Phase, Root, UiAction, duration_label, queue_counts, rating_name},
    card_view,
    components::{
        ClickHandler, Interaction, chip, fade_in, icon, keycap, progress_bar,
    },
    theme::{
        CONTROL_HEIGHT, FOOTER_HEIGHT, MOTION_PROGRESS_MS, MOTION_QUICK_MS,
        PROGRESS_WIDTH, RATING_GAP, SPACE_BLOCK, SPACE_GROUP, SPACE_GUTTER,
        SPACE_INLINE, SPACE_RELATED, SPACE_SECTION, STAGE_HEIGHT,
        STAGE_PADDING, STAGE_RADIUS, STAGE_WIDTH, TOPBAR_HEIGHT, accent_hover,
        accent_press, alpha, interval_label, rating_fill, rating_label,
    },
};

impl Root {
    pub(crate) fn render_review(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = self.colors();
        let style = self.style();
        let theme = self.theme();
        let session = queue_counts(
            self.review.current.as_ref(),
            &self.review.queue,
            &self.review.pending,
        );
        let more_due = self.review.more_due;
        let session_complete = self.session_complete();
        let viewport_height = f32::from(window.viewport_size().height);
        let stage_height = (viewport_height
            - TOPBAR_HEIGHT
            - FOOTER_HEIGHT
            - CONTROL_HEIGHT
            - 48.0
            - 18.0)
            .clamp(220.0, STAGE_HEIGHT);
        let image_height = (stage_height * 0.38).clamp(96.0, 180.0);
        let fraction = self.progress_fraction();
        let from = self.review.progress_from;
        let current = self.review.current.clone();
        let phase = self.review.phase;
        let question = self.review.question.clone();
        let answer = self.review.answer.clone();
        let answer_media = answer
            .as_ref()
            .and_then(|card| {
                let shown = question
                    .as_ref()
                    .map_or(&[][..], |card| card.media.as_slice());
                let extra = card_view::answer_only_media(shown, &card.media);
                (!extra.is_empty()).then(|| {
                    card_view::media_column(
                        &extra,
                        &self.options.media_dir,
                        colors,
                        image_height,
                    )
                })
            })
            .flatten();
        let note = self.review.note.clone();
        let answered = self.review.answered;
        let load_errors = self.review.load_errors;
        let load_error = self.review.load_error.clone();
        let again = self.review.again_count;
        let studied = duration_label(self.session_elapsed());
        let can_undo = self.review.last_grade.is_some();
        let quick = self.motion_ms(MOTION_QUICK_MS);
        let progress_ms = self.motion_ms(MOTION_PROGRESS_MS);
        let active_queue = current.as_ref().map(|card| card.card_type);
        let previews = self.review.previews;
        let deck_name = if self.review.scope_all {
            String::from("All decks")
        } else {
            self.selected
                .and_then(|index| self.decks.get(index))
                .map_or_else(
                    || String::from("All decks"),
                    |deck| deck.name.replace("::", " \u{203a} "),
                )
        };
        let tags = note
            .as_ref()
            .map(|note| note.tags.clone())
            .unwrap_or_default();
        let reading = (phase == Phase::Answer)
            .then(|| reading_label(&self.models, note.as_ref()))
            .flatten();
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
                .gap(px(SPACE_INLINE))
                .pb(px(SPACE_INLINE))
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
                .px(px(SPACE_GUTTER))
                .child(
                    div()
                        .id("review-end")
                        .flex()
                        .items_center()
                        .gap(px(SPACE_GROUP))
                        .h(px(40.))
                        .px(px(SPACE_RELATED))
                        .rounded(px(12.))
                        .bg(colors.surface0)
                        .text_size(px(14.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .cursor_pointer()
                        .focusable()
                        .border_1()
                        .border_color(alpha(colors.text, 0.0))
                        .hover(move |row| row.bg(colors.surface1))
                        .active(move |row| row.bg(alpha(colors.text, 0.08)))
                        .focus(move |row| row.border_color(colors.mauve))
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
                        .gap(px(SPACE_INLINE))
                        .child(
                            div()
                                .text_size(px(14.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(deck_name),
                        )
                        .child(progress_bar(
                            from,
                            fraction,
                            colors,
                            PROGRESS_WIDTH,
                            progress_ms,
                            ("progress", answered),
                        )),
                )
                .child(
                    div()
                        .flex()
                        .items_end()
                        .gap(px(SPACE_SECTION))
                        .child(queue_count(
                            "New",
                            session.new_cards,
                            colors.blue,
                            crate::theme::queue_text(0, theme),
                            active_queue == Some(CardType::New),
                        ))
                        .child(queue_count(
                            "Learning",
                            session.learning_cards,
                            colors.red,
                            crate::theme::queue_text(1, theme),
                            matches!(
                                active_queue,
                                Some(CardType::Learning | CardType::Relearning)
                            ),
                        ))
                        .child(queue_count(
                            "Review",
                            session.review_cards,
                            colors.green,
                            crate::theme::queue_text(2, theme),
                            active_queue == Some(CardType::Review),
                        )),
                ),
        );

        let load_failure = load_error.or_else(|| {
            (load_errors > 0).then(|| {
                format!(
                    "{load_errors} cards could not be loaded and were skipped."
                )
            })
        });
        let body = if current.is_none()
            && let Some(message) = load_failure
        {
            let retry = self.action_click(UiAction::StudyAgain);
            let back = self.action_click(UiAction::EndSession);
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .w(px(STAGE_WIDTH))
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(SPACE_BLOCK))
                        .p(px(SPACE_GUTTER))
                        .rounded(px(STAGE_RADIUS))
                        .bg(colors.surface0)
                        .child(
                            div()
                                .text_size(px(28.))
                                .font_weight(gpui::FontWeight::BOLD)
                                .child("Could not load cards"),
                        )
                        .child(
                            div()
                                .text_size(px(14.))
                                .text_color(colors.subtext)
                                .child(message),
                        )
                        .child(
                            div()
                                .flex()
                                .gap(px(SPACE_GROUP))
                                .child(footer_button(
                                    "Back to decks",
                                    colors,
                                    false,
                                    back,
                                ))
                                .child(footer_button(
                                    "Try again",
                                    colors,
                                    true,
                                    retry,
                                )),
                        )
                        .into_any_element(),
                )
                .into_any_element()
        } else if session_complete {
            let practice = self.action_click(UiAction::StudyAgain);
            let back = self.action_click(UiAction::EndSession);
            let summary =
                div().flex_1().flex().items_center().justify_center().child(
                    div()
                        .w(px(STAGE_WIDTH))
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(SPACE_BLOCK))
                        .p(px(SPACE_GUTTER))
                        .rounded(px(STAGE_RADIUS))
                        .bg(colors.surface0)
                        .child(
                            div()
                                .text_size(px(11.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.subtext)
                                .child("SESSION COMPLETE"),
                        )
                        .child(
                            div()
                                .text_size(px(32.))
                                .font_weight(gpui::FontWeight::BOLD)
                                .child(summary_title(answered)),
                        )
                        .child(
                            div()
                                .flex()
                                .gap(px(SPACE_BLOCK))
                                .child(summary_tile(
                                    "REVIEWED",
                                    &format!("{answered}"),
                                    colors.mauve,
                                    colors,
                                ))
                                .child(summary_tile(
                                    "AGAIN",
                                    &format!("{again}"),
                                    colors.red,
                                    colors,
                                ))
                                .child(summary_tile(
                                    "TIME",
                                    &studied,
                                    colors.green,
                                    colors,
                                )),
                        )
                        .children(more_due.then(|| {
                            div()
                                .text_size(px(14.))
                                .text_color(colors.subtext)
                                .child("More cards are due now.")
                        }))
                        .child(
                            div()
                                .flex()
                                .gap(px(SPACE_GROUP))
                                .child(footer_button(
                                    "Back to decks",
                                    colors,
                                    !more_due,
                                    back,
                                ))
                                .children(more_due.then(|| {
                                    footer_button(
                                        "Study again",
                                        colors,
                                        true,
                                        practice,
                                    )
                                })),
                        ),
                );
            summary.into_any_element()
        } else if current.is_none() {
            div()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(SPACE_GROUP))
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
                        )
                        .child(footer_button(
                            "Back to decks",
                            colors,
                            false,
                            self.action_click(UiAction::EndSession),
                        ))
                        .into_any_element(),
                )
                .into_any_element()
        } else {
            let mut stage = div()
                .w(px(STAGE_WIDTH))
                .h(px(stage_height))
                .flex_none()
                .flex()
                .flex_col()
                .p(px(STAGE_PADDING))
                .rounded(px(STAGE_RADIUS))
                .bg(colors.surface0)
                .child(div().flex().items_center().justify_between().child(
                    div().flex().gap(px(SPACE_GROUP)).children(
                        tags.iter().map(|tag| chip(tag.clone(), colors)),
                    ),
                ));
            let question_size = question.as_ref().map_or(84.0, |card| {
                card_text_size(
                    card,
                    stage_height,
                    !card.media.is_empty() || !card.masks.is_empty(),
                )
            });
            let question_size = if phase == Phase::Answer {
                question_size.min(40.0)
            } else {
                question_size
            };
            let answer_size = answer.as_ref().map_or(32.0, |card| {
                card_text_size(
                    card,
                    stage_height,
                    !card.media.is_empty() || !card.masks.is_empty(),
                )
                .clamp(22.0, 32.0)
            });
            let center = div()
                .id("review-stage-scroll")
                .flex_1()
                .min_h(px(0.))
                .overflow_scroll();
            let content_height =
                (stage_height - 2.0 * STAGE_PADDING - 56.0).max(120.0);
            let mut content = div()
                .min_h(px(content_height))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(SPACE_GROUP))
                .children((phase == Phase::Question).then(|| {
                    div()
                        .text_size(px(11.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.subtext)
                        .child("RECALL THE MEANING")
                }))
                .child(if let Some(card) = question.as_ref() {
                    card_view::rendered_text(card, colors.text, question_size)
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
                }));
            if phase == Phase::Answer {
                let mut answer_group = div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(SPACE_GROUP))
                    .child(separator(colors))
                    .child(if let Some(card) = answer.as_ref() {
                        card_view::rendered_text(
                            card,
                            colors.mauve,
                            answer_size,
                        )
                        .into_any_element()
                    } else {
                        div().into_any_element()
                    });
                if let Some(media) = answer_media {
                    answer_group = answer_group.child(media);
                }
                let key = current
                    .as_ref()
                    .map_or(0, |card| usize::try_from(card.id).unwrap_or(0));
                content = content.child(fade_in(
                    answer_group,
                    ("answer", key),
                    quick,
                ));
            }
            stage = stage.child(center.child(content));
            let occlusion_image = note
                .as_ref()
                .and_then(|note| note.fields.get(1))
                .and_then(|field| {
                    card_view::media_refs(field)
                        .into_iter()
                        .find(|media| media.kind == card_view::MediaKind::Image)
                });
            if let Some(card) = question.as_ref() {
                let masked =
                    occlusion_image.as_ref().filter(|_| !card.masks.is_empty());
                if let Some(image) = masked {
                    let mut media = div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(SPACE_GROUP))
                        .child(card_view::occlusion_media(
                            &self.options.media_dir,
                            &image.name,
                            &card.masks,
                            colors,
                            phase == Phase::Answer,
                            image_height,
                        ));
                    let rest = card
                        .media
                        .iter()
                        .filter(|media| {
                            Some(*media) != occlusion_image.as_ref()
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    if let Some(rest) = card_view::media_column(
                        &rest,
                        &self.options.media_dir,
                        colors,
                        image_height,
                    ) {
                        media = media.child(rest);
                    }
                    stage = stage.child(media);
                } else if let Some(media) = card_view::media_column(
                    &card.media,
                    &self.options.media_dir,
                    colors,
                    image_height,
                ) {
                    stage = stage.child(media);
                }
            }
            let stage = gpui::image_cache(self.images.clone()).child(stage);
            let key = current
                .as_ref()
                .map_or(0, |card| usize::try_from(card.id).unwrap_or(0));

            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(SPACE_SECTION))
                .child(fade_in(stage, ("stage", key), quick))
                .child(if phase == Phase::Question {
                    div()
                        .id("review-reveal")
                        .w(px(STAGE_WIDTH))
                        .h(px(CONTROL_HEIGHT))
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(SPACE_RELATED))
                        .rounded(px(18.))
                        .bg(colors.mauve)
                        .text_color(style.ink())
                        .text_size(px(17.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .cursor_pointer()
                        .focusable()
                        .border_1()
                        .border_color(alpha(colors.text, 0.0))
                        .hover(move |button| button.bg(accent_hover(theme)))
                        .active(move |button| button.bg(accent_press(theme)))
                        .focus(move |button| button.border_color(style.ink()))
                        .on_click(reveal)
                        .child("Show answer")
                        .child(keycap("Space", colors, true))
                        .into_any_element()
                } else {
                    rating_row(colors, &previews, cx).into_any_element()
                })
                .into_any_element()
        };
        screen = screen.child(body);

        let undo = self.action_click(UiAction::Undo);
        let help = self.action_click(UiAction::ToggleHelp);
        let add = self.action_click(UiAction::OpenAdd);
        screen.child(
            div()
                .h(px(FOOTER_HEIGHT))
                .flex_none()
                .flex()
                .items_center()
                .justify_between()
                .px(px(SPACE_GUTTER))
                .child(
                    div()
                        .flex()
                        .gap(px(SPACE_INLINE))
                        .child(footer_action(
                            "Undo",
                            "Ctrl Z",
                            Some("icon-undo"),
                            colors,
                            can_undo,
                            undo,
                        ))
                        .child(footer_action(
                            "Shortcuts",
                            "?",
                            None,
                            colors,
                            true,
                            help,
                        )),
                )
                .child(div().flex().gap(px(SPACE_INLINE)).child(
                    footer_action(
                        "Add card",
                        "A",
                        Some("icon-plus"),
                        colors,
                        true,
                        add,
                    ),
                )),
        )
    }
}

fn card_text_size(
    card: &card_view::RenderedCard,
    stage_height: f32,
    media: bool,
) -> f32 {
    let characters = card.text.chars().count();
    let lines = card.text.lines().count().max(1);
    let estimated_lines = characters.div_ceil(40).max(lines);
    let base = if media { 48.0 } else { 84.0 };
    let available = stage_height * if media { 0.42 } else { 0.68 };
    (available / (estimated_lines as f32 * 1.25))
        .min(base)
        .clamp(24.0, 84.0)
}

/// Headline for a finished session, sized by what actually happened.
fn summary_title(answered: usize) -> String {
    match answered {
        0 => String::from("Nothing due"),
        1 => String::from("One card reviewed"),
        _ => format!("{answered} cards reviewed"),
    }
}

fn summary_tile(
    label: &str,
    value: &str,
    hue: Rgba,
    colors: crate::theme::Palette,
) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(SPACE_INLINE))
        .w(px(160.))
        .p(px(SPACE_RELATED))
        .rounded(px(16.))
        .border_1()
        .border_color(colors.surface1)
        .bg(colors.mantle)
        .child(
            div()
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.subtext)
                .child(label.to_owned()),
        )
        .child(
            div()
                .text_size(px(22.))
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(hue)
                .child(value.to_owned()),
        )
}

fn separator(colors: crate::theme::Palette) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(SPACE_RELATED))
        .w_full()
        .my(px(SPACE_RELATED))
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
        let hover_fill = alpha(hue, 0.22);
        let hover_border = alpha(hue, 0.6);
        let press_fill = alpha(hue, 0.3);
        let press_border = alpha(hue, 0.8);
        row = row.child(
            div()
                .id(SharedString::from(format!("review-rating-{code}")))
                .flex_1()
                .h(px(CONTROL_HEIGHT))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(SPACE_INLINE))
                .rounded(px(18.))
                .bg(alpha(hue, 0.14))
                .border_1()
                .border_color(alpha(hue, 0.4))
                .cursor_pointer()
                .focusable()
                .hover(move |button| {
                    button.bg(hover_fill).border_color(hover_border)
                })
                .active(move |button| {
                    button.bg(press_fill).border_color(press_border)
                })
                .focus(move |button| button.border_color(hue))
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
                        .gap(px(SPACE_GROUP))
                        .text_size(px(16.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(label_color)
                        .child(rating_name(rating))
                        .child(
                            div()
                                .px(px(SPACE_GROUP))
                                .py(px(SPACE_INLINE))
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

/// Footer hint row. Disabled actions keep their key badge off and explain the
/// reason through their label, so a dead control never looks live.
fn footer_action(
    label: &'static str,
    key: &'static str,
    icon_name: Option<&'static str>,
    colors: crate::theme::Palette,
    enabled: bool,
    on_click: ClickHandler,
) -> gpui::Stateful<gpui::Div> {
    let ink = if enabled {
        colors.subtext
    } else {
        alpha(colors.subtext, 0.4)
    };
    let mut row = div()
        .id(SharedString::from(format!("footer-{label}")))
        .flex()
        .items_center()
        .gap(px(SPACE_GROUP))
        .h(px(36.))
        .px(px(SPACE_RELATED))
        .rounded(px(10.))
        .text_size(px(13.))
        .text_color(ink)
        .children(icon_name.map(|name| icon(name, 16., ink)))
        .child(label.to_owned())
        .children(enabled.then(|| keycap(key.to_owned(), colors, false)));
    if enabled {
        row = Interaction::surface(colors)
            .apply(row.cursor_pointer())
            .on_click(on_click);
    }
    row
}

/// One filled or outlined button for the session summary.
fn footer_button(
    label: &'static str,
    colors: crate::theme::Palette,
    primary: bool,
    on_click: ClickHandler,
) -> impl IntoElement {
    let theme = theme_from(colors);
    let (bg, ink) = if primary {
        (colors.mauve, crate::theme::accent_ink(theme))
    } else {
        (colors.mantle, colors.text)
    };
    let hover = if primary {
        accent_hover(theme)
    } else {
        colors.surface0
    };
    let border = if primary {
        colors.mauve
    } else {
        colors.surface1
    };
    let press = if primary {
        accent_press(theme)
    } else {
        colors.surface1
    };
    Interaction::new(
        border,
        hover,
        press,
        if primary {
            crate::theme::accent_ink(theme)
        } else {
            colors.mauve
        },
    )
    .apply(
        div()
            .id(SharedString::from(format!("summary-{label}")))
            .flex()
            .items_center()
            .justify_center()
            .h(px(44.))
            .px(px(SPACE_SECTION))
            .rounded(px(14.))
            .bg(bg)
            .text_color(ink)
            .text_size(px(15.))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .cursor_pointer(),
    )
    .on_click(on_click)
    .child(label)
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

    #[test]
    fn long_card_text_uses_a_smaller_reading_size() {
        let short = crate::card_view::RenderedCard {
            text: "Short question".into(),
            ..Default::default()
        };
        let long = crate::card_view::RenderedCard {
            text: "A long question ".repeat(40),
            ..Default::default()
        };
        assert!(
            super::card_text_size(&long, 322.0, false)
                < super::card_text_size(&short, 322.0, false)
        );
    }

    #[test]
    fn short_viewports_leave_room_for_grading_controls() {
        let stage =
            (600.0_f32 - 72.0 - 64.0 - 76.0 - 48.0 - 18.0).clamp(220.0, 440.0);
        assert!(stage <= 322.0);
        assert!(stage + 72.0 + 64.0 + 76.0 + 48.0 + 18.0 <= 600.0);
    }

    #[test]
    fn summary_titles_count_cards() {
        assert_eq!(super::summary_title(0), "Nothing due");
        assert_eq!(super::summary_title(1), "One card reviewed");
        assert_eq!(super::summary_title(12), "12 cards reviewed");
    }
}
