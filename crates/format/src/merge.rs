use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::Transaction;
use rusqlite::params;
use rusqlite::types::Value as SqlValue;
use serde_json::Map;
use serde_json::Value;

use crate::FormatError;
use crate::ImportReport;
use crate::error::db_error;
use crate::error::json_error;
use crate::media::MediaRename;
use crate::media::ReferenceMatcher;
use crate::package::maybe_decompress;
use crate::schema::encode_json;

const DAY_SECS: i64 = 86_400;

#[derive(Debug)]
pub struct ColJson {
    pub crt: i64,
    pub conf: Value,
    pub models: Value,
    pub decks: Value,
    pub dconf: Value,
    pub tags: Value,
}

struct MergeState {
    next_id: i64,
    source_day_delta: i64,
    model_map: BTreeMap<i64, i64>,
    deck_map: BTreeMap<i64, i64>,
    dconf_map: BTreeMap<i64, i64>,
    note_map: BTreeMap<i64, Option<i64>>,
    card_map: BTreeMap<i64, Option<i64>>,
    imported_decks: Vec<(i64, i64)>,
    imported_deck_configs: Vec<(i64, i64)>,
    imported_notetypes: Vec<(i64, i64)>,
}

struct NoteRow {
    id: i64,
    guid: String,
    mid: i64,
    modified: i64,
    usn: i32,
    tags: String,
    fields: String,
    sort_field: String,
    checksum: i64,
    flags: i32,
    data: String,
}

struct CardRow {
    id: i64,
    note_id: i64,
    deck_id: i64,
    ordinal: i32,
    modified: i64,
    usn: i32,
    kind: i32,
    queue: i32,
    due: i64,
    interval: i32,
    factor: i32,
    reps: i32,
    lapses: i32,
    left: i32,
    original_due: i64,
    original_deck_id: i64,
    flags: i32,
    data: String,
}

struct RevlogRow {
    id: i64,
    card_id: i64,
    usn: i32,
    ease: i32,
    interval: i32,
    last_interval: i32,
    factor: i32,
    time_ms: i32,
    kind: i32,
}

pub fn merge_collection(
    connection: &mut Connection,
    renames: &[MediaRename],
    media_count: usize,
) -> Result<ImportReport, FormatError> {
    let transaction = connection
        .transaction()
        .map_err(db_error("cannot start import transaction"))?;
    rewrite_incoming_notes(&transaction, renames)?;
    let destination = read_col(&transaction, "main")?;
    let incoming = read_col(&transaction, "incoming")?;
    let now_secs = current_unix_seconds()?;
    let day_delta = source_day_delta(now_secs, incoming.crt, destination.crt)?;
    let mut state = MergeState::new(
        global_next_id(&transaction, &destination, &incoming)?,
        day_delta,
    );
    let mut merged = destination;
    merge_col_json(&mut merged, &incoming, &mut state)?;
    write_col(&transaction, &merged)?;
    write_modern_rows(&transaction, &merged, &state)?;

    let notes = merge_notes(&transaction, &mut state)?;
    let cards = merge_cards(&transaction, &mut state)?;
    let _reviews = merge_revlog(&transaction, &mut state)?;

    transaction
        .commit()
        .map_err(db_error("cannot commit imported collection"))?;
    Ok(ImportReport {
        notes,
        cards,
        media: media_count,
    })
}

fn current_unix_seconds() -> Result<i64, FormatError> {
    let elapsed =
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| {
                FormatError::with_source("cannot read system clock", error)
            })?;
    i64::try_from(elapsed.as_secs())
        .map_err(|_| FormatError::new("system clock out of range"))
}

fn day_number(now_secs: i64, crt: i64) -> Result<i64, FormatError> {
    now_secs
        .checked_sub(crt)
        .map(|elapsed| elapsed.div_euclid(DAY_SECS))
        .ok_or_else(|| FormatError::new("invalid collection creation time"))
}

fn source_day_delta(
    now_secs: i64,
    incoming_crt: i64,
    destination_crt: i64,
) -> Result<i64, FormatError> {
    let incoming_day = day_number(now_secs, incoming_crt)?;
    let destination_day = day_number(now_secs, destination_crt)?;
    incoming_day
        .checked_sub(destination_day)
        .ok_or_else(|| FormatError::new("invalid collection creation time"))
}

impl MergeState {
    const fn new(next_id: i64, source_day_delta: i64) -> Self {
        Self {
            next_id,
            source_day_delta,
            model_map: BTreeMap::new(),
            deck_map: BTreeMap::new(),
            dconf_map: BTreeMap::new(),
            note_map: BTreeMap::new(),
            card_map: BTreeMap::new(),
            imported_decks: Vec::new(),
            imported_deck_configs: Vec::new(),
            imported_notetypes: Vec::new(),
        }
    }

    fn take_id(&mut self) -> Result<i64, FormatError> {
        let id = self.next_id;
        self.next_id = id.checked_add(1).ok_or_else(|| {
            FormatError::new("identifier overflow while importing package")
        })?;
        Ok(id)
    }
}

fn read_col(
    connection: &Connection,
    prefix: &str,
) -> Result<ColJson, FormatError> {
    let sql = format!(
        "SELECT crt, conf, models, decks, dconf, tags FROM {prefix}.col WHERE id = 1"
    );
    let row = connection
        .query_row(&sql, [], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })
        .optional()
        .map_err(db_error(format!(
            "cannot read {prefix} collection metadata"
        )))?
        .ok_or_else(|| {
            FormatError::new(format!("{prefix} collection has no metadata row"))
        })?;

    let conf = decode_json(row.1, "conf")?;
    let models = decode_json(row.2, "models")?;
    let decks = decode_json(row.3, "decks")?;
    let dconf = decode_json(row.4, "dconf")?;
    let tags = decode_json(row.5, "tags")?;
    let incoming = prefix == "incoming";
    let table_models = has_table(connection, prefix, "notetypes")?
        && (!incoming || is_empty_object(&models));
    let table_decks = has_table(connection, prefix, "decks")?
        && (!incoming || is_empty_object(&decks));
    let table_dconf = has_table(connection, prefix, "deck_config")?
        && (!incoming || is_empty_object(&dconf));
    Ok(ColJson {
        crt: row.0,
        conf: if incoming && is_empty_object(&conf) {
            modern_conf(connection, prefix)
        } else {
            conf
        },
        models: if table_models {
            modern_models(connection, prefix)?
        } else {
            models
        },
        decks: if table_decks {
            ranki_storage::decks_json_from_tables(connection, prefix).map_err(
                |error| FormatError::with_source("cannot decode decks", error),
            )?
        } else {
            decks
        },
        dconf: if table_dconf {
            ranki_storage::deck_configs_json_from_tables(connection, prefix)
                .map_err(|error| {
                    FormatError::with_source(
                        "cannot decode deck configs",
                        error,
                    )
                })?
        } else {
            dconf
        },
        tags,
    })
}

