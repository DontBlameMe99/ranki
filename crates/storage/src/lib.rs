//! Anki-compatible SQLite storage for Ranki.
//!
//! Collections use Anki's `collection.anki2` tables, so files written here can
//! be opened by Anki. Existing collections are read as they are; their schema
//! and collection JSON are never rewritten.
//!
//! # Search syntax
//!
//! [`Collection::find_cards`] accepts a subset of Anki's search bar:
//!
//! - `deck:Name` matches a deck and all of its subdecks.
//! - `tag:Name` matches a note tag.
//! - `note:Name` matches a note model name.
//! - `is:new`, `is:learn`, `is:review`, `is:suspended`, `is:buried`.
//! - Plain words match note fields. `*` matches any number of characters.
//! - Quoted text keeps spaces inside one term; terms are combined with AND.
//!
//! All values are bound as SQL parameters.

mod anki;
mod records;
mod schema;
mod search;

use std::cell::OnceCell;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt::Display;
use std::path::Path;
use std::path::PathBuf;
use std::slice;
use std::str;

pub use ranki_model::StoreError;
pub use ranki_model::stock_models;

pub use anki::decode_deck_name;
pub use anki::encode_deck_name;
pub use anki::normal_deck_kind;

pub use records::models_from_tables_with_prefix;
pub use records::models_to_json;

use ranki_model::Card;
use ranki_model::DEFAULT_DECK_ID;
use ranki_model::Deck;
use ranki_model::DeckStats;
use ranki_model::DeckTotals;
use ranki_model::Id;
use ranki_model::NewNote;
use ranki_model::NoteModel;
use ranki_model::RevLog;
use ranki_model::ReviewDecision;
use ranki_model::StockModelKind;
use ranki_model::Store;
use rusqlite::Connection;
use rusqlite::OpenFlags;
use rusqlite::OptionalExtension;
use rusqlite::Transaction;
use rusqlite::params;
use rusqlite::params_from_iter;
use rusqlite::types::Value as SqlValue;
use serde_json::Map;
use serde_json::Value as Json;
use serde_json::json;
pub const SEARCH_SYNTAX: &str = "deck:, tag:, note:, is:new/learn/review/suspended/buried, plain field text, * wildcard";

/// An Anki collection backed by a SQLite file.
pub struct Collection {
    conn: Connection,
    path: PathBuf,
    created: OnceCell<i64>,
}

impl Collection {
    /// Creates a new Anki collection at `path`.
    ///
    /// # Errors
    ///
    /// Returns an error when the file already exists or SQLite cannot create
    /// the schema and the default collection row.
    pub fn create(path: &Path) -> Result<Self, StoreError> {
        if path.exists() {
            return Err(StoreError::InvalidData(format!(
                "collection already exists: {}",
                path.display()
            )));
        }
        let mut conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )
        .map_err(backend)?;
        register_collations(&conn)?;
        let transaction = conn.transaction().map_err(backend)?;
        for statement in schema::CREATE_STATEMENTS {
            transaction.execute(statement, []).map_err(backend)?;
        }
        let now_secs = anki::now_secs()?;
        let col = schema::default_col(now_secs, anki::now_ms()?);
        let text = |key: &str| -> Result<String, StoreError> {
            let value =
                col.get(key).ok_or_else(|| invalid("missing col field"))?;
            serde_json::to_string(value)
                .map_err(|error| invalid(error.to_string()))
        };
        transaction
            .execute(
                "INSERT INTO col (id, crt, mod, scm, ver, dty, usn, ls, conf, models, decks, dconf, \
                 tags) VALUES (1, ?1, ?2, ?3, ?4, 0, 0, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    col.get("crt").and_then(Json::as_i64).unwrap_or(now_secs),
                    col.get("mod").and_then(Json::as_i64).unwrap_or_default(),
                    col.get("scm").and_then(Json::as_i64).unwrap_or_default(),
                    col.get("ver").and_then(Json::as_i64).unwrap_or_default(),
                    col.get("ls").and_then(Json::as_i64).unwrap_or_default(),
                    text("conf")?,
                    text("models")?,
                    text("decks")?,
                    text("dconf")?,
                    text("tags")?,
                ],
            )
            .map_err(backend)?;
        transaction.commit().map_err(backend)?;
        Self::open(path)
    }

    /// Opens an existing collection for reading and writing.
    ///
    /// # Errors
    ///
    /// Returns an error when the file is missing, locked, or has no `col`
    /// table.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE,
        )
        .map_err(backend)?;
        register_collations(&conn)?;
        if !records::table_exists(&conn, "col")? {
            return Err(invalid("not an Anki collection"));
        }
        Ok(Self {
            conn,
            path: path.to_path_buf(),
            created: OnceCell::new(),
        })
    }

    /// Returns the collection's file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Installs a stock note type and returns the stored model.
    ///
    /// The existing model is returned when the collection already has that
    /// stock type. Schema-18 collections that keep note types in normalized
    /// tables get rows there instead of `col.models`.
    fn install_model(
        &mut self,
        kind: StockModelKind,
    ) -> Result<NoteModel, StoreError> {
        let existing = self.model_list()?;
        if let Some(model) = existing
            .iter()
            .find(|model| StockModelKind::of(model) == Some(kind))
        {
            return Ok(model.clone());
        }
        let mut model = kind.definition();
        model.id = fresh_model_id(&self.conn, &existing)?;
        let col = records::read_col(&self.conn)?;
        let stamp = col_stamp(&col)?;
        let tables = models_in_tables(&self.conn, &col)?;
        let models = if tables {
            None
        } else {
            Some(models_json_installing(&col.models, &model)?)
        };
        let transaction = self.conn.transaction().map_err(backend)?;
        if let Some(json) = models.as_deref() {
            transaction
                .execute(
"UPDATE col SET mod = ?1, scm = ?2, models = ?3 WHERE id = 1",
params![stamp.modified, stamp.scm, json],
                )
                .map_err(backend)?;
        } else {
            transaction
                .execute(
                    "UPDATE col SET mod = ?1, scm = ?2 WHERE id = 1",
                    params![stamp.modified, stamp.scm],
                )
                .map_err(backend)?;
            insert_model_tables(&transaction, &model, &stamp)?;
        }
        transaction.commit().map_err(backend)?;
        Ok(model)
    }

    /// Returns the collection creation time in Unix seconds.
    ///
    /// # Errors
    ///
    /// Returns an error when the database cannot be read.
    pub fn collection_created(&self) -> Result<i64, StoreError> {
        if let Some(created) = self.created.get() {
            return Ok(*created);
        }
        let created = self
            .conn
            .query_row("SELECT crt FROM col WHERE id = 1", [], |row| row.get(0))
            .map_err(backend)?;
        let _ = self.created.set(created);
        Ok(created)
    }

    /// Checkpoints the write-ahead log, if the collection uses one.
    ///
    /// # Errors
    ///
    /// Returns an error when SQLite cannot report or checkpoint the journal.
    pub fn checkpoint(&self) -> Result<(), StoreError> {
        let mode: String = self
            .conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .map_err(backend)?;
        if !mode.eq_ignore_ascii_case("wal") {
            return Ok(());
        }
        let busy: i64 = self
            .conn
            .pragma_query_value(None, "wal_checkpoint(TRUNCATE)", |row| {
                row.get(0)
            })
            .map_err(backend)?;
        if busy != 0 {
            return Err(invalid("checkpoint blocked by another writer"));
        }
        Ok(())
    }

    /// Checkpoints and closes the collection.
    ///
    /// # Errors
    ///
    /// Returns an error when the checkpoint or the SQLite close fails.
    pub fn close(self) -> Result<(), StoreError> {
        self.checkpoint()?;
        let Self { conn, .. } = self;
        conn.close().map_err(|(_, error)| backend(error))
    }

    /// Returns a single review log entry by id.
    ///
    /// # Errors
    ///
    /// Returns an error when the database cannot be read.
    pub fn revlog(&self, id: Id) -> Result<Option<RevLog>, StoreError> {
        self.conn
            .query_row(
                "SELECT id, cid, usn, ease, ivl, lastIvl, factor, time, type FROM revlog WHERE id = \
                 ?1",
                [id],
                records::revlog_from_row,
            )
            .optional()
            .map_err(backend)
    }

    /// Persists several scheduled reviews and their log entries in one
    /// transaction.
    ///
    /// # Errors
    ///
    /// Returns an error when any card does not exist or the write fails. A
    /// failed batch writes nothing.
    pub fn apply_reviews(
        &mut self,
        decisions: &[ReviewDecision],
    ) -> Result<(), StoreError> {
        let transaction = self.conn.transaction().map_err(backend)?;
        let usn = update_col(&transaction, true)?;
        for decision in decisions {
            write_review(&transaction, usn, decision)?;
        }
        transaction.commit().map_err(backend)?;
        Ok(())
    }

    fn deck_list(&self) -> Result<Vec<Deck>, StoreError> {
        deck_list_from(&self.conn)
    }

    fn model_list(&self) -> Result<Vec<NoteModel>, StoreError> {
        let col = records::read_col(&self.conn)?;
        let models = records::models_from_json(&col.models)?;
        if models.is_empty() && records::table_exists(&self.conn, "notetypes")?
        {
            return records::models_from_tables(&self.conn);
        }
        Ok(models)
    }

    fn deck_subtree(&self, deck_id: Id) -> Result<Vec<Id>, StoreError> {
        Ok(deck_subtree_ids(&self.deck_list()?, deck_id))
    }

    fn deck_filter(
        &self,
        deck_id: Option<Id>,
        first_placeholder: i64,
    ) -> Result<DeckFilter, StoreError> {
        let Some(id) = deck_id else {
            return Ok(DeckFilter {
                clause: String::new(),
                params: Vec::new(),
                missing: false,
            });
        };
        let ids = self.deck_subtree(id)?;
        if ids.is_empty() {
            return Ok(DeckFilter {
                clause: String::new(),
                params: Vec::new(),
                missing: true,
            });
        }
        Ok(DeckFilter {
            clause: format!(
                "c.did IN ({})",
                placeholders(first_placeholder, ids.len())
            ),
            params: ids.into_iter().map(SqlValue::Integer).collect(),
            missing: false,
        })
    }

    fn ensure_due_index(&self) -> Result<(), StoreError> {
        self.conn
            .execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_ranki_cards_queue_due ON cards (queue, due, id)",
            )
            .map_err(backend)
    }
}

