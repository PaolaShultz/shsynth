# Configuration and tracker routing

SHR-DAW is a Raspberry Pi mini DAW and MIDI routing hub. Hardware names belong
in `shsynth.conf`, `controller.conf`, or a saved Project. They are not compiled
into the program.

Both configuration files use one `KEY=VALUE` entry per line. A comment must
start with `#` after optional leading whitespace; `#` inside a value is kept as
part of a hardware name or path rather than treated as an inline comment.

The installed templates live under `share/shsynth/`. On first use, `shr`
copies them without replacing existing files to
`${XDG_STATE_HOME:-~/.local/state}/shsynth/`. A repository-local launch uses
the checkout's `config/` and private `user/` tree instead. Environment
overrides are documented in [Installation](INSTALLATION.md).

## Runtime key reference

Repeated `midi.input`, `audio.output`, `yoshimi.preset_root`,
`yoshimi.category`, `fluidsynth.soundfont`, `external_midi.channel`,
`external_midi.percussion_note`, `capture.input`, and `loop.output` keys build
ordered lists. Empty optional values disable that choice. The current parser
accepts:

| Group | Keys and constraints |
| --- | --- |
| Startup and status | `synth.startup_timeout_ms`; optional `status.cpu_temperature_path` |
| Display | `display.note_names` (`german` for B/H or `english` for A#/B) |
| synthv1 | `synthv1.command`, `.client`, `.presets`, `.midi_output`; legacy `synth.command`, `synth.client`, `presets.directory`, and `midi.synth_output` remain accepted |
| Yoshimi | `yoshimi.command`, `.client`, `.midi_output`, repeated `.preset_root` and `.category`, `.presets_per_category` |
| FluidSynth | `fluidsynth.command`, `.client`, `.midi_output`, `.gain`, repeated `.soundfont` |
| Managed MIDI/audio | `midi.autoconnect`, repeated `midi.input`; `audio.autoconnect`, repeated `audio.output`; optional `audio.engine_cpu` |
| Owned graph | `audio.graph.enabled`, `.client`, `.maximum_callback_frames` (1–4096) |
| External tracker MIDI | `external_midi.enabled`, `.client`, `.output`, `.max_tracks`, repeated `.channel`, `.melody_channel`, optional `.percussion_channel` and `.percussion_program`, `.percussion_input_base`, repeated `.percussion_note`, `.bank_select` (`off`, `cc0`, or `cc0+cc32`), `.program_changes`, `.send_transport`, `.default_tempo` (20–300), `.pattern_rows` (1–256), `.steps_per_beat` (1–16), `.live_thru`, `.profile`, `.gate_percent` (1–100), `.gesture_settle_ms` |
| Stereo capture | `capture.directory`, `.client`, repeated `capture.input=NAME|LEFT|RIGHT`, `.ring_frames` (1024–4194304) |
| WAV loop | `loop.client`, `loop.import_directory`, exactly two repeated `loop.output` entries when playback is used |

Boolean values are `true` or `false`; numbers and structured entries are
rejected when malformed or out of range. Commands, clients, paths, and ports
remain data: copy the template and change them for the actual machine instead
of editing Rust constants.

`display.note_names` changes Playback chord roots, slash bass notes, and the
held-note row together. It does not transpose MIDI or alter the keyboard-state
positions. The default is `german`, matching the existing central-European B
and H convention; `english` names those pitch classes A# and B.

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

## Owned audio graph

The opt-in SHR-owned JACK client processes one managed software instrument,
its Project-persisted source insert rack, two aux buses, and master rack. Each
aux has an independent pre/post source-insert send, forced-wet rack, return
gain, and return meter. The dry-plus-return sum passes through the master rack
and final post-master meter. It remains disabled by default after the measured
Raspberry Pi checkpoints:

```text
audio.autoconnect=true
audio.output=system:playback_1
audio.output=system:playback_2
audio.graph.enabled=false
audio.graph.client=shr-graph
audio.graph.maximum_callback_frames=4096
```

Exactly two `audio.output` entries are both the conservative direct fallback
and the graph's main destinations. Enabling the graph requires those two
entries and direct autoconnection. The callback frame bound may be 1–4096 and
must cover the active JACK period; an unexpectedly larger callback is counted
and written as silence rather than overrunning fixed memory.

On a managed-engine load, SHR-DAW first establishes direct playback. The graph
client stays muted while its exact source/input/output links are connected,
then both direct links are removed transactionally before graph output is
published at a callback boundary. Validation, client activation, exact port
resolution, or connection failure leaves or restores direct playback. Graph
shutdown restores only those two managed direct links and closing the owned
client releases its own ports. The callback is deactivated before those direct
links are restored, preventing a final graph block from doubling the source;
unrelated JACK clients and connections are not changed.

An orderly `shr stop` writes the owned graph's
callback count, mean, p95, p99, maximum, missed-deadline count, and oversized
callback count to the private `engine.log`. The FX rack/editor remains
available while the graph is disabled, and those validated Project edits do
not publish a runtime plan. With the graph enabled, stop transport and all
recording before an FX change can rebuild and publish the owned graph. Projects
save their racks in either mode, but direct playback does not process or meter
them.

Each rack holds at most eight effects; the complete graph holds at most 16,
including at most two reverbs. Source and master racks offer Utility, EQ,
Compressor, Distortion, Delay, Reverb, Chorus, Flanger, Phaser, Tremolo/Pan,
Filter, Gate, and Crusher. Aux creation offers Delay, Reverb, Chorus, Flanger,
and Phaser and fixes their wet signal to 100% and dry signal to zero so an aux
return never doubles the source. `BYPASS` fades a source/master effect toward
dry passthrough. An all-bypassed aux is silent; a delay configured to keep its
tail may drain that wet tail with new input muted. See the
[audio graph contract](AUDIO_GRAPH.md) for exact schemas, publication rules,
meters, and topology limits.

Do not enable this merely to perform a routine setup check. The first
authorized dry-path comparison is recorded in
[Phase 1 dry audio graph measurement](PHASE1_AUDIO_GRAPH_MEASUREMENT.md).
Phase 2's software and Pi performance gates are recorded in
[Phase 2 insert-effects measurement](PHASE2_AUDIO_GRAPH_MEASUREMENT.md). The
time/modulation, reverb, aux, and master evidence is recorded in
[Phase 3/4 effects measurement](PHASE3_4_AUDIO_GRAPH_MEASUREMENT.md).

The maintainer-only low-gain performance command is:

```sh
shr effects-checkpoint ENGINE:PRESET [PROFILE] [SECONDS]
```

Phase 2 profiles are `dry`, `eq`, `compressor`, `soft-cubic`, `hard-clip`,
`asymmetric`, `gate`, `filter-lp`, `filter-bp`, `filter-hp`, `crusher`, and
`full`. Expanded profiles are `delay`, `chorus`, `flanger`, `phaser`,
`tremolo`, `autopan`, `time-full`, `reverb-room`, `reverb-plate`,
`reverb-hall`, `two-reverbs`, and `phase4-full`. The last profile deliberately
uses eight source inserts, two reverb buses, and one master compressor.
Duration is bounded to 1–60 seconds. The command enables the graph only
in its cloned in-memory configuration, sends note 48 at velocity 8, measures
the owned graph and synth processes, restores the exact direct route, and stops
only the engine it owns. It does not persist graph enablement or JACK settings.

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
- four column channel/bank/program setups, with channels 1–16;
- page velocity, mute, and percussion settings;
- four lane names and lane mute states;
- a reserved list of MIDI setup messages for later use.

Open **TOOLS** → **PAGES** from FT2. The resulting **TRACKS** screen edits
pages and columns. Use the main encoder to select a page. **ADD** creates
another four-lane page in that Pattern. **TARGET**
chooses an ALSA MIDI output that is currently visible, the active SHR-DAW
software instrument, or the configured output. **CHANNEL**
chooses 1–16. Encoder press confirms a field. **DONE** keeps all page changes;
**SYS** → **EXIT** restores the Project from before TRACKS opened. On the
**COLUMN** and **BANK** pages, **COL−/COL+**, **PROG−/PROG+**, and the bank
controls edit the selected column. In a target/channel chooser, **CONFIRM**
keeps that field and **EXIT** cancels it.

The active-instrument choice always means the single software instrument that
SHR-DAW currently owns and monitors. It does not start another engine. It is
offline when no managed instrument is active.

An exact hardware port name is saved in the Project. If that device is later
missing, the page shows `OFFLINE`. SHR-DAW keeps the name and pattern data,
does not rewrite the file, and continues playing pages whose targets are
available. If multiple ports have the same exact name, or a configured partial
match selects more than one port, the target is ambiguous and no port is chosen.

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
the ports present on the Raspberry Pi. The configuration template retains the
original Casiotone proof-of-concept profile id, but no Casiotone named-program
JSON is distributed; without a private matching profile, the browser correctly
falls back to numeric programs 0–127.

`external_midi.profile` selects named program data for the configured route.
Use `roland-d-50` for a D-50, or leave an unknown id to retain the numeric
0–127 browser. JSON profiles are discovered, in override order, from
`SHSYNTH_DEVICE_PROFILE_DIR`, `${XDG_DATA_HOME}/shsynth/midi-devices/`, the
installed shared-data directory, and the checkout's `midi-devices/` directory.
Exact MIDI port targets can also match a profile's `port_matches` entries.

## Project files

Projects are stored as `.shsong` text files below
`${XDG_DATA_HOME:-~/.local/share}/shsynth/songs/`. Current Project format 3
stores each FT2 Pattern as a self-contained unit with its own tempo,
meter, page targets, setup messages, four lanes per page, four column
channel/bank/program setups, every cell field, the source insert rack, aux
routing, and master rack. Versions 0 and 1 migrate with empty effects routing;
version 2 retains its source rack and gains empty aux/master routing. Version 0
page-wide channel/bank/program data is copied to all four columns. Unknown
newer versions, unknown fields, and invalid effect data are refused rather than
partly loaded or written back.

On **FILES**, **NEW PRJ** requires a second press and creates the next available
`project-001` style unsaved name. **SAVE AS** writes a non-overwriting
`<current-name>-copy-001` style copy and makes it current. Normal **SAVE** asks
for a second press only when it would replace an existing Project. Arrangement
repeat/remove operations live on the separate **ARRANGE** screen. **NAME**
accepts a printable display name while deriving a safe filename; an existing
Project is published under the new name without replacing a collision.

Reusable drum patterns are independent `.shdrum` files. Bundled patterns are
installed below `share/shsynth/drum-patterns/`; controller-created user saves
go below `${XDG_DATA_HOME}/shsynth/drum-patterns/`. They store four lanes of
cells plus meter and row count, but deliberately do not store MIDI destinations,
channels, banks, or programs. Loading therefore keeps the current percussion
page's hardware routing intact. The installed `.shrdrums` catalog is a compact
authored collection used to build filtered 24/48/96-row 3/4 and 32/64/128-row
4/4 phrase variations. It is read-only like the individual bundled grooves.

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
`SHSYNTH_USER_DIR` tree set by the local launcher. Projects retain the private
filename, source BPM, cut region, and bar placement offset. Disk I/O, decoding,
allocation, import, and auto-alignment analysis happen outside the JACK
callback.

**TOOLS** → **LOOP** → **REMOVE** requires a second press, clears the Project's
loop reference, and unloads the loop client. It never deletes the imported WAV
from private storage. On FT2 Tools, the **LOOP** menu page's **LIBRARY** action
opens the separate physical cleanup workflow. It lists only regular WAV files,
marks current and saved-Project
references, rejects symlinks/unsafe paths, and requires confirmation before
deleting an unreferenced file.

Loop playback is native-speed and native-pitch. Import and auto-align set the
current Pattern tempo from the interpreted WAV BPM; they do not stretch the WAV
to the previous Project tempo. The loop player also requires the JACK server
sample rate to match the WAV sample rate. Choose 44100 Hz in JACK setup for
44.1 kHz loops, or 48000 Hz for 48 kHz loops, and restart JACK yourself when it
is safe. Decoded loop data is capped at 6,000,000 frames, about 46 MiB of stereo
sample memory and 125 seconds at 48 kHz.

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
mute, lane mute, Project replacement, route changes, and exit release notes only
on affected destinations. Lanes that share a device/channel keep separate note
ownership; a shared note is released only after its last lane owner ends.

FT2 **REC** is deliberately hardware-only. It refuses an `ActiveInstrument`
page, consumes musical controller notes before the loaded synth route, and
auditions them on the current page's configured/exact MIDI output and channel.
Recording loops only the selected pattern, writes only the visible page's four
lanes, and does not advance through or alter other order entries.

Pattern setup offers 4/4 row counts of 8, 16, 32, 64, and 128, or matching 3/4
counts of 6, 12, 24, 48, and 96. New patterns are distinct pattern records and
are appended to the Arrangement; clone duplicates the selected Pattern, while
repeat adds another order reference to the same pattern.
