# Tracker guide

The FT2 screen is a vertical MIDI pattern sequencer. Its quick, top-to-bottom
editing style is inspired by FastTracker II, but SHR-DAW is not an FT2 clone.
It does not use FT2 code or read XM files.

## Modes

The normal FT2 screen has **PLAY**, hardware-only **REC**, and detailed
**EDIT** modes. **N00B is a separate on/off filter** that can remain enabled in
all three. It keeps the selected melodic page as the instrument and filters
input through a chosen chromatic root plus major or natural-minor scale. An
in-scale key keeps its original pitch; an out-of-scale key is consumed and
stays silent. N00B never quantizes a rejected key to a different note.
Each entry to the main FT2 screen opens controller-menu page 1, **PLAY**, where
the **PLAY** and **RECORD** buttons are immediately available.

In Play, N00B changes only what is heard. In REC and EDIT, allowed notes can be
written normally while rejected notes remain silent and unwritten. Turning the
filter on or off never changes Play/REC/EDIT. N00B is refused on a percussion
page; moving onto Drums turns only the filter off and preserves the current
mode.

The N00B button is present in Play, REC, and EDIT. Each press toggles the
Player-selected scale directly without opening another screen or changing
existing cells. Command pads and their releases remain consumed.

On the main tracker grid, the physical main rotary selects the previous or next
column while Play or REC transport is active, continuing through page
boundaries from Software Synth to MIDI, Drums, and later pages. The selected
column has a subtle dark full-column shade; the yellow cell cursor and
row/warning emphasis remain stronger. While transport is paused the rotary
moves rows, as it does in EDIT, and keyboard arrows retain row navigation in
every mode. Active-transport rotary selection does not move the row, playhead,
Arrangement Step, or transport.

## Projects, patterns, and arrangement

An SHR-DAW Project contains FT2 Patterns and an FT2 Arrangement. An FT2 Pattern
is a self-contained tracker pattern. The FT2 Arrangement is the ordered chain
of Arrangement Steps; each step references a pattern ID. Repeating a step reuses
the same pattern until you explicitly clone or paste a new pattern.

Each FT2 Pattern owns its own rows, meter, master tempo, pages, page targets,
per-column MIDI channels/banks/programs, velocity defaults, mutes, percussion
settings, lane settings, and cell data. A new Project starts with one pattern
whose FT2 workspace exposes four musician-facing pages:

1. `Software Synth`, a four-track page using the first available synthv1 preset;
2. `MIDI`, a four-track page using the configured external output, MIDI channel
   1, and program 1;
3. `Drums`, a four-track page using the configured external output, MIDI
   channel 10, program 1, and the existing percussion-note mapping;
4. `Loop Player`, the Project-wide WAV source in an explicit `NOT READY` state.

The Loop Player is a page in the musician-facing FT2 workflow, not four empty
MIDI lanes. **SELECT** → **PAGE** opens it directly, so a new
Project does not require adding or naming a page before importing a WAV.
The blank Pattern, unloaded loop state, loop inbox, and startup MIDI-output
snapshot are initialized when SHR-DAW starts. Entering a genuinely new, empty,
unsaved FT2 Project loads its page 1 software instrument immediately. If Player
already owns a loaded instrument, page 1 adopts that exact instrument and the
same managed engine session becomes FT2-owned without a restart. Otherwise
page 1 loads the first available synthv1 preset.

The Loop Player's white position bar uses a green playhead to show the
approximate position within the selected WAV region while the shared FT2
transport plays or records. The bar remains visible at the top of the loop
page whenever a valid WAV region is selected; an output fault leaves it at the
start and reports the fault explicitly instead of using colour as the only
state cue.

Channels and programs are zero-based in MIDI bytes and in the in-memory model.
Every musician-facing screen shows channels 1–16 and programs 1–128.

Each page keeps one MIDI target plus four independent column channel, bank, and
master-program setups. It also keeps velocity, mute, percussion, optional
device-profile metadata, and lane settings. A software target stores its engine
and that engine's stable instrument identity in the Pattern. When page 1 is
part of a genuinely new, empty, unsaved default Project, entering FT2 may
replace its factory route with the currently loaded Player engine/instrument.
A loaded/saved Project or an unsaved Project with any explicit change is never
retargeted, even when its Pattern has no notes.
Columns may share a destination/channel only when their master bank
and program match, because MIDI program selection is channel-wide. Pages play
together, so one pattern can control several hardware instruments and its
Pattern-owned SHR-DAW software instrument. Because SHR owns only one synth host
at a time, playback refuses an Arrangement that would require two different
software routes instead of sending both through the wrong engine or sound.

Computer-keyboard notes and ordinary incoming musical MIDI audition the
selected page's target, channel, program, and drum mapping throughout the FT2
workspace. Main-rotary column navigation preserves already sounding notes on
their original routes while later notes start from the newly selected column.
Explicit page/track route, preset, channel, program, or destination changes
still end notes on the old route. The FX rack/editor is an FT2 child: live input
and the owned synth stay active, and Back returns to its FT2 caller. Leaving
top-level FT2 for an unrelated workspace ends notes and unloads its owned synth.

