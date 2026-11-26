//! Statistics screen: a review heatmap, a seven-day forecast, and card counts.

use std::{collections::HashMap, time::Duration};

use gpui::{
    Context, Div, ElementId, InteractiveElement, IntoElement, ParentElement,
    Rgba, StatefulInteractiveElement, Styled, div, px,
};
use ranki_model::{Rating, RevLog, RevLogKind};

use crate::{
    app::{Root, date_label, duration_label},
    components::{icon, panel},
    theme::{
        PAGE_X, PAGE_Y, SPACE_BLOCK, SPACE_GROUP, SPACE_GUTTER, SPACE_INLINE,
        SPACE_SECTION, Style, alpha, blend, queue_text,
    },
};

const SECONDS_PER_DAY: i64 = 86_400;
const DAYS: usize = 7;
/// Six months of review days plus one partial week for Sunday alignment.
const HEATMAP_WEEKS: usize = 24;
pub const HEATMAP_DAYS: usize = HEATMAP_WEEKS * 7;
const HEATMAP_COLUMNS: usize = HEATMAP_WEEKS + 1;
const HEATMAP_CELL: f32 = 26.0;
const HEATMAP_GAP: f32 = 4.0;
const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// One day of the review heatmap, oldest first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DayReviews {
    pub day: i64,
    pub reviews: usize,
    pub millis: u64,
    /// `Again`, `Hard`, `Good`, and `Easy` counts.
    pub ratings: [usize; 4],
}

impl DayReviews {
    /// Percentage of a day's answers that were not Again.
    #[must_use]
    pub fn accuracy_percent(&self) -> Option<u8> {
        if self.reviews == 0 {
            return None;
        }
        let correct = self.reviews.saturating_sub(self.ratings[0]);
        let percent = correct.saturating_mul(100) / self.reviews;
        u8::try_from(percent).ok()
    }
}

/// Review history aggregated for the statistics screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Statistics {
    pub reviews_today: usize,
    pub millis_today: u64,
    /// `Again`, `Hard`, `Good`, and `Easy` counts for today.
    pub ratings_today: [usize; 4],
    /// Days ending today, oldest first.
    pub days: [DayReviews; HEATMAP_DAYS],
    pub streak: usize,
    pub total_reviews: usize,
    pub total_millis: u64,
}

impl Default for Statistics {
    fn default() -> Self {
        Self {
            reviews_today: 0,
            millis_today: 0,
            ratings_today: [0; 4],
            days: [DayReviews::default(); HEATMAP_DAYS],
            streak: 0,
            total_reviews: 0,
            total_millis: 0,
        }
    }
}

impl Statistics {
    /// Percentage of today's answers that were not Again.
    #[must_use]
    pub fn accuracy_percent(&self) -> Option<u8> {
        if self.reviews_today == 0 {
            return None;
        }
        let again = self.ratings_today[0];
        let correct = self.reviews_today.saturating_sub(again);
        let percent = correct.saturating_mul(100) / self.reviews_today;
        u8::try_from(percent).ok()
    }
}

/// Anki numbers days from the collection's creation time, not from midnight.
#[must_use]
pub fn day_number(timestamp_secs: i64, collection_start_secs: i64) -> i64 {
    timestamp_secs
        .saturating_sub(collection_start_secs)
        .div_euclid(SECONDS_PER_DAY)
}

/// Index of a day in the Sunday-first week grid.
#[must_use]
pub fn weekday_index(day: i64, collection_start_secs: i64) -> usize {
    let absolute = collection_start_secs
        .div_euclid(SECONDS_PER_DAY)
        .saturating_add(day);
    let index = absolute.saturating_add(4).rem_euclid(7);
    usize::try_from(index).unwrap_or(0)
}

/// Sunday of the week containing the first heatmap day.
#[must_use]
pub fn heatmap_start(first_day: i64, collection_start_secs: i64) -> i64 {
    first_day.saturating_sub(
        i64::try_from(weekday_index(first_day, collection_start_secs))
            .unwrap_or(0),
    )
}

