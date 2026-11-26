//! Shared navigation rail, toasts, and modal overlays.

use gpui::{
    AnyElement, ClickEvent, Context, Corner, CursorStyle, InteractiveElement,
    IntoElement, MouseButton, MouseDownEvent, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, anchored, div, point, px,
};

use crate::{
    app::{Confirmation, Root, UiAction, View},
    components::{
        ButtonTone, ClickHandler, Interaction, action_button, brand, icon,
        keycap, panel, secondary_button,
    },
    theme::{
        NAV_DIVIDER_WIDTH, SPACE_BLOCK, SPACE_GROUP, SPACE_INLINE,
        SPACE_RELATED, SPACE_SECTION, Style, alpha,
    },
};

impl Root {
    pub(crate) fn render_nav(
        &mut self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = self.colors();
        let style = Style::new(colors, self.theme());
        let active = self.view;
        let compact = self.nav_compact();
        let add_selected = active == View::AddCard;
        let add_ink = if add_selected {
            style.ink()
        } else {
            colors.subtext
        };
        let mut add = if add_selected {
            Interaction::accent(style)
        } else {
            Interaction::tinted(colors, colors.mauve)
        }
        .apply(
            div()
                .id("nav-add")
                .flex()
                .items_center()
                .gap(px(SPACE_GROUP))
                .h(px(44.))
                .px(px(SPACE_RELATED))
                .rounded(px(12.))
                .text_size(px(14.))
                .text_color(add_ink)
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .cursor_pointer(),
        )
        .on_click(self.action_click(UiAction::OpenAdd))
        .child(icon("icon-plus", 18., add_ink));
        if compact {
            add = add.justify_center();
        } else {
            add = add.child("Add card").child(div().flex_1()).child(keycap(
                "A",
                colors,
                add_selected,
            ));
        }
        if add_selected {
            add = add.bg(colors.mauve).text_color(style.ink());
        }
        let add = add.w_full();
        let nav = |label: &'static str,
                   icon_name: &'static str,
                   key: &'static str,
                   item: View,
                   action: UiAction,
                   enabled: bool| {
            let selected = enabled && active == item;
            let ink = if selected {
                colors.mauve
            } else if enabled {
                colors.subtext
            } else {
                crate::theme::alpha(colors.subtext, 0.45)
            };
            let mut row = div()
                .id(SharedString::from(format!("nav-{label}")))
                .flex()
                .items_center()
                .gap(px(SPACE_RELATED))
                .h(px(40.))
                .px(px(SPACE_RELATED))
                .rounded(px(10.))
                .text_size(px(14.))
                .text_color(ink)
                .font_weight(gpui::FontWeight::SEMIBOLD);
            if selected {
                row = row.bg(alpha(colors.mauve, 0.16));
            }
            if enabled {
                row = Interaction::tinted(colors, colors.mauve)
                    .apply(row.cursor_pointer())
                    .on_click(self.action_click(action));
            }
            row = row.child(icon(icon_name, 18., ink));
            if compact {
                row = row.justify_center();
            } else {
                row = row.child(label.to_owned()).child(div().flex_1()).child(
                    if enabled {
                        keycap(key, colors, false).into_any_element()
                    } else {
                        div()
                            .text_size(px(10.))
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(crate::theme::alpha(
                                colors.subtext,
                                0.6,
                            ))
                            .child("SOON")
                            .into_any_element()
                    },
                );
            }
            row
        };
        let mut header = div()
            .flex()
            .items_center()
            .gap(px(SPACE_GROUP))
            .px(px(SPACE_GROUP))
            .pb(px(SPACE_BLOCK))
            .child(brand());
        if compact {
            header = header.justify_center();
        } else {
            header = header.child(
                div()
                    .text_size(px(18.))
                    .font_weight(gpui::FontWeight::BOLD)
                    .child("Ranki"),
            );
        }
        div()
            .w(px(self.nav_width))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .relative()
            .gap(px(SPACE_GROUP))
            .bg(colors.crust)
            .pt(px(SPACE_SECTION))
            .px(px(SPACE_RELATED))
            .pb(px(SPACE_SECTION))
            .child(header)
            .child(add)
            .child(div().h(px(SPACE_RELATED)))
            .child(nav(
                "Decks",
                "icon-grid",
                "D",
                View::Decks,
                UiAction::OpenDecks,
                true,
            ))
            .child(nav(
                "Browse",
                "icon-book",
                "B",
                View::Browse,
                UiAction::OpenBrowse,
                true,
            ))
            .child(nav(
                "Statistics",
                "icon-chart",
                "T",
                View::Statistics,
                UiAction::OpenStatistics,
                true,
            ))
            .child(div().flex_1())
            .child(nav(
                "Settings",
                "icon-settings",
                ",",
                View::Settings,
                UiAction::OpenSettings,
                true,
            ))
            .child(self.render_nav_divider(cx))
    }

