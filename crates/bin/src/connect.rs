//! AnkiConnect-compatible JSON-RPC server.

use std::fmt;
use std::path::Path;

use ranki_core::FsrsScheduler;
use ranki_model::CardQueue;
use ranki_model::CardType;
use ranki_model::Deck;
use ranki_model::Id;
use ranki_model::Note;
use ranki_model::NoteModel;
use ranki_model::Rating;
use ranki_model::Store;
use ranki_storage::Collection;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value;
use serde_json::json;
use tiny_http::Header;
use tiny_http::Request;
use tiny_http::Response;
use tiny_http::Server;

use crate::CliError;
use crate::CliResult;
use crate::answer_card;
use crate::error::{ErrorCode, InterfaceError};
use crate::save_card;
const API_VERSION: u32 = 6;

#[derive(Debug)]
struct RpcError(InterfaceError);

impl From<InterfaceError> for RpcError {
    fn from(error: InterfaceError) -> Self {
        Self(error)
    }
}

impl From<ranki_model::StoreError> for RpcError {
    fn from(error: ranki_model::StoreError) -> Self {
        Self(error.into())
    }
}

impl From<CliError> for RpcError {
    fn from(error: CliError) -> Self {
        Self(error.into())
    }
}

impl From<String> for RpcError {
    fn from(error: String) -> Self {
        Self(InterfaceError::message(error))
    }
}

impl From<&str> for RpcError {
    fn from(error: &str) -> Self {
        Self(InterfaceError::message(error))
    }
}

impl fmt::Display for RpcError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl From<RpcError> for InterfaceError {
    fn from(error: RpcError) -> Self {
        error.0
    }
}

#[derive(Debug, Deserialize)]
struct RpcRequest {
    action: String,
    #[serde(default)]
    version: Option<u32>,
    #[serde(default)]
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct RpcResponse {
    result: Value,
    error: Option<String>,
    #[serde(rename = "errorCode", skip_serializing_if = "Option::is_none")]
    error_code: Option<&'static str>,
}

impl RpcResponse {
    const fn ok(result: Value) -> Self {
        Self {
            result,
            error: None,
            error_code: None,
        }
    }

