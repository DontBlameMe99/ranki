//! Decks screen: header, summary cards, five-column table, and empty states.

use std::collections::HashSet;

use gpui::{
    ClickEvent, Context, InteractiveElement, IntoElement, ParentElement, Rgba,
    StatefulInteractiveElement, Styled, Window, div, px,
};

use crate::{
    app::{self, Confirmation, Root, UiAction, due_total, leaf_deck_totals},
    components::{icon, icon_tile, keycap, small_button},
    theme::{PAGE_X, PAGE_Y, Style, TABLE_COLUMNS, queue_text},
};

impl Root {
    pub(crate) fn render_decks(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = self.colors();
        let style = Style::new(colors, self.theme());
        let query = self.deck_search.read(cx).text();
        let all_rows = if query.trim().is_empty() {
            self.deck_rows.clone()
        } else {
            let expanded = self
                .decks
                .iter()
                .map(|deck| deck.name.clone())
                .collect::<HashSet<_>>();
            app::build_deck_rows(&self.decks, &self.stats, &expanded)
        };
        let rows = app::filter_decks(&all_rows, &query);
        let totals = leaf_deck_totals(&self.decks, &self.stats);
        let due = due_total(totals);

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
                        .text_size(px(32.))
                        .font_weight(gpui::FontWeight::BOLD)
                        .child("Decks"),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .child(
                            div()
                                .id("decks-search")
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
                                .child(icon("icon-search", 18., colors.subtext))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.))
                                        .child(self.deck_search.clone()),
                                )
                                .child(keycap("/", colors, false)),
                        )
                        .child(
                            div()
                                .id("decks-create")
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .h(px(44.))
                                .px(px(16.))
                                .rounded(px(12.))
                                .border_1()
                                .border_color(colors.surface1)
                                .text_size(px(14.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .cursor_pointer()
                                .on_click(create_click)
                                .child(icon("icon-plus", 18., colors.text))
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
                        style,
                        self.action_click(UiAction::SubmitDeck),
                    ))
                    .child(
                        div()
                            .id("decks-create-cancel")
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

        content = content.child(self.render_summary(totals, cx));

        let kind = app::empty_kind(self.decks.len(), rows.len(), due);
        if !rows.is_empty() {
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
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(header_cell("DECK", colors.subtext, 0))
                    .child(header_cell("NEW", queue_text(0, self.theme()), 1))
                    .child(header_cell(
                        "LEARNING",
                        queue_text(1, self.theme()),
                        2,
                    ))
                    .child(header_cell(
                        "REVIEW",
                        queue_text(2, self.theme()),
                        3,
                    ))
                    .child(header_cell("", colors.subtext, 4)),
            );
            for row in &rows {
                table = table.child(self.render_deck_row(row, cx));
            }
            content = content.child(table);
        }
        if kind != app::EmptyKind::None {
            content = content.child(self.render_empty(kind));
        }
        content
    }

    fn render_summary(
        &mut self,
        totals: ranki_model::DeckStats,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = self.colors();
        let style = Style::new(colors, self.theme());
        let due = due_total(totals);
        let queue_stat = |label: &str, value: usize, color: Rgba| {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
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
        };
        div()
            .flex()
            .gap(px(20.))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .flex_col()
                    .p(px(24.))
                    .rounded(px(24.))
                    .bg(colors.surface0)
                    .child(
                        div()
                            .flex()
                            .items_end()
                            .justify_between()
                            .gap(px(16.))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .line_height(px(16.))
                                            .text_color(colors.subtext)
                                            .child("Due today"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(64.))
                                            .line_height(px(68.))
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .text_color(colors.mauve)
                                            .child(format!("{due}")),
                                    ),
                            )
                            .child(
                                div()
                                    .id("decks-study-all")
                                    .flex()
                                    .flex_none()
                                    .items_center()
                                    .gap(px(10.))
                                    .h(px(52.))
                                    .px(px(20.))
                                    .rounded(px(26.))
                                    .bg(if due > 0 {
                                        colors.mauve
                                    } else {
                                        colors.surface1
                                    })
                                    .text_color(style.ink())
                                    .text_size(px(16.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .cursor_pointer()
                                    .on_click(cx.listener(
                                        |this, _: &ClickEvent, _, cx| {
                                            this.study_all(cx);
                                        },
                                    ))
                                    .child(icon("icon-play", 18., style.ink()))
                                    .child("Study all"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(16.))
                            .mt(px(16.))
                            .pt(px(16.))
                            .border_t_1()
                            .border_color(colors.surface1)
                            .child(queue_stat(
                                "New",
                                totals.new_cards,
                                colors.blue,
                            ))
                            .child(queue_stat(
                                "Learning",
                                totals.learning_cards,
                                colors.red,
                            ))
                            .child(queue_stat(
                                "Review",
                                totals.review_cards,
                                colors.green,
                            )),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(320.))
                    .flex_none()
                    .p(px(24.))
                    .rounded(px(24.))
                    .bg(colors.mantle)
                    .border_1()
                    .border_color(colors.surface0)
                    .child(
                        div().flex().items_center().justify_between().child(
                            div()
                                .text_size(px(14.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("Next 7 days"),
                        ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(colors.overlay)
                                    .child("7-day forecast unavailable"),
                            ),
                    ),
            )
    }

    fn render_deck_row(
        &mut self,
        row: &app::DeckRow,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = self.colors();
        let style = Style::new(colors, self.theme());
        let index = row
            .id
            .and_then(|id| self.decks.iter().position(|deck| deck.id == id));
        let is_selected = index.is_some() && index == self.selected;
        let has_due = due_total(row.stats) > 0;
        let menu_open = row.id.is_some_and(|id| self.deck_menu == Some(id));
        let meta = app::deck_meta(&self.deck_rows, row);
        let mut name_cell = div()
            .flex()
            .items_center()
            .gap(px(12.))
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
                    .id("decks-chevron")
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(24.))
                    .h(px(24.))
                    .flex_none()
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
                        colors.overlay,
                    )),
            );
        } else {
            name_cell = name_cell.child(div().w(px(24.)).flex_none());
        }
        let mut label = div().flex().items_center().gap(px(10.));
        label = label.child(
            div()
                .text_size(px(if row.depth == 0 { 15. } else { 14.5 }))
                .font_weight(if row.depth == 0 {
                    gpui::FontWeight::SEMIBOLD
                } else {
                    gpui::FontWeight::MEDIUM
                })
                .font_family(crate::theme::font_for_text(&row.name))
                .child(row.name.clone()),
        );
        if row.depth > 0 && row.has_children {
            label = label.child(
                div()
                    .text_size(px(12.))
                    .font_weight(gpui::FontWeight::NORMAL)
                    .text_color(colors.subtext)
                    .child(meta.clone()),
            );
        }
        if row.depth == 0 {
            name_cell = name_cell.child(icon_tile(
                if row.has_children {
                    "icon-layers"
                } else {
                    "icon-book"
                },
                deck_hue(&row.path, colors),
            ));
            name_cell = name_cell.child(
                div().flex().flex_col().child(label).child(
                    div()
                        .text_size(px(12.5))
                        .text_color(colors.subtext)
                        .child(meta),
                ),
            );
        } else {
            name_cell = name_cell.child(label);
        }
        if let Some(deck_id) = row.id
            && menu_open
            && let Some(edit) = self.deck_edit.clone()
            && edit.id == deck_id
        {
            name_cell = name_cell.child(self.rename_field.clone()).child(
                div()
                    .id("decks-save-rename")
                    .text_size(px(13.))
                    .text_color(colors.mauve)
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.apply_rename(cx);
                    }))
                    .child("Save"),
            );
        }
        let count = |value: usize, color: Rgba| {
            div()
                .w(px(TABLE_COLUMNS[1]))
                .flex_none()
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
            .items_center()
            .justify_end()
            .gap(px(8.));
        if let Some(target) = index {
            if has_due {
                action = action.child(
                    div()
                        .id("decks-study")
                        .h(px(30.))
                        .px(px(14.))
                        .flex()
                        .items_center()
                        .rounded(px(9.))
                        .bg(colors.mauve)
                        .text_color(style.ink())
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
            }
            let id = row.id.unwrap_or(0);
            action = action.child(
                div()
                    .id("decks-menu")
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(30.))
                    .h(px(30.))
                    .rounded(px(8.))
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
                    .child(icon("icon-more", 18., colors.overlay)),
            );
        }
        let mut row_div = div()
            .flex()
            .items_center()
            .min_h(px(if row.depth == 0 { 60. } else { 45. }))
            .px(px(20.))
            .border_t_1()
            .border_color(colors.surface0)
            .bg(if is_selected {
                colors.surface0
            } else {
                colors.mantle
            })
            .child(name_cell)
            .child(count(row.stats.new_cards, queue_text(0, self.theme())))
            .child(count(row.stats.learning_cards, queue_text(1, self.theme())))
            .child(count(row.stats.review_cards, queue_text(2, self.theme())))
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
                            .id("decks-rename")
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
                            .id("decks-remove")
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
                    .id("decks-empty-action")
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

/// One table header cell. Every column keeps the exact width its row cell
/// uses, so counts stay under their labels and the action cell stays reserved.
fn header_cell(label: &str, color: Rgba, column: usize) -> gpui::Div {
    let mut cell = div()
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(color);
    if column == 0 {
        cell = cell.flex_1();
    } else {
        cell = cell
            .flex()
            .flex_none()
            .justify_end()
            .w(px(TABLE_COLUMNS[column]));
    }
    cell.child(label.to_owned())
}

/// Deterministic deck tint from the path, matching the mockup palette.
fn deck_hue(path: &str, colors: crate::theme::Palette) -> Rgba {
    let fingerprint = path
        .bytes()
        .fold(0usize, |sum, byte| sum.wrapping_add(byte as usize));
    match fingerprint % 3 {
        0 => colors.mauve,
        1 => colors.teal,
        _ => colors.peach,
    }
}

#[cfg(test)]
mod tests {
    use super::{deck_hue, header_cell};

    #[test]
    fn deck_header_cells_share_the_row_column_widths() {
        let head = header_cell("NEW", crate::theme::MOCHA.blue, 1);
        let _ = head;
        let columns = crate::theme::TABLE_COLUMNS;
        assert_eq!(columns[0], 1.0);
        assert_eq!(columns[1], columns[2]);
        assert_eq!(columns[2], columns[3]);
        assert!(columns[4] >= 120.0);
    }

    #[test]
    fn deck_tints_are_stable_and_varied() {
        let colors = crate::theme::MOCHA;
        assert_eq!(deck_hue("Japanese", colors), deck_hue("Japanese", colors));
        let hues = ["Default", "Japanese", "Spanish"]
            .map(|path| deck_hue(path, colors));
        assert!(hues.iter().any(|hue| *hue != hues[0]));
        let _ = &colors;
    }
}
