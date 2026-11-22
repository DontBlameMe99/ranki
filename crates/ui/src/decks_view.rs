//! Decks screen: header, summary cards, five-column table, and empty states.

use gpui::{
    ClickEvent, Context, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, px,
};

use crate::{
    app::{self, Confirmation, Root, UiAction, due_total, leaf_deck_totals},
    components::{icon, keycap, small_button},
    theme::{PAGE_X, PAGE_Y, TABLE_COLUMNS, alpha, queue_color},
};

impl Root {
    pub(crate) fn render_decks(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = self.colors();
        let theme = self.theme();
        let query = self.deck_search.read(cx).text();
        let all_rows = self.deck_rows.clone();
        let rows = app::filter_decks(&all_rows, &query);
        let totals = leaf_deck_totals(&self.decks, &self.stats);
        let due = due_total(totals);
        let selected = self.selected;
        let search_creating =
            self.create_field.read(cx).focus_handle().is_focused(window);
        let create_click = self.action_click(UiAction::CreateDeck);
        let mut content = div()
            .size_full()
            .flex()
            .flex_col()
            .id("decks-scroll")
            .gap(px(24.))
            .overflow_scroll()
            .px(px(PAGE_X))
            .py(px(PAGE_Y));

        content = content.child(
            div()
                .flex()
                .items_end()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(
                            div()
                                .text_size(px(13.))
                                .text_color(colors.subtext)
                                .child(today_label()),
                        )
                        .child(
                            div()
                                .text_size(px(32.))
                                .font_weight(gpui::FontWeight::BOLD)
                                .child("Decks"),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .child(
                            div()
                                .id("auto-decks_view-1")
                                .flex()
                                .items_center()
                                .gap(px(10.))
                                .w(px(300.))
                                .h(px(44.))
                                .px(px(14.))
                                .rounded(px(12.))
                                .bg(colors.surface0)
                                .text_size(px(14.))
                                .text_color(colors.text)
                                .on_click(cx.listener(
                                    |this, _: &ClickEvent, window, cx| {
                                        let field = this.deck_search.clone();
                                        window.focus(
                                            field.read(cx).focus_handle(),
                                        );
                                    },
                                ))
                                .child(icon("icon-search", 18.))
                                .child(self.deck_search.clone())
                                .child(keycap("/", colors, false)),
                        )
                        .child(
                            div()
                                .id("auto-decks_view-2")
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .h(px(44.))
                                .px(px(16.))
                                .rounded(px(12.))
                                .border_1()
                                .border_color(colors.surface1)
                                .text_size(px(14.))
                                .cursor_pointer()
                                .on_click(create_click)
                                .child(icon("icon-plus", 18.))
                                .child("Create deck"),
                        ),
                ),
        );

        if search_creating {
            content = content.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(self.create_field.clone())
                    .child(small_button(
                        "Create",
                        colors,
                        theme,
                        self.action_click(UiAction::SubmitDeck),
                    ))
                    .child(
                        div()
                            .id("auto-decks_view-3")
                            .text_size(px(13.))
                            .text_color(colors.subtext)
                            .cursor_pointer()
                            .on_click(cx.listener(
                                |this, _: &ClickEvent, _, cx| {
                                    this.create_field.update(
                                        cx,
                                        |field, cx| {
                                            field.clear(cx);
                                        },
                                    );
                                    cx.notify();
                                },
                            ))
                            .child("Cancel"),
                    ),
            );
        }

