#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
mode="${1:-check}"

if [[ "$mode" != "check" && "$mode" != "session" ]]; then
  echo "usage: $0 [check|session]" >&2
  exit 2
fi

mkdir -p .tmp
dir="$(mktemp -d -p "$PWD/.tmp" ranki-visual.XXXXXX)"
host_runtime="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
runtime="$host_runtime/ranki-visual.$$"
mkdir -p "$runtime" "$dir/config/ranki" "$dir/collection.media"
chmod 700 "$runtime"

repo="$PWD"
nix develop --command cargo build -p ranki >/dev/null

bin="$repo/target/debug/ranki"
collection="$dir/collection.anki2"
cat > "$dir/config/ranki/config.toml" <<EOF
[general]
collection = "$collection"
media_dir = "$dir/collection.media"
[ui]
theme = "dark"
reduce_motion = true
EOF
seed_env=(env XDG_CONFIG_HOME="$dir/config")
"${seed_env[@]}" "$bin" --no-sync --collection "$collection" deck-create 'Visual Deck' >/dev/null
deck_id="$("${seed_env[@]}" "$bin" --no-sync --collection "$collection" decks | awk -F '\t' '$2 == "Visual Deck" {print $1}')"
"${seed_env[@]}" "$bin" --no-sync --collection "$collection" add-note --model Basic --deck "$deck_id" \
  --field 'Question?' --field 'Answer!' >/dev/null
"${seed_env[@]}" "$bin" --no-sync --collection "$collection" add-note --model Cloze --deck "$deck_id" \
  --field 'The {{c1::capital}} is Paris.' --field 'Remember it' >/dev/null

cat > "$dir/sway.conf" <<'EOF'
output * resolution 1280x800
output * bg #1e1e2e solid_color
seat seat0 hide_cursor 1000
EOF

nix_packages=(nixpkgs#sway nixpkgs#wtype nixpkgs#grim nixpkgs#mesa)

nix develop --command nix shell "${nix_packages[@]}" --command true
terminate_group() {
  local pid="$1"
  kill -TERM -- "-$pid" 2>/dev/null || true
  for _ in {1..10}; do
    kill -0 -- "-$pid" 2>/dev/null || return 0
    sleep 0.1
  done
  kill -KILL -- "-$pid" 2>/dev/null || true
}

active_app_pid=''
setsid env XDG_RUNTIME_DIR="$runtime" \
  WLR_BACKENDS=headless \
  WLR_RENDERER=gles2 \
  WLR_LIBINPUT_NO_DEVICES=1 \
  nix develop --command nix shell "${nix_packages[@]}" --command sway -c "$dir/sway.conf" \
  >"$dir/sway.log" 2>&1 &
sway_pid=$!

cleanup() {
  if [[ -n "$active_app_pid" ]]; then
    terminate_group "$active_app_pid"
  fi
  terminate_group "$sway_pid"
  rm -rf "$runtime"
}
trap cleanup EXIT

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
  env XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY="$display" SWAYSOCK="$ipc" \
    nix develop --command nix shell "${nix_packages[@]}" --command "$@"
}

