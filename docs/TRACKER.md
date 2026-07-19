# Tracker guide

The FT2 screen is a vertical MIDI pattern sequencer. Its quick, top-to-bottom
editing style is inspired by FastTracker II, but SHR-DAW is not an FT2 clone.
It does not use FT2 code or read XM files.

## Modes

The normal FT2 screen has a prominent **MODE** controller page. **PLAY** is
normal performance and playback, **REC** enters the existing hardware-only
real-time workflow, **EDIT** enters step editing, and **N00B** constrains live
FT2 MIDI input to a selected root and scale.

N00B supports every chromatic root plus major and natural minor, including
compact choices such as D# minor. Incoming notes map to the nearest scale tone;
equal-distance ties map downward, preserving octave position as closely as
possible. Each output is remembered by input channel/note, including repeated
notes, so note-off, velocity-zero note-on, mode changes, stop, panic, and exit
release the note actually played. Command pads remain consumed.

## Projects, patterns, and arrangement

An SHR-DAW Project contains FT2 Patterns and an FT2 Arrangement. An FT2 Pattern
is a self-contained tracker pattern. The FT2 Arrangement is the ordered chain
of Arrangement Steps; each step references a pattern ID. Repeating a step reuses
the same pattern until you explicitly clone or paste a new pattern.

Each FT2 Pattern owns its own rows, meter, master tempo, pages, page targets,
per-column MIDI channels/banks/programs, velocity defaults, mutes, percussion
settings, lane settings, and cell data. A new Project starts with one pattern containing
`MELODY` and `DRUMS`, and more pages can be added per pattern.

Each page keeps one MIDI target plus four independent column channel, bank, and
master-program setups. It also keeps velocity, mute, percussion, and lane
settings. Columns may share a destination/channel only when their master bank
and program match, because MIDI program selection is channel-wide. Pages play
together, so one pattern can control several hardware instruments and the
active SHR-DAW software instrument.

Open **TOOLS** → **PAGES** to reach the **TRACKS** screen. There you can add or
select a page, choose a column, and set its target, channel, bank, and program.
**DONE** validates shared-channel compatibility and keeps the changes. **SYS**
→ **EXIT** restores the Project as it was before TRACKS opened. A disconnected
saved target is marked `OFFLINE`; its route and notes are not deleted.

## Step editing

Step entry accepts notes and chords from a MIDI controller. A chord fills up to
four lanes and keeps its velocities. The **ADD** controller page chooses a
persistent advance of 1, 2, 4, or 8 rows for note/chord entry, blank, erase,
and note-off. This makes evenly spaced bass notes and drum hits quick to enter;
the FT2 title shows `EDIT +n`. A computer keyboard can enter notes with
`Z S X D C V G B H N J M` and choose advance with `1`, `2`, `4`, or `8`.

The editor can add a note, note-off, or blank step. It can also change the page
program and pattern master tempo, mute a lane, and move through rows, lanes,
pages, and arrangement steps.

Tempo commands inside cells still work inside the current pattern. When
playback enters the next arrangement step, tempo starts again from that
referenced pattern's master tempo. The arrangement boundary itself does not
send note-off for active lanes; a lane is released by its own gate/cut/note-off,
by a later note in the same lane, or by stop/panic/mute cleanup.

## Cell editing

**CELL EDIT** changes one cell as a draft. **CONFIRM** saves the draft. **EXIT**
or cancel restores the original cell.

A cell contains:

- a blank, MIDI note 0–127, or note-off;
- an inherited gate or a gate from 1–100% of one row;
- inherited velocity or MIDI velocity 0–127;
- inherited program or a MIDI program override from 0–127;
- one optional command: cut or delay tick 0–15, retrigger count 1–8, or tempo
  20–300 BPM.

The grid shows `C` for cut, `D` for delay, `R` for retrigger, and `T` for tempo.
One cell cannot contain more than one command. Velocity, program, gate, and
retrigger need a note-on in a newly confirmed edit. Invalid combinations stay
in the draft and show an error.

Choosing **PROGRAM** opens a full-height sound browser. A matching MIDI device
profile adds the instrument's slot labels and sound names. Without a profile,
all MIDI program numbers 0–127 remain available. Controller notes audition the
draft sound on that page's exact target and selected-column channel. Confirm
keeps the cell override without changing the column master; cancel restores
the previous value and selection.

