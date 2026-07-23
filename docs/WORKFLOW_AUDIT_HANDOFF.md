# Workflow audit and repair handoff

Date: 2026-07-23

Status: review complete; implementation not started

This is the durable handoff and repair ledger for the first complete
musician/operator workflow audit. It contains the analysis, the evidence gaps,
the safe first repair queue, the questions reserved for the next decision pass,
and the verification required before any item can be marked complete.

Do not delete a finding when it is repaired. Change its status, record the
scoped commit and verification evidence, and leave any remaining question or
physical gate visible. This file is a work ledger, not a replacement for the
focused documents that define current behavior.

## Rules for the next worker

1. Read `AGENTS.md`, `docs/WORKSPACE_HANDOFF.md`, and this file completely.
2. Inspect live Git state before editing and preserve other workers' changes.
3. Work only on `READY` items during the first repair pass. Do not choose an
   answer for a `DECISION` item merely to make implementation convenient.
4. Before changing an installer, setup helper, diagnostic helper, or related
   Make target, read `docs/MAINTAINER_HELPERS.md` completely.
5. Keep each repair scoped. Update its ledger entry after source inspection,
   after implementation, and after verification.
6. The temporary combined-pass gate in `AGENTS.md` remains active. Until the
   user explicitly authorizes that pass, use formatting, source inspection,
   focused static review, and `git diff --check` only. Do not compile.
7. Do not start JACK, a synth, MIDI transmission/playback, recording, or any
   audible or hardware-changing test without fresh explicit permission.
8. Do not inspect or modify private material below `user/`.
9. These are repairs to existing promised behavior. A completed independently
   stageable repair should follow the repository repair publication contract:
   scoped commit and push, unless the user says otherwise or publication is
   blocked. Never stage a `user/` path.

## Status vocabulary

- `READY` — explicit source/product contract fixes the intended behavior; no
  product decision is needed.
- `IN PROGRESS` — implementation has begun; name the owning paths below.
- `SOURCE DONE` — source/docs/tests are updated, but the authorized build/test
  or screenshot pass has not happened.
- `VERIFIED` — proportionate permitted checks passed and evidence is recorded.
- `DONE` — repair is verified, documented, committed, and pushed when required.
- `DECISION` — user choice would materially change the behavior.
- `PHYSICAL` — source work may be complete, but physical/user evidence is
  required before acceptance.
- `BLOCKED` — an external prerequisite prevents progress; record it precisely.

## Evidence vocabulary

- `provided/reported` — user instructions, repository contracts, or historical
  handoff claims.
- `observed` — direct inspection of documentation, source, configuration,
  scripts, tests, or rendered screenshots.
- `inference` — likely consequence derived from observed artifacts.
- `open question` — physical, user, or operational evidence is required.

No physical testing or user validation was performed during this audit.
Focused tests were inspected as source but not run. The workspace handoff
reports an earlier repository-only pass of 651 passing tests and four
intentionally ignored tests; that is historical reported evidence, not fresh
validation.

## Executive summary

SHR-DAW has strong local safety foundations:

- exact routing and retained offline choices;
- one managed synth with clean note ownership and shutdown;
- bounded real-time audio callbacks;
- atomic per-file configuration writes;
- conservative interrupted-recording recovery;
- controller commands that cannot quit the application;
- explicit public/private data boundaries.

The largest risks occur at workflow boundaries:

1. unsaved Project edits can be lost through LOAD or keyboard quit;
2. Tracks is described as a draft but can mutate the Project and live route;
3. recording or transport can continue on Home without authoritative transport
   rendering;
4. Routing can collapse several performance inputs to one;
5. some FT2 order changes reset the row despite an explicit preservation rule;
6. install/setup actions are individually careful but not interruption-safe as
   one journey.

Secondary issues concern automatic LAN help, undifferentiated `doctor`
readiness, controller-visible naming that needs keyboard text entry, unclear
Project ownership in Effects, and Loop Library browsing that stops transport
before a selection is committed.

### Priority ranking

