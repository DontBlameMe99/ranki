//! Anki's due-queue model: limits, gather order, mixing and sibling burying.

use std::collections::HashMap;
use std::fmt::Write as _;

use rand::RngExt as _;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use ranki_model::Card;
use ranki_model::CardType;
use ranki_model::DeckPreset;
use ranki_model::EffectiveDeck;
use ranki_model::Id;
use ranki_model::StoreError;
use ranki_model::Timing;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::params;
use rusqlite::params_from_iter;
use rusqlite::types::Value as SqlValue;
use serde_json::Value as Json;
use serde_json::json;

use crate::anki;
use crate::backend;
use crate::fnv_hash;
use crate::records;
use crate::records::card_from_row;
use ranki_model::DEFAULT_DECK_ID;

const DEFAULT_LEARN_AHEAD_SECS: i64 = 1200;
const UNIX_DUE_THRESHOLD: i64 = 1_000_000_000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum LimitKind {
    New,
    Review,
}

#[derive(Clone, Copy)]
struct Remaining {
    new: u32,
    review: u32,
}

impl Remaining {
    const fn get(self, kind: LimitKind) -> u32 {
        match kind {
            LimitKind::New => self.new,
            LimitKind::Review => self.review,
        }
    }

    const fn set(&mut self, kind: LimitKind, value: u32) {
        match kind {
            LimitKind::New => self.new = value,
            LimitKind::Review => self.review = value,
        }
    }
}

struct Limits {
    remaining: HashMap<Id, Remaining>,
    parents: HashMap<Id, Id>,
    cap_new_to_review: bool,
}

impl Limits {
    #[expect(
        clippy::too_many_arguments,
        reason = "limit construction needs decks, presets and today's state"
    )]
    fn build(
        decks: &[EffectiveDeck],
        presets: &[DeckPreset],
        active: &[Id],
        today: u32,
        new_cards_ignore_review_limit: bool,
    ) -> Self {
        let mut parents = HashMap::new();
        for deck in decks {
            if let Some((parent, _)) = deck.name.rsplit_once("::")
                && let Some(id) = decks
                    .iter()
                    .find(|candidate| candidate.name == parent)
                    .map(|candidate| candidate.id)
            {
                let _ = parents.insert(deck.id, id);
            }
        }
        let mut remaining = HashMap::new();
        for id in active {
            let Some(deck) = decks.iter().find(|deck| deck.id == *id) else {
                continue;
            };
            let preset = presets
                .iter()
                .find(|preset| preset.id == deck.preset_id)
                .cloned()
                .unwrap_or_default();
            let new_day = current_limit(&deck.overrides.new_today, today)
                .unwrap_or_else(|| {
                    deck.overrides.new_limit.unwrap_or(preset.new_per_day)
                });
            let review_day = current_limit(&deck.overrides.review_today, today)
                .unwrap_or_else(|| {
                    deck.overrides
                        .review_limit
                        .unwrap_or(preset.reviews_per_day)
                });
            let studied_today = deck.common.last_day_studied == today;
            let new_done = if studied_today {
                u32::try_from(deck.common.new_studied).unwrap_or(0)
            } else {
                0
            };
            let review_done = if studied_today {
                u32::try_from(deck.common.review_studied).unwrap_or(0)
            } else {
                0
            };
            let mut review = review_day.saturating_sub(review_done);
            let mut new = new_day.saturating_sub(new_done);
            if !new_cards_ignore_review_limit {
                review = review.saturating_sub(new_done);
                new = new.min(review);
            }
            let _ = remaining.insert(deck.id, Remaining { new, review });
        }
        Self {
            remaining,
            parents,
            cap_new_to_review: !new_cards_ignore_review_limit,
        }
    }

    fn reached(&self, deck_id: Id, kind: LimitKind) -> bool {
        let mut current = Some(deck_id);
        while let Some(id) = current {
            if self
                .remaining
                .get(&id)
                .is_some_and(|entry| entry.get(kind) == 0)
            {
                return true;
            }
            current = self.parents.get(&id).copied();
        }
        false
    }

    fn decrement(&mut self, deck_id: Id, kind: LimitKind) {
        let mut current = Some(deck_id);
        while let Some(id) = current {
            if let Some(entry) = self.remaining.get_mut(&id) {
                let after = entry.get(kind).saturating_sub(1);
                entry.set(kind, after);
                if kind == LimitKind::Review && self.cap_new_to_review {
                    entry.new = entry.new.min(after);
                }
            }
            current = self.parents.get(&id).copied();
        }
    }
}

