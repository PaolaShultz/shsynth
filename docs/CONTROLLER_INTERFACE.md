# Controller action inventory and paging design

This document is the implementation checklist used for the paged controller
interface. The authoritative inventory was taken from `ui.rs` keyboard, mouse,
encoder, command-pad, screen, and contextual dispatch paths before paging was
implemented.

## Startup splash

Startup first shows a 40×20 old-school stereo VU animation with independent
three-row `L` and `R` bars. It uses the same dBFS colour thresholds as the live
meter: green below −12 dBFS, yellow from −12 through −3 dBFS, and red above
−3 dBFS. The animation is decorative and does not start audio, playback, or
MIDI transmission.

The splash remains visible for at least 900 ms. A terminal computer keyboard
is a fully qualified input device, equal to an available configured controller
or performance MIDI input; it is not described as a fallback. Only when none
of those inputs is available does the splash remain on `CONNECT KEYBOARD OR
MIDI INPUT` and rescan the configured MIDI inputs. `Esc` or `q` can still exit
from the splash.

## Action inventory

| Screen or mode | Existing user-facing operations and input paths |
|---|---|
| Home | Centered startup navigation root with equal-width bars for Software Synths, FT2, Recorder, Performance, MIDI Learn, Routing, Effects, Ideas, and Help. Encoder/Up/Down selects a workspace and encoder click/Enter opens it. Home has no MIDI quit command; Esc or `q` quits from the computer keyboard. |
| Presets | Select previous/next, keyboard page up/down, first/last, previous/next engine, and load the selected sound. Its physical pages contain only sound browsing, engine choice, panic, contextual help, and Exit to Home. |
| MTR | With the final bus enabled: choose Synth/Loop/Input, adjust its bounded smoothed level, toggle mute, inspect readiness/final peaks/clips/limiter reduction, and start/stop the callback-boundary final stereo recording. With it disabled: retain the passive CPU and legacy graph meter. Its FX launcher uses the same master-overlay framework as FT2, then opens the existing selected source/AUX/master rack. |
| Playback | Inspect held notes/chords, aligned decimal MIDI strike velocities, and keyboard state; toggle the N00B filter in place and, while enabled, turn the master rotary through all root plus major/natural-minor choices shown by a compact `SCALE` control; reset the 12 mapped parameters in place; open and return from the FX rack without stopping the sound; record/play/save MIDI Ideas; stop/panic; contextual help; return to Presets. N00B never replaces the Player body. The 12 configured synthv1 CC controls continuously adjust parameters with pickup. |
| Ideas | Previous/next/first/last idea; inspect, load, play, delete, record, and save; panic; contextual help; Exit to Home. |
| FT2 normal | While Play or Rec transport is active, the main rotary selects the previous/next column across page boundaries. While transport is paused it moves rows, as it does in Edit; keyboard Up/Down always moves rows. The redundant Page−/Page+/Track−/Track+ buttons are gone: PLAY holds cell edit and transport, SELECT opens PAGE/PATTERN/SONG/ROUTE rotary overlays, and SYS holds panic/N00B/help/Exit. |
| FT2 record | Record quantized notes into the selected page/current pattern and route live notes only to that page's hardware MIDI target. Rotary turns are ignored while any recorded notes are held and work again after every Note Off; stop record, stop, exit, and panic remain available. |
| FT2 edit | Musical keyboard or incoming MIDI note/chord gesture entry; independent 1/1–1/128 note length; blank/skip; erase; note off; a 0–32-row ADD value; PAGE, LENGTH, and ADD rotary overlays; and leave edit. N00B may remain on so only allowed scale notes are entered. Command notes are consumed for editing and never doubled through the synth. |
| FT2 N00B | Independent on/off scale filter layered over Play, Record, and Step Edit on a melodic page, using the scale selected on Player. Accepted notes keep their pitch; rejected notes stay silent. Play remains non-writing, while Record/Edit write only accepted notes. Toggling N00B is immediate, opens no screen, preserves the current mode, and moving to Drums turns only the filter off. |
| FT2 loop | Fourth musician-facing FT2 page; import or attach WAV; explicit `READY`/`NOT READY`/`OUTPUT FAULT`; persistent valid-region position bar/playhead; separate loop-only stereo RMS/peak/`MAX`/clip meter; confirmed Project detach without deleting the private WAV; rewind/play; source BPM and half/normal/double interpretation; start/length cuts in beat or bar units; shared inbox/private Library overlay; align child screen for auto bar alignment and one-bar placement shifts. |
| FT2 cell edit | Transactional route/channel/instrument, banks, note, gate, velocity, per-note program, single command type/parameter, clear-field, save/cancel, and panic actions. Four-button encoder page selection remains available. |
| Tracker files | Select saved Project; load; preview/stop; save with overwrite confirmation; create a confirmed blank Project; save a numbered non-overwriting copy; delete with repeat confirmation; rename; open the Pattern child; back/cancel and panic. |
| Pattern tools | New, clone, clear, copy, paste-new, paste-over, or clean unused Patterns; transpose melodic pages by semitone or octave; open reusable drum patterns. |
| Drum patterns | Filter 72 bundled plus user rhythms by genre, meter, and 2/4/8-bar size; load into the percussion page; save that page separately; confirmed deletion of user saves only; list navigation. Empty Patterns may adopt the selected shape, while existing melody blocks resizing. |
| FT2 arrange | Select arrangement step; append/insert current pattern; duplicate/remove step; move step earlier/later; jump to referenced pattern; play from selected step; back and panic. |
| Pattern setup | Choose 3/4 or 4/4 and pattern size; confirm new/destructive resize, cancel, or clear while retaining the current size. |
| Tracks page manager | Select pages with the encoder; add a four-lane page; edit target, column, channel, bank, and program; confirm all changes; or exit and restore the original Project. |
| Target/channel field mode | Previous/next choice, confirm field, cancel field. Encoder turn/press and menu items share these operations. |
| Audio recorder | Select and name a track; assign an exact discovered JACK source; arm/disarm one, every resolved track, or all; refresh source discovery without rewriting preferences; start/stop one synchronized take; inspect elapsed time, active count, selected-track activity, drop/xrun/high-water status, final path or failure; Exit to Home and panic. |
| FX rack/editor | Choose source, AUX 1, AUX 2, or master; select the typed `+ INSERT EFFECT` row; add/select/remove/bypass/reorder bounded effects; and edit every parameter together at 40×20 using explicit compact labels and type-aware values. Aux time effects are forced wet. An active graph publishes FX changes only with stopped transport and recording; a disabled graph accepts Project-only edits without touching audio. |
| Routing | Transactional rotary editor for controller input/role, performance input, external enable/output/profile, controller clock enable/output, and audio output. Browsing never writes or transmits. Field confirmation validates the whole candidate, backs up and atomically saves it, safely activates live MIDI input changes, refreshes discovery, and rolls back on failure. Interface availability and unverified downstream DIN profile are separate states. |
| Help | Compact Markdown user help, temporary LAN web help when port 80 is available, section links selected by the master encoder, keyboard page scrolling, top, and return to the previous screen. |
| Global/safety | Stop MIDI playback, tracker transport, recorder, managed engine, and owned notes; All Notes Off; cancel or leave the current controller level. Application exit remains computer-keyboard-only. Help is also reachable from `?` or F1. Process termination remains limited to the engine owned by SHR-DAW. |

