# Controller action inventory and paging design

This document is the implementation checklist used for the paged controller
interface. The authoritative inventory was taken from `ui.rs` keyboard, mouse,
encoder, command-pad, screen, and contextual dispatch paths before paging was
implemented.

## Action inventory

| Screen or mode | Existing user-facing operations and input paths |
|---|---|
| Home | Startup navigation root. Encoder/Up/Down selects a musician-facing workspace and encoder click/Enter opens it. Home has no MIDI quit command; Esc or `q` quits from the computer keyboard. |
| Presets | Select previous/next, keyboard page up/down, first/last, previous/next engine, and load the selected sound. Its physical pages contain only sound browsing, engine choice, panic, contextual help, and Exit to Home. |
| MTR | With the final bus enabled: choose Synth/Loop/Input, adjust its bounded smoothed level, toggle mute, inspect readiness/final peaks/clips/limiter reduction, and start/stop the callback-boundary final stereo recording. With it disabled: retain the passive CPU and legacy graph meter. Master level is visible; back, help, and panic remain reachable. |
| Playback | Inspect held notes/chords, aligned decimal MIDI strike velocities, and keyboard state; reset the 12 mapped parameters in place; record/play/save MIDI Ideas; stop/panic; contextual help; return to Presets. The 12 configured synthv1 CC controls continuously adjust parameters with pickup. |
| Ideas | Previous/next/first/last idea; inspect, load, play, delete, record, and save; panic; contextual help; Exit to Home. |
| FT2 normal | Previous/next row (keyboard/encoder); Page−/Page+/Track−/Track+ on controller page 1; play, record, cell edit, and step edit on page 2; child Tracks, Files, and Tools on page 3; panic/help/Exit on page 4. |
| FT2 record | Record quantized notes into only the current page/current pattern; route live notes only to that page's hardware MIDI target; stop record, stop, exit, and panic remain available. |
| FT2 edit | All cursor and transport operations; musical keyboard or incoming MIDI note/chord gesture entry; blank/skip; erase; note off; 1/2/4/8-row entry advance; leave edit; lane mute. Command notes are consumed for editing and never doubled through the synth. |
| FT2 N00B | Enter notes on the selected page with a visible 1/1–1/32 length; open the one-item rotary length selector; delete, write note-off, change page/track, play, save, open Files, return to normal mode, or Exit. Existing cells are unchanged by mode switches. |
| FT2 loop | Select/import WAV; inspect its separate loop-only stereo RMS/peak/`MAX`/clip meter; confirmed Project detach without deleting the private WAV; play here/from start/stop; source BPM and half/normal/double interpretation; start/length cuts in beat or bar units; align child screen for auto bar alignment and one-bar placement shifts. |
| FT2 cell edit | Transactional note, gate, velocity, per-note program, single command type/parameter, clear-field, confirm/cancel, step-entry handoff, stop, and panic actions. Four-button encoder page selection remains available. |
| Tracker files | Select saved Project; load; preview/stop; save with overwrite confirmation; create a confirmed blank Project; save a numbered non-overwriting copy; delete with repeat confirmation; rename; open the Pattern child; back/cancel and panic. |
| Pattern tools | New, clone, clear, copy, paste-new, paste-over, or clean unused Patterns; transpose melodic pages by semitone or octave; open reusable drum patterns. |
| Drum patterns | Filter 72 bundled plus user rhythms by genre, meter, and 2/4/8-bar size; load into the percussion page; save that page separately; confirmed deletion of user saves only; list navigation. Empty Patterns may adopt the selected shape, while existing melody blocks resizing. |
| FT2 arrange | Select arrangement step; append/insert current pattern; duplicate/remove step; move step earlier/later; jump to referenced pattern; play from selected step; back and panic. |
| Pattern setup | Choose 3/4 or 4/4 and pattern size; confirm new/destructive resize, cancel, or clear while retaining the current size. |
| Tracks page manager | Select pages with the encoder; add a four-lane page; edit target, column, channel, bank, and program; confirm all changes; or exit and restore the original Project. |
| Target/channel field mode | Previous/next choice, confirm field, cancel field. Encoder turn/press and menu items share these operations. |
| Audio recorder | Select and name a track; assign an exact discovered JACK source; arm/disarm one, every resolved track, or all; refresh source discovery without rewriting preferences; start/stop one synchronized take; inspect elapsed time, active count, selected-track activity, drop/xrun/high-water status, final path or failure; Exit to Home and panic. |
| FX rack/editor | Choose source, AUX 1, AUX 2, or master; add/select/remove/bypass/reorder bounded effects; edit strict named physical-unit parameters; set independent send level, pre/post point, and return level; inspect peak/RMS/clip/non-finite/gain-reduction meters; and panic. Aux time effects are forced wet. An active graph publishes FX changes only with stopped transport and recording; a disabled graph accepts Project-only edits without touching audio. |
| MIDI setup | Read-only overview of the selected controller, external tracker route/profile, and controller clock. Hardware changes remain an explicit external `shr-setup` action. |
| Help | Compact Markdown user help, temporary LAN web help when port 80 is available, section links selected by the master encoder, keyboard page scrolling, top, and return to the previous screen. |
| Global/safety | Stop MIDI playback, tracker transport, recorder, managed engine, and owned notes; All Notes Off; cancel or leave the current controller level. Application exit remains computer-keyboard-only. Help is also reachable from `?` or F1. Process termination remains limited to the engine owned by SHR-DAW. |

