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
MIDI channels, programs, velocity defaults, mutes, percussion settings, lane
settings, and cell data. A new Project starts with one pattern containing
`MELODY` and `DRUMS`, and more pages can be added per pattern.

Each page keeps its own MIDI target, channel, bank, program, velocity, mute,
percussion settings, and lane settings. Pages play together, so one pattern can
control several hardware instruments and the active SHR-DAW software instrument.

Open **PAGES** to add or select a page and set its target and channel. **DONE**
keeps the changes. **CANCEL** restores the Project as it was before the page editor
opened. A disconnected saved target is marked `OFFLINE`; its route and notes
are not deleted.

## Step editing

Step entry accepts notes and chords from a MIDI controller. A chord fills up to
four lanes, keeps its velocities, and moves the cursor to the next row. A
computer keyboard can enter notes with `Z S X D C V G B H N J M`.

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
draft sound on that page's exact target and channel. Confirm keeps the program;
cancel restores the previous value and selection.

## Real-time recording

**REC** loops the selected pattern and records only the visible page. Played
notes are placed on its four lanes and quantized to pattern rows. During
recording, those notes do not also pass to the loaded software synth. They are
auditioned only through the page's hardware MIDI target and channel.

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
tracked repository. The Project stores only the imported filename, meter, source
BPM, 1/2x/1x/2x interpretation, non-destructive start/length in beats, and a
bar-based placement offset.

WAV has no dependable standard BPM metadata, so SHR-DAW does not invent it.
Import and **AUTO** estimate pulse spacing when the audio has useful
transients; otherwise they fall back to project tempo and duration. Correct
**BPM-**/**BPM+** when needed. **BPM x** cycles half, normal, and double
interpretations (120 gives 60, 120, and 240). **UNIT** changes whether CUT
controls move one beat or one measure.

The loop screen's **ALIGN** child has **AUTO**, **BAR-**, and **BAR+**. **AUTO**
re-runs the offline pulse/length estimate and resets placement to bar zero.
**BAR-** and **BAR+** move the whole WAV placement one Project bar left or right
without changing the cut region.

The loop follows FT2 play-here, play-from-start, stop, restart, order/pattern
transitions, tempo changes, and looping. This first implementation uses
real-time rate conversion, so tempo matching and sample-rate conversion are
safe but pitch changes with tempo; the limitation is explicit on screen. A
bounded 5 ms fade is applied at cut/loop edges. The 40×20 screen shows text for
filename, BPMs and ratio, region, state, elapsed/total time, rate, and channels.

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

## FT2 Arrangement

Open **TOOLS**, then **ARR** to edit the FT2 Arrangement separately from
pattern editing and Project files. The ARRANGE screen can select a step, append
or insert the current pattern, duplicate or remove a step, move a step earlier
or later, jump to the referenced pattern for editing, and play from the selected
step.

## Pattern and Project files

Patterns can use 8, 16, 32, 64, or 128 rows in 4/4. The matching 3/4 sizes are
6, 12, 24, 48, or 96 rows.

The Files screen saves, loads, previews, and deletes the whole Project. It also
keeps compact pattern operations for now: create, clone, copy, paste, resize,
or clear a pattern. New patterns are distinct records. Clone copies the selected
pattern. Arrangement repeat/duplicate adds another step that references the
same pattern.

Projects are readable `.shsong` text files stored below
`${XDG_DATA_HOME:-~/.local/share}/shsynth/songs/`. The current development
format stores each pattern's own tempo, meter, pages, lanes, setup messages,
and cells. Files with another version or unknown shape are not loaded or
overwritten.

## Detailed controls and routing

See the [Controller interface](CONTROLLER_INTERFACE.md) for the full FT2 menu
map. See [Configuration and routing](CONFIGURATION.md) for page routing, exact
targets, note ownership, and Project behavior.

FastTracker II was created by Fredrik “Mr.H” Huss and Magnus “Vogue” Högdahl of
the demo group Triton. Learn more at
[Demozoo](https://demozoo.org/productions/99958/).