impl Store for Collection {
    fn decks(&self) -> Result<Vec<Deck>, StoreError> {
        let decks = self.deck_list()?;
        if decks.len() <= 1 {
            return Ok(decks);
        }
        let Some(default) =
            decks.iter().find(|deck| deck.id == DEFAULT_DECK_ID)
        else {
            return Ok(decks);
        };
        let child_prefix = format!("{}::", default.name);
        if decks
            .iter()
            .any(|deck| deck.name.starts_with(&child_prefix))
        {
            return Ok(decks);
        }
        let occupied = self
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM cards WHERE did = ?1)",
                [DEFAULT_DECK_ID],
                |row| row.get::<_, bool>(0),
            )
            .map_err(backend)?;
        if occupied {
            return Ok(decks);
        }
        Ok(decks
            .into_iter()
            .filter(|deck| deck.id != DEFAULT_DECK_ID)
            .collect())
    }

    fn create_deck(&mut self, name: &str) -> Result<Deck, StoreError> {
        let deck_name = validate_deck_name(name)?;
        let decks = self.deck_list()?;
        if decks.iter().any(|deck| deck.name == deck_name) {
            return Err(invalid(format!("deck already exists: {deck_name}")));
        }
        let col = records::read_col(&self.conn)?;
        let stamp = col_stamp(&col)?;
        let id = fresh_id(&self.conn, "notes")?;
        let modern = uses_deck_table(&self.conn, &col)?;
        if modern {
            self.conn
                .execute(
                    "INSERT INTO decks (id, name, mtime_secs, usn, common, kind) VALUES (?1, ?2, \
                     ?3, ?4, ?5, ?6)",
                    params![
                        id,
                        anki::encode_deck_name(deck_name),
                        stamp.now_secs,
                        stamp.usn,
                        Vec::<u8>::new(),
                        anki::normal_deck_kind(1),
                    ],
                )
                .map_err(backend)?;
            self.conn
                .execute(
                    "UPDATE col SET mod = ?1, scm = ?2 WHERE id = 1",
                    params![stamp.modified, stamp.scm],
                )
                .map_err(backend)?;
        } else {
            let mut value: Json = serde_json::from_str(&col.decks)
                .map_err(|error| invalid(error.to_string()))?;
            let object = value
                .as_object_mut()
                .ok_or_else(|| invalid("decks is not an object"))?;
            object.insert(
                id.to_string(),
                json!({"id": id, "name": deck_name, "usn": stamp.usn, "mod": stamp.now_secs, "conf": 1, "desc": "", "dyn": 0, "collapsed": false, "browserCollapsed": false, "lrnToday": [0, 0], "revToday": [0, 0], "newToday": [0, 0], "timeToday": [0, 0], "extendNew": 0, "extendRev": 0}),
            );
            let decks_json = serde_json::to_string(&value)
                .map_err(|error| invalid(error.to_string()))?;
            self.conn
                .execute(
"UPDATE col SET mod = ?1, scm = ?2, decks = ?3 WHERE id = 1",
                    params![stamp.modified, stamp.scm, decks_json],
                )
                .map_err(backend)?;
        }
        Ok(Deck {
            id,
            name: deck_name.into(),
            description: String::new(),
        })
    }

    fn remove_deck(&mut self, id: Id) -> Result<(), StoreError> {
        if id == DEFAULT_DECK_ID {
            return Err(invalid("the Default deck cannot be removed"));
        }
        let transaction = self.conn.transaction().map_err(backend)?;
        let decks = deck_list_from(&transaction)?;
        if !decks.iter().any(|deck| deck.id == id) {
            return Err(StoreError::NotFound(format!("deck {id}")));
        }
        let subtree = deck_subtree_ids(&decks, id);
        let (card_ids, note_ids) = cards_in_decks(&transaction, &subtree)?;
        delete_cards(&transaction, &card_ids)?;
        let deleted_notes = delete_orphan_notes(&transaction, &note_ids)?;
        reset_original_decks(&transaction, &subtree)?;

        let col = records::read_col(&transaction)?;
        let stamp = col_stamp(&col)?;
        let modern = uses_deck_table(&transaction, &col)?;
        let decks_json = if modern {
            None
        } else {
            Some(decks_json_without(&col.decks, &subtree)?)
        };
        write_col_metadata(&transaction, &stamp, decks_json.as_deref())?;
        if modern {
            delete_from_deck_table(&transaction, &subtree)?;
        }
        let graves = Graves {
            cards: card_ids,
            notes: deleted_notes,
            decks: subtree,
        };
        write_graves(&transaction, &stamp, &graves)?;
        transaction.commit().map_err(backend)
    }

    fn rename_deck(&mut self, id: Id, name: &str) -> Result<Deck, StoreError> {
        let new_name = validate_deck_name(name)?;
        let transaction = self.conn.transaction().map_err(backend)?;
        let decks = deck_list_from(&transaction)?;
        let changes = rename_plan(&decks, id, new_name)?;
        let description = decks
            .iter()
            .find(|deck| deck.id == id)
            .map_or(String::new(), |deck| deck.description.clone());
        let col = records::read_col(&transaction)?;
        let stamp = col_stamp(&col)?;
        let modern = uses_deck_table(&transaction, &col)?;
        let decks_json = if modern {
            None
        } else {
            Some(renamed_decks_json(&col.decks, &changes, &stamp)?)
        };
        write_col_metadata(&transaction, &stamp, decks_json.as_deref())?;
        if modern {
            rename_in_deck_table(&transaction, &changes, &stamp)?;
        }
        transaction.commit().map_err(backend)?;
        Ok(Deck {
            id,
            name: new_name.into(),
            description,
        })
    }

    fn models(&self) -> Result<Vec<NoteModel>, StoreError> {
        self.model_list()
    }

    fn ensure_model(
        &mut self,
        kind: StockModelKind,
    ) -> Result<NoteModel, StoreError> {
        self.install_model(kind)
    }

    fn find_cards(&self, query: &str) -> Result<Vec<Id>, StoreError> {
        let tokens = search::tokenize(query)?;
        let decks = if search::needs_decks(&tokens) {
            self.deck_list()?
        } else {
            Vec::new()
        };
        let models = if search::needs_models(&tokens) {
            self.model_list()?
        } else {
            Vec::new()
        };
        let search = search::build(&tokens, &decks, &models)?;
        let sql = format!(
            "SELECT c.id FROM cards c JOIN notes n ON n.id = c.nid WHERE {} ORDER BY c.id",
            search.clause
        );
        let mut statement = self.conn.prepare(&sql).map_err(backend)?;
        let rows = statement
            .query_map(params_from_iter(search.params), |row| {
                row.get::<_, Id>(0)
            })
            .map_err(backend)?;
        let mut ids = Vec::new();
        for row in rows {
            ids.push(row.map_err(backend)?);
        }
        Ok(ids)
    }

    #[expect(
        clippy::excessive_nesting,
        reason = "bounded queue queries stay together in one read path"
    )]
    fn due_cards(
        &self,
        deck_id: Option<Id>,
        limit: usize,
        now_secs: i64,
    ) -> Result<Vec<Card>, StoreError> {
        self.ensure_due_index()?;
        let limit_value =
            i64::try_from(limit).map_err(|_| invalid("limit too large"))?;
        if limit_value == 0 {
            return Ok(Vec::new());
        }
        let filter = self.deck_filter(deck_id, 3)?;
        if filter.missing {
            return Ok(Vec::new());
        }
        let today = anki::day_number(now_secs, self.collection_created()?)?;
        let deck_clause = if filter.clause.is_empty() {
            String::new()
        } else {
            format!("AND {}", filter.clause)
        };
        let mut cards = Vec::with_capacity(limit);
        for (queue, due) in
            [(1, now_secs), (3, today), (2, today), (0, i64::MAX)]
        {
            let sql = format!(
                "SELECT c.id, c.nid, c.did, c.ord, c.mod, c.usn, c.type, c.queue, c.due, c.ivl, \
                 c.factor, c.reps, c.lapses, c.left, c.odue, c.odid, c.flags, c.data FROM cards c \
                 WHERE c.queue = ?1 AND c.due <= ?2 {deck_clause} ORDER BY c.due, c.id LIMIT ?{}",
                filter.params.len().saturating_add(3)
            );
            let mut parameters =
                vec![SqlValue::Integer(queue), SqlValue::Integer(due)];
            parameters.extend(filter.params.iter().cloned());
            parameters.push(SqlValue::Integer(limit_value));
            let mut statement = self.conn.prepare(&sql).map_err(backend)?;
            let rows = statement
                .query_map(params_from_iter(parameters), records::card_from_row)
                .map_err(backend)?;
            for row in rows {
                cards.push(row.map_err(backend)?);
            }
            if cards.len() >= limit {
                break;
            }
        }
        cards.truncate(limit);
        Ok(cards)
    }

    fn card(&self, id: Id) -> Result<Option<Card>, StoreError> {
        self.conn
            .query_row(
                "SELECT id, nid, did, ord, mod, usn, type, queue, due, ivl, factor, reps, lapses, \
                 left, odue, odid, flags, data FROM cards WHERE id = ?1",
                [id],
                records::card_from_row,
            )
            .optional()
            .map_err(backend)
    }

    fn note(&self, id: Id) -> Result<Option<ranki_model::Note>, StoreError> {
        self.conn
            .query_row(
                "SELECT id, guid, mid, mod, usn, tags, flds, sfld, csum, flags, data FROM notes \
                 WHERE id = ?1",
                [id],
                records::note_from_row,
            )
            .optional()
            .map_err(backend)
    }

    fn add_note(&mut self, note: &NewNote) -> Result<Vec<Id>, StoreError> {
        let model = self
            .model_list()?
            .into_iter()
            .find(|model| model.id == note.model_id)
            .ok_or_else(|| {
                StoreError::NotFound(format!("model {}", note.model_id))
            })?;
        if note.fields.is_empty() || note.fields.len() != model.fields.len() {
            return Err(invalid("field count does not match model"));
        }
        if !self.deck_subtree(note.deck_id)?.contains(&note.deck_id) {
            return Err(StoreError::NotFound(format!("deck {}", note.deck_id)));
        }
        let id = fresh_id(&self.conn, "notes")?;
        let guid = anki::guid(id)?;
        let modified = anki::now_secs()?;
        let first = note.fields.first().map_or("", String::as_str);
        let sort_field =
            anki::sort_field(&note.fields, usize::from(model.sort_field_index));
        let checksum = anki::checksum(first);
        let transaction = self.conn.transaction().map_err(backend)?;
        let usn = update_col(&transaction, false)?;
        let next_position = next_position(&transaction)?;
        transaction
            .execute(
                "INSERT INTO notes (id, guid, mid, mod, usn, tags, flds, sfld, csum, flags, data) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, '')",
                params![
                    id,
                    guid,
                    note.model_id,
                    modified,
                    usn,
                    anki::encode_tags(&note.tags),
                    anki::encode_fields(&note.fields),
                    sort_field,
                    anki::csum_value(checksum),
                ],
            )
            .map_err(backend)?;
        let ordinals = model.card_ordinals(&note.fields);
        let mut cards: Vec<Id> = Vec::with_capacity(ordinals.len());
        for (offset, ordinal) in ordinals.into_iter().enumerate() {
            let card_id = fresh_id(&transaction, "cards")?;
            let due = next_position
                .saturating_add(i64::try_from(offset).unwrap_or_default());
            transaction
                .execute(
                    "INSERT INTO cards (id, nid, did, ord, mod, usn, type, queue, due, ivl, factor, \
                     reps, lapses, left, odue, odid, flags, data) VALUES (?1, ?2, ?3, ?4, ?5, ?6, \
                     0, 0, ?7, 0, 0, 0, 0, 0, 0, 0, 0, ?8)",
                    params![
                        card_id,
                        id,
                        note.deck_id,
                        ordinal,
                        modified,
                        usn,
                        due,
                        format!("{{\"pos\":{due}}}"),
                    ],
                )
                .map_err(backend)?;
            cards.push(card_id);
        }
        set_next_position(
            &transaction,
            next_position
                .saturating_add(i64::try_from(cards.len()).unwrap_or_default()),
        )?;
        transaction.commit().map_err(backend)?;
        Ok(cards)
    }

    fn apply_review(
        &mut self,
        decision: &ReviewDecision,
    ) -> Result<Id, StoreError> {
        let transaction = self.conn.transaction().map_err(backend)?;
        let current: Option<(i64, i32)> = transaction
            .query_row(
                "SELECT mod, usn FROM cards WHERE id = ?1",
                [decision.card.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(backend)?;
        if current != Some((decision.expected_modified, decision.expected_usn))
        {
            return Err(invalid("card changed since it was loaded"));
        }
        let usn = update_col(&transaction, true)?;
        let log_id = write_review(&transaction, usn, decision)?;
        transaction.commit().map_err(backend)?;
        Ok(log_id)
    }

    fn undo_review(
        &mut self,
        before: &Card,
        revlog_id: Id,
    ) -> Result<bool, StoreError> {
        let transaction = self.conn.transaction().map_err(backend)?;
        let logged = transaction
            .query_row(
                "SELECT 1 FROM revlog WHERE id = ?1 AND cid = ?2 AND id = \
                 (SELECT MAX(id) FROM revlog WHERE cid = ?2)",
                params![revlog_id, before.id],
                |_| Ok(()),
            )
            .optional()
            .map_err(backend)?
            .is_some();
        if !logged {
            return Ok(false);
        }
        let usn = update_col(&transaction, true)?;
        transaction
            .execute(
                "DELETE FROM revlog WHERE id = ?1 AND cid = ?2",
                params![revlog_id, before.id],
            )
            .map_err(backend)?;
        let updated = transaction
            .execute(
                "UPDATE cards SET did = ?1, mod = ?2, usn = ?3, type = ?4, queue = ?5, due = ?6, \
                 ivl = ?7, factor = ?8, reps = ?9, lapses = ?10, left = ?11, odue = ?12, odid = \
                 ?13, flags = ?14, data = ?15 WHERE id = ?16",
                params![
                    before.deck_id,
                    before.modified,
                    usn,
                    before.card_type.code(),
                    before.queue.code(),
                    before.due,
                    before.interval,
                    before.factor,
                    before.reps,
                    before.lapses,
                    before.left,
                    before.original_due,
                    before.original_deck_id,
                    before.flags,
                    before.data,
                    before.id,
                ],
            )
            .map_err(backend)?;
        if updated == 0 {
            return Err(StoreError::NotFound(format!("card {}", before.id)));
        }
        transaction.commit().map_err(backend)?;
        Ok(true)
    }

    fn stats(
        &self,
        deck_id: Option<Id>,
        now_secs: i64,
    ) -> Result<DeckStats, StoreError> {
        let filter = self.deck_filter(deck_id, 3)?;
        if filter.missing {
            return Ok(DeckStats::default());
        }
        let where_clause = if filter.clause.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", filter.clause)
        };
        let today = anki::day_number(now_secs, self.collection_created()?)?;
        let sql = format!(
            "SELECT SUM(CASE WHEN c.queue = 0 THEN 1 ELSE 0 END), \
             SUM(CASE WHEN (c.queue = 1 AND c.due <= ?1) OR (c.queue = 3 AND c.due <= ?2) THEN 1 ELSE 0 END), \
             SUM(CASE WHEN c.queue = 2 AND c.due <= ?2 THEN 1 ELSE 0 END), COUNT(*) FROM cards c {where_clause}"
        );
        let mut parameters =
            vec![SqlValue::Integer(now_secs), SqlValue::Integer(today)];
        parameters.extend(filter.params);
        let mut statement = self.conn.prepare(&sql).map_err(backend)?;
        let counts: Counts = statement
            .query_row(params_from_iter(parameters), |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .map_err(backend)?;
        Ok(stats_from_counts(counts))
    }

    fn due_forecast(
        &self,
        days: usize,
        now_secs: i64,
    ) -> Result<Vec<usize>, StoreError> {
        let mut counts = vec![0_usize; days];
        if days == 0 {
            return Ok(counts);
        }
        let created = self.collection_created()?;
        let today = anki::day_number(now_secs, created)?;
        let last = i64::try_from(days)
            .map_err(|_| invalid("forecast too long"))?
            .saturating_sub(1);
        let horizon = today
            .checked_add(last)
            .ok_or_else(|| invalid("forecast overflow"))?;
        let end_today_secs = today
            .checked_add(1)
            .and_then(|day| day.checked_mul(86_400))
            .and_then(|offset| created.checked_add(offset))
            .ok_or_else(|| invalid("forecast overflow"))?;
        let sql = "SELECT CASE WHEN c.queue = 1 OR c.due < ?1 THEN ?1 ELSE c.due END AS due_day, \
                   COUNT(*) FROM cards c WHERE (c.queue = 1 AND c.due <= ?2) \
                   OR (c.queue IN (2, 3) AND c.due <= ?3) GROUP BY due_day";
        let mut statement = self.conn.prepare(sql).map_err(backend)?;
        let rows = statement
            .query_map(params![today, end_today_secs, horizon], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(backend)?;
        for row in rows {
            let (day, count) = row.map_err(backend)?;
            let offset = usize::try_from(day.saturating_sub(today))
                .unwrap_or(usize::MAX);
            record_forecast_count(&mut counts, offset, count);
        }
        Ok(counts)
    }

    fn deck_stats(&self, now_secs: i64) -> Result<DeckTotals, StoreError> {
        let decks = self.deck_list()?;
        let today = anki::day_number(now_secs, self.collection_created()?)?;
        let counts = deck_card_counts(&self.conn, now_secs, today)?;
        let ids_by_name = decks
            .iter()
            .map(|deck| (deck.name.as_str(), deck.id))
            .collect::<HashMap<_, _>>();
        let mut totals: HashMap<Id, DeckStats> = HashMap::new();
        for deck in &decks {
            let subdeck = counts.get(&deck.id).copied().unwrap_or_default();
            let ids = deck_and_ancestor_ids(deck, &ids_by_name);
            add_deck_totals(&mut totals, &ids, subdeck);
        }
        Ok(decks
            .iter()
            .map(|deck| {
                (deck.id, totals.get(&deck.id).copied().unwrap_or_default())
            })
            .collect())
    }
    fn review_history(&self) -> Result<Vec<RevLog>, StoreError> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT id, cid, usn, ease, ivl, lastIvl, factor, time, type \
                 FROM revlog ORDER BY cid, id",
            )
            .map_err(backend)?;
        let rows = statement
            .query_map([], records::revlog_from_row)
            .map_err(backend)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(backend)
    }

    fn collection_created(&self) -> Result<i64, StoreError> {
        Self::collection_created(self)
    }

    fn collection_modified(&self) -> Result<i64, StoreError> {
        Ok(records::read_col(&self.conn)?.modified)
    }
}

