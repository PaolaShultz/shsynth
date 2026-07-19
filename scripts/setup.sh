#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -d "$ROOT/config" ]]; then
  CONFIG_SOURCE="$ROOT/config"
  PROFILE_SOURCE="$ROOT/midi-devices"
  LOOP_SOURCE="$ROOT/loops"
else
  CONFIG_SOURCE="$ROOT/share/shsynth/config"
  PROFILE_SOURCE="$ROOT/share/shsynth/midi-devices"
  LOOP_SOURCE="$ROOT/share/shsynth/loops"
fi
STATE_ROOT="${XDG_STATE_HOME:-$HOME/.local/state}"
DATA_ROOT="${XDG_DATA_HOME:-$HOME/.local/share}"
STATE_DIR="$STATE_ROOT/shsynth"
SHSYNTH_BIN="${SHSYNTH_BIN:-}"

usage() {
  cat <<'EOF'
Usage: setup.sh [--state-dir DIR]

Install starter loops and interactively configure SHR-DAW's display, MIDI, and
JACK routes. The wizard does not start JACK, synth engines, or audible tests.
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
  if [[ -x "$ROOT/target/release/shr" ]]; then
    SHSYNTH_BIN="$ROOT/target/release/shr"
  elif command -v shr >/dev/null 2>&1; then
    SHSYNTH_BIN="$(command -v shr)"
  else
    printf 'Build or install SHR-DAW before running this wizard.\n' >&2
    exit 1
  fi
fi

mkdir -p "$STATE_DIR"
RUNTIME_CONFIG="$STATE_DIR/shsynth.conf"
CONTROLLER_CONFIG="$STATE_DIR/controller.conf"
RUNTIME_CREATED=false

if [[ ! -f "$RUNTIME_CONFIG" || ! -f "$CONTROLLER_CONFIG" ]]; then
  [[ -f "$RUNTIME_CONFIG" ]] || RUNTIME_CREATED=true
  if [[ "$STATE_DIR" == "$STATE_ROOT/shsynth" ]]; then
    "$SHSYNTH_BIN" config init
  else
    [[ -f "$RUNTIME_CONFIG" ]] || cp "$CONFIG_SOURCE/shsynth.conf" "$RUNTIME_CONFIG"
    [[ -f "$CONTROLLER_CONFIG" ]] || cp "$CONFIG_SOURCE/controller.conf" "$CONTROLLER_CONFIG"
  fi
fi

if [[ -t 0 ]]; then
  STAMP_BASE="$(date +%Y%m%d-%H%M%S)"
  STAMP="$STAMP_BASE"
  backup_number=1
  while [[ -e "$RUNTIME_CONFIG.bak-$STAMP" || -e "$CONTROLLER_CONFIG.bak-$STAMP" ]]; do
    STAMP="$STAMP_BASE-$backup_number"
    ((backup_number += 1))
  done
  cp -p "$RUNTIME_CONFIG" "$RUNTIME_CONFIG.bak-$STAMP"
  cp -p "$CONTROLLER_CONFIG" "$CONTROLLER_CONFIG.bak-$STAMP"
fi

validate_value() {
  local value=$1
  [[ "$value" != *$'\n'* && "$value" != *$'\r'* ]] || {
    printf 'Values cannot contain a newline or carriage return.\n' >&2
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

config_value() {
  local file=$1 key=$2
  awk -v wanted="$key" '
    {
      line=$0
      sub(/^[[:space:]]*/, "", line)
      if (index(line, wanted "=") == 1) value=substr(line, length(wanted) + 2)
    }
    END { print value }
  ' "$file"
}

expand_home_path() {
  local value=$1 tilde='~'
  case "$value" in
    "$tilde") printf '%s\n' "$HOME" ;;
    "$tilde/"*) printf '%s/%s\n' "$HOME" "${value#"$tilde/"}" ;;
    *) printf '%s\n' "$value" ;;
  esac
}