The complete final screen × page × item mapping is maintained below. The table
uses expanded action names where that is clearer; the compact visible label is
shown in parentheses when it differs materially. `src/navigation.rs` is the
executable canonical copy: labels and dispatch actions are one definition. A unit test builds the
union of every normal and contextual menu and checks every action in this
  screen-specific inventory for controller reachability. Top-level Home entries
  are reached by the master rotary rather than duplicated on child command pages.

## Master overlays

An overlay is transient state above its caller, not another `Screen` and not a
second Project/engine owner. Its central state records identity, caller, title,
canonical launcher, selection/scroll, active field snapshot, typed draft, and
the caller's controller-page state. At 40×20 its outer rectangle is exactly
`x=1`, `y=1`, `width=38`, `height=18`; the bordered inner content is exactly
`x=2`, `y=2`, `width=36`, `height=16`. Compact terminals clamp those values
without drawing outside the terminal.

While open, only the launcher action remains on the bottom row, in its original
physical item position and with an active highlight. All other page and item
commands are hidden and silent. There is no controller-strip Back button.
Pressing the highlighted launcher again closes the overlay. The rotary and
Up/Down browse; rotary click and Enter select or confirm. Back/Esc cancels an
active field first, then cancels the overlay draft and closes, before a later
Back can leave the caller. Four-button page-selection state and every layout's
previous page are restored deterministically.

FT2 demonstrates seven caller-specific adapters: PAGE navigates four-column
locations and links to Tracks; PATTERN navigates the Project's existing Pattern
owners and links to Pattern/Project tools; SONG navigates Arrangement steps and
links to its detailed editors; ROUTE edits a detached copy of the active page
and applies it only through the existing Project/route synchronization path;
Step Edit LENGTH chooses 1/1 through 1/128 and ADD chooses 0 through 32 rows;
Pattern Setup LNGTH chooses every value from 1 through 32 plus 48, 64, 96,
128, 192, and 256.
The Loop Player's LIBRARY launcher uses it for one combined inbox/private
browser; inbox selection imports and loads, while private/current/saved
selection attaches and loads. MTR's FX launcher reuses the same rendering,
input, toggle, and return layer.