/// Registers the case-insensitive collation used by schema-18 table indexes.
///
/// # Errors
///
/// Returns an error when SQLite cannot install the collation.
pub fn register_collations(conn: &Connection) -> Result<(), StoreError> {
    conn.create_collation("unicase", |left, right| {
        left.to_lowercase().cmp(&right.to_lowercase())
    })
    .map_err(backend)
}

pub(crate) fn backend(error: impl Display) -> StoreError {
    StoreError::Backend(error.to_string())
}

pub(crate) fn invalid(message: impl Into<String>) -> StoreError {
    StoreError::InvalidData(message.into())
}

struct DeckFilter {
    clause: String,
    params: Vec<SqlValue>,
    missing: bool,
}

type Counts = (Option<i64>, Option<i64>, Option<i64>, i64);

type DeckRenames = Vec<(Id, String)>;
type CardsAndNotes = (Vec<Id>, Vec<Id>);

struct ColStamp {
    usn: i32,
    modified: i64,
    scm: i64,
    now_secs: i64,
}

struct Graves {
    cards: Vec<Id>,
    notes: Vec<Id>,
    decks: Vec<Id>,
}

fn fresh_id(conn: &Connection, table: &str) -> Result<Id, StoreError> {
    let highest: Option<Id> = conn
        .query_row(&format!("SELECT MAX(id) FROM {table}"), [], |row| {
            row.get(0)
        })
        .map_err(backend)?;
    let mut id = anki::next_id()?;
    if highest.is_some_and(|maximum| id <= maximum) {
        id = highest.unwrap_or(id).saturating_add(1);
    }
    loop {
        let exists: bool = conn
            .query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE id = ?1)"),
                [id],
                |row| row.get(0),
            )
            .map_err(backend)?;
        if !exists {
            return Ok(id);
        }
        id = id.saturating_add(1);
    }
}