fn decode_json(
    text: Option<String>,
    label: &str,
) -> Result<Value, FormatError> {
    let Some(contents) = text else {
        return Ok(Value::Object(Map::new()));
    };
    if contents.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    serde_json::from_str(&contents)
        .map_err(json_error(format!("invalid collection {label} JSON")))
}

fn is_empty_object(value: &Value) -> bool {
    value.as_object().is_some_and(Map::is_empty)
}

fn modern_conf(connection: &Connection, prefix: &str) -> Value {
    let mut conf = Map::new();
    let Ok(mut statement) =
        connection.prepare(&format!("SELECT KEY, val FROM {prefix}.config"))
    else {
        return Value::Object(conf);
    };
    let Ok(rows) = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
    }) else {
        return Value::Object(conf);
    };
    for row in rows {
        let Ok((key, stored)) = row else {
            continue;
        };
        let Ok(bytes) = maybe_decompress(stored) else {
            continue;
        };
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        conf.insert(key, value);
    }
    Value::Object(conf)
}

fn modern_models(
    connection: &Connection,
    prefix: &str,
) -> Result<Value, FormatError> {
    if !has_table(connection, prefix, "notetypes")? {
        return Ok(Value::Object(Map::new()));
    }
    let models =
        ranki_storage::models_from_tables_with_prefix(connection, prefix)
            .map_err(|error| {
                FormatError::with_source("cannot decode note types", error)
            })?;
    Ok(ranki_storage::models_to_json(&models))
}

fn write_col(
    transaction: &Transaction<'_>,
    col: &ColJson,
) -> Result<(), FormatError> {
    transaction
        .execute(
            "UPDATE main.col SET conf = ?, models = ?, decks = ?, dconf = ?, tags = ? WHERE id = 1",
            params![
                encode_json(&col.conf)?,
                encode_json(&col.models)?,
                encode_json(&col.decks)?,
                encode_json(&col.dconf)?,
                encode_json(&col.tags)?,
            ],
        )
        .map_err(db_error("cannot write collection metadata"))?;
    Ok(())
}

/// Copies decks and note types that were added to the merged metadata into a
/// schema-18 destination, which reads those rows instead of the `col` blobs.
fn write_modern_rows(
    transaction: &Transaction<'_>,
    merged: &ColJson,
    state: &MergeState,
) -> Result<(), FormatError> {
    write_deck_rows(transaction, merged, state)?;
    write_note_type_rows(transaction, state)
}

fn write_deck_rows(
    transaction: &Transaction<'_>,
    merged: &ColJson,
    state: &MergeState,
) -> Result<(), FormatError> {
    if !has_table(transaction, "main", "decks")? {
        return Ok(());
    }
    let copy = has_table(transaction, "incoming", "decks")?;
    for (old_id, new_id) in &state.imported_decks {
        if copy {
            let _ = copy_deck_row(transaction, *old_id, *new_id)?;
            rewrite_copied_deck_config(transaction, *new_id, &state.dconf_map)?;
        } else {
            insert_deck_row(transaction, &merged.decks, *new_id)?;
        }
    }
    write_deck_configs(transaction, merged, state)?;
    Ok(())
}

/// Points a copied schema-18 deck at the preset ID the merge actually wrote.
fn rewrite_copied_deck_config(
    transaction: &Transaction<'_>,
    deck_id: i64,
    dconf_map: &BTreeMap<i64, i64>,
) -> Result<(), FormatError> {
    if dconf_map.is_empty() {
        return Ok(());
    }
    let kind: Vec<u8> = transaction
        .query_row(
            "SELECT kind FROM main.decks WHERE id = ?1",
            [deck_id],
            |row| row.get(0),
        )
        .map_err(db_error("cannot read imported deck kind"))?;
    let outer = ranki_storage::protobuf_fields(&kind).unwrap_or_default();
    let Some(normal) = ranki_storage::proto_bytes(&outer, 1) else {
        return Ok(());
    };
    let fields = ranki_storage::protobuf_fields(normal).unwrap_or_default();
    let Some(raw_conf) = ranki_storage::proto_varint(&fields, 1) else {
        return Ok(());
    };
    let old_conf = i64::try_from(raw_conf).unwrap_or(1);
    let Some(mapped_conf) = dconf_map.get(&old_conf).copied() else {
        return Ok(());
    };
    if mapped_conf == old_conf {
        return Ok(());
    }
    let Some(inner) = rewrite_varint_field(
        normal,
        1,
        u64::try_from(mapped_conf).unwrap_or(1),
    ) else {
        return Ok(());
    };
    let Some(rewritten) = rewrite_message_field(&kind, 1, &inner) else {
        return Ok(());
    };
    transaction
        .execute(
            "UPDATE main.decks SET kind = ?1 WHERE id = ?2",
            params![rewritten, deck_id],
        )
        .map(|_| ())
        .map_err(db_error("cannot update imported deck preset"))
}

fn read_pb_varint(bytes: &[u8], mut pos: usize) -> Option<(u64, usize)> {
    let mut value = 0_u64;
    let mut shift = 0_u32;
    loop {
        let byte = *bytes.get(pos)?;
        pos = pos.checked_add(1)?;
        value |= u64::from(byte & 0x7f).checked_shl(shift)?;
        if byte & 0x80 == 0 {
            return Some((value, pos));
        }
        shift = shift.checked_add(7)?;
    }
}

fn pb_field_end(bytes: &[u8], wire: u64, after_tag: usize) -> Option<usize> {
    match wire {
        0 => read_pb_varint(bytes, after_tag).map(|(_, end)| end),
        1 => after_tag.checked_add(8),
        2 => {
            let (raw_length, start) = read_pb_varint(bytes, after_tag)?;
            let size = usize::try_from(raw_length).ok()?;
            start.checked_add(size)
        }
        5 => after_tag.checked_add(4),
        _ => None,
    }
}

fn rewrite_varint_field(
    bytes: &[u8],
    wanted: u32,
    value: u64,
) -> Option<Vec<u8>> {
    let mut output = Vec::with_capacity(bytes.len());
    let mut pos = 0;
    let mut found = false;
    while pos < bytes.len() {
        let start = pos;
        let (tag, after_tag) = read_pb_varint(bytes, pos)?;
        let field = u32::try_from(tag >> 3).ok()?;
        let wire = tag & 0x07;
        let end = pb_field_end(bytes, wire, after_tag)?;
        output.extend_from_slice(bytes.get(start..after_tag)?);
        if field == wanted && wire == 0 && !found {
            pb_value(&mut output, value);
            found = true;
        } else {
            output.extend_from_slice(bytes.get(after_tag..end)?);
        }
        pos = end;
    }
    found.then_some(output)
}

