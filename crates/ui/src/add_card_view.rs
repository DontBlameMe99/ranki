//! Add-card screen: setup, dynamic fields, tags, live preview, and footer.

use std::collections::BTreeMap;

use gpui::{
    App, ClickEvent, Context, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels,
    SharedString, StatefulInteractiveElement, Styled, StyledImage, Window, div,
    img, px, relative,
};

use crate::{
    app::{AddedNote, Root, UiAction, note_type_name, resolve_model},
    card_view::{self, Side},
    components::{ClickHandler, icon, keycap, removable_chip},
    occlusion::{self, OcclusionMask},
    theme::{
        ADD_CONTENT_WIDTH, ADD_FORM_WIDTH, ADD_GAP, NAV_WIDTH, PAGE_X,
        PREVIEW_WIDTH, Style, alpha,
    },
};

/// Height of the occlusion drawing area.
const OCCLUSION_CANVAS_HEIGHT: f32 = 320.0;

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
        let tags_field = self.add.tags.clone();
        let tag_chips = self.add.tag_chips.clone();
        let added = self.add.added.clone();
        let errors = self.add.errors.clone();
        let preview_side = self.add.preview_side;
        let model = resolve_model(&models, self.add.model_index);
        let occlusion_mode =
            model.is_some_and(ranki_model::NoteModel::is_image_occlusion);
        let field_values = if occlusion_mode {
            self.occlusion_fields(cx)
        } else {
            fields
                .iter()
                .map(|field| field.read(cx).text())
                .collect::<Vec<_>>()
        };
        let has_content = has_preview_content(&field_values);
        let preview_text = model.map_or_else(
            || String::from("Choose a note type"),
            |model| card_view::preview_card(model, &field_values, preview_side),
        );
        let preview_answer = model.map_or_else(String::new, |model| {
            card_view::preview_card(model, &field_values, Side::Answer)
        });
        let rendered_preview = model.map(|model| {
            card_view::preview_rendered(model, &field_values, preview_side)
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
                        .map(ToOwned::to_owned)
                })
        });
        let save = self.action_click(UiAction::SaveNote);
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

        let screen = div()
            .size_full()
            .id("add-scroll")
            .flex()
            .flex_col()
            .overflow_scroll()
            .px(px(PAGE_X))
            .pt(px(32.));
        let mut column = div()
            .flex()
            .flex_col()
            .w_full()
            .max_w(px(ADD_CONTENT_WIDTH));

        column = column
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .text_size(px(28.))
                            .font_weight(gpui::FontWeight::BOLD)
                            .child("New card"),
                    )
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
                                    .id("add-deck-picker")
                                    .cursor_pointer()
                                    .on_click(cx.listener(
                                        |this, _: &ClickEvent, window, cx| {
                                            this.toggle_add_deck_picker(window, cx);
                                        },
                                    ))
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .text_size(px(14.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(self.add_deck_index().and_then(|index| decks.get(index)).map_or_else(
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
                                            .id(SharedString::from(format!(
                                                "add-model-tab-{index}"
                                            )))
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
        if self.add.deck_picker {
            column = column.child(self.render_deck_picker(colors, cx));
        }

        if let Some(error) = errors.get("deck") {
            column = column.child(validation(error, colors));
        }
        if let Some(error) = errors.get("model") {
            column = column.child(validation(error, colors));
        }

        let occlusion_mode =
            model.is_some_and(ranki_model::NoteModel::is_image_occlusion);
        let is_cloze = model
            .is_some_and(|model| model.kind == ranki_model::NoteKind::Cloze);
        let mut left =
            div().flex().flex_col().w_full().gap(px(18.)).min_w(px(0.));
        if occlusion_mode {
            let editor = self.render_occlusion_editor(colors, &errors, cx);
            left = left.child(editor);
        }
        for (index, field) in fields
            .iter()
            .enumerate()
            .filter(|(index, _)| !occlusion_mode || *index >= 2)
        {
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
            let mut field_box = div()
                .flex()
                .flex_col()
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
                .child(
                    div()
                        .min_h(px(56.))
                        .text_size(px(26.))
                        .child(field.clone()),
                );
            if index == 0 {
                let cloze = is_cloze.then(|| {
                    let selection = field.read(cx).selection();
                    cloze_insert_actions(&field_values, selection.start)
                });
                field_box = field_box.child(formatting_toolbar(
                    field.clone(),
                    cloze,
                    colors,
                    cx,
                ));
            }
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
                    .child(field_box),
            );
        }
        if let Some(error) = errors.get("fields") {
            left = left.child(validation(error, colors));
        }
        let tags_handle = tags_field.read(cx).focus_handle().clone();
        let mut tags_row = div()
            .id("add-tags")
            .flex()
            .items_center()
            .flex_wrap()
            .gap(px(8.))
            .min_h(px(46.))
            .px(px(12.))
            .py(px(8.))
            .rounded(px(12.))
            .border_1()
            .border_color(colors.surface1);
        let weak = cx.entity().downgrade();
        for tag in &tag_chips {
            let tag = tag.clone();
            let handle = tags_handle.clone();
            let weak = weak.clone();
            tags_row = tags_row.child(removable_chip(
                tag.clone(),
                colors,
                Box::new(move |_, window, cx: &mut App| {
                    let tag = tag.clone();
                    let _ = weak.update(cx, |root, cx| {
                        root.add.tag_chips.retain(|known| *known != tag);
                        cx.notify();
                    });
                    window.focus(&handle);
                }),
            ));
        }
        let tag_input = tags_field;
        let click_handle = tags_handle;
        let tags_group = div()
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
                tags_row
                    .on_click(cx.listener(
                        move |_, _: &ClickEvent, window, _| {
                            window.focus(&click_handle);
                        },
                    ))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .min_w(px(140.))
                            .child(icon("icon-tag", 14., colors.mauve))
                            .child(
                                div().flex_1().min_w(px(80.)).child(tag_input),
                            ),
                    ),
            );

        let mut preview = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .p(px(20.))
            .min_h(px(300.))
            .rounded(px(24.))
            .bg(colors.surface0);
        if side_by_side {
            preview = preview.w(px(PREVIEW_WIDTH)).flex_none();
        } else {
            preview = preview.w_full();
        }
        let mut preview_header =
            div().flex().items_center().justify_between().child(
                div()
                    .text_size(px(11.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(colors.subtext)
                    .child("LIVE PREVIEW"),
            );
        if has_content {
            preview_header = preview_header.child(
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
            );
        }
        let mut preview = preview.child(preview_header).child(if has_content {
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
                        .font_family(crate::theme::font_for_text(&preview_text))
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
                }))
                .into_any_element()
        } else {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .min_h(px(160.))
                .child(
                    div()
                        .text_size(px(14.))
                        .text_color(colors.overlay)
                        .child("Preview appears as you type"),
                )
                .into_any_element()
        });
        if let Some(name) = self.add.occlusion.image.clone() {
            let mut image = div().relative().child(
                img(self.options.media_dir.join(name))
                    .h(px(120.))
                    .w_full()
                    .object_fit(gpui::ObjectFit::Contain),
            );
            if let Some(rendered) = rendered_preview.as_ref()
                && !rendered.masks.is_empty()
            {
                image = image.child(card_view::occlusion_layer(
                    &rendered.masks,
                    colors,
                    preview_side == Side::Answer,
                ));
            }
            preview = preview.child(image);
        }

        let history = added_history(&added, colors);

        let mut form = div().flex().w_full().gap(px(ADD_GAP));
        if occlusion_mode {
            form = form
                .flex_col()
                .gap(px(18.))
                .child(left)
                .child(tags_group)
                .child(history)
                .child(preview);
            column = column.child(form);
        } else if side_by_side {
            form = form
                .items_start()
                .child(
                    div().flex_1().min_w(px(ADD_FORM_WIDTH * 0.5)).child(left),
                )
                .child(div().w(px(PREVIEW_WIDTH)).flex_none().child(preview));
            let tags_and_history = div()
                .mt(px(18.))
                .flex()
                .w_full()
                .items_start()
                .gap(px(ADD_GAP))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(ADD_FORM_WIDTH * 0.5))
                        .child(tags_group),
                )
                .child(div().w(px(PREVIEW_WIDTH)).flex_none().child(history));
            column = column.child(form).child(tags_and_history);
        } else {
            form = form
                .flex_col()
                .gap(px(18.))
                .child(left)
                .child(tags_group)
                .child(preview)
                .child(history);
            column = column.child(form);
        }

        column = column.child(
            div()
                .mt(px(20.))
                .mb(px(22.))
                .flex()
                .items_center()
                .gap(px(14.))
                .pt(px(16.))
                .border_t_1()
                .border_color(colors.surface1)
                .child(div().flex_1())
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
        );

        screen.child(div().flex().w_full().justify_center().child(column))
    }

    /// Image-occlusion editor: upload control, drawing area, and mask list.
    #[expect(
        clippy::too_many_arguments,
        reason = "one editor needs the palette, its errors, and the context"
    )]
    fn render_occlusion_editor(
        &mut self,
        colors: crate::theme::Palette,
        errors: &BTreeMap<String, String>,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let image = self.add.occlusion.image.clone();
        let masks = self.add.occlusion.masks.clone();
        let selected = self.add.occlusion.selected;
        let dragging = self.add.occlusion.drag_rect();
        let media_dir = self.options.media_dir.clone();

        let button = |id: &'static str,
                      label: &'static str,
                      icon_name: &'static str,
                      enabled: bool,
                      colors: crate::theme::Palette,
                      on_click: ClickHandler| {
            let mut element = div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(6.))
                .h(px(32.))
                .px(px(12.))
                .rounded(px(10.))
                .border_1()
                .border_color(colors.surface1)
                .text_size(px(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(if enabled { colors.text } else { colors.overlay })
                .child(icon(
                    icon_name,
                    14.,
                    if enabled {
                        colors.subtext
                    } else {
                        colors.overlay
                    },
                ))
                .child(label);
            if enabled {
                element = element.cursor_pointer().on_click(on_click);
            }
            element
        };

        let add_mask = {
            let handler: ClickHandler =
                Box::new(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.add_default_mask(cx);
                }));
            button(
                "io-add-mask",
                "Add rectangle",
                "icon-plus",
                true,
                colors,
                handler,
            )
        };
        let delete_mask = {
            let handler: ClickHandler =
                Box::new(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.remove_selected_mask(cx);
                }));
            button(
                "io-delete-mask",
                "Delete mask",
                "icon-trash",
                selected.is_some_and(|index| index < masks.len()),
                colors,
                handler,
            )
        };
        let clear_masks = {
            let handler: ClickHandler =
                Box::new(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.clear_occlusion_masks(cx);
                }));
            button(
                "io-clear-masks",
                "Clear all",
                "icon-x",
                !masks.is_empty(),
                colors,
                handler,
            )
        };

        let mut canvas = div()
            .id("io-canvas")
            .relative()
            .w_full()
            .h(px(OCCLUSION_CANVAS_HEIGHT))
            .rounded(px(16.))
            .overflow_hidden()
            .border_1()
            .border_color(if errors.contains_key("masks") {
                colors.red
            } else {
                colors.surface1
            })
            .bg(colors.mantle);
        if image.is_none() {
            canvas = canvas.cursor_pointer();
        }
        if let Some(name) = &image {
            canvas = canvas.child(
                img(media_dir.join(name))
                    .size_full()
                    .object_fit(gpui::ObjectFit::Contain),
            );
        } else {
            canvas = canvas.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(13.))
                    .text_color(colors.overlay)
                    .child("Upload an image, then drag to draw masks"),
            );
        }
        let mut overlay = div().absolute().inset_0().id("io-overlay");
        for (index, mask) in masks.iter().enumerate() {
            overlay = overlay.child(mask_box(
                mask,
                index.saturating_add(1),
                selected == Some(index),
                colors,
            ));
        }
        if let Some(rect) = dragging {
            overlay = overlay.child(
                div()
                    .absolute()
                    .left(relative(rect.left))
                    .top(relative(rect.top))
                    .w(relative(rect.width))
                    .h(relative(rect.height))
                    .border_1()
                    .border_dashed()
                    .border_color(colors.mauve),
            );
        }
        canvas = canvas
            .child(overlay)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    if this.add.occlusion.image.is_none() {
                        this.upload_occlusion_image(cx);
                    } else {
                        this.occlusion_press(pointer(event.position), cx);
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.upload_occlusion_image(cx);
                }),
            )
            .on_mouse_move(cx.listener(
                move |this, event: &MouseMoveEvent, _, cx| {
                    this.occlusion_drag_to(pointer(event.position), cx);
                },
            ))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.occlusion_release(cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.cancel_occlusion_drag(cx);
                }),
            );

        let weak = cx.entity().downgrade();
        let canvas = div()
            .w_full()
            .on_children_prepainted(move |children, _, cx| {
                if let Some(first) = children.first() {
                    let _ = weak.update(cx, |root, _| {
                        root.set_occlusion_bounds(*first);
                    });
                }
            })
            .child(canvas);

        let mut list = div().flex().flex_col().gap(px(4.));
        if masks.is_empty() {
            list = list.child(
                div()
                    .text_size(px(12.))
                    .text_color(colors.overlay)
                    .child("No masks yet"),
            );
        }
        for (index, mask) in masks.iter().enumerate() {
            let active = selected == Some(index);
            let ordinal = index.saturating_add(1);
            list = list.child(
                div()
                    .id(SharedString::from(format!("io-mask-{index}")))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .h(px(32.))
                    .px(px(10.))
                    .rounded(px(9.))
                    .bg(if active {
                        alpha(colors.mauve, 0.16)
                    } else {
                        colors.base
                    })
                    .text_size(px(13.))
                    .text_color(if active {
                        colors.mauve
                    } else {
                        colors.subtext
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.select_occlusion_mask(index, cx);
                        },
                    ))
                    .child(format!("c{ordinal}"))
                    .child(mask.shape.label())
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(colors.overlay)
                            .child(format!(
                                "{}, {} · {} × {}",
                                occlusion::ratio(mask.rect.left),
                                occlusion::ratio(mask.rect.top),
                                occlusion::ratio(mask.rect.width),
                                occlusion::ratio(mask.rect.height),
                            )),
                    ),
            );
        }

        let mut editor = div()
            .flex()
            .flex_col()
            .w_full()
            .gap(px(12.))
            .p(px(16.))
            .rounded(px(18.))
            .border_1()
            .border_color(colors.surface1)
            .bg(colors.surface0)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(colors.subtext)
                            .child("OCCLUSION EDITOR"),
                    )
                    .child(div().flex_1())
                    .child(add_mask)
                    .child(delete_mask)
                    .child(clear_masks),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child({
                        let label = div()
                            .id("io-upload-image")
                            .flex_1()
                            .text_size(px(12.))
                            .text_color(if image.is_some() {
                                colors.subtext
                            } else {
                                colors.mauve
                            })
                            .child(image.clone().unwrap_or_else(|| {
                                String::from("Upload an image")
                            }));
                        if image.is_none() {
                            label.cursor_pointer().on_click(cx.listener(
                                |this, _: &ClickEvent, _, cx| {
                                    this.upload_occlusion_image(cx);
                                },
                            ))
                        } else {
                            label
                        }
                    })
                    .child(if image.is_some() {
                        div()
                            .text_size(px(11.))
                            .text_color(colors.overlay)
                            .child("Right-click image to change")
                    } else {
                        div()
                    }),
            )
            .child(canvas)
            .child(list);
        if let Some(error) = errors.get("occlusion") {
            editor = editor.child(validation(error, colors));
        }
        if let Some(error) = errors.get("masks") {
            editor = editor.child(validation(error, colors));
        }
        editor
    }
}
impl Root {
    fn render_deck_picker(
        &mut self,
        colors: crate::theme::Palette,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let query = self.add.deck_filter.read(cx).text();
        let needle = query.trim().to_lowercase();
        let filter = self.add.deck_filter.clone();
        let mut list = div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .max_h(px(220.))
            .id("add-deck-list")
            .overflow_scroll();
        let mut matches = 0;
        for (index, deck) in self.decks.iter().enumerate() {
            if !needle.is_empty() && !deck.name.to_lowercase().contains(&needle)
            {
                continue;
            }
            matches += 1;
            let selected = self.add.deck_id == Some(deck.id);
            let due = self.stats.get(index).map_or(0, |stats| {
                stats
                    .new_cards
                    .saturating_add(stats.learning_cards)
                    .saturating_add(stats.review_cards)
            });
            list = list.child(
                div()
                    .id(SharedString::from(format!("add-deck-{index}")))
                    .flex()
                    .items_center()
                    .justify_between()
                    .h(px(38.))
                    .flex_none()
                    .px(px(12.))
                    .rounded(px(10.))
                    .bg(if selected {
                        alpha(colors.mauve, 0.18)
                    } else {
                        colors.base
                    })
                    .text_color(if selected {
                        colors.mauve
                    } else {
                        colors.text
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, window, cx| {
                            this.select_add_deck(index, cx);
                            window.focus(&this.focus);
                        },
                    ))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(icon("icon-book", 16., colors.subtext))
                            .child(deck.name.replace("::", " › ")),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(colors.subtext)
                            .child(format!("{due} due")),
                    ),
            );
        }
        if matches == 0 {
            list = list.child(
                div()
                    .p(px(12.))
                    .text_size(px(13.))
                    .text_color(colors.overlay)
                    .child("No decks match"),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(12.))
            .rounded(px(16.))
            .border_1()
            .border_color(colors.surface1)
            .bg(colors.mantle)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(38.))
                    .px(px(10.))
                    .rounded(px(10.))
                    .bg(colors.base)
                    .child(icon("icon-search", 15., colors.subtext))
                    .child(div().flex_1().min_w(px(120.)).child(filter)),
            )
            .child(list)
    }
}

