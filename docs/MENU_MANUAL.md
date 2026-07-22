# Screen and menu manual

This is the visual guide to SHR-DAW's current 40×13 workspaces, contextual
screens, editors, and master overlays. The current controller map is
authoritative in [Controller interface](CONTROLLER_INTERFACE.md). Screenshots
are drawn by the real Rust UI from
deterministic, populated presentation states; they do not start JACK, open a
MIDI port, or claim to show a live audio measurement.

The manual is split into three chapters so it remains usable on a phone:

1. [Everyday screens](menu/EVERYDAY_SCREENS.md) — Home, Presets, Playback,
   Ideas, MIDI Learn, Help, synchronized multitrack recording, the performance
   meter, and Routing.
2. [FT2, Projects, and Patterns](menu/TRACKER_AND_PROJECTS.md) — the tracker in
   Play, Record, Edit, and Cell Edit; Tools; the N00B scale-filter switch
   across Play/Record/Edit; separate Edit note length; Projects;
   Pattern tools; drum patterns; Arrangement; the Tracks screen; and routing
   fields.
3. [Loops and effects](menu/LOOPS_AND_EFFECTS.md) — WAV loop setup, the shared
   loop browser, alignment, the effects rack and its contexts, and the
   parameter editor.

## How to read a screen

Each screenshot is a 40-column by 13-row terminal image. It is first rendered
as a native 480×208 bitmap using the project VGA console font, then enlarged to
960×416 by copying every pixel into an exact 2×2 square. There is no font
substitution, smoothing, interpolation, or antialiasing.

The normal bottom controller strip has four page positions and four action
positions:

- On an eight-button controller, the first four buttons choose the page and
  the second four run the shown actions.
- On a five-button controller, one button cycles pages and four run actions.
- On a four-button controller, press the main encoder to enter page selection,
  turn it to choose a page, press it again, then use the four buttons.
- Empty pages and actions are hidden and skipped.
- Every genuine rotary/Up/Down browsing list wraps at both ends. Functional
  entries such as Blank/Skip, AUTO, Off, Clear, and `+ INSERT EFFECT` remain
  selectable exactly once; only decorative blank lines are skipped.
- Page 1 holds the screen's primary workflow; on FT2 it is PLAY, while SELECT
  holds the page/pattern/song/route overlays. On workspaces, child screens, and editors, `SYS` item 4 is `EXIT`,
  which goes back one level. MIDI controls never quit SHR-DAW.
- `PANIC` stops owned playback and sends All Notes Off. It does not kill an
  unrelated synth or JACK client.

A master overlay temporarily changes that strip. The caller remains visible
around a centered double border, whose bottom edge shows only the highlighted
action that opened the overlay near its original physical item position. The
final row remains the shared status row. That same menu
item closes it; there is no fourth-button Back item. The rotary and Up/Down
browse, click/Enter selects or confirms, and Back/Esc cancels the current field
before cancelling and closing the overlay. Unconfirmed drafts never save on
close. On the native 40×13 display the outer rectangle is 38×11 at `(1,1)` and
the usable inner area is 36×9 at `(2,2)`.

The yellow page name at the bottom is the page currently selected. The yellow
bracketed numbers below the actions are the physical item positions. The shared
status row is always the final row on working screens.

That row begins with exactly one transport cell: steady green `>` for play,
steady white `■` for stop, steady white `‖` for pause, or red `●` for record.
Record alone pulses between red and bright red without hiding the circle. Any
text after one space is current useful state or a fault, not filler. Horizontal
meters use only circular `●` LEDs: dark gray when unlit, one consistent green
for safe active cells, yellow/red only at active thresholds, and a brighter
circle in the same threshold colour for a held peak.

## Screen flow

```mermaid
flowchart TD
    H0[Home] --> P[Software Synths / Presets]
    H0 --> T[FT2 Pattern]
    H0 --> A[Recorder]
    H0 --> M[Performance]
    H0 --> ML[MIDI Learn]
    H0 --> RTE[Routing editor]
    H0 --> FX[Effects / FX rack]
    H0 --> I[Ideas]
    H0 --> H[Help]
    P -->|Load| PB[Playback]
    M --> MO[MTR FX overlay]
    MO --> FX
    FX --> FE[FX editor]
    T --> ON[PAGE / PATTERN / SONG / ROUTE overlays]
    T --> N[N00B filter on/off]
    T --> R[Record context]
    T --> E[Edit]
    E --> CE[Cell Edit]
    ON --> TR[Tracks and routing]
    ON --> F[Project Files / Pattern tools]
    ON --> AR[Arrangement]
    ON --> TT[FT2 Tools]
    TT --> L[WAV Loop]
    F --> PT[Pattern tools]
    PT --> D[Drum patterns]
    PT --> PS[Pattern setup]
    L --> LA[Loop align]
    L -. LIBRARY overlay .-> LL[Inbox + private loop browser]
    H -. returns to its caller .-> H0
```

The Help screen returns to whichever screen opened it. `EXIT` follows the
arrows in reverse by one level. Top-level workspaces return Home; nested tools
return to their parent first.

## Naming and safety conventions

- **Project** means the whole saved tracker song: Patterns, Arrangement,
  routes, programs, loop settings, and effects.
- **Pattern** means one reusable block of rows inside a Project.
- **Page** means four tracker lanes that share a MIDI destination. Each lane
  can still have its own channel, bank, and program.
- **Idea** means a free-time MIDI take associated with a sound; it is not an
  audio recording.
- **Audio recording** means one synchronized take containing a 24-bit mono WAV
  for each armed JACK source plus a versioned session manifest. A legacy stereo
  input remains a linked two-track configuration.
- **Remove Loop** detaches the WAV from the Project and unloads SHR-DAW's loop
  JACK client. It never deletes the private WAV. The current loop browser
  imports inbox files or attaches existing private files; it has no deletion
  workflow.
- With the graph active, FX edits require stopped transport and no active
  recording. With it disabled, FX edits change saved Project data only.

For the source-of-truth page/action matrix and controller reachability rules,
see the [controller interface](CONTROLLER_INTERFACE.md). For computer keyboard
commands and deeper musical workflows, continue with [Using SHR-DAW](USING_SHR_DAW.md)
and the [tracker guide](TRACKER.md).