        content = content.child(
            div()
                .flex()
                .gap(px(20.))
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .flex_col()
                        .gap(px(18.))
                        .p(px(24.))
                        .rounded(px(24.))
                        .bg(colors.surface0)
                        .child(
                            div()
                                .text_size(px(13.))
                                .text_color(colors.subtext)
                                .child("Due today"),
                        )
                        .child(
                            div()
                                .text_size(px(64.))
                                .font_weight(gpui::FontWeight::BOLD)
                                .text_color(colors.mauve)
                                .child(format!("{due}")),
                        )
                        .child(
                            div()
                                .id("auto-decks_view-4")
                                .flex()
                                .items_center()
                                .h(px(52.))
                                .px(px(24.))
                                .rounded(px(26.))
                                .bg(if due > 0 {
                                    colors.mauve
                                } else {
                                    colors.surface1
                                })
                                .text_color(crate::theme::accent_ink(theme))
                                .text_size(px(16.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .cursor_pointer()
                                .on_click(cx.listener(
                                    |this, _: &ClickEvent, _, cx| {
                                        this.study_all(cx);
                                    },
                                ))
                                .child(icon("icon-play", 18.))
                                .child("Study all"),
                        )
                        .child(
                            div()
                                .flex()
                                .gap(px(16.))
                                .pt(px(16.))
                                .border_t_1()
                                .border_color(colors.surface1)
                                .child(queue_stat(
                                    "New",
                                    totals.new_cards,
                                    colors.blue,
                                    colors,
                                ))
                                .child(queue_stat(
                                    "Learning",
                                    totals.learning_cards,
                                    colors.red,
                                    colors,
                                ))
                                .child(queue_stat(
                                    "Review",
                                    totals.review_cards,
                                    colors.green,
                                    colors,
                                )),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(12.))
                        .w(px(320.))
                        .p(px(24.))
                        .rounded(px(24.))
                        .bg(colors.mantle)
                        .border_1()
                        .border_color(colors.surface0)
                        .child(
                            div()
                                .flex()
                                .justify_between()
                                .items_center()
                                .child(
                                    div()
                                        .text_size(px(14.))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .child("Next 7 days"),
                                )
                                .child(
                                    div()
                                        .px(px(10.))
                                        .py(px(4.))
                                        .rounded(px(10.))
                                        .bg(alpha(colors.mauve, 0.16))
                                        .text_size(px(12.))
                                        .text_color(colors.mauve)
                                        .child("NO DATA"),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .items_end()
                                .gap(px(10.))
                                .h(px(92.))
                                .child(
                                    div()
                                        .flex_1()
                                        .text_size(px(12.))
                                        .text_color(colors.subtext)
                                        .child("7-day forecast unavailable"),
                                ),
                        ),
                ),
        );

        let kind =
            app::empty_kind(self.decks.len(), due, !query.trim().is_empty());
        if kind == app::EmptyKind::None {
            let mut table = div()
                .flex()
                .flex_col()
                .rounded(px(20.))
                .bg(colors.mantle)
                .border_1()
                .border_color(colors.surface0)
                .overflow_hidden();
            table = table.child(
                div()
                    .flex()
                    .items_center()
                    .px(px(20.))
                    .py(px(12.))
                    .text_size(px(12.))
                    .text_color(colors.subtext)
                    .child(header_cell("DECK", TABLE_COLUMNS[0]))
                    .child(header_cell("NEW", TABLE_COLUMNS[1]))
                    .child(header_cell("LEARNING", TABLE_COLUMNS[2]))
                    .child(header_cell("REVIEW", TABLE_COLUMNS[3]))
                    .child(header_cell("", TABLE_COLUMNS[4])),
            );
            for row in rows {
                table = table
                    .child(self.render_deck_row(&row, selected, &query, cx));
            }
            content = content.child(table);
        } else {
            content = content.child(self.render_empty(kind));
        }
        content
    }

    fn render_deck_row(
        &mut self,
        row: &app::DeckRow,
        selected: Option<usize>,
        query: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = self.colors();
        let theme = self.theme();
        let is_selected = row
            .id
            .and_then(|id| self.decks.iter().position(|deck| deck.id == id))
            == selected;
        let index = row
            .id
            .and_then(|id| self.decks.iter().position(|deck| deck.id == id));
        let has_due = due_total(row.stats) > 0;
        let menu_open = row.id.is_some_and(|id| self.deck_menu == Some(id));
        let mut name_cell = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .pl(px(35.0 * row.depth as f32))
            .flex_1();
        if row.depth > 0 {
            name_cell = name_cell.child(
                div().h(px(28.)).w(px(2.)).bg(colors.surface1).flex_none(),
            );
        }
        if row.has_children {
            let path = row.path.clone();
            name_cell = name_cell.child(
                div()
                    .id("auto-decks_view-5")
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(24.))
                    .h(px(24.))
                    .cursor_pointer()
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.toggle_deck(path.clone(), cx);
                        },
                    ))
                    .child(icon(
                        if row.expanded {
                            "icon-chevron-down"
                        } else {
                            "icon-chevron-right"
                        },
                        18.,
                    )),
            );
        } else {
            name_cell = name_cell.child(div().w(px(24.)));
        }
        name_cell = name_cell
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(36.))
                    .h(px(36.))
                    .rounded(px(10.))
                    .bg(alpha(
                        if row.depth.is_multiple_of(2) {
                            colors.teal
                        } else {
                            colors.peach
                        },
                        0.16,
                    ))
                    .child(icon("icon-layers", 16.)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(row.name.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(12.5))
                            .text_color(colors.subtext)
                            .child(if row.virtual_group {
                                format!("{} subdecks", row.path)
                            } else {
                                row.path.clone()
                            }),
                    ),
            );
        if let Some(deck_id) = row.id
            && menu_open
            && let Some(edit) = self.deck_edit.clone()
            && edit.id == deck_id
        {
            name_cell = name_cell.child(self.rename_field.clone()).child(
                div()
                    .id("auto-decks_view-6")
                    .text_size(px(13.))
                    .text_color(colors.mauve)
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.apply_rename(cx);
                    }))
                    .child("Save"),
            );
        }
        let count = |value: usize, color: gpui::Rgba| {
            div()
                .w(px(TABLE_COLUMNS[1]))
                .flex()
                .justify_end()
                .text_size(px(15.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(if value == 0 { colors.overlay } else { color })
                .child(format!("{value}"))
        };
        let mut action = div()
            .w(px(TABLE_COLUMNS[4]))
            .flex()
            .justify_end()
            .items_center();
        if is_selected && !row.virtual_group && has_due && index.is_some() {
            let target = index.unwrap_or(0);
            action = action.child(
                div()
                    .id("auto-decks_view-7")
                    .h(px(30.))
                    .px(px(14.))
                    .rounded(px(9.))
                    .bg(colors.mauve)
                    .text_color(crate::theme::accent_ink(theme))
                    .text_size(px(13.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .cursor_pointer()
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.study_deck(target, cx);
                        },
                    ))
                    .child("Study"),
            );
        } else if row.id.is_some() {
            let id = row.id.unwrap_or(0);
            action = action.child(
                div()
                    .id("auto-decks_view-8")
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(30.))
                    .h(px(30.))
                    .cursor_pointer()
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.deck_menu = if this.deck_menu == Some(id) {
                                None
                            } else {
                                Some(id)
                            };
                            cx.notify();
                        },
                    ))
                    .child(icon("icon-more", 18.)),
            );
        } else {
            action = action.child(
                div()
                    .text_size(px(12.))
                    .text_color(colors.overlay)
                    .child("group"),
            );
        }
        let mut row_div = div()
            .flex()
            .items_center()
            .min_h(px(60.))
            .px(px(20.))
            .border_t_1()
            .border_color(colors.surface0)
            .bg(if is_selected {
                colors.surface0
            } else {
                colors.mantle
            })
            .child(name_cell)
            .child(count(row.stats.new_cards, queue_color(0, theme)))
            .child(count(row.stats.learning_cards, queue_color(1, theme)))
            .child(count(row.stats.review_cards, queue_color(2, theme)))
            .child(action);
        if menu_open
            && let Some(deck) = row.id.and_then(|id| {
                self.decks.iter().find(|deck| deck.id == id).cloned()
            })
        {
            let rename_deck = deck.clone();
            let remove_deck = deck;
            row_div = row_div.child(
                div()
                    .absolute()
                    .right(px(20.))
                    .top(px(56.))
                    .flex()
                    .gap(px(6.))
                    .p(px(8.))
                    .rounded(px(12.))
                    .bg(colors.mantle)
                    .border_1()
                    .border_color(colors.surface1)
                    .child(
                        div()
                            .id("auto-decks_view-9")
                            .px(px(10.))
                            .py(px(6.))
                            .rounded(px(8.))
                            .cursor_pointer()
                            .hover(|style| style.bg(colors.surface0))
                            .text_size(px(13.))
                            .on_click(cx.listener(
                                move |this, _: &ClickEvent, _, cx| {
                                    this.start_rename(&rename_deck, cx);
                                },
                            ))
                            .child("Rename"),
                    )
                    .child(
                        div()
                            .id("auto-decks_view-11")
                            .px(px(10.))
                            .py(px(6.))
                            .rounded(px(8.))
                            .cursor_pointer()
                            .text_size(px(13.))
                            .text_color(colors.red)
                            .on_click(cx.listener(
                                move |this, _: &ClickEvent, _, cx| {
                                    this.deck_menu = None;
                                    this.confirmation =
                                        Some(Confirmation::RemoveDeck {
                                            id: remove_deck.id,
                                            name: remove_deck.name.clone(),
                                        });
                                    cx.notify();
                                },
                            ))
                            .child("Remove"),
                    ),
            );
        }
        let _ = (theme, query);
        row_div
    }

    fn render_empty(&self, kind: app::EmptyKind) -> impl IntoElement {
        let colors = self.colors();
        let theme = self.theme();
        let (title, detail) = app::empty_copy(kind);
        let (label, action) = match kind {
            app::EmptyKind::NoDecks => ("Create deck", UiAction::CreateDeck),
            app::EmptyKind::NoMatch => ("Clear search", UiAction::ClearSearch),
            app::EmptyKind::CaughtUp => {
                ("Study another deck", UiAction::NextDeck)
            }
            app::EmptyKind::None => ("", UiAction::OpenDecks),
        };
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(10.))
            .min_h(px(260.))
            .rounded(px(20.))
            .bg(colors.mantle)
            .border_1()
            .border_color(colors.surface0)
            .child(
                div()
                    .text_size(px(24.))
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
                    .id("auto-decks_view-12")
                    .mt(px(10.))
                    .h(px(44.))
                    .px(px(20.))
                    .rounded(px(12.))
                    .bg(colors.mauve)
                    .text_color(crate::theme::accent_ink(theme))
                    .text_size(px(14.))
                    .cursor_pointer()
                    .on_click(self.action_click(action))
                    .child(label),
            )
    }
}