## Input model

- Eight buttons: four direct page selectors plus four item buttons.
- Five buttons: one page-cycle button plus four item buttons.
- Four buttons: four item buttons; encoder press enters/leaves page-selection
  mode and encoder turn changes pages while that mode is visible.
- Outside four-button page-selection mode, encoder turns retain list, row, and
  field adjustment except on the normal FT2 grid: active Play or Rec transport
  uses turns for cross-page column selection, while paused transport and Edit
  keep row movement. Encoder press
  retains the existing select/confirm action on eight- and five-button layouts.
  Menu slots do not duplicate those master rotary selection actions.
- An open overlay always gives the encoder to overlay browsing/editing, so a
  four-button controller cannot become stranded in page-selection mode.
- Entering any screen or contextual mode selects its page 1, preventing a page
  choice from a previous visit from becoming the new screen's hidden meaning.
- Page 1 holds the primary screen workflow; for FT2 normal mode it is PLAY.
  On every workspace, child screen,
  and contextual editor, `EXIT` is page 4/item 4 and returns exactly one level.
  Home is the root and has no MIDI Exit; quitting remains keyboard-only.
- When a configured controller is offline, lacks a matching reviewed profile,
  or has an incomplete learned encoder, Home initially selects MIDI Learn and
  gives the reason. A learned master encoder with turn and click is usable even
  without optional command buttons. Home itself neither learns nor transmits.
- Help is a child screen. It tries to show the same help at
  `http://<LAN-IP>/help` while open. The master encoder moves one help row at a
  time. Encoder press follows a highlighted internal section link on eight-
  and five-button layouts; four-button layouts use OPS `OPEN` because encoder
  press is reserved for page selection. The compact help text uses a stable
  38-column width so link targets and rendered rows remain identical.
- Target/channel fields use encoder press to confirm on eight- and five-button
  layouts. Four-button layouts use the visible OPS `CONFIRM` item; SYS `EXIT`
  cancels the field on every layout.
- Empty items and pages are not drawn, are silent when pressed, and are skipped
  by page cycling. The interface exposes working actions only.
- Physical command pages never contain PageUp/PageDown. Keyboard
  PageUp/PageDown retain their existing behavior, while the rotary continues
  ordinary one-step list and row movement.
- Every genuine rotary/Up/Down browse list wraps first-to-last and last-to-first,
  including Home, file/library lists, Arrangement, tracker browse cursors,
  recorder/meter/FX lists, overlays, Routing rows, and enumerated field choices.
  Empty lists are inert, one-item lists remain stable, stale selections clamp
  before wrapping, and scroll offsets follow the selected row. Bounded numeric
  editing does not inherit list wrapping.
- Functional sentinels are typed logical entries, not inferred from their
  visual text. Blank/Skip, Off, Clear, Default/AUTO, and FX `+ INSERT EFFECT`
  therefore remain distinct and reachable exactly once; decorative blank lines
  remain non-selectable.
- The rendered controller strip is centered and capped at 40 columns. Labels
  and brackets use their natural width instead of expanding with the terminal.
- Command notes and CCs may be qualified by MIDI channel. The MiniLab factory
  Arturia/DAW pads are notes 36–43 on channel 10: 36–39 select pages 1–4 and
  40–43 activate items 1–4. Matching pressure and releases are consumed, while
  the same notes on channel 1 remain keyboard input. User 1's captured
  channel-1 pads cannot safely be commands because they collide with the keys.

## Complete controller map

Blank physical positions and wholly empty pages are omitted.