/// One drawn mask, translucent red with its cloze ordinal.
#[expect(
    clippy::too_many_arguments,
    reason = "the mask, its ordinal, selection, and palette are one box"
)]
fn mask_box(
    mask: &OcclusionMask,
    ordinal: usize,
    selected: bool,
    colors: crate::theme::Palette,
) -> gpui::Div {
    let mut shape = div()
        .absolute()
        .left(relative(mask.rect.left))
        .top(relative(mask.rect.top))
        .w(relative(mask.rect.width))
        .h(relative(mask.rect.height))
        .flex()
        .items_center()
        .justify_center()
        .bg(alpha(colors.red, if selected { 0.45 } else { 0.28 }))
        .border_1()
        .border_color(if selected { colors.mauve } else { colors.red })
        .text_size(px(11.))
        .text_color(colors.crust)
        .child(format!("c{ordinal}"));
    shape = if mask.shape == crate::occlusion::MaskShape::Ellipse {
        shape.rounded_full()
    } else {
        shape.rounded(px(4.))
    };
    shape
}

fn pointer(position: gpui::Point<Pixels>) -> (f32, f32) {
    (f32::from(position.x), f32::from(position.y))
}

/// True when any field carries text, which gates the preview side tabs.
#[must_use]
pub fn has_preview_content(values: &[String]) -> bool {
    values.iter().any(|value| !value.trim().is_empty())
}

