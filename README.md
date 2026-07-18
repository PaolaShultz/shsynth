<p align="center">
  <img src="docs/images/shr-daw-header.jpg" alt="SHR-DAW" width="100%">
</p>

SHR-DAW turns a Raspberry Pi, a 40×20 terminal, and the MIDI gear you already
own into a focused music workstation. It is for bedroom musicians and
Linux-audio makers who want a physical, mouse-free workflow without hiding the
routing and safety rules that keep live MIDI predictable.

Use it to play software instruments, control external MIDI instruments, build
FT2 Patterns, arrange complete Projects, save MIDI ideas, and record stereo
audio. Start with only a Raspberry Pi and a computer keyboard. Add a MIDI
controller, synthesizer, audio interface, or dedicated screen when you need one.

Development and release QA are target-native: Codex CLI runs directly on the
Raspberry Pi, and the source editing, Rust compilation, tests, Clippy checks,
and optimized builds happen on that same machine. This is specifically **not a
PC-first development path** where the application is developed or
cross-compiled on a desktop and then copied to the Pi. The creator also
completed a working session with two active Codex CLI instances while
SHR-DAW's managed synth was running. That is a qualitative user-observed
session, not a latency or capacity benchmark.

All Codex CLI work in this development push was deliberately run with the
literal `--yolo` invocation. I gave Codex the goals and durable repository
safety rules, but did very little line-by-line terminal supervision or screen
reading; I reviewed checkpoints and working outcomes instead. Codex still had
to pause for physical hardware, audible listening, destructive/system changes,
publishing, and decisions only I could make. This describes my high-autonomy
workflow on my own Pi and checkout—it is not a general recommendation to bypass
safeguards.

## App screens

SHR-DAW is split into focused 40x20 pages, so performance controls, FT2 pattern
editing, arrangement, routing, files, loops, and recording stay separate.

### Presets

<img src="docs/images/shr-daw-presets.png" alt="Preset browser showing synthv1 sounds" width="100%">

Browse playable software-instrument sounds. The browser is paged across three
software instruments, each with its own independent patch list.

### Playback

<img src="docs/images/shr-daw-playback.png" alt="Playback screen with held notes, control indicators, and recorded MIDI status" width="100%">

Play, record, review, and save MIDI ideas.

### FT2 Pattern Editor

<img src="docs/images/shr-daw-ft2-pattern.png" alt="FT2 Pattern editor with four lanes of note data" width="100%">

Edit notes, velocity, programs, gates, and commands. Every visible column has
its own MIDI channel, bank, and master instrument. Step entry can advance by
1, 2, 4, or 8 rows for fast rhythm and bass-line entry.

### Pattern Pages

<img src="docs/images/shr-daw-ft2-pages.png" alt="FT2 Pattern page routing screen with MIDI channels and targets" width="100%">

Keep one destination per page with four independent column channel/instrument
setups, plus lanes and mutes, inside each Pattern.

### FT2 Arrangement

<img src="docs/images/shr-daw-ft2-arrangement.png" alt="FT2 Arrangement screen listing ordered pattern steps" width="100%">

Chain Pattern IDs into the Project timeline.

### Drum Pattern Library

<img src="docs/images/shr-daw-drum-patterns.png" alt="FT2 drum pattern browser filtered by genre, meter, and phrase size" width="100%">

Choose a genre, 3/4 or 4/4 meter, and a 2/4/8-bar phrase before loading the
rhythm into the current Pattern's percussion page.

### Project Files

<img src="docs/images/shr-daw-project-files.png" alt="Project Files screen listing saved Projects" width="100%">

Name, rename, save, load, preview, and delete whole Projects; clean up only
unreferenced Pattern records.

### FT2 WAV Loop

<img src="docs/images/shr-daw-ft2-loop.png" alt="FT2 WAV Loop screen with tempo and beat-region controls" width="100%">

Import private loops and set tracker tempo from detected WAV beats.

### Stereo Recorder

<img src="docs/images/shr-daw-audio-recorder.png" alt="Stereo recorder screen with input ports and recording status" width="100%">

Capture a JACK stereo input as a 24-bit WAV file.

## Start simple, expand when you want