    fn error(source: impl Into<InterfaceError>) -> Self {
        let mapped = source.into();
        let code = mapped.code();
        let (_, message) = mapped.into_parts();
        Self {
            result: Value::Null,
            error: Some(message),
            error_code: Some(code.as_str()),
        }
    }
}

pub fn serve(
    path: &Path,
    port: u16,
    scheduler: &FsrsScheduler,
) -> CliResult<()> {
    let address = ("127.0.0.1", port);
    let server = Server::http(address).map_err(|error| {
        CliError::Message(format!("cannot listen on 127.0.0.1:{port}: {error}"))
    })?;
    let mut store = Collection::open(path)?;
    for mut request in server.incoming_requests() {
        let response = read_and_dispatch(&mut store, &mut request, scheduler);
        let body = serde_json::to_string(&response)?;
        let header =
            Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
                .map_err(|()| {
                    CliError::Message(String::from("invalid HTTP header"))
                })?;
        request.respond(Response::from_string(body).with_header(header))?;
    }
    Ok(())
}

fn read_and_dispatch(
    store: &mut Collection,
    request: &mut Request,
    scheduler: &FsrsScheduler,
) -> RpcResponse {
    let mut body = Vec::new();
    if let Err(error) = request.as_reader().read_to_end(&mut body) {
        return RpcResponse::error(format!("cannot read request: {error}"));
    }
    handle_body(store, &body, scheduler)
}

fn handle_body(
    store: &mut Collection,
    body: &[u8],
    scheduler: &FsrsScheduler,
) -> RpcResponse {
    let request: RpcRequest = match serde_json::from_slice(body) {
        Ok(request) => request,
        Err(error) => {
            return RpcResponse::error(format!("invalid JSON: {error}"));
        }
    };
    if request.version.is_some_and(|version| version > API_VERSION) {
        return RpcResponse::error(format!(
            "unsupported protocol version: {}",
            request.version.unwrap_or_default()
        ));
    }
    let params = request.params.unwrap_or_else(|| json!({}));
    match dispatch(store, &request.action, &params, scheduler) {
        Ok(result) => RpcResponse::ok(result),
        Err(error) => RpcResponse::error(error),
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "dispatch shares one entry point for every action"
)]
fn dispatch(
    store: &mut Collection,
    action: &str,
    params: &Value,
    scheduler: &FsrsScheduler,
) -> Result<Value, RpcError> {
    match action {
        "version" => Ok(json!(API_VERSION)),
        "deckNames" => Ok(json!(names(
            store.decks()?.iter().map(|deck| deck.name.as_str())
        ))),
        "modelNames" => Ok(json!(names(
            store.models()?.iter().map(|model| model.name.as_str())
        ))),
        "modelFieldNames" => model_field_names(store, params),
        "addNote" => add_note(store, params),
        "findCards" => {
            Ok(json!(store.find_cards(param_str(params, "query")?)?))
        }
        "cardsInfo" => cards_info(store, params),
        "answerCards" => answer_cards(store, params, scheduler),
        "createDeck" => create_deck(store, params),
        "renameDeck" => rename_deck(store, params),
        "deleteDeck" => delete_deck(store, params),
        "suspend" => {
            let suspend = params
                .get("suspend")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            set_suspended(store, params, suspend)
        }
        "unsuspend" => {
            set_suspended(store, params, false)?;
            Ok(Value::Null)
        }
        "changeDeck" => change_deck(store, params),
        _ => Err(InterfaceError::new(
            ErrorCode::Unsupported,
            format!("unsupported action: {action}"),
        )
        .into()),
    }
}

fn names<'a>(values: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    values.collect()
}

fn member<'a>(value: &'a Value, key: &str) -> Result<&'a Value, RpcError> {
    Ok(value
        .get(key)
        .ok_or_else(|| format!("missing parameter: {key}"))?)
}

fn param_str<'a>(params: &'a Value, key: &str) -> Result<&'a str, RpcError> {
    Ok(member(params, key)?
        .as_str()
        .ok_or_else(|| format!("invalid parameter: {key}"))?)
}

fn param_ids(params: &Value, key: &str) -> Result<Vec<Id>, RpcError> {
    let values = member(params, key)?
        .as_array()
        .ok_or_else(|| format!("invalid parameter: {key}"))?;
    values
        .iter()
        .map(|value| {
            value.as_i64().ok_or_else(|| {
                RpcError::from(format!("invalid card id: {value}"))
            })
        })
        .collect()
}

fn model_field_names(
    store: &Collection,
    params: &Value,
) -> Result<Value, RpcError> {
    let requested = param_str(params, "modelName")?;
    let model = store
        .models()?
        .into_iter()
        .find(|model| model.name.eq_ignore_ascii_case(requested))
        .ok_or_else(|| format!("model not found: {requested}"))?;
    Ok(json!(model.fields))
}