/// Cloze marker numbers the field toolbar can insert.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ClozeActions {
    next: u32,
    linked: Option<u32>,
}

/// Next free cloze number, plus the number linked to the caret position.
#[must_use]
fn cloze_insert_actions(values: &[String], caret: usize) -> ClozeActions {
    let mut highest = 0;
    let mut first = None;
    for value in values {
        for marker in ranki_model::cloze_markers(value) {
            for number in marker.numbers {
                if number == 0 {
                    continue;
                }
                highest = highest.max(number);
                let _ = first.get_or_insert(number);
            }
        }
    }
    let max_number = u32::from(ranki_model::cloze::MAX_CLOZE_ORDINAL) + 1;
    ClozeActions {
        next: highest.saturating_add(1).min(max_number),
        linked: nearest_cloze_number(
            values.first().map_or("", String::as_str),
            caret,
        )
        .or(first),
    }
}

fn nearest_cloze_number(text: &str, caret: usize) -> Option<u32> {
    fn positive(marker: &ranki_model::cloze::ClozeMarker<'_>) -> Option<u32> {
        marker.numbers.iter().copied().find(|number| *number > 0)
    }

    fn distance(
        marker: &ranki_model::cloze::ClozeMarker<'_>,
        caret: usize,
    ) -> usize {
        if caret < marker.start {
            marker.start.saturating_sub(caret)
        } else {
            caret.saturating_sub(marker.end)
        }
    }

    ranki_model::cloze_markers(text)
        .into_iter()
        .filter(|marker| positive(marker).is_some())
        .min_by_key(|marker| {
            let distance = distance(marker, caret);
            let order = if distance == 0 {
                usize::MAX.saturating_sub(marker.start)
            } else {
                marker.start
            };
            (distance, order)
        })
        .and_then(|marker| positive(&marker))
}

