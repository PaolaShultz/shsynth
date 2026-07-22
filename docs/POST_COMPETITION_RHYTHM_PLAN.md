# Post-competition rhythm plan

This document is an execution plan for work **after the competition**. None of
the behavior below is part of the competition build, and the submission should
not be destabilized to begin it early. The work is deliberately split into
independently testable releases so SHR-DAW gains useful rhythm features before
it attempts a general notation or time-signature system.

## Product decision

The first goal is not formal support for every written meter. It is to let a
musician make a Pattern repeat at any chosen row, because that already produces
the useful musical result:

- at the default four rows per quarter note, 16 rows lasts 4/4;
- 15 rows lasts 15/16;
- 14 rows lasts 7/8; and
- 10 rows lasts 5/8.

The notes and accents decide whether 7/8 feels like `2+2+3`, `3+2+2`, or
another grouping. Pattern length, rhythmic grouping, swing, and early/late
playing are related but different features. They should not be forced into one
large implementation.

The planned order is:

1. arbitrary Pattern shortening and growing;
2. independent early/late timing on individual cells;
3. Pattern-wide swing that preserves the total phrase duration;
4. reusable groove tools and timing-aware real-time capture; and
5. optional formal time signatures only if later workflows truly need them.

Straight 3/4 and 4/4 operation must remain unchanged by default throughout.

## Existing foundation

The current model and scheduler are already suitable for the first phase:

- `Pattern.rows` is the actual duration and accepts 1–256 rows;
- Project files already store each Pattern's row count;
- the scheduler advances every row, including empty rows, and loops at the
  exact final boundary;
- Arrangement steps already reference Patterns of different lengths; and
- a cell already has a late-only `Delay(0..=15)` command.

The restrictions are above that foundation:

- Pattern setup exposes only fixed 3/4 and 4/4 sizes;
- Pattern and drum metadata validate only meters 3 and 4;
- grid highlighting infers beat divisions from a few recognized total sizes;
- the drum browser expands grooves only to its fixed target sizes;
- `Delay` occupies the cell's only command slot and cannot move a hit early;
- real-time tracker recording discards timing within the selected row; and
- WAV bar alignment interprets the Pattern's existing meter rather than an
  arbitrary phrase end.

These boundaries are reasons to stage the work, not reasons to rewrite the
sequencer before delivering arbitrary lengths.

## Part 1: transactional Pattern Length editor

### User workflow

Add **LENGTH** to Pattern Tools. Opening it copies the current Pattern into a
temporary draft. The musician may shorten and grow the draft repeatedly
without an interruption after every encoder movement.

The compact controller pages should follow the existing four-page contract:

| Page | Item 1 | Item 2 | Item 3 | Item 4 |
|---|---|---|---|---|
| Ops | Row− | Row+ | Beat− | Beat+ |
| Apply | Apply | Original | — | — |
| — | — | — | — | — |
| Sys | Panic | Stop | Help | Exit/cancel |

`Row−` and `Row+` change one tracker row. `Beat−` and `Beat+` change one
quarter-note unit, using the Project's configured rows per quarter note.
Computer keyboard and mouse actions should expose the same operations.

The screen should continuously show:

- original and proposed row counts;
- duration in quarter-note beats;
- a familiar equivalent such as `15/16` when the normal four-row grid makes
  that label exact;
- how many non-empty cells are beyond the proposed end;
- how many Arrangement steps reference this Pattern; and
- `APPLY keeps this length · EXIT restores the original`.

Growing appends completely empty rows across every page and lane. Shortening
removes only the tail; it never compresses or moves surviving cells.

### Drum-load confirmation policy

Loading a drum groove and quickly removing several tail rows is an expected
creative workflow. Requiring confirmation for every removed beat would make
the controller unpleasant to use.

The Length editor is therefore transactional for **all** Patterns:

- repeated shortening never opens a confirmation modal;
- populated tail rows may be removed from the draft immediately;
- the display reports, for example, `CUTS 6 HITS` in a warning color;
- **APPLY** commits immediately when only empty tail rows are removed;
- if **APPLY** would remove populated cells, one final prompt states the exact
  loss, such as `24 notes will be deleted. Continue?`;
- confirming that prompt commits the complete draft, while cancel returns to
  the unchanged Length draft rather than discarding the chosen length;
- **ORIGINAL** resets the draft without leaving the editor; and
- **EXIT** cancels the entire length-edit session.

