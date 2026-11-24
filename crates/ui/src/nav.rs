//! Shared navigation rail, toasts, and modal overlays.

use gpui::{
    AnyElement, ClickEvent, Context, Corner, InteractiveElement, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, Window,
    anchored, div, point, px,
};

use crate::{
    app::{Confirmation, Root, SYNC_STATE, UiAction, View},
    components::{ClickHandler, brand_mark, icon, keycap, secondary_button},
    theme::{NAV_WIDTH, Style, alpha},
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
        let mut add = div()
            .id("nav-add")
            .flex()
            .items_center()
            .gap(px(8.))
            .h(px(44.))
            .px(px(14.))
            .rounded(px(12.))
            .text_size(px(14.))
            .text_color(add_ink)
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .cursor_pointer()
            .on_click(self.action_click(UiAction::OpenAdd))
            .hover(move |row| {
                row.bg(if add_selected {
                    colors.mauve
                } else {
                    crate::theme::alpha(colors.mauve, 0.12)
                })
            })
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
                .gap(px(12.))
                .h(px(40.))
                .px(px(12.))
                .rounded(px(10.))
                .text_size(px(14.))
                .text_color(ink)
                .font_weight(gpui::FontWeight::SEMIBOLD);
            if selected {
                row = row.bg(alpha(colors.mauve, 0.16));
            }
            if enabled {
                row = row
                    .cursor_pointer()
                    .hover(move |style| {
                        style.bg(crate::theme::alpha(colors.mauve, 0.12))
                    })
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
            .gap(px(6.))
            .bg(colors.crust)
            .pt(px(24.))
            .px(px(14.))
            .pb(px(20.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(6.))
                    .pb(px(18.))
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
            .child(div().h(px(10.)))
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
                    .gap(px(8.))
                    .p(px(12.))
                    .rounded(px(14.))
                    .bg(colors.mantle)
                    .text_size(px(12.))
                    .text_color(colors.subtext)
                    .child(crate::components::dot(status_color, 8.))
                    .child(crate::app::status_label(SYNC_STATE, Self::now())),
            )
    }

    pub(crate) fn render_overlay(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let colors = self.colors();
        let mut layers: Vec<AnyElement> = Vec::new();
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
                .gap(px(10.))
                .px(px(14.))
                .py(px(10.))
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
                    div()
                        .id("toast-dismiss")
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(24.))
                        .h(px(24.))
                        .flex_none()
                        .rounded(px(6.))
                        .cursor_pointer()
                        .hover(move |style| style.bg(colors.surface1))
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
                            .flex_col()
                            .w(px(160.))
                            .gap(px(2.))
                            .p(px(6.))
                            .rounded(px(12.))
                            .bg(colors.mantle)
                            .border_1()
                            .border_color(colors.surface1)
                            .child(
                                div()
                                    .id("decks-rename")
                                    .w_full()
                                    .px(px(10.))
                                    .py(px(8.))
                                    .rounded(px(8.))
                                    .cursor_pointer()
                                    .hover(|style| style.bg(colors.surface0))
                                    .text_size(px(13.))
                                    .on_click(cx.listener(
                                        move |this, _: &ClickEvent, _, cx| {
                                            this.deck_menu = None;
                                            this.start_rename(&rename_deck, cx);
                                        },
                                    ))
                                    .child("Rename"),
                            )
                            .child(
                                div()
                                    .id("decks-remove")
                                    .w_full()
                                    .px(px(10.))
                                    .py(px(8.))
                                    .rounded(px(8.))
                                    .cursor_pointer()
                                    .text_size(px(13.))
                                    .text_color(colors.red)
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
                                    .child("Remove"),
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
                Confirmation::SwitchScheduler { kind } => (
                    "Change scheduler?",
                    format!("Use {} for this session only.", kind.label()),
                    "Change scheduler",
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
                            .gap(px(16.))
                            .p(px(24.))
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
                                    .gap(px(10.))
                                    .child(secondary_button(
                                        "Cancel", colors, cancel,
                                    ))
                                    .child(primary(
                                        accept,
                                        Style::new(colors, self.theme()),
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
                    .gap(px(6.))
                    .p(px(16.))
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
                    .child(help_row("T", "Statistics / Scheduler", colors))
                    .child(help_row("?", "This sheet", colors))
                    .child(
                        div()
                            .id("auto-nav-2")
                            .mt(px(8.))
                            .text_size(px(12.))
                            .text_color(colors.mauve)
                            .cursor_pointer()
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
    label: &'static str,
    style: Style,
    cx: &Context<Root>,
) -> impl IntoElement {
    div()
        .id("confirm-primary")
        .h(px(40.))
        .px(px(16.))
        .rounded(px(12.))
        .bg(style.colors.mauve)
        .text_color(style.ink())
        .text_size(px(14.))
        .flex()
        .items_center()
        .cursor_pointer()
        .on_click(cx.listener(|_, _: &ClickEvent, window, cx| {
            crate::app::dispatch_app_action(window, cx, UiAction::Confirm);
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
        .gap(px(10.))
        .text_size(px(12.))
        .child(keycap(key, colors, false))
        .child(div().text_color(colors.subtext).child(label))
}
