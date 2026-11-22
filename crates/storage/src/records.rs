//! Conversion between Anki rows and the shared model types.

use std::collections::BTreeMap;

use ranki_model::Card;
use ranki_model::CardQueue;
use ranki_model::CardTemplate;
use ranki_model::CardType;
use ranki_model::Deck;
use ranki_model::Id;
use ranki_model::Note;
use ranki_model::NoteModel;
use ranki_model::Rating;
use ranki_model::RevLog;
use ranki_model::RevLogKind;
use ranki_model::StoreError;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::Result as SqlResult;
use rusqlite::Row;
use rusqlite::types::Type;
use rusqlite::types::Value;
use serde_json::Value as Json;

use crate::anki;

pub struct ColData {
    pub modified: i64,
    pub scm: i64,
    pub usn: i32,
    pub conf: String,
    pub models: String,
    pub decks: String,
}

pub fn read_col(conn: &Connection) -> Result<ColData, StoreError> {
    conn.query_row(
        "SELECT mod, scm, usn, conf, models, decks, dconf, tags FROM col WHERE id = 1",
        [],
        |row| {
            let usn = row.get::<_, i64>(2)?;
            Ok(ColData {
                modified: row.get(0)?,
                scm: row.get(1)?,
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
        models.push(NoteModel {
            id,
            name: value
                .get("name")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .into(),
            fields: string_array(value.get("flds"), "name"),
            templates: templates.map_or_else(Vec::new, |items| {
                items
                    .iter()
                    .map(|template| CardTemplate {
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
                    })
                    .collect()
            }),
            css: value
                .get("css")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .into(),
        });
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
fn css_from_config(config: Value) -> String {
    match config {
        Value::Blob(bytes) => {
            anki::protobuf_string(&bytes, 3).unwrap_or_default()
        }
        Value::Null | Value::Integer(_) | Value::Real(_) | Value::Text(_) => {
            String::new()
        }
    }
}

pub fn models_from_tables(
    conn: &Connection,
) -> Result<Vec<NoteModel>, StoreError> {
    let mut models = BTreeMap::new();
    {
        let mut statement = conn
            .prepare("SELECT id, name, config FROM notetypes")
            .map_err(crate::backend)?;
        let rows = statement
            .query_map([], |row| {
                let config: Value = row.get(2)?;
                Ok((
                    row.get::<_, i64>(0)?,
                    text(row.get::<_, Value>(1)?),
                    css_from_config(config),
                ))
            })
            .map_err(crate::backend)?;
        for row in rows {
            let (id, name, css) = row.map_err(crate::backend)?;
            models.insert(
                id,
                NoteModel {
                    id,
                    name,
                    fields: Vec::new(),
                    templates: Vec::new(),
                    css,
                },
            );
        }
    }
    add_fields(conn, &mut models)?;
    add_templates(conn, &mut models)?;
    Ok(models.into_values().collect())
}

fn add_fields(
    conn: &Connection,
    models: &mut BTreeMap<i64, NoteModel>,
) -> Result<(), StoreError> {
    let mut statement = conn
        .prepare("SELECT ntid, name FROM fields ORDER BY ntid, ord")
        .map_err(crate::backend)?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, text(row.get::<_, Value>(1)?)))
        })
        .map_err(crate::backend)?;
    for row in rows {
        let (id, name) = row.map_err(crate::backend)?;
        if let Some(model) = models.get_mut(&id) {
            model.fields.push(name);
        }
    }
    Ok(())
}

fn add_templates(
    conn: &Connection,
    models: &mut BTreeMap<i64, NoteModel>,
) -> Result<(), StoreError> {
    let mut statement = conn
        .prepare("SELECT ntid, name, config FROM templates ORDER BY ntid, ord")
        .map_err(crate::backend)?;
    let rows = statement
        .query_map([], |row| {
            let config: Value = row.get(2)?;
            let (question, answer) = match config {
                Value::Blob(bytes) => (
                    anki::protobuf_string(&bytes, 1).unwrap_or_default(),
                    anki::protobuf_string(&bytes, 2).unwrap_or_default(),
                ),
                Value::Null
                | Value::Integer(_)
                | Value::Real(_)
                | Value::Text(_) => (String::new(), String::new()),
            };
            Ok((
                row.get::<_, i64>(0)?,
                text(row.get::<_, Value>(1)?),
                question,
                answer,
            ))
        })
        .map_err(crate::backend)?;
    for row in rows {
        let (id, name, question, answer) = row.map_err(crate::backend)?;
        if let Some(model) = models.get_mut(&id) {
            model.templates.push(CardTemplate {
                name,
                question_format: question,
                answer_format: answer,
            });
        }
    }
    Ok(())
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

fn text(raw: Value) -> String {
    match raw {
        Value::Text(value) => value,
        Value::Blob(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Value::Integer(number) => number.to_string(),
        Value::Real(number) => number.to_string(),
        Value::Null => String::new(),
    }
}
