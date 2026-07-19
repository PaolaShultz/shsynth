# Repository instructions

Read `docs/WORKSPACE_HANDOFF.md` at the start of work in this checkout. It
records the repository/local-data boundary, current hardware setup, publishing
workflow, and the decisions that must survive a fresh conversation.
Before changing or questioning a repository helper, also read
`docs/MAINTAINER_HELPERS.md`; it records every helper's parameters, side
effects, safety boundary, and design rationale, including the intentionally
explicit screenshot scaler.

## Purpose and environment

SHR-DAW is a Raspberry Pi mini DAW with a Rust TUI, FT2-style tracker, synth
hosts, MIDI routing, loops, recording, JACK/ALSA integration, and a small
physical controller. Keep it responsive at 40×20 and safe for live audio.
The system `cargo` may be too old; use the installed Rust 1.85 toolchain:

```sh
PATH=/home/patch/.rustup/toolchains/1.85.0-aarch64-unknown-linux-gnu/bin:$PATH cargo test --locked
```

Before handoff for Rust, Cargo, installer, runtime configuration, preset, or
behavior changes, also run `cargo fmt -- --check`, `cargo clippy --locked -- -D
warnings`, and `cargo build --release --locked` with that PATH. For docs,
README, screenshot, or image-only changes, do not run the full Rust suite just
because a handoff or push is happening; run targeted checks instead, such as
link/reference checks, image size/format checks, `python3 -m py_compile` for
Python helpers, and `git diff --check`.

Install tools required to complete requested setup, validation, or publishing
work instead of silently skipping the check or substituting a weaker one. On
Debian/Raspberry Pi OS this includes `libxml2-utils` for `xmllint` preset
validation and `gh` for GitHub authentication/publishing. Use the existing
GitHub CLI login when it is valid. If authentication is missing or expired,
use the web/device authorization flow and let the user authorize it. Do not
invent a Git author identity or expose authentication credentials.

## Architecture

- `src/ui.rs`: screens, actions, engine lifecycle, recording workflow.
- `src/engine.rs`: synthv1 process, JACK/ALSA connections, monitored MIDI route.
- `src/midi.rs`: routing and pickup/catch state.
- `src/control.rs`: the 12 canonical controls, ranges, and relative colors.
- `src/preset.rs`: preset discovery and XML values read by parameter name.
- `src/recording.rs`: idea snapshots and MIDI encoding/playback.
- `src/navigation.rs` / `src/pads.rs`: screen-specific command pads and config.

## Invariants

- Never layer managed synth engines. Preserve clean shutdown and All Notes Off.
- Never terminate synthv1 processes SHSynth does not own.
- Hardware names, executable/client names, preset paths, and audio/MIDI routes
  belong in `shsynth.conf` or `controller.conf`, not Rust constants.
- Block mapped CC messages before synthv1 until pickup reaches/crosses the
  loaded value. Loading and in-place parameter reset must re-arm pickup.
- On Playback, the main encoder press resets only the 12 mapped parameters and
  re-arms pickup without restarting the engine. PLAY and keyboard `P` control
  MIDI take playback.
- Indicators are relative to the original preset: green below −0.03, bright
  yellow within ±0.03, red above +0.03.
- Command pad note-on and note-off are consumed; musical MIDI passes through.
- Keep synthv1 0.9.29 indices/ranges in `control.rs`. Parse preset XML by name
  because older source files can have obsolete indices.
- Preserve user ideas, controller config, unrelated processes, and existing
  work. Do not start audible synth/JACK tests unless explicitly requested.

## Presets and documentation

Place sounds in `presets/synthv1/` and follow `docs/NEW_PATCHES.md`. Prefer a
complete current-schema preset, validate XML and parameter names, and retain
source/license information for imported patches. Update README when behavior,
commands, mappings, storage, or hardware assumptions change. Remove stale docs
instead of leaving conflicting instructions.

Do not publish the uncleared preset bank until its provenance is established; see
`THIRD_PARTY.md`. Project code and newly authored cleared presets use MIT.

## Collaboration and navigation

Act as a navigator for a user who has musical goals but does not assume prior
music-theory, oscillator/filter, MIDI, JACK, or Rust knowledge. Explain choices
in plain language, recommend a safe default, and connect technical parameters
to what the user will hear or do. For physical setup, give one concrete action
at a time and separate user-performed actions from machine inspection. Research
unfamiliar or current hardware, software, music, and product details from
authoritative sources instead of guessing, and preserve source/provenance notes
when the result affects configuration, sounds, or redistribution.
