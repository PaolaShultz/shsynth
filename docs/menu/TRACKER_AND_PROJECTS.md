# FT2, Projects, and Patterns

[Manual home](../MENU_MANUAL.md) · [Everyday screens](EVERYDAY_SCREENS.md) ·
[Loops and effects](LOOPS_AND_EFFECTS.md)

SHR-DAW's FT2 screen is a compact vertical MIDI Pattern sequencer inspired by
tracker workflow. It is not an XM editor or a clone of FastTracker II. A
Project owns several Patterns and an Arrangement order. Each Pattern has one or
more four-lane pages. Portable `AUTO` pages defer destination and channels to
the active machine; explicit pages retain a destination plus each column's
channel, bank, and program.

The screenshots use a populated demonstration Project. External routes are
shown as offline where no actual device was opened for documentation.

## FT2 Pattern — Play mode

Turn the main encoder to move through rows. Left/right move the order or lane
with the keyboard. The highlighted row is the next edit/play location.

### MOVE — page and track navigation

![Populated FT2 Pattern in Play mode with the OPS page](../images/menu/ft2-play-ops.png)

`PAGE-` and `PAGE+` move between the Pattern's four-track pages. `TRACK-` and
`TRACK+` move the column cursor, crossing a page boundary when needed. These
high-value tracker movements occupy controller page 1.

### PLAY — transport and entry

![Populated FT2 Pattern in Play mode with the MODE page](../images/menu/ft2-play-mode.png)

`CELL` opens the transactional editor. `PLAY` toggles tracker transport,
`RECORD` begins or ends real-time recording on the current external-MIDI page,
and `STEP` toggles step entry.

### OPEN — tracker children

![Populated FT2 Pattern in Play mode with the MOVE page](../images/menu/ft2-play-move.png)

`TRACKS`, `FILES`, and `TOOLS` open FT2-specific child workflows. `TAP` updates
the Project tempo from repeated presses. Unrelated top-level workspaces live on
Home instead of this controller page.

### SYS — stop, tools, and exit

![Populated FT2 Pattern in Play mode with the SYS page](../images/menu/ft2-play-sys.png)

`PANIC` stops all owned notes and transports. `N00B` opens scale setup or turns
an active filter off without leaving Play. `HELP` opens contextual help. `EXIT`
returns Home.

## FT2 Pattern — real-time Record context

Record is allowed only on a page routed to external MIDI. Incoming notes are
consumed before the loaded software synth, auditioned on that page's exact
target/channel, quantized into the looping current Pattern, and written only to
that page.

### PLAY — transport, capture, and filter

![Populated FT2 Pattern recording context with the PLAY page](../images/menu/ft2-record-play.png)

`N00B` opens scale setup or turns the active filter off without ending capture.
`PLAY` controls transport. `RECORD` ends real-time capture while preserving the
notes already entered. With N00B on, only allowed notes are heard and written.

### SYS — emergency and normal exits

![Populated FT2 Pattern recording context with the SYS page](../images/menu/ft2-record-sys.png)

`PANIC` performs the global owned stop. `HELP` explains the current mode.
`EXIT` leaves the recording context safely.

## FT2 Pattern — Step Edit context

In Step Edit, a computer key or incoming MIDI gesture writes a note or chord at
the cursor. Command-pad notes are consumed as controls and are not doubled into
the Pattern or synth. The persistent ADD value chooses how many rows the cursor
advances after entry, blank, erase, or note-off.

On a percussion page, entry searches earlier rows across all four columns and
reuses each drum voice's most recent column. New bass drums and snares prefer
columns 1 and 2; other new voices begin in columns 3 and 4. Occupied cells are
preserved, and simultaneous voices that want the same column fall through to a
free one. Melodic pages still fill from the selected column.

### OPS — enter or remove cells

![Populated FT2 Step Edit with the OPS page](../images/menu/ft2-step-edit-ops.png)

`BLANK` advances without writing a note. `ERASE` clears the selected cell.
`N-OFF` writes a note-off. `N00B` opens scale setup or turns the active filter
off while Step Edit remains active.

### MOVE — order and lane cursor

![Populated FT2 Step Edit with the MOVE page](../images/menu/ft2-step-edit-move.png)

`PG-`, `PG+`, `LANE-`, and `LANE+` move the edit cursor without changing
Pattern data.