This makes a separate persistent `suppress_trim_warnings` option unnecessary.
It also protects melodic work better than a global switch that users could
forget was enabled. The application may retain a runtime-only origin such as
`Manual` or `AfterDrumLoad` to show a more helpful hint, but that origin must
not be saved in `.shsong`, `.shdrum`, configuration, or shared data. It must
not silently change the commit rules.

After a drum load, the status line should explicitly suggest **LENGTH** as the
next action. Drum-loaded Patterns follow the same single final data-loss prompt;
they do not prompt separately for every row or beat removed. Applying a
shortened Pattern must not change its MIDI target, channels, programs, tempo,
lane state, or Arrangement references.

### Model and scheduling scope

Part 1 should keep the existing meter metadata. A 14-row Pattern may still
carry the 4/4 grid context while musically repeating after 7/8 of duration.
This is intentional: the feature is an irregular phrase end, not yet a formal
meter migration.

No Project-format bump should be necessary because Pattern row counts already
round-trip. Playback should require little or no scheduler change. The main
implementation should be UI draft state, safe resize helpers, cursor clamping,
and honest duration rendering.

Beat and row highlighting must stop guessing from total Pattern length. It
should derive regular quarter-note boundaries from `steps_per_beat` and mark
the final phrase boundary separately. A 15-row Pattern must not acquire false
beat spacing merely because it is not one of the old preset sizes.

### Interactions and limits

- Playback and tracker REC stop before the Pattern is committed.
- The editor works on the Pattern record, so every Arrangement reference to
  that Pattern sees the new length; the screen shows the reference count.
- Copy, clone, paste, save, load, and preview retain the exact arbitrary row
  count.
- Real-time REC loops at the new end and continues to quantize to rows.
- Loading a fixed-size library groove afterward may resize an otherwise empty
  melodic Pattern under the existing drum-load rules.
- User-saved odd-length drum pages need a later browser change before they can
  be conveniently filtered and expanded. Part 1 must not pretend the fixed
  drum library already understands formal odd meter.
- WAV loop `AUTO`, bar cuts, and bar offsets retain the current meter rules.
  The UI should warn that an attached WAV loop is aligned to the regular meter,
  not the shortened phrase. Correct odd-phrase WAV alignment belongs to Part 5
  unless a smaller design is proven first.

### Acceptance criteria

- Length can move through every value from 1 to 256 rows without a modal per
  step.
- A loaded drum groove can be shortened across populated hits without an
  interruption per step, then committed with one exact data-loss confirmation.
- Empty-tail shortening commits without a data-loss confirmation.
- Cancelling the final data-loss prompt preserves the draft and original
  Pattern so the musician can adjust the proposed length or exit safely.
- **EXIT** after arbitrary draft changes restores the Pattern byte-for-byte.
- **ORIGINAL** restores the draft and permits further editing.
- Applying an expansion adds only default cells.
- Applying a shrink removes exactly the reported tail cells.
- All page row widths remain consistent and Project cell limits still hold.
- Referenced Patterns change in place without rewriting Arrangement steps.
- Playback, final empty rows, loop restart, play-here, REC wrap, and cursor
  position remain correct at 1, 10, 14, 15, 16, 255, and 256 rows.
- Existing 3/4 and 4/4 setup, drum load, and Project round trips remain
  unchanged.

Estimated effort: **2–3 focused days**, including controller/UI tests,
documentation, and Raspberry Pi verification.

## Part 2: independent cell microtiming

### Product behavior

Add a timing field that is independent of the existing single command. The
cell editor should describe it musically:

- `ON GRID`;
- `EARLY 12 ms`; or
- `LATE 18 ms`.

An advanced detail view may also show the stored musical fraction. Users
should not need to understand PPQN, scheduler durations, or signed integers.
Resetting timing returns exactly to the row boundary.

The timing field must coexist with cut, delay, retrigger, tempo, velocity,
program, and gate. The old `Delay` command remains loadable for compatibility;
whether it is later deprecated should be a separate decision after real songs
have migrated.

### Proposed representation

Store a signed `nudge` in units of 1/96 of a row, initially bounded to half a
row early or late (`-48..=48`). Ninety-six divides cleanly by the common binary
and triplet subdivisions, scales musically with tempo, and gives substantially
finer control than the current 1/16-row Delay command. The UI should calculate
the approximate milliseconds at the Pattern's current tempo.

This exact representation must be confirmed with scheduler tests before the
format is frozen. Once published, its meaning may not change.

To keep the first implementation deterministic and bounded:

- timing may not move an event outside its Pattern;
- the first row cannot move earlier than Pattern start;
- the last row cannot move later than Pattern end;
- play-here clamps unavailable pre-roll rather than emitting an event before
  transport start; and
