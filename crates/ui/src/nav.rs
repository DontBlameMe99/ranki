//! Shared navigation rail, toasts, and modal overlays.

use gpui::{
    AnyElement, ClickEvent, Context, Corner, InteractiveElement, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, Window,
    anchored, div, point, px,
};

use crate::{
    app::{Confirmation, Root, UiAction, View},
    components::{
        ClickHandler, Interaction, brand_mark, icon, keycap, secondary_button,
    },
    theme::{
        NAV_WIDTH, SPACE_BLOCK, SPACE_GROUP, SPACE_INLINE, SPACE_RELATED,
        SPACE_SECTION, Style, alpha,
    },
};

impl Root {
    pub(crate) fn render_nav(&mut self) -> impl IntoElement {
        let colors = self.colors();
        let style = Style::new(colors, self.theme());
        let active = self.view;
        let mark = brand_mark();
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
        .child(icon("icon-plus", 18., add_ink))
        .child("Add card")
        .child(div().flex_1())
        .child(keycap("A", colors, add_selected));
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
            row.child(icon(icon_name, 18., ink))
                .child(label.to_owned())
                .child(div().flex_1())
                .child(if enabled {
                    keycap(key, colors, false).into_any_element()
                } else {
                    div()
                        .text_size(px(10.))
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(crate::theme::alpha(colors.subtext, 0.6))
                        .child("SOON")
                        .into_any_element()
                })
        };
        let status_color = if self.external_change {
            colors.red
        } else {
            colors.green
        };
        div()
            .w(px(NAV_WIDTH))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(SPACE_GROUP))
            .bg(colors.crust)
            .pt(px(SPACE_SECTION))
            .px(px(SPACE_RELATED))
            .pb(px(SPACE_SECTION))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(SPACE_GROUP))
                    .px(px(SPACE_GROUP))
                    .pb(px(SPACE_BLOCK))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(32.))
                            .h(px(32.))
                            .flex_none()
                            .rounded(px(10.))
                            .text_color(style.ink())
                            .text_size(px(mark.glyph))
                            .font_weight(gpui::FontWeight::BOLD)
                            .bg(colors.mauve)
                            .child(mark.label),
                    )
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_weight(gpui::FontWeight::BOLD)
                            .child("Ranki"),
                    ),
            )
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
            .child(nav(
                "Settings",
                "icon-settings",
                ",",
                View::Settings,
                UiAction::OpenSettings,
                true,
            ))
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(SPACE_GROUP))
                    .p(px(SPACE_RELATED))
                    .rounded(px(14.))
                    .bg(colors.mantle)
                    .text_size(px(12.))
                    .text_color(colors.subtext)
                    .child(crate::components::dot(status_color, 8.))
                    .child(crate::app::status_label(
                        self.last_sync,
                        Self::now(),
                    )),
            )
    }

    pub(crate) fn render_overlay(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let colors = self.colors();
        let mut layers: Vec<AnyElement> = Vec::new();
        if self.external_change {
            let refresh = self.action_click(UiAction::Reload);
            layers.push(
                div()
                    .absolute()
                    .top(px(16.))
                    .left(px(crate::theme::NAV_WIDTH + 32.))
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
                        div()
                            .id("external-refresh")
                            .flex()
                            .items_center()
                            .gap(px(SPACE_GROUP))
                            .h(px(36.))
                            .px(px(SPACE_RELATED))
                            .rounded(px(10.))
                            .bg(colors.red)
                            .text_color(crate::theme::accent_ink(self.theme()))
                            .cursor_pointer()
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
                    .left(px(crate::theme::NAV_WIDTH + 32.))
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
        if self.create_modal {
            let cancel = cx.listener(|this, _: &ClickEvent, window, cx| {
                this.cancel_create(window, cx);
            });
            let style = Style::new(colors, self.theme());
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
                                    .border_color(colors.mauve)
                                    .text_size(px(14.))
                                    .text_color(colors.text)
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w(px(0.))
                                            .child(self.create_field.clone()),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .justify_end()
                                    .gap(px(10.))
                                    .child(secondary_button(
                                        "Cancel",
                                        colors,
                                        Box::new(cancel),
                                    ))
                                    .child(primary(
                                        "create-deck-primary",
                                        "Create",
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
            let (title, detail, accept) = match confirmation {
                Confirmation::RemoveDeck { name, .. } => (
                    "Remove deck?",
                    format!("{name} and every card below it will be deleted."),
                    "Remove deck",
                ),
                Confirmation::Reload => (
                    "Reload collection?",
                    String::from(
                        "This ends the current review session and loads the latest collection.",
                    ),
                    "Reload",
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
                        div()
                            .w(px(420.))
                            .flex()
                            .flex_col()
                            .gap(px(SPACE_BLOCK))
                            .p(px(SPACE_SECTION))
                            .rounded(px(24.))
                            .bg(colors.mantle)
                            .border_1()
                            .border_color(colors.surface0)
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
                                    .child(primary(
                                        "confirm-primary",
                                        accept,
                                        Style::new(colors, self.theme()),
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
                    .child(
                        Interaction::surface(colors)
                            .apply(
                                div()
                                    .id("help-close")
                                    .mt(px(SPACE_GROUP))
                                    .text_size(px(12.))
                                    .text_color(colors.mauve)
                                    .cursor_pointer(),
                            )
                            .on_click(close)
                            .child("Close"),
                    )
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

fn primary(
    id: &'static str,
    label: &'static str,
    style: Style,
    action: UiAction,
    cx: &Context<Root>,
) -> impl IntoElement {
    Interaction::accent(style)
        .apply(
            div()
                .id(id)
                .debug_selector(move || id.to_owned())
                .h(px(40.))
                .px(px(SPACE_BLOCK))
                .rounded(px(12.))
                .bg(style.colors.mauve)
                .text_color(style.ink())
                .text_size(px(14.))
                .flex()
                .items_center()
                .cursor_pointer(),
        )
        .on_click(cx.listener(move |_, _: &ClickEvent, window, cx| {
            crate::app::dispatch_app_action(window, cx, action.clone());
        }))
        .child(label)
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
