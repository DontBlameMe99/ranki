# Ranki architecture

Ranki is a Rust reimplementation of Anki. It reads and writes Anki collections
directly, so a collection created here opens in Anki and vice versa.

## Crates

- `crates/model` (`ranki-model`): shared data types, Anki numeric codes, `Store`
  trait and `StoreError`. Owned by the orchestrator. Do not edit from worktrees.
- `crates/core` (`ranki-core`): the Anki-compatible FSRS scheduler and
  deck presets. Depends only on `ranki-model`.
- `crates/storage` (`ranki-storage`): SQLite collection access. Implements
  `ranki_model::Store` against Anki's `col`, `notes`, `cards`, `revlog` and
  `graves` tables.
- `crates/format` (`ranki-format`): `.apkg` import/export and media mapping.
- `crates/sync` (`ranki-sync`): AnkiWeb v11 sync client.
- `crates/ui` (`ranki-ui`): GPUI application with Wayland support; deck list,
  review screen, editor, stats.
- `crates/bench` (`ranki-bench`): headless benchmark harness for Ranki surfaces.
- `crates/bin` (`ranki`): CLI and AnkiConnect-compatible JSON-RPC server.

## Contract

The `Store` trait in `crates/model/src/lib.rs` is the boundary between layers.
Implementations may add convenience methods in their own crate but must keep
the trait signatures intact. `ReviewDecision` carries the final card state and
its log entry; storage persists both atomically.

Timestamps: `notes.modified` and `cards.modified` are Unix seconds;
`revlog.time` is milliseconds; `revlog.id` is milliseconds. A review card's
`due` is a day number, a learning card's `due` is Unix milliseconds, and a new
card's `due` is a queue position.

## Development

- Toolchain: `nix develop --command cargo test --workspace --all-targets --no-fail-fast`.
- Lints come from the workspace; code must pass `nix develop --command cargo clippy --workspace --all-features --all-targets -- -D warnings` and `nix develop --command cargo fmt --all -- --check`.
- Keep dependencies in the crate's own `Cargo.toml`; do not edit the workspace
  manifest from a worktree.
