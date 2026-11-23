# GPUI research for Ranki

Research only, done in a T3 worktree. No product code, manifest, flake, or README changed.

## 1. Headline

`cargo info gpui` reports **gpui 0.2.2**, published 2025-10-22. It is Zed's pre-1.0 GPU UI framework with a native Wayland backend, no winit, and an explicit warning that versions will break often. On this machine a wayland-only build and the upstream `hello_world` example work after adding one missing Nix build input. The default dev shell currently fails at link time.

## 2. Crate facts

Command: `nix develop --command cargo info gpui`.

| Fact | Value | Source |
| --- | --- | --- |
| Latest published version | 0.2.2 | [crates.io](https://crates.io/crates/gpui/0.2.2) |
| Published | 2025-10-22 | [crates.io API](https://crates.io/api/v1/crates/gpui/0.2.2) |
| License | Apache-2.0 | cargo info |
| Edition | 2024 | crates.io API |
| Declared MSRV | none (`rust_version: null`) | crates.io API |
| Local toolchain | rustc 1.99.1, cargo 1.99.1 | `nix develop --command rustc --version` |
| Downloads | 529,252 at research time | crates.io API |
| Crate size | 5,291,873 bytes, 65,836 Rust code lines | crates.io API |

README quote: "GPUI is still in active development as we work on the Zed code editor, and is still pre-1.0. There will often be breaking changes between versions. You'll also need to use the latest version of stable Rust and be on macOS or Linux." Source: [docs.rs README](https://docs.rs/gpui/0.2.2/gpui/).

MSRV conclusion: no crate MSRV is declared. Edition 2024 needs Rust 1.85 at minimum. The README asks for latest stable Rust and this repo already uses `rust-bin.stable.latest`, so no toolchain pin is needed, but an exact `gpui` pin is.

Features from cargo info:

- Default: `font-kit`, `wayland`, `x11`, `windows-manifest`.
- Wayland: `blade-graphics`, `blade-macros`, `blade-util`, `bytemuck`, `ashpd/wayland`, `cosmic-text`,
  `font-kit`, `calloop-wayland-source`, `wayland-backend`, `wayland-client`, `wayland-cursor`,
  `wayland-protocols`, `wayland-protocols-plasma`, `filedescriptor`, `xkbcommon`, `open`.
- Others: `inspector`, `leak-detection`, `macos-blade`, `runtime_shaders`, `screen-capture`,
  `test-support`, `x11`.

For Linux-only Ranki use `gpui = { version = "=0.2.2", default-features = false, features = ["wayland"] }`.

Dependency weight, measured with `cargo tree -e normal --prefix none` on the published crate manifest: 472 unique crates with `wayland` only, 485 with default features. Current `ranki-ui` with eframe and egui resolves 136 unique crates. A check plus example build used 1.2 GB of target space. Published source is 3.0 MB under `src/`.

Stability verdict: usable for a real app, as Zed proves, but 0.2.2 is not stable enough to float versions. Upstream work happens in the Zed monorepo and the main branch README now points to a split `gpui_platform` and `gpui_linux` layout, so fixes after October 2025 are not in 0.2.2. See [Zed README](https://github.com/zed-industries/zed/blob/main/crates/gpui/README.md) and [release discussion](https://github.com/zed-industries/zed/discussions/55271).

## 3. Windowing, rendering, and Wayland

GPUI does **not** use winit. The only `winit` mentions in 0.2.2 are comments linking to winit source. Linux backends are hand written:

> "match guess_compositor() { #[cfg(feature = "wayland")] "Wayland" => Rc::new(WaylandClient::new()), #[cfg(feature = "x11")] "X11" => ... }"

Source: [platform.rs](https://docs.rs/crate/gpui/0.2.2/source/src/platform.rs.html). It selects Wayland when `WAYLAND_DISPLAY` is set and X11 only when `DISPLAY` is set.

The Wayland client uses `wayland-client`, `calloop`, `calloop-wayland-source`, XDG shell, text input v3, fractional scale, and xkbcommon directly:

> "use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, delegate_noop, protocol::{wl_buffer, wl_compositor, ...}};"
> "use xkbcommon::xkb::{self, KEYMAP_COMPILE_NO_FLAGS, Keycode};"

Source: [wayland/client.rs](https://docs.rs/crate/gpui/0.2.2/source/src/platform/linux/wayland/client.rs.html).

Rendering is Blade, not wgpu or OpenGL. `BladeContext::new()` calls `gpu::Context::init(...)` ([source](https://docs.rs/crate/gpui/0.2.2/source/src/platform/blade/blade_context.rs.html)). On Linux `blade-graphics 0.7.0` selects its Vulkan backend and depends on `ash`, `ash-window`, and `gpu-alloc-ash` ([Cargo.toml](https://docs.rs/crate/blade-graphics/0.7.0/source/Cargo.toml.html)). `GpuSpecs` documents "The name of the device, as reported by Vulkan" ([source](https://docs.rs/crate/gpui/0.2.2/source/src/gpui.rs.html)).

System libraries needed on Nix:

| Library | Why | Evidence |
| --- | --- | --- |
| `wayland` | Protocol client, dlopened by `wayland-backend` with `client_system` and `dlopen` | [manifest](https://docs.rs/crate/gpui/0.2.2/source/Cargo.toml.orig) |
| `libxkbcommon` | Link flags `-lxkbcommon -lxkbcommon-x11` from the `xkbcommon` crate | local link error, [manifest](https://docs.rs/crate/gpui/0.2.2/source/Cargo.toml.orig) |
| `vulkan-loader` | Blade Vulkan backend and `ash` | [blade manifest](https://docs.rs/crate/blade-graphics/0.7.0/source/Cargo.toml.html) |
| `fontconfig` | `zed-font-kit` uses `source-fontconfig-dlopen`; cosmic-text enables fontdb `fontconfig` | [manifest](https://docs.rs/crate/gpui/0.2.2/source/Cargo.toml.orig) |
| XDG portal, optional | `ashpd` and `oo7` for appearance and file dialogs | [Zed Linux deps](https://github.com/zed-industries/zed/blob/main/script/linux) |

`libglvnd` in the current flake is not a GPUI Linux requirement. Zed's own Linux dependency script lists `libfontconfig-dev`, `libvulkan1`, `libwayland-dev`, and `libxkbcommon-x11-dev`.

Local verification:

1. `cargo check` with `--no-default-features --features wayland` succeeded.
2. Linking failed on the unmodified dev shell with `rust-lld: error: unable to find library
   -lxkbcommon` and `-lxkbcommon-x11`.
3. Adding the libxkbcommon lib directory to `LIBRARY_PATH` linked the example.
4. Running it under `WAYLAND_DEBUG=1` connected to `wayland-1`, bound `wl_compositor`, `xdg_wm_base`,
   and `zwp_text_input_manager_v3`, created an `xdg_toplevel`, and committed 696 frames in 5 seconds
   with no error.
5. `/proc/<pid>/maps` showed `libwayland-client`, `libxkbcommon`, and `libvulkan` loaded.
   `libfontconfig` was not loaded in this simple run because fontdb parses config in Rust.

**Wayland verdict: viable here.** The current flake needs the changes in section 6 first.

## 4. Core concepts and compile-checked skeleton

GPUI owns state in `Entity<T>` handles created with `cx.new` and mutated with `entity.update` or `cx.notify`. `App` is the application context, `Context<T>` is `App` plus an entity, and `Window` is passed to `Render::render` each frame. Views are entities implementing `Render` that return an element tree, usually rooted at `div()`. Elements use a Tailwind-like styling API. Keyboard actions are separate structs bound in key contexts and mapped to keystrokes with `cx.bind_keys`. Focus is explicit via `FocusHandle` and `window.focus`. Sources: [ownership guide](https://docs.rs/crate/gpui/0.2.2/source/src/_ownership_and_data_flow.rs), [key dispatch](https://docs.rs/crate/gpui/0.2.2/source/docs/key_dispatch.md), [hello_world](https://docs.rs/crate/gpui/0.2.2/source/examples/hello_world.rs).

This skeleton compiles against 0.2.2 with the wayland-only feature set. It covers the requested concepts. `Store` stands for `ranki_model::Store`.

```rust
use gpui::{
    App, Application, Bounds, Context, FocusHandle, KeyBinding, Render, SharedString,
    Window, WindowBounds, WindowOptions, actions, div, img, prelude::*, px, rgb, size,
};
actions!(ranki, [Reveal, ToggleTheme]);

struct Root { focus: FocusHandle, label: SharedString, dark: bool, media: SharedString }

impl Root {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus);
        Self { focus, label: "Question".into(), dark: true, media: "front.png".into() }
    }
    fn reveal(&mut self, _: &Reveal, _: &mut Window, cx: &mut Context<Self>) {
        self.label = "Answer".into();
        cx.notify();
    }
    fn toggle_theme(&mut self, _: &ToggleTheme, _: &mut Window, cx: &mut Context<Self>) {
        self.dark = !self.dark;
        cx.notify();
    }
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (bg, fg) = if self.dark {
            (rgb(0x1e1e2e), rgb(0xcdd6f4)) // Catppuccin Mocha
        } else {
            (rgb(0xeff1f5), rgb(0x4c4f69)) // Latte
        };
        div()
            .id("root").track_focus(&self.focus).key_context("ranki")
            .on_action(cx.listener(Self::reveal))
            .on_action(cx.listener(Self::toggle_theme))
            .size_full().flex().bg(bg).text_color(fg)
            .font_family("CCL Nerd Font") // system fonts come from cosmic-text/fontdb
            .child(div().text_2xl().child(self.label.clone()))
            .child(img(self.media.clone()).w(px(240.))) // Anki media
            .child(div().child(format!("{:?}", window.bounds().size)))
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("space", Reveal, Some("ranki")),
            KeyBinding::new("t", ToggleTheme, Some("ranki")),
        ]);
        let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(800.), px(600.))),
                app_id: Some("ranki".into()), ..Default::default()
            },
            |window, cx| cx.new(|cx| Root::new(window, cx)),
        ).unwrap();
        cx.activate(true);
    });
}
```

Font details: `.font_family("CCL Nerd Font")` selects a system family. For explicit fallback, build a `Font` with `FontFallbacks::from_fonts(vec!["CCL Nerd Font".into(), "Noto Sans JP".into()])` and use `.font(font)`. Embedded fonts use `cx.text_system().add_fonts(...)`. Mockup icons are inline Lucide SVG, so use `svg().path(...)` with an `AssetSource`, not a font.

Theme switching: keep `ThemeChoice` in the root entity. For `System`, call `window.appearance()` and register `cx.observe_window_appearance(...)` to call `cx.notify()`. Derive all colors from a `palette(theme)` function returning `Hsla`. GPUI has no eframe `visuals()` replacement.

Images: replace `card_view::ImageCache` with `ImageSource` plus `img()`, an `AssetSource` rooted at `Options::media_dir`, and `with_loading`/`with_fallback` for missing media. Sources: [image example](https://docs.rs/crate/gpui/0.2.2/source/examples/image/image.rs), [image loading](https://docs.rs/crate/gpui/0.2.2/source/examples/image_loading.rs), [font fallbacks](https://docs.rs/crate/gpui/0.2.2/source/src/text_system/font_fallbacks.rs.html), [appearance observer](https://docs.rs/crate/gpui/0.2.2/source/src/app/context.rs.html).

## 5. Rendering the four mockup views

Design inputs: [design system](mockup-design-system.md), [decks and add](mockup-decks-add-spec.md), [learning](mockup-learning-spec.md), [card types](card-types-research.md).

State model: one `Entity<Root>` holding `store: Box<dyn Store>`, `view: View`, options, `decks`, `models`, `stats`, `selected`, queue and current card, `phase: Phase`, `add_draft`, `theme`, toast, and focus handles. Split pure logic into the view modules proposed in the specs, but have each module return `impl IntoElement` or a `Div` instead of drawing into an `egui::Ui`. Keep `Store` ownership in the root entity and call its `&self`/`&mut self` methods inside `cx.update` closures. A single root view avoids cross-window state plumbing.

Element structure:

- Shell: `div().size_full().flex().bg(base)` with a 232px nav `div` and a `flex_1()` main column.
  The 1280x800 artboard maps to `.w(px(..))`, `.h(px(..))`, `.p(px(..))`, `.gap(px(..))`, and
  `.rounded(px(..))`. Taffy provides flex layout, so replace `columns_const` and `Panel::left` with
  nested `div()` rows and columns.
- Decks: nav rail, header, two summary cards at flex ratios 1.6 and 1, then a five-column table
  shell. Deck rows are `div()` with `.pl(px(35.))` indentation, a 2px rule, selected
  `bg(surface0)`, and right aligned counts. Reuse `build_deck_rows`, `sum_leaf_counts`,
  `filter_decks`, and `resolve_model` from the spec.
- Add card: header and stepper, deck and note-type selectors, form and tags, 360px preview rail,
  fixed footer. Fields use a 1.5px border and `.focus(|s| s.border_color(mauve))`. Keyboard text
  input needs the `ElementInputHandler` pattern from gpui's `input` example because 0.2.2 has no
  ready made field with Ranki validation. Preview text uses template substitution as `StyledText`,
  not HTML.
- Learning: 72px header, centered 760x440 stage with `.rounded(px(32.))`, 24px gap, 64px footer.
  Question and answer are conditional children under `Phase`. The separator, answer text, and
  feedback block appear only in `Answer`. The 760x76 primary button becomes a four-column rating
  grid with `.gap(px(14.))` and rating tints. Bind `space`, `1` to `4`, `r`, and `esc` as actions in
  a `study` key context.

Styling: a `theme.rs` module returning `Hsla` colors and shared element builders (`surface`, `primary_button`, `secondary_button`, `keycap`, `rating_button`, `segmented`, `progress_bar`, `focus_ring`). No hex values in view modules. Use exact Catppuccin values and the Latte adaptations from the design system. Keep motion to instant state swaps.

Card rendering: `card_view.rs` must be rewritten from egui `LayoutJob` and `TextureHandle` to `StyledText`/`TextRun`, `div`, `img`, and `svg`. The conditional, cloze, and image-occlusion work in `docs/card-types-research.md` needs custom elements or plain-text fallbacks. Do not embed HTML, CSS, JavaScript, or MathJax in GPUI.

## 6. Migration impact

`crates/ui/Cargo.toml`: replace `eframe` and `egui` with `gpui = { workspace = true }`. Keep `ranki-model` and `ranki-core`. Drop the direct `image` dependency unless a custom loader needs it, since GPUI already depends on `image 0.25`. Replace `theme.rs`, `card_view.rs`, and `app.rs`; `eframe::App` and `egui` types disappear.

Workspace `Cargo.toml`: replace the `eframe` and `egui` entries with `gpui = { version = "=0.2.2", default-features = false, features = ["wayland"] }`, or `gpui.workspace = true` in the UI crate. Keep resolver 3. Review the lockfile: a fresh resolution picked `xattr 0.2.3` with a newer libc and failed on `libc::ENOATTR`; the crates.io lock selection compiled. Avoid unrelated lock churn.

`flake.nix`, exact changes required:

```nix
guiLibs = pkgs.lib.makeLibraryPath [
  pkgs.wayland
  pkgs.libxkbcommon
  pkgs.vulkan-loader          # add: Blade needs Vulkan, libglvnd is not enough
  pkgs.fontconfig             # add: zed-font-kit/cosmic-text font discovery
];

package = craneLib.buildPackage (commonArgs // {
  inherit cargoArtifacts;
  nativeBuildInputs = [ pkgs.makeWrapper pkgs.libxkbcommon.dev ]; # add dev output
  # existing postFixup with wrapProgram and LD_LIBRARY_PATH stays
});

# devShells.default: inputsFrom [package] propagates libxkbcommon.dev.
# Add pkgs.fontconfig and desired font packages to `packages` so
# cosmic-text sees CCL Nerd Font and Noto Sans JP during development.
```

Do not replace `guiLibs` with `libglvnd`; keep it only if something else needs GL. Add `xdg-desktop-portal` only if system theme and file dialogs are required in the Nix shell.

Preserving `ranki_ui::run(store: Box<dyn Store>, options)`: keep the function name and both arguments, move the `Box<dyn Store>` into the root entity in the `open_window` closure, and keep the caller mapping in `crates/bin/src/main.rs`. The return type must change from `eframe::Result<()>` to a local `RunError` or `anyhow::Result<()>`, since `Application::run` takes a callback returning `()`. Capture any `open_window` error and return it after `Application::run`, or log it and quit. All other CLI, core, model, storage, and format code stays unchanged.

## 7. Performance measurement hooks

Other worktrees own `.github/**`, `.tarpaulin.toml`, `crates/bench/**`, `docs/performance.md`, `flake.nix`, and `README.md`. This research touched none of them. Preserve these hooks for GUI benchmarking:

- Keep `focused_elapsed`, `last_tick`, `last_grade_at`, `answered`, and `session_total` timing fields
  and the reveal, grade, and reload transitions. They define the review loop events a benchmark can
  wrap.
- Keep the 120 ms grade debounce and Store call boundaries so action-level timings stay comparable.
- Add a `profiling::scope!` or `tracing` span around root `Render::render` and its four view
  branches. GPUI itself calls `profiling::finish_frame!()` in `Window::draw`
  ([source](https://docs.rs/crate/gpui/0.2.2/source/src/window.rs.html)).
- Keep startup phases separable: `Application::new`, `open_window`, first `render`, first Store load.
  Expose `ranki_ui::StartupStats` if the benchmark crate needs process-readable timings.

## 8. Risks and blockers

1. Pre-1.0 API and stale upstream release. Pin 0.2.2 and expect a rewrite on upgrade.
2. 472 transitive crates versus 136 today. Cold CI builds, audit noise, and coverage cost rise.
3. Vulkan is mandatory. Systems without a Vulkan driver fail in `BladeContext::new`. This machine has
   `/dev/dri/renderD128`, `vulkan-loader`, and lavapipe in use.
4. Nix link failure without `libxkbcommon.dev`; reproduced here.
5. Fonts: mockups use Inter and Noto Sans JP, Ranki requires CCL Nerd Font. fontdb reads fontconfig
   config in Rust; verify CJK and icon glyphs visually before accepting.
6. No HTML/CSS/JS rendering. Complex Anki templates, MathJax, and image occlusion need custom
   elements or remain deferred.
7. Text input is DIY in 0.2.2. The `input` example is 800 lines and must be copied or trimmed.
8. `Store` calls must stay on the GPUI entity thread; check bounds before background tasks.
9. `eframe::Result` is gone at the CLI boundary.
10. Headless tests need the `test-support` feature or `Application::headless()`.

If Wayland or Vulkan fails in a target environment, the smallest fallback is to keep the existing eframe/egui backend behind a UI feature flag and switch only `ranki_ui::run`. Core, model, storage, and format crates do not change.

## 9. Top 10 migration actions

1. Pin `gpui = "=0.2.2"` with wayland-only features.
2. Replace eframe and egui in workspace and UI manifests.
3. Add Vulkan loader, fontconfig, and libxkbcommon dev to flake.nix.
4. Rewrite `ranki_ui::run` around `Application::new().run` and `open_window`.
5. Move `ReviewApp` state into one `Entity<Root>` implementing `Render`.
6. Port theme.rs to Catppuccin `Hsla` helpers and system appearance observers.
7. Port card_view.rs to `StyledText`, `img`, and `svg`.
8. Build the nav, Decks, Add card, and Learning element trees from the specs.
9. Bind Space, 1 to 4, R, and Esc as actions with explicit focus handles.
10. Add frame and startup timing spans, then verify with a Wayland screenshot.

## 10. Sources

Version and features: https://crates.io/crates/gpui/0.2.2 and https://crates.io/api/v1/crates/gpui/0.2.2 API docs and README: https://docs.rs/gpui/0.2.2/gpui/ Crate source: https://docs.rs/crate/gpui/0.2.2/source/ Wayland platform: https://docs.rs/crate/gpui/0.2.2/source/src/platform/linux/wayland/client.rs.html Vulkan renderer: https://docs.rs/crate/gpui/0.2.2/source/src/platform/blade/blade_context.rs.html Blade graphics: https://docs.rs/crate/blade-graphics/0.7.0/source/Cargo.toml.html Zed Linux dependencies: https://github.com/zed-industries/zed/blob/main/script/linux Local evidence: `nix develop --command cargo info gpui`, `cargo tree`, `cargo check`, `cargo build`, and `WAYLAND_DEBUG=1` run of the upstream hello_world example.