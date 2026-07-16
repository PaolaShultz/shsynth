# Controller action inventory and paging design

This document is the implementation checklist used for the paged controller
interface. The authoritative inventory was taken from `ui.rs` keyboard, mouse,
encoder, command-pad, screen, and contextual dispatch paths before paging was
implemented.

## Pre-implementation action inventory

| Screen or mode | Existing user-facing operations and input paths |
|---|---|
| Presets | Select previous/next, page up/down, first/last (keyboard, wheel, encoder); previous/next engine (keyboard/pads); load selected sound (keyboard, mouse, encoder/pad); tracker, ideas, and audio screens (keyboard/pads); stop synth/panic. Application exit remains keyboard-only. |
| Playback | Reset the 12 mapped parameters in place (encoder press); record/stop/finish-and-save MIDI, play/stop take, save idea (keyboard/pads/mouse); presets/back, ideas, tracker, audio (keyboard/pads/mouse); tap tempo; stop/panic. The 12 configured synthv1 CC controls continuously adjust parameters with pickup. |
| Ideas | Previous/next/first/last idea (keyboard, wheel, encoder); inspect (keyboard/mouse/pad); load with replace confirmation (encoder); play take; delete with repeat confirmation; record/stop MIDI; save timestamped or numbered idea; back/cancel, tracker, audio, presets, panic. |
| FT2 normal | Previous/next row (keyboard/encoder); order/lane movement; play here/from start; prominent Play/Rec/Edit/N00B MODE page; child Tools screen for pages, files, loop, mute, and page switching. |
| FT2 record | Record quantized notes into only the current page/current pattern; route live notes only to that page's hardware MIDI target; stop record, stop, exit, and panic remain available. |
| FT2 edit | All cursor and transport operations; musical keyboard or incoming MIDI note/chord gesture entry; blank/skip; erase; note off; leave edit; lane mute; program and tempo adjustment. Command notes are consumed for editing and never doubled through the synth. |
| FT2 N00B | Choose chromatic root plus major/natural minor; map live notes to the nearest scale tone with downward ties; preserve exact note ownership across releases and mode changes. |
| FT2 loop | Select/import WAV; confirmed Project detach without deleting the private WAV; play here/from start/stop; source BPM and half/normal/double interpretation; start/length cuts in beat or bar units; align child screen for auto bar alignment and one-bar placement shifts. |
| FT2 cell edit | Transactional note, gate, velocity, per-note program, single command type/parameter, clear-field, confirm/cancel, step-entry handoff, stop, and panic actions. Four-button encoder page selection remains available. |
| Tracker files | Select saved Project; load; preview/stop; save with overwrite confirmation; create a confirmed blank Project; save a numbered non-overwriting copy; delete with repeat confirmation; new, clone, copy, paste-new, paste-over, clear, or resize FT2 patterns; back/cancel and panic. |
| FT2 arrange | Select arrangement step; append/insert current pattern; duplicate/remove step; move step earlier/later; jump to referenced pattern; play from selected step; back and panic. |
| Pattern setup | Choose 3/4 or 4/4 and pattern size; confirm new/destructive resize, cancel, or clear while retaining the current size. |
| Page/track manager | Select previous/next page; add four-lane page; edit target; edit channel; confirm all changes; cancel and restore the original song; open files; mute current page. |
| Target/channel field mode | Previous/next choice, confirm field, cancel field. Encoder turn/press and menu items share these operations. |
| Audio recorder | Start/toggle recording, stop/finalize, inspect status, back, open presets/ideas/FT2, and panic. |
| Help | Compact Markdown user help, temporary LAN web help when port 80 is available, section links selected by the master encoder, page scrolling, top, and return to the previous screen. |
| Global/safety | Stop MIDI playback, tracker transport, recorder, managed engine, and owned notes; All Notes Off; cancel or leave the current controller level. Application exit remains computer-keyboard-only. Help is also reachable from `?` or F1. Process termination remains limited to the engine owned by SHR-DAW. |

The complete final screen × page × item mapping is maintained below.
`src/navigation.rs` is the executable canonical copy: labels and dispatch
actions are one definition. A unit test builds the
union of every normal and contextual menu and checks every action in this
inventory for controller reachability.

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
- Page 1 is always `OPS`. On every child screen and contextual editor, `EXIT`
  is page 4/item 4 and returns exactly one level. Presets is the root and has
  no MIDI Exit; quitting the application remains keyboard-only.
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
- The rendered controller strip is centered and capped at 40 columns. Labels
  and brackets use their natural width instead of expanding with the terminal.

## Complete controller map

Blank physical positions and wholly empty pages are omitted.

