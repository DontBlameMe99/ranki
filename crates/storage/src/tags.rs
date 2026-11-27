//! Anki-style tag canonification and registration.

use std::collections::HashSet;

use ranki_model::StoreError;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::params;
use serde_json::Map;
use serde_json::Value as Json;
use serde_json::json;
use unicase::UniCase;
use unicode_normalization::UnicodeNormalization;

use crate::backend;
use crate::invalid;
use crate::records;

/// Canonicalizes note tags and registers any new names in the collection.
///
/// # Errors
///
/// Returns an error when the tag registry cannot be read or written.
pub fn canonify(
    tags: &[String],
    conn: &Connection,
    usn: i32,
) -> Result<Vec<String>, StoreError> {
    let mut registry = Registry::open(conn)?;
    let mut seen = HashSet::new();
    let mut result = Vec::new();

    for entry in tags {
        for part in split_tags(entry) {
            let normalized = normalize_name(part);
            let name = match registry.lookup(&normalized)? {
                Some(existing) => existing,
                None => registry.adjust_case(&normalized)?,
            };
            if seen.insert(UniCase::new(name.clone())) {
                registry.register(&name, usn)?;
                result.push(name);
            }
        }
    }

    let mut sorted: Vec<UniCase<String>> =
        result.into_iter().map(UniCase::new).collect();
    sorted.sort_unstable();
    let canonified: Vec<String> =
        sorted.into_iter().map(UniCase::into_inner).collect();
    registry.flush()?;
    Ok(canonified)
}

/// Returns the registered tags, including tags found only on notes.
///
/// # Errors
///
/// Returns an error when the tag registry or notes cannot be read.
pub fn all(conn: &Connection) -> Result<Vec<String>, StoreError> {
    let mut tags: HashSet<UniCase<String>> = HashSet::new();
    let registry = Registry::open(conn)?;
    for name in registry.names()? {
        tags.insert(UniCase::new(name));
    }
    let mut statement =
        conn.prepare("SELECT tags FROM notes").map_err(backend)?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(backend)?;
    for row in rows {
        let encoded = row.map_err(backend)?;
        for tag in split_tags(&encoded) {
            tags.insert(UniCase::new(tag.to_owned()));
        }
    }
    let mut sorted: Vec<UniCase<String>> = tags.into_iter().collect();
    sorted.sort_unstable();
    Ok(sorted.into_iter().map(UniCase::into_inner).collect())
}

fn split_tags(tags: &str) -> impl Iterator<Item = &str> {
    tags.split([' ', '\u{3000}']).filter(|tag| !tag.is_empty())
}

fn normalize_name(name: &str) -> String {
    name.split("::")
        .map(normalize_component)
        .collect::<Vec<_>>()
        .join("::")
}

fn normalize_component(component: &str) -> String {
    let stripped: String = component
        .chars()
        .filter(|character| !invalid_char(*character))
        .collect();
    let mut without_invalid: String = stripped.nfc().collect();
    let trimmed = without_invalid.trim();
    if trimmed.is_empty() {
        String::from("blank")
    } else if trimmed.len() != without_invalid.len() {
        trimmed.to_owned()
    } else {
        without_invalid.shrink_to_fit();
        without_invalid
    }
}

const fn invalid_char(character: char) -> bool {
    character.is_ascii_control() || character == ' ' || character == '\u{3000}'
}

fn parent_name(name: &str) -> Option<&str> {
    name.rsplit_once("::").map(|(parent, _)| parent)
}

fn adjusted_case(name: &str, existing: &str) -> String {
    let parent_count = existing.matches("::").count().saturating_add(1);
    let remainder = name
        .split("::")
        .skip(parent_count)
        .collect::<Vec<_>>()
        .join("::");
    if remainder.is_empty() {
        existing.to_owned()
    } else {
        format!("{existing}::{remainder}")
    }
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "the registry tracks its storage mode and pending JSON write separately"
)]
struct Registry<'a> {
    conn: &'a Connection,
    table: bool,
    col_tags: Option<Map<String, Json>>,
    dirty: bool,
}

impl<'a> Registry<'a> {
    fn open(conn: &'a Connection) -> Result<Self, StoreError> {
        if records::table_exists(conn, "tags")? {
            return Ok(Self {
                conn,
                table: true,
                col_tags: None,
                dirty: false,
            });
        }
        let raw = conn
            .query_row("SELECT tags FROM col WHERE id = 1", [], |row| {
                row.get::<_, String>(0)
            })
            .map_err(backend)?;
        let parsed: Json = serde_json::from_str(&raw)
            .map_err(|error| invalid(error.to_string()))?;
        let col_tags = parsed
            .as_object()
            .ok_or_else(|| invalid("collection tags must be an object"))?
            .clone();
        Ok(Self {
            conn,
            table: false,
            col_tags: Some(col_tags),
            dirty: false,
        })
    }

    fn lookup(&self, name: &str) -> Result<Option<String>, StoreError> {
        if self.table {
            return self
                .conn
                .query_row(
                    "SELECT tag FROM tags WHERE tag = ?1 COLLATE unicase",
                    [name],
                    |row| row.get(0),
                )
                .optional()
                .map_err(backend);
        }
        Ok(self
            .col_tags
            .as_ref()
            .and_then(|tags| {
                tags.keys().find(|key| {
                    UniCase::new(key.as_str()) == UniCase::new(name)
                })
            })
            .cloned())
    }

    fn adjust_case(&self, name: &str) -> Result<String, StoreError> {
        let mut parent = parent_name(name);
        while let Some(candidate) = parent {
            if let Some(existing) = self.lookup(candidate)? {
                return Ok(adjusted_case(name, &existing));
            }
            parent = parent_name(candidate);
        }
        Ok(name.to_owned())
    }

    fn register(&mut self, name: &str, usn: i32) -> Result<(), StoreError> {
        if self.table {
            self.conn
                .execute(
                    "INSERT OR IGNORE INTO tags (tag, usn, collapsed) VALUES (?1, ?2, 0)",
                    params![name, usn],
                )
                .map_err(backend)?;
        } else if self.lookup(name)?.is_none()
            && let Some(tags) = self.col_tags.as_mut()
        {
            tags.insert(name.to_owned(), json!(usn));
            self.dirty = true;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), StoreError> {
        if !self.dirty {
            return Ok(());
        }
        let Some(tags) = self.col_tags.as_ref() else {
            return Ok(());
        };
        let encoded = serde_json::to_string(tags)
            .map_err(|error| invalid(error.to_string()))?;
        self.conn
            .execute("UPDATE col SET tags = ?1 WHERE id = 1", [encoded])
            .map_err(backend)?;
        self.dirty = false;
        Ok(())
    }

    fn names(&self) -> Result<Vec<String>, StoreError> {
        if self.table {
            let mut statement =
                self.conn.prepare("SELECT tag FROM tags").map_err(backend)?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(backend)?;
            return rows.collect::<Result<Vec<_>, _>>().map_err(backend);
        }
        Ok(self
            .col_tags
            .as_ref()
            .map(|tags| tags.keys().cloned().collect())
            .unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_name;

    #[test]
    fn components_are_nfc_normalized_and_stripped() {
        assert_eq!(normalize_name("\u{fa47}"), "\u{6f22}");
        assert_eq!(normalize_name("  spaced\u{3000}out  "), "spacedout");
        assert_eq!(normalize_name("a::\u{0001}::b"), "a::blank::b");
        assert_eq!(normalize_name("e\u{0001}\u{301}"), "é");
    }
}