fn fresh_revlog_id(conn: &Connection, preferred: Id) -> Result<Id, StoreError> {
    let highest: Option<Id> = conn
        .query_row("SELECT MAX(id) FROM revlog", [], |row| row.get(0))
        .map_err(backend)?;
    Ok(highest.map_or(preferred, |maximum| {
        preferred.max(maximum.saturating_add(1))
    }))
}

fn fresh_model_id(
    conn: &Connection,
    existing: &[NoteModel],
) -> Result<Id, StoreError> {
    let mut id = fresh_id(conn, "notes")?;
    while existing.iter().any(|entry| entry.id == id) {
        id = id.saturating_add(1);
    }
    Ok(id)
}

/// Adds one model to `col.models` JSON and returns the new document.
fn models_json_installing(
    json: &str,
    model: &NoteModel,
) -> Result<String, StoreError> {
    let mut object: Map<String, Json> =
        serde_json::from_str(json).unwrap_or_default();
    if let Some(entry) = models_to_json(slice::from_ref(model)).as_object() {
        object.extend(entry.clone());
    }
    serde_json::to_string(&Json::Object(object))
        .map_err(|error| invalid(error.to_string()))
}

/// True when the collection keeps its note types in schema-18 tables.
fn models_in_tables(
    conn: &Connection,
    col: &records::ColData,
) -> Result<bool, StoreError> {
    if !records::table_exists(conn, "notetypes")? {
        return Ok(false);
    }
    Ok(records::models_from_json(&col.models)?.is_empty())
}