| Rank | Finding | Consequence | Frequency | Recovery cost | Ledger |
|---|---|---:|---:|---:|---|
| P0-1 | Unsaved Project can be replaced or quit without warning | High | Medium-high | High | R01, D01 |
| P1-2 | Tracks draft has live and partially committed effects | High | Medium | High | R03, D03 |
| P1-3 | Active recording/transport can become non-authoritative on Home | High | Medium | High | R02, D02, D09 |
| P1-4 | Routing edit deletes additional performance inputs | High | Low-medium | High | R04 |
| P1-5 | Order navigation loses FT2 row context | Medium | High | Medium | R05 |
| P1-6 | Install/setup is not journey-transactional | High | Low | High | R06, D04 |
| P2-7 | Help automatically opens a LAN listener | Medium | High | Low-medium | D05, D10 |
| P2-8 | `doctor` reports optional JACK absence as whole-check failure | Medium | High on first run | Low | R07, D06 |
| P2-9 | Controller-visible NAME workflows require keyboard text entry | Medium | Medium | Low | R08, D07 |
| P2-10 | Loop Library stops transport before selection | Medium | Medium | Low | R09 |
| P2-11 | Effects ownership is easy to misread | Medium | Medium | Medium | R10 |

## Constraint register

| ID | Necessary constraint | Source and confidence |
|---|---|---|
| C1 | Preserve private data and never silently lose or publish user work. | Explicit privacy/data-integrity requirement in `AGENTS.md` and recording contracts. High. |
| C2 | Own only one managed synth; preserve unowned processes/routes; send clean note releases. | Explicit ownership/safety requirement in `AGENTS.md`. High. |
| C3 | Use exact configured targets; retain missing choices; do not silently substitute MIDI destinations. | Interoperability and ownership requirement in `docs/CONFIGURATION.md`. High. |
| C4 | Keep live-audio work bounded; finalize recordings honestly and recover interrupted work. | Real-time/data-integrity requirement in `docs/MULTITRACK_RECORDING.md`. High. |
| C5 | Do not start or alter JACK, synths, MIDI, audio, or hardware without explicit intent. | Safety/authorization requirement in `AGENTS.md`. High. |
| C6 | Preserve 40×13 layout, one working-screen status row, overlay reveal, and cursor context during page/order changes. | Explicit product/accessibility requirement in `AGENTS.md`. High. |
| C7 | Support controller and keyboard operation; MIDI may Back/cancel but never quit the application. | Accessibility/authorization contract in `docs/CONTROLLER_INTERFACE.md`. High. |
| C8 | Keep hardware/configuration data-driven and support simultaneous configured inputs. | Portability/interoperability requirement in `docs/CONFIGURATION.md`. High. |
| C9 | Home remains visually distinct and has no shared working-screen footer. | Explicit product requirement in `AGENTS.md` and `docs/HELP.md`. High. |

## Workflow coverage inventory

“Covered” means artifacts were traced. It does not mean the workflow was
physically or aurally validated.

