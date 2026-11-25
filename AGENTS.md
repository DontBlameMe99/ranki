# Agent instructions

## Pull requests with code changes

Any pull request that changes code must show the result. When the change is
visible, embed two images in the PR description: a **Before** screenshot and an
**After** screenshot. Label both images so reviewers know which is which.

CI treats every change to a source or workflow file as a code change and fails
the PR when the description has fewer than two embedded images.
Documentation-only PRs are exempt.

## Checks

Run the same checks as CI before finishing UI or storage work:

```bash
nix develop --command cargo test --workspace --all-targets --no-fail-fast
nix develop --command cargo clippy --workspace --all-features --all-targets -- -D warnings
nix develop --command cargo fmt --all -- --check
```

## Visual verification

Never open `ranki gui` on the user's Wayland session. Use the private headless
session instead:

```bash
scripts/visual-check.sh
```

The default mode builds the app, creates a temporary collection under `.tmp`,
runs a Wayland compositor with a per-run `XDG_RUNTIME_DIR` inside the host
runtime directory, captures screenshots, and stops the compositor. Inspect the
printed screenshot paths, then remove the temporary directory.

To drive the app interactively, run `scripts/visual-check.sh session`. Source the
printed env file, then use `start_ranki gui`, `focus_ranki`, `key`, `shot`,
`stop_ranki`, and `stop_visual`. `key space`, `key enter`, and `key 3` work;
`type_text` sends literal text.

## Input safety

The hidden sway session has no pointer device. `WLR_LIBINPUT_NO_DEVICES=1` keeps
the user's physical devices out, and the headless backend does not accept virtual
pointer events. Clicks and scrolls cannot be delivered through that display.

`ydotool` and `xdotool` do not use `WAYLAND_DISPLAY`. `ydotool` talks to the global
daemon at `/run/ydotoold/socket`, whose uinput device is part of the user's real
session. Running it through `hidden` still moves the user's mouse. Umbriel tools
also act on the real compositor. `wlrctl` virtual pointers do not work on the
headless backend, so do not substitute them.

Never use Umbriel input or screenshot tools, `ydotool`, `xdotool`, or any system-wide
input tool for Ranki. Use `key`, `type_text`, `shot`, and the `RANKI_UI_DEV_*` hooks.
For behavior that needs a click, add or use a dev hook so the action runs inside the
app, then screenshot it. The harness sets `YDOTOOL_SOCKET` to a private path and masks
`ydotool` and `xdotool` in the sourced session so stray calls fail without reaching
the desktop.
