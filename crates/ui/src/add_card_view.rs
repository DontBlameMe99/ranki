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
        let field_values = fields
            .iter()
            .map(|field| field.read(cx).text())
            .collect::<Vec<_>>();
        let has_content = has_preview_content(&field_values);
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
            column = column.child(validation(error, colors));
        }
        if let Some(error) = errors.get("model") {
            column = column.child(validation(error, colors));
        }

        let occlusion_mode =
            model.is_some_and(ranki_model::NoteModel::is_image_occlusion);
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
                field_box = field_box.child(formatting_toolbar(
                    field.clone(),
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
                                    div()
                                        .flex_1()
                                        .min_w(px(80.))
                                        .child(tag_input),
                                ),
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
        let preview = preview.child(preview_header).child(if has_content {
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

        let history = added_history(&added, colors);

        let mut form = div().flex().w_full().gap(px(ADD_GAP));
        if occlusion_mode {
            form = form.flex_col().child(left).child(history);
        } else if side_by_side {
            form = form
                .items_start()
                .child(
                    div().flex_1().min_w(px(ADD_FORM_WIDTH * 0.5)).child(left),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(18.))
                        .w(px(PREVIEW_WIDTH))
                        .flex_none()
                        .child(preview)
                        .child(history),
                );
        } else {
            form = form.flex_col().child(left).child(preview).child(history);
        }
        column = column.child(form);

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

    /// Image-occlusion editor: media picker, drawing area, and mask list.
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
        let filter = self.add.media_filter.read(cx).text();
        let visible =
            occlusion::filter_media(&self.add.occlusion.media, &filter)
                .into_iter()
                .cloned()
                .collect::<Vec<_>>();
        let total = self.add.occlusion.media.len();
        let media_dir = self.options.media_dir.clone();
        let media_filter = self.add.media_filter.clone();

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

        let mut picker = div().flex().flex_wrap().gap(px(6.));
        if visible.is_empty() {
            picker = picker.child(
                div()
                    .text_size(px(12.))
                    .text_color(colors.overlay)
                    .child(format!("No images in {}", media_dir.display())),
            );
        }
        let focus_handle = media_filter.read(cx).focus_handle().clone();
        for name in &visible {
            let chosen = image.as_deref() == Some(name.as_str());
            let pick = name.clone();
            let handle = focus_handle.clone();
            picker = picker.child(
                div()
                    .id(SharedString::from(format!("io-image-{name}")))
                    .h(px(30.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .rounded(px(9.))
                    .border_1()
                    .border_color(if chosen {
                        colors.mauve
                    } else {
                        colors.surface1
                    })
                    .bg(if chosen {
                        alpha(colors.mauve, 0.2)
                    } else {
                        colors.base
                    })
                    .text_size(px(12.))
                    .text_color(if chosen {
                        colors.mauve
                    } else {
                        colors.subtext
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, window, cx| {
                            this.choose_occlusion_image(pick.clone(), cx);
                            window.focus(&handle);
                        },
                    ))
                    .child(name.clone()),
            );
        }

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
                    .child("Pick an image, then drag to draw masks"),
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
                    this.occlusion_press(pointer(event.position), cx);
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
                    .h(px(38.))
                    .px(px(12.))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(colors.surface1)
                    .bg(colors.base)
                    .child(icon("icon-search", 14., colors.subtext))
                    .child(div().flex_1().min_w(px(80.)).child(media_filter))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(colors.overlay)
                            .child(format!("{total} images")),
                    ),
            )
            .child(picker)
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

/// Field formatting bar plus the keyboard hint, under the first field.
fn formatting_toolbar(
    field: gpui::Entity<crate::text_field::TextField>,
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
    div()
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
        .child(button("I", false, wrap(("<i>", "</i>"), field, handle)))
        .child(div().flex_1())
        .child(
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
                .mt(px(18.))
                .mb(px(2.))
                .text_size(px(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
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

/// Distinct note types in mockup order, keeping the first model per label.
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
}
