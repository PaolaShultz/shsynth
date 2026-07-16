# Configuration and tracker routing

SHR-DAW is a Raspberry Pi mini DAW and MIDI routing hub. Hardware names belong
in `shsynth.conf`, `controller.conf`, or a saved song. They are not compiled
into the program.

Both configuration files use one `KEY=VALUE` entry per line. A comment must
start with `#` after optional leading whitespace; `#` inside a value is kept as
part of a hardware name or path rather than treated as an inline comment.

## Audio CPU isolation

`audio.engine_cpu` is an optional zero-based CPU number. When set, SHR-DAW pins
the one synth process it owns to that CPU before the process starts, allowing
its JACK threads to inherit the same affinity. It deliberately does not pin the
TUI, MIDI routing, or recording disk writer.

Do not set this key by itself and assume the CPU is reserved: ordinary kernel
and user work can still run there. On supported Raspberry Pi installations,
use `sudo shr-audio-tune install CPU` or the opt-in setup-wizard step. That tool
backs up the boot command line, configures JACK affinity and the performance
governor, and provides a matching `remove` operation. It never starts or
restarts JACK. Reboot after installing or removing the isolation settings.

## Controller menu layouts

`controller.conf` maps physical notes to controller roles, not screen actions.
The current screen and context install the four visible menu pages described in
the [complete controller map](CONTROLLER_INTERFACE.md#complete-controller-map).

Eight-button layout:

```text
menu.layout=8
pad.36=page-1
pad.37=page-2
pad.38=page-3
pad.39=page-4
pad.40=item-1
pad.41=item-2
pad.42=item-3
pad.43=item-4
```

Five-button layout:

```text
menu.layout=5
pad.36=page-cycle
pad.40=item-1
pad.41=item-2
pad.42=item-3
pad.43=item-4
```

Four-button layout:

```text
menu.layout=4
pad.40=item-1
pad.41=item-2
pad.42=item-3
pad.43=item-4
```

The note numbers above are examples only. Use the notes sent by the configured
controller. In four-button mode, press the configured encoder to enter visible
page-selection mode, turn it to choose page 1–4, and press it again to restore
normal list/row/choice operation. The main setup wizard loads a matching known
profile or offers MIDI learn; compact profiles can also be edited directly or
with `shr pads set NOTE ROLE`.

Older physical role aliases are accepted in physical order. New profiles should
use `page-1` through `page-4`, `page-cycle`, and `item-1` through `item-4`.
Command-note on and off remain consumed; unmapped musical notes pass through.
Disabled (`-`) and planned (`~`) entries never dispatch actions.

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

Controller buttons may send notes (`pad.N=ROLE`) or CCs
(`button.cc.N=ROLE`). `encoder.relative_reverse=true` supports relative
encoders whose clockwise messages are below 64, and `encoder.press_note=N`
supports encoder presses sent as notes. Normally `shr-setup` or `shr pads
learn` writes these details. See
[Automatic controller setup and MIDI learn](CONTROLLER_PROFILES.md).

## Tracker pages

Every FT2 page has four note lanes. Pages are stored inside each FT2 Pattern,
not globally on the Project. Pages in the current pattern play at the same time
and each page stores:

- its target;
- MIDI channel 1–16;
- bank, program, velocity, mute, and percussion settings;
- four lane names and lane mute states;
- a reserved list of MIDI setup messages for later use.

Open **PAGES** on the tracker screen. Use the main encoder or **PAGE−** and
**PAGE+** to select a page in the selected pattern. **ADD** creates another
four-lane page in that pattern. **TARGET**
chooses an ALSA MIDI output that is currently visible, the active SHR-DAW
software instrument, or the configured output. **CHANNEL**
chooses 1–16. Encoder press confirms a field. **DONE** keeps all page changes;
**CANCEL** restores the Project from before page management opened.

The active-instrument choice always means the single software instrument that
SHR-DAW currently owns and monitors. It does not start another engine. It is
offline when no managed instrument is active.

An exact hardware port name is saved in the song. If that device is later
missing, the page shows `OFFLINE`. SHR-DAW keeps the name and pattern data,
does not rewrite the file, and continues playing pages whose targets are
available.

## Configured output

The `external_midi.*` settings provide the configured route for new Projects
and newly created FT2 Patterns.
They also hold tracker timing, gate, bank/program, transport, live-thru, and
optional drum-map defaults.

The most important output keys are:

```text
external_midi.enabled=true
external_midi.client=shs-tracker
external_midi.output=part of the ALSA output port name
external_midi.melody_channel=1
external_midi.percussion_channel=10
```

These example values are not device requirements. Run `shr-setup` and choose
the ports present on the Raspberry Pi. The Casiotone profile in the bundled
example is only the original proof-of-concept profile.

`external_midi.profile` selects named program data for the configured route.
Use `roland-d-50` for a D-50, or leave an unknown id to retain the numeric
0–127 browser. JSON profiles are discovered, in override order, from
`SHSYNTH_DEVICE_PROFILE_DIR`, `${XDG_DATA_HOME}/shsynth/midi-devices/`, the
installed shared-data directory, and the checkout's `midi-devices/` directory.
Exact MIDI port targets can also match a profile's `port_matches` entries.

## Project files

Projects are stored as `.shsong` text files below
`${XDG_DATA_HOME:-~/.local/share}/shsynth/songs/`. The current development
format stores each FT2 Pattern as a self-contained unit with its own tempo,
meter, page targets, setup messages, four lanes per page, four column
channel/bank/program setups, and every cell field. Version 0 Projects with one
page-wide channel/bank/program migrate by copying that setup to all four
columns. Unknown newer versions and unknown fields are refused.

On **FILES**, **NEW PRJ** requires a second press and creates the next available
`project-001` style unsaved name. **SAVE AS** writes a non-overwriting
`<current-name>-copy-001` style copy and makes it current. Normal **SAVE** asks
for a second press only when it would replace an existing Project. Arrangement
repeat/remove operations live on the separate **ARRANGE** screen. **NAME**
accepts a printable display name while deriving a safe filename; an existing
Project is published under the new name without replacing a collision.

## FT2 WAV loop routing and storage

Loop hardware and source locations are configured rather than compiled in:

```text
loop.client=shs-loop
loop.import_directory=~/Music
loop.output=system:playback_1
loop.output=system:playback_2
```

Exactly two `loop.output` destinations are required when loading a loop. The
player owns only its JACK client and output ports; it never starts/restarts
JACK, layers a synth engine, or disconnects another client. Missing servers or
ports leave the MIDI tracker usable and produce a useful error.

`loop.import_directory` is only the browseable inbox. A chosen WAV is validated
and copied without replacement to
`${XDG_DATA_HOME:-~/.local/share}/shsynth/loops/`, or the matching
`SHSYNTH_USER_DIR` tree set by the local launcher. Songs retain the private
filename, source BPM, cut region, and bar placement offset. Disk I/O, decoding,
allocation, import, and auto-alignment analysis happen outside the JACK
callback.

**TOOLS** → **LOOP** → **REMOVE** requires a second press, clears the Project's
loop reference, and unloads the loop client. It never deletes the imported WAV
from private storage. **TOOLS** → **LIBRARY** is the separate physical cleanup
workflow. It lists only regular WAV files, marks current and saved-Project
references, rejects symlinks/unsafe paths, and requires confirmation before
deleting an unreferenced file.

Loop playback is native-speed and native-pitch. Import and auto-align set the
current Pattern tempo from the interpreted WAV BPM; they do not stretch the WAV
to the previous Project tempo. The loop player also requires the JACK server
sample rate to match the WAV sample rate. Choose 44100 Hz in JACK setup for
44.1 kHz loops, or 48000 Hz for 48 kHz loops, and restart JACK yourself when it
is safe.

## FT2 cell fields

The contextual **CELL EDIT** menu is four pages of four positions: **OPS**
(Confirm, Step entry, Clear field, Effect type), **FIELDS** (Note, Gate,
Velocity, Program), **ADJUST** (Effect parameter, Value−, Value+), and **SYS**
(Panic, Stop, Exit/cancel). Empty positions are silent. Confirm writes the
draft; Exit/cancel discards it without leaving a preview note. STOP only stops
transport and deliberately preserves the draft.

Gate is inherited or 1–100% of a row. Velocity and program are inherited or
MIDI 0–127. The single command field supports cut `C` and delay `D` ticks
0–15, retrigger `R` counts 1–8, and tempo `T` values 20–300 BPM. The letter is
shown in the first spacer after velocity; blank means no command. Multiple
commands in one cell are not supported. Per-cell program overrides use the
selected column bank and exact page destination/column channel, occur before
that note, and do not mutate the inherited column program.

Choosing **Program** replaces the grid with a named program browser. Controller
notes are routed to the selected page target/column channel for live audition without
being inserted into the pattern or duplicated through the generic live-thru
route. Unknown devices still show every numeric MIDI program. The next played
note receives the current draft bank/program selection, which is transmitted
as soon as the browser selection changes; confirm commits the cell and cancel
restores its original value and selection.

## Note ownership

Tracker events are sent only to their page target and channel. Controller
command pads, the encoder, and mapped controls stay inside SHR-DAW. STOP, page
mute, lane mute, song replacement, route changes, and exit release notes only
on affected destinations. Lanes that share a device/channel keep separate note
ownership; a shared note is released only after its last lane owner ends.

FT2 **REC** is deliberately hardware-only. It refuses an `ActiveInstrument`
page, consumes musical controller notes before the loaded synth route, and
auditions them on the current page's configured/exact MIDI output and channel.
Recording loops only the selected pattern, writes only the visible page's four
lanes, and does not advance through or alter other order entries.

Pattern setup offers 4/4 row counts of 8, 16, 32, 64, and 128, or matching 3/4
counts of 6, 12, 24, 48, and 96. New patterns are distinct pattern records and
are appended to the song order; clone duplicates the selected pattern, while
repeat adds another order reference to the same pattern.