/// Weekday name for a heatmap day, using the Unix epoch as the anchor.
#[must_use]
pub fn weekday(day: i64, collection_start_secs: i64) -> &'static str {
    WEEKDAYS
        .get(weekday_index(day, collection_start_secs))
        .copied()
        .unwrap_or("Sun")
}

/// Calendar label for a heatmap day.
#[must_use]
pub fn day_label(day: i64, collection_start_secs: i64) -> String {
    let seconds = collection_start_secs
        .saturating_add(day.saturating_mul(SECONDS_PER_DAY));
    date_label(u64::try_from(seconds).unwrap_or_default())
}

/// One bar in the forecast chart: past reviews or upcoming dues.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ForecastDay {
    pub day: i64,
    pub reviews: usize,
    pub due: Option<usize>,
}

/// Joins seven days of reviews with seven days of scheduled dues. Both halves
/// include today, so the last history bar and the first forecast bar share a
/// day but show different counts.
#[must_use]
pub fn forecast_days(
    stats: &Statistics,
    due: &[usize],
) -> [ForecastDay; DAYS * 2] {
    let mut days = [ForecastDay::default(); DAYS * 2];
    for (index, history) in
        stats.days.iter().skip(HEATMAP_DAYS - DAYS).enumerate()
    {
        days[index] = ForecastDay {
            day: history.day,
            reviews: history.reviews,
            due: None,
        };
    }
    let today = stats.days.last().map_or(0, |day| day.day);
    for index in 0..DAYS {
        days[DAYS + index] = ForecastDay {
            day: today.saturating_add(i64::try_from(index).unwrap_or(0)),
            reviews: 0,
            due: due.get(index).copied(),
        };
    }
    days
}

/// Aggregates a full review log. Manual log entries are scheduling changes,
/// not answers, so they stay out of every count.
#[must_use]
pub fn build_statistics(
    history: &[RevLog],
    now_secs: i64,
    collection_start_secs: i64,
) -> Statistics {
    let today = day_number(now_secs, collection_start_secs);
    let mut days = [DayReviews::default(); HEATMAP_DAYS];
    for (index, day) in days.iter_mut().enumerate() {
        day.day = today.saturating_sub(
            i64::try_from(HEATMAP_DAYS - 1 - index).unwrap_or(0),
        );
    }

    let mut per_day: HashMap<i64, usize> = HashMap::new();
    let mut stats = Statistics::default();
    for log in history {
        if log.kind == RevLogKind::Manual {
            continue;
        }
        let day = day_number(log.id / 1000, collection_start_secs);
        let millis = u64::from(log.time_ms);
        *per_day.entry(day).or_default() += 1;
        stats.total_reviews += 1;
        stats.total_millis = stats.total_millis.saturating_add(millis);

        let rating = match log.ease {
            Rating::Again => 0,
            Rating::Hard => 1,
            Rating::Good => 2,
            Rating::Easy => 3,
        };
        let offset = today.saturating_sub(day);
        if day <= today && offset < i64::try_from(HEATMAP_DAYS).unwrap_or(168) {
            let index = HEATMAP_DAYS
                - 1
                - usize::try_from(offset).unwrap_or(HEATMAP_DAYS);
            let bucket = &mut days[index];
            bucket.reviews += 1;
            bucket.millis = bucket.millis.saturating_add(millis);
            bucket.ratings[rating] += 1;
        }
        if day == today {
            stats.ratings_today[rating] += 1;
            stats.reviews_today += 1;
            stats.millis_today = stats.millis_today.saturating_add(millis);
        }
    }
    stats.days = days;

    let mut cursor = today;
    if !per_day.contains_key(&cursor) {
        cursor = cursor.saturating_sub(1);
    }
    while per_day.get(&cursor).is_some_and(|&count| count > 0) {
        stats.streak += 1;
        cursor = cursor.saturating_sub(1);
    }
    stats
}

