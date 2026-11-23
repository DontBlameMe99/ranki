//! Conversion between Anki rows and the shared model types.

use std::collections::BTreeMap;

use crate::anki;
use ranki_model::Card;
use ranki_model::CardQueue;
use ranki_model::CardTemplate;
use ranki_model::CardType;
use ranki_model::Deck;
use ranki_model::Id;
use ranki_model::Note;
use ranki_model::NoteKind;
use ranki_model::NoteModel;
use ranki_model::Rating;
use ranki_model::RevLog;
use ranki_model::RevLogKind;
use ranki_model::StoreError;
use ranki_model::decode_hex;
use ranki_model::encode_hex;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::Result as SqlResult;
use rusqlite::Row;
use rusqlite::types::Type;
use rusqlite::types::Value;
use serde_json::Map;
use serde_json::Value as Json;
use serde_json::json;

pub struct ColData {
    pub modified: i64,
    pub scm: i64,
    pub last_sync: i64,
    pub usn: i32,
    pub conf: String,
    pub models: String,
    pub decks: String,
}

pub fn read_col(conn: &Connection) -> Result<ColData, StoreError> {
    conn.query_row(
        "SELECT mod, scm, usn, conf, models, decks, dconf, tags, ls FROM col WHERE id = 1",
        [],
        |row| {
            let usn = row.get::<_, i64>(2)?;
            Ok(ColData {
                modified: row.get(0)?,
                scm: row.get(1)?,
                last_sync: row.get(8)?,
                usn: i32::try_from(usn).unwrap_or(i32::MAX),
                conf: text(row.get(3)?),
                models: text(row.get(4)?),
                decks: text(row.get(5)?),
            })
        },
    )
    .map_err(crate::backend)
}

/// Reads only the `decks` JSON so listing decks does not copy the note types
/// and configuration stored in the same row.
pub fn read_decks(conn: &Connection) -> Result<String, StoreError> {
    conn.query_row("SELECT decks FROM col WHERE id = 1", [], |row| {
        Ok(text(row.get(0)?))
    })
    .map_err(crate::backend)
}

pub fn card_from_row(row: &Row<'_>) -> SqlResult<Card> {
    let ordinal = anki::as_u16(row.get(3)?).map_err(conversion)?;
    let factor = anki::as_u32(row.get(10)?).map_err(conversion)?;
    let reps = anki::as_u32(row.get(11)?).map_err(conversion)?;
    let lapses = anki::as_u32(row.get(12)?).map_err(conversion)?;
    let left = anki::as_u32(row.get(13)?).map_err(conversion)?;
    Ok(Card {
        id: row.get(0)?,
        note_id: row.get(1)?,
        deck_id: row.get(2)?,
        ordinal,
        modified: row.get(4)?,
        usn: i32::try_from(row.get::<_, i64>(5)?).unwrap_or(0),
        card_type: CardType::from_code(row.get(6)?),
        queue: CardQueue::from_code(row.get(7)?),
        due: row.get(8)?,
        interval: i32::try_from(row.get::<_, i64>(9)?).unwrap_or(i32::MAX),
        factor,
        reps,
        lapses,
        left,
        original_due: row.get(14)?,
        original_deck_id: row.get(15)?,
        flags: i32::try_from(row.get::<_, i64>(16)?).unwrap_or(0),
        data: text(row.get(17)?),
    })
}

pub fn note_from_row(row: &Row<'_>) -> SqlResult<Note> {
    let fields = text(row.get::<_, Value>(6)?);
    let raw = row.get::<_, i64>(8)?;
    let checksum = u32::try_from(raw & i64::from(u32::MAX)).unwrap_or(0);
    Ok(Note {
        id: row.get(0)?,
        guid: text(row.get(1)?),
        model_id: row.get(2)?,
        modified: row.get(3)?,
        usn: i32::try_from(row.get::<_, i64>(4)?).unwrap_or(0),
        tags: anki::decode_tags(&text(row.get::<_, Value>(5)?)),
        fields: anki::decode_fields(&fields),
        sort_field: text(row.get::<_, Value>(7)?),
        checksum: anki::signed_i32(checksum),
        flags: i32::try_from(row.get::<_, i64>(9)?).unwrap_or(0),
        data: text(row.get(10)?),
    })
}

pub fn revlog_from_row(row: &Row<'_>) -> SqlResult<RevLog> {
    let ease = u8::try_from(row.get::<_, i64>(3)?).unwrap_or(0);
    let kind = u8::try_from(row.get::<_, i64>(8)?).unwrap_or(0);
    Ok(RevLog {
        id: row.get(0)?,
        card_id: row.get(1)?,
        usn: i32::try_from(row.get::<_, i64>(2)?).unwrap_or(0),
        ease: Rating::from_code(ease).unwrap_or(Rating::Again),
        interval: i32::try_from(row.get::<_, i64>(4)?).unwrap_or_default(),
        last_interval: i32::try_from(row.get::<_, i64>(5)?).unwrap_or_default(),
        factor: anki::as_u32(row.get(6)?).map_err(conversion)?,
        time_ms: anki::as_u32(row.get(7)?).map_err(conversion)?,
        kind: RevLogKind::from_code(kind),
    })
}

pub fn decks_from_json(json: &str) -> Result<Vec<Deck>, StoreError> {
    if json.trim().is_empty() {
        return Ok(Vec::new());
    }
    let value: Json = serde_json::from_str(json)
        .map_err(|error| crate::invalid(error.to_string()))?;
    let object = value
        .as_object()
        .ok_or_else(|| crate::invalid("decks is not an object"))?;
    let mut decks = Vec::with_capacity(object.len());
    for (key, value) in object {
        let id = key
            .parse::<Id>()
            .map_err(|_| crate::invalid("invalid deck id"))?;
        decks.push(Deck {
            id,
            name: value
                .get("name")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .into(),
            description: value
                .get("desc")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .into(),
        });
    }
    decks.sort_unstable_by_key(|deck| deck.id);
    Ok(decks)
}

fn ordinal_at(entry: (usize, &Json)) -> u16 {
    template_ordinal(entry.1, entry.0)
}

fn template_ordinal(template: &Json, position: usize) -> u16 {
    template
        .get("ord")
        .and_then(Json::as_u64)
        .and_then(|ord| u16::try_from(ord).ok())
        .unwrap_or_else(|| u16::try_from(position).unwrap_or_default())
}

