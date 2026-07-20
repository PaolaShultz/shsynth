#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
USER_DIR="${SHSYNTH_USER_DIR:-$ROOT/user}"

export XDG_STATE_HOME="$USER_DIR/state"
export XDG_DATA_HOME="$USER_DIR/data"
export SHSYNTH_PRESET_DIR="$USER_DIR/presets/synthv1"
export SHSYNTH_LOOP_INBOX="$USER_DIR/data/shsynth/loop-inbox"
export SHSYNTH_BIN="${SHSYNTH_BIN:-$ROOT/target/debug/shr}"

if [[ ! -x "$SHSYNTH_BIN" ]]; then
  printf 'Build this checkout debug binary first.\n' >&2
  exit 1
fi

mkdir -p "$SHSYNTH_PRESET_DIR"

for preset in "$ROOT"/presets/synthv1/*.synthv1; do
  destination="$SHSYNTH_PRESET_DIR/${preset##*/}"
  [[ -e "$destination" ]] || cp "$preset" "$destination"
done

exec "$ROOT/scripts/setup.sh" --state-dir "$XDG_STATE_HOME/shsynth" "$@"