- cross-Pattern pickups remain an Arrangement/pickup-Pattern workflow until a
  later scheduler explicitly supports wrapped negative events.

### Storage and migration

Plan one rhythm-suite Project format bump. Old Projects load every cell with
zero nudge and straight feel. Unknown newer formats remain refused and are
never overwritten. `.shdrum` receives the same backward-compatible default so
grooves may carry deliberate timing.

Before publishing the new version, decide and include the Pattern groove
fields required by Part 3 with straight defaults. That avoids consecutive file
format migrations even if the swing UI lands in a later commit.

### Scheduler requirements

- Convert the signed musical offset to `Duration` only while scheduling.
- Sort simultaneous and shifted events deterministically.
- Preserve program-before-note ordering at an identical time.
- Base gate duration on the shifted note-on while retaining bounded note-off
  and ownership behavior.
- Define replacement of an already active note when a late event crosses
  another event in the same lane.
- Keep stop, mute, panic, unavailable-target, and shared-note cleanup exact.
- Keep the total Pattern duration unchanged.

### Acceptance criteria

- Early, on-grid, and late notes schedule at exact tested offsets over the
  entire 20–300 BPM range.
- Nudge and every existing command round-trip together.
- Old Projects and drum files load as zero-nudge without rewriting until save.
- No shifted message escapes its Pattern or produces a stuck note.
- Copy/paste, transpose, drum load/save, clone, preview, and Arrangement retain
  timing values.
- The 40×13 grid exposes a compact timing marker without making ordinary notes
  unreadable.

Estimated effort: **4–7 focused days**.

## Part 3: Pattern-wide swing

### Product behavior

Add a Pattern **FEEL** editor with a small beginner surface:

- `STRAIGHT` at 50%;
- eighth-note swing;
- sixteenth-note swing; and
- an amount moving from straight toward a bounded heavy swing.

The initial useful range should be conservative, approximately 50–75%, with a
clear triplet-feel landmark. The exact maximum should be approved by listening,
not selected only from arithmetic.

Swing changes alternating subdivision positions but never changes the total
quarter note, bar, Pattern, or Arrangement duration. Cell nudge is applied
after swing so an advanced user may further hurry or drag a selected snare.

### Transport boundary

The current transport derives MIDI clock and WAV-loop tempo from uniform row
timing in several places. A naive implementation that alternates row durations
could make external MIDI clock and the loop player's tempo wobble.

Part 3 must separate:

- the steady quarter-note transport and 24-PPQN MIDI clock;
- swung tracker row/event positions;
- the UI play cursor; and
- the WAV loop's continuous beat clock.

MIDI clock stays even. The WAV loop stays at the Pattern tempo. Swung events
return to the exact unswung boundary at the end of each affected pair and at
the final Pattern boundary. Tempo commands and live tempo changes preserve the
selected swing ratio when the remaining schedule is rebuilt or rescaled.

### Acceptance criteria

- Straight mode produces byte-for-byte-equivalent scheduled event times to the
  current behavior.
- Every swung pair has the configured ratio and the same combined duration as
  straight playback.
- Pattern and Arrangement boundaries never drift after long repetition.
- MIDI clock remains 24 evenly timed pulses per quarter note.
- WAV-loop position remains continuous and does not alternate tempo.
- Swing and per-cell nudge combine in the documented order.
- Play-here, tempo commands, live tempo changes, stop, and loop restart remain
  deterministic.

Estimated effort: **4–7 focused days**, including Pi and external-MIDI timing
checks.

## Part 4: groove tools and expressive capture

Part 4 makes Parts 2 and 3 fast to use rather than merely possible.

### Deterministic groove tools

Add a compact advanced **GROOVE** child screen that can apply saved timing and
velocity shapes to:

- the selected cell;
- the selected lane, such as snare only;
- the current percussion page; or
- the whole Pattern.

Useful neutral presets include:

- snare late;
- hats early;
- alternating push/pull;
- increasing drag toward the phrase end; and
- increasing push toward the phrase end.

Names should explain what will happen rather than claim cultural authenticity.
Application should be transactional with strength and affected-hit counts.
Grooves must be deterministic and saved as exact resulting values. Do not
randomize timing anew on every playback. If humanization is later added, it
needs a persisted seed, strict timing/velocity bounds, and an explicit reset.

The “psycho slowing” effect should normally move events around a steady phrase
clock. This keeps loops, MIDI clock, and the next Pattern aligned. Actual tempo
ramps are a different operation and may continue to use explicit tempo
commands or a later automation design.

### Timing-aware tracker REC