#[expect(
    clippy::too_many_lines,
    reason = "schema-11 and schema-18 model fields are decoded together"
)]
pub fn models_from_json(json: &str) -> Result<Vec<NoteModel>, StoreError> {
    if json.trim().is_empty() {
        return Ok(Vec::new());
    }
    let value: Json = serde_json::from_str(json)
        .map_err(|error| crate::invalid(error.to_string()))?;
    let object = value
        .as_object()
        .ok_or_else(|| crate::invalid("models is not an object"))?;
    let mut models = Vec::with_capacity(object.len());
    for (key, value) in object {
        let id = key
            .parse::<Id>()
            .map_err(|_| crate::invalid("invalid model id"))?;
        let templates = value.get("tmpls").and_then(Json::as_array);
        let kind = NoteKind::from_code(
            value
                .get("type")
                .and_then(Json::as_i64)
                .and_then(|code| i32::try_from(code).ok())
                .unwrap_or_default(),
        );
        let fields = string_array(value.get("flds"), "name");
        let field_tags = optional_numbers(value.get("flds"), "tag")
            .into_iter()
            .map(|tag| u32::try_from(tag).ok())
            .collect();
        let template_ordinals = templates.map_or_else(Vec::new, |items| {
            items.iter().enumerate().map(ordinal_at).collect()
        });
        let raw_config = value
            .get("_rankiRawConfig")
            .and_then(Json::as_str)
            .and_then(decode_hex);
        let mut model = NoteModel {
            id,
            name: value
                .get("name")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .into(),
            fields,
            templates: templates.map_or_else(Vec::new, |items| {
                items
                    .iter()
                    .enumerate()
                    .map(|(position, template)| CardTemplate {
                        name: template
                            .get("name")
                            .and_then(Json::as_str)
                            .unwrap_or_default()
                            .into(),
                        question_format: template
                            .get("qfmt")
                            .and_then(Json::as_str)
                            .unwrap_or_default()
                            .into(),
                        answer_format: template
                            .get("afmt")
                            .and_then(Json::as_str)
                            .unwrap_or_default()
                            .into(),
                        ord: template_ordinal(template, position),
                    })
                    .collect()
            }),
            css: value
                .get("css")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .into(),
            kind,
            original_stock_kind: value
                .get("originalStockKind")
                .and_then(Json::as_i64)
                .and_then(|kind| i32::try_from(kind).ok()),
            field_tags,
            template_ordinals,
            raw_config,
        };
        infer_cloze_tags(&mut model);
        models.push(model);
    }
    models.sort_unstable_by_key(|model| model.id);
    Ok(models)
}

pub fn decks_from_tables(conn: &Connection) -> Result<Vec<Deck>, StoreError> {
    let mut statement = conn
        .prepare("SELECT id, name, kind FROM decks")
        .map_err(crate::backend)?;
    let rows = statement
        .query_map([], |row| {
            let kind: Value = row.get(2)?;
            let description = match kind {
                Value::Blob(bytes) => deck_description(&bytes),
                Value::Null
                | Value::Integer(_)
                | Value::Real(_)
                | Value::Text(_) => String::new(),
            };
            Ok(Deck {
                id: row.get(0)?,
                name: anki::decode_deck_name(&text(row.get::<_, Value>(1)?)),
                description,
            })
        })
        .map_err(crate::backend)?;
    let mut decks = Vec::new();
    for row in rows {
        decks.push(row.map_err(crate::backend)?);
    }
    decks.sort_unstable_by_key(|deck| deck.id);
    Ok(decks)
}

fn deck_description(kind: &[u8]) -> String {
    let outer = anki::protobuf_fields(kind).unwrap_or_default();
    let Some(normal) = anki::proto_bytes(&outer, 1) else {
        return String::new();
    };
    let inner = anki::protobuf_fields(normal).unwrap_or_default();
    anki::proto_string(&inner, 4).unwrap_or_default()
}

const fn json_bool(value: u64) -> Json {
    Json::Bool(value != 0)
}

fn json_count(value: Option<u64>) -> Json {
    Json::from(i64::try_from(value.unwrap_or(0)).unwrap_or(i64::MAX))
}

/// Decodes a schema-18 `decks.common` blob into schema-11 day counters.
fn deck_common_json(raw: &[u8]) -> Map<String, Json> {
    let fields = anki::protobuf_fields(raw).unwrap_or_default();
    let number = |field: u32| anki::proto_varint(&fields, field);
    let last_day = number(3).unwrap_or(0);
    let mut json = Map::new();
    json.insert(String::from("collapsed"), json_bool(number(1).unwrap_or(0)));
    json.insert(
        String::from("browserCollapsed"),
        json_bool(number(2).unwrap_or(0)),
    );
    for (key, field) in [
        ("lrnToday", 6_u32),
        ("revToday", 5),
        ("newToday", 4),
        ("timeToday", 7),
    ] {
        let amount = signed_i32(number(field).unwrap_or(0));
        let day = i64::try_from(last_day).unwrap_or(i64::MAX);
        json.insert(
            key.to_owned(),
            Json::Array(vec![Json::from(day), Json::from(amount)]),
        );
    }
    json
}

fn signed_i32(value: u64) -> i64 {
    let low = u32::try_from(value & u64::from(u32::MAX)).unwrap_or(0);
    i64::from(i32::from_ne_bytes(low.to_ne_bytes()))
}

fn day_limit_json(raw: &[u8]) -> Json {
    let fields = anki::protobuf_fields(raw).unwrap_or_default();
    json!({
        "limit": i64::try_from(anki::proto_varint(&fields, 1).unwrap_or(0)).unwrap_or(i64::MAX),
        "today": i64::try_from(anki::proto_varint(&fields, 2).unwrap_or(0)).unwrap_or(i64::MAX),
    })
}

/// Decodes a schema-18 `decks.kind` blob into schema-11 deck keys.
fn deck_kind_json(raw: &[u8], json: &mut Map<String, Json>) {
    let outer = anki::protobuf_fields(raw).unwrap_or_default();
    if let Some(filtered) = anki::proto_bytes(&outer, 2) {
        filtered_deck_json(filtered, json);
        return;
    }
    let normal = anki::proto_bytes(&outer, 1).unwrap_or_default();
    normal_deck_json(normal, json);
}