const fn current_limit(
    limit: &ranki_model::DayLimit,
    today: u32,
) -> Option<u32> {
    if limit.day == today {
        limit.limit
    } else {
        None
    }
}

#[derive(Clone, Copy, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Anki stores three independent sibling burying settings"
)]
struct BuryRule {
    new: bool,
    review: bool,
    interday: bool,
}

impl BuryRule {
    fn of(deck_id: Id, by_deck: &HashMap<Id, DeckPreset>) -> Self {
        by_deck.get(&deck_id).map_or_else(
            || Self {
                new: false,
                review: false,
                interday: false,
            },
            |preset| Self {
                new: preset.bury_new,
                review: preset.bury_reviews,
                interday: preset.bury_interday_learning,
            },
        )
    }

    const fn merge(&mut self, other: Self) {
        self.new |= other.new;
        self.review |= other.review;
        self.interday |= other.interday;
    }

    fn buries(self, kind: LimitKind, interday: bool) -> bool {
        if interday {
            self.interday
        } else if kind == LimitKind::Review {
            self.review
        } else {
            self.new
        }
    }
}

struct Gather<'a> {
    conn: &'a Connection,
    filter: String,
    params: Vec<SqlValue>,
    limits: Limits,
    presets: HashMap<Id, DeckPreset>,
    seen_notes: HashMap<Id, BuryRule>,
    intraday: Vec<Card>,
    day_learning: Vec<Card>,
    review: Vec<Card>,
    new: Vec<Card>,
    deck_order: Vec<Id>,
    root: Option<Id>,
}