impl Root {
    pub(crate) fn render_statistics(
        &self,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = self.colors();
        let style = Style::new(colors, self.theme());
        let stats = &self.statistics;
        let totals = self.leaf_totals;
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(SPACE_SECTION))
            .id("statistics-scroll")
            .overflow_scroll()
            .px(px(PAGE_X))
            .py(px(PAGE_Y))
            .child(
                div()
                    .text_size(px(32.))
                    .font_weight(gpui::FontWeight::BOLD)
                    .child("Statistics"),
            )
            .child(self.render_activity(stats, style, cx))
            .child(self.render_history(stats, style))
            .child(Self::render_card_counts(totals, style))
    }

    fn render_activity(
        &self,
        stats: &Statistics,
        style: Style,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = style.colors;
        let collection_start = self.collection_start;
        let today = stats.days.last().map_or(0, |day| day.day);
        let oldest = stats.days.first().map_or(0, |day| day.day);
        let start = heatmap_start(oldest, collection_start);
        let peak = stats
            .days
            .iter()
            .map(|day| day.reviews)
            .max()
            .unwrap_or_default();
        let total = stats
            .days
            .iter()
            .fold(0_usize, |sum, day| sum.saturating_add(day.reviews));
        let level_fill = |level: u8| {
            if level == 0 {
                alpha(colors.surface1, 0.55)
            } else {
                blend(colors.surface1, colors.mauve, f32::from(level) / 4.0)
            }
        };

        let mut legend =
            div().flex().items_center().gap(px(SPACE_INLINE)).child(
                div()
                    .text_size(px(11.))
                    .text_color(colors.overlay)
                    .child("Less"),
            );
        for level in 0..=4 {
            legend = legend.child(
                div()
                    .w(px(12.))
                    .h(px(12.))
                    .rounded(px(3.))
                    .bg(level_fill(level)),
            );
        }
        legend = legend.child(
            div()
                .text_size(px(11.))
                .text_color(colors.overlay)
                .child("More"),
        );

        let mut labels =
            div().flex().flex_col().gap(px(HEATMAP_GAP)).w(px(32.));
        for label in WEEKDAYS {
            labels = labels.child(
                div()
                    .h(px(HEATMAP_CELL))
                    .flex()
                    .items_center()
                    .text_size(px(11.))
                    .text_color(colors.subtext)
                    .child(label),
            );
        }

        let mut columns = div().flex().gap(px(HEATMAP_GAP));
        for week in 0..HEATMAP_COLUMNS {
            let mut column = div().flex().flex_col().gap(px(HEATMAP_GAP));
            for row in 0..7 {
                let index = week * 7 + row;
                let day =
                    start.saturating_add(i64::try_from(index).unwrap_or(0));
                let entry = if day >= oldest && day <= today {
                    usize::try_from(today.saturating_sub(day))
                        .ok()
                        .filter(|&offset| offset < HEATMAP_DAYS)
                        .and_then(|offset| {
                            stats.days.get(HEATMAP_DAYS - 1 - offset)
                        })
                } else {
                    None
                };
                let reviews = entry.map_or(0, |day| day.reviews);
                let level = if reviews == 0 || peak == 0 {
                    0
                } else {
                    u8::try_from(reviews.saturating_mul(4) / peak)
                        .unwrap_or(4)
                        .clamp(1, 4)
                };
                let fill = if entry.is_none() {
                    alpha(colors.surface1, 0.2)
                } else {
                    level_fill(level)
                };
                let border = if self.heatmap_hover == Some(day) {
                    colors.text
                } else if day == today {
                    colors.peach
                } else {
                    alpha(colors.text, 0.0)
                };
                let mut cell = div()
                    .id(ElementId::named_usize("statistics-heatmap", index))
                    .w(px(HEATMAP_CELL))
                    .h(px(HEATMAP_CELL))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(border)
                    .bg(fill);
                if entry.is_some() {
                    cell = cell.on_hover(cx.listener(
                        move |root, hovered: &bool, _, cx| {
                            if *hovered {
                                root.heatmap_hover = Some(day);
                                cx.notify();
                            } else if root.heatmap_hover == Some(day) {
                                root.heatmap_hover = None;
                                cx.notify();
                            }
                        },
                    ));
                }
                column = column.child(cell);
            }
            columns = columns.child(column);
        }

        let detail = if let Some(entry) = self
            .heatmap_hover
            .and_then(|day| stats.days.iter().find(|entry| entry.day == day))
        {
            let accuracy = entry
                .accuracy_percent()
                .map_or_else(|| String::from("-"), |value| format!("{value}%"));
            div()
                .flex()
                .items_center()
                .gap(px(SPACE_GROUP))
                .child(icon("icon-chart", 14., colors.mauve))
                .child(div().text_size(px(13.)).text_color(colors.text).child(
                    format!(
                        "{} · {} reviews · {} accuracy · {}",
                        day_label(entry.day, collection_start),
                        entry.reviews,
                        accuracy,
                        duration_label(Duration::from_millis(entry.millis)),
                    ),
                ))
        } else {
            div()
                .text_size(px(13.))
                .text_color(colors.subtext)
                .child("Hover a day to see its reviews")
        };

        let accuracy = stats
            .accuracy_percent()
            .map_or_else(|| String::from("-"), |value| format!("{value}%"));
        let metric = |label: &str, value: Div, hue: Rgba| {
            div()
                .flex()
                .flex_col()
                .gap(px(SPACE_INLINE))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(colors.subtext)
                        .child(label.to_owned()),
                )
                .child(value.text_size(px(22.)).text_color(hue))
        };

        panel(colors)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Review activity"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(SPACE_SECTION))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(colors.subtext)
                                    .child(format!(
                                        "{HEATMAP_DAYS} days · {total} reviews"
                                    )),
                            )
                            .child(legend),
                    ),
            )
            .child(
                div()
                    .flex()
                    .justify_center()
                    .gap(px(SPACE_GROUP))
                    .child(labels)
                    .child(columns),
            )
            .child(detail)
            .child(
                div()
                    .flex()
                    .gap(px(SPACE_GUTTER))
                    .border_t_1()
                    .border_color(colors.surface1)
                    .pt(px(SPACE_BLOCK))
                    .child(metric(
                        "Accuracy",
                        div()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(accuracy),
                        colors.green,
                    ))
                    .child(metric(
                        "Time spent learning",
                        div().font_weight(gpui::FontWeight::SEMIBOLD).child(
                            duration_label(Duration::from_millis(
                                stats.millis_today,
                            )),
                        ),
                        colors.text,
                    ))
                    .child(metric(
                        "Streak",
                        div()
                            .flex()
                            .items_center()
                            .gap(px(SPACE_INLINE))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(icon("icon-flame", 20., colors.peach))
                            .child(format!(
                                "{} day{}",
                                stats.streak,
                                if stats.streak == 1 { "" } else { "s" }
                            )),
                        colors.peach,
                    )),
            )
    }

    fn render_history(
        &self,
        stats: &Statistics,
        style: Style,
    ) -> impl IntoElement {
        let colors = style.colors;
        let collection_start = self.collection_start;
        let days = forecast_days(stats, &self.forecast);
        let peak = days
            .iter()
            .map(|day| day.due.unwrap_or(day.reviews))
            .max()
            .unwrap_or_default();
        let mut bars = div().flex().flex_1().items_end().gap(px(SPACE_BLOCK));
        for (index, day) in days.iter().enumerate() {
            let future = index >= DAYS;
            let count = if future { day.due } else { Some(day.reviews) };
            let value = count.unwrap_or_default();
            let height = if peak == 0 || value == 0 {
                6.0
            } else {
                20.0 + 100.0 * value as f32 / peak as f32
            };
            let hue = if value == 0 {
                colors.surface1
            } else if future {
                colors.green
            } else {
                colors.mauve
            };
            let label = count
                .map_or_else(|| String::from("-"), |value| value.to_string());
            bars = bars.child(
                div()
                    .flex()
                    .flex_1()
                    .flex_col()
                    .items_center()
                    .justify_end()
                    .gap(px(SPACE_GROUP))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(if count.is_none() {
                                colors.overlay
                            } else {
                                colors.text
                            })
                            .child(label),
                    )
                    .child(
                        div().w(px(48.)).h(px(height)).rounded(px(8.)).bg(hue),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(if index == DAYS - 1 {
                                colors.mauve
                            } else if index == DAYS {
                                colors.green
                            } else {
                                colors.subtext
                            })
                            .child(weekday(day.day, collection_start)),
                    ),
            );
        }
        panel(colors)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Forecast"),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(colors.subtext)
                            .child(format!(
                                "{} reviews  {} studied",
                                stats.total_reviews,
                                duration_label(Duration::from_millis(
                                    stats.total_millis,
                                ))
                            )),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(SPACE_SECTION))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(SPACE_GROUP))
                            .text_size(px(12.))
                            .text_color(colors.subtext)
                            .child(crate::components::dot(colors.mauve, 8.))
                            .child("Reviews, last 7 days"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(SPACE_GROUP))
                            .text_size(px(12.))
                            .text_color(colors.subtext)
                            .child(crate::components::dot(colors.green, 8.))
                            .child("Due, next 7 days"),
                    ),
            )
            .child(bars.h(px(150.)))
    }

    fn render_card_counts(
        totals: ranki_model::DeckStats,
        style: Style,
    ) -> impl IntoElement {
        let colors = style.colors;
        let count = |label: &str, value: usize, hue: Rgba| {
            div()
                .flex()
                .flex_1()
                .flex_col()
                .gap(px(SPACE_INLINE))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(SPACE_GROUP))
                        .text_size(px(12.))
                        .text_color(colors.subtext)
                        .child(crate::components::dot(hue, 8.))
                        .child(label.to_owned()),
                )
                .child(
                    div()
                        .text_size(px(20.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(format!("{value}")),
                )
        };
        panel(colors)
            .child(
                div()
                    .text_size(px(16.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Cards"),
            )
            .child(
                div()
                    .flex()
                    .gap(px(SPACE_SECTION))
                    .child(count(
                        "New",
                        totals.new_cards,
                        queue_text(0, style.theme),
                    ))
                    .child(count(
                        "Learning",
                        totals.learning_cards,
                        queue_text(1, style.theme),
                    ))
                    .child(count(
                        "Review",
                        totals.review_cards,
                        queue_text(2, style.theme),
                    ))
                    .child(count("Total", totals.total_cards, colors.mauve)),
            )
    }
}

#[cfg(test)]
mod tests {
    use ranki_model::{Rating, RevLog, RevLogKind};

    use super::{
        ForecastDay, HEATMAP_COLUMNS, HEATMAP_DAYS, build_statistics,
        day_label, day_number, forecast_days, heatmap_start, weekday,
        weekday_index,
    };

    const START: i64 = 1_700_000_000;

    fn log(day: i64, rating: Rating, millis: u32) -> RevLog {
        RevLog {
            id: (START + day * 86_400) * 1000,
            card_id: 1,
            usn: 0,
            ease: rating,
            interval: 1,
            last_interval: 0,
            factor: 2500,
            time_ms: millis,
            kind: RevLogKind::Review,
        }
    }

    #[test]
    fn days_follow_the_collection_creation_clock() {
        assert_eq!(day_number(START + 86_401, START), 1);
        assert_eq!(day_number(START - 86_399, START), -1);
        assert_eq!(weekday(0, START), "Tue");
    }

    #[test]
    fn today_counts_ratings_time_and_accuracy() {
        let logs = [
            log(3, Rating::Good, 1000),
            log(3, Rating::Again, 500),
            log(3, Rating::Easy, 1500),
            log(2, Rating::Good, 2000),
        ];
        let stats = build_statistics(&logs, START + 3 * 86_400 + 100, START);
        assert_eq!(stats.reviews_today, 3);
        assert_eq!(stats.ratings_today, [1, 0, 1, 1]);
        assert_eq!(stats.millis_today, 3000);
        assert_eq!(stats.accuracy_percent(), Some(66));
        assert_eq!(stats.total_reviews, 4);
        assert_eq!(stats.total_millis, 5000);
        assert_eq!(stats.days.last().map(|day| day.reviews), Some(3));
        assert_eq!(stats.days[HEATMAP_DAYS - 2].reviews, 1);
    }

    #[test]
    fn heatmap_covers_the_full_window() {
        let logs: Vec<_> =
            (0..7).map(|day| log(day, Rating::Good, 100)).collect();
        let stats = build_statistics(&logs, START + 6 * 86_400, START);
        let recent: Vec<_> = stats
            .days
            .iter()
            .skip(HEATMAP_DAYS - 7)
            .map(|day| day.reviews)
            .collect();
        assert_eq!(recent, [1, 1, 1, 1, 1, 1, 1]);
        assert!(
            stats.days[..HEATMAP_DAYS - 7]
                .iter()
                .all(|day| day.reviews == 0)
        );
        assert_eq!(stats.days[0].day, -161);
        assert_eq!(stats.days[HEATMAP_DAYS - 1].day, 6);
        assert_eq!(stats.streak, 7);
    }

    #[test]
    fn heatmap_grid_holds_every_stat_day() {
        let today = 6;
        let oldest = today - i64::try_from(HEATMAP_DAYS - 1).unwrap_or(0);
        let start = heatmap_start(oldest, START);
        let last_cell = start
            .saturating_add(i64::try_from(HEATMAP_COLUMNS * 7).unwrap_or(0))
            .saturating_sub(1);
        assert_eq!(weekday_index(start, START), 0);
        assert!(start <= oldest);
        assert!(last_cell >= today);
    }

    #[test]
    fn heatmap_days_keep_time_and_ratings() {
        let logs = [
            log(3, Rating::Again, 500),
            log(3, Rating::Good, 1000),
            log(1, Rating::Easy, 2000),
        ];
        let stats = build_statistics(&logs, START + 3 * 86_400 + 100, START);
        let today = stats.days.last().unwrap();
        assert_eq!(today.reviews, 2);
        assert_eq!(today.millis, 1500);
        assert_eq!(today.ratings, [1, 0, 1, 0]);
        assert_eq!(today.accuracy_percent(), Some(50));

        let older = stats.days.get(HEATMAP_DAYS - 3).unwrap();
        assert_eq!(older.day, 1);
        assert_eq!(older.reviews, 1);
        assert_eq!(older.millis, 2000);
        assert_eq!(older.ratings, [0, 0, 0, 1]);
        assert_eq!(day_label(older.day, START), "Wednesday, 15 November");
    }

    #[test]
    fn streak_starts_yesterday_when_today_has_no_reviews() {
        let logs = [log(1, Rating::Good, 100), log(0, Rating::Good, 100)];
        let stats = build_statistics(&logs, START + 2 * 86_400, START);
        assert_eq!(stats.reviews_today, 0);
        assert_eq!(stats.accuracy_percent(), None);
        assert_eq!(stats.streak, 2);
    }

    #[test]
    fn manual_logs_are_not_reviews() {
        let mut entry = log(0, Rating::Good, 100);
        entry.kind = RevLogKind::Manual;
        let stats = build_statistics(&[entry], START, START);
        assert_eq!(stats.total_reviews, 0);
        assert_eq!(stats.reviews_today, 0);
    }

    #[test]
    fn forecast_days_join_last_week_with_next_week() {
        let stats = build_statistics(
            &[log(0, Rating::Good, 100), log(6, Rating::Good, 100)],
            START + 6 * 86_400,
            START,
        );
        let days = forecast_days(&stats, &[2, 3, 4, 5, 6, 7, 8]);

        assert_eq!(days.len(), 14);
        assert_eq!(
            days[0],
            ForecastDay {
                day: 0,
                reviews: 1,
                due: None
            }
        );
        assert_eq!(
            days[6],
            ForecastDay {
                day: 6,
                reviews: 1,
                due: None
            }
        );
        assert_eq!(
            days[7],
            ForecastDay {
                day: 6,
                reviews: 0,
                due: Some(2)
            }
        );
        assert_eq!(
            days[13],
            ForecastDay {
                day: 12,
                reviews: 0,
                due: Some(8)
            }
        );
    }

    #[test]
    fn forecast_days_without_due_data_are_unavailable() {
        let stats = build_statistics(&[], START + 6 * 86_400, START);
        let days = forecast_days(&stats, &[]);
        let upcoming: Vec<_> = days[7..].iter().map(|day| day.day).collect();
        assert_eq!(upcoming, [6, 7, 8, 9, 10, 11, 12]);
        assert!(days[7..].iter().all(|day| day.due.is_none()));
    }
}
