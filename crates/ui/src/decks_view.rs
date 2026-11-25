//! Decks screen: header, summary cards, five-column table, and empty states.

use std::collections::HashSet;

use gpui::{
    App, ClickEvent, Context, InteractiveElement, IntoElement, ParentElement,
    Rgba, SharedString, StatefulInteractiveElement, Styled, div, px,
};

use crate::{
    app::{self, Root, UiAction, due_total, leaf_deck_totals},
    components::{Interaction, icon, icon_tile, keycap},
    theme::{
        PAGE_X, PAGE_Y, SPACE_BLOCK, SPACE_GROUP, SPACE_INLINE, SPACE_RELATED,
        SPACE_SECTION, Style, TABLE_COLUMNS, alpha, blend, queue_text,
    },
};

impl Root {
    pub(crate) fn visible_deck_rows(&self, cx: &App) -> Vec<app::DeckRow> {
        let query = self.deck_search.read(cx).text();
        if query.trim().is_empty() {
            return self.deck_rows.clone();
        }
        let expanded = self
            .decks
            .iter()
            .map(|deck| deck.name.clone())
            .collect::<HashSet<_>>();
        let mut expanded = expanded;
        for deck in &self.decks {
            for (index, _) in deck.name.match_indices("::") {
                let _ = expanded.insert(deck.name[..index].to_owned());
            }
        }
        let rows = app::build_deck_rows(&self.decks, &self.stats, &expanded);
        app::filter_decks(&rows, &query)
    }