fn add_note(store: &mut Collection, params: &Value) -> Result<Value, RpcError> {
    let note = member(params, "note")?;
    let deck_name = member(note, "deckName")?
        .as_str()
        .ok_or("invalid parameter: deckName")?;
    let model_name = member(note, "modelName")?
        .as_str()
        .ok_or("invalid parameter: modelName")?;
    let fields = member(note, "fields")?
        .as_object()
        .ok_or("invalid parameter: fields")?;

    let deck = store
        .decks()?
        .into_iter()
        .find(|deck| deck.name.eq_ignore_ascii_case(deck_name))
        .ok_or_else(|| format!("deck not found: {deck_name}"))?;
    let model = store
        .models()?
        .into_iter()
        .find(|model| model.name.eq_ignore_ascii_case(model_name))
        .ok_or_else(|| format!("model not found: {model_name}"))?;

    for key in fields.keys() {
        if !model
            .fields
            .iter()
            .any(|name| name.eq_ignore_ascii_case(key))
        {
            return Err(format!("unknown field: {key}").into());
        }
    }
    let ordered: Vec<String> = model
        .fields
        .iter()
        .map(|name| {
            fields
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .and_then(|(_, value)| value.as_str())
                .unwrap_or_default()
                .to_owned()
        })
        .collect();
    if ordered.first().is_none_or(|value| value.trim().is_empty()) {
        return Err("empty question".into());
    }
    let tags = note_tags(note)?;
    let ids = store.add_note(&ranki_model::NewNote {
        model_id: model.id,
        deck_id: deck.id,
        fields: ordered,
        tags,
    })?;
    let Some(card_id) = ids.first().copied() else {
        return Ok(Value::Null);
    };
    let note_id = store
        .card(card_id)?
        .map_or(Value::Null, |card| json!(card.note_id));
    Ok(note_id)
}

fn note_tags(note: &Value) -> Result<Vec<String>, RpcError> {
    match note.get("tags") {
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| RpcError::from("invalid parameter: tags"))
            })
            .collect(),
        Some(_) => Err("invalid parameter: tags".into()),
        None => Ok(Vec::new()),
    }
}

fn field_values(
    model: Option<&NoteModel>,
    note: Option<&Note>,
) -> Map<String, Value> {
    let Some(entry) = model else {
        return Map::new();
    };
    let Some(record) = note else {
        return Map::new();
    };
    let mut fields = Map::new();
    for (ord, (name, value)) in
        entry.fields.iter().zip(record.fields.iter()).enumerate()
    {
        let order = i64::try_from(ord).unwrap_or_default();
        fields.insert(name.clone(), json!({"value": value, "order": order}));
    }
    fields
}

#[expect(
    clippy::too_many_arguments,
    reason = "card info needs the store, id and cached lookups"
)]
fn card_info(
    store: &Collection,
    id: Id,
    decks: &[Deck],
    models: &[NoteModel],
) -> Result<Value, RpcError> {
    let Some(card) = store.card(id)? else {
        return Ok(json!({}));
    };
    let note = store.note(card.note_id)?;
    let model = note
        .as_ref()
        .and_then(|entry| models.iter().find(|item| item.id == entry.model_id));
    let deck = decks.iter().find(|entry| entry.id == card.deck_id);
    let question = note
        .as_ref()
        .and_then(|entry| entry.fields.first())
        .cloned()
        .unwrap_or_default();
    let answer = note
        .as_ref()
        .map_or(String::new(), |entry| entry.fields.join(" / "));
    Ok(json!({
        "cardId": card.id,
        "fields": field_values(model, note.as_ref()),
        "fieldOrder": model.map_or_else(Vec::new, |entry| entry.fields.clone()),
        "question": question,
        "answer": answer,
        "modelName": model.map_or(String::new(), |entry| entry.name.clone()),
        "ord": card.ordinal,
        "deckName": deck.map_or(String::new(), |entry| entry.name.clone()),
        "css": model.map_or(String::new(), |entry| entry.css.clone()),
        "factor": card.factor,
        "interval": card.interval,
        "note": card.note_id,
        "type": card.card_type.code(),
        "queue": card.queue.code(),
        "due": card.due,
        "reps": card.reps,
        "lapses": card.lapses,
        "left": card.left,
        "mod": card.modified,
    }))
}

fn cards_info(store: &Collection, params: &Value) -> Result<Value, RpcError> {
    let ids = param_ids(params, "cards")?;
    let decks = store.decks()?;
    let models = store.models()?;
    let mut result = Vec::with_capacity(ids.len());
    for id in ids {
        result.push(card_info(store, id, &decks, &models)?);
    }
    Ok(json!(result))
}

