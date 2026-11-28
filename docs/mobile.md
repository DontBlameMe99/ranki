# Ranki mobile

Ranki has one `ranki` package and one collection format. Desktop windows use the
GPUI renderer in `crates/ui`. Below 640px, that renderer switches from the side
rail to bottom navigation. Android and iOS use the touch renderer in
`crates/bin/src/mobile`, built with egui and eframe 0.32.

The touch renderer shares `ranki-model`, `ranki-core`, `ranki-storage`,
`ranki-format`, and `ranki-sync` with the desktop app. Target-specific dependencies
live in `crates/bin/Cargo.toml`. Android and iOS builds disable the default desktop
feature, so they do not compile GPUI.

Android uses AndroidX `GameActivity` and the vendored winit IME backport so text
fields open the soft keyboard and receive composed text.

The touch renderer includes deck creation, rename and removal, review with undo,
note adding and editing, browse, statistics, settings, and AnkiWeb sync.

## Desktop preview

```bash
nix develop --command cargo run
nix develop --command cargo run -- --mobile
```

`cargo run` opens the normal desktop layout. `--mobile` opens the touch renderer
in a 390x844 window from the same `target/debug/ranki` binary. Both use
`RANKI_DATA_DIR` and `RANKI_CONFIG_DIR`, then XDG directories, then `HOME`. The
app creates the collection on first launch.

## Android and iOS builds

The dev shell includes the Rust standard library targets for Android and iOS.
An Android build also needs the Android SDK and NDK environment that `cargo-ndk`
or the Android toolchain expects. An iOS build needs macOS with Xcode.

```bash
nix develop --command bash -lc \
  'nix shell nixpkgs#cargo-ndk --command cargo ndk --target arm64-v8a --platform 26 check --package ranki --no-default-features --lib'
CC_aarch64_apple_ios=clang CRATE_CC_NO_DEFAULTS=1 \
  nix develop --command cargo check -p ranki --no-default-features --target aarch64-apple-ios
```

The matching target checks run in CI. `docs/Android-build.md` documents the
reproducible arm64 APK build, SDK prerequisites, signing, and output paths.

Android startup uses `app.internal_data_path()` when the platform supplies it.
`platform::run_with_android_app` sets `RANKI_DATA_DIR` to its `ranki` subdirectory
and `RANKI_CONFIG_DIR` to its `config` subdirectory before building app state, so
no `HOME` is required on Android.

## Checks

Run the touch renderer tests during iteration:

```bash
nix develop --command cargo test -p ranki --all-features --lib mobile::
```

Run the same checks as CI before finishing:

```bash
nix develop --command cargo test --workspace --all-targets --no-fail-fast
nix develop --command cargo clippy --workspace --all-features --all-targets -- -D warnings
nix develop --command cargo fmt --all -- --check
bash android/scripts/test-build-apk.sh
```

The touch renderer tests create a temporary `collection.anki2` with `tempfile`.
They do not touch a real collection, make network calls, or depend on a Wayland
session.

## Deterministic screenshots

`scripts/visual-check.sh` captures the normal renderer at desktop, small-window,
and phone widths. `scripts/mobile-visual-check.sh` builds the same `ranki` binary
with the touch renderer, seeds a collection under `.tmp`, starts a private
headless sway session with a phone-sized output, and captures the decks, question,
answer, add, browse, statistics, and settings screens. Neither script uses the user's Wayland
session or a system input tool.

Set `MOBILE_VISUAL_WIDTH` and `MOBILE_VISUAL_HEIGHT` to capture another viewport.
The touch renderer reads these development variables:

| Variable | Value | Effect |
| --- | --- | --- |
| `RANKI_MOBILE_DEV_VIEW` | `decks`, `review`, `add`, `browse`, `statistics`, `settings` | Opens that screen at startup |
| `RANKI_MOBILE_DEV_REVEAL` | `1` | Starts the review screen on the answer side |
| `RANKI_MOBILE_DEV_WIDTH` / `RANKI_MOBILE_DEV_HEIGHT` | pixels | Sets the phone viewport size |

Run them from the repository root:

```bash
scripts/visual-check.sh
scripts/mobile-visual-check.sh
MOBILE_VISUAL_WIDTH=360 MOBILE_VISUAL_HEIGHT=640 scripts/mobile-visual-check.sh
```

The scripts print screenshot paths when they finish. They remove their private
runtime directories on exit and leave screenshots under `.tmp` for review.