fn filtered_deck_json(raw: &[u8], json: &mut Map<String, Json>) {
    let fields = anki::protobuf_fields(raw).unwrap_or_default();
    let terms: Vec<Json> = anki::proto_messages(&fields, 2)
        .iter()
        .map(|term| {
            let entry = anki::protobuf_fields(term).unwrap_or_default();
            let search = anki::proto_string(&entry, 1).unwrap_or_default();
            let limit =
                i64::try_from(anki::proto_varint(&entry, 2).unwrap_or(0))
                    .unwrap_or(i64::MAX);
            let order =
                i64::try_from(anki::proto_varint(&entry, 3).unwrap_or(0))
                    .unwrap_or(i64::MAX);
            json!({"search": search, "limit": limit, "order": order})
        })
        .collect();
    let number = |field: u32| anki::proto_varint(&fields, field);
    let _ = json.insert(String::from("dyn"), Json::from(1));
    let _ =
        json.insert(String::from("resched"), json_bool(number(1).unwrap_or(0)));
    let _ = json.insert(String::from("terms"), Json::Array(terms));
    let _ = json.insert(String::from("separate"), Json::Bool(true));
    let _ = json.insert(String::from("previewDelay"), json_count(number(4)));
    let _ =
        json.insert(String::from("previewAgainSecs"), json_count(number(7)));
    let _ = json.insert(String::from("previewHardSecs"), json_count(number(5)));
    let _ = json.insert(String::from("previewGoodSecs"), json_count(number(6)));
    let delays = anki::proto_floats(&fields, 3);
    if !delays.is_empty() {
        let _ = json.insert(
            String::from("delays"),
            Json::Array(delays.into_iter().map(Json::from).collect()),
        );
    }
}

fn normal_deck_json(raw: &[u8], json: &mut Map<String, Json>) {
    let fields = anki::protobuf_fields(raw).unwrap_or_default();
    let number = |field: u32| anki::proto_varint(&fields, field);
    let _ = json.insert(String::from("dyn"), Json::from(0));
    let _ = json.insert(
        String::from("conf"),
        Json::from(i64::try_from(number(1).unwrap_or(1)).unwrap_or(1)),
    );
    let _ = json.insert(String::from("extendNew"), json_count(number(2)));
    let _ = json.insert(String::from("extendRev"), json_count(number(3)));
    let _ = json.insert(
        String::from("desc"),
        Json::from(anki::proto_string(&fields, 4).unwrap_or_default()),
    );
    if number(5).unwrap_or(0) != 0 {
        let _ = json.insert(String::from("md"), Json::Bool(true));
    }
    if let Some(limit) = number(6) {
        let _ =
            json.insert(String::from("reviewLimit"), json_count(Some(limit)));
    }
    if let Some(limit) = number(7) {
        let _ = json.insert(String::from("newLimit"), json_count(Some(limit)));
    }
    if let Some(today) = anki::proto_bytes(&fields, 8) {
        let _ = json
            .insert(String::from("reviewLimitToday"), day_limit_json(today));
    }
    if let Some(today) = anki::proto_bytes(&fields, 9) {
        let _ =
            json.insert(String::from("newLimitToday"), day_limit_json(today));
    }
    if let Some(retention) = anki::proto_float(&fields, 10)
        && retention.is_finite()
        && retention > 0.0
    {
        let _ = json.insert(
            String::from("desiredRetention"),
            Json::from(retention_percent(retention)),
        );
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "value, range and fallback mirror Anki's validation helper"
)]
const fn validated_u64(value: u64, min: u64, max: u64, fallback: u64) -> u64 {
    if value < min || value > max {
        fallback
    } else {
        value
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "value, range and fallback mirror Anki's validation helper"
)]
const fn validated_f32(value: f32, min: f32, max: f32, fallback: f32) -> f32 {
    if value.is_nan() || value < min || value > max {
        fallback
    } else {
        value
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "Anki stores deck retention as a whole percentage"
)]
fn retention_percent(value: f32) -> u32 {
    (value * 100.0).clamp(0.0, f32::from(u16::MAX)) as u32
}

/// Decodes schema-18 deck rows into the schema-11 `col.decks` JSON map.
///
/// # Errors
///
/// Returns an error when the prefix is not `main` or `incoming`.
pub fn decks_json_from_tables(
    conn: &Connection,
    prefix: &str,
) -> Result<Json, StoreError> {
    if !matches!(prefix, "main" | "incoming") {
        return Err(crate::invalid("invalid table prefix"));
    }
    let sql = format!(
        "SELECT id, name, mtime_secs, usn, common, kind FROM {prefix}.decks"
    );
    let mut statement = conn.prepare(&sql).map_err(crate::backend)?;
    let rows = statement
        .query_map([], |row| {
            let common: Value = row.get(4)?;
            let kind: Value = row.get(5)?;
            Ok((
                row.get::<_, i64>(0)?,
                text(row.get::<_, Value>(1)?),
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                common,
                kind,
            ))
        })
        .map_err(crate::backend)?;
    let mut decks = Map::new();
    for row in rows {
        let (id, name, modified, usn, common_value, kind_value) =
            row.map_err(crate::backend)?;
        let mut json = Map::new();
        let _ = json.insert(String::from("id"), Json::from(id));
        let _ = json.insert(
            String::from("name"),
            Json::from(anki::decode_deck_name(&name)),
        );
        let _ = json.insert(String::from("mod"), Json::from(modified));
        let _ = json.insert(String::from("usn"), Json::from(usn));
        if let (Value::Blob(common), Value::Blob(kind)) =
            (common_value, kind_value)
        {
            json.extend(deck_common_json(&common));
            deck_kind_json(&kind, &mut json);
        } else {
            json.extend(deck_common_json(&[]));
            deck_kind_json(&[], &mut json);
        }
        let _ = decks.insert(id.to_string(), Json::Object(json));
    }
    Ok(Json::Object(decks))
}