SHR-DAW can sit at the center of a larger music setup, but the devices in this
diagram are optional. Begin with a Raspberry Pi and an audio output. Add a MIDI
controller, external instruments, an audio interface, a mixer, or a dedicated
screen as your setup grows.

![SHR-DAW connected to an optional controller, display, audio interface, MIDI instruments, mixer, speakers, and headphones](docs/images/shr-daw-physical-connections.jpg)

The basic signal paths are controller → SHR-DAW, SHR-DAW → software or hardware
instruments, and audio → speakers or the stereo recorder. See
[Physical connections](docs/CONNECTIONS.md) for the detailed MIDI and audio
routes.

## What it does

- Plays synthv1, Yoshimi, and FluidSynth instruments.
- Routes one MIDI controller to software and hardware instruments.
- Builds Projects with self-contained FT2 Patterns and an FT2 Arrangement.
- Provides FT2 Play/Rec/Edit/N00B modes and scale-safe live MIDI input.
- Filters 72 bundled drum grooves by genre, meter, and 2/4/8-bar
  phrase size, saves drum pages separately, and transposes melodic Pattern
  pages by semitone or octave.
- Imports private WAV loops and synchronizes FT2 tempo to them through JACK.
- Records free playing as reusable MIDI ideas.
- Records a stereo JACK input as a 24-bit WAV file.
- Provides a project-persisted, ordered eight-slot insert rack with EQ,
  compressor, distortion, gate, multimode filter, bitcrusher/sample-rate
  reducer, and utility processing when the owned audio graph is enabled.
- Works from a computer keyboard or a small physical controller.

SHR-DAW is designed as a portable music box. It is not tied to one controller,
synthesizer, or audio interface. Hardware names and routes are configured by
the user.

## Quick start

On Patchbox OS, Raspberry Pi OS, or Debian:

```sh
./scripts/install.sh
shr-setup
shr doctor
shr
```

The preset browser and external-MIDI tracker can open without JACK. JACK must
be running before loading a software instrument, playing a WAV loop, or
recording audio. The setup wizard helps choose MIDI and audio ports, but it
does not start or restart JACK.

The SHR-owned audio graph remains opt-in and disabled by default. Its authorized
Raspberry Pi dry-path checkpoint passed with bit-exact stereo output and no
callback deadline misses. Phase 2's bounded insert processors and compact FX
editors are implemented; their final Raspberry Pi performance/listening gate
is still recorded separately. Direct synth playback remains the fallback if
graph validation, activation, or an exact JACK connection fails; see the
[audio graph contract](docs/AUDIO_GRAPH.md), [Phase 1 measurement](docs/PHASE1_AUDIO_GRAPH_MEASUREMENT.md),
and [Phase 2 measurement](docs/PHASE2_AUDIO_GRAPH_MEASUREMENT.md).

Read [Installation](docs/INSTALLATION.md) for supported systems and installer
options. Then follow [First run](docs/FIRST_RUN.md) to configure and test your
setup.

### Fast non-audible evaluation

Judges and contributors without the exact hardware can exercise the real parser,
storage, routing, and 40×20 rendering code without starting JACK:

```sh
cargo test --locked
SHSYNTH_STATE_DIR=/tmp/shr-daw-judge-state cargo run --locked -- config init
SHSYNTH_STATE_DIR=/tmp/shr-daw-judge-state cargo run --locked -- list
SHSYNTH_STATE_DIR=/tmp/shr-daw-judge-state cargo run --locked -- screenshots > /tmp/shr-daw-screens.json
```

Run `cargo run --locked -- menu` in a terminal at least 40×20 cells to inspect
the browser and tracker interactively. Missing MIDI or JACK hardware is reported
in the status line; it does not prevent opening the interface. The two `/tmp`
paths above are the only generated evaluation data and can be deleted afterward.

### Bundled sample and presentation data

The public checkout includes 21 original synthv1 preset XML files and 72
editable drum-pattern seeds. `shr list` exercises the real preset discovery
path; the **Drum Patterns** screen loads and expands the real rhythm files into
2-, 4-, or 8-bar Pattern data. `shr screenshots` uses explicitly seeded
in-memory presentation states to exercise the real 40×20 renderer; it does not
simulate audio or connected hardware.

