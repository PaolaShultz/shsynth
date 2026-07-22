
<img src="docs/images/shr-daw-header.jpg" alt="SHR-DAW" width="100%">

SHR-DAW is a compact Raspberry Pi music workstation for a 40×13 terminal,
optional MIDI gear, software instruments, FT2-style sequencing, WAV loops,
effects, and JACK recording.

> [!WARNING]
> SHR-DAW is experimental. Back up Projects and user data, and begin audio
> testing at a low monitoring level.

## Features

- Play synthv1, Yoshimi, or FluidSynth through one safely managed engine.
- Build routed multi-page Patterns, Arrangements, drum parts, and private WAV
  loop performances in the FT2 workspace.
- Save free-timed MIDI Ideas, synchronized raw JACK stems, or the protected
  final stereo performance mix.
- Use the computer keyboard, mouse, or a configured four-, five-, or
  eight-button controller.

See [Using SHR-DAW](docs/USING_SHR_DAW.md) for musical workflows and
[How it works](docs/HOW_IT_WORKS.md) for routing, ownership, storage, and
failure boundaries.

## Install and run

On Patchbox OS, Raspberry Pi OS, or Debian:

```sh
./scripts/install.sh
shr-setup
shr doctor
shr
```

JACK is optional for browsing and external-MIDI sequencing, but required for
software-instrument audio, WAV loops, effects, and audio recording. SHR-DAW
does not start or restart JACK. Continue with [First run](docs/FIRST_RUN.md) or
the full [installation guide](docs/INSTALLATION.md).

For a repository-local development checkout:

```sh
PATH=/home/patch/.rustup/toolchains/1.85.0-aarch64-unknown-linux-gnu/bin:$PATH cargo build --locked
./scripts/setup-local.sh
./scripts/local.sh
```

The local helpers keep configuration and user data below ignored `user/` and
launch this checkout's visibly marked `DEV` binary.

## Screenshot tour

### Software instruments

<img src="docs/images/shr-daw-presets.png" alt="Preset browser showing synthv1 sounds" width="100%">

Browse the separate synthv1, Yoshimi, and FluidSynth catalogs.

### Playback

<img src="docs/images/shr-daw-playback.png" alt="Playback screen with held notes, velocities, and mapped controls" width="100%">

Play the loaded sound, inspect notes and chords, shape mapped controls, and
capture MIDI Ideas.

### FT2 Pattern editor

<img src="docs/images/shr-daw-ft2-pattern.png" alt="FT2 Pattern editor with four lanes of note data" width="100%">

Edit routed melodic or percussion pages and arrange reusable Patterns.

### Loop Player

<img src="docs/images/shr-daw-ft2-loop.png" alt="FT2 Loop Player with region controls and loop-only stereo meter" width="100%">

Attach a private WAV, align its playable region, and follow it with FT2
transport.

### Audio recorder

<img src="docs/images/shr-daw-audio-recorder.png" alt="Synchronized multitrack recorder with armed and missing inputs" width="100%">

Map exact JACK inputs and record one callback-aligned take as separate mono
stems.

### Performance bus

<img src="docs/images/shr-daw-performance-meter.png" alt="Final performance bus with source, limiter, meter, and recording status" width="100%">

Control and record the opt-in three-source final bus, or inspect the passive
meter view while the graph is disabled.

The [screen and menu manual](docs/MENU_MANUAL.md) contains the complete visual
tour without duplicating its controls here.

## Documentation

- [First run](docs/FIRST_RUN.md) and [Using SHR-DAW](docs/USING_SHR_DAW.md)
- [Tracker guide](docs/TRACKER.md) and [screen and menu manual](docs/MENU_MANUAL.md)
- [Configuration and routing](docs/CONFIGURATION.md) and
  [controller interface](docs/CONTROLLER_INTERFACE.md)
- [How it works](docs/HOW_IT_WORKS.md), [audio graph](docs/AUDIO_GRAPH.md), and
  [multitrack recording](docs/MULTITRACK_RECORDING.md)
- [Complete documentation index](docs/README.md)

## Built with Codex

SHR-DAW was a pre-existing personal project that was meaningfully extended
during OpenAI Build Week using GPT-5.6 through Codex CLI directly on the target
Raspberry Pi. Codex accelerated Rust implementation, ALSA/JACK/MIDI diagnosis,
controller setup, original preset and rhythm design, safety review, validation,
and documentation. The creator chose the product and musical direction,
supplied and operated the hardware, judged the sound, and controlled public
release. The [development story and dated baseline](docs/BUILD_WEEK.md) describe
that collaboration and distinguish earlier work from Build Week additions.

## Licence

SHR-DAW is MIT licensed. Included presets, demos, rhythms, and WAV loops have
their own documented clearance boundaries; read [THIRD_PARTY.md](THIRD_PARTY.md)
before packaging or adding sounds.