/// Decodes a schema-18 `deck_config.config` blob into the schema-11 JSON
/// object Anki reads from `col.dconf`.
#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "one mapping keeps Anki's field numbers and JSON keys side by side"
)]
pub fn deck_config_json(raw: &[u8]) -> Json {
    let fields = anki::protobuf_fields(raw).unwrap_or_default();
    let number = |field: u32| anki::proto_varint(&fields, field);
    let number_or =
        |field: u32, fallback: u64| number(field).unwrap_or(fallback);
    let requested_float =
        |field: u32| anki::proto_float(&fields, field).unwrap_or(0.0);
    let learn = anki::proto_floats(&fields, 1);
    let relearn = anki::proto_floats(&fields, 2);
    let easy_days = anki::proto_floats(&fields, 4);
    let initial_ease = validated_f32(requested_float(11), 1.31, 5.0, 2.5);
    let initial_factor = ease_thousands(initial_ease);
    let good = validated_u64(number_or(18, 0), 1, 36_500, 1);
    let easy = validated_u64(number_or(19, 0), 1, 36_500, 4);
    let mut json = Map::new();
    let _ = json.insert(
        String::from("maxTaken"),
        Json::from(validated_u64(number_or(24, 0), 1, 9_999, 60)),
    );
    let _ = json
        .insert(String::from("autoplay"), Json::Bool(number_or(23, 0) == 0));
    let _ = json.insert(
        String::from("timer"),
        Json::from(u8::from(number_or(25, 0) != 0)),
    );
    let _ =
        json.insert(String::from("replayq"), Json::Bool(number_or(26, 0) == 0));
    let _ = json.insert(
        String::from("new"),
        json!({
            "bury": number_or(27, 0) != 0,
            "delays": learn.into_iter().map(Json::from).collect::<Vec<_>>(),
            "initialFactor": initial_factor,
            "ints": [Json::from(good), Json::from(easy), Json::from(0)],
            "order": Json::from(i32::from(number_or(20, 0) == 0)),
            "perDay": Json::from(validated_u64(number_or(9, 0), 0, 9_999, 20)),
        }),
    );
    let _ = json.insert(
        String::from("rev"),
        json!({
            "bury": number_or(28, 0) != 0,
            "ease4": Json::from(validated_f32(requested_float(12), 1.0, 5.0, 1.3)),
            "ivlFct": Json::from(validated_f32(requested_float(15), 0.5, 2.0, 1.0)),
            "maxIvl": Json::from(validated_u64(number_or(16, 0), 1, 36_500, 36_500)),
            "perDay": Json::from(validated_u64(number_or(10, 0), 0, 9_999, 200)),
            "hardFactor": Json::from(validated_f32(requested_float(13), 0.5, 1.3, 1.2)),
        }),
    );
    let _ = json.insert(
        String::from("lapse"),
        json!({
            "delays": relearn.into_iter().map(Json::from).collect::<Vec<_>>(),
            "leechAction": Json::from(number_or(21, 0)),
            "leechFails": Json::from(validated_u64(number_or(22, 0), 1, 9_999, 8)),
            "minInt": Json::from(validated_u64(number_or(17, 0), 1, 36_500, 1)),
            "mult": Json::from(validated_f32(requested_float(14), 0.0, 1.0, 0.0)),
        }),
    );
    let _ = json.insert(String::from("dyn"), Json::Bool(false));
    let _ = json.insert(String::from("newMix"), Json::from(number_or(30, 0)));
    let _ = json.insert(
        String::from("newPerDayMinimum"),
        Json::from(number_or(35, 0)),
    );
    let _ = json.insert(
        String::from("interdayLearningMix"),
        Json::from(number_or(31, 0)),
    );
    let _ =
        json.insert(String::from("reviewOrder"), Json::from(number_or(33, 0)));
    let _ =
        json.insert(String::from("newSortOrder"), Json::from(number_or(32, 0)));
    let _ = json.insert(
        String::from("newGatherPriority"),
        Json::from(number_or(34, 0)),
    );
    let _ = json.insert(
        String::from("buryInterdayLearning"),
        Json::Bool(number_or(29, 0) != 0),
    );
    let _ = json.insert(
        String::from("fsrsWeights"),
        Json::Array(
            anki::proto_floats(&fields, 3)
                .into_iter()
                .map(Json::from)
                .collect(),
        ),
    );
    for (key, field) in [("fsrsParams5", 5_u32), ("fsrsParams6", 6)] {
        let _ = json.insert(
            key.to_owned(),
            Json::Array(
                anki::proto_floats(&fields, field)
                    .into_iter()
                    .map(Json::from)
                    .collect(),
            ),
        );
    }
    let _ = json.insert(
        String::from("desiredRetention"),
        Json::from(validated_f32(requested_float(37), 0.7, 0.99, 0.9)),
    );
    let _ = json.insert(
        String::from("ignoreRevlogsBeforeDate"),
        Json::from(anki::proto_string(&fields, 46).unwrap_or_default()),
    );
    let _ = json.insert(
        String::from("easyDaysPercentages"),
        Json::Array(easy_days.into_iter().map(Json::from).collect()),
    );
    let _ = json.insert(
        String::from("stopTimerOnAnswer"),
        Json::Bool(number_or(38, 0) != 0),
    );
    let _ = json.insert(
        String::from("secondsToShowQuestion"),
        Json::from(requested_float(41)),
    );
    let _ = json.insert(
        String::from("secondsToShowAnswer"),
        Json::from(requested_float(42)),
    );
    let _ = json
        .insert(String::from("questionAction"), Json::from(number_or(36, 0)));
    let _ =
        json.insert(String::from("answerAction"), Json::from(number_or(43, 0)));
    let _ = json.insert(
        String::from("waitForAudio"),
        Json::Bool(number_or(44, 0) != 0),
    );
    let _ = json.insert(
        String::from("sm2Retention"),
        Json::from(validated_f32(requested_float(40), 0.7, 0.97, 0.9)),
    );
    let _ = json.insert(
        String::from("weightSearch"),
        Json::from(anki::proto_string(&fields, 45).unwrap_or_default()),
    );
    if let Some(other) = anki::proto_bytes(&fields, 255) {
        merge_other_json(&mut json, other);
    }
    Json::Object(json)
}

const DECK_CONFIG_TOP_KEYS: &[&str] = &[
    "id",
    "mod",
    "name",
    "usn",
    "maxTaken",
    "autoplay",
    "timer",
    "replayq",
    "new",
    "rev",
    "lapse",
    "dyn",
    "newMix",
    "newPerDayMinimum",
    "interdayLearningMix",
    "reviewOrder",
    "newSortOrder",
    "newGatherPriority",
    "buryInterdayLearning",
    "fsrsWeights",
    "fsrsParams5",
    "fsrsParams6",
    "desiredRetention",
    "ignoreRevlogsBeforeDate",
    "easyDaysPercentages",
    "stopTimerOnAnswer",
    "secondsToShowQuestion",
    "secondsToShowAnswer",
    "questionAction",
    "answerAction",
    "waitForAudio",
    "sm2Retention",
    "weightSearch",
];
const DECK_CONFIG_NEW_KEYS: &[&str] =
    &["bury", "delays", "initialFactor", "ints", "order", "perDay"];