| Workflow and intended outcome | Entry, decisions, feedback, completion | Cancellation, failure, interruption, return, repeated use | Evidence and gap |
|---|---|---|---|
| Install, upgrade, uninstall | Installer chooses dependency/config phases, builds, installs, and enters setup | Failure leaves earlier package/service/install changes; uninstall preserves private data | `observed`: scripts, Makefile, install docs. Clean Pi path not run |
| Setup and configuration | Operator discovers or enters exact display, MIDI, JACK, capture, download, and tuning values | Backups exist; Ctrl-C or late validation can leave partial setup | `observed`: setup script/helper contract. No interactive hardware run |
| Local checkout launch | `setup-local.sh` seeds isolated state; `local.sh` verifies config and this checkout's debug binary | Missing binary/config fails nearby; repeated seeding preserves existing files | `observed`: scripts/docs. C1, C8 |
| Startup, splash, Home | Config/controller discovery, splash, Home recommendation, workspace selection | Missing controller recommends Learn; engines/audio remain lazy | `observed`: source and generated screenshots. Physical 40×13 pending |
| Shutdown, panic, signal | Keyboard quit or process stop invokes global owned cleanup | Stops recordings/transports, releases notes, drops owned engine | `observed`: source. Power/signal interruption not exercised |
| Global navigation and Back | Controller pages, encoder, keyboard, overlays, and nested editors | Most draft paths restore; order navigation and Tracks have exceptions | `observed`: navigation/UI/tests/screenshots. C6, C7 |
| Status and transport | Working screens compute one global transport cell | Home omits it; FT2 body duplicates transport | `observed`: render source/screenshots. C6, C9 |
| Help | Local help preserves caller and automatically attempts LAN hosting | Leaving Help stops server; opening has network side effects | `observed`: UI/help source. No LAN/accessibility user test |
| MIDI Learn and controller setup | Auto-profile or learn, backup, mapping, Home recommendation | Learn isolates MIDI; incomplete mapping remains recoverable | `observed`: source/docs/config. Current controllers not tested |
| Routing | Browse, detached field edit, full validation/save, live input activation or next-start status | Field cancel and rollback are strong; performance-input list is not | `observed`: source/config/tests/docs. C2, C3, C8 |
| Presets, Playback, synth ownership | Engine browse/load, musical MIDI, mapped controls, pickup, N00B, reset, Ideas/FX | Replacement and panic clean ownership | `observed`: source/docs/tests/screenshots. No audible/pickup hardware test |
| Ideas and take playback | Record, play, save, inspect, load, delete with instrument identity | Mode exclusivity and note cleanup exist; activity may outlive screen | `observed`: source/tests/docs. C1, C2 |
| FT2 entry and navigation | New default may adopt Player route; saved/changed Projects retain ownership | Page navigation preserves row; order navigation does not | `observed`: source/tests/docs. C2, C3, C6 |
| FT2 Edit, Cell, REC, Play | Mutually exclusive modes, quantized REC, release notes, Cell draft | REC Back ends capture safely; exact failures refuse substitution | `observed`: source/tests/docs. Musical feel and latency untested |
| Projects, Files, demos, defaults | Name, save, preview, load, delete, seed demos, save defaults | Save/no-replace is strong; LOAD/quit lack dirty protection | `observed`: source/docs/screenshots. C1 |
| Patterns, Arrangement, clipboard, drums | New, clone, clear, repeat, insert, move, jump, play | Destructive actions usually confirm; some order changes lose row | `observed`: source/tests/docs/screenshots |
| Tracks and page routing | Add/select/edit target/channel/program/bank, DONE validates | Whole EXIT restores; field cancel/live route do not match draft promise | `observed`: source/docs/tests. C2, C3 |
| WAV loops, import, library, alignment | Import/attach, tempo, cuts, alignment, playback, detach | Detach keeps WAV; opening browser stops transport | `observed`: source/docs/screenshots. Audio/sample-rate untested |
| FX, AUX, master | Caller opens rack, selects target/effect/parameter, publishes safely | Parameter cancel restores; owner unclear from Home/Playback | `observed`: source/docs/screenshots. C1, C4 |
| Mixer, final bus, meter, final record | Levels/mutes, limiter/meter, final stereo recorder | Edit blockers protect active state; Home visibility issue remains | `observed`: source/docs/tests/screenshots. JACK/aural evidence absent |
| Raw multitrack recorder | Assign/name/arm exact sources, record, monitor, finalize | Missing source blocks start; interrupted takes recover conservatively | `observed`: source/docs/tests/screenshots. No source-loss/xrun/disk test |
| Diagnosis and recovery | `doctor`, status/log, setup rerun, recording recovery, fallback reports | Full `doctor` fails without optional JACK; setup rerun starts from partial state | `observed`: CLI/source/docs. Real failure drills not run |
| Operation-affecting maintainer helpers | Screenshot/demo validation and non-audible recording stress | Written boundaries are strong; output remains synthetic evidence | `observed`: helper docs. No helpers executed |

The repository has 105 generated PNG frames, two JPGs, and the HTML tour.
Representative Home, FT2, overlay, Files, Playback, Routing, FX, Recorder, and
Meter frames were visually inspected and reconciled with their source fixtures.
This was not an exhaustive pixel-by-pixel review of every PNG.

## First repair queue

These items are scoped to the smallest behavior already fixed by an explicit
contract. Where a larger product question exists, this queue deliberately
repairs only the unambiguous part.

| ID | Status | First-pass repair | Do not decide yet |
|---|---|---|---|
| R01 | READY | Add Project dirty tracking and protect existing LOAD and current keyboard-quit paths with Save/Discard/Cancel; preserve the current reach of `q` | Whether `q` should remain global or become Home-only |
| R02 | READY | Make Home's existing bottom line compute and show authoritative active transport/recording ownership while preserving Home's special layout | Which background activities should be permitted |
| R03 | READY | Restore the exact nested Tracks field value on field EXIT | Whether the whole Tracks draft may live-audition routes |
| R04 | READY | Preserve every repeated performance input when Routing edits one input; use a list-aware draft | No single-input simplification |
| R05 | READY | Preserve/clamp FT2 row, page, lane, and column across Pattern/Song overlay and keyboard order changes | Explicit REWIND/START semantics |
| R06 | READY | Put installer consequences before the FluidSynth mask and add exact interruption/completion reporting to setup | Whether install/setup should become separate commands or a fully collected draft |
| R07 | READY | Add capability-group summaries to `doctor` without changing its current strict exit status | Default exit semantics and optional profiles |
| R08 | READY | Label Project/track text naming as keyboard-required in the present UI/docs | Whether to build controller character entry |
| R09 | READY | Delay Loop Library transport stop until an actual import/attach choice, after verifying no safety dependency requires early stop | Any broader live-preview feature |
| R10 | READY | Show current Project ownership and saved/dirty state in FX rack/editor within 40×13 | Per-preset or non-Project FX |
| R11 | READY | Reconcile embedded Help with current software-instrument FT2 REC behavior | Any change to REC ownership/routing |
| R12 | READY | Reconcile `HOW_IT_WORKS.md` with the implemented/tested Pattern length choices | Groove/microtiming or new length features |
| R13 | READY | Remove duplicate FT2 transport state from the header and update the focused test/screenshot expectation | Redesign of the shared status renderer |
| R14 | READY | Make routing-default persistence and Project save one coherent successful result, or leave defaults unchanged on failed/pending Project save | New default-routing features |
| R15 | READY | Prove the legacy Loop Library delete path unreachable, then remove stale dispatch/state/tests without adding deletion to the overlay | Any loop-file deletion feature |