fn rewrite_message_field(
    bytes: &[u8],
    wanted: u32,
    body: &[u8],
) -> Option<Vec<u8>> {
    let mut output = Vec::with_capacity(bytes.len());
    let mut pos = 0;
    let mut found = false;
    while pos < bytes.len() {
        let start = pos;
        let (tag, after_tag) = read_pb_varint(bytes, pos)?;
        let field = u32::try_from(tag >> 3).ok()?;
        let wire = tag & 0x07;
        let end = pb_field_end(bytes, wire, after_tag)?;
        if field == wanted && wire == 2 && !found {
            output.extend_from_slice(bytes.get(start..after_tag)?);
            pb_value(
                &mut output,
                u64::try_from(body.len()).unwrap_or_default(),
            );
            output.extend_from_slice(body);
            found = true;
        } else {
            output.extend_from_slice(bytes.get(start..end)?);
        }
        pos = end;
    }
    found.then_some(output)
}

fn write_deck_configs(
    transaction: &Transaction<'_>,
    merged: &ColJson,
    state: &MergeState,
) -> Result<(), FormatError> {
    if !has_table(transaction, "main", "deck_config")? {
        return Ok(());
    }
    if has_table(transaction, "incoming", "deck_config")? {
        for (old_id, new_id) in &state.imported_deck_configs {
            let _ = copy_config_row(
                transaction,
                "deck_config",
                (*old_id, *new_id),
            )?;
        }
    } else {
        for (_, new_id) in &state.imported_deck_configs {
            insert_deck_config_row(transaction, &merged.dconf, *new_id)?;
        }
    }
    Ok(())
}

fn insert_deck_config_row(
    transaction: &Transaction<'_>,
    dconf: &Value,
    id: i64,
) -> Result<(), FormatError> {
    let entries = as_object(dconf, "deck config")?;
    let entry = entries.get(&id.to_string()).ok_or_else(|| {
        FormatError::new("merged deck config is missing from metadata")
    })?;
    let name = entry
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("Default");
    let modified = entry.get("mod").and_then(Value::as_i64).unwrap_or(0);
    let usn = entry.get("usn").and_then(Value::as_i64).unwrap_or(0);
    let config = ranki_storage::deck_config_blob(entry);
    transaction
        .execute(
            "INSERT OR IGNORE INTO main.deck_config (id, name, mtime_secs, usn, config)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, name, modified, usn, config],
        )
        .map(|_| ())
        .map_err(db_error("cannot write imported deck config"))
}

fn write_note_type_rows(
    transaction: &Transaction<'_>,
    state: &MergeState,
) -> Result<(), FormatError> {
    if !has_table(transaction, "main", "notetypes")?
        || !has_table(transaction, "incoming", "notetypes")?
    {
        return Ok(());
    }
    for (old_id, new_id) in &state.imported_notetypes {
        let inserted =
            copy_config_row(transaction, "notetypes", (*old_id, *new_id))?;
        if inserted == 0 {
            continue;
        }
        let _ = copy_notetype_fields(transaction, *old_id, *new_id)?;
        let _ = copy_notetype_templates(transaction, *old_id, *new_id)?;
    }
    Ok(())
}

fn has_table(
    connection: &Connection,
    prefix: &str,
    table: &str,
) -> Result<bool, FormatError> {
    let sql = format!(
        "SELECT 1 FROM {prefix}.sqlite_master WHERE type = 'table' AND name = ?1"
    );
    connection
        .query_row(&sql, [table], |_| Ok(()))
        .optional()
        .map(|row| row.is_some())
        .map_err(db_error(format!("cannot inspect {prefix} schema")))
}

fn copy_deck_row(
    transaction: &Transaction<'_>,
    old_id: i64,
    new_id: i64,
) -> Result<usize, FormatError> {
    transaction
        .execute(
            "INSERT OR IGNORE INTO main.decks (id, name, mtime_secs, usn, common, kind)
             SELECT ?2, name, mtime_secs, usn, common, kind FROM incoming.decks WHERE id = ?1",
            params![old_id, new_id],
        )
        .map_err(db_error("cannot copy imported deck"))
}

fn copy_config_row(
    transaction: &Transaction<'_>,
    table: &str,
    (old_id, new_id): (i64, i64),
) -> Result<usize, FormatError> {
    let sql = format!(
        "INSERT OR IGNORE INTO main.{table} (id, name, mtime_secs, usn, config)
         SELECT ?2, name, mtime_secs, usn, config FROM incoming.{table} WHERE id = ?1"
    );
    transaction
        .execute(&sql, params![old_id, new_id])
        .map_err(db_error(format!("cannot copy imported {table}")))
}

fn copy_notetype_fields(
    transaction: &Transaction<'_>,
    old_id: i64,
    new_id: i64,
) -> Result<usize, FormatError> {
    transaction
        .execute(
            "INSERT INTO main.fields (ntid, ord, name, config)
             SELECT ?2, ord, name, config FROM incoming.fields WHERE ntid = ?1",
            params![old_id, new_id],
        )
        .map_err(db_error("cannot copy imported note type fields"))
}

fn copy_notetype_templates(
    transaction: &Transaction<'_>,
    old_id: i64,
    new_id: i64,
) -> Result<usize, FormatError> {
    transaction
        .execute(
            "INSERT INTO main.templates (ntid, ord, name, mtime_secs, usn, config)
             SELECT ?2, ord, name, mtime_secs, usn, config FROM incoming.templates WHERE ntid = ?1",
            params![old_id, new_id],
        )
        .map_err(db_error("cannot copy imported note type templates"))
}

fn insert_deck_row(
    transaction: &Transaction<'_>,
    decks: &Value,
    id: i64,
) -> Result<(), FormatError> {
    let entries = as_object(decks, "decks")?;
    let entry = entries.get(&id.to_string()).ok_or_else(|| {
        FormatError::new("merged deck is missing from collection metadata")
    })?;
    let name = entry
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let usn = entry
        .get("usn")
        .and_then(Value::as_i64)
        .and_then(|usn| i32::try_from(usn).ok())
        .unwrap_or_default();
    let modified = entry.get("mod").and_then(Value::as_i64).unwrap_or_default();
    let kind = deck_kind_blob(entry);
    transaction
        .execute(
            "INSERT OR IGNORE INTO main.decks (id, name, mtime_secs, usn, common, kind)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id,
                ranki_storage::encode_deck_name(name),
                modified,
                usn,
                ranki_storage::deck_common(),
                kind,
            ],
        )
        .map(|_| ())
        .map_err(db_error("cannot write imported deck"))
}

