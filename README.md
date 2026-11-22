# Ranki

Ranki is an Anki-compatible flashcard app written in Rust. It reads and writes
Anki collections, imports and exports `.apkg` packages, and provides a
Wayland-first review window, a command line interface, an AnkiConnect-compatible
API, and a Model Context Protocol server.

## Install and run

With Nix installed:

```bash
nix profile add git+https://codeberg.org/DontBlameMe/ranki
ranki --help
```

Run once without installing:

```bash
nix run git+https://codeberg.org/DontBlameMe/ranki -- --help
```

Build from a checkout:

```bash
nix develop              # development shell with Rust and Wayland libraries
cargo build
./target/debug/ranki --help

nix build .#default      # packaged binary in ./result/bin/ranki
./result/bin/ranki --help
```

## Collections

All commands accept the global `--collection PATH` and `--media-dir PATH`
options. Without `--collection`, Ranki opens
`~/.local/share/Anki2/main/collection.anki2`. Media lives in a `collection.media`
directory next to the collection. A missing collection is created by commands
that need it.

## Commands

```bash
ranki gui                                      # review window
ranki decks                                    # list deck IDs and names
ranki due [--deck ID] [--limit N]              # cards due for review
ranki search 'deck:Default tag:flags'          # Anki-style search
ranki add-note --model ID --deck ID \
  --field 'Front' --field 'Back' [--tag TAG]   # add a note and its cards
ranki answer CARD_ID RATING                    # rate a card, 1 again to 4 easy
ranki stats [--deck ID]                        # card counts
ranki import FILE.apkg                         # import a package
ranki export FILE.apkg                         # export a package
ranki check                                    # verify the collection can be read
ranki connect [--port 8765]                    # AnkiConnect JSON-RPC server
ranki mcp                                      # MCP server over stdio
```

Run `ranki COMMAND --help` for the full argument list.

## Automation

`ranki connect` serves a subset of the AnkiConnect API on `127.0.0.1`. The
default port is 8765; change it with `--port`. Clients send JSON-RPC POST
requests such as `{"action":"version","version":6}` to `http://127.0.0.1:8765`.

`ranki mcp` speaks Model Context Protocol on stdin and stdout and writes logs to
stderr. Register the binary with an MCP client as a stdio server, pointing it at
an existing collection:

```bash
ranki --collection /path/to/collection.anki2 mcp
# or: RANKI_COLLECTION=/path/to/collection.anki2 ranki mcp
```

Unlike other commands, `mcp` requires `--collection` or `RANKI_COLLECTION`; it
does not create or use the default collection. Its tools cover decks, due cards,
search, notes, answers, and import/export. Write tools require `confirm: true`.

## Keyboard shortcuts

The review window uses these keys:

| Key | Action |
| --- | --- |
| Space, Enter | Reveal the answer |
| 1, 2, 3, 4 | Again, Hard, Good, Easy |
| R | Toggle the answer |
| Up, Down | Select another deck |
| T | Switch between FSRS and SM-2 |
| F5 | Reload the collection |

## Compatibility

Ranki targets Anki collections with schema 11 through 18 and AnkiConnect API
version 6. Notes, cards, review logs, media, and package import/export are
preserved. Schema 18 stores deck, model, and preset configuration in normalized
protobuf tables; Ranki reconstructs the config JSON needed for its own use
minimally rather than reproducing every Anki setting. Add-on settings and
unused configuration keys may not survive a round trip. Anki's sync protocol
is not implemented, and AnkiConnect support is a subset of the upstream API.

## Testing

```bash
nix build .#checks.x86_64-linux.nextest --print-build-logs
```
