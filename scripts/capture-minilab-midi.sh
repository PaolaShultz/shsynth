#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: ./scripts/capture-minilab-midi.sh [--check] [--output FILE]

Passively capture every incoming event from every ALSA sequencer port exposed
by the one connected MiniLab 3. The default log is a unique file below /tmp.

Options:
  --check        Run discovery, safety checks, and compilation without opening
                 MIDI ports or changing a service.
  --output FILE  Write the capture to FILE; refuse to replace an existing file.
  -h, --help     Show this help.

While capture is active, type a short label and press Enter to put a timestamped
marker in the log. Press Ctrl-C after the final controller action.
EOF
}

fail() {
    printf 'capture-minilab-midi: %s\n' "$*" >&2
    exit 1
}

check_only=0
output_file=""
while (($# > 0)); do
    case "$1" in
        --check)
            check_only=1
            shift
            ;;
        --output)
            (($# >= 2)) || fail "--output requires a file"
            output_file=$2
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            fail "unknown argument: $1"
            ;;
    esac
done

for command_name in aconnect aseqdump gcc jack_lsp pgrep pkg-config systemctl; do
    command -v "$command_name" >/dev/null 2>&1 ||
        fail "required command is missing: $command_name"
done
pkg-config --exists alsa ||
    fail "ALSA development files are missing (install libasound2-dev)"

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
monitor_source="$script_dir/capture-minilab-midi.c"
[[ -r "$monitor_source" ]] || fail "monitor source is missing: $monitor_source"

declare -a candidate_ids=()
declare -a candidate_names=()
while IFS=$'\t' read -r client_id client_name; do
    normalized_name=$(printf '%s' "$client_name" |
        tr '[:upper:]' '[:lower:]' | tr -d ' _-')
    if [[ $normalized_name == *minilab3* ]]; then
        candidate_ids+=("$client_id")
        candidate_names+=("$client_name")
    fi
done < <(aconnect -l | sed -n "s/^client \([0-9][0-9]*\): '\([^']*\)'.*/\1\t\2/p")

((${#candidate_ids[@]} > 0)) || fail "no MiniLab 3 ALSA sequencer client was found"
if ((${#candidate_ids[@]} != 1)); then
    printf 'Matching ALSA clients:\n' >&2
    for index in "${!candidate_ids[@]}"; do
        printf '  %s %s\n' "${candidate_ids[$index]}" "${candidate_names[$index]}" >&2
    done
    fail "more than one MiniLab 3 matched; disconnect the one not being tested"
fi

minilab_client=${candidate_ids[0]}
minilab_name=${candidate_names[0]}
mapfile -t endpoints < <(
    aseqdump -l | awk -v client="$minilab_client" \
        '$1 ~ ("^" client ":[0-9]+$") { print $1 }'
)
((${#endpoints[@]} > 0)) || fail "MiniLab client $minilab_client has no readable ports"
mapfile -t endpoint_rows < <(
    aseqdump -l | awk -v client="$minilab_client" \
        '$1 ~ ("^" client ":[0-9]+$")'
)

for process_name in shr shsynth synthv1; do
    if pgrep -x "$process_name" >/dev/null 2>&1; then
        pgrep -a -x "$process_name" >&2 || true
        fail "$process_name is running; stop it before passive capture"
    fi
done

jack_seq_client=$(
    sed -n 's/^Client \([0-9][0-9]*\) : "jack_midi".*/\1/p' \
        /proc/asound/seq/clients | head -n 1
)
mapfile -t subscription_lines < <(
    aconnect -l | awk -v client="$minilab_client" '
        /^client [0-9]+:/ { active = ($2 == client ":") }
        active && /Connecting To:|Connected From:/ { print }
    '
)
for subscription_line in "${subscription_lines[@]}"; do
    peer_list=${subscription_line#*:}
    IFS=',' read -r -a peers <<< "$peer_list"
    for peer in "${peers[@]}"; do
        peer=${peer//[[:space:]]/}
        peer=${peer%%\[*}
        peer_client=${peer%%:*}
        if [[ -z $jack_seq_client || $peer_client != "$jack_seq_client" ]]; then
            fail "MiniLab has a non-JACK ALSA subscription ($peer); disconnect it first"
        fi
    done
done

if jack_lsp >/dev/null 2>&1; then
    mapfile -t minilab_jack_ports < <(
        jack_lsp -A | awk '
            /^[^[:space:]]/ { port = $0; next }
            {
                alias = tolower($0)
                gsub(/[ _-]/, "", alias)
                if (alias ~ /minilab3\/midi_(playback|capture)_[0-9]+/)
                    print port
            }
        ' | sort -u
    )
    for jack_port in "${minilab_jack_ports[@]}"; do
        mapfile -t jack_connections < <(jack_lsp -c "$jack_port")
        if ((${#jack_connections[@]} > 1)); then
            printf 'JACK port and connections:\n' >&2
            printf '  %s\n' "${jack_connections[@]}" >&2
            fail "MiniLab JACK port $jack_port is connected; disconnect it before capture"
        fi
    done
fi

build_dir=$(mktemp -d /tmp/shr-minilab-monitor.XXXXXX)
service_was_active=0
cleanup() {
    local saved_status=$?
    trap - EXIT
    if ((service_was_active)); then
        sudo systemctl start amidiminder.service ||
            printf 'WARNING: could not restart amidiminder.service\n' >&2
    fi
    rm -rf -- "$build_dir"
    exit "$saved_status"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

monitor_binary="$build_dir/capture-minilab-midi"
read -r -a alsa_flags <<< "$(pkg-config --cflags --libs alsa)"
gcc -std=c11 -Wall -Wextra -Werror -O2 "$monitor_source" \
    "${alsa_flags[@]}" -pthread -o "$monitor_binary"

printf 'MiniLab client: %s (%s)\n' "$minilab_client" "$minilab_name"
printf 'Incoming ports:\n'
printf '  %s\n' "${endpoint_rows[@]}"
printf 'Safety checks: no SHR/synth process, non-JACK ALSA route, or connected MiniLab JACK port found.\n'

if ((check_only)); then
    printf 'Check complete; no MIDI port was opened and no service was changed.\n'
    exit 0
fi

if [[ -z $output_file ]]; then
    output_file="/tmp/shr-minilab-capture-$(date +%Y%m%d-%H%M%S).log"
fi
[[ ! -e $output_file ]] || fail "output already exists: $output_file"
output_parent=$(dirname -- "$output_file")
[[ -d $output_parent && -w $output_parent ]] ||
    fail "output directory is not writable: $output_parent"

if systemctl is-active --quiet amidiminder.service; then
    service_was_active=1
    sudo systemctl stop amidiminder.service
fi

printf '\nCapture log: %s\n' "$output_file"
printf 'Type PROGRAM_1 and Enter, then select the first program and press pads 1-8.\n'
printf 'Repeat with PROGRAM_2 and PROGRAM_3.\n'
printf 'Type ARP_CHORD and Enter before enabling the arpeggiator and playing the chord.\n'
printf 'Press Ctrl-C after the chord has been released.\n\n'

{
    printf 'capture_started=%s\n' "$(date --iso-8601=seconds)"
    printf 'minilab_client=%s minilab_name=%q\n' "$minilab_client" "$minilab_name"
    printf 'endpoints='
    printf '%s ' "${endpoints[@]}"
    printf '\n'
    "$monitor_binary" "${endpoints[@]}"
} 2>&1 | tee "$output_file"