fn deck_kind_blob(entry: &Value) -> Vec<u8> {
    let filtered = match entry.get("dyn") {
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_u64().unwrap_or(0) != 0,
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Null | Value::Array(_) | Value::Object(_)) | None => false,
    };
    if filtered {
        return filtered_deck_kind(entry);
    }
    normal_deck_kind(entry)
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "Anki stores deck retention as f32"
)]
fn normal_deck_kind(entry: &Value) -> Vec<u8> {
    let mut normal = Vec::new();
    let config_id = preset_id(entry.get("conf")).unwrap_or(1);
    pb_varint(&mut normal, 1, u64::try_from(config_id).unwrap_or(1));
    pb_varint(&mut normal, 2, unsigned(entry.get("extendNew")));
    pb_varint(&mut normal, 3, unsigned(entry.get("extendRev")));
    if let Some(description) = entry.get("desc").and_then(Value::as_str)
        && !description.is_empty()
    {
        pb_string(&mut normal, 4, description);
    }
    if entry.get("md").and_then(Value::as_bool).unwrap_or(false) {
        pb_varint(&mut normal, 5, 1);
    }
    if let Some(limit) = entry.get("reviewLimit").and_then(Value::as_u64) {
        pb_varint(&mut normal, 6, limit);
    }
    if let Some(limit) = entry.get("newLimit").and_then(Value::as_u64) {
        pb_varint(&mut normal, 7, limit);
    }
    if let Some(today) = entry
        .get("reviewLimitToday")
        .filter(|value| value.is_object())
    {
        pb_message(&mut normal, 8, &day_limit(today));
    }
    if let Some(today) =
        entry.get("newLimitToday").filter(|value| value.is_object())
    {
        pb_message(&mut normal, 9, &day_limit(today));
    }
    if let Some(retention) =
        entry.get("desiredRetention").and_then(Value::as_f64)
        && retention > 0.0
    {
        pb_float(&mut normal, 10, (retention / 100.0) as f32);
    }
    let mut container = Vec::new();
    pb_message(&mut container, 1, &normal);
    container
}

fn day_limit(value: &Value) -> Vec<u8> {
    let mut limit = Vec::new();
    pb_varint(&mut limit, 1, unsigned(value.get("limit")));
    pb_varint(&mut limit, 2, unsigned(value.get("today")));
    limit
}

/// Builds a schema-18 `decks.kind` blob for a filtered deck.
fn filtered_deck_kind(entry: &Value) -> Vec<u8> {
    let mut filtered = Vec::new();
    let resched = entry
        .get("resched")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    pb_varint(&mut filtered, 1, u64::from(resched));
    if let Some(terms) = entry.get("terms").and_then(Value::as_array) {
        for term in terms {
            let mut body = Vec::new();
            pb_string(
                &mut body,
                1,
                term.get("search")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            );
            pb_varint(&mut body, 2, unsigned(term.get("limit")));
            pb_varint(&mut body, 3, unsigned(term.get("order")));
            pb_message(&mut filtered, 2, &body);
        }
    }
    push_filtered_delays(&mut filtered, entry);
    pb_varint(&mut filtered, 4, unsigned(entry.get("previewDelay")));
    pb_varint(&mut filtered, 7, unsigned(entry.get("previewAgainSecs")));
    pb_varint(&mut filtered, 5, unsigned(entry.get("previewHardSecs")));
    pb_varint(&mut filtered, 6, unsigned(entry.get("previewGoodSecs")));
    let mut container = Vec::new();
    pb_message(&mut container, 2, &filtered);
    container
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "Anki stores step delays as f32 seconds"
)]
fn push_filtered_delays(filtered: &mut Vec<u8>, entry: &Value) {
    let Some(delays) = entry.get("delays").and_then(Value::as_array) else {
        return;
    };
    for value in delays.iter().filter_map(Value::as_f64) {
        pb_float(filtered, 3, value as f32);
    }
}

fn unsigned(value: Option<&Value>) -> u64 {
    value.and_then(Value::as_u64).unwrap_or(0)
}

fn pb_varint(bytes: &mut Vec<u8>, field: u32, value: u64) {
    pb_tag(bytes, field, 0);
    pb_value(bytes, value);
}

fn pb_string(bytes: &mut Vec<u8>, field: u32, value: &str) {
    pb_tag(bytes, field, 2);
    pb_value(bytes, u64::try_from(value.len()).unwrap_or(0));
    bytes.extend_from_slice(value.as_bytes());
}

fn pb_message(bytes: &mut Vec<u8>, field: u32, body: &[u8]) {
    pb_tag(bytes, field, 2);
    pb_value(bytes, u64::try_from(body.len()).unwrap_or(0));
    bytes.extend_from_slice(body);
}

fn pb_float(bytes: &mut Vec<u8>, field: u32, value: f32) {
    pb_tag(bytes, field, 5);
    bytes.extend_from_slice(&value.to_bits().to_le_bytes());
}

fn pb_tag(bytes: &mut Vec<u8>, field: u32, wire: u8) {
    pb_value(bytes, u64::from(field << 3) | u64::from(wire));
}

fn pb_value(bytes: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        bytes.push(u8::try_from(value & 0x7f).unwrap_or(0) | 0x80);
        value >>= 7;
    }
    bytes.push(u8::try_from(value & 0x7f).unwrap_or(0));
}

fn global_next_id(
    connection: &Connection,
    destination: &ColJson,
    incoming: &ColJson,
) -> Result<i64, FormatError> {
    let mut maximum = None;
    for (main_table, incoming_table) in [
        ("main.notes", "incoming.notes"),
        ("main.cards", "incoming.cards"),
        ("main.revlog", "incoming.revlog"),
    ] {
        maximum = maximum
            .max(max_table_id(connection, main_table)?)
            .max(max_table_id(connection, incoming_table)?);
    }
    for value in [
        &destination.conf,
        &destination.models,
        &destination.decks,
        &destination.dconf,
        &destination.tags,
        &incoming.conf,
        &incoming.models,
        &incoming.decks,
        &incoming.dconf,
        &incoming.tags,
    ] {
        maximum = maximum.max(max_json_id(value)?);
    }
    maximum.unwrap_or(0).checked_add(1).ok_or_else(|| {
        FormatError::new("identifier overflow while importing package")
    })
}

fn max_table_id(
    connection: &Connection,
    table: &str,
) -> Result<Option<i64>, FormatError> {
    let sql = format!("SELECT MAX(id) FROM {table}");
    connection
        .query_row(&sql, [], |row| row.get::<_, Option<i64>>(0))
        .map_err(db_error(format!("cannot read identifiers from {table}")))
}

fn max_json_id(value: &Value) -> Result<Option<i64>, FormatError> {
    let map = as_object(value, "collection metadata")?;
    let mut maximum = None;
    for (key, entry) in map {
        if let Ok(id) = key.parse::<i64>() {
            maximum = maximum.max(Some(id));
        }
        maximum = maximum.max(entry.get("id").and_then(Value::as_i64));
    }
    Ok(maximum)
}

