#!/usr/bin/env bash
set -euo pipefail

SCRIPT_PATH="$(readlink -f "${BASH_SOURCE[0]}")"
ROOT="$(cd "$(dirname "$SCRIPT_PATH")/.." && pwd)"
USER_DIR="${SHSYNTH_USER_DIR:-$ROOT/user}"

export XDG_STATE_HOME="$USER_DIR/state"
export XDG_DATA_HOME="$USER_DIR/data"
export SHSYNTH_PRESET_DIR="$USER_DIR/presets/synthv1"
export SHSYNTH_LOOP_INBOX="$USER_DIR/data/shsynth/loop-inbox"

mkdir -p \
  "$XDG_STATE_HOME/shsynth" \
  "$XDG_DATA_HOME/shsynth" \
  "$XDG_DATA_HOME/shsynth/demos" \
  "$XDG_DATA_HOME/shsynth/songs" \
  "$SHSYNTH_LOOP_INBOX" \
  "$SHSYNTH_PRESET_DIR"

for preset in "$ROOT"/presets/synthv1/*.synthv1; do
  destination="$SHSYNTH_PRESET_DIR/${preset##*/}"
  [[ -e "$destination" ]] || cp "$preset" "$destination"
done

while IFS= read -r loop_name || [[ -n "$loop_name" ]]; do
  [[ -n "$loop_name" && "$loop_name" != \#* ]] || continue
  [[ "$loop_name" != */* && "$loop_name" == *.wav ]] || {
    printf 'Unsafe starter-loop manifest entry: %s\n' "$loop_name" >&2
    exit 1
  }
  source="$ROOT/loops/$loop_name"
  destination="$SHSYNTH_LOOP_INBOX/$loop_name"
  [[ -f "$source" ]] || { printf 'Starter loop not found: %s\n' "$source" >&2; exit 1; }
  [[ -e "$destination" ]] || install -m644 "$source" "$destination"
done <"$ROOT/loops/cleared-loops.txt"

"$ROOT/scripts/generate_demo_songs.py"
while IFS= read -r demo; do
  filename="${demo##*/}"
  source="$ROOT/$demo"
  destination="$XDG_DATA_HOME/shsynth/demos/$filename"
  [[ -e "$destination" ]] || install -m644 "$source" "$destination"
  if [[ "$filename" == *.shsong ]]; then
    destination="$XDG_DATA_HOME/shsynth/songs/$filename"
    [[ -e "$destination" ]] || install -m644 "$source" "$destination"
  fi
done < <("$ROOT/scripts/generate_demo_songs.py" --files)

DEBUG_BIN="$ROOT/target/debug/shr"

if [[ -n "${SHSYNTH_BIN:-}" ]]; then
  [[ -x "$SHSYNTH_BIN" ]] || {
    printf 'SHSYNTH_BIN is not executable: %s\n' "$SHSYNTH_BIN" >&2
    exit 1
  }
elif [[ -x "$DEBUG_BIN" ]]; then
  SHSYNTH_BIN="$DEBUG_BIN"
else
  printf 'Build the SHR-DAW debug binary first.\n' >&2
  exit 1
fi

if [[ ! -f "$XDG_STATE_HOME/shsynth/shsynth.conf" ]]; then
  printf 'Run scripts/setup-local.sh before starting SHR-DAW.\n' >&2
  exit 1
fi

exec "$SHSYNTH_BIN" "$@"