### R01 — Unsaved Project protection

**Priority:** P0. Consequence high; frequency medium-high; recovery high.

**Intended outcome:** LOAD and keyboard quit must not discard uncommitted
Pattern, route, loop, name, or FX changes.

**Observed sequence:** NEW PROJECT asks twice and SAVE protects replacement.
`load_song()` stops transport and replaces `self.song` immediately.
`key()` returns quit for `q` on every non-text-modal screen. Global cleanup is
safe for audio/MIDI but does not save the Project.

**Evidence:** `observed` in `src/ui.rs` around `load_song`, `save_song_file`,
`new_project`, global key dispatch, and app-loop shutdown.

**Constraints:** C1 and C7, high confidence.

**Smallest repair:** Introduce one authoritative dirty baseline covering all
Project-owned data. Reuse one Save/Discard/Cancel decision for LOAD and the
currently reachable keyboard quit action. Keep `q` reachable exactly where it
is for this pass; tomorrow's decision may narrow it.

**Must remain unchanged:** no silent autosave; no controller quit; safe engine/
recording cleanup; atomic/no-replace save; private storage.

**Tracking:**

- [ ] Inventory every Project mutation that must mark dirty
- [ ] Define clean baseline after New, Load, successful Save, and Save As
- [ ] Add controller/keyboard-accessible LOAD decision
- [ ] Add the same protection to current keyboard quit
- [ ] Preserve screen/order/page/lane/row after Cancel or failed Save
- [ ] Update focused Project/menu/help documentation
- [ ] Add focused source tests
- [ ] Run formatting/source inspection and `git diff --check`
- [ ] Run authorized focused/build pass later
- [ ] Record commit/push and verification here

**History/evidence:** _none yet_

### R02 — Authoritative active state on Home

**Priority:** P1. Consequence high; frequency medium; recovery high.

**Intended outcome:** Returning Home must never hide whether raw, final, Idea,
tracker, loop, preview, or other owned transport is active.

**Observed sequence:** Back routes several workspaces to Home. The global
transport model already covers active recordings and transports. Home returns
before `draw_master_status` and uses the last free-form status string.
Recorder EXIT is explicitly documented not to alter recorder state.

**Constraints:** C4 and C9, high confidence.

**Smallest repair:** Keep Home without the shared working-screen footer, but
derive its existing bottom line from authoritative activity whenever any owned
activity is live. Name the activity and owning workspace. Do not add an
automatic stop or invent a new mixer/transport screen.

**Must remain unchanged:** Home composition, safe recorder continuation,
recording finalization, and MIDI inability to quit.

**Tracking:**

- [ ] Define deterministic priority when more than one activity is live
- [ ] Render exact state and owner in Home's existing bottom line
- [ ] Ensure later Home selection messages cannot hide active recording
- [ ] Preserve ordinary Home guidance when fully stopped
- [ ] Add focused 40×13 render/state tests
- [ ] Update Home/status documentation
- [ ] Run authorized screenshot regeneration later
- [ ] Record commit/push and verification here

**History/evidence:** _none yet_

### R03 — Tracks field cancellation

**Priority:** P1. Consequence high; frequency medium; recovery high.

**Intended outcome:** EXIT inside TARGET/ENGINE/INSTR/MIDI OUT/CHANNEL must
return to the unchanged Tracks draft.

**Observed sequence:** The initial target selector has detached selection, but
after its first confirmation the nested engine, instrument, or MIDI-output
turns mutate the current page. `cancel_page_field()` only changes mode.
Documentation promises restoration of the field.