fn sql_text(value: SqlValue) -> String {
    match value {
        SqlValue::Text(text) => text,
        SqlValue::Integer(number) => number.to_string(),
        SqlValue::Real(number) => number.to_string(),
        SqlValue::Blob(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        SqlValue::Null => String::new(),
    }
}

fn as_object(
    value: &Value,
    label: &str,
) -> Result<Map<String, Value>, FormatError> {
    value.as_object().cloned().ok_or_else(|| {
        FormatError::new(format!("{label} must be a JSON object"))
    })
}

fn merge_col_json(
    merged: &mut ColJson,
    incoming: &ColJson,
    state: &mut MergeState,
) -> Result<(), FormatError> {
    merge_tags(&mut merged.tags, &incoming.tags)?;
    let dconf_map = merge_dconf(&mut merged.dconf, &incoming.dconf, state)?;
    state.dconf_map = dconf_map;
    let deck_map = merge_decks(&mut merged.decks, &incoming.decks, state)?;
    state.deck_map = deck_map;
    let model_map = merge_models(&mut merged.models, &incoming.models, state)?;
    state.model_map = model_map;
    Ok(())
}

fn merge_tags(
    destination: &mut Value,
    incoming: &Value,
) -> Result<(), FormatError> {
    let incoming_tags = as_object(incoming, "tags")?;
    let tags = destination.as_object_mut().ok_or_else(|| {
        FormatError::new("collection tags must be a JSON object")
    })?;
    for (name, value) in incoming_tags {
        tags.entry(name).or_insert(value);
    }
    Ok(())
}

fn merge_dconf(
    destination: &mut Value,
    incoming: &Value,
    state: &mut MergeState,
) -> Result<BTreeMap<i64, i64>, FormatError> {
    let mut configs = as_object(destination, "deck config")?;
    let incoming_configs = as_object(incoming, "deck config")?;
    let mut used = collect_ids(&configs);
    let mut mapping = BTreeMap::new();

    for (key, mut value) in incoming_configs {
        let old_id = entry_id(&key, &value, "deck config")?;
        let existing = configs.get(&old_id.to_string()).cloned();
        if existing.as_ref() == Some(&value) {
            let _ = mapping.insert(old_id, old_id);
            continue;
        }
        let new_id = if used.contains(&old_id) {
            allocate(&mut used, state)?
        } else {
            let _ = used.insert(old_id);
            old_id
        };
        set_id(&mut value, new_id, "deck config")?;
        let _ = mapping.insert(old_id, new_id);
        state.imported_deck_configs.push((old_id, new_id));
        let _ = configs.insert(new_id.to_string(), value);
    }

    *destination = Value::Object(configs);
    Ok(mapping)
}

fn merge_decks(
    destination: &mut Value,
    incoming: &Value,
    state: &mut MergeState,
) -> Result<BTreeMap<i64, i64>, FormatError> {
    let mut decks = as_object(destination, "decks")?;
    let incoming_decks = as_object(incoming, "decks")?;
    let mut named = BTreeMap::new();
    for (key, value) in &decks {
        let id = entry_id(key, value, "deck")?;
        if let Some(name) = deck_name(value) {
            let _ = named.entry(name).or_insert(id);
        }
    }
    let dconf_map = state.dconf_map.clone();
    let mut remapper = DeckRemapper {
        used: collect_ids(&decks),
        state,
        dconf_map,
        mapping: BTreeMap::new(),
    };

    for (key, mut value) in incoming_decks {
        let old_id = entry_id(&key, &value, "deck")?;
        let merged =
            deck_name(&value).and_then(|name| named.get(&name).copied());
        remapper.visit(&mut value, merged)?;
        let new_id = value
            .get("id")
            .and_then(Value::as_i64)
            .ok_or_else(|| FormatError::new("deck has no identifier"))?;
        remapper.mapping.insert(old_id, new_id);
        if merged.is_none() {
            decks.insert(new_id.to_string(), value);
        }
    }

    *destination = Value::Object(decks);
    Ok(remapper.mapping)
}

fn deck_name(value: &Value) -> Option<String> {
    value
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_lowercase)
}

struct DeckRemapper<'a> {
    used: BTreeSet<i64>,
    state: &'a mut MergeState,
    dconf_map: BTreeMap<i64, i64>,
    mapping: BTreeMap<i64, i64>,
}

impl DeckRemapper<'_> {
    fn visit(
        &mut self,
        value: &mut Value,
        merged: Option<i64>,
    ) -> Result<(), FormatError> {
        if let Some(object) = value.as_object_mut() {
            self.remap_object(object, merged)?;
        }
        self.visit_children(value)
    }

    #[expect(
        clippy::excessive_nesting,
        reason = "id allocation and JSON updates are clearer together"
    )]
    fn remap_object(
        &mut self,
        object: &mut Map<String, Value>,
        merged: Option<i64>,
    ) -> Result<(), FormatError> {
        if let Some(old_id) = object.get("id").and_then(Value::as_i64) {
            let new_id = if let Some(id) = merged {
                id
            } else if self.used.contains(&old_id) {
                allocate(&mut self.used, self.state)?
            } else {
                let _ = self.used.insert(old_id);
                old_id
            };
            object.insert("id".to_owned(), Value::from(new_id));
            self.mapping.insert(old_id, new_id);
            self.state.imported_decks.push((old_id, new_id));
        }
        remap_number(object, "conf", &self.dconf_map);
        Ok(())
    }

    fn visit_children(&mut self, value: &mut Value) -> Result<(), FormatError> {
        let Some(children) =
            value.get_mut("children").and_then(Value::as_array_mut)
        else {
            return Ok(());
        };
        for child in children.iter_mut() {
            self.visit(child, None)?;
        }
        Ok(())
    }
}

fn merge_models(
    destination: &mut Value,
    incoming: &Value,
    state: &mut MergeState,
) -> Result<BTreeMap<i64, i64>, FormatError> {
    let mut models = as_object(destination, "models")?;
    let incoming_models = as_object(incoming, "models")?;
    let deck_map = state.deck_map.clone();
    let mut named = BTreeMap::new();
    for (key, value) in &models {
        let id = entry_id(key, value, "model")?;
        if let Some(name) = model_name(value) {
            let _ = named.entry((name, field_names(value))).or_insert(id);
        }
    }
    let mut used = collect_ids(&models);
    let mut mapping = BTreeMap::new();

    for (key, mut value) in incoming_models {
        let old_id = entry_id(&key, &value, "model")?;
        if let Some(id) = model_name(&value)
            .and_then(|name| named.get(&(name, field_names(&value))))
        {
            mapping.insert(old_id, *id);
            state.imported_notetypes.push((old_id, *id));
            continue;
        }
        let new_id = if used.contains(&old_id) {
            allocate(&mut used, state)?
        } else {
            let _ = used.insert(old_id);
            old_id
        };
        set_id(&mut value, new_id, "model")?;
        remap_model_decks(&mut value, &deck_map);
        mapping.insert(old_id, new_id);
        state.imported_notetypes.push((old_id, new_id));
        models.insert(new_id.to_string(), value);
    }

    *destination = Value::Object(models);
    Ok(mapping)
}

