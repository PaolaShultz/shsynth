<p align="center">
  <img src="docs/images/shr-daw-header.jpg" alt="SHR-DAW" width="100%">
</p>

> [!WARNING]
> **SHR-DAW is highly experimental.** Back up Projects and user data, expect
> breaking changes, and keep monitoring levels low while testing audio.

SHR-DAW turns a Raspberry Pi, a 40×20 terminal, and optional MIDI gear into a
focused music workstation. Play software or external instruments, build FT2
Patterns and Arrangements, use effects, import private loops, save MIDI ideas,
record synchronized raw multitrack audio, and capture the protected final
stereo performance mix from compatible JACK interfaces.

Start with a Pi and computer keyboard. Add a MIDI controller, synth, audio
interface, or dedicated screen when useful.

The immediate connection between played sound, note spelling, chord names,
per-note MIDI strike velocity, scale-aware entry, and visible keyboard state
also makes SHR-DAW a practical learning companion. A player can relate what
they hear and press to intervals, chord construction, dynamics, scales, and
pattern notation without separating study from making music.

## Quick start

On Patchbox OS, Raspberry Pi OS, or Debian:

```sh
./scripts/install.sh
shr-setup
shr doctor
shr
```

The browser and external-MIDI tracker work without JACK. Software-instrument
audio, WAV loops, effects, and live recording require a running JACK server;
SHR-DAW never starts or restarts it implicitly. Continue with
[First run](docs/FIRST_RUN.md).

## At a glance

- Browse synthv1, Yoshimi, and FluidSynth sounds without layering managed
  engines.
- Route one controller to software or external MIDI instruments.
- Optionally drive a controller arpeggiator from SHR's dedicated 24-PPQN
  clock/transport output without reusing a musical tracker route.
- Sequence self-contained FT2 Patterns through an Arrangement.
- Load and edit 72 bundled drum grooves.
- Open ten cleared public-domain demo Projects, with matching Standard MIDI
  files and five separable arrangement parts.
- Save free-timed MIDI Ideas and private tracker Projects.
- Start with four CC0 48 kHz WAV loops, optionally download private
  tempo-labelled drums during setup, and monitor the loop-only stereo meter.
- Sum the managed software instrument, owned WAV loop, and one exact configured
  stereo input through master effects, a linked lookahead limiter, final meter,
  playback, and a 24-bit stereo final-mix recorder.
- Use a computer keyboard, mouse, or small configured controller.

New Projects use portable `AUTO` routing: they take this machine's configured
MIDI channels and active destination when loaded, without saving a device name
or using channel zero as a sentinel. Explicit routes from older or deliberately
hardware-bound Projects remain intact. Missing preferred MIDI/audio hardware
activates a visible, non-destructive fallback and never rewrites the preference.

Hardware names and routes remain configuration data. The owned effects graph
is opt-in and disabled by default. When enabled it requires exactly the managed
instrument, owned WAV loop, and configured stereo input. See the
[final performance bus](docs/FINAL_PERFORMANCE_BUS.md),
[How it works](docs/HOW_IT_WORKS.md), and the
[audio graph contract](docs/AUDIO_GRAPH.md) for the exact boundaries.

## Screens

This overview shows the main workspaces. The
[complete visual menu manual](docs/MENU_MANUAL.md) contains all 16 screens,
every contextual editor, and all 80 populated controller-menu pages.

### Presets

<img src="docs/images/shr-daw-presets.png" alt="Preset browser showing synthv1 sounds" width="100%">

Browse the three independent software-instrument catalogs.

### Playback

<img src="docs/images/shr-daw-playback.png" alt="Playback screen with held chord, aligned per-note MIDI velocities, continuous keyboard state, and control indicators" width="100%">

Play sounds, compare each held note's MIDI strike velocity, see the continuous
keyboard, shape mapped synthv1 controls, and record MIDI Ideas. Velocity helps
practise dynamics but is not an audio loudness measurement.

### Performance Meter

<img src="docs/images/shr-daw-performance-meter.png" alt="MTR final performance bus with source, limiter, meter, and recording status" width="100%">