**Constraints:** C2, C3, and C6, high confidence.

**Smallest repair:** Snapshot the complete field-owned route before entering
the nested editor and restore it on field EXIT. Do not decide whether the
larger Tracks session live-auditions routes.

**Must remain unchanged:** DONE validation, exact/offline route retention,
whole Tracks EXIT, note cleanup, and current menu reachability.

**Tracking:**

- [ ] Enumerate every nested field and its complete persisted value
- [ ] Restore target/engine/instrument/output/channel on field EXIT
- [ ] Preserve unrelated edits already made in the Tracks draft
- [ ] Add focused cancel-after-each-level tests
- [ ] Record whether any live route action remains after cancellation
- [ ] Run authorized focused/build pass later
- [ ] Record commit/push and verification here

**History/evidence:** _none yet_

### R04 — Repeated performance inputs

**Priority:** P1. Consequence high; frequency low-medium; recovery high.

**Intended outcome:** Editing one performance input must not delete other
simultaneous configured keyboards.

**Observed sequence:** configuration/setup support repeated
`midi.performance_input`. Routing reads the first value and replaces the whole
vector with zero or one selection.

**Constraints:** C3 and C8, high confidence.

**Smallest repair:** Make the Routing draft list-aware with explicit add/remove
of one exact input. Preserve unavailable retained inputs and reject duplicates.

**Must remain unchanged:** stable identities, detached Routing draft, backups,
atomic save, live rollback, no output probe/transmission.

**Tracking:**

- [ ] Define compact list/add/remove motion for 40×13 and 4/5/8-button layouts
- [ ] Preserve all unedited repeated inputs
- [ ] Cover unavailable and duplicate entries
- [ ] Cover Cancel and activation failure rollback
- [ ] Cover save/reload ordering
- [ ] Update Routing/config/controller documentation
- [ ] Run authorized focused/build/screenshot pass later
- [ ] Record commit/push and verification here

**History/evidence:** _none yet_

### R05 — FT2 order context

**Priority:** P1. Consequence medium; frequency high; recovery medium.

**Intended outcome:** Pattern/order navigation preserves row, page, lane, and
column unless the action explicitly means rewind/start/new.

**Observed sequence:** Page overlay navigation already preserves row and track.
Pattern and Song overlay selections plus keyboard PageUp/PageDown set row zero.
This conflicts with the explicit repository navigation contract.

**Constraints:** C6, high confidence.

**Smallest repair:** Retain the current row and clamp only when the destination
Pattern is shorter.

**Must remain unchanged:** REWIND, explicit play-from-start, new Pattern,
destructive replacement, and route/note cleanup.

**Tracking:**

- [ ] Pattern overlay preserves/clamps full cursor
- [ ] Song overlay preserves/clamps full cursor
- [ ] Keyboard PageUp/PageDown preserve/clamp full cursor
- [ ] Shorter and longer Pattern tests
- [ ] Active Play/REC and percussion/N00B source review
- [ ] Focused docs/test correction
- [ ] Run authorized focused/build pass later
- [ ] Record commit/push and verification here

**History/evidence:** _none yet_

### R06 — Install/setup consequence and interruption reporting

**Priority:** P1. Consequence high; frequency low; recovery high.

**Intended outcome:** Operators see meaningful system consequences before they
happen and receive exact recovery information after interruption.

**Observed sequence:** `install.sh` performs package changes and masks the user
FluidSynth service before printing its explanation. `setup.sh` has good unique
backups and atomic individual writes, but performs configuration, `.jackdrc`,
service, download, and tuning actions throughout a long sequence.

**Constraints:** C2, C5, and C8, high confidence.

**Smallest repair:** Before the first consequential action, print the exact
package/service/config phases. Track completed external steps and print exact
completed/not-completed/recovery information on failure or Ctrl-C. Do not
attempt unsafe blanket rollback.

**Must remain unchanged:** exact service scope; no JACK/synth start; backups;
private download boundary; manual JACK restart; tuning ownership records.

**Tracking:**

- [ ] Read `docs/MAINTAINER_HELPERS.md` before any edit
- [ ] Add installer preflight before package/service mutation
- [ ] Explain FluidSynth mask before, not after, the action
- [ ] Add setup phase/checkpoint reporting
- [ ] Add signal/error completion summary
- [ ] Document exact recovery commands without guessing previous state
- [ ] Source-inspect interruption at every phase
- [ ] Run permitted shell/static validation
- [ ] Record commit/push and verification here

**History/evidence:** _none yet_

### R07–R15 compact repair records