fn model_name(value: &Value) -> Option<String> {
    value
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_lowercase)
}

fn field_names(value: &Value) -> Vec<String> {
    value
        .get("flds")
        .and_then(Value::as_array)
        .map(|fields| {
            fields
                .iter()
                .map(|field| {
                    field
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned()
                })
                .collect()
        })
        .unwrap_or_default()
}

#[expect(
    clippy::excessive_nesting,
    reason = "model and template deck references are updated in one pass"
)]
fn remap_model_decks(model: &mut Value, deck_map: &BTreeMap<i64, i64>) {
    let Some(object) = model.as_object_mut() else {
        return;
    };
    remap_number(object, "did", deck_map);
    if let Some(templates) =
        object.get_mut("tmpls").and_then(Value::as_array_mut)
    {
        for template in templates.iter_mut() {
            let Some(template_object) = template.as_object_mut() else {
                continue;
            };
            remap_number(template_object, "did", deck_map);
        }
    }
}

fn remap_number(
    object: &mut Map<String, Value>,
    field: &str,
    mapping: &BTreeMap<i64, i64>,
) {
    if let Some(old_id) = preset_id(object.get(field))
        && let Some(new_id) = mapping.get(&old_id)
    {
        object.insert(field.to_owned(), Value::from(*new_id));
    }
}

fn preset_id(value: Option<&Value>) -> Option<i64> {
    let candidate = value?;
    candidate
        .as_i64()
        .or_else(|| candidate.as_str().and_then(|text| text.parse().ok()))
}

fn collect_ids(map: &Map<String, Value>) -> BTreeSet<i64> {
    let mut ids = BTreeSet::new();
    for (key, value) in map {
        if let Ok(id) = key.parse::<i64>() {
            let _ = ids.insert(id);
        }
        if let Some(id) = value.get("id").and_then(Value::as_i64) {
            let _ = ids.insert(id);
        }
    }
    ids
}

fn entry_id(key: &str, value: &Value, label: &str) -> Result<i64, FormatError> {
    if let Some(id) = value.get("id").and_then(Value::as_i64) {
        return Ok(id);
    }
    key.parse::<i64>().map_err(|_| {
        FormatError::new(format!("invalid {label} identifier '{key}'"))
    })
}

fn set_id(value: &mut Value, id: i64, label: &str) -> Result<(), FormatError> {
    let object = value.as_object_mut().ok_or_else(|| {
        FormatError::new(format!("{label} entry is not a JSON object"))
    })?;
    object.insert("id".to_owned(), Value::from(id));
    Ok(())
}

fn allocate(
    used: &mut BTreeSet<i64>,
    state: &mut MergeState,
) -> Result<i64, FormatError> {
    loop {
        let id = state.take_id()?;
        if used.insert(id) {
            return Ok(id);
        }
    }
}

fn increment(count: usize) -> Result<usize, FormatError> {
    count
        .checked_add(1)
        .ok_or_else(|| FormatError::new("too many records to import"))
}

fn rewrite_incoming_notes(
    connection: &Connection,
    renames: &[MediaRename],
) -> Result<(), FormatError> {
    if renames.is_empty() {
        return Ok(());
    }
    let matcher = ReferenceMatcher::new(renames);
    let mut statement = connection
        .prepare_cached("SELECT id, flds, sfld FROM incoming.notes")
        .map_err(db_error("cannot read package notes"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                sql_text(row.get::<_, SqlValue>(2)?),
            ))
        })
        .map_err(db_error("cannot read package notes"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db_error("cannot read package notes"))?;
    drop(statement);

    let mut update = connection
        .prepare_cached(
            "UPDATE incoming.notes SET flds = ?, sfld = ? WHERE id = ?",
        )
        .map_err(db_error("cannot rewrite package media references"))?;
    for (id, fields, sort_field) in rows {
        let new_fields = matcher.rewrite(&fields);
        let new_sort = matcher.rewrite(&sort_field);
        if new_fields.as_ref() != fields.as_str()
            || new_sort.as_ref() != sort_field.as_str()
        {
            update
                .execute(params![new_fields.as_ref(), new_sort.as_ref(), id])
                .map_err(db_error("cannot rewrite package media references"))?;
        }
    }
    Ok(())
}

fn all_ids(
    connection: &Connection,
    table: &str,
) -> Result<BTreeSet<i64>, FormatError> {
    let sql = format!("SELECT id FROM {table}");
    let mut statement = connection
        .prepare(&sql)
        .map_err(db_error(format!("cannot read identifiers from {table}")))?;
    let rows = statement
        .query_map([], |row| row.get::<_, i64>(0))
        .map_err(db_error(format!("cannot read identifiers from {table}")))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db_error(format!("cannot read identifiers from {table}")))?;
    Ok(rows.into_iter().collect())
}