Current REC chooses the transport row. A later capture mode should also measure
the input's residual distance from the nearest row and store it as cell nudge.

- Quantized REC remains the beginner default.
- `REC FEEL` retains bounded early/late timing.
- Timing comes from the MIDI callback timestamp, not the screen refresh.
- Notes outside the nudge range choose the adjacent row rather than clamping to
  a misleading extreme.
- Existing exact channel/note/lane ownership and selected-page route isolation
  stay intact.

Estimated effort: **3–6 days** after microtiming and swing are stable, plus
human groove authoring and listening time.

## Part 5: optional formal meter and grouping

Do not block Parts 1–4 on this work. Implement it only when the product needs
meter-aware WAV alignment, formal display, metronome accents, or reusable odd
meter filtering rather than simply an irregular phrase length.

A complete design would replace the single `meter` numerator with a time
signature containing at least numerator and denominator, plus optional grouping
such as `2+2+3`. It would affect:

- Pattern and drum-file versions;
- Pattern setup and row-count presets;
- grid and group highlighting;
- drum catalog filters and phrase expansion;
- loop beat units, cuts, offsets, `AUTO` bar alignment, and song position;
- BPM labelling and tap-tempo interpretation; and
- documentation and migration tests.

BPM semantics must be decided explicitly. Keeping BPM tied to the quarter note
is the most compatible option, but odd-meter users may expect an eighth-note or
grouped pulse. The display must state the tempo unit rather than silently
changing old Projects.

Estimated effort: **5–10 days**. This is lower priority because a 14-row loop
already delivers the central musical result of 7/8.

## Execution sequence and release gates

| Work package | Dependency | Estimate | Release value |
|---|---|---:|---|
| 1. Pattern Length | Competition complete | 2–3 days | Immediate irregular phrases |
| 2. Cell microtiming | Part 1 stable | 4–7 days | Individual push and drag |
| 3. Swing | Part 2 timing model | 4–7 days | Coherent Pattern feel |
| 4. Groove tools/REC | Parts 2–3 stable | 3–6 days | Fast advanced workflow |
| 5. Formal meter | Demonstrated product need | 5–10 days | Meter-aware display/library/loops |

Parts 1–3 form the recommended polished rhythm release: approximately **two to
three focused engineering weeks**, followed by at least two human listening
and controller sessions. Part 4 may join that release if it does not weaken
the simpler workflow. Part 5 should remain separately selectable.

After the competition:

1. finish or integrate any already-active audio-graph work before modifying
   overlapping UI/runtime files;
2. record a clean baseline of existing scheduler, drum, Project, and UI tests;
3. implement and publish one work package at a time;
4. migrate formats only in the planned microtiming/swing package;
5. perform non-audible timing and ownership checks first; and
6. ask for explicit authorization before audible synth, JACK, external-MIDI,
   or groove-quality evaluation.

## Likely implementation areas

- `src/ui.rs`: transactional editors, actions, rendering, REC capture, and
  workflow integration.
- `src/navigation.rs`: four-page controller actions and the invariant EXIT
  location.
- `src/sequencer.rs`: resize helpers, cell/pattern fields, version migration,
  event timing, MIDI clock, ownership, and schedule tests.
- `src/drum_pattern.rs`: timed groove storage, odd-length discovery, and
  deterministic groove data.
- `src/loop_player.rs`: only when swing transport separation or formal meter
  requires beat-clock and alignment changes.
- `docs/TRACKER.md`, `docs/CONTROLLER_INTERFACE.md`, `docs/CONFIGURATION.md`,
  `docs/HELP.md`, and `README.md`: update only when each behavior ships.

## Validation and handoff

Every behavior or format package requires the repository's Rust handoff suite
with the installed Rust 1.85 toolchain:

```sh
export PATH=/home/patch/.rustup/toolchains/1.85.0-aarch64-unknown-linux-gnu/bin:$PATH
cargo fmt -- --check
cargo test --locked
cargo clippy --locked -- -D warnings
cargo build --release --locked
```

Also run targeted tests for:

- exact event timestamps and Pattern end boundaries;
- old/current/new Project and drum-file round trips;
- transactional apply/cancel and destructive-tail hit counts;
- Arrangement references and play-here behavior;
- note ownership, gate, mute, stop, panic, and target failure;
- MIDI clock and live tempo changes;
- loop clock continuity where applicable;
- controller navigation at 40×13; and
- arbitrary-length copy, paste, clone, load, save, preview, and REC wrap.

Release-mode Pi checks should record timing results and hardware configuration.
Static tests can prove boundaries and ownership, but only the user can approve
whether a swing amount or groove feels musically right.
