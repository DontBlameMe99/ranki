//! End-to-end coverage of the MCP stdio server.

#![expect(clippy::arithmetic_side_effects, reason = "test request ids")]
#![expect(clippy::excessive_nesting, reason = "test response loop")]
#![expect(clippy::indexing_slicing, reason = "test assertions")]
#![expect(
    clippy::needless_pass_by_value,
    reason = "test helpers own JSON payloads"
)]
#![expect(clippy::panic, reason = "test assertions")]
#![expect(clippy::unwrap_used, reason = "test assertions")]
use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::ChildStdin;
use std::process::Command;
use std::process::Stdio;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::thread;
use std::time::Duration;

use serde_json::Value;
use serde_json::json;
use tempfile::TempDir;

const PROTOCOL_VERSION: &str = "2025-06-18";

struct McpClient {
    process: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    next_id: i64,
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

impl McpClient {
    fn start(collection: &Path) -> Self {
        let mut process = Command::new(env!("CARGO_BIN_EXE_ranki"))
            .arg("--collection")
            .arg(collection)
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = process.stdin.take().unwrap();
        let stdout = process.stdout.take().unwrap();
        let (sender, lines) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) if sender.send(line).is_err() => break,
                    Ok(_) => {}
                }
            }
        });
        let mut client = Self {
            process,
            stdin,
            lines,
            next_id: 0,
        };
        let response = client.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "ranki-ci", "version": "1"},
            }),
        );
        assert_eq!(
            response["result"]["protocolVersion"],
            json!(PROTOCOL_VERSION)
        );
        client.notify("notifications/initialized", json!({}));
        client
    }

    fn send(&mut self, message: Value) {
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({
            "jsonrpc": "2.0", "method": method, "params": params,
        }));
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({
            "jsonrpc": "2.0", "id": id, "method": method, "params": params,
        }));
        loop {
            let line = self
                .lines
                .recv_timeout(Duration::from_secs(10))
                .unwrap_or_else(|error| {
                    panic!("MCP server did not answer id {id}: {error}")
                });
            let response: Value = serde_json::from_str(&line).unwrap();
            if response["id"] == json!(id) {
                return response;
            }
        }
    }

    fn tool(&mut self, name: &str, arguments: Value) -> Value {
        let response = self.request(
            "tools/call",
            json!({"name": name, "arguments": arguments}),
        );
        assert!(response.get("error").is_none(), "{response}");
        assert_eq!(response["result"]["isError"], json!(false), "{response}");
        response["result"]["structuredContent"].clone()
    }

    fn tool_failure(&mut self, name: &str, arguments: Value) -> String {
        let response = self.request(
            "tools/call",
            json!({"name": name, "arguments": arguments}),
        );
        assert_eq!(response["result"]["isError"], json!(true), "{response}");
        response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn invalid_params(&mut self, name: &str, arguments: Value) -> String {
        let response = self.request(
            "tools/call",
            json!({"name": name, "arguments": arguments}),
        );
        assert_eq!(response["error"]["code"], json!(-32602), "{response}");
        response["error"]["message"].as_str().unwrap().to_owned()
    }
}

fn collection(dir: &TempDir, name: &str) -> PathBuf {
    let path = dir.path().join(name);
    let output = Command::new(env!("CARGO_BIN_EXE_ranki"))
        .arg("--collection")
        .arg(&path)
        .arg("check")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    path
}

fn fixture() -> (TempDir, McpClient) {
    let dir = tempfile::tempdir().unwrap();
    let path = collection(&dir, "collection.anki2");
    let client = McpClient::start(&path);
    (dir, client)
}

fn add_card(client: &mut McpClient) -> i64 {
    let added = client.tool(
        "add_note",
        json!({
            "model_id": 1,
            "deck_id": 1,
            "fields": ["MCP front", "MCP back"],
            "tags": ["stdio"],
            "confirm": true,
        }),
    );
    let ids = added["card_ids"].as_array().unwrap();
    assert_eq!(ids.len(), 1);
    ids[0].as_i64().unwrap()
}