    /// Grab strip on the rail's edge, drawn over the last few pixels.
    fn render_nav_divider(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors();
        let mut bar = div()
            .id("nav-divider")
            .debug_selector(|| String::from("nav-divider"))
            .absolute()
            .top(px(0.))
            .right(px(0.))
            .w(px(NAV_DIVIDER_WIDTH))
            .h_full()
            .cursor(CursorStyle::ResizeColumn);
        if self.nav_drag.is_some() {
            bar = bar.bg(colors.mauve);
        }
        bar.hover(move |style| style.bg(alpha(colors.mauve, 0.6)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.start_nav_drag(event, cx);
                }),
            )
    }

    pub(crate) fn render_overlay(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let colors = self.colors();
        let theme = self.theme();
        let style = Style::new(colors, theme);
        let mut layers: Vec<AnyElement> = Vec::new();
        if self.external_change {
            let refresh = self.action_click(UiAction::Reload);
            layers.push(
                div()
                    .absolute()
                    .top(px(16.))
                    .left(px(self.nav_width + 32.))
                    .right(px(32.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(SPACE_BLOCK))
                    .px(px(SPACE_BLOCK))
                    .py(px(SPACE_RELATED))
                    .rounded(px(14.))
                    .bg(crate::theme::alpha(colors.red, 0.16))
                    .border_1()
                    .border_color(crate::theme::alpha(colors.red, 0.5))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(SPACE_GROUP))
                            .child(icon("icon-bell", 18., colors.red))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(colors.text)
                                    .child(
                                        "Collection changed outside Ranki. Refresh before reviewing.",
                                    ),
                            ),
                    )
                    .child(
                        Interaction::accent(style)
                            .apply(
                                div()
                                    .id("external-refresh")
                                    .flex()
                                    .items_center()
                                    .gap(px(SPACE_GROUP))
                                    .h(px(36.))
                                    .px(px(SPACE_RELATED))
                                    .rounded(px(10.))
                                    .bg(colors.mauve)
                                    .text_color(style.ink())
                                    .cursor_pointer(),
                            )
                            .on_click(refresh)
                            .child("Refresh")
                            .child(keycap("F5", colors, true)),
                    )
                    .into_any_element(),
            );
        }
        if let Some(toast) = self.live_toast() {
            let (hue, icon_name, prefix) = match toast.kind {
                crate::app::ToastKind::Success => {
                    (colors.green, "icon-check", "Done")
                }
                crate::app::ToastKind::Error => {
                    (colors.red, "icon-bell", "Error")
                }
            };
            let dismiss = cx.listener(|root, _: &ClickEvent, _, cx| {
                root.clear_toast();
                cx.notify();
            });
            let body = div()
                .flex()
                .items_center()
                .gap(px(SPACE_GROUP))
                .px(px(SPACE_RELATED))
                .py(px(SPACE_RELATED))
                .rounded(px(12.))
                .bg(crate::theme::alpha(hue, 0.16))
                .border_1()
                .border_color(crate::theme::alpha(hue, 0.4))
                .text_size(px(13.))
                .text_color(colors.text)
                .child(icon(icon_name, 16., hue))
                .child(
                    div()
                        .text_size(px(11.))
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(hue)
                        .child(prefix),
                )
                .child(toast.text.clone())
                .child(
                    Interaction::surface(colors)
                        .apply(
                            div()
                                .id("toast-dismiss")
                                .flex()
                                .items_center()
                                .justify_center()
                                .w(px(24.))
                                .h(px(24.))
                                .flex_none()
                                .rounded(px(6.))
                                .cursor_pointer(),
                        )
                        .on_click(dismiss)
                        .child(icon("icon-x", 14., colors.subtext)),
                );
            layers.push(
                div()
                    .absolute()
                    .bottom(px(72.))
                    .left(px(self.nav_width + 32.))
                    .child(crate::components::fade_in(
                        body,
                        ("toast", usize::try_from(toast.seq).unwrap_or(0)),
                        self.motion_ms(crate::theme::MOTION_QUICK_MS),
                    ))
                    .into_any_element(),
            );
        }
        if self.view == View::Decks
            && let Some(menu) = self.deck_menu
            && self
                .visible_deck_rows(cx)
                .iter()
                .any(|row| row.id == Some(menu.id))
            && let Some(deck) =
                self.decks.iter().find(|deck| deck.id == menu.id).cloned()
        {
            let rename_deck = deck.clone();
            let remove_deck = deck;
            layers.push(
                anchored()
                    .anchor(Corner::TopRight)
                    .position(menu.anchor)
                    .offset(point(px(15.), px(19.)))
                    .child(
                        div()
                            .id("deck-actions-menu")
                            .debug_selector(|| "deck-actions-menu".to_owned())
                            .on_mouse_down_out(cx.listener(
                                move |this,
                                      event: &gpui::MouseDownEvent,
                                      _,
                                      cx| {
                                    this.deck_menu = None;
                                    this.deck_menu_dismissed_press =
                                        Some(event.clone());
                                    cx.notify();
                                },
                            ))
                            .flex_col()
                            .w(px(160.))
                            .gap(px(SPACE_INLINE))
                            .p(px(SPACE_GROUP))
                            .rounded(px(12.))
                            .bg(colors.mantle)
                            .border_1()
                            .border_color(colors.surface1)
                            .child(
                                Interaction::surface(colors)
                                    .apply(
                                        div()
                                            .id("decks-rename")
                                            .w_full()
                                            .px(px(SPACE_RELATED))
                                            .py(px(SPACE_GROUP))
                                            .rounded(px(8.))
                                            .cursor_pointer(),
                                    )
                                    .on_click(cx.listener(
                                        move |this, _: &ClickEvent, _, cx| {
                                            this.deck_menu = None;
                                            this.start_rename(&rename_deck, cx);
                                        },
                                    ))
                                    .text_size(px(13.))
                                    .child("Rename"),
                            )
                            .child(
                                Interaction::danger(colors)
                                    .apply(
                                        div()
                                            .id("decks-remove")
                                            .w_full()
                                            .px(px(SPACE_RELATED))
                                            .py(px(SPACE_GROUP))
                                            .rounded(px(8.))
                                            .cursor_pointer()
                                            .text_color(colors.red),
                                    )
                                    .on_click(cx.listener(
                                        move |this, _: &ClickEvent, _, cx| {
                                            this.deck_menu = None;
                                            this.confirmation = Some(
                                                Confirmation::RemoveDeck {
                                                    id: remove_deck.id,
                                                    name: remove_deck
                                                        .name
                                                        .clone(),
                                                },
                                            );
                                            cx.notify();
                                        },
                                    ))
                                    .text_size(px(13.))
                                    .child("Remove"),
                            ),
                    )
                    .into_any_element(),
            );
        }
        if self.view == View::Browse
            && let Some(menu) = self.card_menu
            && let Some(row) =
                self.browse.rows.iter().find(|row| row.card.id == menu.id)
        {
            let card_id = menu.id;
            let front = crate::browser_view::preview_line(&row.front);
            layers.push(
                anchored()
                    .anchor(Corner::TopRight)
                    .position(menu.anchor)
                    .offset(point(px(15.), px(19.)))
                    .child(
                        div()
                            .id("card-actions-menu")
                            .debug_selector(|| "card-actions-menu".to_owned())
                            .on_mouse_down_out(cx.listener(
                                move |this,
                                      event: &gpui::MouseDownEvent,
                                      _,
                                      cx| {
                                    this.card_menu = None;
                                    this.card_menu_dismissed_press =
                                        Some(event.clone());
                                    cx.notify();
                                },
                            ))
                            .flex_col()
                            .w(px(160.))
                            .gap(px(SPACE_INLINE))
                            .p(px(SPACE_GROUP))
                            .rounded(px(12.))
                            .bg(colors.mantle)
                            .border_1()
                            .border_color(colors.surface1)
                            .child(
                                Interaction::surface(colors)
                                    .apply(
                                        div()
                                            .id("browse-edit")
                                            .w_full()
                                            .px(px(SPACE_RELATED))
                                            .py(px(SPACE_GROUP))
                                            .rounded(px(8.))
                                            .cursor_pointer(),
                                    )
                                    .on_click(cx.listener(
                                        move |this,
                                              _: &ClickEvent,
                                              window,
                                              cx| {
                                            this.card_menu = None;
                                            this.start_card_edit(
                                                card_id, window, cx,
                                            );
                                        },
                                    ))
                                    .text_size(px(13.))
                                    .child("Edit"),
                            )
                            .child(
                                Interaction::danger(colors)
                                    .apply(
                                        div()
                                            .id("browse-remove")
                                            .w_full()
                                            .px(px(SPACE_RELATED))
                                            .py(px(SPACE_GROUP))
                                            .rounded(px(8.))
                                            .cursor_pointer()
                                            .text_color(colors.red),
                                    )
                                    .on_click(cx.listener(
                                        move |this,
                                              _: &ClickEvent,
                                              window,
                                              cx| {
                                            this.card_menu = None;
                                            this.confirmation = Some(
                                                Confirmation::RemoveCard {
                                                    id: card_id,
                                                    front: front.clone(),
                                                },
                                            );
                                            cx.notify();
                                            window.focus(&this.dialog_focus);
                                        },
                                    ))
                                    .text_size(px(13.))
                                    .child("Remove"),
                            ),
                    )
                    .into_any_element(),
            );
        }
        if self.view == View::AddCard && self.add.deck_picker {
            layers.push(
                anchored()
                    .anchor(Corner::TopLeft)
                    .position(self.add.deck_picker_anchor)
                    .offset(point(px(0.), px(SPACE_BLOCK + SPACE_INLINE)))
                    .child(self.render_deck_picker(colors, cx))
                    .into_any_element(),
            );
        }
        if self.create_modal {
            let cancel = cx.listener(|this, _: &ClickEvent, window, cx| {
                this.cancel_create(window, cx);
            });
            let create_error = self.create_error.clone();
            layers.push(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(crate::theme::alpha(colors.crust, 0.7))
                    .occlude()
                    .id("create-deck-backdrop")
                    .on_click(cx.listener(
                        |this, _: &ClickEvent, window, cx| {
                            this.cancel_create(window, cx);
                        },
                    ))
                    .key_context("ranki_dialog")
                    .track_focus(&self.dialog_focus)
                    .on_action(cx.listener(
                        |this, _: &crate::app::ConfirmDialog, window, cx| {
                            this.dispatch(UiAction::Confirm, window, cx);
                        },
                    ))
                    .on_action(cx.listener(
                        |this, _: &crate::app::CancelDialog, window, cx| {
                            this.dispatch(UiAction::CancelDialog, window, cx);
                        },
                    ))
                    .child(
                        div()
                            .id("create-deck-modal")
                            .w(px(420.))
                            .flex()
                            .flex_col()
                            .gap(px(16.))
                            .p(px(24.))
                            .rounded(px(24.))
                            .bg(colors.mantle)
                            .border_1()
                            .border_color(colors.surface0)
                            .occlude()
                            .on_click(|_, _, cx| cx.stop_propagation())
                            .child(
                                div()
                                    .text_size(px(20.))
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child("Create deck"),
                            )
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .text_color(colors.subtext)
                                    .child("Name the new deck."),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .h(px(44.))
                                    .px(px(14.))
                                    .rounded(px(12.))
                                    .bg(colors.surface0)
                                    .border_1()
                                    .border_color(if create_error.is_some() {
                                        colors.red
                                    } else {
                                        colors.mauve
                                    })
                                    .text_size(px(14.))
                                    .text_color(colors.text)
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w(px(0.))
                                            .child(self.create_field.clone()),
                                    ),
                            )
                            .children(create_error.map(|message| {
                                div()
                                    .flex()
                                    .items_start()
                                    .gap(px(SPACE_INLINE))
                                    .text_size(px(13.))
                                    .text_color(colors.red)
                                    .child(icon("icon-bell", 14., colors.red))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w(px(0.))
                                            .child(message),
                                    )
                            }))
                            .child(
                                div()
                                    .flex()
                                    .justify_end()
                                    .gap(px(SPACE_GROUP))
                                    .child(secondary_button(
                                        "Cancel",
                                        colors,
                                        Box::new(cancel),
                                    ))
                                    .child(action(
                                        "create-deck-primary",
                                        "Create",
                                        ButtonTone::Primary,
                                        style,
                                        UiAction::SubmitDeck,
                                        cx,
                                    )),
                            ),
                    )
                    .into_any_element(),
            );
        }
        if let Some(confirmation) = self.confirmation.clone() {
            let (title, detail, accept, tone) = match confirmation {
                Confirmation::RemoveCard { front, .. } => (
                    "Remove card?",
                    format!("{front} will be deleted with its review history."),
                    "Remove card",
                    ButtonTone::Danger,
                ),
                Confirmation::RemoveDeck { name, .. } => (
                    "Remove deck?",
                    format!("{name} and every card below it will be deleted."),
                    "Remove deck",
                    ButtonTone::Danger,
                ),
                Confirmation::Reload => (
                    "Reload collection?",
                    String::from(
                        "This ends the current review session and loads the latest collection.",
                    ),
                    "Reload",
                    ButtonTone::Primary,
                ),
            };
            let cancel = self.action_click(UiAction::CancelDialog);
            layers.push(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(crate::theme::alpha(colors.crust, 0.7))
                    .key_context("ranki_dialog")
                    .track_focus(&self.dialog_focus)
                    .on_action(cx.listener(
                        |this, _: &crate::app::ConfirmDialog, window, cx| {
                            this.dispatch(UiAction::Confirm, window, cx);
                        },
                    ))
                    .on_action(cx.listener(
                        |this, _: &crate::app::CancelDialog, window, cx| {
                            this.dispatch(UiAction::CancelDialog, window, cx);
                        },
                    ))
                    .child(
                        panel(colors)
                            .w(px(420.))
                            .child(
                                div()
                                    .text_size(px(20.))
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .text_color(colors.subtext)
                                    .child(detail),
                            )
                            .child(
                                div()
                                    .flex()
                                    .justify_end()
                                    .gap(px(SPACE_GROUP))
                                    .child(secondary_button(
                                        "Cancel", colors, cancel,
                                    ))
                                    .child(action(
                                        "confirm-primary",
                                        accept,
                                        tone,
                                        style,
                                        UiAction::Confirm,
                                        cx,
                                    )),
                            ),
                    )
                    .into_any_element(),
            );
        }
        if self.help_open {
            let close = self.action_click(UiAction::ToggleHelp);
            layers.push(
                div()
                    .absolute()
                    .top(px(16.))
                    .right(px(20.))
                    .w(px(300.))
                    .flex()
                    .flex_col()
                    .gap(px(SPACE_GROUP))
                    .p(px(SPACE_BLOCK))
                    .rounded(px(18.))
                    .bg(colors.mantle)
                    .border_1()
                    .border_color(colors.surface0)
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_weight(gpui::FontWeight::BOLD)
                            .child("Keyboard"),
                    )
                    .child(help_row("Space", "Reveal answer", colors))
                    .child(help_row("1 - 4", "Grade card", colors))
                    .child(help_row("Ctrl Z", "Undo last grade", colors))
                    .child(help_row("R", "Show question", colors))
                    .child(help_row("S", "Study again", colors))
                    .child(help_row("Esc", "End session", colors))
                    .child(help_row("A / D", "Add card / Decks", colors))
                    .child(help_row("T", "Statistics", colors))
                    .child(help_row("?", "This sheet", colors))
                    .child(div().flex().child(action_button(
                        "help-close",
                        "Close",
                        ButtonTone::Neutral,
                        style,
                        close,
                    )))
                    .into_any_element(),
            );
        }
        layers
    }

    pub(crate) fn action_click(&self, action: UiAction) -> ClickHandler {
        let focus = self.focus.clone();
        Box::new(move |_event: &ClickEvent, window: &mut Window, cx| {
            window.focus(&focus);
            crate::app::dispatch_app_action(window, cx, action.clone());
        })
    }
}

fn action(
    id: &'static str,
    label: &'static str,
    tone: ButtonTone,
    style: Style,
    action: UiAction,
    cx: &Context<Root>,
) -> impl IntoElement {
    action_button(
        id,
        label,
        tone,
        style,
        Box::new(cx.listener(move |_, _: &ClickEvent, window, cx| {
            crate::app::dispatch_app_action(window, cx, action.clone());
        })),
    )
}

fn help_row(
    key: &'static str,
    label: &'static str,
    colors: crate::theme::Palette,
) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(SPACE_GROUP))
        .text_size(px(12.))
        .child(keycap(key, colors, false))
        .child(div().text_color(colors.subtext).child(label))
}
