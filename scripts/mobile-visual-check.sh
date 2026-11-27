#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

mkdir -p .tmp
dir="$(mktemp -d -p "$PWD/.tmp" ranki-mobile-visual.XXXXXX)"
host_runtime="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
runtime="$host_runtime/ranki-mobile-visual.$$"
data="$dir/data"
config="$dir/config"
collection="$data/ranki/collection.anki2"
mkdir -p "$runtime" "$config/ranki" "$data/ranki/collection.media"
chmod 700 "$runtime"
active_app_pid=''
sway_pid=''

terminate_group() {
  local pid="$1"
  kill -TERM -- "-$pid" 2>/dev/null || true
  for _ in {1..10}; do
    kill -0 -- "-$pid" 2>/dev/null || return 0
    sleep 0.1
  done
  kill -KILL -- "-$pid" 2>/dev/null || true
}

cleanup() {
  if [[ -n "$active_app_pid" ]]; then
    terminate_group "$active_app_pid"
  fi
  if [[ -n "$sway_pid" ]]; then
    terminate_group "$sway_pid"
  fi
  rm -rf "$runtime"
}
trap cleanup EXIT

mkdir -p "$dir/bin"
cat > "$dir/bin/ydotool" <<'SHIM'
#!/bin/sh
echo "ydotool uses the global input daemon and would move the real mouse." >&2
echo "The mobile harness screenshots a private headless sway session." >&2
exit 127
SHIM
cat > "$dir/bin/xdotool" <<'SHIM'
#!/bin/sh
echo "xdotool targets the real X display, not the hidden sway session." >&2
exit 127
SHIM
chmod +x "$dir/bin/ydotool" "$dir/bin/xdotool"

repo="$PWD"
target_dir="${CARGO_TARGET_DIR:-$repo/target}"
host_target="$(nix develop --command rustc -vV | sed -n 's/^host: //p')"
nix develop --command env CARGO_TARGET_DIR="$target_dir" cargo build --target "$host_target" -p ranki -p ranki-mobile >/dev/null
seed_bin="$target_dir/$host_target/debug/ranki"
mobile_bin="$target_dir/$host_target/debug/ranki-mobile"

cat > "$config/ranki/config.toml" <<EOF
[ui]
theme = "dark"
reduce_motion = true
EOF

seed_env=(env RANKI_DATA_DIR="$data/ranki" XDG_CONFIG_HOME="$config" XDG_DATA_HOME="$data")
"${seed_env[@]}" "$seed_bin" --no-sync deck-create 'Visual Deck' >/dev/null
deck_id="$("${seed_env[@]}" "$seed_bin" --no-sync decks \
  | awk -F '\t' '$2 == "Visual Deck" {print $1}')"
"${seed_env[@]}" "$seed_bin" --no-sync add-note --model Basic --deck "$deck_id" \
  --field 'Question?' --field 'Answer!' >/dev/null
"${seed_env[@]}" "$seed_bin" --no-sync add-note --model Cloze --deck "$deck_id" \
  --field 'The {{c1::capital}} is Paris.' --field 'Remember it' >/dev/null
[[ -f "$collection" ]]

cat > "$dir/sway.conf" <<'EOF'
output * resolution 390x844
output * bg #1e1e2e solid_color
seat seat0 hide_cursor 1000
EOF

nix_packages=(nixpkgs#sway nixpkgs#grim nixpkgs#mesa)
nix develop --command nix shell "${nix_packages[@]}" --command true

setsid env XDG_RUNTIME_DIR="$runtime" \
  WLR_BACKENDS=headless \
  WLR_RENDERER=gles2 \
  WLR_LIBINPUT_NO_DEVICES=1 \
  nix develop --command nix shell "${nix_packages[@]}" --command sway -c "$dir/sway.conf" \
  >"$dir/sway.log" 2>&1 &
sway_pid=$!

display=''
ipc=''
for _ in {1..50}; do
  display="$(find "$runtime" -maxdepth 1 -name 'wayland-*' ! -name '*.lock' -printf '%f\n' 2>/dev/null | head -1)"
  ipc="$(find "$runtime" -maxdepth 1 -name 'sway-ipc.*.sock' -print -quit 2>/dev/null)"
  [[ -n "$display" && -n "$ipc" ]] && break
  sleep 0.1
done
if [[ -z "$display" || -z "$ipc" ]]; then
  cat "$dir/sway.log" >&2
  exit 1
fi

hidden() {
  env -u DISPLAY \
    XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY="$display" SWAYSOCK="$ipc" \
    YDOTOOL_SOCKET="$runtime/ydotool.sock" \
    nix develop --command nix shell "${nix_packages[@]}" --command "$@"
}

capture() {
  local name="$1"
  local view="$2"
  local reveal="${3:-0}"
  local app_pid

  setsid env XDG_CONFIG_HOME="$config" XDG_DATA_HOME="$data" \
    RANKI_DATA_DIR="$data/ranki" RANKI_CONFIG_DIR="$config/ranki" \
    RANKI_MOBILE_DEV_VIEW="$view" RANKI_MOBILE_DEV_REVEAL="$reveal" \
    nix develop --command nix shell "${nix_packages[@]}" --command env \
    XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY="$display" \
    "$mobile_bin" >"$dir/$name.log" 2>&1 &
  app_pid=$!
  active_app_pid=$app_pid

  local ready=0
  for _ in {1..60}; do
    if hidden swaymsg -t get_tree 2>/dev/null \
      | grep -q '"app_id": "ranki-mobile"'; then
      ready=1
      break
    fi
    if ! kill -0 "$app_pid" 2>/dev/null; then
      break
    fi
    sleep 0.25
  done
  if [[ "$ready" -ne 1 ]]; then
    cat "$dir/$name.log" >&2
    return 1
  fi
  sleep 0.3
  hidden grim "$dir/$name.png"
  terminate_group "$app_pid"
  wait "$app_pid" 2>/dev/null || true
  active_app_pid=''
}

capture decks decks
capture question review
capture answer review 1
capture add add
capture browse browse
capture settings settings

printf 'Screenshots:\n'
printf '%s\n' "$dir"/*.png
printf 'Temporary directory: %s\n' "$dir"
