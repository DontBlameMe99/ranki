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
        .prepare("SELECT id, name, common FROM decks")
        .map_err(crate::backend)?;
    let rows = statement
        .query_map([], |row| {
            let blob: Value = row.get(2)?;
            let description = match blob {
                Value::Blob(bytes) => {
                    anki::protobuf_string(&bytes, 2).unwrap_or_default()
                }
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
    if let Some(index) = model.fields.iter().position(|field| field == name)
        && let Some(tag) = model.field_tags.get_mut(index)
    {
        *tag = Some(0);
    }
    if model.fields.len() > 1
        && let Some(tag) = model.field_tags.get_mut(1)
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
