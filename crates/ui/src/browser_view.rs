//! Card browser: store-backed search, result table, and card preview.

use std::sync::Arc;

use crate::{
    app::{Root, UiAction},
    card_view::{self, CardRequest, MediaRef, RenderedCard},
    components::{chip, icon, secondary_button},
    text_field::{EditHook, TextField},
    theme::{PAGE_X, PAGE_Y, Style, interval_label},
};
use gpui::{
    ClickEvent, Context, Entity, InteractiveElement, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, Window,
    div, px,
};
use ranki_model::{Card, CardQueue, Deck, Id, Note, NoteModel};

/// Keeps the result table responsive on large collections.
pub const BROWSE_LIMIT: usize = 200;

/// One search result, with both sides rendered once for the table and preview.
#[derive(Clone, Debug)]
pub struct BrowserRow {
    pub card: Card,
    pub note: Note,
    pub front: String,
    pub back: String,
    pub front_media: Vec<MediaRef>,
    pub back_media: Vec<MediaRef>,
    pub deck: String,
    pub model: String,
}

/// Search state. The generation drops results from an older query.
pub struct BrowseState {
    pub search: Entity<TextField>,
    pub rows: Vec<BrowserRow>,
    pub total: usize,
    pub selected: Option<Id>,
    pub error: Option<String>,
    pub generation: u64,
    pub loading: bool,
}

/// Renders both sides of one card and resolves its deck and note type names.
#[must_use]
pub fn browser_row(
    card: &Card,
    note: &Note,
    decks: &[Deck],
    models: &[NoteModel],
) -> Option<BrowserRow> {
    let model = models.iter().find(|model| model.id == note.model_id)?;
    let deck = decks
        .iter()
        .find(|deck| deck.id == card.deck_id)
        .map_or_else(
            || format!("Deck {}", card.deck_id),
            |deck| deck.name.clone(),
        );
    let front = card_view::render_card(
        model,
        note,
        CardRequest::question(card.ordinal),
    );
    let back = card_view::render_card(
        model,
        note,
        CardRequest::answer_only(card.ordinal),
    );
    Some(BrowserRow {
        card: card.clone(),
        note: note.clone(),
        front: card_label(&front),
        front_media: front.media,
        back: card_label(&back),
        back_media: back.media,
        deck,
        model: model.name.clone(),
    })
}

/// Uses media names when a rendered side has no plain text, as with flag images.
fn card_label(rendered: &RenderedCard) -> String {
    if rendered.text.trim().is_empty() {
        rendered
            .media
            .iter()
            .map(|media| media.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        rendered.text.clone()
    }
}

/// Flattens rendered card text into a single bounded table cell.
#[must_use]
pub fn preview_line(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= 120 {
        return flat;
    }
    let mut short = flat.chars().take(117).collect::<String>();
    short.push_str("...");
    short
}

/// Short status of a card in the browser table.
#[must_use]
pub const fn queue_label(queue: CardQueue) -> &'static str {
    match queue {
        CardQueue::New => "New",
        CardQueue::Learning | CardQueue::DayLearning => "Learning",
        CardQueue::Review => "Review",
        CardQueue::Suspended => "Suspended",
        CardQueue::Buried => "Buried",
        CardQueue::Preview => "Preview",
    }
}

/// Due cell: the current interval for review cards, a dash for inactive ones.
#[must_use]
pub fn due_label(card: &Card) -> String {
    match card.queue {
        CardQueue::Review => interval_label(card.interval),
        CardQueue::Suspended | CardQueue::Buried | CardQueue::Preview => {
            String::from("-")
        }
        CardQueue::New | CardQueue::Learning | CardQueue::DayLearning => {
            String::from("Today")
        }
    }
}

/// Sorts by question text, then id, so the table order is stable.
pub fn sort_browser_rows(rows: &mut [BrowserRow]) {
    rows.sort_by(|left, right| {
        left.front
            .cmp(&right.front)
            .then_with(|| left.card.id.cmp(&right.card.id))
    });
}