if [[ "$mode" == "session" ]]; then
  {
    printf 'export VISUAL_DIR=%q\n' "$dir"
    printf 'export VISUAL_REPO=%q\n' "$repo"
    printf 'export VISUAL_RUNTIME=%q\n' "$runtime"
    printf 'export VISUAL_DISPLAY=%q\n' "$display"
    printf 'export VISUAL_CONFIG=%q\n' "$dir/config"
    printf 'export VISUAL_BIN=%q\n' "$bin"
    printf 'export VISUAL_SWAY_SOCK=%q\n' "$ipc"
    printf 'export VISUAL_SWAY_PGID=%q\n' "$sway_pid"
    printf 'VISUAL_PACKAGES=(nixpkgs#sway nixpkgs#wtype nixpkgs#grim nixpkgs#mesa)\n'
    cat <<'EOF'

visual_terminate() {
  local pid="$1"
  kill -TERM -- "-$pid" 2>/dev/null || true
  for _ in {1..10}; do
    kill -0 -- "-$pid" 2>/dev/null || return 0
    sleep 0.1
  done
  kill -KILL -- "-$pid" 2>/dev/null || true
}

hidden() {
  env XDG_RUNTIME_DIR="$VISUAL_RUNTIME" WAYLAND_DISPLAY="$VISUAL_DISPLAY" SWAYSOCK="$VISUAL_SWAY_SOCK" \
    nix develop "$VISUAL_REPO" --command nix shell "${VISUAL_PACKAGES[@]}" --command "$@"
}

start_ranki() {
  setsid env XDG_CONFIG_HOME="$VISUAL_CONFIG" RANKI_REDUCE_MOTION=1 \
    nix develop "$VISUAL_REPO" --command nix shell "${VISUAL_PACKAGES[@]}" --command env \
    XDG_RUNTIME_DIR="$VISUAL_RUNTIME" WAYLAND_DISPLAY="$VISUAL_DISPLAY" \
    "$VISUAL_BIN" "$@" >"$VISUAL_DIR/app.log" 2>&1 &
  echo $! > "$VISUAL_DIR/app.pid"
  echo "ranki pid $!"
  ready=0
  for _ in {1..60}; do
    if hidden swaymsg -t get_tree 2>/dev/null | grep -q '"app_id": "ranki"'; then
      ready=1
      break
    fi
    if [[ -f "$VISUAL_DIR/app.pid" ]] && ! kill -0 "$(cat "$VISUAL_DIR/app.pid")" 2>/dev/null; then
      break
    fi
    sleep 0.25
  done
  if [[ "$ready" -ne 1 ]]; then
    cat "$VISUAL_DIR/app.log" >&2
    return 1
  fi
  sleep 0.3
}

stop_ranki() {
  [[ -f "$VISUAL_DIR/app.pid" ]] || return 0
  visual_terminate "$(cat "$VISUAL_DIR/app.pid")"
  rm -f "$VISUAL_DIR/app.pid"
}

shot() {
  hidden grim "${1:-$VISUAL_DIR/manual.png}"
}

focus_ranki() {
  hidden swaymsg '[app_id=ranki] focus' >/dev/null
  sleep 0.2
}

key() {
  case "$1" in
    space|Space|SPACE) hidden wtype ' ' ;;
    enter|Enter|return|Return) hidden wtype $'\n' ;;
    tab|Tab) hidden wtype $'\t' ;;
    *) hidden wtype "$1" ;;
  esac
}

type_text() {
  hidden wtype -- "$@"
}


stop_visual() {
  stop_ranki
  visual_terminate "$VISUAL_SWAY_PGID"
  rm -rf "$VISUAL_RUNTIME"
}
EOF
  } > "$dir/env.sh"

  trap - EXIT
  echo "Interactive visual session ready."
  echo "Session env: $dir/env.sh"
  echo "Source it with: source $dir/env.sh"
  echo "Then use: start_ranki gui; focus_ranki; key space; shot; stop_ranki; stop_visual"
  exit 0
fi

capture() {
  local name="$1"
  local view="${2:-}"
  local reveal="${3:-}"
  local app_pid
  local -a app_env=(env XDG_CONFIG_HOME="$dir/config" RANKI_REDUCE_MOTION=1)
  if [[ -n "$view" ]]; then
    app_env+=("RANKI_UI_DEV_VIEW=$view")
  fi
  if [[ -n "$reveal" ]]; then
    app_env+=("RANKI_UI_DEV_REVEAL=1")
  fi
  setsid "${app_env[@]}" \
    nix develop --command nix shell "${nix_packages[@]}" --command env \
    XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY="$display" \
    "$bin" gui >"$dir/$name.log" 2>&1 &
  app_pid=$!
  active_app_pid=$app_pid
  ready=0
  for _ in {1..60}; do
    if hidden swaymsg -t get_tree 2>/dev/null | grep -q '"app_id": "ranki"'; then
      ready=1
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

capture decks
capture question question
capture answer answer 1
capture add add

hidden swaymsg output '*' resolution 900x600 >/dev/null
capture small question

printf 'Screenshots:\n'
printf '%s\n' "$dir"/*.png
printf 'Temporary directory: %s\n' "$dir"
