# Ranki

Ranki is an Anki-compatible flashcard app written in Rust. It manages its own
Anki-compatible collection, imports and exports packages, and provides a
Wayland-first review window, a command line interface, an AnkiConnect-compatible
API, and a Model Context Protocol server.

## Install and run

With Nix installed:

```bash
nix profile add git+https://github.com/DontBlameMeDev/ranki
ranki --help
```

Run once without installing:

```bash
nix run git+https://github.com/DontBlameMeDev/ranki -- --help
```

Build from a checkout:

```bash
nix develop              # development shell with Rust and Wayland libraries
cargo build
./target/debug/ranki --help

nix build .#default      # packaged binary in ./result/bin/ranki
./result/bin/ranki --help
```

The desktop UI uses GPUI and stays the default. The touch UI is a separate
crate with no GPUI dependency:

```bash
nix develop --command cargo run -p ranki-mobile
CC_aarch64_linux_android=gcc CRATE_CC_NO_DEFAULTS=1 \
  nix develop --command cargo check -p ranki-mobile --target aarch64-linux-android
CC_aarch64_apple_ios=clang CRATE_CC_NO_DEFAULTS=1 \
  nix develop --command cargo check -p ranki-mobile --target aarch64-apple-ios
```

See `docs/mobile.md` for the mobile data directories, platform entry points,
and visual check command.

## Home Manager

Add Ranki and Home Manager as flake inputs, both following the same nixpkgs:

```nix
{
  inputs.ranki.url = "git+https://github.com/DontBlameMeDev/ranki";
  inputs.ranki.inputs.nixpkgs.follows = "nixpkgs";
  inputs.home-manager.url = "github:nix-community/home-manager";
  inputs.home-manager.inputs.nixpkgs.follows = "nixpkgs";
}
```

Import the module and enable it:

```nix
{ inputs, ... }:
{
  imports = [ inputs.ranki.homeManagerModules.default ];

  programs.ranki = {
    enable = true;
    settings.ui.theme = "dark";
  };
}
```

For a build without the Wayland review window and its GPUI dependency, use
`cargo build -p ranki --no-default-features`. The CLI, storage, format, and sync crates
build without desktop UI libraries.

```bash
home-manager switch --flake .#user@host
```

The module writes `~/.config/ranki/config.toml` when `settings` is set and leaves an
existing file alone unless `forceConfig = true`. It installs a desktop entry by default
and uses the wrapped Ranki package unless `package` is overridden. The package installs
`assets/rai.svg` as the hicolor icon `ranki`, which the desktop entry uses. The generated
settings use this schema:

```toml
[ui]
theme = "system"   # dark | light | system
font_size = 16.0
review_batch_size = 20
show_keyboard_hints = true
reduce_motion = false # omitted: follow the desktop animation preference
[scheduler]
desired_retention = 0.9
```

The GUI keeps window state in `ui-state.toml` next to `config.toml`: the active
screen, sidebar width, expanded decks, selected deck, search queries, and theme
choice. Home Manager does not manage that file, so switching generations does
not undo the layout.

## Collections

Ranki owns one application data directory:

```text
$RANKI_DATA_DIR/            # or $XDG_DATA_HOME/ranki, then ~/.local/share/ranki
├── collection.anki2
├── collection.media/
└── ranki-sync.json
```

Set `RANKI_DATA_DIR` to use an app-owned directory supplied by a platform or a
custom location. Desktop builds otherwise use `XDG_DATA_HOME/ranki` or
`~/.local/share/ranki`. Settings live in `$RANKI_CONFIG_DIR/config.toml`, then
`$XDG_CONFIG_HOME/ranki/config.toml`, then `~/.config/ranki/config.toml`.
The data directory is the storage seam for mobile ports: a platform supplies its
own app data and config directories and the files inside stay the same.