`AUTO · machine default` is a real portable target. Its saved channel, bank,
program, and setup fields are blank; at playback the machine's configured
melody/percussion channels and available default destination are used. `AUTO`
does not mean channel 1, channel zero, muted, or disabled. Choose an explicit
target only when a song intentionally belongs to particular hardware.

Use FT2 **SELECT** → **PAGE** to browse every page/column without leaving the
Pattern. Its final row opens the full **TRACKS** screen. There you can add or
select a page, choose a column, and set its target, channel, bank, and program.
**DONE** validates shared-channel compatibility and keeps the changes. Internal
routes use `TARGET → ENGINE → INSTR`; external routes use
`TARGET → MIDI OUT → CH → INSTR/PROG`. **SYS**
→ **EXIT** restores the Project as it was before TRACKS opened. A disconnected
saved target is marked `OFFLINE` (or `AMBIG` for duplicate stable identities);
its exact route, notes, raw channels 1–16, and programs 0–127 are not changed.

For a quick routing change, **SELECT** → **ROUTE** opens a centered overlay over
FT2. It shows target type, software engine/instrument or MIDI output, optional
device profile, plus all four columns' channel, bank, program/instrument name,
and interface availability. Turn and click/Enter to activate a
field; Back/Esc cancels that field first. Only **APPLY ROUTING** changes the
Project. The same highlighted ROUTE item or Back closes and cancels every
unconfirmed change. At 40×20 the bordered outer window is 38×18 at `(1,1)` and
the usable inner area is 36×16 at `(2,2)`.

## Step editing

Step entry accepts notes and chords from any configured musical input. A chord
fills up to four lanes and keeps its velocities. **ADD** opens an overlay for
every persistent advance from 0 through 32 rows for note/chord entry, blank,
erase, and note-off; 0 keeps the current row. The FT2 title shows `EDIT +n`.
A computer keyboard can enter notes with `Z S X D C V G B H N J M`.

**LENGTH** is a separate Step Edit overlay. It chooses `1/1`, `1/2`, `1/4`,
`1/8`, `1/16`, `1/32`, `1/64`, or `1/128` for melodic entries and defaults to `1/16`. The
selected duration writes the existing gate/explicit note-off representation;
it does not change the independent **ADD** cursor advance or create a second
timing system.

Percussion pages keep drum voices visually stable during Step Edit. For each
played note, SHR searches all four columns in earlier rows of
the current Pattern, newest row first, and reuses the column where that exact
GM drum note last appeared. The two GM bass-drum notes share a family fallback,
as do the acoustic and electric snare; with no history, bass drums start in
column 1 and snares in column 2. Other new drum voices start in columns 3–4 so
the kick/snare homes remain available. Simultaneous voices cannot share one
cell, so a collision uses the next free column. An unrelated note or command
already on the destination row is never overwritten; a note-off in a voice's
own reused/home column can be replaced by its new hit. If all four columns are
occupied, the status reports the ignored note. Existing Patterns are not
rearranged, and melodic pages retain selected-column, left-to-right chord
entry.

The editor can add a note, note-off, or blank step. It can also change the page
program and pattern master tempo, mute a lane, and move through rows, lanes,
pages, and arrangement steps.

Pressing **PLAY** on the main FT2 screen starts the first pass at the selected
row. When playback reaches the end, subsequent passes restart at row 1 of that
Pattern rather than at the original play cursor.

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
- inherited program or a MIDI program override stored as 0–127 and shown as
  instrument/program 1–128;
- one optional command: cut or delay tick 0–15, retrigger count 1–8, or tempo
  20–300 BPM.

The grid shows `C` for cut, `D` for delay, `R` for retrigger, and `T` for tempo.
One cell cannot contain more than one command. Velocity, program, gate, and
retrigger need a note-on in a newly confirmed edit. Invalid combinations stay
in the draft and show an error.

Choosing **PROGRAM** opens a full-height sound browser. A matching MIDI device
profile adds the instrument's slot labels and sound names. Without a profile,
all MIDI programs 1–128 remain available. Performance notes audition the
draft sound on that page's exact target and selected-column channel. Confirm
keeps the cell override without changing the column master; cancel restores
the previous value and selection.

## Real-time recording

From a stopped transport, **REC** loops the selected pattern and records into
the selected page; pressing **REC** again stops. During song playback, **REC**
punches in without stopping, restarting, or moving the playhead, and the next
press punches out to uninterrupted Play. Between notes, the main rotary may
select another column or page without leaving REC, and later notes use that
selected page. While one or more recorded notes are held, rotary turns are
ignored rather than queued; movement resumes only after every matching Note Off.
Played notes are placed on the selected page's four lanes and quantized to
pattern rows. Each assigned lane auditions through that column's channel/
instrument setup. During recording, those notes do not also pass to the loaded
software synth. They are auditioned only through the page's hardware MIDI
target and column channels.
The source port, not a special MIDI channel, separates a performance keyboard
from a control-only surface. A combined device retains channel-qualified
controller mappings.