const DECK_CONFIG_REV_KEYS: &[&str] =
    &["bury", "ease4", "ivlFct", "maxIvl", "perDay", "hardFactor"];
const DECK_CONFIG_LAPSE_KEYS: &[&str] =
    &["delays", "leechAction", "leechFails", "minInt", "mult"];

fn json_flag(value: Option<&Json>) -> bool {
    match value {
        Some(Json::Bool(flag)) => *flag,
        Some(Json::Number(number)) => number.as_u64().unwrap_or_default() != 0,
        Some(Json::String(text)) => !text.is_empty(),
        Some(Json::Array(items)) => !items.is_empty(),
        Some(Json::Object(object)) => !object.is_empty(),
        Some(Json::Null) | None => false,
    }
}

fn json_number(value: Option<&Json>, fallback: u64) -> u64 {
    value.and_then(Json::as_u64).unwrap_or(fallback)
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "Anki stores these settings as f32"
)]
fn json_float(value: Option<&Json>, fallback: f32) -> f32 {
    value
        .and_then(Json::as_f64)
        .map_or(fallback, |number| number as f32)
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "Anki stores these settings as f32"
)]
fn json_floats(value: Option<&Json>) -> Vec<f32> {
    value
        .and_then(Json::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Json::as_f64)
                .map(|number| number as f32)
                .collect()
        })
        .unwrap_or_default()
}

fn json_text<'a>(value: Option<&'a Json>, fallback: &'a str) -> &'a str {
    value.and_then(Json::as_str).unwrap_or(fallback)
}

/// Encodes a schema-11 deck config JSON object as an Anki
/// `DeckConfig.Config` blob.
#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "one mapping keeps Anki's JSON keys and field numbers side by side"
)]
pub fn deck_config_blob(config: &Json) -> Vec<u8> {
    let new = config.get("new");
    let rev = config.get("rev");
    let lapse = config.get("lapse");
    let mut bytes = Vec::new();
    for (field, values) in [
        (1, json_floats(new.and_then(|value| value.get("delays")))),
        (2, json_floats(lapse.and_then(|value| value.get("delays")))),
        (3, json_floats(config.get("fsrsWeights"))),
        (4, json_floats(config.get("easyDaysPercentages"))),
        (5, json_floats(config.get("fsrsParams5"))),
        (6, json_floats(config.get("fsrsParams6"))),
    ] {
        if !values.is_empty() {
            anki::floats_field(&mut bytes, field, &values);
        }
    }
    let order = json_number(new.and_then(|value| value.get("order")), 1);
    let ints = new
        .and_then(|value| value.get("ints"))
        .and_then(Json::as_array);
    let good = ints
        .and_then(|items| items.first())
        .and_then(Json::as_u64)
        .unwrap_or(1);
    let easy = ints
        .and_then(|items| items.get(1))
        .and_then(Json::as_u64)
        .unwrap_or(4);
    for (field, value) in [
        (
            9,
            json_number(new.and_then(|value| value.get("perDay")), 20),
        ),
        (
            10,
            json_number(rev.and_then(|value| value.get("perDay")), 200),
        ),
        (
            16,
            json_number(rev.and_then(|value| value.get("maxIvl")), 36_500),
        ),
        (
            17,
            json_number(lapse.and_then(|value| value.get("minInt")), 1),
        ),
        (18, good),
        (19, easy),
        (20, u64::from(order == 0)),
        (
            21,
            json_number(lapse.and_then(|value| value.get("leechAction")), 1),
        ),
        (
            22,
            json_number(lapse.and_then(|value| value.get("leechFails")), 8),
        ),
        (
            23,
            u64::from(
                !config
                    .get("autoplay")
                    .is_none_or(|value| json_flag(Some(value))),
            ),
        ),
        (24, json_number(config.get("maxTaken"), 60)),
        (25, json_number(config.get("timer"), 0)),
        (
            26,
            u64::from(
                !config
                    .get("replayq")
                    .is_none_or(|value| json_flag(Some(value))),
            ),
        ),
        (
            27,
            u64::from(json_flag(new.and_then(|value| value.get("bury")))),
        ),
        (
            28,
            u64::from(json_flag(rev.and_then(|value| value.get("bury")))),
        ),
        (29, u64::from(json_flag(config.get("buryInterdayLearning")))),
        (30, json_number(config.get("newMix"), 0)),
        (31, json_number(config.get("interdayLearningMix"), 0)),
        (32, json_number(config.get("newSortOrder"), 0)),
        (33, json_number(config.get("reviewOrder"), 0)),
        (34, json_number(config.get("newGatherPriority"), 0)),
        (35, json_number(config.get("newPerDayMinimum"), 0)),
        (36, json_number(config.get("questionAction"), 0)),
        (38, u64::from(json_flag(config.get("stopTimerOnAnswer")))),
        (43, json_number(config.get("answerAction"), 0)),
        (
            44,
            u64::from(
                config
                    .get("waitForAudio")
                    .is_none_or(|value| json_flag(Some(value))),
            ),
        ),
    ] {
        anki::varint_field(&mut bytes, field, value);
    }
    for (field, value) in [
        (
            11,
            json_float(
                new.and_then(|value| value.get("initialFactor")),
                2_500.0,
            ) / 1_000.0,
        ),
        (
            12,
            json_float(rev.and_then(|value| value.get("ease4")), 1.3),
        ),
        (
            13,
            json_float(rev.and_then(|value| value.get("hardFactor")), 1.2),
        ),
        (
            14,
            json_float(lapse.and_then(|value| value.get("mult")), 0.0),
        ),
        (
            15,
            json_float(rev.and_then(|value| value.get("ivlFct")), 1.0),
        ),
        (37, json_float(config.get("desiredRetention"), 0.9)),
        (40, json_float(config.get("sm2Retention"), 0.9)),
        (41, json_float(config.get("secondsToShowQuestion"), 0.0)),
        (42, json_float(config.get("secondsToShowAnswer"), 0.0)),
    ] {
        anki::float_field(&mut bytes, field, value);
    }
    for (field, value) in [
        (45, json_text(config.get("weightSearch"), "")),
        (46, json_text(config.get("ignoreRevlogsBeforeDate"), "")),
    ] {
        if !value.is_empty() {
            anki::string_field(&mut bytes, field, value);
        }
    }
    if let Some(other) = deck_config_other(config) {
        anki::message_field(&mut bytes, 255, &other);
    }
    bytes
}