fn answer_cards(
    store: &mut Collection,
    params: &Value,
    scheduler: &FsrsScheduler,
) -> Result<Value, RpcError> {
    let answers = member(params, "answers")?
        .as_array()
        .ok_or("invalid parameter: answers")?;
    let now = crate::now_secs()?;
    let mut result = Vec::with_capacity(answers.len());
    for answer in answers {
        let card_id = member(answer, "cardId")?
            .as_i64()
            .ok_or("invalid parameter: cardId")?;
        let ease = member(answer, "ease")?
            .as_i64()
            .and_then(|ease| u8::try_from(ease).ok())
            .and_then(Rating::from_code)
            .ok_or("invalid parameter: ease")?;
        if store.card(card_id)?.is_none() {
            result.push(false);
            continue;
        }
        answer_card(store, card_id, ease, now, scheduler)?;
        result.push(true);
    }
    Ok(json!(result))
}

fn create_deck(
    store: &mut Collection,
    params: &Value,
) -> Result<Value, RpcError> {
    let name = param_str(params, "deck")?;
    Ok(json!(store.create_deck(name)?.id))
}

fn required_string<'a>(
    params: &'a Value,
    key: &str,
) -> Result<&'a str, RpcError> {
    let value = params.get(key).ok_or_else(|| {
        InterfaceError::new(
            ErrorCode::InvalidData,
            format!("missing parameter: {key}"),
        )
    })?;
    value.as_str().ok_or_else(|| {
        InterfaceError::new(
            ErrorCode::InvalidData,
            format!("invalid parameter: {key}"),
        )
        .into()
    })
}

fn deck_id_param(store: &Collection, params: &Value) -> Result<Id, RpcError> {
    if let Some(id) = params.get("id").and_then(Value::as_i64) {
        return Ok(id);
    }
    let name = required_string(params, "deck")?;
    let deck = store
        .decks()?
        .into_iter()
        .find(|deck| deck.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| {
            ranki_model::StoreError::NotFound(format!("deck not found: {name}"))
        })?;
    Ok(deck.id)
}

fn rename_deck(
    store: &mut Collection,
    params: &Value,
) -> Result<Value, RpcError> {
    let id = deck_id_param(store, params)?;
    let new_name = required_string(params, "newName")?;
    store.rename_deck(id, new_name)?;
    Ok(Value::Null)
}

fn delete_deck(
    store: &mut Collection,
    params: &Value,
) -> Result<Value, RpcError> {
    let id = deck_id_param(store, params)?;
    store.remove_deck(id)?;
    Ok(Value::Null)
}

fn set_suspended(
    store: &mut Collection,
    params: &Value,
    suspend: bool,
) -> Result<Value, RpcError> {
    for id in param_ids(params, "cards")? {
        let Some(mut card) = store.card(id)? else {
            continue;
        };
        card.queue = if suspend {
            CardQueue::Suspended
        } else {
            queue_for(card.card_type)
        };
        save_card(store, card)?;
    }
    Ok(json!(suspend))
}

const fn queue_for(card_type: CardType) -> CardQueue {
    match card_type {
        CardType::New => CardQueue::New,
        CardType::Learning => CardQueue::Learning,
        CardType::Review => CardQueue::Review,
        CardType::Relearning => CardQueue::DayLearning,
    }
}

