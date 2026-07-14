# SHSynth

SHSynth is a small Raspberry Pi mini DAW. It runs in a 40×20 terminal and is
made for hands-on use with a MIDI controller, a small screen, and an audio
interface.

It can play software instruments, route MIDI, sequence hardware, save MIDI
ideas, and record stereo audio. The goal is a portable music box that can grow
from one synth into a hub for many MIDI instruments.

The first test rig used an Arturia MiniLab, an AudioBox USB interface, and a
Casio Casiotone MT-240. That rig proved that the routing and pattern workflow
work on real hardware. SHSynth is not a controller made for that keyboard. All
device names, MIDI routes, channels, control maps, and JACK ports live in
configuration files.

## The idea

One Raspberry Pi can sit in the middle of a small setup:

```text
MIDI controller ──> SHSynth ──> synthv1 / Yoshimi / FluidSynth ──> JACK out
                         └────> FT2 pattern pages ──> MIDI hardware

JACK input ──> SHSynth stereo recorder ──> WAV
```

SHSynth reads the controller once. It keeps menu buttons and mapped controls
inside the app, then sends musical MIDI to the right place. This avoids double
notes and random direct connections from desktop MIDI tools.

Only one managed software synth runs at a time. Changing the synth sends All
Notes Off, stops the process started by SHSynth, and starts the next one.
SHSynth never kills a synth process that it does not own.

## What works now

### Instruments

SHSynth can browse and play:

- synthv1 presets;
- Yoshimi `.xiz` banks;
- FluidSynth `.sf2` and `.sf3` SoundFonts.

The Presets screen changes between the three engines without mixing their file
formats or control rules. The Playback screen shows held notes, chord names,
MIDI recording state, and the 12 hands-on synthv1 controls.

### MIDI routing

The routing setup and song pages can choose:

- the controller MIDI input;
- the MIDI destination for each software synth;
- a different hardware MIDI output and channel for every tracker page;
- the active SHSynth software instrument as a tracker-page target;
- a compatibility/default external MIDI output and optional drum note map;
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
active SHSynth software instrument. A MIDI chord can fill up to four lanes in
one step. Notes keep their velocity and the cursor moves to the next row.

Open **PAGES** to select a page, add a four-lane page, choose a currently
available MIDI output or the active SHSynth instrument, and choose channel
1–16. Press **DONE** to keep the changes or **CANCEL** to restore the song as
it was. If saved hardware is unplugged, the page says `OFFLINE`; its notes and
saved target are kept and the page remains editable.

Patterns can use:

- 4/4 with 32 rows and beat marks at rows 1, 9, 17, and 25;
- 3/4 with 24 rows and beat marks at rows 1, 7, 13, and 19.

The pattern file screen can create a new pattern, clear a pattern, choose 3/4
or 4/4, preview a song, save, load, and delete. Songs keep all patterns and the
order in one readable text file.

The main workflow does not need a computer keyboard. On the FT2 screen the
eight command pads are:

| Pad | Action |
|---|---|
| 1 | PAGES |
| 2 | EDIT |
| 3 | LANE− |
| 4 | LANE+ |
| 5 | STOP |
| 6 | PLAY |
| 7 | SAVE |
| 8 | TAP / ERASE while editing |

While editing, the main encoder moves through rows. Pressing it adds a blank
step. The ERASE pad clears the selected cell and moves down one row. Hold the
TAP pad and turn the encoder to change tempo.

The page-management pad layout is:

| Pad | Action |
|---|---|
| 1 | FILE |
| 2 | ADD four-lane page |
| 3 | PAGE− |
| 4 | PAGE+ |
| 5 | CANCEL / BACK |
| 6 | TARGET |
| 7 | CHANNEL |
| 8 | DONE / CONFIRM |

The main encoder also selects pages and changes target/channel choices. Its
press confirms the current choice.

### MIDI ideas

The Ideas recorder saves free playing as MIDI. Each idea keeps timing, the
instrument reference, and synthv1 control values when they apply. Ideas can be
loaded and played through any active engine.

### Stereo audio

The Audio Recorder writes the selected JACK stereo pair as 24-bit WAV. The
JACK callback only moves samples into a fixed ring buffer. A normal disk thread
writes the file. The screen shows time, sample rate, file size, dropped frames,
and errors.

An interrupted recording stays as `.wav.part`. SHSynth tries to recover it on
the next recording start.

## Page routing and the first test rig

SHSynth is a Raspberry Pi mini DAW and MIDI routing hub, not a controller for
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

SHSynth is not an FT2 clone. It does not read XM files or use FT2 code. It uses
the row-and-lane idea for live MIDI sequencing on a Raspberry Pi.

## Install

On Raspberry Pi OS or Debian:

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

- `shr` or `shsynth`: the Rust mini DAW;
- `shr-setup` or `shsynth-setup`: the routing wizard;
- `shs`: the old synthv1-only shell program.

JACK must be running before SHSynth starts. The wizard can write a backed-up
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

## Screens

- **Presets:** choose synthv1, Yoshimi, or FluidSynth and load a sound.
- **Playback:** play, view notes and chords, adjust synthv1, and record ideas.
- **Ideas:** load, preview, play, save, and delete MIDI ideas.
- **FT2:** edit and play patterns.
- **Pages:** add/select four-lane pages and set each target and channel.
- **Files:** manage songs and patterns.
- **Audio Recorder:** record the configured JACK stereo input.

The screen shows the current pad labels. The same eight pads can do different
jobs on different screens. The main encoder moves through lists and rows; its
press selects or confirms.

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
| `external_midi.*` | compatibility/default tracker route, timing, program/bank and drum-map defaults |
| `capture.*` | JACK recording input and output directory |

List or change controller mappings with:

```sh
shr pads list
shr pads input "Controller port name"
shr pads cc 20 74
shr pads set 51 rec
shr pads clear 51
```

## Files

By default SHSynth stores:

- ideas in `${XDG_DATA_HOME:-~/.local/share}/shsynth/ideas/`;
- songs in `${XDG_DATA_HOME:-~/.local/share}/shsynth/songs/`;
- recordings in the configured `capture.directory`;
- runtime state and logs in the SHSynth state directory.

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

See [docs/CONFIGURATION.md](docs/CONFIGURATION.md) for page targets, song
format compatibility, and offline-device behavior.

## License

SHSynth code and the 21 included presets are MIT licensed. The larger private
legacy preset bank is not in Git because its archive has no clear license note.
See [THIRD_PARTY.md](THIRD_PARTY.md) when making packages or adding sounds.

For new synthv1 sounds, see [docs/NEW_PATCHES.md](docs/NEW_PATCHES.md).
