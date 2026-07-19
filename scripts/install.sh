#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
INSTALL_DEPS=true
INIT_CONFIG=true

for arg in "$@"; do
  case "$arg" in
    --no-deps) INSTALL_DEPS=false ;;
    --no-config) INIT_CONFIG=false ;;
    -h|--help)
      printf 'Usage: %s [--no-deps] [--no-config]\n' "$0"
      exit 0
      ;;
    *) printf 'Unknown option: %s\n' "$arg" >&2; exit 2 ;;
  esac
done

if $INSTALL_DEPS; then
  command -v apt-get >/dev/null || {
    printf 'Automatic dependencies require Debian/Raspberry Pi OS (apt-get).\n' >&2
    exit 1
  }
  sudo apt-get update
  sudo apt-get install -y \
    alsa-utils build-essential ca-certificates curl jackd2 libasound2-dev \
    fluidsynth pkg-config python3 sox synthv1 timgm6mb-soundfont unzip yoshimi yoshimi-data
fi

version_ok() {
  local version
  version="$($1 --version 2>/dev/null | awk '{print $2}')"
  [[ "$(printf '%s\n' 1.85.0 "$version" | sort -V | head -n1)" == 1.85.0 ]]
}

CARGO=(cargo)
if ! command -v cargo >/dev/null || ! version_ok cargo; then
  if ! command -v rustup >/dev/null; then
    printf 'Installing the official minimal Rust toolchain for the current user.\n'
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs |
      sh -s -- -y --profile minimal --default-toolchain 1.85.0
    export PATH="$HOME/.cargo/bin:$PATH"
  fi
  rustup toolchain install 1.85.0 --profile minimal
  CARGO=(cargo +1.85.0)
fi

cd "$ROOT"
"${CARGO[@]}" test --locked
"${CARGO[@]}" build --release --locked
sudo make install-files

if $INIT_CONFIG; then
  shr-setup
fi

printf '\nInstalled: shr (Rust app), shs and synth-player (compatibility aliases)\n'
printf 'Run shr doctor, then run shr. Reconfigure hardware with shr-setup.\n'