| Screen/context | Page | Item 1 | Item 2 | Item 3 | Item 4 |
|---|---|---|---|---|---|
| Presets | Ops | Load | First | Last | — |
| Presets | Engine | Engine− | Engine+ | — | — |
| Presets | Sys | Panic | Help | — | Exit |
| MTR | Ops | Source− | Source+ | Level− | Level+ |
| MTR | Mix | Mute | — | Final rec/stop | Reset holds |
| MTR | Nav | FX overlay | — | — | — |
| MTR | Sys | Panic | — | Help | Exit |
| Playback | Play | — | Play take | Record MIDI | — |
| Playback | Sound | Reset controls | Save | N00B on/off | — |
| Playback | Sys | Panic | FX | Help | Exit |
| FX rack | Ops | Add | Delete | Edit type | Parameters |
| FX rack | Order | Up | Down | Bypass | — |
| FX rack | Route | Target | Send− | Send+ | Point |
| FX rack | Sys | Panic | Return | Help | Exit |
| FX rack empty | Ops | Add | — | — | — |
| FX rack empty | Route | Target | Send− | Send+ | Point |
| FX rack empty | Sys | Panic | Return | Help | Exit |
| FX type | Type | Type− | Type+ | OK | Cancel |
| FX editor | State | Bypass | — | — | — |
| FX editor | Sys | Panic | — | Help | Exit |
| Ideas | Play | Inspect | Play | Record | Delete |
| Ideas | File | Load | Save | First | Last |
| Ideas | Sys | Panic | — | Help | Exit |
| Help | Ops | Open link | Top | — | — |
| Help | Sys | Panic | — | — | Exit |
| FT2 | Play | Cell edit | Play | Record | Step edit |
| FT2 | Select | Page overlay | Pattern overlay | Song overlay | Route overlay |
| FT2 | Sys | Panic | N00B | Help | Exit |
| FT2 tools | Ops | Arrange | Loop | FX | Mute lane |
| FT2 tools | Clip | Copy lane (`COPY L`) | Paste lane (`PASTE L`) | Copy page (`COPY PG`) | Paste page (`PSTE PG`) |
| FT2 tools | Page | Mute page (`MUTE PG`) | — | — | — |
| FT2 tools | Sys | Panic | Help | — | Exit |
| FT2 Record | Play | N00B | Play | Record | — |
| FT2 loop | Play | Rewind | Play | Import | Remove |
| FT2 loop | BPM | BPM− | BPM+ | BPM x | Unit |
| FT2 loop | Cut | Start− | Start+ | Length− | Length+ |
| FT2 loop | Sys | Panic | Align | Library | Exit |
| FT2 loop align | Ops | Auto | Bar− | Bar+ | Done |
| FT2 loop align | Sys | Panic | Help | — | Exit |
| FT2 record | Play | N00B | Play | Record/stop | — |
| FT2 record | Sys | Panic | Help | — | Exit |
| FT2 step edit | Ops | Blank/skip | Erase | N-off | N00B |
| FT2 step edit | Set | Page overlay | ADD 0–32 overlay | Note-length overlay | — |
| FT2 step edit | Sys | Panic | Help | — | Exit edit |
| FT2 cell edit | Route | Destination | Channel | Instrument | — |
| FT2 cell edit | Sound | Bank MSB | Bank LSB | Cell program | Clear field |
| FT2 cell edit | Cell | Note | Gate | Velocity | Effect |
| FT2 cell edit | Done | Panic | Save | Effect parameter | Exit/cancel |
| Files | Ops | Load | Save | Preview/stop | Delete |
| Files | Project | New Project | Save As | Name/rename | Pattern tools |
| Files | Sys | Panic | — | Help | Exit |
| Routing-default prompt | Default | Confirm | Cancel | — | — |
| Routing-default prompt | Sys | Panic | — | — | Exit/cancel |
| Pattern tools | Ops | New | Clone | Clear | Drum patterns |
| Pattern tools | Clip | Copy | Paste new (`NEW`) | Paste over (`OVER`) | Clean unused (`CLEAN`) |
| Pattern tools | Trans | Octave− (`OCT-`) | Semitone− (`NOTE-`) | Semitone+ (`NOTE+`) | Octave+ (`OCT+`) |
| Pattern tools | Sys | Panic | — | Help | Exit |
| Drum patterns | Ops | Load | Save | Delete user | — |
| Drum patterns | Filter | Genre− | Genre+ | Meter | Size |
| Drum patterns | Move | First | Last | — | — |
| Drum patterns | Sys | Panic | — | Help | Exit |
| Arrange | Ops | Jump | Play | Append | Insert |
| Arrange | Step | Up | Down | Repeat | Remove |
| Arrange | Sys | Panic | Help | — | Exit |
| Pattern setup | Ops | 3/4 | 4/4 | Size− | Size+ |
| Pattern setup | Apply | Confirm | Keep | — | — |
| Pattern setup | Sys | Panic | — | Help | Exit/cancel |
| Tracks | Ops | Add four lanes | Target | Channel | Done |
| Tracks | Column | Column− | Column+ | Program− | Program+ |
| Tracks | Bank | MSB− | MSB+ | LSB− | LSB+ |
| Tracks | Sys | Panic | — | Help | Exit/cancel |
| Target/channel editor | Ops | Confirm | — | — | — |
| Target/channel editor | Sys | Panic | — | Help | Exit/cancel |
| Audio recorder | Record | — | — | Record/toggle | Arm selected |
| Audio recorder | Track | Previous track | Next track | Assign source | Name track |
| Audio recorder | Setup | Arm all resolved | Disarm all | Refresh sources | — |
| Audio recorder | Sys | Panic | — | Help | Exit |
| Routing | Edit | Previous row/value | Next row/value | Edit/OK | Cancel |
| Routing | Sys | Panic | Help | — | Exit |