    pub(crate) fn render_decks(
        &mut self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = self.colors();
        let rows = self.visible_deck_rows(cx);
        let totals = leaf_deck_totals(&self.decks, &self.stats);
        let due = due_total(totals);

        let create_click = self.action_click(UiAction::CreateDeck);
        let mut content = div()
            .size_full()
            .flex()
            .flex_col()
            .id("decks-scroll")
            .gap(px(SPACE_SECTION))
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
                        .gap(px(SPACE_RELATED))
                        .child(
                            Interaction::raised(colors)
                                .apply(
                                    div()
                                        .id("decks-search")
                                        .flex()
                                        .items_center()
                                        .gap(px(SPACE_GROUP))
                                        .w(px(300.))
                                        .h(px(44.))
                                        .px(px(SPACE_RELATED))
                                        .rounded(px(12.))
                                        .bg(colors.surface0)
                                        .text_size(px(14.))
                                        .text_color(colors.text)
                                        .cursor_pointer(),
                                )
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
                            Interaction::new(
                                colors.surface1,
                                colors.surface0,
                                colors.surface1,
                                colors.mauve,
                            )
                            .apply(
                                div()
                                    .id("decks-create")
                                    .flex()
                                    .items_center()
                                    .gap(px(SPACE_GROUP))
                                    .h(px(44.))
                                    .px(px(SPACE_BLOCK))
                                    .rounded(px(12.))
                                    .text_size(px(14.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .cursor_pointer(),
                            )
                            .on_click(create_click)
                            .child(icon("icon-plus", 18., colors.text))
                            .child("Create deck"),
                        ),
                ),
        );

        content = content.child(self.render_summary(totals, cx));

        let kind = app::empty_kind(self.decks.len(), rows.len(), due);
        if !rows.is_empty() {
            let mut table = div()
                .flex()
                .flex_col()
                .flex_shrink_0()
                .rounded(px(20.))
                .bg(colors.mantle)
                .border_1()
                .border_color(colors.surface0)
                .overflow_hidden();
            table = table.child(
                div()
                    .flex()
                    .items_center()
                    .px(px(SPACE_SECTION))
                    .py(px(SPACE_RELATED))
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
        // Accent ink only reads on the mauve fill. With nothing due the pill
        // falls back to surface1, so its label and icon take the surface1 ink.
        let study_all_ink = if due > 0 {
            style.ink()
        } else {
            crate::theme::keycap_ink(colors)
        };
        let queue_stat = |label: &str, value: usize, color: Rgba| {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap(px(SPACE_INLINE))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(SPACE_GROUP))
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
            .gap(px(SPACE_SECTION))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .flex_col()
                    .p(px(SPACE_SECTION))
                    .rounded(px(24.))
                    .bg(colors.surface0)
                    .child(
                        div()
                            .flex()
                            .items_end()
                            .justify_between()
                            .gap(px(SPACE_BLOCK))
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
                                if due > 0 {
                                    Interaction::accent(style)
                                } else {
                                    Interaction::new(
                                        alpha(colors.text, 0.0),
                                        blend(
                                            colors.surface1,
                                            colors.surface0,
                                            0.5,
                                        ),
                                        blend(
                                            colors.surface1,
                                            colors.crust,
                                            0.14,
                                        ),
                                        colors.mauve,
                                    )
                                }
                                .apply(
                                    div()
                                        .id("decks-study-all")
                                        .flex()
                                        .flex_none()
                                        .items_center()
                                        .gap(px(SPACE_GROUP))
                                        .h(px(52.))
                                        .px(px(SPACE_SECTION))
                                        .rounded(px(26.))
                                        .bg(if due > 0 {
                                            colors.mauve
                                        } else {
                                            colors.surface1
                                        })
                                        .text_color(study_all_ink)
                                        .text_size(px(16.))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .cursor_pointer(),
                                )
                                .on_click(cx.listener(
                                    |this, _: &ClickEvent, _, cx| {
                                        this.study_all(cx);
                                    },
                                ))
                                .child(icon("icon-play", 18., study_all_ink))
                                .child("Study all"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(SPACE_BLOCK))
                            .mt(px(SPACE_BLOCK))
                            .pt(px(SPACE_BLOCK))
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
                    .p(px(SPACE_SECTION))
                    .rounded(px(24.))
                    .bg(colors.mantle)
                    .border_1()
                    .border_color(colors.surface0)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child("Next 7 days"),
                            )
                            .child(
                                div()
                                    .px(px(SPACE_RELATED))
                                    .py(px(SPACE_INLINE))
                                    .rounded(px(10.))
                                    .bg(colors.surface0)
                                    .text_size(px(12.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(colors.mauve)
                                    .child(format!(
                                        "{} due",
                                        app::forecast_total(&self.forecast)
                                    )),
                            ),
                    )
                    .child(if self.forecast.is_empty() {
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
                            )
                    } else {
                        let today = crate::statistics_view::day_number(
                            Self::now(),
                            self.collection_start,
                        );
                        let peak = self
                            .forecast
                            .iter()
                            .copied()
                            .max()
                            .unwrap_or_default();
                        let mut bars = div()
                            .mt(px(SPACE_BLOCK))
                            .flex()
                            .flex_1()
                            .gap(px(SPACE_GROUP));
                        for (index, count) in self.forecast.iter().enumerate() {
                            bars = bars.child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .flex()
                                    .flex_col()
                                    .items_center()
                                    .justify_end()
                                    .gap(px(SPACE_GROUP))
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .font_weight(
                                                gpui::FontWeight::SEMIBOLD,
                                            )
                                            .text_color(if *count == 0 {
                                                colors.overlay
                                            } else {
                                                colors.text
                                            })
                                            .child(format!("{count}")),
                                    )
                                    .child(
                                        div()
                                            .w_full()
                                            .h(px(app::forecast_bar_height(
                                                *count, peak,
                                            )))
                                            .rounded(px(8.))
                                            .bg(if *count == 0 {
                                                colors.surface1
                                            } else {
                                                colors.mauve
                                            }),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(10.))
                                            .text_color(if index == 0 {
                                                colors.mauve
                                            } else {
                                                colors.subtext
                                            })
                                            .child(
                                                crate::statistics_view::weekday(
                                                    today.saturating_add(
                                                        i64::try_from(index)
                                                            .unwrap_or(
                                                                i64::MAX,
                                                            ),
                                                    ),
                                                    self.collection_start,
                                                ),
                                            ),
                                    ),
                            );
                        }
                        bars
                    }),
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
        let meta = app::deck_meta(&self.deck_rows, row);
        let mut name_cell = div()
            .flex()
            .items_center()
            .gap(px(SPACE_RELATED))
            .pl(px(SPACE_SECTION * row.depth as f32))
            .flex_1();
        if row.depth > 0 {
            name_cell = name_cell.child(
                div().h(px(28.)).w(px(2.)).bg(colors.surface1).flex_none(),
            );
        }
        if row.has_children {
            let path = row.path.clone();
            name_cell = name_cell.child(
                Interaction::surface(colors)
                    .apply(
                        div()
                            .id(SharedString::from(format!(
                                "decks-chevron-{}",
                                row.path
                            )))
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(24.))
                            .h(px(24.))
                            .flex_none()
                            .cursor_pointer(),
                    )
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
        let mut label = div().flex().items_center().gap(px(SPACE_GROUP));
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
            && let Some(edit) = self.deck_edit.clone()
            && edit.id == deck_id
        {
            name_cell = name_cell.child(self.rename_field.clone()).child(
                Interaction::surface(colors)
                    .apply(
                        div()
                            .id("decks-save-rename")
                            .text_size(px(13.))
                            .text_color(colors.mauve)
                            .cursor_pointer(),
                    )
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
            .gap(px(SPACE_GROUP));
        if let Some(target) = index {
            if has_due {
                action = action.child(
                    Interaction::accent(style)
                        .apply(
                            div()
                                .id(SharedString::from(format!(
                                    "decks-study-{}",
                                    row.path
                                )))
                                .h(px(30.))
                                .px(px(SPACE_RELATED))
                                .flex()
                                .items_center()
                                .rounded(px(9.))
                                .bg(colors.mauve)
                                .text_color(style.ink())
                                .text_size(px(13.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .cursor_pointer(),
                        )
                        .on_click(cx.listener(
                            move |this, _: &ClickEvent, _, cx| {
                                this.study_deck(target, cx);
                            },
                        ))
                        .child("Study"),
                );
            }
            let id = row.id.unwrap_or(0);
            let menu_open = self.deck_menu.is_some_and(|menu| menu.id == id);
            action = action.child(
                Interaction::new(
                    alpha(colors.text, 0.0),
                    if menu_open {
                        alpha(colors.mauve, 0.24)
                    } else {
                        alpha(colors.mauve, 0.12)
                    },
                    if menu_open {
                        alpha(colors.mauve, 0.32)
                    } else {
                        alpha(colors.mauve, 0.22)
                    },
                    colors.mauve,
                )
                .apply(
                    div()
                        .id(SharedString::from(format!("decks-menu-{id}")))
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(30.))
                        .h(px(30.))
                        .rounded(px(8.))
                        .bg(if menu_open {
                            alpha(colors.mauve, 0.16)
                        } else {
                            alpha(colors.text, 0.0)
                        })
                        .cursor_pointer(),
                )
                .on_click(cx.listener(
                    move |this, event: &ClickEvent, _, cx| {
                        let same_press = match event {
                            gpui::ClickEvent::Mouse(click) => this
                                .deck_menu_dismissed_press
                                .as_ref()
                                .is_some_and(|down| {
                                    down.button == click.down.button
                                        && down.position == click.down.position
                                        && down.click_count
                                            == click.down.click_count
                                        && down.first_mouse
                                            == click.down.first_mouse
                                }),
                            gpui::ClickEvent::Keyboard(_) => false,
                        };
                        this.deck_menu_dismissed_press = None;
                        if !same_press {
                            this.deck_menu = if menu_open {
                                None
                            } else {
                                Some(app::DeckMenu {
                                    id,
                                    anchor: event.position(),
                                })
                            };
                        }
                        cx.notify();
                    },
                ))
                .child(icon("icon-more", 18., colors.overlay)),
            );
        }
        let hover_bg = if is_selected {
            crate::theme::blend(colors.surface0, colors.surface1, 0.5)
        } else {
            colors.surface0
        };
        let press_bg = if is_selected {
            crate::theme::blend(colors.surface1, colors.crust, 0.14)
        } else {
            crate::theme::blend(colors.surface0, colors.surface1, 0.7)
        };
        div()
            .id(SharedString::from(format!("deck-row-{}", row.path)))
            .flex()
            .items_center()
            .min_h(px(if row.depth == 0 { 60. } else { 45. }))
            .px(px(SPACE_SECTION))
            .border_t_1()
            .border_color(colors.surface0)
            .cursor_pointer()
            .focusable()
            .hover(move |style| style.bg(hover_bg))
            .active(move |style| style.bg(press_bg))
            .focus(move |style| {
                style
                    .bg(crate::theme::blend(hover_bg, colors.text, 0.08))
                    .border_color(colors.mauve)
            })
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if let Some(index) = index {
                    this.selected = Some(index);
                    this.rebuild_deck_rows();
                    cx.notify();
                }
            }))
            .bg(if is_selected {
                colors.surface0
            } else {
                colors.mantle
            })
            .child(name_cell)
            .child(count(row.stats.new_cards, queue_text(0, self.theme())))
            .child(count(row.stats.learning_cards, queue_text(1, self.theme())))
            .child(count(row.stats.review_cards, queue_text(2, self.theme())))
            .child(action)
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
            .gap(px(SPACE_GROUP))
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
                Interaction::accent(Style::new(colors, theme))
                    .apply(
                        div()
                            .id("decks-empty-action")
                            .mt(px(SPACE_RELATED))
                            .flex()
                            .items_center()
                            .h(px(44.))
                            .px(px(SPACE_SECTION))
                            .rounded(px(12.))
                            .bg(colors.mauve)
                            .text_color(crate::theme::accent_ink(theme))
                            .text_size(px(14.))
                            .cursor_pointer(),
                    )
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
