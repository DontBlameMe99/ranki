# Ranki mobile

`crates/mobile` is the phone UI for Ranki. It uses egui and eframe 0.32. Desktop
Linux builds use the `glow` backend. Android and iOS builds use `wgpu`. GPUI
stays in `crates/ui` for the desktop app and is not a dependency of this crate.
Card text rendering uses plain text and local images only. It does not use
LaTeX.

The mobile crate shares `ranki-model`, `ranki-core`, `ranki-storage`, and
`ranki-sync` with the desktop app. Target-specific dependencies are declared in
`crates/mobile/Cargo.toml`, so a desktop build pulls only the desktop eframe
features and a mobile build pulls only the mobile features.

## Desktop build and run

```bash
nix develop --command cargo build -p ranki-mobile
nix develop --command cargo run -p ranki-mobile
```

The binary is `target/debug/ranki-mobile`. It opens a 390x844 window. On Linux
and macOS the data and config directories come from `RANKI_DATA_DIR` and
`RANKI_CONFIG_DIR`, then XDG directories, then `HOME`. The app creates the
collection on first launch.

## Mobile builds

The dev shell includes the Rust standard library targets for Android and iOS.
An Android build also needs the Android SDK and NDK environment that `cargo-ndk`
or the Android toolchain expects. An iOS build needs macOS with Xcode.

```bash
nix develop --command cargo build -p ranki-mobile --target aarch64-linux-android
nix develop --command cargo build -p ranki-mobile --target aarch64-apple-ios
```

The matching mobile target checks from CI are:

```bash
CC_aarch64_linux_android=gcc CRATE_CC_NO_DEFAULTS=1 \
  nix develop --command cargo check -p ranki-mobile --target aarch64-linux-android
CC_aarch64_apple_ios=clang CRATE_CC_NO_DEFAULTS=1 \
  nix develop --command cargo check -p ranki-mobile --target aarch64-apple-ios
```

Android startup uses `app.internal_data_path()` when the platform supplies it.
`platform::run` puts that path into `RANKI_DATA_DIR` before building the app
state, so no `HOME` is required on Android.

## Checks

Run the mobile tests alone during iteration:

```bash
nix develop --command cargo test -p ranki-mobile
```

Run the same checks as CI before finishing:

```bash
nix develop --command cargo test --workspace --all-targets --no-fail-fast
nix develop --command cargo clippy --workspace --all-features --all-targets -- -D warnings
nix develop --command cargo fmt --all -- --check
```

The mobile tests create a temporary `collection.anki2` with `tempfile`. They do
not touch a real collection, make network calls, or depend on a Wayland session.

## Deterministic screenshots

`scripts/mobile-visual-check.sh` builds the CLI and the mobile binary, seeds a
collection under `.tmp`, starts a private headless sway session with a 390x844
output, and captures the decks, question, answer, add, browse, and settings
screens. It never uses the user's Wayland session or a system input tool.

`AppState::new` reads two development variables for these captures:

| Variable | Value | Effect |
| --- | --- | --- |
| `RANKI_MOBILE_DEV_VIEW` | `decks`, `review`, `add`, `browse`, `settings` | Opens that screen at startup |
| `RANKI_MOBILE_DEV_REVEAL` | `1` | Starts the review screen on the answer side |

Run it from the repository root:

```bash
scripts/mobile-visual-check.sh
```

The script prints the screenshot paths when it finishes. It removes its private
runtime directory on exit and leaves the screenshots under `.tmp` for review.