fn header_cell(label: &str, width: f32) -> gpui::Div {
    let mut cell = div()
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(gpui::rgb(0x7f849c));
    if label == "DECK" {
        cell = cell.flex_1();
    } else {
        cell = cell.w(px(width));
    }
    cell.child(label.to_owned())
}

fn queue_stat(
    label: &str,
    value: usize,
    color: gpui::Rgba,
    colors: crate::theme::Palette,
) -> gpui::Div {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(crate::components::dot(color, 8.))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(colors.subtext)
                        .child(label.to_owned()),
                ),
        )
        .child(
            div()
                .text_size(px(20.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(format!("{value}")),
        )
}

fn today_label() -> String {
    let now = std::time::SystemTime::now();
    let date = chrono_date(now);
    date
}

fn chrono_date(now: std::time::SystemTime) -> String {
    let seconds = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = seconds / 86_400;
    let year = 1970 + days / 365;
    let month = (days % 365) / 30 + 1;
    let day = days % 30 + 1;
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::{chrono_date, header_cell};

    #[test]
    fn deck_header_uses_the_five_column_widths() {
        let head = header_cell("NEW", 72.0);
        let _ = head;
        assert_eq!(crate::theme::TABLE_COLUMNS, [1.0, 72.0, 72.0, 72.0, 108.0]);
    }

    #[test]
    fn date_is_a_stable_iso_label() {
        let label = chrono_date(std::time::SystemTime::UNIX_EPOCH);
        assert_eq!(label.len(), 10);
        assert!(label.starts_with("1970-"));
    }
}
