#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -d "$ROOT/config" ]]; then
  CONFIG_SOURCE="$ROOT/config"
else
  CONFIG_SOURCE="$ROOT/share/shsynth/config"
fi
STATE_ROOT="${XDG_STATE_HOME:-$HOME/.local/state}"
STATE_DIR="$STATE_ROOT/shsynth"
SHSYNTH_BIN="${SHSYNTH_BIN:-}"

usage() {
  cat <<'EOF'
Usage: setup.sh [--state-dir DIR]

Interactively configure SHSynth's MIDI and JACK routes. The wizard only writes
configuration: it does not start JACK, synth engines, or audible tests.
EOF
}

while (($#)); do
  case "$1" in
    --state-dir)
      [[ $# -ge 2 ]] || { printf '%s\n' '--state-dir requires a path' >&2; exit 2; }
      STATE_DIR="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      printf 'Unknown option: %s\n' "$1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ -z "$SHSYNTH_BIN" ]]; then
  if command -v shsynth >/dev/null 2>&1; then
    SHSYNTH_BIN="$(command -v shsynth)"
  elif [[ -x "$ROOT/target/release/shsynth" ]]; then
    SHSYNTH_BIN="$ROOT/target/release/shsynth"
  else
    printf 'Build or install SHSynth before running this wizard.\n' >&2
    exit 1
  fi
fi

mkdir -p "$STATE_DIR"
RUNTIME_CONFIG="$STATE_DIR/shsynth.conf"
CONTROLLER_CONFIG="$STATE_DIR/controller.conf"

if [[ ! -f "$RUNTIME_CONFIG" || ! -f "$CONTROLLER_CONFIG" ]]; then
  if [[ "$STATE_DIR" == "$STATE_ROOT/shsynth" ]]; then
    "$SHSYNTH_BIN" config init
  else
    [[ -f "$RUNTIME_CONFIG" ]] || cp "$CONFIG_SOURCE/shsynth.conf" "$RUNTIME_CONFIG"
    [[ -f "$CONTROLLER_CONFIG" ]] || cp "$CONFIG_SOURCE/controller.conf" "$CONTROLLER_CONFIG"
  fi
fi

if [[ ! -t 0 ]]; then
  printf 'Created or kept configuration in %s.\n' "$STATE_DIR"
  printf 'Interactive routing was skipped because standard input is not a terminal.\n'
  exit 0
fi

STAMP_BASE="$(date +%Y%m%d-%H%M%S)"
STAMP="$STAMP_BASE"
backup_number=1
while [[ -e "$RUNTIME_CONFIG.bak-$STAMP" || -e "$CONTROLLER_CONFIG.bak-$STAMP" ]]; do
  STAMP="$STAMP_BASE-$backup_number"
  ((backup_number += 1))
done
cp -p "$RUNTIME_CONFIG" "$RUNTIME_CONFIG.bak-$STAMP"
cp -p "$CONTROLLER_CONFIG" "$CONTROLLER_CONFIG.bak-$STAMP"

validate_value() {
  local value=$1
  [[ "$value" != *$'\n'* && "$value" != *'#'* ]] || {
    printf 'Values cannot contain a newline or #.\n' >&2
    return 1
  }
}

replace_values() {
  local file=$1 key=$2
  shift 2
  local tmp
  tmp="$(mktemp "${file}.tmp.XXXXXX")"
  awk -v wanted="$key" '
    {
      line=$0
      sub(/^[[:space:]]*/, "", line)
      if (index(line, wanted "=") != 1) print $0
    }
  ' "$file" >"$tmp"
  local value
  for value in "$@"; do
    validate_value "$value"
    printf '%s=%s\n' "$key" "$value" >>"$tmp"
  done
  chmod --reference="$file" "$tmp" 2>/dev/null || true
  mv "$tmp" "$file"
}

ask_yes_no() {
  local prompt=$1 default=$2 answer suffix
  if [[ "$default" == yes ]]; then suffix='[Y/n]'; else suffix='[y/N]'; fi
  while true; do
    read -r -p "$prompt $suffix " answer
    answer="${answer:-$default}"
    case "${answer,,}" in
      y|yes) return 0 ;;
      n|no) return 1 ;;
      *) printf 'Please answer yes or no.\n' >&2 ;;
    esac
  done
}