## Routing editor contract

Routing opens in browse mode with a highlighted row. Rotary/Up/Down moves one
row and wraps; click/Enter opens a detached field draft; rotary/Up/Down changes
only that draft; click/Enter validates and confirms; Back/Esc restores the
original field. Back/Esc from browse returns Home. Re-entry always starts with
clean browse state.

Confirmation validates the complete runtime and controller candidate, creates
non-overwriting backups, atomically replaces both files, releases source-owned
notes/controller state, replaces SHR-owned MIDI inputs without layering, and
refreshes live discovery. Failure restores the old files and runtime route.
An audio-output change is saved for the next managed engine start and reported
as `AUDIO NEXT START` instead of being described as hot/live. Controller-clock
enable/output changes likewise report `CLOCK NEXT START`; live MIDI input role/
source changes activate immediately. Selecting
or confirming a MIDI output uses discovery only; it never opens an output as a
probe and never transmits.

The MIDI row describes the selected ALSA interface port as `ONLINE` or
`OFFLINE`. The Device row describes only the configured profile and remains
`UNVERIFIED` for a downstream DIN instrument. `AudioBox · ONLINE` plus
`D-50 · UNVERIFIED` is therefore the truthful expected presentation; SHR never
claims that the D-50 itself was detected.

## FX editor and 40×20 text contract

The FX editor is a spatial 2×4 grid matching the eight physical rotary
positions. Every control has its title above its value; the selected pair is
highlighted yellow while browsing and green while editing. Titles use clear
words such as `RATE`, `RATIO`, `ATTACK`, and `FEEDBACK`, while values retain
type-aware units. EQ is deliberately mapped as low, low-mid, high-mid, and
high frequency on knobs 1–4 with their matching gains on knobs 5–8. Its
secondary low-cut and output-trim Project values remain compatible but are not
misrepresented as knob 1. Every effect exposes at most eight performance
controls; full persisted schema names and unassigned secondary values do not
change.

All working-screen single-line regions have explicit terminal-cell budgets.
Static operational labels are written to fit; unpredictable device/file/user
names pass through cell-aware fitting; fixed label/value rows reserve the
selection marker and right-side state. Help remains the intentional wrapped,
scrollable prose surface. The controller footer, `DEV`/`REL` badge, Help, and
Exit areas retain their assigned cells at 40×20.

## FT2 cell editor inventory and mapping

A cell contains `note`, optional `velocity`, optional per-note `program`,
optional `gate`, and one `command`: none, cut, delay, retrigger, or tempo. Song
format stores all of these fields directly inside each FT2 Pattern.

| Page | Item 1 | Item 2 | Item 3 | Item 4 |
|---|---|---|---|---|
| Route | Destination | Channel | Instrument | — |
| Sound | Bank MSB | Bank LSB | Cell program | Clear selected field |
| Cell | Note | Gate | Velocity | Effect type |
| Done | Panic | Save | Effect parameter | Exit/cancel |

The first display spacer uses `C` for cut, `D` for delay, `R` for retrigger,
`T` for tempo, and blank for no command. The data model supports one command
per cell. Gate is 1–100% of a row or inherited; delayed notes and retrigger
pulses are bounded by the row. Program is a per-note override of the page
program, routed before the note on the same exact target/channel.

Physical MIDI notes and CCs remain configuration. Older `arp`, `pad`, `prog`,
`loop`, `stop`, `play`, `rec`, and `tap-tempo` pad role aliases load as the
same physical first-through-eighth positions, so local profiles can move to
page 1–4 and item 1–4 without changing note numbers.

## Parameters, pickup, and extension points

Menu navigation is discrete. The 12 synthv1 controls are continuous and remain
on configured CCs. Preset load, idea load, and in-place reset re-arm pickup;
the verified synthv1 0.9.29 indices/ranges and green/yellow/red ±0.03 indicators
are unchanged. `MAPPED_CONTROL_CAPACITY` reserves 16 entries while only the 12
schema-verified controls are populated.

`Action` and the empty menu slots remain extension points. Future features are
not shown on the hardware menu until they actually dispatch a working action.