seed_public_loops() {
  local destination=$1 manifest="$LOOP_SOURCE/cleared-loops.txt" loop_name source
  [[ -f "$manifest" ]] || {
    printf 'Starter-loop manifest not found: %s\n' "$manifest" >&2
    return 1
  }
  mkdir -p "$destination"
  while IFS= read -r loop_name || [[ -n "$loop_name" ]]; do
    [[ -n "$loop_name" && "$loop_name" != \#* ]] || continue
    [[ "$loop_name" != */* && "$loop_name" == *.wav ]] || {
      printf 'Unsafe starter-loop manifest entry: %s\n' "$loop_name" >&2
      return 1
    }
    source="$LOOP_SOURCE/$loop_name"
    [[ -f "$source" ]] || {
      printf 'Starter loop not found: %s\n' "$source" >&2
      return 1
    }
    [[ -e "$destination/$loop_name" ]] || install -m644 "$source" "$destination/$loop_name"
  done <"$manifest"
}

install_private_musicradar_loops() {
  local destination=$1 rate=$2
  local url='https://cdn.mos.musicradar.com/audio/samples/sampleradar-80s-pop-drums-samples.zip'
  local temp_dir archive bpm source output
  for dependency in curl unzip sox; do
    command -v "$dependency" >/dev/null 2>&1 || {
      printf 'Optional loop download needs %s.\n' "$dependency" >&2
      return 1
    }
  done
  temp_dir="$(mktemp -d /tmp/shr-private-loops.XXXXXX)"
  archive="$temp_dir/80s-pop-drums.zip"
  if ! curl --fail --location --retry 2 --output "$archive" "$url"; then
    find "$temp_dir" -type f -delete
    rmdir "$temp_dir"
    return 1
  fi
  mkdir -p "$destination"
  for bpm in 85 110 120 140; do
    output="$destination/private-80s-pop-${bpm}.wav"
    [[ -e "$output" ]] && continue
    source="$temp_dir/source-${bpm}.wav"
    if ! unzip -p "$archive" \
      "80s Pop Drums/${bpm}bpm/*Beat*-01.wav" >"$source"; then
      printf 'Could not extract the %s BPM loop from the downloaded pack.\n' "$bpm" >&2
      find "$temp_dir" -type f -delete
      rmdir "$temp_dir"
      return 1
    fi
    if ! sox "$source" -b 24 -r "$rate" "$temp_dir/output-${bpm}.wav" rate -v; then
      find "$temp_dir" -type f -delete
      rmdir "$temp_dir"
      return 1
    fi
    install -m644 "$temp_dir/output-${bpm}.wav" "$output"
  done
  if [[ ! -e "$destination/PRIVATE-LOOPS-SOURCE.txt" ]]; then
    printf '%s\n' \
      'MusicRadar SampleRadar: 183 free 80s pop drums samples' \
      'https://www.musicradar.com/news/sampleradar-free-80s-pop-drums-samples' \
      'Royalty-free for music use; do not redistribute the raw samples.' \
      >"$destination/PRIVATE-LOOPS-SOURCE.txt"
  fi
  find "$temp_dir" -type f -delete
  rmdir "$temp_dir"
}

configured_loop_inbox="$(config_value "$RUNTIME_CONFIG" loop.import_directory)"
if [[ -n "${SHSYNTH_LOOP_INBOX:-}" ]]; then
  LOOP_INBOX="$SHSYNTH_LOOP_INBOX"
  replace_values "$RUNTIME_CONFIG" loop.import_directory "$LOOP_INBOX"
elif $RUNTIME_CREATED || [[ -z "$configured_loop_inbox" ]]; then
  LOOP_INBOX="$DATA_ROOT/shsynth/loop-inbox"
  replace_values "$RUNTIME_CONFIG" loop.import_directory "$LOOP_INBOX"
else
  LOOP_INBOX="$(expand_home_path "$configured_loop_inbox")"
fi
seed_public_loops "$LOOP_INBOX"

if [[ ! -t 0 ]]; then
  printf 'Created or kept configuration in %s.\n' "$STATE_DIR"
  printf 'Starter loops are available in %s.\n' "$LOOP_INBOX"
  printf 'Interactive routing was skipped because standard input is not a terminal.\n'
  exit 0
fi

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

choose_note_names() {
  local current default_choice answer
  current="$(config_value "$RUNTIME_CONFIG" display.note_names)"
  if [[ "$current" == english ]]; then default_choice=1; else default_choice=2; fi
  printf '\nWhich note-name system do you use?\n'
  printf '  1) English: C D E F G A B C\n'
  printf '  2) German:  C D E F G A H C  (B means B-flat)\n'
  while true; do
    read -r -p "[$default_choice] > " answer
    answer="${answer:-$default_choice}"
    case "$answer" in
      1) replace_values "$RUNTIME_CONFIG" display.note_names english; return 0 ;;
      2) replace_values "$RUNTIME_CONFIG" display.note_names german; return 0 ;;
      *) printf 'Choose 1 or 2.\n' >&2 ;;
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
          client !~ /^SHR-DAW/ && client !~ /^shs-/ && port != "") {
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

printf 'SHR-DAW hardware setup\n'
printf 'No audio server, synth engine, or audible test will be started.\n'
printf 'Starter loops: %s\n' "$LOOP_INBOX"

choose_note_names

sample_rate=48000
mapfile -t cards < <(alsa_cards)
if ((${#cards[@]})) && ask_yes_no 'Select the ALSA card JACK should use on its next start?' no; then
  choose_value 'JACK audio interface' no "${cards[@]}"
  card="${CHOSEN%% (*}"
  [[ "$card" =~ ^[A-Za-z0-9_-]+$ ]] || {
    printf 'Invalid ALSA card identifier: %s\n' "$card" >&2
    exit 1
  }
  read -r -p 'Sample rate, match your WAV loops when possible [48000]: ' sample_rate
  read -r -p 'Period size in frames [256]: ' period_size
  read -r -p 'Periods per buffer [3]: ' periods
  sample_rate="${sample_rate:-48000}"
  period_size="${period_size:-256}"
  periods="${periods:-3}"
  [[ "$sample_rate" =~ ^[0-9]+$ && "$period_size" =~ ^[0-9]+$ && "$periods" =~ ^[0-9]+$ ]] || {
    printf 'JACK timing values must be positive integers.\n' >&2
    exit 1
  }
  ((sample_rate > 0 && period_size > 0 && periods > 0)) || {
    printf 'JACK timing values must be greater than zero.\n' >&2
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
profile_result="$(SHSYNTH_STATE_DIR="$STATE_DIR" "$SHSYNTH_BIN" pads auto "$controller_input")"
printf '%s\n' "$profile_result"
if [[ "$profile_result" == *'no known profile'* ]] && \
   ask_yes_no 'Learn this controller now? MIDI will not be forwarded to a synth.' yes; then
  SHSYNTH_STATE_DIR="$STATE_DIR" "$SHSYNTH_BIN" pads learn "$controller_input"
fi
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
  replace_values "$RUNTIME_CONFIG" loop.output "$left_playback" "$right_playback"
else
  replace_values "$RUNTIME_CONFIG" audio.autoconnect false
  replace_values "$RUNTIME_CONFIG" audio.output
  replace_values "$RUNTIME_CONFIG" loop.output
fi

if ask_yes_no 'Download four private MusicRadar drum loops (78 MB; raw samples cannot be redistributed)?' no; then
  read -r -p "WAV sample rate [$sample_rate]: " loop_sample_rate
  loop_sample_rate="${loop_sample_rate:-$sample_rate}"
  if [[ ! "$loop_sample_rate" =~ ^[0-9]+$ ]] || \
     ((loop_sample_rate < 8000 || loop_sample_rate > 384000)); then
    printf 'WAV sample rate must be between 8000 and 384000 Hz.\n' >&2
    exit 1
  fi
  if install_private_musicradar_loops "$LOOP_INBOX" "$loop_sample_rate"; then
    printf 'Installed private 85, 110, 120, and 140 BPM loops in %s.\n' "$LOOP_INBOX"
  else
    printf 'Optional private loops were not installed; setup will continue.\n' >&2
  fi
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
  profile_ids=(unknown-monophonic-safe)
  if [[ -d "$PROFILE_SOURCE" ]]; then
    while IFS= read -r profile_id; do
      [[ -n "$profile_id" ]] && profile_ids+=("$profile_id")
    done < <(sed -n 's/^[[:space:]]*"id"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
      "$PROFILE_SOURCE"/*.json 2>/dev/null | sort -u)
  fi
  choose_value 'External instrument profile (numeric fallback is always available)' no \
    "${profile_ids[@]}"
  replace_values "$RUNTIME_CONFIG" external_midi.profile "$CHOSEN"
else
  replace_values "$RUNTIME_CONFIG" external_midi.enabled false
fi

cpu_count="$(getconf _NPROCESSORS_ONLN 2>/dev/null || printf '0')"
TUNE_TOOL=''
if command -v shr-audio-tune >/dev/null 2>&1; then
  TUNE_TOOL="$(command -v shr-audio-tune)"
elif [[ -x "$ROOT/scripts/audio-performance.sh" ]]; then
  TUNE_TOOL="$ROOT/scripts/audio-performance.sh"
fi
if [[ "$cpu_count" =~ ^[0-9]+$ ]] && ((cpu_count >= 4)) && [[ -n "$TUNE_TOOL" ]]; then
  if ask_yes_no 'Reserve one CPU for real-time audio (sudo and reboot required)?' no; then
    default_cpu=$((cpu_count - 1))
    read -r -p "Zero-based audio CPU [$default_cpu]: " audio_cpu
    audio_cpu="${audio_cpu:-$default_cpu}"
    if [[ ! "$audio_cpu" =~ ^[0-9]+$ ]] || ((audio_cpu >= cpu_count)); then
      printf 'Audio CPU must be an online zero-based CPU number.\n' >&2
      exit 1
    fi
    if ((EUID == 0)); then
      "$TUNE_TOOL" install "$audio_cpu"
    else
      sudo "$TUNE_TOOL" install "$audio_cpu"
    fi
    replace_values "$RUNTIME_CONFIG" audio.engine_cpu "$audio_cpu"
    printf 'The audio CPU becomes isolated after reboot; JACK was not restarted.\n'
  fi
fi

printf '\nConfiguration complete.\n'
printf '  Runtime:    %s\n' "$RUNTIME_CONFIG"
printf '  Controller: %s\n' "$CONTROLLER_CONFIG"
printf '  Backups:    *.bak-%s\n' "$STAMP"
printf '%s\n' "Run \`shr doctor\` after JACK is running, then edit either file for any finer routing."