impl Gather<'_> {
    fn root_reached(&self, kind: LimitKind) -> bool {
        self.root.is_some_and(|id| self.limits.reached(id, kind))
    }

    const fn home_deck(card: &Card) -> Id {
        if card.original_deck_id == 0 {
            card.deck_id
        } else {
            card.original_deck_id
        }
    }

    fn note_seen(&mut self, card: &Card) {
        let rule = BuryRule::of(Self::home_deck(card), &self.presets);
        let entry = self.seen_notes.entry(card.note_id).or_insert(rule);
        entry.merge(rule);
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "burying needs the card, kind and gathered queue"
    )]
    fn keep(&mut self, card: &Card, kind: LimitKind, interday: bool) -> bool {
        let previous = self.seen_notes.get(&card.note_id).copied();
        self.note_seen(card);
        !previous.is_some_and(|rule| rule.buries(kind, interday))
    }

    fn gather_intraday(&mut self, cutoff: i64) -> Result<(), StoreError> {
        let sql = format!(
            "SELECT c.id, c.nid, c.did, c.ord, c.mod, c.usn, c.type, c.queue, c.due, c.ivl, \
             c.factor, c.reps, c.lapses, c.left, c.odue, c.odid, c.flags, c.data FROM cards c \
             WHERE c.queue = 1 AND c.due <= ? AND {} ORDER BY (c.reps = 0), c.due, c.id",
            self.filter
        );
        let mut parameters = vec![SqlValue::Integer(cutoff)];
        parameters.extend(self.params.iter().cloned());
        let mut statement = self.conn.prepare(&sql).map_err(backend)?;
        let rows = statement
            .query_map(params_from_iter(parameters), card_from_row)
            .map_err(backend)?;
        for row in rows {
            let card = row.map_err(backend)?;
            self.note_seen(&card);
            self.intraday.push(card);
        }
        Ok(())
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "a due query needs its queue, order and time window"
    )]
    fn gather_due(
        &mut self,
        queue: i64,
        kind: LimitKind,
        order: &str,
        today: i64,
    ) -> Result<(), StoreError> {
        if self.root_reached(LimitKind::Review) {
            return Ok(());
        }
        let sql = format!(
            "SELECT c.id, c.nid, c.did, c.ord, c.mod, c.usn, c.type, c.queue, c.due, c.ivl, \
             c.factor, c.reps, c.lapses, c.left, c.odue, c.odid, c.flags, c.data FROM cards c \
             WHERE c.queue = ? AND c.due <= ? AND {} ORDER BY {}",
            self.filter, order
        );
        let mut parameters =
            vec![SqlValue::Integer(queue), SqlValue::Integer(today)];
        parameters.extend(self.params.iter().cloned());
        let mut statement = self.conn.prepare(&sql).map_err(backend)?;
        let rows = statement
            .query_map(params_from_iter(parameters), card_from_row)
            .map_err(backend)?;
        for row in rows {
            let card = row.map_err(backend)?;
            if self.limits.reached(card.deck_id, LimitKind::Review)
                || !self.keep(&card, kind, queue == 3)
            {
                continue;
            }
            self.limits.decrement(card.deck_id, LimitKind::Review);
            let target = if queue == 3 {
                &mut self.day_learning
            } else {
                &mut self.review
            };
            target.push(card);
            if self.root_reached(LimitKind::Review) {
                break;
            }
        }
        Ok(())
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "new-card gathering needs its priority, sort and day"
    )]
    fn gather_new(
        &mut self,
        priority: u32,
        sort_order: u32,
        today: u32,
    ) -> Result<(), StoreError> {
        if self.root_reached(LimitKind::New) {
            return Ok(());
        }
        let salt = today.wrapping_mul(2_654_435_761);
        let by_deck = matches!(priority, 0 | 5);
        if by_deck {
            let decks = self.deck_order.clone();
            for deck_id in decks {
                if self.root_reached(LimitKind::New) {
                    break;
                }
                let order = if priority == 5 {
                    format!("fnvhash(c.nid, {salt}), c.ord")
                } else {
                    String::from("c.due, c.ord")
                };
                self.gather_new_deck(deck_id, &order)?;
            }
        } else {
            let order = match priority {
                2 => String::from("c.due DESC, c.ord"),
                3 => format!("fnvhash(c.nid, {salt}), c.ord"),
                4 => format!("fnvhash(c.id, {salt})"),
                _ => String::from("c.due, c.ord"),
            };
            self.gather_new_deck(0, &order)?;
        }
        match sort_order {
            0 => self.new.sort_by_key(|card| card.ordinal),
            2 => self.new.sort_by_key(|card| {
                (card.ordinal, fnv_hash(card.id, i64::from(today)))
            }),
            3 => self.new.sort_by_key(|card| {
                (fnv_hash(card.note_id, i64::from(today)), card.ordinal)
            }),
            4 => self
                .new
                .sort_by_key(|card| fnv_hash(card.id, i64::from(today))),
            _ => {}
        }
        Ok(())
    }

    fn gather_new_deck(
        &mut self,
        deck_id: Id,
        order: &str,
    ) -> Result<(), StoreError> {
        let filter = if deck_id == 0 {
            self.filter.clone()
        } else {
            format!("c.did = {deck_id}")
        };
        let sql = format!(
            "SELECT c.id, c.nid, c.did, c.ord, c.mod, c.usn, c.type, c.queue, c.due, c.ivl, \
             c.factor, c.reps, c.lapses, c.left, c.odue, c.odid, c.flags, c.data FROM cards c \
             WHERE c.queue = 0 AND {filter} ORDER BY {order}"
        );
        let mut parameters = Vec::new();
        if deck_id == 0 {
            parameters.extend(self.params.iter().cloned());
        }
        let mut statement = self.conn.prepare(&sql).map_err(backend)?;
        let rows = statement
            .query_map(params_from_iter(parameters), card_from_row)
            .map_err(backend)?;
        for row in rows {
            let card = row.map_err(backend)?;
            if self.limits.reached(card.deck_id, LimitKind::New)
                || !self.keep(&card, LimitKind::New, false)
            {
                continue;
            }
            self.limits.decrement(card.deck_id, LimitKind::New);
            self.new.push(card);
            if self.root_reached(LimitKind::New) {
                break;
            }
        }
        Ok(())
    }
}

