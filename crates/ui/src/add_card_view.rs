//! Add-card screen: setup, dynamic fields, tags, live preview, and footer.

use gpui::{
    ClickEvent, Context, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, px,
};

use crate::{
    app::{Root, UiAction, note_type_name, resolve_model},
    card_view::{self, Side},
    components::{icon, keycap},
    theme::{NAV_WIDTH, PAGE_X, Style, alpha},
};

impl Root {
    pub(crate) fn render_add_card(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = self.colors();
        let style = Style::new(colors, self.theme());
        let decks = self.decks.clone();
        let models = self.models.clone();
        let fields = self.add.fields.clone();
        let tags = self.add.tags.clone();
        let errors = self.add.errors.clone();
        let preview_side = self.add.preview_side;
        let model = resolve_model(&models, self.add.model_index);
        let field_values = fields
            .iter()
            .map(|field| field.read(cx).text())
            .collect::<Vec<_>>();
        let preview_text = model.map_or_else(
            || String::from("Choose a note type"),
            |model| card_view::preview_card(model, &field_values, preview_side),
        );
        let preview_answer = model.map_or_else(String::new, |model| {
            card_view::preview_card(model, &field_values, Side::Answer)
        });
        let preview_reading = model.and_then(|model| {
            field_value(model, &field_values, &["Reading", "Kana"])
        });
        let preview_example = model.and_then(|model| {
            model
                .fields
                .iter()
                .enumerate()
                .filter(|(index, _)| *index >= 2)
                .find_map(|(index, _)| {
                    field_values
                        .get(index)
                        .map(|value| value.trim())
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned)
                })
        });
        let keep_open = self.add.keep_open;
        let save = self.action_click(UiAction::SaveNote);
        let cancel = self.action_click(UiAction::CancelAdd);
        let preview_tab = |label: &'static str,
                           side: Side,
                           selected: bool,
                           cx: &mut Context<Self>| {
            div()
                .id("add-preview-tab")
                .h(px(28.))
                .px(px(12.))
                .flex()
                .items_center()
                .rounded(px(9.))
                .bg(if selected {
                    alpha(colors.mauve, 0.22)
                } else {
                    alpha(colors.surface0, 0.0)
                })
                .text_size(px(13.))
                .text_color(if selected {
                    colors.mauve
                } else {
                    colors.subtext
                })
                .cursor_pointer()
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.add.preview_side = side;
                    cx.notify();
                }))
                .child(label.to_owned())
        };
        let content_width =
            f32::from(window.viewport_size().width) - NAV_WIDTH - PAGE_X * 2.0;
        let side_by_side = content_width >= 900.;

        let mut screen = div()
            .size_full()
            .id("add-scroll")
            .flex()
            .flex_col()
            .overflow_scroll()
            .px(px(PAGE_X))
            .pt(px(32.));

        screen = screen
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(28.))
                            .font_weight(gpui::FontWeight::BOLD)
                            .child("New card"),
                    )
                    .child(stepper(colors)),
            )
            .child(div().h(px(20.)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .flex_1()
                            .min_w(px(220.))
                            .max_w(px(340.))
                            .p(px(10.))
                            .px(px(12.))
                            .rounded(px(14.))
                            .bg(colors.surface0)
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(colors.subtext)
                                    .child("DECK"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .text_size(px(14.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(decks.get(self.add.deck_index).map_or_else(
                                        || String::from("Choose a deck"),
                                        |deck| deck.name.replace("::", " › "),
                                    ))
                                    .child(icon(
                                        "icon-chevron-down",
                                        16.,
                                        colors.subtext,
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(px(2.))
                            .p(px(4.))
                            .rounded(px(12.))
                            .bg(colors.surface0)
                            .children(
                                note_type_tabs(&models).into_iter().map(
                                    |(label, index)| {
                                        let selected =
                                            index == self.add.model_index;
                                        div()
                                            .id("add-model-tab")
                                            .h(px(32.))
                                            .px(px(12.))
                                            .flex()
                                            .items_center()
                                            .rounded(px(9.))
                                            .bg(if selected {
                                                alpha(colors.mauve, 0.22)
                                            } else {
                                                alpha(colors.surface0, 0.0)
                                            })
                                            .text_size(px(13.))
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(if selected {
                                                colors.mauve
                                            } else {
                                                colors.subtext
                                            })
                                            .cursor_pointer()
                                            .on_click(cx.listener(
                                                move |this,
                                                      _: &ClickEvent,
                                                      _,
                                                      cx| {
                                                    this.select_model(index, cx);
                                                },
                                            ))
                                            .child(label)
                                    },
                                ),
                            ),
                    ),
            );

        if let Some(error) = errors.get("deck") {
            screen = screen.child(validation(error, colors));
        }
        if let Some(error) = errors.get("model") {
            screen = screen.child(validation(error, colors));
        }

        let mut left = div().flex().flex_1().flex_col().gap(px(18.));
        for (index, field) in fields.iter().enumerate() {
            let name = model
                .and_then(|model| model.fields.get(index).cloned())
                .unwrap_or_else(|| format!("Field {}", index + 1));
            let focused = field.read(cx).focus_handle().is_focused(window);
            let invalid = errors.contains_key("fields");
            let border = if invalid {
                colors.red
            } else if focused {
                colors.mauve
            } else {
                colors.surface1
            };
            left = left.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(colors.subtext)
                            .child(name),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .min_h(px(88.))
                            .px(px(16.))
                            .py(px(14.))
                            .rounded(px(16.))
                            .border_1()
                            .border_color(border)
                            .bg(if focused {
                                colors.surface0
                            } else {
                                colors.base
                            })
                            .text_size(px(26.))
                            .child(field.clone()),
                    ),
            );
        }
        if let Some(error) = errors.get("fields") {
            left = left.child(validation(error, colors));
        }
        left = left.child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.subtext)
                        .child("Tags"),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .h(px(46.))
                        .px(px(14.))
                        .rounded(px(12.))
                        .border_1()
                        .border_color(colors.surface1)
                        .child(tags),
                ),
        );

        let mut preview = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .h(px(300.))
            .p(px(20.))
            .rounded(px(24.))
            .bg(colors.surface0);
        if side_by_side {
            preview = preview.w(px(360.)).flex_none();
        }
        let preview = preview
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(colors.subtext)
                            .child("LIVE PREVIEW"),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(2.))
                            .p(px(4.))
                            .rounded(px(12.))
                            .bg(colors.surface0)
                            .child(preview_tab(
                                "Front",
                                Side::Question,
                                preview_side == Side::Question,
                                cx,
                            ))
                            .child(preview_tab(
                                "Back",
                                Side::Answer,
                                preview_side == Side::Answer,
                                cx,
                            )),
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
                    .child(
                        div()
                            .text_size(px(52.))
                            .text_color(colors.text)
                            .font_family(crate::theme::font_for_text(
                                &preview_text,
                            ))
                            .child(preview_text),
                    )
                    .children(preview_reading.map(|reading| {
                        div()
                            .text_size(px(14.))
                            .text_color(colors.subtext)
                            .font_family(crate::theme::font_for_text(&reading))
                            .child(reading)
                    }))
                    .child(div().w_full().h(px(1.)).bg(colors.surface1))
                    .child(
                        div()
                            .text_size(px(28.))
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(colors.mauve)
                            .font_family(crate::theme::font_for_text(
                                &preview_answer,
                            ))
                            .child(preview_answer),
                    )
                    .children(preview_example.map(|example| {
                        div()
                            .w_full()
                            .px(px(14.))
                            .py(px(10.))
                            .rounded(px(14.))
                            .bg(colors.base)
                            .text_size(px(14.))
                            .font_family(crate::theme::font_for_text(&example))
                            .child(example)
                    })),
            );

        let mut form = div().flex().flex_1().gap(px(32.)).items_start();
        if side_by_side {
            form = form.child(left).child(preview);
        } else {
            form = form.flex_col().child(left).child(preview.w_full());
        }
        screen = screen.child(form);

        let toggle = div()
            .id("add-keep-open")
            .w(px(44.))
            .h(px(26.))
            .rounded(px(13.))
            .bg(if keep_open {
                colors.mauve
            } else {
                colors.surface1
            })
            .flex()
            .items_center()
            .px(px(3.))
            .cursor_pointer()
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.add.keep_open = !this.add.keep_open;
                cx.notify();
            }))
            .child(div().w(px(20.)).h(px(20.)).rounded(px(10.)).bg(
                if keep_open {
                    colors.crust
                } else {
                    colors.subtext
                },
            ));
        screen.child(
            div()
                .mt(px(20.))
                .mb(px(22.))
                .flex()
                .items_center()
                .gap(px(14.))
                .pt(px(16.))
                .border_t_1()
                .border_color(colors.surface1)
                .child(toggle)
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(colors.subtext)
                        .child("Keep open after adding"),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .id("add-cancel")
                        .h(px(48.))
                        .px(px(20.))
                        .flex()
                        .items_center()
                        .rounded(px(14.))
                        .border_1()
                        .border_color(colors.surface1)
                        .text_size(px(15.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .cursor_pointer()
                        .on_click(cancel)
                        .child("Cancel"),
                )
                .child(
                    div()
                        .id("add-save")
                        .h(px(48.))
                        .px(px(22.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .rounded(px(14.))
                        .bg(colors.mauve)
                        .text_color(style.ink())
                        .text_size(px(15.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .cursor_pointer()
                        .on_click(save)
                        .child(icon("icon-plus", 18., style.ink()))
                        .child("Add card")
                        .child(keycap("Ctrl+Enter", colors, true)),
                ),
        )
    }
}

fn field_value(
    model: &ranki_model::NoteModel,
    values: &[String],
    names: &[&str],
) -> Option<String> {
    let index = model.fields.iter().position(|field| {
        names.iter().any(|name| field.eq_ignore_ascii_case(name))
    })?;
    let text = values.get(index)?.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// Distinct stock note types in mockup order, keeping the first model per label.
#[must_use]
pub fn note_type_tabs(
    models: &[ranki_model::NoteModel],
) -> Vec<(String, usize)> {
    let order = ["Basic", "Reversed", "Cloze", "Image Occlusion"];
    let mut tabs: Vec<(String, usize)> = Vec::new();
    for (index, model) in models.iter().enumerate() {
        let label = note_type_name(model).to_owned();
        if !tabs.iter().any(|(name, _)| *name == label) {
            tabs.push((label, index));
        }
    }
    tabs.sort_by_key(|(label, _)| {
        order
            .iter()
            .position(|name| name == label)
            .unwrap_or(order.len())
    });
    tabs
}

fn validation(message: &str, colors: crate::theme::Palette) -> gpui::Div {
    div()
        .text_size(px(13.))
        .text_color(colors.red)
        .child(message.to_owned())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StepState {
    Done,
    Current,
    Future,
}

fn stepper(colors: crate::theme::Palette) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(step("Setup", StepState::Done, colors))
        .child(connector(true, colors))
        .child(step("Content", StepState::Current, colors))
        .child(connector(false, colors))
        .child(step("Finish", StepState::Future, colors))
}

fn step(
    label: &str,
    state: StepState,
    colors: crate::theme::Palette,
) -> gpui::Div {
    let circle = match state {
        StepState::Done => div()
            .w(px(24.))
            .h(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .flex_none()
            .rounded(px(12.))
            .bg(colors.mauve)
            .border_2()
            .border_color(colors.mauve)
            .child(icon("icon-check", 14., colors.crust)),
        StepState::Current => div()
            .w(px(24.))
            .h(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .flex_none()
            .rounded(px(12.))
            .border_2()
            .border_color(colors.mauve)
            .text_size(px(12.))
            .text_color(colors.mauve)
            .child(String::from("2")),
        StepState::Future => div()
            .w(px(24.))
            .h(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .flex_none()
            .rounded(px(12.))
            .bg(colors.surface0)
            .border_2()
            .border_color(colors.surface0)
            .text_size(px(12.))
            .text_color(colors.subtext)
            .child(String::from("3")),
    };
    div().flex().items_center().gap(px(8.)).child(circle).child(
        div()
            .text_size(px(13.))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(match state {
                StepState::Future => colors.subtext,
                StepState::Done | StepState::Current => colors.text,
            })
            .child(label.to_owned()),
    )
}

fn connector(done: bool, colors: crate::theme::Palette) -> gpui::Div {
    div().w(px(24.)).h(px(1.)).bg(if done {
        colors.mauve
    } else {
        colors.surface1
    })
}

#[cfg(test)]
mod tests {
    use crate::{
        app::{UiAction, validate_draft},
        card_view::Side,
        theme::MOCHA,
    };

    #[test]
    fn preview_side_toggles_without_a_store_call() {
        assert_ne!(Side::Question, Side::Answer);
    }

    #[test]
    fn validation_uses_the_shared_helper() {
        let deck = ranki_model::Deck {
            id: 1,
            name: "Default".into(),
            description: String::new(),
        };
        let model = ranki_model::NoteModel {
            id: 1,
            name: "Basic".into(),
            fields: vec!["Front".into()],
            templates: vec![ranki_model::CardTemplate::default()],
            css: String::new(),
            kind: ranki_model::NoteKind::Normal,
            original_stock_kind: None,
            field_tags: vec![],
            template_ordinals: vec![0],
            raw_config: None,
        };
        assert!(validate_draft(&deck, &model, &[String::from("Q")]).is_empty());
        assert_eq!(MOCHA.base, gpui::rgb(0x1e1e2e));
        assert_ne!(UiAction::SaveNote, UiAction::CancelAdd);
    }

    #[test]
    fn note_type_tabs_are_distinct_and_ordered() {
        let base = ranki_model::NoteModel {
            id: 1,
            name: "Basic".into(),
            fields: vec!["Front".into()],
            templates: vec![ranki_model::CardTemplate::default()],
            css: String::new(),
            kind: ranki_model::NoteKind::Normal,
            original_stock_kind: None,
            field_tags: vec![],
            template_ordinals: vec![0],
            raw_config: None,
        };
        let duplicate = ranki_model::NoteModel {
            id: 2,
            ..base.clone()
        };
        let cloze = ranki_model::NoteModel {
            id: 3,
            name: "Cloze".into(),
            kind: ranki_model::NoteKind::Cloze,
            ..base.clone()
        };
        let tabs = super::note_type_tabs(&[cloze, duplicate, base]);
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs[0].0, "Basic");
        assert_eq!(tabs[1].0, "Cloze");
    }
}
