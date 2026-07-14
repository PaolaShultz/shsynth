<p align="center">
  <img src="docs/images/shr-daw-header.png" alt="shr-daw" width="100%">
</p>

SHR-DAW is a small Raspberry Pi mini DAW that runs in a 40×20 terminal. A MIDI
controller, external synthesizer, dedicated screen, and USB audio interface can
all be added, but none of them is required to start. The name means **Shome
Rust DAW**.

It can play software instruments, route MIDI, sequence hardware, save MIDI
ideas, and record stereo audio. The goal is a portable music box that can grow
from one synth into a hub for many MIDI instruments.

## Start with what you have

The large hardware diagrams below show a fully expanded example, not a shopping
list. Useful configurations include:

- **Software only:** run the terminal locally or over SSH, load synthv1,
  Yoshimi, or FluidSynth, and send its JACK output to any configured Pi,
  HDMI, or USB audio output.
- **Computer keyboard only:** navigate the complete UI and use FT2 step edit to
  enter notes with `Z S X D C V G B H N J M`, then play the pattern through the
  active software instrument. No MIDI keyboard, mouse, or external MIDI device
  is required.
- **Add a MIDI keyboard:** play and record with velocity and chords instead of
  entering every step from the computer keyboard.
- **Add hardware when useful:** a control surface, external synths, audio
  interface, mixer, capture input, and dedicated display are independent
  upgrades rather than prerequisites.

Computer-keyboard step entry works now. Free live performance of a software
synth from the computer keyboard, a wider key range, and expanded bindings such
as F1–F12 are planned; the README does not treat them as finished features.

<p align="center">
  <img src="docs/images/shr-daw-ft2-tracker.png" alt="shr-daw FT2-style four-lane tracker" width="480">
</p>

The first test rig used an Arturia MiniLab, an AudioBox USB interface, and a
Casio Casiotone MT-240. That rig proved that the routing and pattern workflow
work on real hardware. SHR-DAW is not a controller made for that keyboard. All
device names, MIDI routes, channels, control maps, and JACK ports live in
configuration files.

## An expanded setup

One Raspberry Pi can eventually sit in the middle of a much larger setup. Every
device around it in this example is optional:

![Physical SHR-DAW connections: controller, Raspberry Pi, display, USB audio/MIDI interface, chained MIDI instruments, mixer, monitors, and headphones](docs/images/shr-daw-physical-connections.png)

On a narrow mobile screen, the same optional paths are:

- controller or computer-keyboard input → SHR-DAW;
- SHR-DAW → synthv1, Yoshimi, or FluidSynth → JACK audio output;
- FT2 pages → optional external MIDI hardware;
- optional audio input → direct monitoring and/or the stereo WAV recorder.

When a MIDI controller is present, SHR-DAW reads it once. It keeps menu buttons
and mapped controls inside the app, then sends musical MIDI to the right place.
This avoids double notes and random direct connections from desktop MIDI tools.

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

- available menu pages and the selected page's working actions are visible;
  empty positions are omitted;
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

Every screen uses the same four-page/four-item controller model. Page 1 is the
main `OPS` page. On child screens and editors, `EXIT` is always page 4/item 4
and returns one level; it never quits the application. Empty items and pages
are hidden and skipped. The on-screen controls stay compact at wide terminal
sizes.

Page selection adapts to the configured controller:

- the current eight-button layout can use four buttons for direct, fast page
  selection and four buttons for the active page's items;
- a five-button controller can use one button to cycle pages and four buttons
  for the active page's items;
- a four-button controller can use the rotary encoder to switch pages, leaving
  all four buttons for the active page's items.

Set `menu.layout=8`, `5`, or `4` in `controller.conf`. The complete action map,
editor behavior, and compatibility rules are in
[`docs/CONTROLLER_INTERFACE.md`](docs/CONTROLLER_INTERFACE.md).

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
- data-driven MIDI device profiles with named program browsing;
- the left and right JACK playback ports;
- the left and right JACK recording ports.

The setup wizard finds ALSA MIDI ports and JACK audio ports. Exact names can
also be typed when automatic detection is not enough. When configuring an
external hardware output it also offers installed device-profile ids; the
numeric fallback remains available for unlisted instruments.

### FT2 pattern sequencer

The pattern screen is based on the quick top-to-bottom flow of FastTracker II.
It has rows, lanes, pages, an order list, step entry, real-time record, note
off, blank steps, program changes, delay, retrigger, cut, tempo changes, mute,
and looped play.

Every page has four note lanes. A new song starts with `MELODY` and `DRUMS`,
but it can have more pages. Each page stores its own target device and MIDI
channel. Pages can play together through several hardware outputs and the one
active SHR-DAW software instrument. A MIDI chord can fill up to four lanes in
one step. Notes keep their velocity and the cursor moves to the next row.

Choose **REC** to loop and record only the selected pattern and visible page.
Played notes are quantized to its rows and distributed across that page's four
lanes. During REC they are consumed before the loaded software instrument and
sent only to the page's hardware MIDI target/channel. A page targeting the
active SHR-DAW instrument cannot enter REC; choose a configured or exact MIDI
output first. **STOP REC**, **STOP**, **EXIT**, and **PANIC** release auditioned
notes safely.

