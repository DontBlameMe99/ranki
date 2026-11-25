#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
mkdir -p .tmp
mode="${1:-compare}"

if [[ "$mode" != "catalog" && "$mode" != "download" && "$mode" != "compare" ]]; then
  echo "usage: $0 [catalog|download|compare]" >&2
  exit 2
fi

nix develop --command cargo build -p ranki >/dev/null

python_bin="${ANKI_PYTHON:-/nix/store/bya4ansy58az5133kryqp2cvrxbnz8qy-python3-3.14.10/bin/python3}"
site_packages="${ANKI_PYTHONPATH:-/nix/store/vvbz1w27anaw40kp510fxpnrhgncwp76-anki-26.09.4-lib/lib/python3.14/site-packages}"

env -u DISPLAY \
  HOME="$PWD/.tmp/anki-compat/home" \
  XDG_CONFIG_HOME="$PWD/.tmp/anki-compat/home/.config" \
  XDG_DATA_HOME="$PWD/.tmp/anki-compat/home/.local/share" \
  PYTHONPATH="$site_packages" \
  "$python_bin" scripts/anki-compat.py "$mode" "${@:2}"