fn column_exists(
    conn: &Connection,
    table: &str,
    column: &str,
) -> Result<bool, StoreError> {
    let mut statement = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(backend)?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(backend)?;
    for row in rows {
        if row.map_err(backend)? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Writes one note type into the schema-18 normalized tables.
fn insert_model_tables(
    conn: &Connection,
    model: &NoteModel,
    stamp: &ColStamp,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO notetypes (id, name, mtime_secs, usn, config) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            model.id,
            model.name,
            stamp.now_secs,
            stamp.usn,
            anki::notetype_config(
                model.kind.code(),
                model.sort_field_index,
                model.original_stock_kind,
                &model.css,
            ),
        ],
    )
    .map_err(backend)?;
    for (index, name) in model.fields.iter().enumerate() {
        let ord = i64::try_from(index).unwrap_or_default();
        let tag = model.field_tags.get(index).copied().flatten();
        conn.execute(
            "INSERT INTO fields (ntid, ord, name, config) VALUES (?1, ?2, ?3, ?4)",
            params![model.id, ord, name, anki::field_config(tag)],
        )
        .map_err(backend)?;
    }
    let stamped = column_exists(conn, "templates", "mtime_secs")?;
    for template in &model.templates {
        let ord = i64::from(template.ord);
        let config = anki::template_config(
            &template.question_format,
            &template.answer_format,
        );
        if stamped {
            conn.execute(
                "INSERT INTO templates (ntid, ord, name, mtime_secs, usn, config) VALUES (?1, \
                 ?2, ?3, ?4, ?5, ?6)",
                params![
                    model.id,
                    ord,
                    template.name,
                    stamp.now_secs,
                    stamp.usn,
                    config
                ],
            )
            .map_err(backend)?;
        } else {
            conn.execute(
                "INSERT INTO templates (ntid, ord, name, config) VALUES (?1, ?2, ?3, ?4)",
                params![model.id, ord, template.name, config],
            )
            .map_err(backend)?;
        }
    }
    Ok(())
}