fn review_order_sql(order: u32, today: i64, rank: &str) -> String {
    let hash = "fnvhash(c.id, c.mod)";
    match order {
        1 => format!("c.due, {rank}, {hash}"),
        2 => format!("{rank}, c.due, {hash}"),
        3 => format!("c.ivl, {hash}"),
        4 => format!("c.ivl DESC, {hash}"),
        5 => format!("c.factor, {hash}"),
        6 => format!("c.factor DESC, {hash}"),
        8 => hash.to_owned(),
        9 => String::from("c.nid, c.ord"),
        10 => String::from("c.nid DESC, c.ord"),
        12 => format!(
            "-(1 + CAST({today}-c.due+0.001 AS REAL)/MAX(c.ivl, 1)) ASC, {hash}"
        ),
        _ => format!("c.due, {hash}"),
    }
}

fn deck_rank_sql(order: &[Id]) -> String {
    let mut sql = String::from("CASE c.did");
    for (index, id) in order.iter().enumerate() {
        let _ = write!(sql, " WHEN {id} THEN {index}");
    }
    let _ = write!(sql, " ELSE {} END", order.len());
    sql
}

fn intersperse(one: Vec<Card>, two: Vec<Card>) -> Vec<Card> {
    let one_len = one.len();
    let two_len = two.len();
    if one_len == 0 {
        return two;
    }
    if two_len == 0 {
        return one;
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "Anki's queue mixer uses f32 ratios"
    )]
    let ratio =
        (one_len.saturating_add(1)) as f32 / (two_len.saturating_add(1)) as f32;
    let mut result = Vec::with_capacity(one_len.saturating_add(two_len));
    let mut one_idx = 0_usize;
    let mut two_idx = 0_usize;
    while one_idx < one_len || two_idx < two_len {
        let take_two = if one_idx < one_len && two_idx < two_len {
            #[expect(
                clippy::cast_precision_loss,
                reason = "Anki's queue mixer uses f32 ratios"
            )]
            let position = (two_idx.saturating_add(1)) as f32 * ratio;
            #[expect(
                clippy::cast_precision_loss,
                reason = "Anki's queue mixer uses f32 ratios"
            )]
            let limit = (one_idx.saturating_add(1)) as f32;
            position < limit
        } else {
            two_idx < two_len
        };
        if take_two && let Some(card) = two.get(two_idx) {
            result.push(card.clone());
            two_idx = two_idx.saturating_add(1);
        } else if let Some(card) = one.get(one_idx) {
            result.push(card.clone());
            one_idx = one_idx.saturating_add(1);
        }
    }
    result
}

fn merge(one: Vec<Card>, two: Vec<Card>, mode: u32) -> Vec<Card> {
    match mode {
        1 => {
            let mut merged = one;
            merged.extend(two);
            merged
        }
        2 => {
            let mut merged = two;
            merged.extend(one);
            merged
        }
        _ => intersperse(one, two),
    }
}

fn read_config_value(conn: &Connection, key: &str) -> Option<Json> {
    let stored: Option<Vec<u8>> = conn
        .query_row("SELECT val FROM config WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .optional()
        .unwrap_or(None);
    if let Some(bytes) = stored
        && let Ok(value) = serde_json::from_slice::<Json>(&bytes)
    {
        return Some(value);
    }
    let path = format!("$.{key}");
    let value: Option<SqlValue> = conn
        .query_row(
            "SELECT json_extract(conf, ?1) FROM col WHERE id = 1",
            [path],
            |row| row.get(0),
        )
        .optional()
        .unwrap_or(None);
    match value {
        Some(SqlValue::Integer(number)) => Some(json!(number)),
        Some(SqlValue::Real(number)) => Some(json!(number)),
        Some(SqlValue::Text(text)) => serde_json::from_str(&text).ok(),
        _ => None,
    }
}

pub fn config_number(conn: &Connection, key: &str, default: i64) -> i64 {
    read_config_value(conn, key)
        .and_then(|value| value.as_i64())
        .unwrap_or(default)
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "the Store contract returns a Result for collection reads"
)]
pub fn learn_ahead_secs(conn: &Connection) -> Result<i64, StoreError> {
    let secs = config_number(conn, "collapseTime", DEFAULT_LEARN_AHEAD_SECS);
    Ok(secs.clamp(0, 86_400))
}
#[expect(
    clippy::struct_excessive_bools,
    reason = "these flags mirror independent collection booleans"
)]
#[derive(Clone, Copy)]
struct QueueConfig {
    learn_ahead: i64,
    new_ignores_review: bool,
    all_parents: bool,
    last_unburied: i64,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            learn_ahead: DEFAULT_LEARN_AHEAD_SECS,
            new_ignores_review: false,
            all_parents: false,
            last_unburied: 0,
        }
    }
}