fn cloze_tags(number: u32) -> (String, String) {
    (format!("{{{{c{number}::"), String::from("}}"))
}

/// Field formatting bar plus the keyboard hint, under the first field.
fn formatting_toolbar(
    field: gpui::Entity<crate::text_field::TextField>,
    cloze_actions: Option<ClozeActions>,
    colors: crate::theme::Palette,
    cx: &mut Context<Root>,
) -> impl IntoElement {
    let handle = field.read(cx).focus_handle().clone();
    let wrap = |tags: (&'static str, &'static str),
                field: gpui::Entity<crate::text_field::TextField>,
                handle: gpui::FocusHandle| {
        let handler: ClickHandler = Box::new(
            move |_event: &ClickEvent, window: &mut Window, cx: &mut App| {
                field.update(cx, |field, cx| {
                    field.wrap_markup(tags, cx);
                });
                window.focus(&handle);
            },
        );
        handler
    };
    let wrap_cloze = |number: u32,
                      field: gpui::Entity<crate::text_field::TextField>,
                      handle: gpui::FocusHandle| {
        let (open, close) = cloze_tags(number);
        let handler: ClickHandler = Box::new(
            move |_event: &ClickEvent, window: &mut Window, cx: &mut App| {
                field.update(cx, |field, cx| {
                    field.wrap_markup((&open, &close), cx);
                });
                window.focus(&handle);
            },
        );
        handler
    };
    let button = |label: &'static str, bold: bool, on_click: ClickHandler| {
        let mut button = div()
            .id("add-format")
            .flex()
            .items_center()
            .justify_center()
            .w(px(30.))
            .h(px(30.))
            .rounded(px(8.))
            .text_size(px(14.))
            .text_color(colors.subtext)
            .cursor_pointer()
            .on_click(on_click);
        button = if bold {
            button.font_weight(gpui::FontWeight::BOLD)
        } else {
            button.font_weight(gpui::FontWeight::MEDIUM)
        };
        button.child(label)
    };
    let cloze_button = |label: String,
                        number: u32,
                        field: gpui::Entity<crate::text_field::TextField>,
                        handle: gpui::FocusHandle| {
        div()
            .id(SharedString::from(format!("add-cloze-{label}")))
            .flex()
            .items_center()
            .h(px(30.))
            .px(px(10.))
            .rounded(px(8.))
            .text_size(px(13.))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(colors.mauve)
            .cursor_pointer()
            .on_click(wrap_cloze(number, field, handle))
            .child(label)
    };
    let mut toolbar = div()
        .mt(px(10.))
        .pt(px(8.))
        .border_t_1()
        .border_color(colors.surface1)
        .flex()
        .items_center()
        .gap(px(2.))
        .child(button(
            "B",
            true,
            wrap(("<b>", "</b>"), field.clone(), handle.clone()),
        ))
        .child(button(
            "I",
            false,
            wrap(("<i>", "</i>"), field.clone(), handle.clone()),
        ));
    if let Some(actions) = cloze_actions {
        toolbar = toolbar.child(cloze_button(
            "[…] New".to_owned(),
            actions.next,
            field.clone(),
            handle.clone(),
        ));
        if let Some(number) = actions.linked {
            toolbar = toolbar.child(cloze_button(
                format!("[…] Link c{number}"),
                number,
                field,
                handle,
            ));
        }
    }
    toolbar.child(div().flex_1()).child(
        div()
            .text_size(px(12.))
            .text_color(colors.overlay)
            .child("Tab for next field"),
    )
}