fn placeholders(start: i64, count: usize) -> String {
    (0..count)
        .map(|index| {
            let number =
                start.saturating_add(i64::try_from(index).unwrap_or_default());
            format!("?{number}")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn deck_list_from(conn: &Connection) -> Result<Vec<Deck>, StoreError> {
    let col = records::read_col(conn)?;
    let decks = records::decks_from_json(&col.decks)?;
    if decks.is_empty() && records::table_exists(conn, "decks")? {
        return records::decks_from_tables(conn);
    }
    Ok(decks)
}

/// Counts cards due at `now_secs` per deck id in one grouped query.
fn deck_card_counts(
    conn: &Connection,
    now_secs: i64,
    today: i64,
) -> Result<HashMap<Id, DeckStats>, StoreError> {
    let mut statement = conn
        .prepare(
            "SELECT c.did, SUM(CASE WHEN c.queue = 0 THEN 1 ELSE 0 END), \
             SUM(CASE WHEN (c.queue = 1 AND c.due <= ?1) OR (c.queue = 3 AND c.due <= ?2) THEN 1 ELSE 0 END), \
             SUM(CASE WHEN c.queue = 2 AND c.due <= ?2 THEN 1 ELSE 0 END), COUNT(*) FROM cards c GROUP BY \
             c.did",
        )
        .map_err(backend)?;
    let rows = statement
        .query_map(params![now_secs, today], |row| {
            Ok((
                row.get::<_, Id>(0)?,
                (row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?),
            ))
        })
        .map_err(backend)?;
    let mut counts = HashMap::new();
    for row in rows {
        let (id, tally) = row.map_err(backend)?;
        let _ = counts.insert(id, stats_from_counts(tally));
    }
    Ok(counts)
}

/// Adds one deck's counts to the deck and each id in `ids`.
fn add_deck_totals(
    totals: &mut HashMap<Id, DeckStats>,
    ids: &[Id],
    subdeck: DeckStats,
) {
    for id in ids {
        add_deck_stats(totals.entry(*id).or_default(), subdeck);
    }
}

/// Ids of a deck followed by each ancestor deck that is named in the map.
fn deck_and_ancestor_ids(
    deck: &Deck,
    ids_by_name: &HashMap<&str, Id>,
) -> Vec<Id> {
    let mut ids = vec![deck.id];
    for (index, _) in deck.name.char_indices() {
        if !deck
            .name
            .get(index..)
            .is_some_and(|tail| tail.starts_with("::"))
        {
            continue;
        }
        let Some(&ancestor) = ids_by_name.get(&deck.name[..index]) else {
            continue;
        };
        ids.push(ancestor);
    }
    ids
}

const fn add_deck_stats(total: &mut DeckStats, counts: DeckStats) {
    total.new_cards = total.new_cards.saturating_add(counts.new_cards);
    total.learning_cards =
        total.learning_cards.saturating_add(counts.learning_cards);
    total.review_cards = total.review_cards.saturating_add(counts.review_cards);
    total.total_cards = total.total_cards.saturating_add(counts.total_cards);
}
fn record_forecast_count(counts: &mut [usize], offset: usize, count: i64) {
    if let Some(slot) = counts.get_mut(offset) {
        *slot = usize::try_from(count).unwrap_or(usize::MAX);
    }
}

fn stats_from_counts(counts: Counts) -> DeckStats {
    DeckStats {
        new_cards: usize::try_from(counts.0.unwrap_or_default())
            .unwrap_or_default(),
        learning_cards: usize::try_from(counts.1.unwrap_or_default())
            .unwrap_or_default(),
        review_cards: usize::try_from(counts.2.unwrap_or_default())
            .unwrap_or_default(),
        total_cards: usize::try_from(counts.3).unwrap_or_default(),
    }
}

fn deck_subtree_ids(decks: &[Deck], deck_id: Id) -> Vec<Id> {
    let Some(root) = decks.iter().find(|deck| deck.id == deck_id) else {
        return Vec::new();
    };
    let prefix = format!("{}::", root.name);
    decks
        .iter()
        .filter(|deck| deck.id == deck_id || deck.name.starts_with(&prefix))
        .map(|deck| deck.id)
        .collect()
}

fn validate_deck_name(name: &str) -> Result<&str, StoreError> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed.split("::").any(|part| part.trim().is_empty())
    {
        Err(invalid("invalid deck name"))
    } else {
        Ok(trimmed)
    }
}

fn uses_deck_table(
    conn: &Connection,
    col: &records::ColData,
) -> Result<bool, StoreError> {
    if !records::table_exists(conn, "decks")? {
        return Ok(false);
    }
    Ok(records::decks_from_json(&col.decks)?.is_empty())
}

/// Keeps a stamp at least one past `previous` so writes in the same
/// millisecond stay ordered. Saturation at [`i64::MAX`] keeps a collection
/// writable at the end of the range instead of panicking.
fn next_stamp(previous: i64, now: i64) -> i64 {
    now.max(previous.saturating_add(1))
}

fn col_stamp(col: &records::ColData) -> Result<ColStamp, StoreError> {
    stamp_from(&Stamps {
        modified: col.modified,
        scm: col.scm,
    })
}

fn stamp_from(stamps: &Stamps) -> Result<ColStamp, StoreError> {
    let now = anki::now_ms()?;
    Ok(ColStamp {
        usn: -1,
        modified: next_stamp(stamps.modified, now),
        scm: next_stamp(stamps.scm, now),
        now_secs: anki::now_secs()?,
    })
}

fn write_col_metadata(
    conn: &Connection,
    stamp: &ColStamp,
    decks_json: Option<&str>,
) -> Result<(), StoreError> {
    if let Some(decks) = decks_json {
        conn.execute(
            "UPDATE col SET mod = ?1, scm = ?2, decks = ?3 WHERE id = 1",
            params![stamp.modified, stamp.scm, decks],
        )
        .map_err(backend)?;
    } else {
        conn.execute(
            "UPDATE col SET mod = ?1, scm = ?2 WHERE id = 1",
            params![stamp.modified, stamp.scm],
        )
        .map_err(backend)?;
    }
    Ok(())
}

fn sql_params(ids: &[Id]) -> Vec<SqlValue> {
    ids.iter().copied().map(SqlValue::Integer).collect()
}

fn cards_in_decks(
    conn: &Connection,
    deck_ids: &[Id],
) -> Result<CardsAndNotes, StoreError> {
    if deck_ids.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let list = placeholders(1, deck_ids.len());
    let mut statement = conn
        .prepare(&format!("SELECT id, nid FROM cards WHERE did IN ({list})"))
        .map_err(backend)?;
    let rows = statement
        .query_map(params_from_iter(sql_params(deck_ids)), |row| {
            Ok((row.get::<_, Id>(0)?, row.get::<_, Id>(1)?))
        })
        .map_err(backend)?;
    let mut cards = Vec::new();
    let mut notes = Vec::new();
    for row in rows {
        let (card_id, note_id) = row.map_err(backend)?;
        cards.push(card_id);
        if !notes.contains(&note_id) {
            notes.push(note_id);
        }
    }
    Ok((cards, notes))
}

fn delete_cards(conn: &Connection, card_ids: &[Id]) -> Result<(), StoreError> {
    if card_ids.is_empty() {
        return Ok(());
    }
    let list = placeholders(1, card_ids.len());
    conn.execute(
        &format!("DELETE FROM revlog WHERE cid IN ({list})"),
        params_from_iter(sql_params(card_ids)),
    )
    .map_err(backend)?;
    conn.execute(
        &format!("DELETE FROM cards WHERE id IN ({list})"),
        params_from_iter(sql_params(card_ids)),
    )
    .map_err(backend)?;
    Ok(())
}

fn delete_orphan_notes(
    conn: &Connection,
    note_ids: &[Id],
) -> Result<Vec<Id>, StoreError> {
    let mut deleted = Vec::new();
    for note_id in note_ids {
        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM cards WHERE nid = ?1",
                [*note_id],
                |row| row.get(0),
            )
            .map_err(backend)?;
        if remaining == 0 {
            conn.execute("DELETE FROM notes WHERE id = ?1", [*note_id])
                .map_err(backend)?;
            deleted.push(*note_id);
        }
    }
    Ok(deleted)
}

fn reset_original_decks(
    conn: &Connection,
    deck_ids: &[Id],
) -> Result<(), StoreError> {
    if deck_ids.is_empty() {
        return Ok(());
    }
    let list = placeholders(1, deck_ids.len());
    conn.execute(
        &format!("UPDATE cards SET odid = 0 WHERE odid IN ({list})"),
        params_from_iter(sql_params(deck_ids)),
    )
    .map(|_| ())
    .map_err(backend)
}

fn delete_from_deck_table(
    conn: &Connection,
    deck_ids: &[Id],
) -> Result<(), StoreError> {
    if deck_ids.is_empty() {
        return Ok(());
    }
    let list = placeholders(1, deck_ids.len());
    conn.execute(
        &format!("DELETE FROM decks WHERE id IN ({list})"),
        params_from_iter(sql_params(deck_ids)),
    )
    .map(|_| ())
    .map_err(backend)
}

fn decks_json_without(json: &str, ids: &[Id]) -> Result<String, StoreError> {
    let mut value: Json = serde_json::from_str(json)
        .map_err(|error| invalid(error.to_string()))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| invalid("decks is not an object"))?;
    for id in ids {
        object.remove(&id.to_string());
    }
    serde_json::to_string(&value).map_err(|error| invalid(error.to_string()))
}