fn unknown_keys(
    object: &Map<String, Json>,
    known: &[&str],
) -> Map<String, Json> {
    let mut kept = Map::new();
    for (key, value) in object {
        if !known.contains(&key.as_str()) {
            let _ = kept.insert(key.clone(), value.clone());
        }
    }
    kept
}

fn deck_config_other(config: &Json) -> Option<Vec<u8>> {
    let mut other = config
        .as_object()
        .map(|object| unknown_keys(object, DECK_CONFIG_TOP_KEYS))
        .unwrap_or_default();
    for (section, known) in [
        ("new", DECK_CONFIG_NEW_KEYS),
        ("rev", DECK_CONFIG_REV_KEYS),
        ("lapse", DECK_CONFIG_LAPSE_KEYS),
    ] {
        let Some(object) = config.get(section).and_then(Json::as_object) else {
            continue;
        };
        let nested = unknown_keys(object, known);
        if !nested.is_empty() {
            let _ = other.insert(section.to_owned(), Json::Object(nested));
        }
    }
    if other.is_empty() {
        None
    } else {
        serde_json::to_vec(&Json::Object(other)).ok()
    }
}

/// Restores add-on settings Anki keeps in the trailing `other` blob.
fn merge_other_json(json: &mut Map<String, Json>, raw: &[u8]) {
    let Ok(other) = serde_json::from_slice::<Map<String, Json>>(raw) else {
        return;
    };
    for (key, value) in other {
        if matches!(key.as_str(), "new" | "rev" | "lapse") {
            merge_nested_json(json, &key, &value);
        } else if !json.contains_key(&key) {
            let _ = json.insert(key, value);
        }
    }
}

fn merge_nested_json(json: &mut Map<String, Json>, key: &str, value: &Json) {
    let (Some(source), Some(target)) = (
        value.as_object(),
        json.get_mut(key).and_then(Json::as_object_mut),
    ) else {
        return;
    };
    for (inner, entry) in source {
        if !target.contains_key(inner) {
            let _ = target.insert(inner.clone(), entry.clone());
        }
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "Anki stores initial ease as a thousands-scaled integer"
)]
fn ease_thousands(value: f32) -> u16 {
    (value * 1000.0).clamp(0.0, 65535.0) as u16
}

/// Decodes schema-18 deck configs into the schema-11 `col.dconf` JSON map.
///
/// # Errors
///
/// Returns an error when the prefix is not `main` or `incoming`.
pub fn deck_configs_json_from_tables(
    conn: &Connection,
    prefix: &str,
) -> Result<Json, StoreError> {
    if !matches!(prefix, "main" | "incoming") {
        return Err(crate::invalid("invalid table prefix"));
    }
    let sql = format!(
        "SELECT id, name, mtime_secs, usn, config FROM {prefix}.deck_config"
    );
    let mut statement = conn.prepare(&sql).map_err(crate::backend)?;
    let rows = statement
        .query_map([], |row| {
            let config: Value = row.get(4)?;
            Ok((
                row.get::<_, i64>(0)?,
                text(row.get::<_, Value>(1)?),
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                config,
            ))
        })
        .map_err(crate::backend)?;
    let mut configs = Map::new();
    for row in rows {
        let (id, name, modified, usn, config) = row.map_err(crate::backend)?;
        let mut json = match config {
            Value::Blob(bytes) => deck_config_json(&bytes),
            Value::Null
            | Value::Integer(_)
            | Value::Real(_)
            | Value::Text(_) => deck_config_json(&[]),
        };
        if let Some(object) = json.as_object_mut() {
            let _ = object.insert(String::from("id"), Json::from(id));
            let _ = object.insert(String::from("name"), Json::from(name));
            let _ = object.insert(String::from("mod"), Json::from(modified));
            let _ = object.insert(String::from("usn"), Json::from(usn));
        }
        let _ = configs.insert(id.to_string(), json);
    }
    Ok(Json::Object(configs))
}
fn blob(bytes: &[u8], field: u32) -> Option<u64> {
    anki::protobuf_varint(bytes, field)
}

fn config_number(config: &Value, field: u32) -> Option<u64> {
    match config {
        Value::Blob(bytes) => blob(bytes, field),
        Value::Null | Value::Integer(_) | Value::Real(_) | Value::Text(_) => {
            None
        }
    }
}

fn raw_blob(config: Value) -> Option<Vec<u8>> {
    match config {
        Value::Blob(bytes) => Some(bytes),
        Value::Null | Value::Integer(_) | Value::Real(_) | Value::Text(_) => {
            None
        }
    }
}

fn notetype_config(raw: Option<&Vec<u8>>) -> (u64, Option<u64>, String) {
    let Some(bytes) = raw else {
        return (0, None, String::new());
    };
    (
        blob(bytes, 1).unwrap_or_default(),
        blob(bytes, 9),
        anki::protobuf_string(bytes, 3).unwrap_or_default(),
    )
}

/// Reads schema-18 note types from normalized tables.
pub fn models_from_tables(
    conn: &Connection,
) -> Result<Vec<NoteModel>, StoreError> {
    models_from_tables_with_prefix(conn, "main")
}