fn existing_guids(
    connection: &Connection,
) -> Result<BTreeSet<String>, FormatError> {
    let mut statement = connection
        .prepare("SELECT guid FROM main.notes")
        .map_err(db_error("cannot read existing note guids"))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(db_error("cannot read existing note guids"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db_error("cannot read existing note guids"))?;
    Ok(rows.into_iter().collect())
}

const INSERT_NOTE_SQL: &str =
    "INSERT INTO main.notes (id, guid, mid, mod, usn, tags, flds, sfld, csum, flags, data)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";

fn merge_notes(
    transaction: &Transaction<'_>,
    state: &mut MergeState,
) -> Result<usize, FormatError> {
    let duplicate: Option<String> = transaction
        .query_row(
            "SELECT guid FROM incoming.notes GROUP BY guid HAVING COUNT(*) > 1 ORDER BY guid LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_error("cannot check package note guids"))?;
    if let Some(guid) = duplicate {
        return Err(FormatError::new(format!(
            "duplicate guid in package: '{guid}'"
        )));
    }

    let mut used = all_ids(transaction, "main.notes")?;
    let mut guids = existing_guids(transaction)?;
    let mut statement = transaction
        .prepare(
            "SELECT id, guid, mid, mod, usn, tags, flds, sfld, csum, flags, data
             FROM incoming.notes ORDER BY id",
        )
        .map_err(db_error("cannot read package notes"))?;
    let rows = statement
        .query_map([], |row| {
            Ok(NoteRow {
                id: row.get(0)?,
                guid: row.get(1)?,
                mid: row.get(2)?,
                modified: row.get(3)?,
                usn: row.get(4)?,
                tags: row.get(5)?,
                fields: row.get(6)?,
                sort_field: sql_text(row.get(7)?),
                checksum: row.get(8)?,
                flags: row.get(9)?,
                data: row.get(10)?,
            })
        })
        .map_err(db_error("cannot read package notes"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db_error("cannot read package notes"))?;
    drop(statement);

    let mut insert = transaction
        .prepare_cached(INSERT_NOTE_SQL)
        .map_err(db_error("cannot insert imported note"))?;
    let mut inserted = 0;
    for row in rows {
        if guids.contains(&row.guid) {
            state.note_map.insert(row.id, None);
            continue;
        }
        let id = if used.contains(&row.id) {
            allocate(&mut used, state)?
        } else {
            let _ = used.insert(row.id);
            row.id
        };
        let model_id =
            state.model_map.get(&row.mid).copied().unwrap_or(row.mid);
        insert
            .execute(params![
                id,
                row.guid,
                model_id,
                row.modified,
                row.usn,
                row.tags,
                row.fields,
                row.sort_field,
                row.checksum,
                row.flags,
                row.data,
            ])
            .map_err(db_error("cannot insert imported note"))?;
        state.note_map.insert(row.id, Some(id));
        let _ = guids.insert(row.guid);
        inserted = increment(inserted)?;
    }
    Ok(inserted)
}

fn merge_cards(
    transaction: &Transaction<'_>,
    state: &mut MergeState,
) -> Result<usize, FormatError> {
    let rows = read_card_rows(transaction)?;
    insert_cards(transaction, state, &rows)
}

fn read_card_rows(
    transaction: &Transaction<'_>,
) -> Result<Vec<CardRow>, FormatError> {
    let mut statement = transaction
        .prepare(
            "SELECT id, nid, did, ord, mod, usn, type, queue, due, ivl, factor, reps, lapses,
                    left, odue, odid, flags, data
             FROM incoming.cards ORDER BY id",
        )
        .map_err(db_error("cannot read package cards"))?;
    let rows = statement
        .query_map([], |row| {
            Ok(CardRow {
                id: row.get(0)?,
                note_id: row.get(1)?,
                deck_id: row.get(2)?,
                ordinal: row.get(3)?,
                modified: row.get(4)?,
                usn: row.get(5)?,
                kind: row.get(6)?,
                queue: row.get(7)?,
                due: row.get(8)?,
                interval: row.get(9)?,
                factor: row.get(10)?,
                reps: row.get(11)?,
                lapses: row.get(12)?,
                left: row.get(13)?,
                original_due: row.get(14)?,
                original_deck_id: row.get(15)?,
                flags: row.get(16)?,
                data: row.get(17)?,
            })
        })
        .map_err(db_error("cannot read package cards"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db_error("cannot read package cards"))?;
    Ok(rows)
}

const INSERT_CARD_SQL: &str =
    "INSERT INTO main.cards (id, nid, did, ord, mod, usn, type, queue, due, ivl,
         factor, reps, lapses, left, odue, odid, flags, data)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";

fn insert_cards(
    transaction: &Transaction<'_>,
    state: &mut MergeState,
    rows: &[CardRow],
) -> Result<usize, FormatError> {
    let mut used = all_ids(transaction, "main.cards")?;
    let mut insert = transaction
        .prepare_cached(INSERT_CARD_SQL)
        .map_err(db_error("cannot insert imported card"))?;
    let mut inserted = 0;
    for row in rows {
        let Some(note_id) = state.note_map.get(&row.note_id).copied().flatten()
        else {
            state.card_map.insert(row.id, None);
            continue;
        };
        let deck_id = state
            .deck_map
            .get(&row.deck_id)
            .copied()
            .unwrap_or(row.deck_id);
        let original_deck_id = if row.original_deck_id == 0 {
            0
        } else {
            state
                .deck_map
                .get(&row.original_deck_id)
                .copied()
                .unwrap_or(row.original_deck_id)
        };
        let id = if used.contains(&row.id) {
            allocate(&mut used, state)?
        } else {
            let _ = used.insert(row.id);
            row.id
        };
        // Review dues stay day numbers even while suspended or buried.
        let day_delta = state.source_day_delta;
        let due = shifted_due(row.due, row, day_delta)?;
        let original_due = if row.original_due == 0 {
            0
        } else {
            shifted_due(row.original_due, row, day_delta)?
        };
        insert
            .execute(params![
                id,
                note_id,
                deck_id,
                row.ordinal,
                row.modified,
                row.usn,
                row.kind,
                row.queue,
                due,
                row.interval,
                row.factor,
                row.reps,
                row.lapses,
                row.left,
                original_due,
                original_deck_id,
                row.flags,
                row.data,
            ])
            .map_err(db_error("cannot insert imported card"))?;
        state.card_map.insert(row.id, Some(id));
        inserted = increment(inserted)?;
    }
    Ok(inserted)
}

fn shifted_due(
    value: i64,
    row: &CardRow,
    source_day_delta: i64,
) -> Result<i64, FormatError> {
    if !matches!(row.queue, 2 | 3) && row.kind != 2 {
        return Ok(value);
    }
    value.checked_sub(source_day_delta).ok_or_else(|| {
        FormatError::new("card due overflow while importing package")
    })
}

const INSERT_REVLOG_SQL: &str =
    "INSERT INTO main.revlog (id, cid, usn, ease, ivl, lastIvl, factor, time, type)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)";

fn merge_revlog(
    transaction: &Transaction<'_>,
    state: &mut MergeState,
) -> Result<usize, FormatError> {
    let mut used = all_ids(transaction, "main.revlog")?;
    let mut statement = transaction
        .prepare(
            "SELECT id, cid, usn, ease, ivl, lastIvl, factor, time, type
             FROM incoming.revlog ORDER BY id",
        )
        .map_err(db_error("cannot read package review log"))?;
    let rows = statement
        .query_map([], |row| {
            Ok(RevlogRow {
                id: row.get(0)?,
                card_id: row.get(1)?,
                usn: row.get(2)?,
                ease: row.get(3)?,
                interval: row.get(4)?,
                last_interval: row.get(5)?,
                factor: row.get(6)?,
                time_ms: row.get(7)?,
                kind: row.get(8)?,
            })
        })
        .map_err(db_error("cannot read package review log"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db_error("cannot read package review log"))?;
    drop(statement);

    let mut insert = transaction
        .prepare_cached(INSERT_REVLOG_SQL)
        .map_err(db_error("cannot insert imported review log entry"))?;
    let mut inserted = 0;
    for row in rows {
        let Some(card_id) = state.card_map.get(&row.card_id).copied().flatten()
        else {
            continue;
        };
        let id = if used.contains(&row.id) {
            allocate(&mut used, state)?
        } else {
            let _ = used.insert(row.id);
            row.id
        };
        insert
            .execute(params![
                id,
                card_id,
                row.usn,
                row.ease,
                row.interval,
                row.last_interval,
                row.factor,
                row.time_ms,
                row.kind,
            ])
            .map_err(db_error("cannot insert imported review log entry"))?;
        inserted = increment(inserted)?;
    }
    Ok(inserted)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{MergeState, merge_models};

    fn basic(id: i64, fields: &[&str]) -> Value {
        json!({
            "id": id,
            "name": "Basic",
            "flds": fields
                .iter()
                .map(|name| json!({ "name": name }))
                .collect::<Vec<_>>(),
        })
    }

    #[test]
    fn reimport_matches_models_by_name_and_fields() {
        let mut destination = json!({
            "1": basic(1, &["Front", "Back"]),
            "2": basic(2, &["Front", "Back", "Extra"]),
        });
        let incoming = json!({
            "50": basic(50, &["Front", "Back", "Extra"]),
            "51": basic(51, &["Front", "Back"]),
        });
        let mut state = MergeState::new(1_000, 0);

        let mapping = merge_models(&mut destination, &incoming, &mut state)
            .expect("merge models");

        assert_eq!(mapping.get(&50), Some(&2));
        assert_eq!(mapping.get(&51), Some(&1));
        let models = destination.as_object().expect("models");
        assert_eq!(models.len(), 2);
    }

    #[test]
    fn filtered_deck_kind_accepts_boolean_dyn() {
        let deck = json!({
            "dyn": true,
            "resched": false,
            "terms": [["is:due", 5, 0]],
            "previewDelay": 0,
        });
        let kind = super::deck_kind_blob(&deck);
        let outer =
            ranki_storage::protobuf_fields(&kind).expect("kind container");
        assert!(ranki_storage::proto_bytes(&outer, 2).is_some());
    }

    #[test]
    fn normal_deck_kind_accepts_string_conf_and_null_day_limits() {
        let deck = json!({
            "conf": "2",
            "newLimitToday": null,
            "reviewLimitToday": null,
        });
        let kind = super::deck_kind_blob(&deck);
        let outer =
            ranki_storage::protobuf_fields(&kind).expect("kind container");
        let normal = ranki_storage::proto_bytes(&outer, 1).expect("normal");
        let fields =
            ranki_storage::protobuf_fields(normal).expect("normal body");
        assert_eq!(ranki_storage::proto_varint(&fields, 1), Some(2));
        assert!(ranki_storage::proto_bytes(&fields, 8).is_none());
        assert!(ranki_storage::proto_bytes(&fields, 9).is_none());
    }

    #[test]
    fn copied_deck_kind_preset_reference_can_be_rewritten() {
        let mut normal = Vec::new();
        super::pb_varint(&mut normal, 1, 2);
        super::pb_string(&mut normal, 4, "Instructions");
        let mut container = Vec::new();
        super::pb_message(&mut container, 1, &normal);

        let inner = super::rewrite_varint_field(&normal, 1, 7)
            .expect("rewrite config id");
        let rewritten = super::rewrite_message_field(&container, 1, &inner)
            .expect("rewrite normal field");
        let outer =
            ranki_storage::protobuf_fields(&rewritten).expect("container");
        let normal = ranki_storage::proto_bytes(&outer, 1).expect("normal");
        let fields =
            ranki_storage::protobuf_fields(normal).expect("normal body");
        assert_eq!(ranki_storage::proto_varint(&fields, 1), Some(7));
        assert_eq!(
            ranki_storage::proto_string(&fields, 4).as_deref(),
            Some("Instructions")
        );
    }

    #[test]
    fn normal_deck_kind_keeps_metadata_and_explicit_zero_limits() {
        let deck = json!({
            "conf": 2,
            "desc": "Instructions",
            "md": true,
            "extendNew": 3,
            "extendRev": 4,
            "reviewLimit": 5,
            "newLimit": 0,
            "reviewLimitToday": {"limit": 7, "today": 9},
            "desiredRetention": 85,
        });
        let kind = super::deck_kind_blob(&deck);
        let outer =
            ranki_storage::protobuf_fields(&kind).expect("kind container");
        let normal = ranki_storage::proto_bytes(&outer, 1).expect("normal");
        let fields =
            ranki_storage::protobuf_fields(normal).expect("normal body");
        assert_eq!(ranki_storage::proto_varint(&fields, 1), Some(2));
        assert_eq!(ranki_storage::proto_varint(&fields, 2), Some(3));
        assert_eq!(ranki_storage::proto_varint(&fields, 3), Some(4));
        assert_eq!(
            ranki_storage::proto_string(&fields, 4).as_deref(),
            Some("Instructions")
        );
        assert_eq!(ranki_storage::proto_varint(&fields, 5), Some(1));
        assert_eq!(ranki_storage::proto_varint(&fields, 6), Some(5));
        assert_eq!(ranki_storage::proto_varint(&fields, 7), Some(0));
        let today = ranki_storage::proto_bytes(&fields, 8).expect("today");
        let today_fields =
            ranki_storage::protobuf_fields(today).expect("today body");
        assert_eq!(ranki_storage::proto_varint(&today_fields, 1), Some(7));
        assert_eq!(ranki_storage::proto_varint(&today_fields, 2), Some(9));
        let retention =
            ranki_storage::proto_float(&fields, 10).unwrap_or_default();
        assert!((retention - 0.85).abs() < 1e-6, "{retention}");
    }

    #[test]
    fn filtered_deck_kind_keeps_terms_and_preview_delays() {
        let deck = json!({
            "dyn": 1,
            "resched": true,
            "terms": [{"search": "is:due", "limit": 25, "order": 4}],
            "delays": [1.0, 10.0],
            "previewDelay": 120,
            "previewAgainSecs": 60,
            "previewHardSecs": 120,
            "previewGoodSecs": 240,
        });
        let kind = super::deck_kind_blob(&deck);
        let outer =
            ranki_storage::protobuf_fields(&kind).expect("kind container");
        assert!(ranki_storage::proto_bytes(&outer, 2).is_some());
        let filtered = ranki_storage::proto_bytes(&outer, 2).expect("filtered");
        let fields =
            ranki_storage::protobuf_fields(filtered).expect("filtered body");
        assert_eq!(ranki_storage::proto_varint(&fields, 1), Some(1));
        let term_bytes = ranki_storage::proto_bytes(&fields, 2).expect("term");
        let term =
            ranki_storage::protobuf_fields(term_bytes).expect("term body");
        assert_eq!(
            ranki_storage::proto_string(&term, 1).as_deref(),
            Some("is:due")
        );
        assert_eq!(ranki_storage::proto_varint(&term, 2), Some(25));
        assert_eq!(ranki_storage::proto_varint(&term, 3), Some(4));
        assert_eq!(ranki_storage::proto_floats(&fields, 3), vec![1.0, 10.0]);
        assert_eq!(ranki_storage::proto_varint(&fields, 4), Some(120));
        assert_eq!(ranki_storage::proto_varint(&fields, 7), Some(60));
        assert_eq!(ranki_storage::proto_varint(&fields, 5), Some(120));
        assert_eq!(ranki_storage::proto_varint(&fields, 6), Some(240));
    }
}
