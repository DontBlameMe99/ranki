# Builtin MCP server design

Research and proposal only. Nothing here is implemented yet.

## What "artcraft" is

A shallow `rg -i artcraft` over `/home/user/projects` and `/home/user/.pi` found only the
user prompt in session logs. There is no local ArtCraft checkout. The web result is real:
[ArtCraft](https://getartcraft.com/) is a suite of 17 open source Rust desktop apps by
`storytold`, such as PhotoCraft, DesignCraft and PdfCraft. It is not one program named
"artcraft mcp". Each app ships a CLI with an `mcp` subcommand.

The primary source, [DesignCraft's MCP docs](https://github.com/storytold/designcraft/blob/main/docs/mcp.md),
says `designcraft-cli mcp` speaks newline-delimited JSON-RPC 2.0 on stdin/stdout, logs to
stderr, and uses protocol `2025-06-18` with older versions accepted. The
[CLI source](https://github.com/storytold/designcraft/blob/main/apps/designcraft-cli/src/main.rs)
shows the wiring: a manual `match args.first().map(String::as_str)` dispatches
`Some("mcp") => report(mcp(&args[1..]))`, and `mcp()` parses flags, builds a backend, then
calls `Server::new(backend).serve(stdin.lock(), stdout.lock())`. The PhotoCraft
[book page](https://github.com/storytold/photocraft/blob/main/book/src/automation/mcp.md)
describes the same shape with three backends: in-process headless engine, loopback bridge
to a running app, and a consent-gated open window.
Concrete patterns from ArtCraft worth copying:

- The CLI is the host. `claude mcp add ... -- designcraft-cli mcp` registers one binary.
  No separate server binary, no port, no installer-side config.
- Tools are an agent-facing view over the same engine commands the CLI and UI use.
  `list_commands` exposes the command registry with parameter schemas. `execute` and `batch`
  call commands. The docs list tools such as `inspect_document`, `render_page` and
  `export_png`. Resources `designcraft://document` and `designcraft://commands` expose live
  JSON. All input schemas are closed with `additionalProperties: false`, tools carry
  read-only, idempotent and destructive hints, and unknown keys are reported as invalid
  params.
- Stdio keeps the trust boundary local. `--automation-read-root` and
  `--automation-write-root` confine file access, may repeat, and are rejected together
  with `--connect` because a running GUI is not confined. Refused paths come back as
  readable tool errors.
- Headless and connected modes share one tool surface. Connected mode forwards each tool
  to a token-authenticated loopback control channel. ArtCraft notes that a failed request
  is not replayed, since the operation may have completed.

Other real examples:

- [mise](https://github.com/jdx/mise/blob/main/src/cli/mcp.rs) has `mise mcp`, built on
  `rmcp` with `ToolRouter`, explicit `ServerHandler`, resources `mise://tools`,
  `mise://tasks`, `mise://env`, `mise://config`, and tools `list_commands`, `run_task`,
  `install_tool`. It refuses untrusted project config and labels commands with effects
  `read`, `write` and `destructive`.
- [usage](https://github.com/jdx/usage/blob/main/cli/src/cli/mcp.rs) has `usage mcp -f FILE.usage.kdl`, also on `rmcp`. It builds a current-thread Tokio runtime because the
  server is one stdio conversation. It returns tool errors, not protocol errors, for
  caller mistakes such as an unknown command, and puts usage instructions in
  `ServerConfig`.

## Recommendation

Use one binary. Add a `ranki mcp` subcommand to the existing `clap` enum in `crates/bin`.
Do not add a workspace member or a separate daemon. Register tools only in the first cut.
Resources and prompts can wait until a client actually needs them. An optional later
resource would be `ranki://collection` returning read-only collection metadata.

`nix develop --command cargo info rmcp` on this machine reports:

- `rmcp` 3.11.0, Apache-2.0, rust-version 1.88, docs <https://docs.rs/rmcp>.
- Default features are `base64`, `macros`, `server`. `server` enables `schemars`,
  `transport-async-rw`, `uuid`. `transport-io` adds server stdio via Tokio's `io-std`
  and `net`.
- `tokio` 1.54.1, `schemars` 1.2.3, `clap` 4.6.9, `serde_json` 1.0.154.

Add these to `crates/bin/Cargo.toml` only:

```toml
rmcp = { version = "=3.11.0", features = ["transport-io"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

`serde` and `serde_json` are already present. Use `rmcp::schemars`, so no direct `schemars`
dependency is needed. The SDK is async and its stdio transport is built on Tokio. The
`ranki mcp` arm creates a runtime and stays inside it. `Store` stays synchronous. Use
`std::sync::Mutex<Box<dyn Store>>` (or the concrete storage handle) and keep the lock only
for the synchronous call. Other subcommands and the eframe GUI remain unchanged. A
current-thread runtime is enough; `rt-multi-thread` is included because it is the common
Tokio feature set, but `Builder::new_current_thread()` is the smaller fit.

The following sketch was compile-checked with `rmcp` 3.11.0 and Rust 1.99.1:

```rust
use rmcp::{
    ErrorData, handler::server::wrapper::{Json, Parameters},
    schemars, tool, tool_router, transport::stdio, ServiceExt,
};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListDecks {}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
struct DeckView { id: i64, name: String }

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct DeckList { decks: Vec<DeckView> }

struct RankiMcp { decks: Mutex<Vec<DeckView>> }

#[tool_router(server_handler)]
impl RankiMcp {
    #[tool(description = "List decks in the active collection")]
    fn list_decks(&self, Parameters(_): Parameters<ListDecks>)
        -> Result<Json<DeckList>, ErrorData>
    {
        let decks = self.decks.lock()
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?.clone();
        Ok(Json(DeckList { decks }))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = RankiMcp { decks: Mutex::new(vec![]) };
    let service = server.serve(stdio()).await?; // from ServiceExt
    service.waiting().await?;
    Ok(())
}
```

In Ranki, `RankiMcp` holds the opened collection and a scheduler. The `#[tool_router]`
macro generates `tools/list` schemas from the DTO types. Since `ranki-model` derives
`Serialize` but not `JsonSchema`, define small DTOs in the binary crate with the same field
names. Add `#[serde(deny_unknown_fields)]` to close input schemas, like ArtCraft does.

CLI wiring:

```rust
enum Command {
    Mcp { collection: PathBuf },
    Connect { /* HTTP options */ },
    // review, import, export, GUI
}
```

`Mcp` resolves `--collection PATH` or `RANKI_COLLECTION`, opens the collection once, then
serves stdio. A missing path exits with a usage error on stderr. The subcommand never
starts eframe, never binds a socket and never prints protocol data to stdout. Logs go to
stderr through `tracing-subscriber`.

## Tool surface

All names are snake\_case. Outputs are JSON text in `content` plus `structuredContent`
where the client supports it. `Deck`, `Card`, `Note`, `RevLog` and `Stats` below use the
field names and numeric codes from `crates/model/src/lib.rs`. `Card.due` keeps its
type-dependent meaning from `docs/architecture.md`. `model_id`, `deck_id` and all card
and note ids are Anki `Id` values, that is i64.

Shared shapes:

```text
Deck  = {id:int, name:string, description:string}
Card  = {id:int, note_id:int, deck_id:int, ordinal:int, modified:int, usn:int,
         card_type:"new"|"learning"|"review"|"relearning",
         queue:"new"|"learning"|"review"|"day_learning"|"suspended"|"buried"|"preview",
         due:int, interval:int, factor:int, reps:int, lapses:int, left:int,
         original_due:int, original_deck_id:int, flags:int, data:string}
Note  = {id:int, guid:string, model_id:int, modified:int, usn:int,
         tags:string[], fields:string[], sort_field:string, checksum:int,
         flags:int, data:string}
RevLog= {id:int, card_id:int, usn:int, ease:"again"|"hard"|"good"|"easy",
         interval:int, last_interval:int, factor:int, time_ms:int,
         kind:"learn"|"review"|"relearn"|"filtered"|"manual"}
Stats = {new_cards:int, learning_cards:int, review_cards:int, total_cards:int}
```

Input and output schemas. Generated DTO schemas set `additionalProperties: false`. Optional
fields are omitted when absent.

```json
{
  "list_decks": {
    "input": {"type": "object", "properties": {}},
    "output": {"type": "object", "required": ["decks"],
      "properties": {"decks": {"type": "array", "items": "Deck"}}}
  },
  "due_cards": {
    "input": {"type": "object", "properties": {
      "deck_id": {"type": ["integer", "null"]},
      "limit": {"type": "integer", "minimum": 1, "maximum": 200, "default": 20},
      "now_secs": {"type": "integer", "description": "Unix seconds; defaults to server clock"}}},
    "output": {"type": "object", "required": ["cards", "now_secs"],
      "properties": {"cards": {"type": "array", "items": "Card"}, "now_secs": {"type": "integer"}}}
  },
  "find_cards": {
    "input": {"type": "object", "required": ["query"],
      "properties": {"query": {"type": "string", "minLength": 1}}},
    "output": {"type": "object", "required": ["card_ids"],
      "properties": {"card_ids": {"type": "array", "items": {"type": "integer"}}}}
  },
  "get_card": {
    "input": {"type": "object", "required": ["id"],
      "properties": {"id": {"type": "integer"}}},
    "output": {"type": "object", "required": ["card", "note"],
      "properties": {"card": ["Card", "null"], "note": ["Note", "null"]}}
  },
  "add_note": {
    "input": {"type": "object", "required": ["model_id", "deck_id", "fields", "confirm"],
      "properties": {
        "model_id": {"type": "integer"}, "deck_id": {"type": "integer"},
        "fields": {"type": "array", "minItems": 1, "items": {"type": "string"}},
        "tags": {"type": "array", "items": {"type": "string"}, "default": []},
        "confirm": {"const": true}}},
    "output": {"type": "object", "required": ["card_ids"],
      "properties": {"card_ids": {"type": "array", "items": {"type": "integer"}}}}
  },
  "answer_card": {
    "input": {"type": "object", "required": ["card_id", "rating", "confirm"],
      "properties": {
        "card_id": {"type": "integer"},
        "rating": {"enum": ["again", "hard", "good", "easy"]},
        "elapsed_ms": {"type": "integer", "minimum": 0, "default": 0},
        "confirm": {"const": true}}},
    "output": {"type": "object", "required": ["card", "revlog"],
      "properties": {"card": "Card", "revlog": "RevLog"}}
  },
  "stats": {
    "input": {"type": "object", "properties": {
      "deck_id": ["integer", "null"], "now_secs": {"type": "integer"}}},
    "output": {"type": "object", "required": ["stats"], "properties": {"stats": "Stats"}}
  },
  "import_apkg": {
    "input": {"type": "object", "required": ["path", "confirm"],
      "properties": {"path": {"type": "string"}, "confirm": {"const": true}}},
    "output": {"type": "object", "required": ["path", "notes_added", "cards_added"],
      "properties": {"path": {"type": "string"}, "notes_added": {"type": "integer"},
                     "cards_added": {"type": "integer"}}}
  },
  "export_apkg": {
    "input": {"type": "object", "required": ["path", "confirm"],
      "properties": {"path": {"type": "string"}, "confirm": {"const": true}}},
    "output": {"type": "object", "required": ["path", "bytes"],
      "properties": {"path": {"type": "string"}, "bytes": {"type": "integer"}}}
  }
}
```

Mapping to the `Store` trait:

- `list_decks` -\> `decks`. `due_cards` -\> `due_cards`. `find_cards` -\> `find_cards`.
  `get_card` -\> `card` and `note`. `add_note` builds `NewNote { model_id, deck_id, fields, tags }` and returns `add_note`'s card ids. `answer_card` loads the card, maps the rating
  to `Rating::code()` 1 to 4, asks `ranki-core` for the new card and `RevLog`, then calls
  `apply_review` with the resulting `ReviewDecision`. `stats` -\> `stats`. `import_apkg`
  and `export_apkg` call `ranki-format`; their counters follow that crate's result.

## Errors and safety

- Malformed calls, unknown fields and invalid enum values return a protocol error
  (`ErrorData::invalid_params`). A valid call that cannot complete returns
  `Ok(CallToolResult::error(...))` with a short message the model can act on. This follows
  the `rmcp` guidance and the mise and usage code.
- Map `StoreError::NotFound` to a tool error naming the id. `InvalidData` and `Backend`
  become tool errors with the storage message. `Unsupported` says the operation is not
  available. A poisoned mutex is an internal error. Never panic in a tool, never include a
  backtrace, and never write logs to stdout.
- Writes (`add_note`, `answer_card`, `import_apkg`, `export_apkg`) require `confirm: true`. A missing or false value performs no write and returns a tool error. Tool
  annotations mark them non-idempotent and not destructive.
- Expose no delete, suspend, bury, reset, empty-deck or schema-changing tool. If a later
  change needs one, it gets its own design.
- The collection comes from `ranki mcp --collection PATH` or `RANKI_COLLECTION`. Missing
  collection is a startup error; do not create one implicitly. Relative paths resolve
  against the server's launch directory and are canonicalized at startup.
- Filesystem access is limited to the collection's directory. `import_apkg` reads only a
  file inside it. `export_apkg` writes only inside it and fails if the target exists.
  Canonicalize before opening, follow symlinks, and reject paths outside the directory.
  This is the small version of ArtCraft's read and write roots.
- Import adds notes and cards but must not delete or overwrite existing records. Export is
  read-only on the collection.
- One MCP client owns one server process. Do not share a mutable `Store` between the MCP
  server and the HTTP server in the first cut.

## Coexistence with `ranki connect`

`ranki connect` remains the AnkiConnect-compatible HTTP JSON-RPC API. It is a separate
subcommand with a local listener, used by Anki tooling that expects AnkiConnect. `ranki mcp` is a local stdio process launched by an MCP client. There is no shared port, no HTTP
inside `mcp`, and no MCP inside `connect`. Both call the same `Store` implementations and
the same `ranki-core` scheduler through thin adapters, so their behavior stays aligned
without coupling their transports. A later bridge from MCP to a running GUI would follow
ArtCraft's connected mode, but it is not needed to ship the first tool surface.

## Sources

- <https://getartcraft.com/>
- <https://github.com/storytold/designcraft/blob/main/docs/mcp.md>
- <https://github.com/storytold/designcraft/blob/main/apps/designcraft-cli/src/main.rs>
- <https://github.com/storytold/photocraft/blob/main/book/src/automation/mcp.md>
- <https://github.com/jdx/mise/blob/main/src/cli/mcp.rs>
- <https://github.com/jdx/usage/blob/main/cli/src/cli/mcp.rs>
- <https://github.com/modelcontextprotocol/rust-sdk/blob/main/README.md>
- `nix develop --command cargo info rmcp`, `tokio`, `clap`, `schemars`, `serde_json`