## Real-time recording

**REC** loops the selected pattern and records only the visible page. Played
notes are placed on its four lanes and quantized to pattern rows. Each assigned
lane auditions through that column's channel/instrument setup. During
recording, those notes do not also pass to the loaded software synth. They are
auditioned only through the page's hardware MIDI target and column channels.

Real-time recording is hardware-page-only. A page targeting the active SHR-DAW
instrument cannot enter **REC**. Choose a configured or exact hardware MIDI
output first. **REC END**, **STOP**, **EXIT**, and **PANIC** release auditioned
notes.

## WAV loops

Open **TOOLS**, then **LOOP** to import a mono or stereo WAV from the configured
inbox. Import validates it, estimates the loop length from transient pulses
when possible, snaps the length to whole Project bars, and copies it into private
storage below
`${XDG_DATA_HOME:-~/.local/share}/shsynth/loops/`; user audio never enters the
tracked repository. The Project stores only the imported filename, source BPM,
1/2x/1x/2x interpretation, non-destructive start/length in beats, and a
bar-based placement offset. Meter comes from the Pattern.

WAV has no dependable standard BPM metadata, so SHR-DAW does not invent it.
Import and **AUTO** estimate pulse spacing when the audio has useful
transients; otherwise they use duration and the current tempo to choose a whole
bar length. The current Pattern tempo is then set from the interpreted WAV BPM.
Correct **BPM-**/**BPM+** when needed; these controls also update the Pattern
tempo. **BPM x** cycles half, normal, and double interpretations (120 gives
60, 120, and 240). **UNIT** changes whether CUT controls move one beat or one
measure.

The loop screen's **ALIGN** child has **AUTO**, **BAR-**, and **BAR+**. **AUTO**
re-runs the offline pulse/length estimate and resets placement to bar zero.
**BAR-** and **BAR+** move the whole WAV placement one Project bar left or right
without changing the cut region.

The loop follows FT2 play-here, play-from-start, stop, restart, order/pattern
transitions, and looping. It plays at native speed and pitch; beat detection
adjusts the Pattern tempo to the WAV, not the WAV to the previous Pattern
tempo. The loop player requires the JACK server sample rate to match the WAV
sample rate. For a 44.1 kHz loop, configure/restart JACK at 44100 Hz before
loading it. A bounded 5 ms fade is applied at cut/loop edges. The 40×20 screen
shows text for filename, BPMs, region, state, elapsed/total time, rate, and
channels. A decoded loop is limited to 6,000,000 frames (about 125 seconds at
48 kHz) so one imported file cannot exhaust Raspberry Pi memory.

From **TOOLS** → **LOOP**, press **REMOVE** twice to detach the loop from the
Project and unload its JACK client. The imported private WAV is kept on disk so
another Project can still use it.

On FT2 Tools, open the **LOOP** menu page and choose **LIBRARY**. It is separate
from Remove: it pages through imported private WAVs and marks the current loop,
saved-Project references, and free files. Physical deletion requires
confirmation and is refused for referenced, symlinked, or unsafe paths.

## Copy and Paste

Pattern copy stores the complete current FT2 Pattern, including rows, pages,
routes, channels, programs, mutes, meter, and tempo. Paste can create a new
pattern or paste over the current pattern after confirmation. Clone remains the
fast one-step way to copy the selected pattern into a new arrangement step.

The FT2 tools clipboard can copy and paste one lane/column or one full page
block. Lane and page paste keep note, velocity, program, gate, and command
cells. When source and destination row counts differ, only overlapping rows are
pasted and the status line reports truncation. Page paste targets the selected
destination page; missing destinations are not created implicitly.

## Drum pattern library and transpose

Open **FILES** → **PATTERN** → **DRUMS** for reusable rhythms stored separately
from Projects. The bundled library has 72 authored grooves across
Rock, Pop, House, Techno, Hip-Hop, Funk, Reggae, Breaks, Latin, and Jazz. The
**FILTER** page selects genre, 3/4 or 4/4 meter, and phrase length. 4/4 offers
32/64/128 rows (2/4/8 bars at the default four steps per beat); 3/4 offers the
matching 24/48/96 rows. Longer choices add alternating-bar changes and
genre-aware phrase-end fills rather than merely duplicating a filename. Genre
names are compact creative labels for editable starting points, not claims of
an authoritative historical transcription.

**LOAD** replaces only the current Pattern's first percussion page. Its
destination, channels, bank/program setup, lane state, tempo, and arrangement
remain unchanged. An empty melodic Pattern is resized to the selected meter and
length for the quick load-drums-then-enter-bass workflow. If melodic cells
already contain data, any load that would resize or change meter is refused.

**SAVE** writes the current percussion page as a non-overwriting `.shdrum` file
below `${XDG_DATA_HOME:-~/.local/share}/shsynth/drum-patterns/`. **DELETE**
requires confirmation and applies only to user-saved files; bundled grooves
are read-only.

The Pattern **TRANS** page moves all note-ons on non-percussion pages by a
semitone or octave up/down. Percussion pages and note-offs are never changed.
If any melodic note would leave MIDI range 0–127, the whole transpose is
refused without changing the Pattern.

## FT2 Arrangement

Open **TOOLS**, then **ARR** to edit the FT2 Arrangement separately from
pattern editing and Project files. The ARRANGE screen can select a step, append
or insert the current pattern, duplicate or remove a step, move a step earlier
or later, jump to the referenced pattern for editing, and play from the selected
step.

## Pattern and Project files

Patterns can use 8, 16, 32, 64, or 128 rows in 4/4. The matching 3/4 sizes are
6, 12, 24, 48, or 96 rows.

The Files screen saves, loads, previews, and deletes the whole Project. Its
**PATTERN** child keeps create, clone, copy, paste, resize, clear, transpose,
and drum-library operations together. New patterns are distinct records. Clone copies the selected
pattern. Arrangement repeat/duplicate adds another step that references the
same pattern. **CLEAN** offers only Pattern records with zero Arrangement
references, confirms deletion, preserves at least one Pattern, and never
rewrites an Arrangement step.

**NEW PRJ** requires a second press before replacing the in-memory Project and
chooses the next free `project-001` style name. **SAVE AS** immediately writes
the next free `<current-name>-copy-001` style copy and switches to it. These
automatic names keep both actions usable from a four-button controller.
**NAME** accepts a useful display name and derives a safe filename; collisions
are refused and a saved rename keeps the loaded Project state.

Projects are readable `.shsong` text files stored below
`${XDG_DATA_HOME:-~/.local/share}/shsynth/songs/`. Current Project format 3
stores each Pattern's tempo, meter, pages, four column setups, lanes, setup
messages, cells, source insert rack, two aux routes, and master rack. Versions
0 and 1 gain empty effects routing; version 2 retains its source rack and gains
empty aux/master routing. Version 0 page-wide setups copy the old
channel/bank/program into all four columns. Unknown newer versions, fields, or
invalid effect shapes are not loaded or overwritten.

## Effects saved with the Project

The Project also owns the managed instrument's ordered source insert rack, two
aux send/rack/return routes, and master rack. Those settings are independent of
the Pattern/Arrangement structure: repeating a Pattern does not duplicate an
effect, and changing Arrangement steps does not change rack order. The two aux
sends take their pre/post source-insert taps from the one managed software
instrument, not from individual MIDI lanes.

With the opt-in graph active, the source and wet returns meet once before the
master rack and final meter. The private WAV loop, external-instrument audio,
and recorder capture are separate audio paths, so their MIDI pages or loop
references do not acquire source inserts or aux sends. See
[How SHR-DAW works](HOW_IT_WORKS.md#the-managed-audio-graph) for the musical
workflow and [Audio graph and DSP contract](AUDIO_GRAPH.md) for exact effect
schemas and limits.

## Detailed controls and routing

See the [Controller interface](CONTROLLER_INTERFACE.md) for the full FT2 menu
map. See [Configuration and routing](CONFIGURATION.md) for page routing, exact
targets, note ownership, and Project behavior.

FastTracker II was created by Fredrik “Mr.H” Huss and Magnus “Vogue” Högdahl of
the demo group Triton. Learn more at
[Demozoo](https://demozoo.org/productions/99958/).
