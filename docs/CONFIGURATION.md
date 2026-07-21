# Configuration and tracker routing

SHR-DAW is a Raspberry Pi mini DAW and MIDI routing hub. Machine defaults and
hardware fallbacks belong in `shsynth.conf` or `controller.conf`; only a page
deliberately bound to exact hardware stores that preferred target in a Project.
Names are not compiled into the program.

Both configuration files use one `KEY=VALUE` entry per line. A comment must
start with `#` after optional leading whitespace; `#` inside a value is kept as
part of a hardware name or path rather than treated as an inline comment.

The installed templates live under `share/shsynth/`. On first use, `shr`
copies them without replacing existing files to
`${XDG_STATE_HOME:-~/.local/state}/shsynth/`. A repository-local launch uses
the checkout's `config/` and private `user/` tree instead. Environment
overrides are documented in [Installation](INSTALLATION.md).

## Runtime key reference

Repeated `midi.performance_input`, `audio.output`, `audio.internal_output`, `yoshimi.preset_root`,
`yoshimi.category`, `fluidsynth.soundfont`, `external_midi.channel`,
`external_midi.percussion_note`, `capture.input`, `capture.track`, and `loop.output` keys build
ordered lists. Empty optional values disable that choice. The current parser
accepts:

| Group | Keys and constraints |
| --- | --- |
| Startup and status | `synth.startup_timeout_ms`; optional `status.cpu_temperature_path` |
| Display | `display.note_names` (`german` for B/H or `english` for A#/B) |
| synthv1 | `synthv1.command`, `.client`, `.presets`, `.midi_output`; legacy `synth.command`, `synth.client`, `presets.directory`, and `midi.synth_output` remain accepted |
| Yoshimi | `yoshimi.command`, `.client`, `.midi_output`, repeated `.preset_root` and `.category`, `.presets_per_category` |
| FluidSynth | `fluidsynth.command`, `.client`, `.midi_output`, `.gain`, repeated `.soundfont` |
| Managed MIDI/audio | `midi.autoconnect`; legacy ordered controller fallbacks in repeated `midi.input`; `midi.controller_musical_input`; simultaneous repeated `midi.performance_input`; `audio.autoconnect`, exactly two preferred `audio.output` entries, ordered `audio.internal_output=NAME|LEFT|RIGHT` fallbacks, final optional `audio.headphone_output=NAME|LEFT|RIGHT`; optional `audio.engine_cpu` |
| Owned final bus | `audio.graph.enabled`, `.client`, `.maximum_callback_frames` (1–4096), `.input`, monitoring confirmations |
| External tracker MIDI | `external_midi.enabled`, `.client`, `.output`, `.max_tracks`, repeated `.channel`, `.melody_channel`, optional `.percussion_channel` and `.percussion_program`, `.percussion_input_base`, repeated `.percussion_note`, `.bank_select` (`off`, `cc0`, or `cc0+cc32`), `.program_changes`, `.send_transport`, `.default_tempo` (20–300), `.pattern_rows` (1–256), `.steps_per_beat` (1–16), `.live_thru`, `.profile`, `.gate_percent` (1–100), `.gesture_settle_ms` |
| Controller clock | `controller_clock.enabled`, `.client`, `.output`; disabled by default, with one exact stable ALSA MIDI output name required when enabled |
| Synchronized capture | `capture.directory`, `.client`, repeated `capture.track=ID|LABEL|GROUP|ROLE|ARMED|EXACT_SOURCE`, legacy stereo `capture.input=NAME|LEFT|RIGHT`, `.ring_frames` (1024–4194304), `.maximum_callback_frames` (16–65536) |
| WAV loop | `loop.client`, `loop.import_directory`, exactly two repeated `loop.output` entries when playback is used |

Boolean values are `true` or `false`; numbers and structured entries are
rejected when malformed or out of range. Commands, clients, paths, and ports
remain data: copy the template and change them for the actual machine instead
of editing Rust constants.

`display.note_names` changes Playback chord roots, slash bass notes, and the
held-note row together. It does not transpose MIDI or alter the keyboard-state
positions. The default is `german`, matching the existing central-European B
and H convention; `english` names those pitch classes A# and B. `shr-setup`
asks with the two example scales `C D E F G A B C` and
`C D E F G A H C (B means B-flat)` and writes this key.

## Controller and performance MIDI inputs

`controller.conf input=` is the explicit control-surface selector. When it is
empty, repeated `midi.input=` values are tried in order as legacy alternatives;
they do not open multiple devices. The default
`midi.controller_musical_input=true` preserves combined-device behavior:
mapped commands are consumed and unmatched notes or performance messages pass
to the active route. Set it to `false` for a control-only surface.

Each non-empty repeated `midi.performance_input=` is an independent musical
source and is opened when available. Performance sources never enter controller
command, encoder, mapped-control, or learning interpretation. The same exact
resolved ALSA port may be named for both roles; SHR deduplicates it to one
connection. Partial absence and ambiguous substring matches are reported per
role without disabling the other inputs or the computer keyboard.

```text
midi.autoconnect=true
midi.input=Control Surface MIDI
midi.controller_musical_input=false
midi.performance_input=Performance Keyboard MIDI
midi.performance_input=Second Keyboard MIDI
```

For a combined controller/keyboard, omit the performance entries and leave the
controller musical setting true. For keyboard-only use, leave both controller
selectors empty, set the controller musical setting false, and configure one or
more performance inputs. `external_midi.output` is an output destination, not
an input; one interface may safely be configured in both directions.

## Dedicated controller clock and transport

The controller sync route is separate from every tracker page and from the
managed instrument. It opens only the one exact standard-MIDI output selected
by `controller_clock.output`; it never broadcasts and never falls back to a
tracker, synth, DIN THRU, MCU/HUI, or ALV port. Its protocol is deliberately
closed: `F8` Timing Clock, `FA` Start, and `FC` Stop are the only bytes the
connection can send. SHR uses directly addressed ALSA sequencer events from a
non-exportable source port, so an automatic JACK bridge cannot subscribe to or
copy the clock. It cannot send notes, CCs, bank/program selection, Song
Position Pointer, feedback, identity requests, or configuration SysEx.

```text
controller_clock.enabled=false
controller_clock.client=shs-controller-clock
controller_clock.output=
```

`shr clock ports` performs read-only output discovery. Each `current:` line is
what ALSA reports now; the paired `configure:` value removes only the trailing
volatile ALSA client address and remains an exact client-and-port-name match.
If zero or multiple ports have that stable exact name, SHR leaves controller
clock offline rather than guessing. For the MiniLab 3 choose its standard
`Minilab3 MIDI` endpoint, never `DIN THRU`, `MCU/HUI`, or `ALV`.

SHR's tracker transport is the authority. Every accepted Play, Play from start,
or Record transport launch is a fresh run, so it sends one `FA`; SHR has no
paused position that could truthfully use `FB` Continue. Stop sends one `FC`.
Clean shutdown sends `FC` if transport is still running. Pattern repeats remain
one continuous transport and do not emit another Start. `F2` Song Position
Pointer is not used: the MiniLab arpeggiator needs tempo and run state, while
SHR does not expose pause/continue or remote song-location semantics.

Timing Clock runs whenever the feature is enabled and SHR is open; Start and
Stop still follow only tracker transport. This is the explicit clock-run state:
there is no second hidden switch. Direct hardware validation found that the
MiniLab must detect clock before it receives Start; sending Start before the
first pulse left its External-Sync arpeggiator waiting. Continuous stopped-state
clock is therefore the least surprising live-safe behavior. It lets the
controller know the tempo before Play, while `FC` still stops its arpeggiator.
When controller clock is enabled, SHR permits an otherwise empty Pattern to run
so a player can launch the live arpeggiator with ordinary tracker transport.
Clock stays at 24 PPQN from the current transport tempo (or configured default
tempo before the first run); cell timing, number of pages/destinations, and
swing/event placement do not create or move pulses. A live tempo change
preserves the remaining pulse phase, and a delayed worker skips missed
deadlines instead of producing a catch-up burst.

### Raspberry Pi setup, backup, verification, and rollback

Run all of these on the Raspberry Pi. Do not use MIDI Control Center or write
controller memory.

1. Stop SHR-DAW and any synth, then list ALSA subscriptions with `aconnect -l`.
   Run `shr clock ports` and identify the one `configure:` line paired with the
   MiniLab standard MIDI endpoint.
2. Back up the active `shsynth.conf` before editing it. For a normal install it
   is below `${XDG_STATE_HOME:-$HOME/.local/state}/shsynth/`; for a checkout,
   use the private state path selected by `scripts/local.sh`. Give the copy a
   date-stamped name and do not replace an older backup.
3. Set `controller_clock.output` to that complete `configure:` value, then set
   `controller_clock.enabled=true`. Leave MiniLab arpeggiator Sync at External.
4. Run `shr doctor`. Start SHR and confirm `aconnect -l` shows the
   `shs-controller-clock` source as non-exportable with no subscriptions; its
   events are directly addressed rather than represented by an ALSA
   subscription. Start tracker transport, enable the MiniLab arpeggiator, and
   play keys; no tracker page should target the controller.
5. To disable without losing the remembered endpoint, set
   `controller_clock.enabled=false` and restart SHR. Confirm the
   `shs-controller-clock` client is gone. To roll back completely, stop
   SHR and restore the dated backup of `shsynth.conf`.

The official [MiniLab 3 manual](https://downloads.arturia.net/products/minilab-3/manual/minilab-3_Manual_1_0_5_EN.pdf)
states that External Sync takes arpeggiator rate from host tempo and requires
clock plus host playback. The MIDI Association's
[MIDI 1.0 message summary](https://midi.org/summary-of-midi-1-0-messages)
defines Timing Clock as 24 per quarter note and distinguishes Start, Continue,
Stop, and Song Position Pointer. These are protocol facts; the channel-10 pad
mapping below is direct evidence from this Raspberry Pi/controller pair.
Arturia's current [MiniLab 3 support page](https://support.arturia.com/hc/en-us/articles/6189475866396-MiniLab-3-General-Questions)
documents the four separate ports, identifies `MiniLab 3 MIDI` as the standard
port, and lists DAW Shift as CC27. Its
[download page](https://www.arturia.com/support/downloads-manuals/product/minilab-3)
currently lists firmware 1.2.0. SHR does not infer the installed firmware or
claim that a controller-side setting persists: the no-clock behavior and
program/pad messages are observations of the connected unit in the stated test
state.

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

The opt-in SHR-owned JACK client sums exactly the managed software instrument,
the owned WAV loop, and one configured stereo capture pair. The instrument
retains its Project-persisted source insert rack and two aux buses; all three
sources then pass through the master rack, master level, linked sample-peak
limiter, final meter, final stereo recorder tap, and playback. It remains
disabled by default:

```text
audio.autoconnect=true
audio.output=system:playback_1
audio.output=system:playback_2
audio.graph.enabled=false
audio.graph.client=shr-graph
audio.graph.maximum_callback_frames=4096
audio.graph.input=External mix|system:capture_1|system:capture_2
audio.graph.input_direct_monitoring=false
audio.graph.confirm_doubled_monitoring=false
```

Exactly two `audio.output` entries are the preferred direct route and the
graph's main destinations. `audio.graph.input` is `LABEL|LEFT|RIGHT`; both JACK
capture names are resolved exactly. If it is absent, the first legacy
`capture.input` supplies a backward-compatible preference. Missing, ambiguous,
or identical ports keep the bus visibly unavailable—SHR-DAW never picks a
nearby name. The callback frame bound may be 1–4096 and must cover the active
JACK period; an unexpectedly larger callback faults final recording and writes
safe silence rather than overrunning fixed memory.

On managed-engine and loop load, SHR-DAW first establishes their direct
playback. The graph client stays muted while the exact synth, loop, live-input,
and playback links are connected. It then removes the four direct synth/loop
links transactionally before publishing at a callback boundary. Failure leaves
or restores those exact prior links. Shutdown restores only them; unrelated
JACK clients and connections are not changed.

Software monitoring means the configured capture pair passes through the final
bus and adds JACK-buffer plus 2.5 ms limiter lookahead latency. Interface direct
monitoring is outside SHR-DAW. If both are declared, activation is refused
unless `audio.graph.confirm_doubled_monitoring=true` deliberately acknowledges
the doubled/comb-filtered path. See [Final performance bus](FINAL_PERFORMANCE_BUS.md).

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

## Non-destructive runtime fallbacks

Saved configuration is preference, not a cache of what happens to be connected
today. At each safe engine or loop activation SHR-DAW compares visible JACK
ports without changing the configuration it will later save. It tries the
preferred `audio.output` pair, each configured `audio.internal_output` in
order, and `audio.headphone_output` last. The final entry is for the Pi analogue
jack or another lowest-quality emergency route; no port name is assumed. The
status names the fallback and missing preferred pair. If none is visible,
audio reports unavailable while retaining the preference for the next
activation.

Failure to open a controller or performance input leaves the TUI and tracker
computer keyboard active and reports each role independently. Other available
MIDI inputs still open. An exact Project MIDI target
falls back first to the configured external output, then to the already loaded
internal instrument. The target text in the Project never changes, and
transport resolves it again on the next play so reconnected hardware is used.

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

Add an optional 1-based MIDI channel between `pad` and the note number when a
keyboard and command pads share note numbers. The verified MiniLab factory
mapping is:

```text
pad.10.36=page-1
pad.10.37=page-2
pad.10.38=page-3
pad.10.39=page-4
pad.10.40=item-1
pad.10.41=item-2
pad.10.42=item-3
pad.10.43=item-4
```

Only channel-10 presses, releases, velocity-zero Note On releases, and
polyphonic pressure for those notes are consumed. Notes 36–43 on channel 1 or
any other channel remain musical input. CC buttons use the parallel
`button.cc.CHANNEL.NUMBER=ROLE` form. Old `pad.NUMBER` and
`button.cc.NUMBER` entries remain intentionally channel-agnostic.

Five-button layout:

```text
menu.layout=5
pad.36=page-cycle
pad.40=item-1
pad.41=item-2
pad.42=item-3
pad.43=item-4
```

The page-cycle action may instead be a held chord. This example holds CC27 and
uses CC93 on the same MIDI channel; the trigger may also be a normally mapped
control because its ordinary behavior remains active when the modifier is not
held:

```text
menu.layout=5
page_cycle.modifier=cc.1.27
page_cycle.trigger=cc.1.93
```

Use `note.CHANNEL.NUMBER` for a note-based modifier or trigger.

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
Command-note on/off and matching polyphonic pressure remain consumed; unmapped
musical notes pass through.
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

Open FT2 **NAV** → **PAGE**, then choose **MANAGE PAGES / TRACKS**. The
resulting **TRACKS** screen edits pages and columns. Use the main encoder to
select a page. **ADD** creates
another four-lane page in that Pattern. **TARGET** chooses `AUTO` (portable
machine default), an ALSA MIDI output that is currently visible, the active
SHR-DAW software instrument, or the configured output. `AUTO` displays an
`AUTO` channel and does not permit channel/bank/program editing because those
values would bind the Project to one machine. **CHANNEL**
chooses 1–16. Encoder press confirms a field. **DONE** keeps all page changes;
**SYS** → **EXIT** restores the Project from before TRACKS opened. On the
**COLUMN** and **BANK** pages, **COL−/COL+**, **PROG−/PROG+**, and the bank
controls edit the selected column. In a target/channel chooser, **CONFIRM**
keeps that field and **EXIT** cancels it.

FT2 **NAV** → **ROUTE** is the passive quick editor. Its 38×18 bordered overlay
shows the active page target and 16 per-column channel/bank/program rows inside
a 36×16 content area. Opening and browsing use cached discovery information;
they do not create a MIDI discovery client, send MIDI, synchronize routes, or
start an engine. A field changes only after click/Enter activates it. Back/Esc
cancels that field first. **APPLY ROUTING** validates and copies the detached
page draft through the same Project and route-synchronization owner used by
Tracks. Closing with the highlighted ROUTE launcher or Back cancels a dirty
draft and never saves silently.

The active-instrument choice always means the single software instrument that
SHR-DAW currently owns and monitors. It does not start another engine. It is
offline when no managed instrument is active.

An exact hardware port name is saved as a preferred route. If it is missing,
the page and persistent status show `FALLBACK` and name the missing target while
runtime may use the configured external hardware route. It never falls into
the Pattern's software synth. With no hardware route
at all it shows `OFFLINE`. SHR-DAW never rewrites the saved name. If multiple
ports have the same exact name, or a configured partial match selects more than
one port, the preferred target is ambiguous and is not guessed.

## Configured output

The `external_midi.*` settings provide the machine route resolved by portable
pages in new Projects and newly created FT2 Patterns.
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
`${XDG_DATA_HOME:-~/.local/share}/shsynth/songs/`. Current Project format 4
stores each FT2 Pattern as a self-contained unit with its own tempo,
meter, page targets, setup messages, four lanes per page, four column
channel/bank/program setups, every cell field, the source insert rack, aux
routing, and master rack. A format-4 `default` target plus four `default`
column markers is the canonical portable/unassigned state; it is not channel
zero, mute, or disabled. Versions 0 and 1 migrate with empty effects routing;
version 2 retains its source rack and gains empty aux/master routing; format 3
keeps every explicit target/channel unchanged. Version 0
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
loop.import_directory=~/.local/share/shsynth/loop-inbox
loop.output=system:playback_1
loop.output=system:playback_2
```

Exactly two `loop.output` destinations are required when loading a loop. At
application start its in-memory route follows the same resolved audio fallback
pair described above; its remembered configured pair is not rewritten. The
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

On first setup, the inbox is placed below the active XDG data root and seeded
without replacement from the four CC0 48 kHz WAVs in the installed
`loops/cleared-loops.txt` allowlist. Setup copies the selected JACK playback
pair to `loop.output`. Its optional MusicRadar download adds private 85, 110,
120, and 140 BPM drum loops to the same inbox after explicit confirmation; the
raw files are not part of the public package.

The callback also publishes a bounded stereo `LOOP OUT` snapshot after region
selection, interpolation, transport gating, and edge fades. This uses the same
client, `output_l`/`output_r` ports, and two configured destinations above; it
does not create a graph route or any additional JACK connection. Stop, unload,
load failure, oversize, and client loss clear availability and stale levels.

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

The selected FT2 page also owns live keyboard and ordinary musical MIDI
audition. A synth page uses its saved synthv1 preset name; each MIDI page uses
its own output/channel/program; a percussion page uses its drum mapping.
Switching any route field cancels the old route first. Internal channels and
programs are zero-based MIDI values, while every FT2 screen shows channels
1–16 and programs 1–128.

FT2 **REC** is deliberately hardware-only. It refuses a synthv1 page,
consumes musical controller notes before the loaded synth route, and
auditions them on the current page's configured/exact MIDI output and channel.
Recording loops only the selected pattern, writes only the visible page's four
lanes, and does not advance through or alter other order entries.

Pattern setup offers 4/4 row counts of 8, 16, 32, 64, and 128, or matching 3/4
counts of 6, 12, 24, 48, and 96. New patterns are distinct pattern records and
are appended to the Arrangement; clone duplicates the selected Pattern, while
repeat adds another order reference to the same pattern.

Fresh Patterns use the private routing template at
`${XDG_DATA_HOME:-~/.local/share}/shsynth/ft2-routing-defaults.shsong`. Without
one, the factory pages are Software Synth (first synthv1 preset), MIDI (channel
1/program 1), and Drums (channel 10). Saving a changed but note-empty Pattern
asks whether to replace this template; confirm changes it, cancel does not. Projects with
notes never update it implicitly. Legacy Projects without Pattern synth routing
receive safe in-memory defaults and are rewritten only by an explicit save.