#[test]
fn stdio_lists_the_documented_tool_surface() {
    let (_dir, mut client) = fixture();
    let response = client.request("tools/list", json!({}));
    let tools = response["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "add_note",
            "answer_card",
            "create_deck",
            "due_cards",
            "export_apkg",
            "find_cards",
            "get_card",
            "import_apkg",
            "list_decks",
            "remove_deck",
            "rename_deck",
            "stats",
        ]
    );
    for name in ["add_note", "answer_card", "create_deck", "export_apkg"] {
        let tool = tools
            .iter()
            .find(|tool| tool["name"].as_str() == Some(name))
            .unwrap();
        assert_eq!(
            tool["inputSchema"]["properties"]["confirm"]["const"],
            json!(true),
            "{name} must require confirmation"
        );
    }
}

#[test]
fn stdio_adds_answers_and_reports_one_card() {
    let (_dir, mut client) = fixture();
    let card_id = add_card(&mut client);

    let found = client.tool("find_cards", json!({"query": "MCP front"}));
    assert_eq!(found["card_ids"], json!([card_id]));

    let due =
        client.tool("due_cards", json!({"limit": 10, "now_secs": 1_000_000}));
    assert_eq!(due["cards"][0]["id"], json!(card_id));
    assert_eq!(due["now_secs"], json!(1_000_000));

    let card = client.tool("get_card", json!({"id": card_id}));
    assert_eq!(card["note"]["fields"], json!(["MCP front", "MCP back"]));
    assert_eq!(card["note"]["tags"], json!(["stdio"]));

    let reviewed = client.tool(
        "answer_card",
        json!({
            "card_id": card_id,
            "rating": "good",
            "elapsed_ms": 1234,
            "confirm": true,
        }),
    );
    assert_eq!(reviewed["card"]["id"], json!(card_id));
    assert_eq!(reviewed["revlog"]["ease"], json!("good"));
    assert_eq!(reviewed["revlog"]["time_ms"], json!(1234));

    let stats = client.tool("stats", json!({"now_secs": 1_000_000}));
    assert_eq!(stats["stats"]["total_cards"], json!(1));
    assert_eq!(stats["stats"]["new_cards"], json!(0));
}

#[test]
fn stdio_rejects_unconfirmed_and_invalid_calls() {
    let (_dir, mut client) = fixture();
    let refusal = client.tool_failure(
        "add_note",
        json!({"model_id": 1, "deck_id": 1, "fields": ["x"], "confirm": false}),
    );
    assert!(refusal.contains("confirm: true"), "{refusal}");

    assert_eq!(
        client.invalid_params("find_cards", json!({"query": "  "})),
        "query must not be empty"
    );
    assert_eq!(
        client.invalid_params("due_cards", json!({"limit": 201})),
        "limit must be between 1 and 200"
    );
    assert_eq!(
        client.invalid_params(
            "create_deck",
            json!({"name": "A::", "confirm": true})
        ),
        "invalid deck name"
    );
    assert_eq!(
        client.invalid_params(
            "import_apkg",
            json!({"path": "../escape.apkg", "confirm": true})
        ),
        "path must not contain '.' or '..'"
    );
    assert_eq!(
        client.tool("get_card", json!({"id": 999})),
        json!({"card": null, "note": null})
    );
}

#[test]
fn stdio_exports_and_imports_within_the_collection_directory() {
    let dir = tempfile::tempdir().unwrap();
    let source = collection(&dir, "source.anki2");
    let mut from = McpClient::start(&source);
    add_card(&mut from);

    let exported = from.tool(
        "export_apkg",
        json!({"path": "backup.apkg", "confirm": true}),
    );
    assert_eq!(exported["path"], json!("backup.apkg"));
    assert!(exported["bytes"].as_u64().unwrap() > 0);
    assert!(dir.path().join("backup.apkg").is_file());
    let duplicate = from.tool_failure(
        "export_apkg",
        json!({"path": "backup.apkg", "confirm": true}),
    );
    assert!(duplicate.contains("already exists"), "{duplicate}");
    drop(from);

    let destination = collection(&dir, "destination.anki2");
    let mut to = McpClient::start(&destination);
    let imported = to.tool(
        "import_apkg",
        json!({"path": "backup.apkg", "confirm": true}),
    );
    assert_eq!(imported["notes_added"], json!(1));
    assert_eq!(imported["cards_added"], json!(1));
    let found = to.tool("find_cards", json!({"query": "MCP front"}));
    assert_eq!(found["card_ids"].as_array().unwrap().len(), 1);

    let missing = to.tool_failure(
        "import_apkg",
        json!({"path": "missing.apkg", "confirm": true}),
    );
    assert!(missing.contains("package not found"), "{missing}");
}