The complete final screen × page × item mapping is maintained below. The table
uses expanded action names where that is clearer; the compact visible label is
shown in parentheses when it differs materially. `src/navigation.rs` is the
executable canonical copy: labels and dispatch actions are one definition. A unit test builds the
union of every normal and contextual menu and checks every action in this
  screen-specific inventory for controller reachability. Top-level Home entries
  are reached by the master rotary rather than duplicated on child command pages.

## Input model

- Eight buttons: four direct page selectors plus four item buttons.
- Five buttons: one page-cycle button plus four item buttons.
- Four buttons: four item buttons; encoder press enters/leaves page-selection
  mode and encoder turn changes pages while that mode is visible.
- Outside four-button page-selection mode, encoder turns retain list, row, and
  field adjustment. Encoder press retains the existing select/confirm action on
  eight- and five-button layouts. Menu slots do not duplicate those master
  rotary selection actions.
- Each screen remembers its last selected page. Entering/leaving a contextual
  mode resets that context to page 1, preventing stale hidden meanings.
- Page 1 holds the primary screen workflow; for FT2 normal mode it is the
  Page−/Page+/Track−/Track+ movement page. On every workspace, child screen,
  and contextual editor, `EXIT` is page 4/item 4 and returns exactly one level.
  Home is the root and has no MIDI Exit; quitting remains keyboard-only.
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
| MTR | Nav | FX | — | — | — |
| MTR | Sys | Panic | — | Help | Exit |
| Playback | Play | — | Play take | Record MIDI | — |
| Playback | Sound | Reset controls | Save | — | — |
| Playback | Sys | Panic | Help | — | Exit |
| FX rack | Ops | Add | Delete | Edit type | Parameters |
| FX rack | Order | Up | Down | Bypass | — |
| FX rack | Route | Target | Send− | Send+ | Point |
| FX rack | Sys | Panic | Return | Help | Exit |
| FX editor | Ops | Parameter− | Parameter+ | Value− | Value+ |
| FX editor | State | Bypass | — | — | — |
| FX editor | Nav | Rack | — | — | — |
| FX editor | Sys | Panic | — | Help | Exit |
| Ideas | Play | Inspect | Play | Record | Delete |
| Ideas | File | Load | Save | First | Last |
| Ideas | Sys | Panic | — | Help | Exit |
| Help | Ops | Open link | Top | — | — |
| Help | Sys | Panic | — | — | Exit |
| FT2 | Move | Page− | Page+ | Track− | Track+ |
| FT2 | Play | Cell edit | Play | Record | Step edit |
| FT2 | Open | Tracks | Files | Tools | Tap tempo |
| FT2 | Sys | Panic | — | Help | Exit |
| FT2 tools | Ops | Arrange | Loop | N00B | Mute lane |
| FT2 tools | Clip | Copy lane (`COPY L`) | Paste lane (`PASTE L`) | Copy page (`COPY PG`) | Paste page (`PSTE PG`) |
| FT2 tools | Page | Mute page (`MUTE PG`) | — | — | — |
| FT2 tools | Sys | Panic | Help | — | Exit |
| FT2 N00B | Move | Page− | Page+ | Track− | Track+ |
| FT2 N00B | Edit | Length | Delete | N-Off | Normal |
| FT2 N00B | Play | Play | Save | Files | — |
| FT2 N00B | Sys | Panic | — | Help | Exit |
| N00B length | Ops | Done | Cancel | — | — |
| N00B length | Sys | Panic | Help | — | Exit |
| FT2 loop | Play | Rewind | Play | Import | Remove |
| FT2 loop | BPM | BPM− | BPM+ | BPM x | Unit |
| FT2 loop | Cut | Start− | Start+ | Length− | Length+ |
| FT2 loop | Sys | Panic | Align | Library | Exit |
| Loop library | Ops | Delete WAV | — | — | — |
| Loop library | Sys | Panic | Help | — | Exit |
| FT2 loop align | Ops | Auto | Bar− | Bar+ | Done |
| FT2 loop align | Sys | Panic | Help | — | Exit |
| FT2 record | Play | — | Play | Record/stop | — |
| FT2 record | Sys | Panic | Help | — | Exit |
| FT2 step edit | Ops | Blank/skip | Erase | N-off | Done |
| FT2 step edit | Move | Arrangement step− (`PG-`) | Arrangement step+ (`PG+`) | Lane− | Lane+ |
| FT2 step edit | Add | 1 row | 2 rows | 4 rows | 8 rows |
| FT2 step edit | Sys | Panic | — | Next page (`PAGE`) | Exit edit |
| FT2 cell edit | Route | Destination | Channel | Instrument | — |
| FT2 cell edit | Sound | Bank MSB | Bank LSB | Cell program | Clear field |
| FT2 cell edit | Cell | Note | Gate | Velocity | Effect |
| FT2 cell edit | Done | Panic | Save | Effect parameter | Exit/cancel |
| Files | Ops | Load | Save | Preview/stop | Delete |
| Files | Project | New Project | Save As | Name/rename | Pattern tools |
| Files | Sys | Panic | — | Help | Exit |
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
| MIDI setup | Sys | Panic | Help | — | Exit |

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