### ADD — choose row advance

![Populated FT2 Step Edit with the ADD page](../images/menu/ft2-step-edit-add.png)

`1`, `2`, `4`, and `8` set the persistent number of rows added after each step
operation. This affects movement, not note duration or tempo.

### SYS — stop, next page, and leave edit

![Populated FT2 Step Edit with the SYS page](../images/menu/ft2-step-edit-sys.png)

`PANIC` and `STOP` retain their safety meanings. `PAGE` moves to the next
four-lane page. `EXIT` leaves Step Edit and returns to Play mode.

## FT2 Cell Edit

Cell Edit uses a draft copy: adjustments are not published until `CONFIRM`.
The cell can contain a note, inherited or explicit velocity, inherited or
explicit gate, an optional per-note program, and one command: cut, delay,
retrigger, tempo, or none.

### OPS — commit and change command type

![Populated FT2 Cell Edit with the OPS page](../images/menu/ft2-cell-edit-ops.png)

`CONFIRM` commits the whole draft. `STEP` commits and hands off to Step Edit.
`CLEAR` clears only the selected field. `EFFECT` cycles the command type.

### FIELDS — select the value to edit

![Populated FT2 Cell Edit with the FIELDS page](../images/menu/ft2-cell-edit-fields.png)

`NOTE`, `GATE`, `VEL`, and `PROGRAM` select the corresponding field. Gate is a
percentage of one row; inherited values use the page/project default. Program
is sent before that note on the exact target and channel.

### ADJUST — command parameter and value

![Populated FT2 Cell Edit with the ADJUST page](../images/menu/ft2-cell-edit-adjust.png)

`PARAM` selects the current command's parameter. `VALUE-` and `VALUE+` adjust
the selected field within its validated range. Turning the encoder performs
the same adjustment.

### SYS — cancel without partial edits

![Populated FT2 Cell Edit with the SYS page](../images/menu/ft2-cell-edit-sys.png)

`PANIC` and `STOP` stay reachable. `EXIT` cancels and restores the original
cell, so a half-edited draft never leaks into the Project.

## FT2 Tools

This child screen keeps the main Pattern screen compact. It routes to track
configuration, Project files, Arrangement, clip operations, and WAV loops.

### OPS — open focused tools

![Populated FT2 Tools screen with the OPS page](../images/menu/ft2-tools-ops.png)

`PAGES` opens the TRACKS screen for pages, columns, and MIDI routing. `FILES`
opens Project and Pattern management. `ARR` opens the Pattern order. `MUTE` toggles the
selected lane.

### CLIP — lane and page clipboard

![Populated FT2 Tools screen with the CLIP page](../images/menu/ft2-tools-clip.png)

`COPY L`, `PASTE L`, `COPY PG`, and `PSTE PG` copy or paste the current lane or full
four-lane page. These are in-memory editing clipboards, not saved Projects.

### LOOP — attach or detach private audio

![Populated FT2 Tools screen with the LOOP page](../images/menu/ft2-tools-loop.png)

`LOOP` opens WAV-loop setup. `REMOVE` detaches the loop from this Project after
confirmation; it does not delete the private WAV file. `LIBRARY` opens safe
private-WAV cleanup. `MUTE PG` toggles the current four-lane page.

### SYS — safety, help, and return

![Populated FT2 Tools screen with the SYS page](../images/menu/ft2-tools-sys.png)

`PANIC`, `STOP`, and `HELP` retain their normal meanings. `EXIT` returns to the
Pattern editor.

## N00B filter and Step Edit note length

N00B is an independent scale-filter switch, not a fourth FT2 mode and not a
duration control. Choose any chromatic root plus major or natural minor. On the
selected melodic page, an in-scale key keeps its original pitch and an
out-of-scale key stays silent; no rejected key is shifted to a nearby note.
The filter can stay on while playing, recording, or using Step Edit. Play does
not write cells; Record and Step Edit write only allowed notes. The N00B button
is reachable in all three modes and turns the active filter off without
changing mode. Moving onto Drums also turns only the filter off.

Note duration belongs separately to Step Edit. `LENGTH` opens a rotary selector
for `1/1`, `1/2`, `1/4`, `1/8`, `1/16`, and `1/32`; `1/16` is the default.
The duration uses the existing gate/note-off representation. It does not alter
the independent 1/2/4/8-row `ADD` value that chooses the next insertion row.