| Screen/context | Page | Item 1 | Item 2 | Item 3 | Item 4 |
|---|---|---|---|---|---|
| Presets | Ops | Load | Page up | Page down | First |
| Presets | Engine | Engine− | Engine+ | — | Last |
| Presets | Nav | — | Ideas | FT2 | Audio |
| Presets | Sys | Panic | Help | — | — |
| Playback | Ops | Record MIDI | Rec end | Take | Save |
| Playback | Sound | Reset controls | Finish + save | Tap tempo | — |
| Playback | Nav | Presets | Ideas | FT2 | Audio |
| Playback | Sys | Panic | Stop take | Help | Exit |
| Ideas | Ops | Inspect | Load | Play | Delete |
| Ideas | Capture | Record | Rec end | Save | First |
| Ideas | Nav | Presets | Help | FT2 | Audio |
| Ideas | Sys | Panic | Stop take | Last | Exit |
| Help | Ops | Open link | Page up | Page down | Top |
| Help | Sys | Panic | — | — | Exit |
| FT2 | Ops | Play here | Play from start | Step edit | Cell edit |
| FT2 | Mode | Play | Record | Edit | N00B |
| FT2 | Move | Order− | Order+ | Lane− | Lane+ |
| FT2 | Sys | Panic | Stop | Tools | Exit |
| FT2 tools | Ops | Pages/tracks | Files | Arrange | Mute lane |
| FT2 tools | Clip | Copy lane | Paste lane | Copy page | Paste page |
| FT2 tools | Loop | Loop | Remove | — | — |
| FT2 tools | Sys | Panic | Stop | Help | Exit |
| N00B setup | Ops | Root− | Root+ | Scale | Done |
| N00B setup | Sys | Panic | Stop | Help | Exit |
| FT2 loop | Ops | Import | Play here | Start | Stop |
| FT2 loop | BPM | BPM− | BPM+ | BPM x | Unit |
| FT2 loop | Cut | Start− | Start+ | Length− | Length+ |
| FT2 loop | Sys | Panic | Stop | Align | Exit |
| Loop library | Ops | Delete WAV | Page up | Page down | — |
| Loop library | Sys | Panic | Stop | Help | Exit |
| FT2 loop align | Ops | Auto | Bar− | Bar+ | Done |
| FT2 loop align | Sys | Panic | Stop | Help | Exit |
| FT2 record | Ops | Rec end | — | — | — |
| FT2 record | Sys | Panic | Stop | Help | Exit |
| FT2 step edit | Ops | Blank/skip | Erase | N-off | Done |
| FT2 step edit | Move | Order− | Order+ | Lane− | Lane+ |
| FT2 step edit | Adjust | Program− | Program+ | Tempo− | Tempo+ |
| FT2 step edit | Sys | Panic | Stop | Next page | Exit edit |
| FT2 cell edit | Ops | Confirm | Step edit | Clear field | Effect type |
| FT2 cell edit | Fields | Note | Gate | Vel | Program |
| FT2 cell edit | Adjust | Effect parameter | Value− | Value+ | — |
| FT2 cell edit | Sys | Panic | Stop | — | Exit/cancel |
| Files | Ops | Load | Save | Preview/stop | Delete |
| Files | Pattern | New | Clone | New Project | Save As |
| Files | Edit | Paste over | Clear | Clean unused | Name/rename |
| Files | Sys | Panic | Stop | Help | Exit |
| Arrange | Ops | Play | Jump | Append | Insert |
| Arrange | Step | Up | Down | Repeat | Remove |
| Arrange | Sys | Panic | Stop | Help | Exit |
| Pattern setup | Ops | 3/4 | 4/4 | Size− | Size+ |
| Pattern setup | Apply | Confirm | Keep | — | — |
| Pattern setup | Sys | Panic | — | Help | Exit/cancel |
| Pages/tracks | Ops | Add four lanes | Target | Channel | Done |
| Pages/tracks | Column | Column− | Column+ | Program− | Program+ |
| Pages/tracks | Bank | MSB− | MSB+ | LSB− | LSB+ |
| Pages/tracks | Sys | Panic | Stop | Help | Exit/cancel |
| Target/channel editor | Ops | Confirm | — | — | — |
| Target/channel editor | Sys | Panic | Stop | Help | Exit/cancel |
| Audio recorder | Ops | Record/toggle | — | — | — |
| Audio recorder | Nav | Presets | Ideas | FT2 | — |
| Audio recorder | Sys | Panic | Stop/finalize | Help | Exit |

## FT2 cell editor inventory and mapping

A cell contains `note`, optional `velocity`, optional per-note `program`,
optional `gate`, and one `command`: none, cut, delay, retrigger, or tempo. Song
format stores all of these fields directly inside each FT2 Pattern.

| Page | Item 1 | Item 2 | Item 3 | Item 4 |
|---|---|---|---|---|
| Ops | Confirm | Step entry | Clear selected field | Effect type |
| Fields | Note | Gate | Vel | Program |
| Adjust | Effect parameter | Value− | Value+ | — |
| Sys | Panic | Stop | — | Exit/cancel |

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