fn renamed_decks_json(
    json: &str,
    changes: &DeckRenames,
    stamp: &ColStamp,
) -> Result<String, StoreError> {
    let mut value: Json = serde_json::from_str(json)
        .map_err(|error| invalid(error.to_string()))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| invalid("decks is not an object"))?;
    for (deck_id, deck_name) in changes {
        let key = deck_id.to_string();
        let entry = object.get_mut(&key).ok_or_else(|| {
            StoreError::NotFound(format!(
                "deck {deck_id} missing from col.decks"
            ))
        })?;
        let fields = entry
            .as_object_mut()
            .ok_or_else(|| invalid("deck entry is not an object"))?;
        fields.insert("name".into(), json!(deck_name));
        fields.insert("usn".into(), json!(stamp.usn));
        fields.insert("mod".into(), json!(stamp.now_secs));
    }
    serde_json::to_string(&value).map_err(|error| invalid(error.to_string()))
}

fn rename_in_deck_table(
    conn: &Connection,
    changes: &DeckRenames,
    stamp: &ColStamp,
) -> Result<(), StoreError> {
    for (deck_id, deck_name) in changes {
        let updated = conn
            .execute(
                "UPDATE decks SET name = ?1, mtime_secs = ?2, usn = ?3 WHERE id = ?4",
                params![
                    anki::encode_deck_name(deck_name),
                    stamp.now_secs,
                    stamp.usn,
                    deck_id,
                ],
            )
            .map_err(backend)?;
        if updated == 0 {
            return Err(StoreError::NotFound(format!("deck {deck_id}")));
        }
    }
    Ok(())
}

fn rename_plan(
    decks: &[Deck],
    id: Id,
    new_name: &str,
) -> Result<DeckRenames, StoreError> {
    let root = decks
        .iter()
        .find(|deck| deck.id == id)
        .ok_or_else(|| StoreError::NotFound(format!("deck {id}")))?;
    let prefix = format!("{}::", root.name);
    let mut changes = Vec::with_capacity(decks.len());
    for deck in decks {
        if deck.id == id {
            changes.push((deck.id, new_name.to_owned()));
        } else if let Some(suffix) = deck.name.strip_prefix(&prefix) {
            changes.push((deck.id, format!("{new_name}::{suffix}")));
        }
    }
    let mut seen: HashSet<&str> = HashSet::with_capacity(decks.len());
    for deck in decks {
        let name = changes
            .iter()
            .find(|(changed_id, _)| *changed_id == deck.id)
            .map_or(deck.name.as_str(), |(_, name)| name.as_str());
        if !seen.insert(name) {
            return Err(invalid(format!("deck already exists: {name}")));
        }
    }
    Ok(changes)
}