These remain individually scoped even when implemented in one working session.

| ID | Evidence and smallest repair | Must remain unchanged | Verification/status record |
|---|---|---|---|
| R07 | `observed`: `doctor` counts missing JACK as a whole-check failure although editor/external MIDI can work. Add grouped capability summaries while retaining strict exit for now. | Exact missing command/device/CPU/MIDI/JACK checks | Status READY; test absent/running JACK and strict exit; history none |
| R08 | `observed`: controller NAME opens a text modal, but only keyboard characters edit it. Label NAME/modal/docs as keyboard-required. | Generated safe defaults, validation, Cancel | Status READY; inspect 4/5/8 layouts and 40×13; history none |
| R09 | `observed`: opening Loop Library calls `tracker_stop()` before browse/cancel. Move stop to committed import/attach after confirming no verified safety dependency. | Private WAV ownership, no delete, exact load validation | Status READY; test playing cancel/current/new/missing loop; history none |
| R10 | `observed`: FX mutates current Project racks but UI mostly says SOURCE/AUX/MASTER. Show Project name and dirty state. | Project ownership, caller return, edit blockers, topology | Status READY; render Home/Playback/FT2/Meter callers at 40×13; history none |
| R11 | `observed`: embedded Help says FT2 REC is hardware-only; source/tests/Tracker docs accept exact software routes. Correct Help. | Exact target, one engine, no fallback | Status READY; source/doc consistency review; history none |
| R12 | `observed`: architecture doc says arbitrary Pattern lengths are planned; current menu/source/tests expose 1–32 plus 48/64/96/128/192/256. Correct current-behavior text only. | No groove/microtiming/new feature promise | Status READY; link/source consistency review; history none |
| R13 | `observed`: FT2 header and test require REC/PLAY/PAUSE although final row owns transport and Tracker docs forbid duplication. Remove duplicate state and update test/screenshot fixture. | Project/order/pattern identity; exact final transport cell | Status READY; source render now, screenshot regeneration after authorization; history none |
| R14 | `observed`: routing defaults save before Project save, so failed/pending Project save can still change defaults. Make the outcome transactional or defer defaults until Project success. | Explicit default confirmation and no-replace Project save | Status READY; test overwrite pending, I/O failure, Cancel, success; history none |
| R15 | `observed`: current Loop overlay has no delete; legacy menu/state/delete dispatch remains. Prove unreachable, then remove stale code/tests. | No loop deletion from browser; Project REMOVE keeps WAV | Status READY; navigation reachability/static test; history none |

## Decision queue for tomorrow

Do not answer these in the first repair pass.

| ID | Decision needed | Why it is not safe to assume |
|---|---|---|
| D01 | Should keyboard `q` remain global, become Home-only, or behave like Back until Home? | It changes expert keyboard navigation and quit reachability. R01 only protects the current path. |
| D02 | Which transports/recorders are intentionally allowed to continue after Home return? | Recorder explicitly continues; other workflows are not equally clear. R02 only makes activity visible. |
| D03 | Should the whole Tracks draft be completely detached until DONE, or have explicit live audition? | Documentation says draft; source currently synchronizes routes while browsing. R03 only fixes field Cancel. |
| D04 | Should install and setup remain one command, become explicit phases, or collect all ordinary config before commit? | Packaging, recovery, and operator expectations change materially. R06 only improves consequence/interruption reporting. |
| D05 | Should LAN Help be per-use SHARE, a persisted opt-in, or removed? Which interface/port? | Network exposure and accessibility trade off; current automatic `0.0.0.0:80` is not a necessary local-help constraint. |
| D06 | Should default `doctor` mean core/editor readiness or complete-audio readiness? | Exit-code consumers and first-run interpretation may differ. R07 preserves current strict exit. |
| D07 | Is controller-only custom text naming required? | A character editor is new interaction work; R08 only makes the keyboard dependency honest. |
| D08 | Is Help's documented one-step START desired, or is current REWIND then PLAY intended? | Either source or embedded Help must change, but the musical motion needs owner choice. |
| D09 | Should active Home state offer a direct reopen/stop action, or only authoritative information? | Adds navigation/transport behavior beyond R02's visibility repair. |
| D10 | Should LAN help ever use a privileged port? | Port 80 affects permissions, deployment, and exposure. |

### Deferred finding evidence