fn sql_number(value: SqlValue) -> Option<i64> {
    match value {
        SqlValue::Integer(number) => Some(number),
        SqlValue::Text(text) => text.parse().ok(),
        SqlValue::Real(_) | SqlValue::Null | SqlValue::Blob(_) => None,
    }
}

fn queue_config(conn: &Connection) -> QueueConfig {
    let mut config = QueueConfig::default();
    if let Ok(row) = conn.query_row(
        "SELECT json_extract(conf, '$.collapseTime'), \
         json_extract(conf, '$.newCardsIgnoreReviewLimit'), \
         json_extract(conf, '$.applyAllParentLimits'), \
         json_extract(conf, '$.lastUnburied') FROM col WHERE id = 1",
        [],
        |row| {
            Ok((
                row.get::<_, SqlValue>(0)?,
                row.get::<_, SqlValue>(1)?,
                row.get::<_, SqlValue>(2)?,
                row.get::<_, SqlValue>(3)?,
            ))
        },
    ) {
        if let Some(value) = sql_number(row.0) {
            config.learn_ahead = value;
        }
        config.new_ignores_review = sql_number(row.1) == Some(1);
        config.all_parents = sql_number(row.2) == Some(1);
        if let Some(value) = sql_number(row.3) {
            config.last_unburied = value;
        }
    }
    if let Ok(mut statement) = conn.prepare(
        "SELECT key, val FROM config WHERE key IN ('collapseTime', \
         'newCardsIgnoreReviewLimit', 'applyAllParentLimits', 'lastUnburied')",
    ) && let Ok(rows) = statement.query_map([], |row| {
        let key: String = row.get(0)?;
        let value: Vec<u8> = row.get(1)?;
        Ok((key, value))
    }) {
        for row in rows.flatten() {
            let parsed: Option<Json> = serde_json::from_slice(&row.1).ok();
            match (row.0.as_str(), parsed) {
                ("collapseTime", Some(value)) => {
                    if let Some(number) = value.as_i64() {
                        config.learn_ahead = number;
                    }
                }
                ("newCardsIgnoreReviewLimit", Some(value)) => {
                    config.new_ignores_review =
                        value.as_bool().unwrap_or(false);
                }
                ("applyAllParentLimits", Some(value)) => {
                    config.all_parents = value.as_bool().unwrap_or(false);
                }
                ("lastUnburied", Some(value)) => {
                    if let Some(number) = value.as_i64() {
                        config.last_unburied = number;
                    }
                }
                _ => {}
            }
        }
    }
    config.learn_ahead = config.learn_ahead.clamp(0, 86_400);
    config
}

fn set_config_number(
    conn: &Connection,
    key: &str,
    value: i64,
) -> Result<(), StoreError> {
    let encoded = value.to_string();
    if records::table_exists(conn, "config")? {
        conn.execute(
            "INSERT INTO config (key, usn, mtime_secs, val) VALUES (?1, -1, ?2, ?3) \
             ON CONFLICT(key) DO UPDATE SET val = excluded.val, \
             mtime_secs = excluded.mtime_secs",
            params![key, anki::now_secs()?, encoded.as_bytes()],
        )
        .map_err(backend)?;
        return Ok(());
    }
    let col = records::read_col(conn)?;
    let mut config = crate::parse_conf(&col.conf)?;
    if let Some(object) = config.as_object_mut() {
        let _ = object.insert(key.to_owned(), json!(value));
    }
    let text = serde_json::to_string(&config).map_err(backend)?;
    conn.execute("UPDATE col SET conf = ?1 WHERE id = 1", [text])
        .map_err(backend)?;
    Ok(())
}