impl Root {
    /// Refreshes the table after each edit, debounced by the search clock.
    pub(crate) fn browse_edit_hook(cx: &mut Context<Self>) -> EditHook {
        let weak = cx.entity().downgrade();
        Arc::new(move |app| {
            let weak = weak.clone();
            app.defer(move |app| {
                let _ = weak.update(app, |root, cx| {
                    root.schedule_browse_refresh(cx);
                    cx.notify();
                });
            });
        })
    }
    pub(crate) fn schedule_browse_refresh(&mut self, cx: &mut Context<Self>) {
        self.browse.generation = self.browse.generation.wrapping_add(1);
        let generation = self.browse.generation;
        self.browse.loading = true;
        self.browse.error = None;
        let timer = cx.background_executor().timer(super::app::DEBOUNCE);
        cx.spawn(async move |this, cx| {
            timer.await;
            let _ = this.update(cx, |root, cx| {
                if root.browse.generation == generation {
                    root.refresh_browse(cx);
                }
            });
        })
        .detach();
    }

    /// Reads the current query from the store and rebuilds the table.
    pub(crate) fn refresh_browse(&mut self, cx: &mut Context<Self>) {
        let query = self.browse.search.read(cx).text();
        let ids = match self.store.find_cards(&query) {
            Ok(ids) => ids,
            Err(error) => {
                self.browse.rows.clear();
                self.browse.total = 0;
                self.browse.selected = None;
                self.browse.error =
                    Some(format!("Could not search cards: {error}"));
                self.browse.loading = false;
                cx.notify();
                return;
            }
        };
        let total = ids.len();
        let mut rows = Vec::with_capacity(total.min(BROWSE_LIMIT));
        for id in ids {
            let Ok(Some(card)) = self.store.card(id) else {
                continue;
            };
            let Ok(Some(note)) = self.store.note(card.note_id) else {
                continue;
            };
            if let Some(row) =
                browser_row(&card, &note, &self.decks, &self.models)
            {
                rows.push(row);
            }
            if rows.len() >= BROWSE_LIMIT {
                break;
            }
        }
        sort_browser_rows(&mut rows);
        if !rows
            .iter()
            .any(|row| self.browse.selected == Some(row.card.id))
        {
            self.browse.selected = rows.first().map(|row| row.card.id);
        }
        self.browse.total = total;
        self.browse.rows = rows;
        self.browse.error = None;
        self.browse.loading = false;
        cx.notify();
    }

    pub(crate) fn select_browse_row(&mut self, id: Id, cx: &mut Context<Self>) {
        if self.browse.selected != Some(id) {
            self.browse.selected = Some(id);
            cx.notify();
        }
    }

    pub(crate) fn render_browse(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = self.colors();
        let style = Style::new(colors, self.theme());
        let query = self.browse.search.read(cx).text();
        let selected = self.browse.selected;
        let total = self.browse.total;
        let loading = self.browse.loading;
        let error = self.browse.error.clone();
        let search_focused = self
            .browse
            .search
            .read(cx)
            .focus_handle()
            .is_focused(window);
        let mut content = div()
            .id("browse-scroll")
            .size_full()
            .flex()
            .flex_col()
            .gap(px(20.))
            .px(px(PAGE_X))
            .py(px(PAGE_Y))
            .child(
                div()
                    .flex()
                    .items_end()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(32.))
                            .font_weight(gpui::FontWeight::BOLD)
                            .child("Browse"),
                    )
                    .child(
                        div()
                            .id("browse-search")
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .w(px(460.))
                            .h(px(44.))
                            .px(px(14.))
                            .rounded(px(12.))
                            .bg(colors.surface0)
                            .text_size(px(14.))
                            .text_color(colors.text)
                            .on_click(cx.listener(
                                |this, _: &ClickEvent, window, cx| {
                                    let field = this.browse.search.clone();
                                    window.focus(field.read(cx).focus_handle());
                                },
                            ))
                            .child(icon(
                                "icon-search",
                                18.,
                                if search_focused {
                                    colors.mauve
                                } else {
                                    colors.subtext
                                },
                            ))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .child(self.browse.search.clone()),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_size(px(12.))
                    .text_color(colors.subtext)
                    .child(browse_status(
                        loading,
                        error.as_deref(),
                        self.browse.rows.len(),
                        total,
                    ))
                    .child(
                        div()
                            .child("Try deck:Default, tag:review, or is:new")
                            .font_family(self.font_family),
                    ),
            );

