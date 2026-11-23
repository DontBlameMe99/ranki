//! A small, parameterized subset of Anki's search syntax.

use std::mem;

use ranki_model::Deck;
use ranki_model::Id;
use ranki_model::NoteModel;
use ranki_model::StoreError;
use rusqlite::types::Value as SqlValue;
pub struct SearchQuery {
    pub clause: String,
    pub params: Vec<SqlValue>,
}

pub fn build(
    tokens: &[String],
    decks: &[Deck],
    models: &[NoteModel],
) -> Result<SearchQuery, StoreError> {
    let mut conditions = Vec::new();
    let mut params = Vec::new();
    for token in tokens {
        let condition = match token.split_once(':') {
            Some(("deck", value)) => deck_condition(value, decks, &mut params),
            Some(("tag", value)) => tag_condition(value, &mut params),
            Some(("note", value)) => {
                model_condition(value, models, &mut params)
            }
            Some(("is", value)) => state_condition(value)?,
            _ => {
                params.push(SqlValue::Text(like_pattern(token, false)));
                "n.flds LIKE ? ESCAPE '\\'".into()
            }
        };
        conditions.push(condition);
    }
    if conditions.is_empty() {
        conditions.push("1 = 1".into());
    }
    Ok(SearchQuery {
        clause: conditions.join(" AND "),
        params,
    })
}

/// Whether a token set needs a deck or note-type lookup to build its clause.
///
/// Deck and note-type filters resolve names against the collection, and that
/// lookup reads and parses the whole collection row. Plain field text and
/// `is:`/`tag:` filters never need it.
pub fn needs_decks(tokens: &[String]) -> bool {
    tokens
        .iter()
        .any(|token| matches!(token.split_once(':'), Some(("deck", _))))
}

pub fn needs_models(tokens: &[String]) -> bool {
    tokens
        .iter()
        .any(|token| matches!(token.split_once(':'), Some(("note", _))))
}

pub fn tokenize(query: &str) -> Result<Vec<String>, StoreError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for character in query.chars() {
        if escaped {
            current.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '"' {
            quoted = !quoted;
        } else if character.is_whitespace() && !quoted {
            flush_token(&mut tokens, &mut current);
        } else {
            current.push(character);
        }
    }
    if quoted {
        return Err(StoreError::InvalidData(
            "unterminated quote in search".into(),
        ));
    }
    if escaped {
        current.push('\\');
    }
    flush_token(&mut tokens, &mut current);
    Ok(tokens)
}

fn flush_token(tokens: &mut Vec<String>, current: &mut String) {
    if !current.is_empty() {
        tokens.push(mem::take(current));
    }
}
fn tag_condition(value: &str, params: &mut Vec<SqlValue>) -> String {
    params.push(SqlValue::Text(like_pattern(value, true)));
    if value.contains('*') {
        return "(' ' || n.tags || ' ') LIKE ? ESCAPE '\\'".into();
    }
    params.push(SqlValue::Text(like_pattern(&format!(" {value}::"), false)));
    "((' ' || n.tags || ' ') LIKE ? ESCAPE '\\' OR \
     (' ' || n.tags || ' ') LIKE ? ESCAPE '\\')"
        .into()
}

fn deck_matches(name: &str, pattern: &str) -> bool {
    if wildcard_match(name, pattern) {
        return true;
    }
    let prefix = format!("{}::", pattern.to_lowercase());
    name.to_lowercase().starts_with(&prefix)
}

fn deck_condition(
    value: &str,
    decks: &[Deck],
    params: &mut Vec<SqlValue>,
) -> String {
    let mut ids = Vec::new();
    for deck in decks {
        if deck_matches(&deck.name, value) {
            ids.extend(subtree_ids(deck, decks));
        }
    }
    ids.sort_unstable();
    ids.dedup();
    id_condition("c.did", &ids, params)
}

fn subtree_ids(deck: &Deck, decks: &[Deck]) -> Vec<Id> {
    let prefix = format!("{}::", deck.name);
    decks
        .iter()
        .filter(|candidate| {
            candidate.id == deck.id || candidate.name.starts_with(&prefix)
        })
        .map(|candidate| candidate.id)
        .collect()
}

fn model_condition(
    value: &str,
    models: &[NoteModel],
    params: &mut Vec<SqlValue>,
) -> String {
    let ids: Vec<Id> = models
        .iter()
        .filter(|model| wildcard_match(&model.name, value))
        .map(|model| model.id)
        .collect();
    id_condition("n.mid", &ids, params)
}

fn id_condition(
    column: &str,
    ids: &[Id],
    params: &mut Vec<SqlValue>,
) -> String {
    if ids.is_empty() {
        return "0".into();
    }
    let placeholders = vec!["?"; ids.len()].join(", ");
    params.extend(ids.iter().map(|id| SqlValue::Integer(*id)));
    format!("{column} IN ({placeholders})")
}

fn state_condition(value: &str) -> Result<String, StoreError> {
    let condition = match value.to_ascii_lowercase().as_str() {
        "new" => "c.queue = 0",
        "learn" | "learning" | "relearn" => "c.queue IN (1, 3)",
        "review" => "c.queue = 2",
        "suspended" => "c.queue = -1",
        "buried" => "c.queue = -2",
        _ => {
            return Err(StoreError::InvalidData(format!(
                "unsupported is: filter: {value}"
            )));
        }
    };
    Ok(condition.into())
}

fn like_pattern(value: &str, wrap: bool) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
        .replace('*', "%");
    if wrap {
        format!("% {escaped} %")
    } else {
        format!("%{escaped}%")
    }
}

fn wildcard_match(text: &str, pattern: &str) -> bool {
    let lowercase_text = text.to_lowercase();
    let lowercase_pattern = pattern.to_lowercase();
    let parts: Vec<&str> = lowercase_pattern.split('*').collect();
    let Some(first) = parts.first() else {
        return false;
    };
    if parts.len() == 1 {
        return lowercase_text == *first;
    }
    if !lowercase_text.starts_with(first) {
        return false;
    }
    let mut position = first.len();
    for (index, part) in parts.iter().enumerate().skip(1) {
        if index.saturating_add(1) == parts.len() {
            return lowercase_text
                .get(position..)
                .is_some_and(|tail| tail.ends_with(part));
        }
        let Some(found) = lowercase_text
            .get(position..)
            .and_then(|tail| tail.find(part))
        else {
            return false;
        };
        position = position.saturating_add(found).saturating_add(part.len());
    }
    true
}