fn added_history(
    added: &[AddedNote],
    colors: crate::theme::Palette,
) -> impl IntoElement {
    let mut card = div()
        .flex()
        .flex_col()
        .rounded(px(16.))
        .bg(colors.mantle)
        .border_1()
        .border_color(colors.surface0)
        .min_h(px(46.))
        .overflow_hidden();
    if added.is_empty() {
        card = card.child(
            div()
                .px(px(14.))
                .py(px(11.))
                .text_size(px(13.))
                .text_color(colors.overlay)
                .child("No cards added yet"),
        );
    } else {
        for note in added.iter().rev() {
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(14.))
                    .py(px(11.))
                    .child(icon("icon-check", 16., colors.green))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w(px(0.))
                            .child(
                                div()
                                    .text_size(px(15.))
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .font_family(crate::theme::font_for_text(
                                        &note.headline,
                                    ))
                                    .child(note.headline.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(colors.subtext)
                                    .child(note.detail.clone()),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(colors.overlay)
                            .child(note.time.clone()),
                    ),
            );
        }
    }
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(
            div()
                .text_size(px(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.subtext)
                .child("Added this session"),
        )
        .child(card)
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

/// One tab per note kind, preferring installed genanki models.
#[must_use]
pub fn note_type_tabs(
    models: &[ranki_model::NoteModel],
) -> Vec<(String, usize)> {
    let order = ["Basic", "Reversed", "Cloze", "Image Occlusion"];
    let mut tabs: Vec<(String, usize)> = Vec::new();
    for (index, model) in models.iter().enumerate() {
        let label = note_type_name(model);
        match tabs.iter_mut().find(|(name, _)| *name == label) {
            Some((_, selected))
                if tab_rank(model) < tab_rank(&models[*selected]) =>
            {
                *selected = index;
            }
            Some(_) => {}
            None => tabs.push((label, index)),
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

#[must_use]
pub fn preferred_model_index(
    models: &[ranki_model::NoteModel],
    index: usize,
) -> Option<usize> {
    let label = note_type_name(models.get(index)?);
    note_type_tabs(models)
        .into_iter()
        .find(|(tab_label, _)| *tab_label == label)
        .map(|(_, tab_index)| tab_index)
}

#[must_use]
fn tab_rank(model: &ranki_model::NoteModel) -> (bool, bool) {
    (
        model.id == 0,
        !model.name.to_ascii_lowercase().contains("genanki"),
    )
}

fn validation(message: &str, colors: crate::theme::Palette) -> gpui::Div {
    div()
        .text_size(px(13.))
        .text_color(colors.red)
        .child(message.to_owned())
}

#[cfg(test)]
mod tests {
    use crate::{
        app::{UiAction, models_with_stock},
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
        assert!(
            crate::app::validate_draft(&deck, &model, &[String::from("Q")])
                .is_empty()
        );
        assert_eq!(MOCHA.base, gpui::rgb(0x1e1e2e));
        assert_ne!(UiAction::SaveNote, UiAction::CancelAdd);
    }

    #[test]
    fn note_type_tabs_are_distinct_and_ordered() {
        let tabs = super::note_type_tabs(&models_with_stock(Vec::new()));
        assert_eq!(
            tabs.iter()
                .map(|(label, _)| label.as_str())
                .collect::<Vec<_>>(),
            ["Basic", "Reversed", "Cloze", "Image Occlusion"]
        );
    }

    #[test]
    fn note_type_tabs_prefer_genanki_models() {
        let plain_basic = ranki_model::NoteModel {
            id: 9,
            ..ranki_model::StockModelKind::Basic.definition()
        };
        let genanki_basic = ranki_model::NoteModel {
            id: 10,
            name: "Basic (genanki)".into(),
            ..ranki_model::StockModelKind::Basic.definition()
        };
        let genanki_reversed = ranki_model::NoteModel {
            id: 11,
            name: "Basic (and reversed card) (genanki)".into(),
            ..ranki_model::StockModelKind::BasicAndReversed.definition()
        };
        let genanki_cloze = ranki_model::NoteModel {
            id: 12,
            name: "Cloze (genanki)".into(),
            ..ranki_model::StockModelKind::Cloze.definition()
        };
        let models = models_with_stock(vec![
            plain_basic,
            genanki_basic,
            genanki_reversed,
            genanki_cloze,
        ]);
        let tabs = super::note_type_tabs(&models);

        assert_eq!(tabs.len(), 4);
        assert_eq!(tabs[0].0, "Basic");
        assert_eq!(models[tabs[0].1].name, "Basic (genanki)");
        assert_eq!(tabs[1].0, "Reversed");
        assert_eq!(
            models[tabs[1].1].name,
            "Basic (and reversed card) (genanki)"
        );
        assert_eq!(tabs[2].0, "Cloze");
        assert_eq!(models[tabs[2].1].name, "Cloze (genanki)");
        assert!(tabs.iter().all(|(label, _)| !label.contains("genanki")));
        assert_eq!(tabs[0].1, 1);
        assert_eq!(super::preferred_model_index(&models, 0), Some(tabs[0].1));
        assert_eq!(
            super::preferred_model_index(&models, tabs[0].1),
            Some(tabs[0].1)
        );
    }

    #[test]
    fn custom_note_types_stay_distinct() {
        let first = ranki_model::NoteModel {
            id: 10,
            name: String::from("Vocabulary"),
            ..ranki_model::StockModelKind::Basic.definition()
        };
        let second = ranki_model::NoteModel {
            id: 11,
            name: String::from("Basic"),
            ..ranki_model::StockModelKind::Basic.definition()
        };
        let tabs = super::note_type_tabs(&[first, second]);
        assert_eq!(tabs.len(), 2);
        let labels = tabs
            .iter()
            .map(|(label, _)| label.as_str())
            .collect::<Vec<_>>();
        assert!(labels.contains(&"Vocabulary"));
        assert!(labels.contains(&"Basic"));
    }

    #[test]
    fn stock_tabs_reuse_installed_models() {
        let installed = ranki_model::stock_models()
            .into_iter()
            .next()
            .map(|mut model| {
                model.id = 42;
                model
            })
            .expect("stock basic");
        let models = models_with_stock(vec![installed]);
        let tabs = super::note_type_tabs(&models);
        assert_eq!(tabs.len(), 4);
        assert_eq!(tabs[0].0, "Basic");
        assert_eq!(models[tabs[0].1].id, 42);
        assert_eq!(models[tabs[1].1].id, 0);
    }

    #[test]
    fn tab_selection_switches_the_field_list() {
        let models = models_with_stock(Vec::new());
        let cloze = models
            .iter()
            .find(|model| model.kind == ranki_model::NoteKind::Cloze)
            .expect("stock cloze");
        assert_eq!(cloze.fields, ["Text", "Back Extra"]);
    }

    #[test]
    fn preview_tabs_need_field_text() {
        assert!(!super::has_preview_content(&[String::new()]));
        assert!(!super::has_preview_content(&[String::from("  ")]));
        assert!(super::has_preview_content(&[
            String::from(" "),
            String::from("問い")
        ]));
    }

    #[test]
    fn cloze_actions_start_at_one_without_markers() {
        let actions = super::cloze_insert_actions(&[String::from("plain")], 5);
        assert_eq!(actions.next, 1);
        assert_eq!(actions.linked, None);
    }

    #[test]
    fn cloze_actions_offer_next_and_nearest_linked_number() {
        let values = [String::from("A {{c2::two}} B {{c1::one}} C")];
        let first = super::cloze_insert_actions(&values, 0);
        assert_eq!(first.next, 3);
        assert_eq!(first.linked, Some(2));
        let caret = values[0].find("one").expect("cloze body");
        assert_eq!(super::cloze_insert_actions(&values, caret).linked, Some(1));
        assert_eq!(
            super::cloze_insert_actions(&values, values[0].len()).linked,
            Some(1)
        );
    }

    #[test]
    fn cloze_actions_cap_the_next_number() {
        let values = [String::from("{{c500::last}}")];
        assert_eq!(super::cloze_insert_actions(&values, 0).next, 500);
    }

    #[test]
    fn link_uses_the_nearest_and_innermost_marker() {
        let values = [String::from("{{c1::one}} B {{c2::two}}")];
        let caret = values[0].find("B ").expect("gap") + 1;
        assert_eq!(super::cloze_insert_actions(&values, caret).linked, Some(2));

        let nested = [String::from("{{c1::outer {{c2::inner}} end}}")];
        let caret = nested[0].find("inner").expect("nested body") + 1;
        assert_eq!(super::cloze_insert_actions(&nested, caret).linked, Some(2));
    }

    #[test]
    fn cloze_tags_wrap_the_selected_text() {
        let (open, close) = super::cloze_tags(2);
        assert_eq!(open, "{{c2::");
        let (text, caret) =
            crate::text_field::wrap_selection("France", 0..6, (&open, &close));
        assert_eq!(text, "{{c2::France}}");
        assert_eq!(caret, 14);
    }
}