With the graph disabled, inspect CPU load and the legacy managed-source meter.
With the graph enabled, MTR becomes the compact three-source performance-bus
surface: source level/mute/readiness, master level, final meter, limiter gain
reduction, and final recording status.
[Meter details](docs/USING_SHR_DAW.md#performance-meters).

### FT2 Pattern Editor

<img src="docs/images/shr-daw-ft2-pattern.png" alt="FT2 Pattern editor with four lanes of note data" width="100%">

Edit notes, velocity, programs, gates, commands, and multiple routed pages.

### Pattern Pages

<img src="docs/images/shr-daw-ft2-pages.png" alt="FT2 Pattern page routing screen with MIDI channels and targets" width="100%">

Choose one destination per page and an instrument setup for each column.

### FT2 Arrangement

<img src="docs/images/shr-daw-ft2-arrangement.png" alt="FT2 Arrangement screen listing ordered pattern steps" width="100%">

Chain Pattern IDs into the Project timeline.

### Drum Pattern Library

<img src="docs/images/shr-daw-drum-patterns.png" alt="FT2 drum pattern browser filtered by genre, meter, and phrase size" width="100%">

Filter, load, edit, and save reusable four-lane rhythms.

### Project Files

<img src="docs/images/shr-daw-project-files.png" alt="Project Files screen listing saved Projects" width="100%">

Name, save, preview, and safely clean up Projects and Patterns.

### FT2 WAV Loop

<img src="docs/images/shr-daw-ft2-loop.png" alt="FT2 WAV Loop screen with tempo, beat-region controls, and separate LOOP OUT stereo meter" width="100%">

Import private loops, align tempo and beat region, and monitor that WAV alone
on the separate `LOOP OUT` meter.

### Synchronized Multitrack Recorder

<img src="docs/images/shr-daw-audio-recorder.png" alt="Compact synchronized multitrack recorder with armed, ready, and missing inputs" width="100%">

Name, map, and arm exact JACK inputs, then capture one shared timeline as
separate 24-bit mono stems plus a versioned session manifest. Missing preferred
hardware remains missing instead of silently falling back. See [multitrack
recording](docs/MULTITRACK_RECORDING.md) and the concrete [MR18 acceptance
plan](docs/MR18_TEST_PLAN.md).

## Optional hardware

![SHR-DAW connected to an optional controller, display, audio interface, MIDI instruments, mixer, speakers, and headphones](docs/images/shr-daw-physical-connections.jpg)

Every device beyond the Raspberry Pi and audio output is optional. See
[Physical connections](docs/CONNECTIONS.md) for safe MIDI and audio paths.

## Documentation

- [First run](docs/FIRST_RUN.md) — configure and start.
- [Using SHR-DAW](docs/USING_SHR_DAW.md) — normal musical workflow.
- [Complete screen and menu manual](docs/MENU_MANUAL.md) — every screen,
  editor context, and controller-menu page with populated screenshots.
- [Tracker guide](docs/TRACKER.md) — Patterns, pages, Arrangement, drums, and
  loops.
- [Installation](docs/INSTALLATION.md) — dependencies, local evaluation,
  upgrades, and removal.
- [Development story](docs/BUILD_WEEK.md) — how SHR-DAW was built on the Pi
  with Codex, what each side contributed, and the Build Week record.
- [Complete documentation index](docs/README.md) — configuration, controller,
  architecture, measurements, audits, and future plans.

## Development

SHR-DAW is a personal weekend project developed and validated directly on its
Raspberry Pi. The detailed target-native, high-autonomy Codex workflow and its
human/AI decision boundary belong in the [development story](docs/BUILD_WEEK.md),
not in this product overview.

## License

SHR-DAW code, the 21 included presets, original demo arrangements, and bundled
rhythm data are MIT licensed; the four bundled WAV loops are CC0. The demo
compositions themselves are documented public-domain sources. Read
[THIRD_PARTY.md](THIRD_PARTY.md) before packaging the project or adding sounds.

---

<p align="center">
While I was releasing the first version of this software, my uncle died. So I dedicate this project to him.<br>
Počivao u miru, striče Mile, puno te volim!
</p>
