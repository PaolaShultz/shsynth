<p align="center">
  <img src="docs/images/shr-daw-header.jpg" alt="SHR-DAW" width="100%">
</p>

> [!WARNING]
> **SHR-DAW is highly experimental.** Back up Projects and user data, expect
> breaking changes, and keep monitoring levels low while testing audio.

SHR-DAW turns a Raspberry Pi, a 40×20 terminal, and optional MIDI gear into a
focused music workstation. Play software or external instruments, build FT2
Patterns and Arrangements, use effects, import private loops, save MIDI ideas,
and record stereo audio from a compact physical interface.

Start with a Pi and computer keyboard. Add a MIDI controller, synth, audio
interface, or dedicated screen when useful.

## Quick start

On Patchbox OS, Raspberry Pi OS, or Debian:

```sh
./scripts/install.sh
shr-setup
shr doctor
shr
```

The browser and external-MIDI tracker work without JACK. Software-instrument
audio, WAV loops, effects, and stereo recording require a running JACK server;
SHR-DAW never starts or restarts it implicitly. Continue with
[First run](docs/FIRST_RUN.md).

## At a glance

- Browse synthv1, Yoshimi, and FluidSynth sounds without layering managed
  engines.
- Route one controller to software or external MIDI instruments.
- Sequence self-contained FT2 Patterns through an Arrangement.
- Load and edit 72 bundled drum grooves.
- Save free-timed MIDI Ideas and private tracker Projects.
- Import private WAV loops and record a configured stereo JACK input.
- Process the managed software instrument through source inserts, two aux
  returns, master effects, and a passive final-output meter.
- Use a computer keyboard, mouse, or small configured controller.

Hardware names and routes remain configuration data. The current owned effects
graph is opt-in, disabled by default, and intentionally limited to one managed
software-instrument source. See [How it works](docs/HOW_IT_WORKS.md) and the
[audio graph contract](docs/AUDIO_GRAPH.md) for the exact boundaries.

## Screens

This overview shows the main workspaces. The
[complete visual menu manual](docs/MENU_MANUAL.md) contains all 16 screens,
every contextual editor, and all 80 populated controller-menu pages.

### Presets

<img src="docs/images/shr-daw-presets.png" alt="Preset browser showing synthv1 sounds" width="100%">

Browse the three independent software-instrument catalogs.

### Playback

<img src="docs/images/shr-daw-playback.png" alt="Playback screen with held notes, control indicators, and recorded MIDI status" width="100%">

Play sounds, shape mapped synthv1 controls, and record MIDI Ideas.

### Performance Meter

<img src="docs/images/shr-daw-performance-meter.png" alt="MTR performance screen with four CPU bars and stereo output meters" width="100%">

Inspect CPU load and the owned graph's final stereo output without changing the
audio route. [Meter details](docs/USING_SHR_DAW.md#performance-meters).

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

<img src="docs/images/shr-daw-ft2-loop.png" alt="FT2 WAV Loop screen with tempo and beat-region controls" width="100%">

Import private loops and align the tracker tempo and beat region.

### Stereo Recorder

<img src="docs/images/shr-daw-audio-recorder.png" alt="Stereo recorder screen with input ports and recording status" width="100%">

Capture a configured JACK stereo input as a 24-bit WAV file.

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

SHR-DAW code, the 21 included presets, and bundled original rhythm data are MIT
licensed. Read [THIRD_PARTY.md](THIRD_PARTY.md) before packaging the project or
adding sounds.

---

<p align="center">
While I was releasing the first version of this software, my uncle died. So I dedicate this project to him.<br>
Počivao u miru, striče Mile, puno te volim!
</p>