| Decision | Current sequence and evidence | Constraint and friction | Smallest later motion; preserve |
|---|---|---|---|
| D01 | `observed`: `q` exits from every non-text-modal screen, while menu documentation describes quitting from Home | C1/C7; high loss risk and keyboard-navigation ambiguity | Choose global protected quit, Home-only quit, or Back-until-Home; preserve MIDI-never-quits and clean shutdown |
| D02 | `observed`: raw recorder explicitly survives EXIT; Idea/final/loop/tracker transport paths are not documented with one consistent background policy | C4/C9; active work can cross a screen boundary without a single ownership rule | Decide permitted background owners; preserve finalization and R02's authoritative visibility |
| D03 | `observed`: Tracks stores an original Project but edits `self.song` and synchronizes routes while browsing/adding | C2/C3/C6; a documented draft can change live ownership | Choose detached-until-DONE or explicit audition; preserve exact routes, conflict validation, note cleanup, and R03 field Cancel |
| D04 | `observed`: install and setup combine packages, service masks, configuration, downloads, `.jackdrc`, and optional tuning | C2/C5/C8; interruption recovery spans several ownership domains | Choose one command with checkpoints, explicit phases, or collected config commit; preserve exact service/helper ownership and no JACK start |
| D05 | `observed`: opening local Help always calls the server starter, which route-discovers through `8.8.8.8` and binds `0.0.0.0:80`; leaving Help drops it | C5 plus phone accessibility; opening text is not clear consent to LAN publication | Choose per-use SHARE, persisted opt-in, or removal; preserve embedded offline Help, caller return, bounded server, and auto-stop |
| D06 | `observed`: `doctor` counts unreachable JACK as a problem even though docs support editor/external-MIDI use without JACK | C3/C5; valid partial readiness looks like whole-product failure | Choose core-ready or complete-audio default semantics; preserve R07 grouped truth and every exact diagnostic |
| D07 | `observed`: controller can open NAME and confirm/cancel, but only keyboard character/backspace events edit text | C7; visible controller action cannot complete custom naming | Choose honest keyboard boundary or bounded controller editor; preserve validation, generated defaults, and Cancel |
| D08 | `observed`: embedded Help promises one-step START; source exposes REWIND that stops/positions and asks for PLAY | C6 and literal navigation; source and musician instruction disagree | Choose START or REWIND→PLAY; preserve explicit stop/rewind note cleanup and selected-position PLAY |
| D09 | R02 can show exact activity without adding a new command, but a direct reopen/stop shortcut would alter Home navigation | C7/C9; visibility and control are separate requirements | Decide information-only versus direct owner action after R02; preserve Home layout and no controller quit |
| D10 | `observed`: Help uses privileged port 80 and all-interface binding, so access may fail or expose more interfaces than intended | C5 and platform limits; port choice changes deployment/security behavior | Choose nonprivileged port and intended interface together with D05; preserve phone accessibility and local Help |

Record tomorrow's answers here:

| Decision | Answer/date | Resulting scoped work |
|---|---|---|
| D01 | _pending_ | _pending_ |
| D02 | _pending_ | _pending_ |
| D03 | _pending_ | _pending_ |
| D04 | _pending_ | _pending_ |
| D05 | _pending_ | _pending_ |
| D06 | _pending_ | _pending_ |
| D07 | _pending_ | _pending_ |
| D08 | _pending_ | _pending_ |
| D09 | _pending_ | _pending_ |
| D10 | _pending_ | _pending_ |

## Physical and user-evidence queue

These cannot be marked accepted from source/tests/screenshots alone.

| ID | Required evidence |
|---|---|
| P01 | Physical 40×13 overlay reveal, launcher, status row, wrapping, Back, page-1 entry, and silent hidden launcher review |
| P02 | Four-, five-, and eight-button controller reachability for every confirmation/cancel path |
| P03 | Pickup re-arm after preset load, Project route change, reset, Idea load, and failed engine restart |
| P04 | What remains audible/routed when leaving FT2 for Home: hardware MIDI, software synth, and WAV loop separately |
| P05 | Raw/final recorder behavior under JACK loss, source loss, xrun, disk full, and power interruption |
| P06 | Clean Raspberry Pi OS Lite install/setup/doctor/launch/recovery journey |
| P07 | Pi 5/NVMe and later MR18 acceptance under their focused plans |
| P08 | Exhaustive visual review of all 105 generated PNG frames after repairs |

## Cross-artifact contradictions