## Project Files

Files manages complete saved Projects. Names shown to the musician are
editable. Save and Save As publish atomically and never silently replace a
collision. Preview uses the selected saved Project without treating it as the
current edit.

### OPS — load, save, preview, delete

![Populated Project Files screen with the OPS page](../images/menu/files-ops.png)

`LOAD` opens the selected Project. `SAVE` writes the current Project and asks
before replacement. `PREVIEW` starts or stops the selected Project preview.
`DELETE` requires repeat confirmation.

### PROJECT — lifecycle and Pattern child

![Populated Project Files screen with the PROJECT page](../images/menu/files-project.png)

`NEW` creates a confirmed blank Project. `SAVE AS` writes a numbered
non-overwriting copy. `NAME` edits the Project display name. `PATTERN` opens
Pattern tools.

### SYS — stop and return

![Populated Project Files screen with the SYS page](../images/menu/files-sys.png)

`PANIC`, `STOP`, and `HELP` remain available. `EXIT` cancels pending file
actions and returns to the tracker.

## Pattern tools

Pattern tools operate on the Pattern referenced by the current Arrangement
step. Cleanup deletes only zero-reference Patterns; it never rewrites the
Arrangement behind the user's back. Transposition affects melodic pages only.

### OPS — Pattern lifecycle

![Populated Pattern tools with the OPS page](../images/menu/pattern-tools-ops.png)

`NEW` opens Pattern setup. `CLONE` creates a separate copy and selects it.
`CLEAR` opens a confirmed clear/resize setup. `DRUMS` opens reusable rhythms.

### CLIP — Pattern clipboard and cleanup

![Populated Pattern tools with the CLIP page](../images/menu/pattern-tools-clip.png)

`COPY` stores the current Pattern in memory. `NEW` creates a new Pattern from
it. `OVER` asks before replacing the current Pattern. `CLEAN` deletes
only Patterns not referenced by any Arrangement step.

### TRANS — transpose melody only

![Populated Pattern tools with the TRANS page](../images/menu/pattern-tools-trans.png)

`OCT-`, `NOTE-`, `NOTE+`, and `OCT+` transpose melodic notes by −12, −1, +1,
or +12 semitones. Percussion pages and note-off commands are left unchanged.

### SYS — stop and return

![Populated Pattern tools with the SYS page](../images/menu/pattern-tools-sys.png)

`PANIC`, `STOP`, and `HELP` stay available. `EXIT` returns to Project Files.

## Drum patterns

The library contains bundled read-only grooves plus user-saved four-lane drum
Patterns. Filters select genre, 3/4 or 4/4, and supported two-, four-, or
eight-bar row sizes. Loading may resize an empty melodic Pattern, but refuses a
shape change once melody exists.

### OPS — load and manage a rhythm

![Populated drum-pattern library with the OPS page](../images/menu/drum-patterns-ops.png)

`LOAD` writes the selected rhythm into the percussion page without changing
its route. `SAVE` stores the current percussion page as a user rhythm.
`DELETE` can remove only a user save and requires confirmation.

### FILTER — narrow the library

![Populated drum-pattern library with the FILTER page](../images/menu/drum-patterns-filter.png)

`GENRE-` and `GENRE+` move among the available genres and `ALL`. `METER`
toggles 3/4 and 4/4. `SIZE` cycles the supported Pattern lengths for that meter.

### MOVE — navigate a long result list

![Populated drum-pattern library with the MOVE page](../images/menu/drum-patterns-move.png)

`FIRST` and `LAST` move to the filtered result-list boundaries without loading
anything. Turn the rotary for one-step movement, type a first letter to jump,
or use keyboard PageUp/PageDown for coarse scrolling; physical pads omit the
coarse page commands.

### SYS — stop and return

![Populated drum-pattern library with the SYS page](../images/menu/drum-patterns-sys.png)

`PANIC`, `STOP`, and `HELP` remain available. `EXIT` returns to Pattern tools.

## Pattern setup

This confirmation context chooses musical meter and row count before a new or
destructively cleared Pattern is created. The supported sizes represent two,
four, eight, sixteen, or thirty-two bars in the selected meter.

### OPS — meter and size

![Populated Pattern setup with the OPS page](../images/menu/pattern-setup-ops.png)

