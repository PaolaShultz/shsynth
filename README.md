<p align="center">
  <img src="docs/images/shr-daw-header.png" alt="shr-daw" width="100%">
</p>

SHR-DAW is a small Raspberry Pi mini DAW. It runs in a 40×20 terminal and is
made for hands-on use with a MIDI controller, a small screen, and an audio
interface. The name means **Shome Rust DAW**.

It can play software instruments, route MIDI, sequence hardware, save MIDI
ideas, and record stereo audio. The goal is a portable music box that can grow
from one synth into a hub for many MIDI instruments.

<p align="center">
  <img src="docs/images/shr-daw-ft2-tracker.png" alt="shr-daw FT2-style four-lane tracker" width="480">
</p>

The first test rig used an Arturia MiniLab, an AudioBox USB interface, and a
Casio Casiotone MT-240. That rig proved that the routing and pattern workflow
work on real hardware. SHR-DAW is not a controller made for that keyboard. All
device names, MIDI routes, channels, control maps, and JACK ports live in
configuration files.

## The idea

One Raspberry Pi can sit in the middle of a small setup:

![Physical SHR-DAW connections: controller, Raspberry Pi, display, USB audio/MIDI interface, chained MIDI instruments, mixer, monitors, and headphones](docs/images/shr-daw-physical-connections.png)

```text
MIDI controller ──> SHR-DAW ──> synthv1 / Yoshimi / FluidSynth ──> JACK out
                         └────> FT2 pattern pages ──> MIDI hardware

JACK input ──> SHR-DAW stereo recorder ──> WAV
```

SHR-DAW reads the controller once. It keeps menu buttons and mapped controls
inside the app, then sends musical MIDI to the right place. This avoids double
notes and random direct connections from desktop MIDI tools.

Only one managed software synth runs at a time. Changing the synth sends All
Notes Off, stops the process started by SHR-DAW, and starts the next one.
SHR-DAW never kills a synth process that it does not own.

## Sound engines and credit

SHR-DAW is a controller and host for three separately installed open-source
synth engines. Their code and factory sound data are not part of this project:

- [synthv1](https://synthv1.sourceforge.io/) is Rui Nuno Capela's four-oscillator
  subtractive polyphonic synthesizer. Its upstream source is mirrored on
  [GitHub](https://github.com/rncbc/synthv1) and is GPL-2.0-or-later.
- [Yoshimi](https://yoshimi.github.io/) is a Linux software synthesizer that
  grew from ZynAddSubFX, with Will Godfrey as its current lead developer and
  maintainer. Its upstream source is on
  [GitHub](https://github.com/Yoshimi/yoshimi) and is GPL-2.0-or-later.
- [FluidSynth](https://www.fluidsynth.org/) is the FluidSynth project's
  real-time SoundFont 2 software synthesizer. Its upstream source is on
  [GitHub](https://github.com/FluidSynth/fluidsynth) and is LGPL-2.1-or-later.

SHR-DAW starts these programs as external processes and connects to the
selected engine through configured JACK and MIDI routes. See
[THIRD_PARTY.md](THIRD_PARTY.md) for redistribution details.

## Keyboardless and mouseless operation

SHR-DAW is designed to be played and operated as a hardware appliance. After
the initial routing setup, the main performance and sequencing workflow does
not require a computer keyboard or mouse. The attached display shows what the
physical controls do on the current screen:

- four menu-page labels and the selected page's four item labels are always
  visible; working, disabled, and planned items use different styles;
- the main encoder moves through menus, presets, rows, pages, and choices, and
  its press selects or confirms;
- mapped knobs adjust the 12 current synthv1 performance parameters with
  pickup, so loading a preset does not cause sudden value jumps;
- transport, pattern editing, page routing, idea recording, and audio
  recording are reachable from the controller.

A computer keyboard and mouse remain optional conveniences. The setup wizard
and direct configuration-file editing may still need a terminal when hardware
or audio routes change.

### Controller menu paging

Paging is implemented on every screen and contextual editor. Each screen has
four named pages of four menu items, for 16 controller-accessible slots. Page
selection matches the available hardware:

- the current eight-button layout can use four buttons for direct, fast page
  selection and four buttons for the active page's items;
- a five-button controller can use one button to cycle pages and four buttons
  for the active page's items;
- a four-button controller can use the rotary encoder to switch pages, leaving
  all four buttons for the active page's items.

Set `menu.layout=8`, `5`, or `4` in `controller.conf`. In four-button mode,
encoder press enters page-selection mode, turning selects a page, and another
press returns the encoder to its normal screen-specific operation. This makes
page selection deterministic without hiding row/list/choice adjustment.

Each screen remembers its last page. Entering or leaving a contextual editor
(FT2 cell/step entry, page target/channel choice, or pattern-clear confirmation)
predictably opens that context on page 1. Keyboard and mouse controls remain
available as secondary inputs.

The 12 verified synthv1 0.9.29 controls remain continuous configured CC
controls, separate from page selection and menu-item presses. They keep pickup,
preset-relative colors, and in-place reset behavior. The control architecture
reserves 16 entries, so four verified controls can be added later without
changing the four-by-four menu model.

`~` marks planned/unavailable items and `-` marks disabled informational slots.
Neither kind dispatches an action. `ARP` reserves the future performance
arpeggiator. `WAV LOOP` reserves future selection of one WAV, BPM detection or
entry, tempo synchronization, looping beside FT2 MIDI playback, replacement,
and synchronized stop; it is separate from the working stereo recorder.

### Complete screen menu map

| Screen/context | Page | Item 1 | Item 2 | Item 3 | Item 4 |
|---|---|---|---|---|---|
| Presets | Browse | Up | Down | Page up | Page down |
| Presets | Engine | Engine− | Engine+ | First | Last |
| Presets | Open | Load | Ideas | FT2 | Audio |
| Presets | Safety | Panic/stop synth | Exit | Disabled | Disabled |
| Playback | Idea | Record MIDI | Stop recording | Play take | Save idea |
| Playback | Sound | Reset 12 controls | Presets | Ideas | Arpeggiator (planned) |
| Playback | Open | FT2 | Audio | Tap tempo | Back |
| Playback | Safety | Stop take | Finish + save take | Panic/stop synth | Disabled |
| Ideas | Browse | Up | Down | First | Last |
| Ideas | Idea | Inspect | Load/confirm replace | Play | Delete/confirm |
| Ideas | Capture | Record | Stop recording | Save new | Presets |
| Ideas | Open | Back/cancel | FT2 | Audio | Panic |
| FT2 | Cursor | Row− | Row+ | Lane− | Lane+ |
| FT2 | Transport | Play here | Play from start | Stop/back | Cell edit |
| FT2 | Manage | Pages/tracks | Files | Mute lane | Tap tempo |
| FT2 | Adjust | Program− | Program+ | Tempo− | Tempo+ |
| FT2 edit | Cursor | Row− | Row+ | Lane− | Lane+ |
| FT2 edit | Entry | Blank/skip | Erase | Note off | Finish edit |
| FT2 edit | Transport | Play here | Play from start | Stop | Next visible page |
| FT2 edit | Adjust | Program− | Program+ | Tempo− | Tempo+ |
| FT2 cell edit | Fields | Note | Gate | Velocity | Program |
| FT2 cell edit | Effect | Effect type | Effect parameter | Clear selected field | Step entry |
| FT2 cell edit | Adjust | Previous field | Next field | Value− | Value+ |
| FT2 cell edit | Finish | Confirm | Cancel/back | Stop | Panic |
| Files | Browse | Up | Down | Load song | Back/cancel |
| Files | Song | Save/confirm overwrite | Preview/stop | Delete/confirm | Panic |
| Files | Pattern | New | Clone | Clear with meter confirmation | WAV loop (planned) |
| Files | Order | Previous | Next | Repeat current | Remove current |
| Pattern-clear context | Meter | 3/4 | 4/4 | Confirm | Cancel |
| Pattern-clear context | Current | Clear, keep size | Disabled | Disabled | Disabled |
| Pattern-clear context | Locked 3 | Disabled | Disabled | Disabled | Disabled |
| Pattern-clear context | Locked 4 | Disabled | Disabled | Disabled | Disabled |
| Pages/tracks | Pages | Page− | Page+ | Add four lanes | Cancel/restore |
| Pages/tracks | Route | Target | Channel | Done/keep | Files |
| Pages/tracks | Status | Mute page | Disabled | Disabled | Disabled |
| Pages/tracks | Future | Disabled | Disabled | Disabled | Disabled |
| Target/channel context | Edit | Previous | Next | Confirm field | Cancel field |
| Target/channel context | Locked 2–4 | Field mode (disabled) | Disabled | Disabled | Disabled |
| Audio recorder | Record | Record/toggle | Stop/finalize | Back | Panic |
| Audio recorder | Open | Presets | Ideas | FT2 | Disabled |
| Audio recorder | Status | 24-bit WAV (info) | Stereo (info) | Disabled | Disabled |
| Audio recorder | Future | Disabled | Disabled | Disabled | Disabled |

Musical controller notes remain the FT2 note/chord-entry mechanism while edit
is active; they are not menu slots. Command-pad note-on and note-off are
consumed, while unmapped musical MIDI continues through the configured route.

## What works now

### Instruments

SHR-DAW can browse and play:

- synthv1 presets;
- Yoshimi `.xiz` banks;
- FluidSynth `.sf2` and `.sf3` SoundFonts.

The Presets screen changes between the three engines without mixing their file
formats or control rules. The Playback screen shows held notes, chord names,
MIDI recording state, and the 12 hands-on synthv1 controls.

### MIDI routing

The routing setup and song pages can choose:

- the controller MIDI input;
- the MIDI input port that SHR-DAW sends notes and controls to for each software synth;
- a different hardware MIDI output and channel for every tracker page;
- the active SHR-DAW software instrument as a tracker-page target;
- a configured external MIDI output and optional drum note map;
- live thru, program changes, bank select, and MIDI transport;
- the left and right JACK playback ports;
- the left and right JACK recording ports.

The setup wizard finds ALSA MIDI ports and JACK audio ports. Exact names can
also be typed when automatic detection is not enough.

### FT2 pattern sequencer

The pattern screen is based on the quick top-to-bottom flow of FastTracker II.
It has rows, lanes, pages, an order list, step entry, note off, blank steps,
program changes, delay, retrigger, cut, tempo changes, mute, and looped play.

Every page has four note lanes. A new song starts with `MELODY` and `DRUMS`,
but it can have more pages. Each page stores its own target device and MIDI
channel. Pages can play together through several hardware outputs and the one
active SHR-DAW software instrument. A MIDI chord can fill up to four lanes in
one step. Notes keep their velocity and the cursor moves to the next row.

Choose **CELL EDIT** for the selected cell. Changes stay in a draft until
**CONFIRM**, while **CANCEL/BACK** restores the whole original cell. Choose a
field directly or use **FIELD−/FIELD+**, then use **VALUE−/VALUE+** (or the
encoder on eight- and five-button layouts). **CLEAR FLD** clears only the
selected field; the step editor's **ERASE** clears the whole cell. **STEP
EDIT** enters controller-note/chord entry. In four-button layout, encoder
press/turn remains reserved for selecting the four visible pages.

Editable fields and ranges are:

- note: empty, MIDI note 0–127, or note-off;
- gate: inherited song gate or 1–100% of one row;
- velocity: inherited page velocity or 0–127;
- program: inherited page program or a 0–127 per-note override;
- one command: none, cut tick 0–15, delay tick 0–15, retrigger count 1–8,
  or tempo 20–300 BPM.

Velocity, program, gate, and retrigger require a note-on in newly confirmed
edits. Unsupported combinations remain in the draft with a visible error. A
cell program sends the page bank/program selection to that exact target and
channel immediately before the affected note; it does not alter another page
or replace the inherited page program. Delayed gates end no later than the row
boundary, and every retrigger pulse receives a bounded sub-gate.

The first spacer after note/velocity shows `C` cut, `D` delay, `R` retrigger,
or `T` tempo; blank means no command. The format stores one command per cell,
so multiple simultaneous effects are not represented.

Open **PAGES** to select a page, add a four-lane page, choose a currently
available MIDI output or the active SHR-DAW instrument, and choose channel
1–16. Press **DONE** to keep the changes or **CANCEL** to restore the song as
it was. If saved hardware is unplugged, the page says `OFFLINE`; its notes and
saved target are kept and the page remains editable.

Patterns can use:

- 4/4 with 32 rows and beat marks at rows 1, 9, 17, and 25;
- 3/4 with 24 rows and beat marks at rows 1, 7, 13, and 19.

The pattern file screen can create a new pattern, clear a pattern, choose 3/4
or 4/4, preview a song, save, load, and delete. Songs keep all patterns and the
order in one readable text file.

The main workflow does not need a computer keyboard or mouse. The complete FT2,
edit, file, order, and page-management mappings are in the table above. The
encoder still moves rows and choices and confirms where applicable; all of its
former operations also have visible menu-item equivalents for four-button mode.

### MIDI ideas

The Ideas recorder saves free playing as MIDI. Each idea keeps timing, the
instrument reference, and synthv1 control values when they apply. Ideas can be
loaded and played through any active engine.

### Stereo audio

The Audio Recorder writes the selected JACK stereo pair as 24-bit WAV. The
JACK callback only moves samples into a fixed ring buffer. A normal disk thread
writes the file. The screen shows time, sample rate, file size, dropped frames,
and errors.

An interrupted recording stays as `.wav.part`. SHR-DAW tries to recover it on
the next recording start.

## Page routing and the first test rig

SHR-DAW is a Raspberry Pi mini DAW and MIDI routing hub, not a controller for
one keyboard. The Casiotone was only the proof-of-concept hardware device.
Current songs store page targets and channels directly. A reserved per-page
MIDI setup list is also preserved by the song format for later work, but there
is intentionally no large setup-message editor on the small tracker screen.

## FastTracker II credit

The pattern layout and editing flow are inspired by the legendary
[FastTracker II](https://demozoo.org/productions/99958/) for DOS. FastTracker II
was made by Fredrik “Mr.H” Huss and Magnus “Vogue” Högdahl of the demo group
Triton. Their tracker made fast vertical pattern editing feel natural, and that
is the feeling we want on a tiny hardware screen.

SHR-DAW is not an FT2 clone. It does not read XM files or use FT2 code. It uses
the row-and-lane idea for live MIDI sequencing on a Raspberry Pi.

## Install

SHR-DAW is developed and hardware-tested on Patchbox OS based on Debian 12
(Bookworm). A clean Debian 11 (Bullseye) ARM64 environment resolves all required
packages, builds the locked project with Rust 1.85, and passes its test suite,
but Bullseye has not been tested with audio/MIDI hardware. Raspberry Pi
OS Bullseye and Debian or Raspberry Pi OS 13 (Trixie) are expected to work but
have not been tested.

On Patchbox OS, Raspberry Pi OS, or Debian:

```sh
./scripts/install.sh
```

The installer adds the build tools, JACK and ALSA tools, synthv1, Yoshimi,
FluidSynth, and a small default SoundFont. It builds the release version and
opens the routing wizard.

Useful installer flags:

```sh
./scripts/install.sh --no-deps
./scripts/install.sh --no-config
```

Installed commands:

- `shr`: the SHR-DAW mini DAW;
- `shr-setup`: the routing wizard;
- `shs`: the old synthv1-only shell program.

JACK must be running before SHR-DAW starts. The wizard can write a backed-up
`~/.jackdrc` for a chosen USB or Raspberry Pi audio device, but it never starts
or restarts JACK.

## First run

Configure the hardware, check it, then start:

```sh
shr-setup
shr doctor
shr
```

Run `shr-setup` again after changing a controller, MIDI interface, sound card,
or JACK port layout.

For a self-contained development checkout:

```sh
./scripts/setup-local.sh
./scripts/local.sh
```

This keeps configuration, logs, ideas, songs, recordings, downloads, and local
presets under the ignored `user/` directory. Set `SHSYNTH_USER_DIR` to put that
tree somewhere else.

The product and Cargo package are named `shr-daw`, while the executable remains
`shr`. Existing `shsynth` configuration and data paths are retained for
compatibility with prior installations.

### Display and terminal size

SHR-DAW is designed for a 40×20 terminal and adapts its layout to the terminal
cell dimensions it receives. It reports when the terminal is too small, but the
current installer does not change font size or desktop/display settings.

Pixel resolution alone is not enough to choose a font: the terminal emulator,
window decorations, scaling, and fullscreen state also determine how many rows
and columns fit. A future setup step should detect the active output and
terminal, then offer a backed-up font/profile change appropriate to that
combination. Per-user terminal settings, such as LXTerminal's configuration on
Patchbox OS, normally do not need `sudo`; Linux console fonts and system-wide
display configuration may do. SHR-DAW must never silently change either.

### Optional Codex-assisted setup and recovery

`install.sh` and `shr-setup` are the normal installation path and are expected
to work without AI. For recovery, unusual hardware, or a heavily rewired rig,
users may optionally install and sign in to Codex CLI on the Raspberry Pi and
run the repository's assisted-setup brief. See the official
[Codex CLI documentation](https://developers.openai.com/codex/cli/) for current
installation and sign-in instructions. Then run:

```sh
cd /path/to/shr-daw
codex -C . "$(cat docs/CODEX_ASSISTED_SETUP.md)"
```

This is useful when editing the configuration by hand would mean discovering
and correlating many raw MIDI numbers: 12 continuous synth controls, the main
encoder and its press, a lock control, and as many as eight command pads. Codex
can observe the controller while the user moves one control at a time, identify
CCs, notes, and relative-encoder behavior, back up the existing files, and
write a checked `controller.conf`. It can also inspect JACK/ALSA routes, repair
failed dependencies, adapt terminal sizing, configure private SoundFonts, and
help with complex per-page MIDI routing.

The assisted path follows the same safety rules as SHR-DAW: machine-specific
values stay in user configuration, existing ideas and recordings are
preserved, downloads retain source and licence information, unrelated
processes are left alone, and audible JACK/synth tests require explicit user
permission. Generic installer bugs should be fixed and validated in the
project rather than hidden in a one-machine workaround.

Full interactive MIDI learn is planned, as is a USB device infobank containing
known controller/audio-interface identities and reviewed mapping profiles. The
infobank should make common devices automatic while MIDI learn handles unknown
ones. Codex-assisted discovery is the optional bridge for uncommon or deeply
customized setups until those features exist.

## Screens

- **Presets:** choose synthv1, Yoshimi, or FluidSynth and load a sound.
- **Playback:** play, view notes and chords, adjust synthv1, and record ideas.
- **Ideas:** load, preview, play, save, and delete MIDI ideas.
- **FT2:** edit and play patterns.
- **Pages:** add/select four-lane pages and set each target and channel.
- **Files:** manage songs and patterns.
- **Audio Recorder:** record the configured JACK stereo input.

The screen always shows its name, active page, all four page labels, and the
selected page's four item labels. The main encoder moves through lists and rows;
its press selects or confirms, or toggles page-selection mode in the configured
four-button layout.

The default MiniLab mapping is only an example. `controller.conf` can change
the controller input, encoder CCs, pad notes, lock button, and the 12 synthv1
control CCs.

## Configuration

User configuration lives in:

```text
${XDG_STATE_HOME:-~/.local/state}/shsynth/
```

The two main files are:

- `shsynth.conf`: engines, preset paths, MIDI routes, JACK routes, recording,
  and pattern output settings;
- `controller.conf`: controller input, encoder, pads, and mapped controls.

Important `shsynth.conf` groups:

| Group | Purpose |
|---|---|
| `synthv1.*` | synthv1 command, client, presets, and MIDI port |
| `yoshimi.*` | Yoshimi command, banks, categories, and MIDI port |
| `fluidsynth.*` | FluidSynth command, SoundFonts, gain, and MIDI port |
| `midi.*` | controller input and automatic routing |
| `audio.*` | JACK playback outputs |
| `external_midi.*` | configured tracker route, timing, program/bank and drum-map defaults |
| `capture.*` | JACK recording input and output directory |

List or change controller mappings with:

```sh
shr pads list
shr pads input "Controller port name"
shr pads layout 5
shr pads cc 20 74
shr pads set 51 page-cycle
shr pads set 52 item-1
shr pads clear 51
```

## Files

By default SHR-DAW stores:

- ideas in `${XDG_DATA_HOME:-~/.local/share}/shsynth/ideas/`;
- songs in `${XDG_DATA_HOME:-~/.local/share}/shsynth/songs/`;
- recordings in the configured `capture.directory`;
- runtime state and logs in the SHR-DAW state directory.

The public repository contains 21 project presets. A private local bank can be
set with `synthv1.presets` or `SHSYNTH_PRESET_DIR`. Yoshimi banks and
SoundFonts stay where they are installed and are read in place.

## Command line

```sh
shr menu
shr list
shr status
shr start "synthv1:Velvet Tines"
shr start "Yoshimi:Fat Bass"
shr stop
shr log 80
shr casio diagnostic
```

`shr casio diagnostic` is an old command name from the first test rig. It does
not send MIDI. It lists output ports and shows the messages that would be used.
The tracker UI itself is device-neutral and stores exact selected ports in the
song.

See [docs/CONFIGURATION.md](docs/CONFIGURATION.md) for page targets, the song
schema, and offline-device behavior.

## License

SHR-DAW code and the 21 included presets are MIT licensed. The larger private
legacy preset bank is not in Git because its archive has no clear license note.
See [THIRD_PARTY.md](THIRD_PARTY.md) when making packages or adding sounds.

For new synthv1 sounds, see [docs/NEW_PATCHES.md](docs/NEW_PATCHES.md).

---

<p align="center">
While I was releasing the first version of this software, my uncle died. So I dedicate this project to him.<br>
Počivao u miru, striče Mile, puno te volim!
</p>