CHOSEN=''
choose_value() {
  local prompt=$1 allow_off=$2
  shift 2
  local -a values=("$@")
  local i answer manual
  printf '\n%s\n' "$prompt"
  for i in "${!values[@]}"; do
    printf '  %d) %s\n' "$((i + 1))" "${values[$i]}"
  done
  [[ "$allow_off" == yes ]] && printf '  0) Disable / none\n'
  printf '  m) Enter an exact port match manually\n'
  while true; do
    read -r -p '> ' answer
    if [[ "$answer" == m || "$answer" == M ]]; then
      read -r -p 'Exact value: ' manual
      validate_value "$manual" || continue
      [[ -n "$manual" ]] || { printf 'A value is required.\n' >&2; continue; }
      CHOSEN="$manual"
      return 0
    fi
    if [[ "$allow_off" == yes && "$answer" == 0 ]]; then
      CHOSEN=''
      return 0
    fi
    if [[ "$answer" =~ ^[0-9]+$ ]] && ((answer >= 1 && answer <= ${#values[@]})); then
      CHOSEN="${values[answer - 1]}"
      return 0
    fi
    printf 'Choose a listed number%s or m.\n' "$([[ "$allow_off" == yes ]] && printf ', 0')" >&2
  done
}

alsa_ports() {
  local direction=$1
  command -v aconnect >/dev/null 2>&1 || return 0
  aconnect "$direction" -l 2>/dev/null | awk -F "'" '
    /^client [0-9]+:/ { client=$2; next }
    /^[[:space:]]+[0-9]+ / {
      port=$2
      if (client != "System" && client != "Midi Through" &&
          client !~ /^SHSynth/ && client !~ /^shs-/ && port != "") {
        print client " " port
      }
    }
  ' | awk '!seen[tolower($0)]++'
}

jack_audio_ports() {
  local wanted_property=$1
  command -v jack_lsp >/dev/null 2>&1 || return 0
  jack_lsp -p -t 2>/dev/null | awk -v wanted="$wanted_property" '
    function flush() {
      if (port != "" && properties ~ wanted && properties ~ /physical/ && type ~ /audio/) print port
    }
    /^[^[:space:]]/ { flush(); port=$0; properties=""; type=""; next }
    /^[[:space:]]+properties:/ { properties=tolower($0); next }
    /^[[:space:]]+/ { type=tolower($0); next }
    END { flush() }
  '
}

alsa_cards() {
  [[ -r /proc/asound/cards ]] || return 0
  awk '
    /^[[:space:]]*[0-9]+[[:space:]]+\[[^]]+\]:/ {
      line=$0
      sub(/^[[:space:]]*[0-9]+[[:space:]]+\[/, "", line)
      split(line, parts, "]")
      id=parts[1]
      gsub(/[[:space:]]/, "", id)
      description=parts[2]
      sub(/^[^:]*:[[:space:]]*/, "", description)
      print id " (" description ")"
    }
  ' /proc/asound/cards
}

printf 'SHSynth hardware setup\n'
printf 'No audio server, synth engine, or audible test will be started.\n'

mapfile -t cards < <(alsa_cards)
if ((${#cards[@]})) && ask_yes_no 'Select the ALSA card JACK should use on its next start?' no; then
  choose_value 'JACK audio interface' no "${cards[@]}"
  card="${CHOSEN%% (*}"
  [[ "$card" =~ ^[A-Za-z0-9_-]+$ ]] || {
    printf 'Invalid ALSA card identifier: %s\n' "$card" >&2
    exit 1
  }
  read -r -p 'Sample rate [48000]: ' sample_rate
  read -r -p 'Period size in frames [256]: ' period_size
  read -r -p 'Periods per buffer [3]: ' periods
  sample_rate="${sample_rate:-48000}"
  period_size="${period_size:-256}"
  periods="${periods:-3}"
  [[ "$sample_rate" =~ ^[0-9]+$ && "$period_size" =~ ^[0-9]+$ && "$periods" =~ ^[0-9]+$ ]] || {
    printf 'JACK timing values must be positive integers.\n' >&2
    exit 1
  }
  if [[ -f "$HOME/.jackdrc" ]]; then
    cp -p "$HOME/.jackdrc" "$HOME/.jackdrc.bak-$STAMP"
  fi
  printf 'jackd -R -d alsa -d hw:%s -r %s -p %s -n %s\n' \
    "$card" "$sample_rate" "$period_size" "$periods" >"$HOME/.jackdrc"
  printf 'Wrote %s; restart JACK yourself when it is safe to do so.\n' "$HOME/.jackdrc"
fi

mapfile -t midi_sources < <(alsa_ports -i)
choose_value 'Controller MIDI input (notes and physical controls)' no "${midi_sources[@]}"
controller_input="$CHOSEN"
replace_values "$RUNTIME_CONFIG" midi.autoconnect true
replace_values "$RUNTIME_CONFIG" midi.input "$controller_input"
replace_values "$CONTROLLER_CONFIG" input "$controller_input"
if [[ -n "${SHSYNTH_PRESET_DIR:-}" ]]; then
  replace_values "$RUNTIME_CONFIG" synthv1.presets "$SHSYNTH_PRESET_DIR"
fi
replace_values "$RUNTIME_CONFIG" capture.directory \
  "${XDG_DATA_HOME:-$HOME/.local/share}/shsynth/recordings"

mapfile -t playback_ports < <(jack_audio_ports input)
if ((${#playback_ports[@]})); then
  choose_value 'Left JACK playback destination' yes "${playback_ports[@]}"
else
  choose_value 'Left JACK playback destination (JACK is not currently exposing one)' yes
fi
left_playback="$CHOSEN"
if [[ -n "$left_playback" ]]; then
  if ((${#playback_ports[@]})); then
    choose_value 'Right JACK playback destination' no "${playback_ports[@]}"
  else
    choose_value 'Right JACK playback destination' no
  fi
  right_playback="$CHOSEN"
  [[ "$left_playback" != "$right_playback" ]] || {
    printf 'Left and right playback destinations must be different.\n' >&2
    exit 1
  }
  replace_values "$RUNTIME_CONFIG" audio.autoconnect true
  replace_values "$RUNTIME_CONFIG" audio.output "$left_playback" "$right_playback"
else
  replace_values "$RUNTIME_CONFIG" audio.autoconnect false
  replace_values "$RUNTIME_CONFIG" audio.output
fi

mapfile -t capture_ports < <(jack_audio_ports output)
if ask_yes_no 'Configure a stereo JACK recording input?' no; then
  if ((${#capture_ports[@]})); then
    choose_value 'Left JACK capture source' no "${capture_ports[@]}"
  else
    choose_value 'Left JACK capture source' no
  fi
  left_capture="$CHOSEN"
  if ((${#capture_ports[@]})); then
    choose_value 'Right JACK capture source' no "${capture_ports[@]}"
  else
    choose_value 'Right JACK capture source' no
  fi
  right_capture="$CHOSEN"
  [[ "$left_capture" != "$right_capture" ]] || {
    printf 'Left and right capture sources must be different.\n' >&2
    exit 1
  }
  read -r -p 'Recorder source label [Soundcard]: ' capture_label
  capture_label="${capture_label:-Soundcard}"
  validate_value "$capture_label"
  [[ "$capture_label" != *'|'* ]] || { printf 'The label cannot contain |.\n' >&2; exit 1; }
  replace_values "$RUNTIME_CONFIG" capture.input \
    "$capture_label|$left_capture|$right_capture"
fi

mapfile -t midi_destinations < <(alsa_ports -o)
if ask_yes_no 'Configure an external hardware MIDI output?' no; then
  choose_value 'External MIDI destination' no "${midi_destinations[@]}"
  replace_values "$RUNTIME_CONFIG" external_midi.enabled true
  replace_values "$RUNTIME_CONFIG" external_midi.output "$CHOSEN"
else
  replace_values "$RUNTIME_CONFIG" external_midi.enabled false
fi

printf '\nConfiguration complete.\n'
printf '  Runtime:    %s\n' "$RUNTIME_CONFIG"
printf '  Controller: %s\n' "$CONTROLLER_CONFIG"
printf '  Backups:    *.bak-%s\n' "$STAMP"
printf 'Run `shr doctor` after JACK is running, then edit either file for any finer routing.\n'