fn change_deck(
    store: &mut Collection,
    params: &Value,
) -> Result<Value, RpcError> {
    let deck_name = param_str(params, "deck")?;
    let deck = store
        .decks()?
        .into_iter()
        .find(|deck| deck.name.eq_ignore_ascii_case(deck_name))
        .ok_or_else(|| format!("deck not found: {deck_name}"))?;
    for id in param_ids(params, "cards")? {
        let Some(mut card) = store.card(id)? else {
            continue;
        };
        card.deck_id = deck.id;
        card.original_deck_id = 0;
        save_card(store, card)?;
    }
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {

    use serde_json::Value;
    use serde_json::json;
    use tempfile::TempDir;

    use super::FsrsScheduler;
    use super::dispatch;
    use super::handle_body;
    use crate::answer_card;
    use ranki_model::Rating;
    use ranki_model::Store;
    use ranki_storage::Collection;

    use super::RpcResponse;
    use crate::error::ErrorCode;
    fn fixture() -> (TempDir, Collection) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("collection.anki2");
        (dir, Collection::create(&path).unwrap())
    }

    fn scheduler() -> FsrsScheduler {
        ranki_core::from_settings(&ranki_model::Settings::default())
    }

    #[expect(
        clippy::needless_pass_by_value,
        reason = "the helper builds a JSON-RPC envelope from an owned payload"
    )]
    fn call(store: &mut Collection, action: &str, params: Value) -> Value {
        let body = serde_json::to_vec(&json!({
            "action": action, "version": 6, "params": params,
        }))
        .unwrap();
        let response = handle_body(store, &body, &scheduler());
        assert_eq!(response.error, None);
        response.result
    }

    fn note(deck: &str) -> Value {
        json!({
            "deckName": deck,
            "modelName": "Basic",
            "fields": {"Front": "Question", "Back": "Answer"},
            "tags": ["test"],
        })
    }

    fn add_card(store: &mut Collection) -> i64 {
        call(store, "addNote", json!({"note": note("Default")}))
            .as_i64()
            .unwrap()
    }

    #[test]
    fn version_returns_six() {
        let (_dir, mut store) = fixture();
        assert_eq!(call(&mut store, "version", json!({})), json!(6));
    }

    #[test]
    fn deck_names_are_returned() {
        let (_dir, mut store) = fixture();
        assert_eq!(
            call(&mut store, "deckNames", json!({})),
            json!(["Default"])
        );
    }

    #[test]
    fn model_names_are_returned() {
        let (_dir, mut store) = fixture();
        assert_eq!(call(&mut store, "modelNames", json!({})), json!(["Basic"]));
    }

    #[test]
    fn model_field_names_follow_field_order() {
        let (_dir, mut store) = fixture();
        assert_eq!(
            call(&mut store, "modelFieldNames", json!({"modelName": "basic"})),
            json!(["Front", "Back"])
        );
    }

    #[test]
    fn create_deck_returns_an_id() {
        let (_dir, mut store) = fixture();
        let id = call(&mut store, "createDeck", json!({"deck": "A::B"}))
            .as_i64()
            .unwrap();
        assert!(id > 0);
        assert!(
            call(&mut store, "deckNames", json!({}))
                .as_array()
                .unwrap()
                .contains(&json!("A::B"))
        );
    }

    #[test]
    fn add_note_returns_the_note_id() {
        let (_dir, mut store) = fixture();
        let id = add_card(&mut store);
        assert!(id > 0);
        let card_id = store.find_cards("").unwrap().first().copied().unwrap();
        let note_id = store.card(card_id).unwrap().unwrap().note_id;
        assert_eq!(id, note_id);
        assert_eq!(
            call(&mut store, "findCards", json!({"query": ""}))
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn add_note_rejects_unknown_fields() {
        let (_dir, mut store) = fixture();
        let mut payload = note("Default");
        payload["fields"]["Missing"] = json!("x");
        let error = dispatch(
            &mut store,
            "addNote",
            &json!({"note": payload}),
            &scheduler(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn find_cards_matches_fields() {
        let (_dir, mut store) = fixture();
        let id = add_card(&mut store);
        assert_eq!(
            call(&mut store, "findCards", json!({"query": "Question"})),
            json!([store
                .find_cards("Question")
                .unwrap()
                .first()
                .copied()
                .unwrap()])
        );
        assert!(id > 0, "id was {id}");
    }

    #[test]
    fn cards_info_returns_objects_for_existing_and_missing_cards() {
        let (_dir, mut store) = fixture();
        add_card(&mut store);
        let card_id = store.find_cards("").unwrap()[0];
        let info =
            call(&mut store, "cardsInfo", json!({"cards": [card_id, 999]}));
        let entries = info.as_array().unwrap();
        assert_eq!(entries[1], json!({}));
        assert_eq!(entries[0]["cardId"], json!(card_id));
        assert_eq!(entries[0]["deckName"], json!("Default"));
        assert_eq!(entries[0]["fields"]["Front"]["value"], json!("Question"));
    }

    #[test]
    fn answer_cards_reports_which_cards_existed() {
        let (_dir, mut store) = fixture();
        add_card(&mut store);
        let card_id = store.find_cards("").unwrap()[0];
        assert_eq!(
            call(
                &mut store,
                "answerCards",
                json!({"answers": [
                    {"cardId": card_id, "ease": 3}, {"cardId": 999, "ease": 3}
                ]})
            ),
            json!([true, false])
        );
        assert_eq!(store.card(card_id).unwrap().unwrap().reps, 1);
    }

    #[test]
    fn answer_cards_rejects_bad_ease() {
        let (_dir, mut store) = fixture();
        add_card(&mut store);
        let error = dispatch(
            &mut store,
            "answerCards",
            &json!({"answers": [{"cardId": 1, "ease": 9}]}),
            &scheduler(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("ease"));
    }

    #[test]
    fn suspend_and_unsuspend_change_the_queue() {
        let (_dir, mut store) = fixture();
        add_card(&mut store);
        let card_id = store.find_cards("").unwrap()[0];
        assert_eq!(
            call(
                &mut store,
                "suspend",
                json!({"cards": [card_id], "suspend": true})
            ),
            json!(true)
        );
        assert_eq!(store.card(card_id).unwrap().unwrap().queue.code(), -1);
        assert_eq!(
            call(&mut store, "unsuspend", json!({"cards": [card_id]})),
            json!(null)
        );
        assert_eq!(store.card(card_id).unwrap().unwrap().queue.code(), 0);
    }

    #[test]
    fn change_deck_moves_the_card() {
        let (_dir, mut store) = fixture();
        add_card(&mut store);
        let card_id = store.find_cards("").unwrap()[0];
        let deck_id = call(&mut store, "createDeck", json!({"deck": "Other"}))
            .as_i64()
            .unwrap();
        assert_eq!(
            call(
                &mut store,
                "changeDeck",
                json!({"cards": [card_id], "deck": "Other"})
            ),
            json!(null)
        );
        assert_eq!(store.card(card_id).unwrap().unwrap().deck_id, deck_id);
    }

    #[test]
    fn unknown_actions_and_bad_json_report_errors() {
        let (_dir, mut store) = fixture();
        let response = handle_body(
            &mut store,
            br#"{"action":"nope","version":6,"params":{}}"#,
            &scheduler(),
        );
        assert_eq!(response.result, Value::Null);
        assert_eq!(response.error.as_deref(), Some("unsupported action: nope"));
        let response = handle_body(&mut store, b"not json", &scheduler());
        assert!(response.error.unwrap_or_default().contains("invalid JSON"));
    }

    #[test]
    fn answer_card_helper_is_reused_by_the_http_api() {
        let (_dir, mut store) = fixture();
        add_card(&mut store);
        let card_id = store.find_cards("").unwrap()[0];
        let decision = answer_card(
            &mut store,
            card_id,
            Rating::Good,
            crate::now_secs().unwrap(),
            &scheduler(),
        )
        .unwrap();
        assert_eq!(decision.card.id, card_id);
    }

    #[test]
    fn transport_errors_keep_a_stable_code() {
        let (_dir, mut store) = fixture();
        let error = dispatch(&mut store, "cardInfo", &json!({}), &scheduler())
            .unwrap_err();
        assert_eq!(error.0.code(), ErrorCode::Unsupported);

        let response = RpcResponse::error(ranki_model::StoreError::NotFound(
            "card".into(),
        ));
        assert_eq!(response.error_code, Some("not_found"));
        let response =
            RpcResponse::error(ranki_format::FormatError::new("bad"));
        assert_eq!(response.error_code, Some("format"));
        assert_eq!(
            serde_json::to_value(response).unwrap()["errorCode"],
            json!("format")
        );
    }
    #[test]
    fn rename_deck_and_delete_deck_update_the_deck_list() {
        let (_dir, mut store) = fixture();
        let id = call(&mut store, "createDeck", json!({"deck": "Old::Child"}))
            .as_i64()
            .unwrap();
        assert_eq!(
            call(
                &mut store,
                "renameDeck",
                json!({"deck": "Old::Child", "newName": "New::Child"})
            ),
            json!(null)
        );
        let names = call(&mut store, "deckNames", json!({}));
        assert!(names.as_array().unwrap().contains(&json!("New::Child")));
        assert!(!names.as_array().unwrap().contains(&json!("Old::Child")));

        assert_eq!(
            call(&mut store, "deleteDeck", json!({"deck": "new::child"})),
            json!(null)
        );
        let names = call(&mut store, "deckNames", json!({}));
        assert!(!names.as_array().unwrap().contains(&json!("New::Child")));
        assert!(store.decks().unwrap().iter().all(|deck| deck.id != id));
    }

    #[test]
    fn deck_actions_report_stable_error_codes() {
        let (_dir, mut store) = fixture();
        let cases = [
            (
                br#"{"action":"deleteDeck","version":6,"params":{"deck":"Missing"}}"#.as_slice(),
                Some("not_found"),
            ),
            (
                br#"{"action":"deleteDeck","version":6,"params":{"deck":"Default"}}"#.as_slice(),
                Some("invalid_data"),
            ),
            (
                br#"{"action":"renameDeck","version":6,"params":{"id":999}}"#.as_slice(),
                Some("invalid_data"),
            ),
            (
                br#"{"action":"renameDeck","version":6,"params":{"id":999,"newName":"New"}}"#.as_slice(),
                Some("not_found"),
            ),
        ];
        for (body, expected) in cases {
            let response = handle_body(&mut store, body, &scheduler());
            assert_eq!(response.result, Value::Null);
            assert_eq!(response.error_code, expected, "{body:?}");
        }
    }

    #[test]
    fn malformed_parameters_report_precise_errors() {
        let (_dir, mut store) = fixture();
        let request = json!({
            "action": "version", "version": 7, "params": {},
        });
        let body = serde_json::to_vec(&request).unwrap();
        let response = handle_body(&mut store, &body, &scheduler());
        assert_eq!(
            response.error.as_deref(),
            Some("unsupported protocol version: 7")
        );

        let cases = [
            (
                json!({"action":"findCards","params":{"query":42}}),
                "invalid parameter: query",
            ),
            (
                json!({"action":"cardsInfo","params":{"cards":"1"}}),
                "invalid parameter: cards",
            ),
            (
                json!({"action":"cardsInfo","params":{"cards":[1,"x"]}}),
                "invalid card id: \"x\"",
            ),
            (
                json!({"action":"modelFieldNames","params":{"modelName":"Missing"}}),
                "model not found: Missing",
            ),
            (
                json!({"action":"addNote","params":{}}),
                "missing parameter: note",
            ),
            (
                json!({"action":"addNote","params":{"note":{"deckName":"Default","modelName":"Basic","fields":{}}}}),
                "empty question",
            ),
            (
                json!({"action":"addNote","params":{"note":{"deckName":"Missing","modelName":"Basic","fields":{"Front":"Q"}}}}),
                "deck not found: Missing",
            ),
            (
                json!({"action":"answerCards","params":{}}),
                "missing parameter: answers",
            ),
            (
                json!({"action":"answerCards","params":{"answers":{}}}),
                "invalid parameter: answers",
            ),
            (
                json!({"action":"suspend","params":{"cards":[1,"x"]}}),
                "invalid card id: \"x\"",
            ),
        ];
        for (request, expected) in cases {
            let body = serde_json::to_vec(&request).unwrap();
            let response = handle_body(&mut store, &body, &scheduler());
            assert_eq!(response.error.as_deref(), Some(expected), "{request}");
        }
    }
}