/// Reads schema-18 note types from a named database prefix.
/// Import passes `incoming` for the attached package collection.
///
/// # Errors
///
/// Returns an error when the prefix is invalid or the tables cannot be read.
pub fn models_from_tables_with_prefix(
    conn: &Connection,
    prefix: &str,
) -> Result<Vec<NoteModel>, StoreError> {
    if !matches!(prefix, "main" | "incoming") {
        return Err(crate::invalid("invalid table prefix"));
    }
    let mut models = BTreeMap::new();
    {
        let sql = format!("SELECT id, name, config FROM {prefix}.notetypes");
        let mut statement = conn.prepare(&sql).map_err(crate::backend)?;
        let rows = statement
            .query_map([], |row| {
                let config: Value = row.get(2)?;
                let raw = raw_blob(config);
                let (kind, stock_kind, css) = notetype_config(raw.as_ref());
                Ok((
                    row.get::<_, i64>(0)?,
                    text(row.get::<_, Value>(1)?),
                    css,
                    kind,
                    stock_kind,
                    raw,
                ))
            })
            .map_err(crate::backend)?;
        for row in rows {
            let (id, name, css, kind, stock_kind, raw) =
                row.map_err(crate::backend)?;
            models.insert(
                id,
                NoteModel {
                    id,
                    name,
                    fields: Vec::new(),
                    templates: Vec::new(),
                    css,
                    kind: NoteKind::from_code(
                        i32::try_from(kind).unwrap_or_default(),
                    ),
                    original_stock_kind: stock_kind
                        .and_then(|kind| i32::try_from(kind).ok()),
                    field_tags: Vec::new(),
                    template_ordinals: Vec::new(),
                    raw_config: raw,
                },
            );
        }
    }
    add_fields(conn, prefix, &mut models)?;
    add_templates(conn, prefix, &mut models)?;
    for model in models.values_mut() {
        infer_cloze_tags(model);
    }
    Ok(models.into_values().collect())
}

fn add_fields(
    conn: &Connection,
    prefix: &str,
    models: &mut BTreeMap<i64, NoteModel>,
) -> Result<(), StoreError> {
    let sql = format!(
        "SELECT ntid, name, config FROM {prefix}.fields ORDER BY ntid, ord"
    );
    let mut statement = conn.prepare(&sql).map_err(crate::backend)?;
    let rows = statement
        .query_map([], |row| {
            let config: Value = row.get(2)?;
            Ok((
                row.get::<_, i64>(0)?,
                text(row.get::<_, Value>(1)?),
                config_number(&config, 10),
            ))
        })
        .map_err(crate::backend)?;
    for row in rows {
        let (id, name, tag) = row.map_err(crate::backend)?;
        if let Some(model) = models.get_mut(&id) {
            model.fields.push(name);
            model
                .field_tags
                .push(tag.and_then(|raw_tag| u32::try_from(raw_tag).ok()));
        }
    }
    Ok(())
}

fn add_templates(
    conn: &Connection,
    prefix: &str,
    models: &mut BTreeMap<i64, NoteModel>,
) -> Result<(), StoreError> {
    let sql = format!(
        "SELECT ntid, ord, name, config FROM {prefix}.templates ORDER BY ntid, ord"
    );
    let mut statement = conn.prepare(&sql).map_err(crate::backend)?;
    let rows = statement
        .query_map([], |row| {
            let config: Value = row.get(3)?;
            let (question, answer) = match &config {
                Value::Blob(bytes) => (
                    anki::protobuf_string(bytes, 1).unwrap_or_default(),
                    anki::protobuf_string(bytes, 2).unwrap_or_default(),
                ),
                Value::Null
                | Value::Integer(_)
                | Value::Real(_)
                | Value::Text(_) => (String::new(), String::new()),
            };
            let ordinal = row.get::<_, i64>(1)?.try_into().unwrap_or_default();
            Ok((
                row.get::<_, i64>(0)?,
                text(row.get::<_, Value>(2)?),
                question,
                answer,
                ordinal,
            ))
        })
        .map_err(crate::backend)?;
    for row in rows {
        let (id, name, question, answer, ordinal) =
            row.map_err(crate::backend)?;
        if let Some(model) = models.get_mut(&id) {
            model.templates.push(CardTemplate {
                name,
                question_format: question,
                answer_format: answer,
                ord: ordinal,
            });
            model.template_ordinals.push(ordinal);
        }
    }
    Ok(())
}

/// Builds schema-11 model JSON from decoded note types.
///
/// Unknown protobuf config bytes are kept in `_rankiRawConfig` so an export
/// can be imported by Anki without losing settings it does not expose.
#[expect(
    clippy::excessive_nesting,
    reason = "optional model JSON keys are clearer inline"
)]
#[must_use]
pub fn models_to_json(models: &[NoteModel]) -> Json {
    let mut map = Map::new();
    for model in models {
        let fields = model
            .fields
            .iter()
            .enumerate()
            .map(|(ord, name)| {
                let tag = model.field_tags.get(ord).copied().flatten();
                json!({
                    "name": name,
                    "ord": ord,
                    "sticky": false,
                    "rtl": false,
                    "font": "Arial",
                    "size": 20,
                    "tag": tag,
                })
            })
            .collect::<Vec<_>>();
        let templates = model
            .templates
            .iter()
            .map(|template| {
                json!({
                    "name": template.name,
                    "ord": template.ord,
                    "qfmt": template.question_format,
                    "afmt": template.answer_format,
                    "bqfmt": "",
                    "bafmt": "",
                    "did": null,
                    "bfont": "",
                    "bsize": 0,
                })
            })
            .collect::<Vec<_>>();
        let mut entry = json!({
            "id": model.id,
            "name": model.name,
            "type": model.kind.code(),
            "mod": 0,
            "usn": 0,
            "sortf": 0,
            "did": 1,
            "flds": fields,
            "tmpls": templates,
            "css": model.css,
            "latexPre": "",
            "latexPost": "\\end{document}",
            "latexsvg": false,
            "req": [],
        });
        if let Some(object) = entry.as_object_mut() {
            if let Some(kind) = model.original_stock_kind {
                let _ = object.insert("originalStockKind".into(), json!(kind));
            }
            if let Some(raw) = &model.raw_config {
                let _ = object
                    .insert("_rankiRawConfig".into(), json!(encode_hex(raw)));
            }
        }
        let _ = map.insert(model.id.to_string(), entry);
    }
    Json::Object(map)
}

pub fn table_exists(conn: &Connection, name: &str) -> Result<bool, StoreError> {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [name],
        |_| Ok(()),
    )
    .optional()
    .map(|value| value.is_some())
    .map_err(crate::backend)
}

fn conversion(error: StoreError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(error))
}

fn string_array(value: Option<&Json>, key: &str) -> Vec<String> {
    let Some(items) = value.and_then(Json::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| item.get(key).and_then(Json::as_str))
        .map(ToOwned::to_owned)
        .collect()
}
fn optional_numbers(value: Option<&Json>, key: &str) -> Vec<u64> {
    let Some(items) = value.and_then(Json::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| item.get(key).and_then(Json::as_u64))
        .collect()
}