No WAV, private Idea, recording, or finished demo Project is bundled. The final
Build Week Project remains a human listening/performance task until its sounds,
routes, and original music are approved. Private and uncleared material stays
below ignored `user/` or the normal XDG data directories and is never required
for the judge path.

## Documentation

### Use SHR-DAW

- [First run](docs/FIRST_RUN.md) — configure, check, and open SHR-DAW.
- [Using SHR-DAW](docs/USING_SHR_DAW.md) — instruments, screens, ideas, and
  audio recording.
- [In-app help](docs/HELP.md) — compact help text shown by `?`, F1, or the
  controller Help action. While Help is open, SHR-DAW also tries to serve the
  same page temporarily at `http://<LAN-IP>/help`.
- [Tracker guide](docs/TRACKER.md) — FT2 Patterns, pages, arrangement, step
  editing, live recording, and Project files.
- [Controller interface](docs/CONTROLLER_INTERFACE.md) — physical controls and
  the complete menu map.
- [Physical connections](docs/CONNECTIONS.md) — simple and expanded hardware
  setups, MIDI paths, and audio paths.
- [Future improvements](docs/FUTURE_IMPROVEMENTS.md) — intentionally deferred
  rhythm, routing, insert/send effects, and their safety requirements.
- [Post-competition rhythm plan](docs/POST_COMPETITION_RHYTHM_PLAN.md) — staged
  arbitrary Pattern length, microtiming, swing, groove, and optional meter work.

### Install and customize it

- [Installation](docs/INSTALLATION.md) — dependencies, installed commands, and
  optional Raspberry Pi audio tuning.
- [Configuration and routing](docs/CONFIGURATION.md) — configuration files,
  page targets, channels, and offline devices.
- [MIDI device profiles](docs/MIDI_DEVICE_PROFILES.md) — named sounds and bank
  data for external instruments.
- [Controller profiles and MIDI learn](docs/CONTROLLER_PROFILES.md) — automatic
  matching and non-audible setup for USB input controllers.
- [Codex-assisted setup](docs/CODEX_ASSISTED_SETUP.md) — optional help for
  unusual hardware or recovery.

### Understand or develop it

- [How it works](docs/HOW_IT_WORKS.md) — synth ownership, MIDI safety, pickup,
  recording, and data locations.
- [Audio graph and DSP contract](docs/AUDIO_GRAPH.md) — owned-routing schema,
  real-time rules, fixed limits, fallback, measurement, and curation gates.
- [Add patches and sounds](docs/NEW_PATCHES.md) — create and validate synthv1
  presets.
- [Third-party software and sounds](THIRD_PARTY.md) — credits, licences, and
  redistribution rules.
- [Workspace handoff](docs/WORKSPACE_HANDOFF.md) — current development and
  local-machine context.
- [Build Week record](docs/BUILD_WEEK.md) — Codex's development role, current
  metrics, eligibility timeline, and the human/AI decision boundary.

## About the creator

The first public commit was not the beginning of the code. It was the moment I
released the first version and dedicated it to my uncle, who died while I was
releasing it. I developed the code leading to that release with GPT-5.6 Sol
through Codex CLI on the Raspberry Pi; the privacy-preserving evidence summary
is in the [Build Week record](docs/BUILD_WEEK.md#model-provenance-for-the-first-release).

`PaolaShultz`, the repository owner, is my gaming name and a nickname I
sometimes use online. I chose it after the empty tombstone used in the
buried-alive sequence in *Kill Bill: Volume 2*—the marker where nobody was
actually buried before that scene. It is not a company or a separate
contributor: I am the same person making the product and musical decisions
described here.

SHR-DAW is personal in the same way. It is a weekend/free-time project built
around the Raspberry Pi and music hardware I actually own, sometimes alongside
my main work on the `bee247.hr` portal. Codex helped me turn those goals into a
working instrument, but the reasons for building it, the physical setup, and
the final musical taste are mine.

## License

SHR-DAW code, the 21 included presets, and bundled original rhythm data are MIT
licensed. See
[THIRD_PARTY.md](THIRD_PARTY.md) before packaging the project or adding sounds.

---

<p align="center">
While I was releasing the first version of this software, my uncle died. So I dedicate this project to him.<br>
Počivao u miru, striče Mile, puno te volim!
</p>
