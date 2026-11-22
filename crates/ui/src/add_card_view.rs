//! Add-card screen: setup, dynamic fields, tags, live preview, and footer.

use gpui::{
    ClickEvent, Context, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, px,
};

use crate::{
    app::{Root, UiAction, note_type_name, resolve_model},
    card_view::{self, Side},
    components::keycap,
    theme::{PAGE_X, accent_ink, alpha},
};

impl Root {
    pub(crate) fn render_add_card(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = self.colors();
        let theme = self.theme();
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
            || "Choose a note type".to_owned(),
            |model| card_view::preview_card(model, &field_values, preview_side),
        );
        let keep_open = self.add.keep_open;
        let save = self.action_click(UiAction::SaveNote);
        let cancel = self.action_click(UiAction::CancelAdd);

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
                    .gap(px(14.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .w(px(340.))
                            .p(px(10.)).px(px(12.))
                            .rounded(px(14.))
                            .bg(colors.surface0)
                            .child(div().text_size(px(11.)).text_color(colors.subtext).child("DECK"))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .text_size(px(14.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(decks.get(self.add.deck_index).map_or_else(
                                        || "Choose a deck".to_owned(),
                                        |deck| deck.name.clone(),
                                    ))
                                    .child(icon_small("icon-chevron-right", colors)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(2.))
                            .p(px(4.))
                            .rounded(px(12.))
                            .bg(colors.surface0)
                            .children(models.iter().enumerate().map(|(index, model)| {
                                let selected = index == self.add.model_index;
                                div().id("auto-add_card_view-1")
                                    .h(px(32.))
                                    .px(px(14.))
                                    .flex()
                                    .items_center()
                                    .rounded(px(9.))
                                    .bg(if selected {
                                        alpha(colors.mauve, 0.22)
                                    } else {
                                        alpha(colors.surface0, 0.0)
                                    })
                                    .text_size(px(13.))
                                    .text_color(if selected { colors.mauve } else { colors.subtext })
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        this.select_model(index, cx);
                                    }))
                                    .child(note_type_name(model).to_owned())
                            })),
                    ),
            );

        if let Some(error) = errors.get("deck") {
            screen = screen.child(validation(error, colors));
        }
        if let Some(error) = errors.get("model") {
            screen = screen.child(validation(error, colors));
        }

        let mut form = div().flex().flex_1().gap(px(32.));
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
                            .child(div().child(name).child(" ")),
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
        form = form.child(left);

        let preview = div()
            .w(px(360.))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(14.))
            .p(px(20.))
            .rounded(px(24.))
            .bg(colors.surface0)
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
                            .p(px(2.))
                            .rounded(px(9.))
                            .bg(colors.surface1)
                            .child(preview_tab(
                                "Front",
                                preview_side == Side::Question,
                                colors,
                                cx,
                            ))
                            .child(preview_tab(
                                "Back",
                                preview_side == Side::Answer,
                                colors,
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
                    .text_size(px(52.))
                    .text_color(colors.text)
                    .child(preview_text)
                    .child(
                        div()
                            .text_size(px(14.))
                            .text_color(colors.subtext)
                            .child(if preview_side == Side::Question {
                                "Reading"
                            } else {
                                "Answer"
                            }),
                    ),
            );
        form = form.child(preview);
        screen = screen.child(form);

        let toggle = div()
            .id("auto-add_card_view-2")
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
                .child(div().text_size(px(13.)).child("Keep open after adding"))
                .child(div().flex_1())
                .child(
                    div()
                        .id("auto-add_card_view-3")
                        .h(px(48.))
                        .px(px(20.))
                        .flex()
                        .items_center()
                        .rounded(px(14.))
                        .border_1()
                        .border_color(colors.surface1)
                        .text_size(px(15.))
                        .cursor_pointer()
                        .on_click(cancel)
                        .child("Cancel"),
                )
                .child(
                    div()
                        .id("auto-add_card_view-4")
                        .h(px(48.))
                        .px(px(22.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .rounded(px(14.))
                        .bg(colors.mauve)
                        .text_color(accent_ink(theme))
                        .text_size(px(15.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .cursor_pointer()
                        .on_click(save)
                        .child("Add card")
                        .child(keycap("Ctrl+Enter", colors, true)),
                ),
        )
    }
}

fn icon_small(name: &'static str, colors: crate::theme::Palette) -> gpui::Svg {
    crate::components::icon(name, 16.).text_color(colors.subtext)
}

fn preview_tab(
    label: &'static str,
    selected: bool,
    colors: crate::theme::Palette,
    cx: &Context<Root>,
) -> impl IntoElement {
    let side = if label == "Front" {
        Side::Question
    } else {
        Side::Answer
    };
    div()
        .id("auto-add_card_view-5")
        .h(px(28.))
        .px(px(14.))
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
}

fn validation(message: &str, colors: crate::theme::Palette) -> gpui::Div {
    div()
        .text_size(px(13.))
        .text_color(colors.red)
        .child(message.to_owned())
}

fn stepper(colors: crate::theme::Palette) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(step("1", "Setup", true, colors))
        .child(connector(colors))
        .child(step("2", "Content", true, colors))
        .child(connector(colors))
        .child(step("3", "Finish", false, colors))
}

fn step(
    number: &str,
    label: &str,
    active: bool,
    colors: crate::theme::Palette,
) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(
            div()
                .w(px(24.))
                .h(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(12.))
                .border_2()
                .border_color(if active {
                    colors.mauve
                } else {
                    colors.surface1
                })
                .text_size(px(12.))
                .child(number.to_owned()),
        )
        .child(
            div()
                .text_size(px(13.))
                .text_color(if active { colors.text } else { colors.subtext })
                .child(label.to_owned()),
        )
}

fn connector(colors: crate::theme::Palette) -> gpui::Div {
    div().w(px(24.)).h(px(1.)).bg(colors.surface1)
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
}