Choose **CELL EDIT** for the selected cell. Changes stay in a draft until
**CONFIRM**, while **EXIT** restores the whole original cell. Choose a field
directly, then use **VALUE−/VALUE+** (or the encoder on eight- and five-button
layouts). **CLEAR** clears only the selected field; the step editor's
**ERASE** clears the whole cell. **STEP** enters controller-note/chord entry.
In four-button layout, encoder press/turn remains reserved for selecting the
available menu pages.

Editable fields and ranges are:

- note: empty, MIDI note 0–127, or note-off;
- gate: inherited song gate or 1–100% of one row;
- velocity: inherited page velocity or 0–127;
- program: inherited page program or a 0–127 per-note override;
- one command: none, cut tick 0–15, delay tick 0–15, retrigger count 1–8,
  or tempo 20–300 BPM.

Selecting **PROGRAM** opens a full-height sound browser. If the page target has
a matching JSON device profile, it shows the device's native slot labels and
sound names; otherwise it remains a generic MIDI 0–127 browser. Incoming MIDI
notes stay free for auditioning on that exact page target and channel while the
browser is open. Turning the value changes the draft sound heard by the next
note by transmitting its bank/program selection immediately. **CONFIRM** stores
the program in the cell and **CANCEL** restores the original cell and selection.
Profiles are data files under `midi-devices/`, not device logic compiled into
the tracker.

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

New or resized patterns can use 8, 16, 32, 64, or 128 rows in 4/4, and the
matching 6, 12, 24, 48, or 96 rows in 3/4.

The pattern file screen can create, clone, or clear a pattern; choose its meter
and size; edit the multi-pattern order; preview a song; save; load; and delete.
Songs keep every distinct pattern and the full order in one readable text file.

The main workflow does not need a computer keyboard or mouse. The complete FT2,
edit, file, order, and page-management mappings are in the table above. The
master rotary moves rows, lists, pages, and choices and confirms where
applicable. Menu slots are reserved for operations that are not the same
previous/next selection again.

### MIDI ideas

The Ideas recorder saves free playing as MIDI. Each idea keeps timing, the
instrument reference, and synthv1 control values when they apply. Ideas can be
loaded and played through any active engine.

### Stereo audio

The Audio Recorder writes the selected JACK stereo pair as 24-bit WAV. The
JACK callback only moves samples into a fixed ring buffer. A normal disk thread
writes the file. The screen shows time, sample rate, file size, dropped frames,
and errors.

Live line input is deliberately monitored in the audio interface, not routed
through SHR-DAW or from JACK capture back to JACK playback. The interface's
direct-monitor balance makes it easy to mix external instruments with SHR-DAW's
software instruments without adding a software-monitoring process or its
latency and CPU cost. JACK capture remains available to the Audio Recorder.
Software monitoring can be added later if a routing or effects use case makes
it worthwhile.

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
- `shr-audio-tune`: reversible Raspberry Pi dedicated-core audio tuning;
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

### Optional dedicated audio CPU

On a Raspberry Pi with at least four cores, the setup wizard can reserve one
CPU for JACK and SHR-DAW's single managed synth engine. The opt-in profile:

- pins JACK and the managed synth to the selected CPU;
- keeps normal IRQ handling on the other CPUs;
- enables full-tickless and RCU offload for the audio CPU at boot;
- selects the `performance` frequency governor while its systemd service is
  active;
- preserves the original boot command line and refuses to replace isolation
  settings it did not create.

The wizard never restarts JACK or reboots. Inspect or reverse the profile with:

```sh
shr-audio-tune status
sudo shr-audio-tune remove
```

After removal, clear `audio.engine_cpu` in `shsynth.conf` and reboot. CPU
isolation reduces the machine to three housekeeping cores, so it is optional
and disabled by default. It improves scheduling isolation but cannot guarantee
that faulty hardware, firmware, or a too-small JACK buffer will never xrun.

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

## External MIDI device profiles

Set `external_midi.profile` to a profile id for the configured hardware route.
The first bundled profile is `roland-d-50`. It contains the 64 original D-50
internal patch names and the 64 card-memory slots. The D-50 internal and RAM
card memories are writable, so names shown for `I-11`–`I-88` describe the
original factory data; `C-11`–`C-88` deliberately remain named only as card
slots. Both groups are selectable: D-50 Program Change 0–63 addresses internal
memory and 64–127 addresses card memory.

Additional profiles can be placed in `${XDG_DATA_HOME}/shsynth/midi-devices/`
or a directory named by `SHSYNTH_DEVICE_PROFILE_DIR`. Installed profiles live
under `share/shsynth/midi-devices/`. A profile supplies labels and MIDI metadata;
the FT2 editor and live audition route remain generic. The schema and sourcing
rules are documented in
[docs/MIDI_DEVICE_PROFILES.md](docs/MIDI_DEVICE_PROFILES.md).

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