fn write_graves(
    conn: &Connection,
    stamp: &ColStamp,
    graves: &Graves,
) -> Result<(), StoreError> {
    for (kind, ids) in [
        (0_i64, graves.cards.as_slice()),
        (1_i64, graves.notes.as_slice()),
        (2_i64, graves.decks.as_slice()),
    ] {
        for id in ids {
            conn.execute(
                "DELETE FROM graves WHERE oid = ?1 AND type = ?2",
                params![id, kind],
            )
            .map_err(backend)?;
            conn.execute(
                "INSERT INTO graves (oid, type, usn) VALUES (?1, ?2, ?3)",
                params![id, kind, stamp.usn],
            )
            .map_err(backend)?;
        }
    }
    Ok(())
}
fn update_col(conn: &Connection, bump_scm: bool) -> Result<i32, StoreError> {
    let stamps = read_stamps(conn)?;
    let stamp = stamp_from(&stamps)?;
    let scm = if bump_scm { stamp.scm } else { stamps.scm };
    conn.execute(
        "UPDATE col SET mod = ?1, scm = ?2 WHERE id = 1",
        params![stamp.modified, scm],
    )
    .map_err(backend)?;
    Ok(stamp.usn)
}

/// Collection stamps that every write bumps.
struct Stamps {
    modified: i64,
    scm: i64,
}

/// Reads the stamps without decoding the note types, decks, and configuration
/// JSON that the same row carries.
fn read_stamps(conn: &Connection) -> Result<Stamps, StoreError> {
    conn.query_row("SELECT mod, scm FROM col WHERE id = 1", [], |row| {
        Ok(Stamps {
            modified: row.get(0)?,
            scm: row.get(1)?,
        })
    })
    .map_err(backend)
}

/// Writes one scheduled card and its review log entry inside `transaction`.
fn write_review(
    transaction: &Transaction<'_>,
    usn: i32,
    decision: &ReviewDecision,
) -> Result<Id, StoreError> {
    let card = &decision.card;
    let updated = transaction
        .execute(
            "UPDATE cards SET did = ?1, mod = ?2, usn = ?3, type = ?4, queue = ?5, due = ?6, \
             ivl = ?7, factor = ?8, reps = ?9, lapses = ?10, left = ?11, odue = ?12, odid = \
             ?13, flags = ?14, data = ?15 WHERE id = ?16",
            params![
                card.deck_id,
                card.modified,
                usn,
                card.card_type.code(),
                card.queue.code(),
                card.due,
                card.interval,
                card.factor,
                card.reps,
                card.lapses,
                card.left,
                card.original_due,
                card.original_deck_id,
                card.flags,
                card.data,
                card.id,
            ],
        )
        .map_err(backend)?;
    if updated == 0 {
        return Err(StoreError::NotFound(format!("card {}", card.id)));
    }
    let log = &decision.revlog;
    let log_id = fresh_revlog_id(transaction, log.id)?;
    transaction
        .execute(
            "INSERT INTO revlog (id, cid, usn, ease, ivl, lastIvl, factor, time, type) VALUES \
             (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                log_id,
                card.id,
                usn,
                log.ease.code(),
                log.interval,
                log.last_interval,
                log.factor,
                log.time_ms,
                log.kind.code(),
            ],
        )
        .map_err(backend)?;
    Ok(log_id)
}

/// Reads the next new-card position.
///
/// Schema-18 collections keep configuration in the `config` table and leave
/// `col.conf` empty, so both paths are supported.
fn next_position(conn: &Connection) -> Result<i64, StoreError> {
    if records::table_exists(conn, "config")? {
        let stored: Option<SqlValue> = conn
            .query_row(
                "SELECT val FROM config WHERE key = 'nextPos'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(backend)?;
        let position = match stored {
            Some(SqlValue::Blob(bytes)) => parse_position(&bytes),
            Some(SqlValue::Text(text)) => text.parse().ok(),
            Some(SqlValue::Integer(value)) => Some(value),
            _ => None,
        };
        return Ok(position.unwrap_or(1));
    }
    let col = records::read_col(conn)?;
    let config = parse_conf(&col.conf)?;
    Ok(config.get("nextPos").and_then(Json::as_i64).unwrap_or(1))
}

fn set_next_position(
    transaction: &Transaction<'_>,
    position: i64,
) -> Result<(), StoreError> {
    if records::table_exists(transaction, "config")? {
        return transaction
            .execute(
                "INSERT INTO config (key, usn, mtime_secs, val) \
                 VALUES ('nextPos', -1, ?1, ?2) \
                 ON CONFLICT(key) DO UPDATE SET val = excluded.val, \
                 mtime_secs = excluded.mtime_secs, usn = excluded.usn",
                params![anki::now_secs()?, position.to_string().into_bytes()],
            )
            .map(|_| ())
            .map_err(backend);
    }
    let col = records::read_col(transaction)?;
    let mut config = parse_conf(&col.conf)?;
    let object = config
        .as_object_mut()
        .ok_or_else(|| invalid("conf is not an object"))?;
    object.insert("nextPos".into(), json!(position));
    let config_json = serde_json::to_string(&config)
        .map_err(|error| invalid(error.to_string()))?;
    transaction
        .execute("UPDATE col SET conf = ?1 WHERE id = 1", [config_json])
        .map(|_| ())
        .map_err(backend)
}

/// Parses `col.conf`, treating an empty blob as an empty object.
fn parse_conf(conf: &str) -> Result<Json, StoreError> {
    if conf.trim().is_empty() {
        return Ok(Json::Object(serde_json::Map::new()));
    }
    serde_json::from_str(conf).map_err(|error| invalid(error.to_string()))
}

/// Reads a decimal position stored as UTF-8 bytes in the config table.
fn parse_position(bytes: &[u8]) -> Option<i64> {
    str::from_utf8(bytes).ok()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use ranki_model::NewNote;
    use ranki_model::Rating;
    use ranki_model::RevLog;
    use ranki_model::RevLogKind;
    use ranki_model::ReviewDecision;
    use ranki_model::Store;

    use super::Collection;

    #[test]
    fn review_history_returns_logs_in_card_order() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("collection.anki2");
        let mut collection = Collection::create(&path).unwrap();
        let ids = collection
            .add_note(&NewNote {
                model_id: 1,
                deck_id: 1,
                fields: vec![String::from("front"), String::from("back")],
                tags: Vec::new(),
            })
            .unwrap();
        let card = collection.card(ids[0]).unwrap().unwrap();
        let log = RevLog {
            id: 1_700_000_000_000,
            card_id: card.id,
            usn: 0,
            ease: Rating::Good,
            interval: 1,
            last_interval: 0,
            factor: 2_500,
            time_ms: 100,
            kind: RevLogKind::Review,
        };
        collection
            .apply_review(&ReviewDecision {
                card: card.clone(),
                expected_modified: card.modified,
                expected_usn: card.usn,
                revlog: log.clone(),
            })
            .unwrap();
        let history = collection.review_history().unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].id, log.id);
        assert_eq!(history[0].card_id, log.card_id);
        assert_eq!(history[0].kind, RevLogKind::Review);
    }
}