        let mut table = div()
            .flex_1()
            .min_w(px(0.))
            .id("browse-table")
            .flex()
            .flex_col()
            .rounded(px(20.))
            .bg(colors.mantle)
            .border_1()
            .border_color(colors.surface0)
            .overflow_scroll();
        table = table.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(16.))
                .py(px(12.))
                .text_size(px(12.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.subtext)
                .child(browse_cell("QUESTION", 0, colors))
                .child(browse_cell("DECK", 1, colors))
                .child(browse_cell("TYPE", 2, colors))
                .child(browse_cell("STATE", 3, colors))
                .child(browse_cell("DUE", 4, colors)),
        );
        for row in &self.browse.rows {
            let id = row.card.id;
            let active = selected == Some(id);
            let state = queue_label(row.card.queue);
            let color = crate::theme::queue_color(
                u8::try_from(row.card.queue.code()).unwrap_or(3),
                self.theme(),
            );
            let mut line = div()
                .id(SharedString::from(format!("browse-row-{id}")))
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(52.))
                .px(px(16.))
                .border_t_1()
                .border_color(colors.surface0)
                .bg(if active {
                    colors.surface0
                } else {
                    colors.mantle
                })
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.select_browse_row(id, cx);
                }));
            if active {
                line = line.border_l_2().border_color(colors.mauve);
            }
            line = line
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(14.))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child(preview_line(&row.front)),
                )
                .child(browse_cell(row.deck.clone(), 1, colors))
                .child(browse_cell(row.model.clone(), 2, colors))
                .child(
                    browse_cell(state, 3, colors)
                        .text_color(color)
                        .font_weight(gpui::FontWeight::SEMIBOLD),
                )
                .child(browse_cell(due_label(&row.card), 4, colors));
            table = table.child(line);
        }
        if self.browse.rows.is_empty() {
            table = table.child(self.render_browse_empty(
                error.as_deref(),
                &query,
                style,
            ));
        }
        content = content.child(
            div()
                .flex()
                .flex_1()
                .min_h(px(0.))
                .gap(px(20.))
                .child(table)
                .child(self.render_browse_detail(selected, style)),
        );
        gpui::image_cache(self.images.clone()).child(content)
    }
    fn render_browse_empty(
        &self,
        error: Option<&str>,
        query: &str,
        style: Style,
    ) -> impl IntoElement {
        let colors = style.colors;
        let (title, detail) = if let Some(error) = error {
            ("Could not search", error)
        } else if query.trim().is_empty() {
            (
                "No cards yet",
                "Add a card to start browsing your collection",
            )
        } else {
            ("No cards match", "Try a different search")
        };
        let mut empty = div()
            .flex()
            .flex_1()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(12.))
            .py(px(48.))
            .child(
                div()
                    .text_size(px(18.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(colors.subtext)
                    .child(detail.to_owned()),
            );
        if error.is_none() && !query.trim().is_empty() {
            empty = empty.child(secondary_button(
                "Clear search",
                colors,
                self.action_click(UiAction::ClearBrowseSearch),
            ));
        } else if error.is_none() {
            empty = empty.child(secondary_button(
                "Add card",
                colors,
                self.action_click(UiAction::OpenAdd),
            ));
        }
        empty
    }
    fn render_browse_detail(
        &self,
        selected: Option<Id>,
        style: Style,
    ) -> impl IntoElement {
        let colors = style.colors;
        let mut detail = div()
            .w(px(BROWSE_PREVIEW_WIDTH))
            .id("browse-detail")
            .flex()
            .flex_col()
            .gap(px(16.))
            .p(px(24.))
            .rounded(px(20.))
            .bg(colors.mantle)
            .border_1()
            .border_color(colors.surface0)
            .overflow_scroll();
        let Some(row) = self
            .browse
            .rows
            .iter()
            .find(|row| Some(row.card.id) == selected)
        else {
            return detail.child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(13.))
                    .text_color(colors.subtext)
                    .child("Select a card to preview it"),
            );
        };
        detail = detail
            .child(
                div()
                    .text_size(px(16.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Card preview"),
            )
            .child(browse_meta("Deck", &row.deck, colors))
            .child(browse_meta("Note type", &row.model, colors))
            .child(
                div()
                    .text_size(px(11.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(colors.subtext)
                    .child("QUESTION"),
            )
            .child(div().text_size(px(15.)).child(row.front.clone()))
            .children(card_view::media_column(
                &row.front_media,
                &self.options.media_dir,
                colors,
                180.0,
            ))
            .child(div().h(px(1.)).bg(colors.surface0))
            .child(
                div()
                    .text_size(px(11.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(colors.subtext)
                    .child("ANSWER"),
            )
            .child(div().text_size(px(15.)).child(row.back.clone()))
            .children(card_view::media_column(
                &row.back_media,
                &self.options.media_dir,
                colors,
                180.0,
            ));
        if !row.note.tags.is_empty() {
            detail =
                detail.child(div().flex().flex_wrap().gap(px(6.)).children(
                    row.note.tags.iter().map(|tag| chip(tag.clone(), colors)),
                ));
        }
        detail.child(browse_meta("Reviews", &row.card.reps.to_string(), colors))
    }
}

fn browse_status(
    loading: bool,
    error: Option<&str>,
    shown: usize,
    total: usize,
) -> String {
    if loading {
        return String::from("Searching...");
    }
    if let Some(error) = error {
        return error.to_owned();
    }
    if total > shown {
        return format!("Showing {shown} of {total} cards");
    }
    format!("{total} cards")
}

fn browse_cell(
    text: impl Into<SharedString>,
    column: usize,
    colors: crate::theme::Palette,
) -> gpui::Div {
    let mut cell = div()
        .flex_none()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .text_size(px(12.))
        .text_color(colors.subtext)
        .child(text.into());
    if column == 0 {
        cell = cell.flex_1().min_w(px(0.));
    } else {
        cell = cell.w(px(BROWSE_COLUMNS[column]));
    }
    cell
}

fn browse_meta(
    label: &str,
    value: &str,
    colors: crate::theme::Palette,
) -> gpui::Div {
    div()
        .flex()
        .justify_between()
        .gap(px(12.))
        .text_size(px(12.))
        .child(div().text_color(colors.subtext).child(label.to_owned()))
        .child(div().child(value.to_owned()))
}

const BROWSE_PREVIEW_WIDTH: f32 = 320.;
const BROWSE_COLUMNS: [f32; 5] = [1.0, 130., 90., 85., 60.];

#[cfg(test)]
mod tests {
    use ranki_model::{
        Card, CardQueue, CardTemplate, CardType, DEFAULT_DECK_ID,
        DEFAULT_MODEL_ID, Deck, Id, Note, NoteModel, StockModelKind,
    };

    use super::{
        browse_status, browser_row, due_label, preview_line, queue_label,
        sort_browser_rows,
    };

    fn card_with_id(id: Id, queue: CardQueue, interval: i32) -> Card {
        Card {
            id,
            note_id: 1,
            deck_id: DEFAULT_DECK_ID,
            ordinal: 0,
            modified: 0,
            usn: 0,
            card_type: CardType::New,
            queue,
            due: 0,
            interval,
            factor: 0,
            reps: 0,
            lapses: 0,
            left: 0,
            original_due: 0,
            original_deck_id: 0,
            flags: 0,
            data: String::new(),
        }
    }

    fn model() -> NoteModel {
        NoteModel {
            id: DEFAULT_MODEL_ID,
            name: "Basic".to_owned(),
            fields: vec!["Front".to_owned(), "Back".to_owned()],
            templates: vec![CardTemplate {
                name: "Card 1".to_owned(),
                question_format: "{{Front}}".to_owned(),
                answer_format: "{{FrontSide}}<hr id=answer>{{Back}}".to_owned(),
                ord: 0,
            }],
            ..NoteModel::default()
        }
    }

    #[test]
    fn preview_line_flattens_and_bounds_rendered_text() {
        assert_eq!(preview_line(" One\n two "), "One two");
        assert_eq!(preview_line(&"word ".repeat(40)).chars().count(), 120);
    }

    #[test]
    fn queue_and_due_labels_cover_every_state() {
        assert_eq!(queue_label(CardQueue::New), "New");
        assert_eq!(queue_label(CardQueue::DayLearning), "Learning");
        assert_eq!(queue_label(CardQueue::Buried), "Buried");
        assert_eq!(due_label(&card_with_id(1, CardQueue::Review, 12)), "12d");
        assert_eq!(due_label(&card_with_id(1, CardQueue::Suspended, 12)), "-");
        assert_eq!(due_label(&card_with_id(1, CardQueue::New, 0)), "Today");
    }

    #[test]
    fn browser_rows_sort_by_question_then_id() {
        let deck = Deck {
            id: DEFAULT_DECK_ID,
            name: "Default".to_owned(),
            description: String::new(),
        };
        let model = model();
        let mut rows = Vec::new();
        for (id, front) in [(3, "Zebra"), (1, "Apple"), (2, "Apple")] {
            let note = Note {
                id,
                guid: String::new(),
                model_id: DEFAULT_MODEL_ID,
                modified: 0,
                usn: 0,
                tags: Vec::new(),
                fields: vec![front.to_owned(), "Answer".to_owned()],
                sort_field: front.to_owned(),
                checksum: 0,
                flags: 0,
                data: String::new(),
            };
            rows.push(
                browser_row(
                    &card_with_id(id, CardQueue::New, 0),
                    &note,
                    std::slice::from_ref(&deck),
                    std::slice::from_ref(&model),
                )
                .unwrap(),
            );
        }
        sort_browser_rows(&mut rows);
        let ids = rows.iter().map(|row| row.card.id).collect::<Vec<_>>();
        assert_eq!(ids, vec![1, 2, 3]);
        assert_eq!(rows[0].front, "Apple");
        assert_eq!(rows[0].back, "Answer");
        assert_eq!(rows[0].deck, "Default");
    }

    #[test]
    fn status_reports_loading_errors_and_truncated_results() {
        assert_eq!(browse_status(true, None, 0, 0), "Searching...");
        assert_eq!(browse_status(false, Some("bad query"), 0, 0), "bad query");
        assert_eq!(browse_status(false, None, 3, 10), "Showing 3 of 10 cards");
        assert_eq!(browse_status(false, None, 3, 3), "3 cards");
    }

    #[test]
    fn cloze_cards_show_the_active_cloze_on_the_front() {
        let deck = Deck {
            id: DEFAULT_DECK_ID,
            name: "Default".to_owned(),
            description: String::new(),
        };
        let mut model = StockModelKind::Cloze.definition();
        model.id = DEFAULT_MODEL_ID;
        let note = Note {
            id: 4,
            guid: String::new(),
            model_id: DEFAULT_MODEL_ID,
            modified: 0,
            usn: 0,
            tags: Vec::new(),
            fields: vec![
                "The {{c1::capital}} is Paris.".to_owned(),
                "Remember it".to_owned(),
            ],
            sort_field: String::new(),
            checksum: 0,
            flags: 0,
            data: String::new(),
        };
        let row = browser_row(
            &card_with_id(4, CardQueue::New, 0),
            &note,
            std::slice::from_ref(&deck),
            std::slice::from_ref(&model),
        )
        .unwrap();
        assert_eq!(row.front, "The [...] is Paris.");
    }

    #[test]
    fn browser_rows_keep_image_media_for_preview() {
        let deck = Deck {
            id: DEFAULT_DECK_ID,
            name: "Default".to_owned(),
            description: String::new(),
        };
        let note = Note {
            id: 5,
            guid: String::new(),
            model_id: DEFAULT_MODEL_ID,
            modified: 0,
            usn: 0,
            tags: Vec::new(),
            fields: vec![
                "<img src=\"Abkhazia.png\" />".to_owned(),
                "Abkhazia".to_owned(),
            ],
            sort_field: String::new(),
            checksum: 0,
            flags: 0,
            data: String::new(),
        };
        let row = browser_row(
            &card_with_id(5, CardQueue::New, 0),
            &note,
            std::slice::from_ref(&deck),
            std::slice::from_ref(&model()),
        )
        .unwrap();
        assert_eq!(row.front, "Abkhazia.png");
        assert_eq!(row.front_media.len(), 1);
        assert_eq!(
            row.front_media.first().map(|media| media.name.as_str()),
            Some("Abkhazia.png")
        );
        assert_eq!(row.back_media.len(), 0);
    }
}