Real-time recording is hardware-page-only. A page targeting a Pattern-owned
software instrument, or an `AUTO` page currently resolved to the internal
instrument, cannot enter **REC**. Choose an available hardware MIDI output
first. Real-time REC retains its separate active-note lane allocator so note
releases remain paired with overlapping held notes; the history-based drum
placement above applies only to Step Edit. **REC END**, **STOP**, **EXIT**, and
**PANIC** release auditioned notes.

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

The loop follows FT2 rewind/play, stop, restart, order/pattern
transitions, and looping. It plays at native speed and pitch; beat detection
adjusts the Pattern tempo to the WAV, not the WAV to the previous Pattern
tempo. A loop-only Project does not start the default software synth merely
because its blank Software Synth page exists. The loop player requires the JACK
server sample rate to match the WAV
sample rate. For a 44.1 kHz loop, configure/restart JACK at 44100 Hz before
loading it. A bounded 5 ms fade is applied at cut/loop edges. The 40×20 screen
shows text for filename, BPMs, region, state, elapsed/total time, rate, and
channels. A decoded loop is limited to 6,000,000 frames (about 125 seconds at
48 kHz) so one imported file cannot exhaust Raspberry Pi memory.

From **TOOLS** → **LOOP**, choose **LIBRARY** to open the shared overlay over
the loop page. Turn the master rotary to browse inbox and private WAVs and press
it to import or attach and load the selected file. Inbox, current, private, and
saved-Project entries are labelled in the overlay. Press **LIBRARY** again or
Back to close it without changing the Project.

Selecting an `INBOX` entry imports it into private storage and loads it.
Selecting `PRIVATE`, `CURRENT`, or `SAVED` attaches the existing private file
and loads it. The browser has no deletion action.

Press **REMOVE** twice to detach the loop from the
Project and unload its JACK client. The imported private WAV is kept on disk so
another Project can still use it.

**LIBRARY** is separate from Remove: its overlay browses inbox and imported
private WAVs and marks the current loop and saved-Project references without
leaving the Loop Player.

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

Use **SELECT** → **SONG** for quick Arrangement-step navigation. Choose **EDIT
ARRANGEMENT** there to edit the FT2 Arrangement separately from
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
`${XDG_DATA_HOME:-~/.local/share}/shsynth/songs/`. Current Project format 5
stores each Pattern's tempo, meter, pages, four column setups, lanes, setup
messages, cells, source insert rack, two aux routes, and master rack. Portable
pages use explicit `default` markers rather than numeric routing. Pattern-owned
software pages store explicit engine and stable instrument identities; optional
external-device profiles are stored separately from raw output/channel/bank/
program data.
Versions 0 and 1 gain empty effects routing; version 2 retains its source rack and gains
empty aux/master routing. Format 3 routes stay explicit. Version 0 page-wide setups copy the old
channel/bank/program into all four columns. Unknown newer versions, fields, or
invalid effect shapes are not loaded or overwritten. Older
`ActiveInstrument` and old `synthv1:<preset name>` routes are upgraded in
memory to explicit synthv1 engine/instrument routes and are not rewritten until
the musician explicitly saves the Project.

If an empty Pattern's routing differs from the current new-Pattern template,
**SAVE** asks: “Save this routing as the default for new patterns?” Confirming
updates the private template; cancelling saves the Project but keeps the old
template. A Pattern with notes never changes that template implicitly, and no
prompt appears when routing is unchanged. The template is stored outside the
repository at
`${XDG_DATA_HOME:-~/.local/share}/shsynth/ft2-routing-defaults.shsong` and is
used by every subsequently created Project or Pattern.

## Cleared demo songs

Setup seeds ten public-domain demo Projects into the same song directory, so
they appear on **FILES** without an import step. Matching format-1 MIDI files
and the clearance manifest live below
`${XDG_DATA_HOME:-~/.local/share}/shsynth/demos/`. Seed copies never replace a
same-named user Project. Each arrangement has separate drums, bass, pad, lead,
and counterline pages on `AUTO`, making it easy to choose new sounds or bind a
page to hardware. See [Public-domain demo songs](DEMO_SONGS.md).

## Effects saved with the Project

The Project also owns the managed instrument's ordered source insert rack, two
aux send/rack/return routes, and master rack. Those settings are independent of
the Pattern/Arrangement structure: repeating a Pattern does not duplicate an
effect, and changing Arrangement steps does not change rack order. The two aux
sends take their pre/post source-insert taps from the one managed software
instrument, not from individual MIDI lanes.

With the opt-in graph active, the managed source and wet returns, private WAV
loop, and exact configured stereo external-input return meet once before the
master rack and final meter. The loop and external input do not acquire source
inserts or aux sends; the raw multitrack recorder remains a separate capture
path. See
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