pub const fn restore_queue(card_type: CardType, due: i64) -> i64 {
    match card_type {
        CardType::New => 0,
        CardType::Review => 2,
        CardType::Learning | CardType::Relearning => {
            if due > UNIX_DUE_THRESHOLD { 1 } else { 3 }
        }
    }
}

fn unbury_if_day_rolled_over(
    conn: &Connection,
    today: u32,
) -> Result<(), StoreError> {
    let transaction = conn.unchecked_transaction().map_err(backend)?;
    let last = config_number(&transaction, "lastUnburied", 0);
    if last >= i64::from(today) && last <= i64::from(today.saturating_add(7)) {
        return Ok(());
    }
    let mut statement = transaction
        .prepare("SELECT id, type, due FROM cards WHERE queue IN (-2, -3)")
        .map_err(backend)?;
    let rows = statement
        .query_map([], |row| {
            let card_type: i64 = row.get(1)?;
            Ok((row.get::<_, Id>(0)?, card_type, row.get::<_, i64>(2)?))
        })
        .map_err(backend)?;
    let mut restored = Vec::new();
    for row in rows {
        let (id, card_type, due) = row.map_err(backend)?;
        restored.push((id, restore_queue(CardType::from_code(card_type), due)));
    }
    drop(statement);
    for (id, queue) in restored {
        transaction
            .execute(
                "UPDATE cards SET queue = ?1 WHERE id = ?2",
                params![queue, id],
            )
            .map_err(backend)?;
    }
    set_config_number(&transaction, "lastUnburied", i64::from(today))?;
    transaction.commit().map_err(backend)
}