/// Anki stores field tags in protobuf configs. Older cloze models omit them,
/// so recover them from the cloze filter in the question template.
fn infer_cloze_tags(model: &mut NoteModel) {
    if !model.is_cloze()
        || model.field_tags.iter().any(Option::is_some)
        || model.templates.is_empty()
    {
        return;
    }
    let Some(question) =
        model.templates.first().map(|item| &item.question_format)
    else {
        return;
    };
    let Some(start) = question.find("{{cloze:") else {
        return;
    };
    let name_start = start.saturating_add(8);
    let Some(name) = question
        .get(name_start..)
        .and_then(|tail| tail.find("}}").and_then(|end| tail.get(..end)))
    else {
        return;
    };
    let Some(index) = model.fields.iter().position(|field| field == name)
    else {
        return;
    };
    if let Some(tag) = model.field_tags.get_mut(index) {
        *tag = Some(0);
    }
    if let Some(tag) =
        model
            .field_tags
            .iter_mut()
            .enumerate()
            .find_map(|(position, tag)| {
                (position != index && tag.is_none()).then_some(tag)
            })
    {
        *tag = Some(1);
    }
}

fn text(raw: Value) -> String {
    match raw {
        Value::Text(value) => value,
        Value::Blob(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Value::Integer(number) => number.to_string(),
        Value::Real(number) => number.to_string(),
        Value::Null => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::deck_config_json;

    #[test]
    fn empty_repeated_fields_are_preserved_not_invented() {
        let config = deck_config_json(&[0x48, 0x14]);
        assert_eq!(config["new"]["delays"], json!([]));
        assert_eq!(config["lapse"]["delays"], json!([]));
        assert_eq!(config["new"]["perDay"], 20);
    }

    #[test]
    fn omitted_zero_settings_decode_as_zero_not_defaults() {
        let config = deck_config_json(&[]);
        assert_eq!(config["new"]["perDay"], 0);
        assert_eq!(config["rev"]["perDay"], 0);
        assert_eq!(config["lapse"]["leechAction"], 0);
        assert_eq!(config["maxTaken"], 60);
        assert_eq!(config["new"]["ints"], json!([1, 4, 0]));
    }

    #[test]
    fn negative_study_counters_decode_as_signed_int32() {
        let mut common = vec![0x18, 0x05, 0x20];
        let mut value = u64::MAX.saturating_sub(4);
        while value >= 0x80 {
            common.push(u8::try_from(value & 0x7f).unwrap() | 0x80);
            value >>= 7;
        }
        common.push(u8::try_from(value & 0x7f).unwrap());
        let decoded = super::deck_common_json(&common);
        assert_eq!(decoded["newToday"], json!([5, -5]));
    }

    #[test]
    fn deck_config_blob_round_trips_schema11_settings() {
        let source = json!({
            "new": {
                "bury": false,
                "delays": [2.0, 15.0],
                "initialFactor": 2500,
                "ints": [3, 9, 0],
                "order": 0,
                "perDay": 0,
            },
            "rev": {
                "bury": false,
                "ease4": 1.4,
                "ivlFct": 1.1,
                "maxIvl": 180,
                "perDay": 0,
                "hardFactor": 1.25,
            },
            "lapse": {
                "delays": [7.0],
                "leechAction": 0,
                "leechFails": 4,
                "minInt": 2,
                "mult": 0.4,
            },
            "maxTaken": 45,
            "autoplay": true,
            "timer": 1,
            "replayq": false,
            "dyn": false,
            "newMix": 1,
            "newPerDayMinimum": 5,
            "interdayLearningMix": 2,
            "reviewOrder": 3,
            "newSortOrder": 4,
            "newGatherPriority": 5,
            "buryInterdayLearning": true,
            "fsrsWeights": [],
            "fsrsParams5": [],
            "fsrsParams6": [0.2, 1.3],
            "desiredRetention": 0.85,
            "ignoreRevlogsBeforeDate": "2023-01-01",
            "easyDaysPercentages": [1.0, 0.5, 1.0, 1.0, 1.0, 1.0, 1.0],
            "stopTimerOnAnswer": true,
            "secondsToShowQuestion": 1.5,
            "secondsToShowAnswer": 2.5,
            "questionAction": 1,
            "answerAction": 2,
            "waitForAudio": false,
            "sm2Retention": 0.8,
            "weightSearch": "preset",
            "fuzz": 0.05,
        });
        let decoded = deck_config_json(&super::deck_config_blob(&source));
        assert_eq!(decoded["new"]["delays"], json!([2.0, 15.0]));
        assert_eq!(decoded["new"]["perDay"], 0);
        assert_eq!(decoded["new"]["ints"], json!([3, 9, 0]));
        assert_eq!(decoded["new"]["order"], 0);
        assert_eq!(decoded["rev"]["maxIvl"], 180);
        assert_eq!(decoded["rev"]["perDay"], 0);
        assert_eq!(decoded["lapse"]["leechAction"], 0);
        let mult = decoded["lapse"]["mult"].as_f64().unwrap_or_default();
        assert!((mult - 0.4).abs() < 1e-6, "{mult}");
        assert_eq!(decoded["maxTaken"], 45);
        assert_eq!(decoded["timer"], 1);
        assert_eq!(decoded["replayq"], false);
        assert_eq!(decoded["newGatherPriority"], 5);
        assert_eq!(decoded["buryInterdayLearning"], true);
        let params = decoded["fsrsParams6"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        assert_eq!(params.len(), 2);
        for (value, expected) in params.iter().zip([0.2, 1.3]) {
            let number = value.as_f64().unwrap_or_default();
            assert!((number - expected).abs() < 1e-6, "{number}");
        }
        assert_eq!(decoded["waitForAudio"], false);
        assert_eq!(decoded["weightSearch"], "preset");
        assert_eq!(decoded["fuzz"], json!(0.05));
    }

    #[test]
    fn new_card_insert_order_maps_between_the_two_enums() {
        let due = deck_config_json(&[]);
        assert_eq!(due["new"]["order"], 1);
        let random = deck_config_json(&[0xa0, 0x01, 0x01]);
        assert_eq!(random["new"]["order"], 0);
    }
}