The `--collection` and `--media-dir` flags, `RANKI_COLLECTION`, and path keys in
`[general]` no longer select storage. An existing `[general]` section is ignored with a
warning. Ranki does not open an Anki profile; use `.apkg` or `.colpkg` packages to move
data between apps. A missing collection is created by commands that need it.

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
ranki import FILE.apkg                         # import .apkg or .colpkg package
ranki export FILE.apkg                         # export an .apkg package
ranki export-colpkg FILE.colpkg                # export a full collection backup
ranki check                                    # verify the collection can be read
ranki connect [--port 8765]                    # AnkiConnect JSON-RPC server
ranki mcp                                      # MCP server over stdio
```

Run `ranki COMMAND --help` for the full argument list.

`ranki sync` also uploads, downloads, or checks the remote collection. Section
[Sync](#sync) covers credentials and the optional window login.

## Sync

AnkiWeb sync is optional. In the review window, open Settings and sign in with
your AnkiWeb username and password. Use **Sign out** in the same panel to stop
syncing. Ranki stores only the resulting host key in `ranki-sync.json` in its app data
directory, with mode `0600` on Unix. The password is discarded after login. Ranki
syncs after each completed study session, at launch, and at exit. The sidebar
shows the age of the last successful sync.

Command-line sync accepts a host key or a one-time login without putting a
password in the process arguments:

```bash
export RANKI_SYNC_HKEY='your-host-key'  # or RANKI_SYNC_USERNAME + RANKI_SYNC_PASSWORD
ranki sync --status                     # inspect remote metadata
ranki sync                              # safe automatic direction
ranki sync --full-upload                # explicit one-way choices
ranki sync --full-download
```

`ranki sync` writes the same state file, so the window and the CLI share one
sync status. Ordinary commands sync the collection only, so a large media
library never delays them. `ranki sync` also refreshes media; the review
window does that in the background.

## Automation

`ranki connect` serves a subset of the AnkiConnect API on `127.0.0.1`. The
default port is 8765; change it with `--port`. Clients send JSON-RPC POST
requests such as `{"action":"version","version":6}` to `http://127.0.0.1:8765`.

`ranki mcp` speaks Model Context Protocol on stdin and stdout and writes logs to
stderr. Register the binary with an MCP client as a stdio server:

```bash
ranki mcp
```

It uses Ranki's app collection and takes no path flag. Its tools cover decks, due
cards, search, notes, answers, and import/export. Write tools require `confirm: true`.

## Keyboard shortcuts

The review window uses these keys:

| Key | Action |
| --- | --- |
| Space, Enter | Reveal the answer |
| 1, 2, 3, 4 | Again, Hard, Good, Easy |
| R | Toggle the answer |
| N | Select another deck |
| F5 | Reload the collection |

## Compatibility

Ranki targets Anki collections with schema 11 through 18 and AnkiConnect API
version 6. Notes, cards, review logs, media, and package import/export are
preserved. Schema 18 stores deck, model, and preset configuration in normalized
protobuf tables; Ranki reconstructs the config JSON needed for its own use
minimally rather than reproducing every Anki setting. Add-on settings and
unused configuration keys may not survive a round trip. Anki's sync protocol
is limited to login, metadata checks, and full collection transfers. Media sync
and delta merges are not implemented, and AnkiConnect support is a subset of the
upstream API.

All reviews use Anki-compatible FSRS: Anki's default parameters unless the
config overrides them, and memory state stored in `cards.data` for Anki to read.

## Testing

```bash
nix build .#checks.x86_64-linux.nextest --print-build-logs
```

Coverage must stay at or above 80%. The floor is set in `.tarpaulin.toml` and
applies to local runs and CI:

```bash
nix build .#checks.x86_64-linux.tarpaulin --print-build-logs
```

Performance is measured across the CLI, AnkiConnect, MCP, GUI, storage,
scheduling, and package surfaces in release builds. A pull request fails the
`Performance audit` check when a workload regresses. See
[docs/performance.md](docs/performance.md) for the method and local commands.