`3/4` and `4/4` choose the meter. `SIZE-` and `SIZE+` move among the matching
row counts. Turning the encoder also changes size.

### APPLY — confirm or preserve

![Populated Pattern setup with the APPLY page](../images/menu/pattern-setup-apply.png)

`CONFIRM` performs the new/clear operation with the displayed shape. `KEEP`
cancels the destructive reset and retains the current Pattern size.

### SYS — safety and cancellation

![Populated Pattern setup with the SYS page](../images/menu/pattern-setup-sys.png)

`PANIC` and `HELP` remain available. `EXIT` cancels the setup and returns to
Pattern tools.

## Arrangement

Arrangement is the ordered list of Pattern IDs that forms the Project
timeline. Repeated steps reference the same Pattern until it is cloned.

### OPS — play and insert Pattern references

![Populated Arrangement screen with the OPS page](../images/menu/arrange-ops.png)

`PLAY` starts at the selected step. `JUMP` opens that step's Pattern in the
editor. `APPEND` adds the current Pattern at the end. `INSERT` adds it before
the selected step.

### STEP — reorder and repeat

![Populated Arrangement screen with the STEP page](../images/menu/arrange-step.png)

`UP` and `DOWN` move the selected step earlier or later. `REPEAT` duplicates
the reference. `REMOVE` removes only this step, not the underlying Pattern.

### SYS — stop and return

![Populated Arrangement screen with the SYS page](../images/menu/arrange-sys.png)

`PANIC`, `STOP`, and `HELP` remain available. `EXIT` returns to the tracker.

## Tracks and routing

The Tracks screen edits four-lane pages. Changes are kept as a draft until
`DONE`; `EXIT` restores the original Project. Turn the encoder to choose a page
in normal mode. A destination is shared by the page, while channel, bank, and
program belong to the selected column.

### OPS — add and route pages

![Populated Tracks screen with the OPS page](../images/menu/tracks-ops.png)

`ADD` adds one four-lane page. `TARGET` opens the destination field. `CHANNEL`
opens the selected column's MIDI channel field. `DONE` validates conflicts and
keeps all page-manager changes.

### COLUMN — choose column and program

![Populated Tracks screen with the COLUMN page](../images/menu/tracks-column.png)

`COL-` and `COL+` select one of the page's four columns. `PROG-` and `PROG+`
choose its 0–127 program, using a device profile's name when available.

### BANK — choose the selected column's bank

![Populated Tracks screen with the BANK page](../images/menu/tracks-bank.png)

`MSB-`, `MSB+`, `LSB-`, and `LSB+` adjust the MIDI bank-select bytes for the
selected column. The configured bank-select order is honored during playback.

### SYS — mute, cancel, and safety

![Populated Tracks screen with the SYS page](../images/menu/tracks-sys.png)

`PANIC` and `STOP` remain available. `MUTE` toggles the whole current page.
`EXIT` cancels the entire Tracks draft and restores the original Project.

## Target field editor

The target field lists discovered synthv1 presets, the configured external
route, and discovered named MIDI outputs. A synth choice belongs to the Pattern,
not the standalone Software Synth workspace. Offline selections are retained in
the Project rather than silently rewritten.

### OPS — confirm destination

![Populated target editor with the OPS page](../images/menu/target-editor-ops.png)

Turn the encoder to choose a device. `CONFIRM` applies the field to the draft
page and returns to Tracks. On eight- and five-button layouts, encoder press is
also confirm.

### SYS — cancel only this field

![Populated target editor with the SYS page](../images/menu/target-editor-sys.png)

`PANIC`, `STOP`, and `HELP` stay available. `EXIT` cancels only the target
field and returns to the unchanged Tracks draft.

## Channel field editor

Channel editing affects only the selected column. The visible value is 1–16;
the persisted MIDI byte remains the standard zero-based 0–15 representation.

### OPS — confirm channel

![Populated channel editor with the OPS page](../images/menu/channel-editor-ops.png)

Turn the encoder to choose 1–16. `CONFIRM` applies the field and returns to
Tracks. Encoder press also confirms on eight- and five-button layouts.

### SYS — cancel only this field

![Populated channel editor with the SYS page](../images/menu/channel-editor-sys.png)

`PANIC`, `STOP`, and `HELP` stay available. `EXIT` discards only the channel
draft and returns to Tracks.