#[expect(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "queue building resolves options, limits and all four gather paths"
)]
pub fn gather_due_cards(
    conn: &Connection,
    deck_id: Option<Id>,
    limit: usize,
    now_secs: i64,
    timing: Timing,
    real_today: u32,
    decks: &[EffectiveDeck],
    presets: &[DeckPreset],
) -> Result<Vec<Card>, StoreError> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let today = i64::from(timing.days_elapsed);
    let config = queue_config(conn);
    unbury_if_day_rolled_over(conn, real_today)?;
    let learn_ahead = config.learn_ahead;
    let new_ignores_review = config.new_ignores_review;
    let all_parent_limits = config.all_parents;

    let root = deck_id.and_then(|id| decks.iter().find(|deck| deck.id == id));
    if deck_id.is_some() && root.is_none() {
        return Ok(Vec::new());
    }
    let candidates: Vec<Id> = root.map_or_else(
        || decks.iter().map(|deck| deck.id).collect(),
        |selected| {
            let prefix = format!("{}::", selected.name);
            decks
                .iter()
                .filter(|deck| {
                    deck.id == selected.id || deck.name.starts_with(&prefix)
                })
                .map(|deck| deck.id)
                .collect()
        },
    );
    let mut limit_ids = candidates.clone();
    if all_parent_limits && let Some(selected) = root {
        for (index, _) in selected.name.char_indices() {
            if selected
                .name
                .get(index..)
                .is_some_and(|tail| tail.starts_with("::"))
                && let Some(parent) = decks
                    .iter()
                    .find(|deck| deck.name == selected.name[..index])
            {
                limit_ids.push(parent.id);
            }
        }
    }
    let mut candidates = candidates;
    candidates.sort_unstable();
    candidates.dedup();
    limit_ids.sort_unstable();
    limit_ids.dedup();
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = vec!["?"; candidates.len()].join(", ");
    let filter = format!("c.did IN ({placeholders})");
    let params: Vec<SqlValue> =
        candidates.iter().map(|id| SqlValue::Integer(*id)).collect();
    let limits = Limits::build(
        decks,
        presets,
        &limit_ids,
        timing.days_elapsed,
        new_ignores_review,
    );
    let by_deck: HashMap<Id, DeckPreset> = decks
        .iter()
        .filter_map(|deck| {
            presets
                .iter()
                .find(|preset| preset.id == deck.preset_id)
                .cloned()
                .map(|preset| (deck.id, preset))
        })
        .collect();
    let mut keyed: Vec<(String, Id)> = decks
        .iter()
        .filter(|deck| candidates.contains(&deck.id))
        .map(|deck| (deck.name.clone(), deck.id))
        .collect();
    keyed.sort_by(|left, right| {
        left.0
            .split("::")
            .collect::<Vec<_>>()
            .cmp(&right.0.split("::").collect::<Vec<_>>())
    });
    let deck_order: Vec<Id> = keyed.into_iter().map(|(_, id)| id).collect();

    let rank_sql = deck_rank_sql(&deck_order);
    let mut gather = Gather {
        conn,
        filter,
        params,
        limits,
        presets: by_deck,
        seen_notes: HashMap::new(),
        intraday: Vec::new(),
        day_learning: Vec::new(),
        review: Vec::new(),
        new: Vec::new(),
        deck_order,
        root: root.map(|deck| deck.id),
    };
    gather.gather_intraday(now_secs.saturating_add(learn_ahead))?;
    let sort_root =
        root.or_else(|| decks.iter().find(|deck| deck.id == DEFAULT_DECK_ID));
    let root_preset = sort_root.and_then(|deck| {
        presets.iter().find(|preset| preset.id == deck.preset_id)
    });
    let review_order = root_preset.map_or(0, |preset| preset.review_order);
    let order = review_order_sql(review_order, today, &rank_sql);
    gather.gather_due(3, LimitKind::Review, &order, today)?;
    gather.gather_due(2, LimitKind::Review, &order, today)?;
    let gather_priority =
        root_preset.map_or(0, |preset| preset.new_card_gather_priority);
    let new_sort = root_preset.map_or(0, |preset| preset.new_card_sort_order);
    gather.gather_new(gather_priority, new_sort, timing.days_elapsed)?;

    let interday_mix =
        root_preset.map_or(0, |preset| preset.interday_learning_mix);
    let new_mix = root_preset.map_or(0, |preset| preset.new_mix);
    let due_cards = merge(gather.review, gather.day_learning, interday_mix);
    let mixed = merge(due_cards, gather.new, new_mix);
    let mut cards = gather.intraday;
    cards.extend(mixed);
    cards.truncate(limit);
    Ok(cards)
}

/// Anki's seeded position for `new.order = Random`.
pub fn random_position(highest_position: u32) -> i64 {
    let mut rng = StdRng::seed_from_u64(u64::from(highest_position));
    let upper = highest_position.max(1000);
    let value = rng.random_range(1..upper);
    i64::from(value)
}

/// Position for a generated card under the destination deck's insert order.
pub fn new_card_position(next: i64, offset: i64, random: bool) -> i64 {
    if !random {
        return next.saturating_add(offset);
    }
    let highest = u32::try_from(next).unwrap_or(u32::MAX);
    random_position(highest)
}

#[cfg(test)]
mod tests {
    use super::intersperse;
    use super::random_position;
    use ranki_model::Card;

    #[test]
    fn random_positions_match_anki() {
        assert_eq!(random_position(5), 180);
        assert_eq!(random_position(500), 13);
        assert_eq!(random_position(5001), 3731);
    }

    #[test]
    fn mixer_spreads_cards_from_both_queues() {
        let card = |id| Card {
            id,
            note_id: id,
            deck_id: 1,
            ordinal: 0,
            modified: 0,
            usn: 0,
            card_type: ranki_model::CardType::New,
            queue: ranki_model::CardQueue::New,
            due: 0,
            interval: 0,
            factor: 0,
            reps: 0,
            lapses: 0,
            left: 0,
            original_due: 0,
            original_deck_id: 0,
            flags: 0,
            data: String::new(),
        };
        let one: Vec<_> = (0..3).map(card).collect();
        let two: Vec<_> = (10..13).map(card).collect();
        let ids: Vec<_> =
            intersperse(one, two).iter().map(|card| card.id).collect();
        assert_eq!(ids, [0, 10, 1, 11, 2, 12]);
    }
}
