# How SHR-DAW works

SHR-DAW is a small music workstation built from several deliberately separate
parts: one controller input, one managed software instrument, an FT2-style MIDI
sequencer, a private WAV loop player, a stereo recorder, and an optional owned
effects graph. This guide connects those parts and explains what the musician
can do with them. For exact configuration keys use
[Configuration and routing](CONFIGURATION.md); for the DSP and real-time
contract use [Audio graph and DSP contract](AUDIO_GRAPH.md).

## The whole signal model

The shortest useful picture is:

```text
controller / MIDI keyboard
        |
        v
SHR input router
  |                    |
  | commands/controls  +---- musical notes ----------------------+
  v                                                           |  |
screen, menus, pickup                           managed synth <-+  |
                                                or FT2 page -------+

FT2 scheduler -> each page's MIDI destination -> software or hardware instrument

managed synth audio -> direct JACK playback
                    or SOURCE -> AUX 1/2 -> MASTER -> FINAL OUT -> playback

private WAV loop -----------------------------------------------> playback
configured stereo capture --------------------------------------> WAV recorder
```

The last two audio paths are separate clients. The current master rack and
`FINAL OUT` meter process only the one managed software instrument and its two
aux returns. They do not secretly include the WAV loop, recorder input,
external-instrument audio, hardware returns, or unrelated JACK clients.

## One input, controls, and musical notes

SHR-DAW opens one configured ALSA MIDI input. Messages are classified before
they reach an instrument:

- menu buttons, the main encoder, encoder press, and the 12 mapped synthv1
  controls stay inside SHR-DAW;
- command-pad note-on and note-off are both consumed, so releasing a menu pad
  cannot leak a musical note;
- ordinary musical notes, velocity, and performance messages go to the active
  live or tracker destination; and
- pad lock can temporarily treat command pads as notes when that is wanted.

A controller profile describes what a physical device sends. The setup wizard
can apply a reviewed profile or learn absolute controls, either relative
encoder direction, CC/note buttons, and an encoder press without forwarding
the learning messages to a synth. Learned mappings remain private; reviewed
catalog updates are validated and published atomically. See
[Controller profiles](CONTROLLER_PROFILES.md).

The 12 synthv1 controls use pickup. After a preset or Idea loads, or after
`RESET`, mapped CC messages are blocked until the physical control reaches or
crosses the stored value. This prevents a knob position from making the sound
jump. Playback indicators compare each value with the original preset: green
is more than 0.03 below it, bright yellow is within 0.03, and red is more than
0.03 above it. Reset changes only those mapped parameters, re-arms pickup, and
does not restart the engine.

Held notes drive the Playback note/chord display and its continuous keyboard
strip. German B/H spelling is the default; `display.note_names=english` uses
A#/B spelling. Naming changes only the display, never the MIDI notes.

## Software instruments and ownership

SHR-DAW browses three separately installed instrument hosts:

- [synthv1](https://synthv1.sourceforge.io/) for subtractive synth presets;
- [Yoshimi](https://yoshimi.github.io/) for `.xiz` instruments and banks; and
- [FluidSynth](https://www.fluidsynth.org/) for `.sf2` and `.sf3` SoundFonts.

Only one SHR-managed software instrument runs at a time. Loading another sound
may reuse the current owned process or replace it; replacement sends All Notes
Off, performs a clean shutdown, and starts the next configured host. SHR-DAW
records enough process identity to stop only the engine it started. It neither
layers managed engines nor kills an unrelated synthv1, Yoshimi, or FluidSynth
process opened by the user.

Commands, client names, preset roots, SoundFonts, MIDI ports, and JACK ports
are configuration. The engine code does not assume the development hardware.
The three catalogs also remain separate: a synthv1 XML preset is not treated as
a Yoshimi instrument or a SoundFont program.

## Three different kinds of recording

SHR-DAW uses “record” for three intentionally different jobs:

1. An **Idea** captures free-time MIDI while playing a managed sound. It keeps
   event timing and instrument identity; synthv1 Ideas also keep a private
   preset snapshot and the mapped control values. `TAKE` plays that MIDI back
   through the restored instrument. An Idea is not audio.
2. FT2 **REC** captures notes into the current Pattern grid. It quantizes them
   to rows, writes only the visible page's four lanes, loops that Pattern, and
   auditions through the page's external MIDI target. It refuses an Active
   Instrument page so notes are not doubled through the managed synth.
3. **Audio recording** captures the first configured JACK stereo input pair as
   a 24-bit WAV. It records the sound arriving at those ports, not the MIDI
   events that produced it.

Idea take playback runs independently of screen redraw. Stop, route changes,
replacement, panic, and exit release the exact notes still owned by that take.
Ideas publish into new private directories without replacing a same-named
Idea.

The audio callback writes capture samples into a fixed stereo ring; an ordinary
worker thread performs disk I/O. A unique filename is selected without
replacement. Until finalization the file ends in `.wav.part`; a later recording
start recovers complete frames from a recognized interrupted file and never
follows a `.part` symlink. The recorder stops cleanly at the RIFF 4 GiB limit
and reports dropped frames or disk errors. It does not software-monitor its
input back to playback, so an external instrument or microphone should use
safe hardware direct monitoring when available.

## Projects, Patterns, pages, and columns

A **Project** is the complete tracker work saved as one `.shsong` file. It
contains:

- distinct **Patterns**;
- an **Arrangement** whose ordered steps reference Pattern IDs;
- each Pattern's tempo, meter, rows, pages, lanes, and cells;
- page/column MIDI routing and setup data;
- the optional private WAV-loop reference and placement; and
- the source, aux, and master effects state.

A Pattern is reusable musical data, while an Arrangement step is a place that
plays a Pattern. `REPEAT` adds another reference to the same Pattern, so later
edits affect every repeated use. `CLONE` or paste-new creates a separate
Pattern when the copies need to diverge. Cleanup deletes only Patterns with no
Arrangement reference and never silently rewrites the Arrangement.

Each Pattern owns one or more **pages**, and every page has four note
**columns**. All enabled pages play together. A page chooses one MIDI
destination: the active managed instrument, the configured external output, or
an exact visible ALSA MIDI port. Each column then stores its own channel 1–16,
bank MSB/LSB, master program, lane name, and mute state. Two columns may share
the same destination/channel only when their master bank/program selections
match, because MIDI program changes affect the whole channel.

This separation makes several useful routes possible in one Pattern: one page
can play the managed software instrument, another can address a drum machine,
and another can play a hardware synth on a different port. A disconnected
exact target is displayed as `OFFLINE`; its name and notes stay in the Project,
and other available pages continue. Ambiguous exact or partial port matches
are refused rather than guessed.

External MIDI device profiles add trustworthy bank labels and program names to
the column and cell program browsers. They remain JSON data, can be privately
overridden for writable user memories, and never remove the numeric 0–127
fallback. See [MIDI device profiles](MIDI_DEVICE_PROFILES.md).

## FT2 Play, Record, Edit, and N00B modes

The FT2 screen has four explicit modes:

- **Play** navigates rows, pages, lanes, and Arrangement steps and starts
  transport from the cursor or Project beginning.
- **Record** performs the hardware-only quantized capture described above.
- **Edit** writes notes or chords from MIDI/computer-keyboard input. Blank,
  erase, and note-off are explicit operations, and the persistent 1/2/4/8-row
  advance determines the next cursor position.
- **N00B** maps live input to the nearest note in a chosen major or natural
  minor scale. Equal-distance choices go downward, and each source note retains
  ownership of the mapped note used for its eventual note-off.

Cell Edit is transactional: changes are made to a draft, `CONFIRM` publishes
the whole cell, and `EXIT` restores the original. A cell can hold a note or
note-off, inherited/explicit velocity and gate, a per-note program override,
and one command: cut, delay, retrigger, tempo, or none. Program audition uses
the selected page destination and column channel without inserting a note or
duplicating generic live thru.

Pattern Setup offers musically convenient fixed shapes: 4/4 Patterns of
8/16/32/64/128 rows and corresponding 3/4 Patterns of 6/12/24/48/96 rows.
`CONFIRM` applies a new/destructive shape; `KEEP` clears while retaining the
current shape. The underlying scheduler can represent 1–256 rows, but arbitrary
interactive resizing and groove timing remain planned work rather than a
current menu promise.

The reusable drum library contains 72 authored four-lane starting points in ten
creative genre groups. Filters choose 3/4 or 4/4 and a 2/4/8-bar phrase. Loading
changes the first percussion page's cells without replacing its MIDI target,
channels, bank, program, tempo, or Arrangement. User saves are separate
`.shdrum` files; bundled patterns are read-only. Melody-only transpose leaves
percussion pages and note-offs unchanged and refuses the whole edit if a note
would leave MIDI range.

## The managed audio graph

Without the owned graph, the managed instrument connects directly to the two
configured playback ports. With `audio.graph.enabled=true`, the same source is
moved transactionally into this stereo route:

```text
managed instrument -> SOURCE insert rack ---------------------------> dry sum
       |                    |                                            ^
       |                    +-> POST send -> wet AUX 1/2 -> return ------+
       +----------------------> PRE send  -> wet AUX 1/2 -> return ------+

dry sum -> MASTER rack -> FINAL OUT meter -> configured playback L/R
```

There are four useful placement ideas:

- A **source insert** processes the instrument in series. It is the normal
  place for tone shaping, dynamics, distortion, or an effect that belongs to
  this sound.
- An **aux send** makes a parallel copy. `PRE` takes it before source inserts;
  `POST` takes it after them. Each of AUX 1 and AUX 2 has its own send, rack,
  return gain, and meter.
- An **aux return** brings only the effected copy back into the sum. The normal
  aux editor offers Delay, Reverb, Chorus, Flanger, and Phaser and forces them
  to 100% effect/0% dry so the original instrument is not accidentally doubled.
- The **master rack** processes the complete source-plus-returns sum. It is the
  place for final corrective EQ, bus compression, overall utility changes, or
  deliberate whole-mix coloration.

Send and return levels run from -60 to +12 dB. A new aux starts with a
conservative -18 dB post-insert send. The compact controls use 3 dB steps;
sends below -60 dB show `OFF`. Each serial rack holds at most eight processors,
the complete graph at most 16, and no more than two reverbs. These limits are
rejections, not silent truncation.

### Effect possibilities

Source and master racks can use all 13 effect types:

- **Utility** trims level, pans, changes stereo width, inverts either channel,
  or mutes. It is useful for gain staging and stereo correction rather than a
  flashy sound.
- **EQ** provides a low cut, low/high shelves, two broad mid bands, and output
  trim. Use it to remove rumble, reduce boxiness or harshness, or emphasize the
  part of a sound that should speak.
- **Compressor** controls peaks and movement with threshold, ratio, knee,
  attack, release, makeup, parallel mix, and sidechain high-pass. Fast attack
  restrains transients; slower attack lets the front of a note through.
- **Distortion** offers soft cubic, hard clip, and asymmetric modes plus drive,
  bias, tone, output, and mix. They range from rounded saturation to an
  intentionally sharp edge; output trim is important for fair comparison.
- **Gate** reduces sound below a threshold with hysteresis, depth, attack,
  hold, and release. It can clean gaps or deliberately shorten a noisy/long
  texture, but aggressive settings can cut note tails.
- **Filter** is a resonant low-pass, band-pass, or high-pass with drive and
  mix. It can darken, thin, isolate a moving band, or add a resonant sweep.
- **Crusher** reduces bit depth and sample-hold rate, with optional dither and
  parallel mix, for stepped digital texture.
- **Delay** supports stereo, ping-pong, and mono-to-stereo echoes, free time or
  tempo divisions, feedback, stereo ratio, tone, wet/dry mix, and optional
  tail-on-bypass.
- **Reverb** offers room, plate, and hall voicings with predelay, decay, size,
  damping, input low cut, width, and wet/dry balance.
- **Chorus** uses a short modulated delay to add width and gentle pitch motion;
  rate, depth, stereo phase, feedback, mix, and dry level shape the result.
- **Flanger** uses a much shorter modulated delay and signed feedback for
  moving comb-filter sweeps, from subtle motion to metallic resonance.
- **Phaser** uses four or six stable all-pass stages with rate, center, range,
  feedback, stereo phase, and mix for a smoother notched sweep.
- **Tremolo/Pan** changes level or stereo position with sine, triangle, or
  smoothed-square motion, plus rate, depth, stereo phase, and output trim.

Exact names, defaults, physical ranges, and delay divisions are centralized in
[the effect schema table](AUDIO_GRAPH.md#effect-parameter-schemas). The rack UI
uses those schemas rather than a different hidden set of values.

### Bypass, tails, meters, and publication

Source/master bypass fades toward the dry signal rather than switching on one
sample. An aux cannot use that same fallback because raw send audio on a return
would double the source. If every wet generator on an aux is bypassed, its
return fades to silence. A delay with tail-on-bypass may stop accepting new
input while its already-created wet echoes drain; serial conditioning can
continue to pass an already-wet signal or feed another active wet generator.

Every processor publishes bounded input/output peak and RMS plus clip and
non-finite state; compressor editing also exposes gain reduction. Each aux
meters after its return gain. `FINAL OUT` is a dedicated post-master meter
immediately before the graph playback ports, so master effects are included.

The FX rack and parameter editor remain available while the graph is disabled,
so a Project can be designed silently without an audio callback to rebuild.
When the graph is enabled, every FX change that would publish a replacement
runtime plan requires stopped transport and no active recording. The complete
plan, coefficients, buffers, ports, and memory are prepared and validated away
from the real-time callback. Stable instance IDs let compatible effects retain
DSP state when moved. The callback uses fixed memory and atomics: no file
access, subprocess, logging, allocation, or locks.

The graph remains opt-in and disabled by default. A managed engine is connected
directly first; the graph is activated muted, its exact boundary is connected,
the two direct links are removed as one rollback-capable transaction, and only
then is graph output published at a block boundary. Validation, activation, or
connection failure leaves or restores direct playback. Shutdown deactivates
the callback before restoring direct links, avoiding a doubled final block.

FX state is saved in the Project while the graph is disabled, but direct
playback cannot process or meter it. The graph owns exactly one current stereo
source: the managed instrument. The typed model reserves future source/sink
kinds, but there is currently no loop strip, live-input strip, external
hardware return, hardware insert, multi-source master, or graph recording tap.

## WAV loops are a separate audio route

A Project may attach one privately imported mono or stereo WAV. The import
inbox is only a browser source; the selected file is validated and copied
without replacement below the private SHR data directory. The Project saves
the private filename, interpreted source BPM, half/normal/double mode,
non-destructive cut region in beats, and whole-bar placement offset.

Loop analysis is offline, outside the JACK callback. `AUTO` uses bounded pulse
and duration analysis when useful and proposes a whole-bar interpretation.
Tempo matching changes the current Pattern tempo to follow the WAV—it does not
stretch or pitch-shift the WAV to the old tempo. Playback remains native-speed
and requires the WAV sample rate to match JACK. Decoded audio is bounded to
6,000,000 frames, about 125 seconds at 48 kHz.

The loop player is a separate owned JACK client connected to its configured
playback pair. It follows FT2 start, play-here, Pattern/Arrangement transitions,
looping, and stop, but it does not pass through source inserts, aux sends,
master effects, or `FINAL OUT`. Project `REMOVE` only detaches the loop. The
Library performs separate confirmed physical deletion and refuses current,
saved-Project, symlinked, unsafe, or otherwise referenced files.

## Note ownership and failure behavior

MIDI notes are owned by their route, page, column/lane, and playback source.
Two lanes may hold the same note on the same destination/channel; SHR-DAW sends
note-off only after the last owner releases it. Stop, page/lane mute, route
change, Project replacement, Idea/take stop, recorder stop, panic, and exit
clean up only the activity each action owns. This prevents one lane or screen
from cutting off another shared note.

Missing JACK leaves browsing and external-MIDI sequencing usable. A missing
external MIDI target stays `OFFLINE` without rewriting Project data. Ambiguous
ports are refused. Missing optional engines or sound banks remain visible with
an explanation. A failed graph returns to direct playback. None of those
failures authorizes SHR-DAW to rewire unrelated clients or terminate processes
it does not own.

## Project and private-data safety

Project format 3 persists the complete tracker state and effects routing.
Formats 0 and 1 migrate with empty effects; format 2 retains its source rack and
gains empty aux/master routing. Unknown newer formats, fields, malformed rack
data, unsafe paths, and over-limit structures are refused rather than partly
loaded and then written back.

Normal Project save asks again before replacing an existing file. `SAVE AS`
chooses a numbered non-overwriting copy. Rename publishes the complete new
Project before removing the old filename and refuses collisions. New Ideas,
audio recordings, imported loops, and user drum patterns likewise choose or
require unused destinations. Destructive deletion is explicit and scoped:
Pattern cleanup checks zero Arrangement references, Project loop removal keeps
the WAV, and loop-library deletion rescans saved Projects at commit time.

Configuration lives below
`${XDG_STATE_HOME:-~/.local/state}/shsynth/`; private user data normally lives
below `${XDG_DATA_HOME:-~/.local/share}/shsynth/`. A repository-local launch
redirects both into ignored `user/`. Important private data includes Ideas,
Projects, recordings, imported loops, user drum patterns, learned controller
configuration, profile overrides, and uncleared presets. Public packaging uses
only the 21-presets allowlist and the authored drum data. See
[Licensing and redistribution](../THIRD_PARTY.md).

## Performance information and honest limits

MTR CPU rows show whole Linux CPU-core activity from `/proc/stat`, not the CPU
used by one synth/effect, JACK callback duration, scheduling jitter, or xruns.
The stereo meter is available only from the active owned graph and covers only
its post-master managed-instrument output. Direct mode reports audio metering
unavailable instead of creating a hidden tap or displaying unrelated audio.

Maintainer checkpoints separately collect callback count, mean, p95, p99,
maximum, deadline misses, oversized blocks, xruns, process/core CPU, memory,
and shutdown behavior. The implemented one-source graph has passed its recorded
Raspberry Pi engineering checkpoints. Those measurements establish bounded
technical behavior for the tested routes; unfinished listening/curation and
future multi-source routing remain documented as such rather than being
silently presented as current features.