| ID | Contradiction | Disposition |
|---|---|---|
| X01 | `docs/HELP.md` says FT2 REC refuses software instruments; source, focused tests, and `docs/TRACKER.md` accept an exact online software route | R11 READY |
| X02 | Help says START plays from Arrangement beginning; current UI exposes REWIND then PLAY | D08 DECISION |
| X03 | `docs/HOW_IT_WORKS.md` calls arbitrary interactive Pattern lengths planned; menu/source/tests expose the wider list | R12 READY |
| X04 | Menu docs say keyboard quit is from Home; source accepts `q` globally | R01 protects loss; D01 decides reach; docs wait |
| X05 | Tracker docs say header does not repeat transport; source/test/screenshot require PAUSE/PLAY/REC | R13 READY |
| X06 | Current loop overlay has no deletion, but legacy menu/state/delete code remains | R15 READY |

The screenshot count is not contradictory: 95 menu-manual images are a subset
of the 105 generated PNG frames.

## Isolated defects kept separate from workflow redesign

1. **Routing-default partial commit:** defaults are written before Project save
   succeeds or overwrite confirmation resolves. Tracked as R14.
2. **FT2 duplicate transport header:** source/test/screenshot conflict with the
   final-row contract. Tracked as R13.
3. **Legacy Loop Library deletion state:** stale/unreachable code should not be
   used to redesign the no-delete overlay. Tracked as R15.

## Recommended repair order

1. R01 — protect irreplaceable Project work.
2. R04 and R14 — prevent configuration/default data loss.
3. R03 — restore Tracks field cancellation.
4. R05 — preserve FT2 musical location.
5. R11, R12, and R13 — restore one truthful UI/documentation contract.
6. R02 — make background state authoritative on Home.
7. R09 and R10 — remove premature transport action and clarify ownership.
8. R06 and R07 — improve operator consequence/recovery/readiness reporting.
9. R08 — make current naming accessibility honest.
10. R15 — remove proven stale code.
11. Hold D01–D10 for the owner decision pass.
12. After explicit combined-pass authorization, run focused checks and required
    screenshot regeneration.
13. Obtain P01–P08 only under their separate safety/physical authorizations.

## Verification matrix

Every completed repair must name the applicable rows and record evidence in its
tracking section.

| Scenario | Project/files | Navigation/UI | MIDI/audio ownership | Operator/setup | Existing state that must remain untouched |
|---|---|---|---|---|---|
| Normal completion | Save, Save As, Load, demos, defaults, loop attach, FX persist | Every screen, editor, overlay, Back, caller return | One engine, exact target, pickup, finalization | Install, setup, doctor, launch | Other Projects, Ideas, WAVs, presets, configs, routes |
| Cancellation | Dirty quit/load Cancel; Tracks field/session; Routing/FX/loop | Exact order/page/lane/row/menu state restored | No new output, transmitted probe, or layered engine | Ctrl-C at every wizard phase | Existing services, `.jackdrc`, tuning, downloads |
| Failure and retry | Save failure, overwrite refusal, missing/corrupt Project, disk full | Error near action; draft retained | JACK loss, offline/ambiguous target, engine exit, source loss | sudo/network/package/learn/tune failure with recovery summary | Old config and runtime route restored |
| Repeated use | Load/edit/save/load; repeat import/remove without WAV deletion | Re-entry resets only intended menu state | Repeated preset/Idea/Project ownership transfers leave one engine | Setup rerun and upgrade preserve data | No duplicate seeds, routes, services, or downloads |
| Interruption/handoff | Process/power stop during recording and Project edit | Return after Help/Home/background completion | All Notes Off on owned shutdown; recognized takes recover honestly | Resume from known checkpoint | Unknown partial data reported, never silently deleted |
| Accessibility/parity | Save/load/cancel on keyboard and 4/5/8-button layouts | Encoder, pads, keyboard, mouse Back; 40×13 legibility | Commands consumed; musical MIDI forwarded; MIDI cannot quit | Offline controller recommends Learn without blocking keyboard | Existing learned mapping and physical position |
| Untouched-state regression | Preview/cancelled load retain current Project | Overlay reveal/status/Home layout preserved | Unowned synths/routes/JACK untouched; missing targets retained | Diagnostics remain read-only | Everything below `user/`, public allowlists, unrelated processes |
| Physical gate | User-selected disposable test Project | Real 40×13 TTY and controllers | Separately authorized monitored MIDI/audio/record tests | Pi 4 first; later hardware only when available | No audible/hardware change without explicit approval |

## Completion log

Append one row per coherent repair. Do not replace history.

| Date | IDs | Outcome | Source/static checks | Authorized build/test/screenshot evidence | Commit/push |
|---|---|---|---|---|---|
| 2026-07-23 | Audit | Review and repair ledger created; no implementation | Read-only artifact reconciliation; no compile | None; no physical/user validation | Not applicable |
