# Workspace handoff

This document is the durable context for continuing work in
`/home/patch/p/shsynth` without the conversation that created the workspace.
Do not put credentials, GitHub device codes, or private preset files here.

## Repository and publishing

- The checkout is a Git repository on branch `main`.
- Public remote: <https://github.com/PaolaShultz/shr-daw>.
- `main` tracks `origin/main`; push with `git push` after committing.
- The complete 80-image visual menu manual was checkpointed and pushed as
  commit `46c1a1b` (`Add complete visual menu manual`) before the subsequent
  source-backed documentation refresh began.
- GitHub CLI is installed and authenticated as `PaolaShultz` using HTTPS.
- This repository uses the local commit identity `PaolaShultz` with GitHub's
  numeric no-reply address. Do not replace it with an invented identity.
- Before committing, run `git status --short` and confirm that no path beneath
  `user/` is staged. Before pushing, run `git diff --cached --check`.
- The GitHub repository is public. Treat every tracked file as publishable.

Typical publish sequence:

```sh
git status --short
git add --all
git diff --cached --check
git commit -m "Describe the change"
git push
```

Use `gh auth status` if publishing fails. If authorization has expired, use
`gh auth login --hostname github.com --git-protocol https --web` and let the
user complete the device flow. Never print or record the resulting token.

## Public and private data boundary

The whole private runtime tree is `user/`, which is ignored by Git. It contains
state, configuration, logs, ideas, tracker songs, recordings, downloads, and
local/uncleared synthv1 presets. Cargo caches and `target/` are ignored too.

Repository-local operation uses:

```sh
./scripts/setup-local.sh  # interactive hardware configuration
./scripts/local.sh        # run SHR-DAW with all writable data below user/
```

`SHSYNTH_USER_DIR` may replace `user/`. The launchers set `XDG_STATE_HOME`,
`XDG_DATA_HOME`, and `SHSYNTH_PRESET_DIR`; do not replace this with hardcoded
Rust paths. The important local paths are:

- `user/state/shsynth/`: runtime/controller configuration, backups, PID/log
  state, and generated engine configuration;
- `user/data/shsynth/ideas/`: recorded MIDI ideas;
- `user/data/shsynth/songs/`: tracker Projects (`.shsong`);
- `user/data/shsynth/recordings/`: stereo WAV recordings;
- `user/data/shsynth/loops/`: privately imported FT2 WAV loops;
- `user/data/shsynth/drum-patterns/`: user-saved reusable four-lane drum patterns;
- `user/presets/synthv1/`: cleared copies plus local/legacy presets;
- `user/downloads/`: private source archives.

The local setup currently selects the MiniLab3 MIDI controller, JACK
`system:playback_1`/`system:playback_2`, AudioBox USB 96 stereo capture through
`system:capture_1`/`system:capture_2`, and the AudioBox MIDI port as the external
hardware destination. These are configuration values, not Rust constants.
Rerun `scripts/setup-local.sh` when hardware or JACK port names change. The
wizard did not replace `~/.jackdrc` and never starts or restarts JACK.

The optional dedicated-core audio profile is installed for CPU 3. Both
`user/state/shsynth/shsynth.conf` and the normal per-user runtime configuration
set `audio.engine_cpu=3`. `/boot/firmware/cmdline.txt` has the tool-owned
`isolcpus`, `nohz_full`, `rcu_nocbs`, and `irqaffinity` arguments; the
`shr-audio-performance.service` governor service is enabled and active; and the
JACK system service has a tool-owned CPU-affinity drop-in. The boot-time parts
are now active after a later reboot. Inspect with `shr-audio-tune status`;
reverse only the managed settings with `sudo shr-audio-tune remove`, clear
`audio.engine_cpu`, and reboot. The original boot command line is retained
below `/var/lib/shr-audio-tune/`.

The Phase 1 owned dry graph passed its first authorized Raspberry Pi checkpoint
on 2026-07-18 at 48 kHz, 128 frames, and 3 periods. One managed `Compact Bass`
source was bit exact through the graph for all 384,000 captured frames, with no
direct-path doubling, missed callback deadlines, or oversized callbacks. The
main run measured 8.246 us mean, 13 us p95, 28 us p99, and 151.907 us maximum
over 29,039 callbacks. Normal shutdown exposed the exact direct fallback before
the managed synth exited; an unrelated capture connection survived; full JACK
loss left no stale owned resources; and a subsequent direct start succeeded.
The ignored local `audio.graph.enabled` flag was returned to `false`. See
`docs/PHASE1_AUDIO_GRAPH_MEASUREMENT.md`.

Phase 2's bounded eight-slot source insert rack is implemented: five-section
EQ, linked feed-forward compressor, three honestly named distortion transfers,
crusher/sample-rate reducer, linked gate/expander, and stable multimode filter.
It includes strict schemas and persistence, stable IDs and compatible state
retention, allocation-free preallocated processing, smoothing/click-conscious
bypass, compact 40x20 editors, the four-page controller contract, and meters.
Deterministic response/curve/stability tests provide objective technical
quality measurements; musical preference remains a human curation decision.

The authorized Raspberry Pi 4 checkpoint on 2026-07-18/19 used the dedicated
audio core, AudioBox USB 96, JACK 48 kHz/3 periods, and low-velocity `Compact
Bass`. At 128 frames, an eight-effect capacity chain ran 30.084 seconds with
11,335 callbacks, 164.390 us mean, 185 us p95, 202 us p99, 266.313 us maximum,
and zero missed/oversized callbacks. At 64 frames it ran 60.038 seconds with
45,151 callbacks, 87.809 us mean, 107 us p95, 132 us p99, 446.516 us maximum,
and zero missed/oversized callbacks. There were no xruns in either sustained
window; synth-client xruns began at deliberate teardown and are recorded as a
future cleanup issue. The service was restored to 128 frames, the ignored local
graph flag remains `false`, the engine was stopped, and no stale JACK resources
remained. See `docs/PHASE2_AUDIO_GRAPH_MEASUREMENT.md`.

The user then explicitly changed the phase order to build as much of the safe
internal effects graph as possible before one consolidated listen-and-repair
pass. Phase 3/4 now add bounded stereo delay, chorus, flanger, phaser,
tremolo/autopan, three shared-topology FDN reverb voicings, two forced-wet
pre/post aux buses, and a master insert rack. Project format 3 migrates the
routing without overwriting newer data. Hardware loops and full-duplex live
input are still deferred because they require physical-interface monitoring
decisions. See `docs/PHASE3_4_AUDIO_GRAPH_MEASUREMENT.md`.

The competition appliance keeps that one-managed-source topology. The later
multi-strip mixer and two shared-aux migration is specified in
`docs/POST_COMPETITION_MIXER_AUX_PLAN.md`. Its audit found two narrow
correctness defects that were repaired without starting that rewrite. A
dedicated final meter now sits after the complete master rack and immediately
before playback. Aux bypass is placement-aware: source/master inserts retain
dry passthrough, while an aux with no active wet generator fades its return to
silence; delay-tail bypass drains wet-only with its input muted, and bypassed
serial processors preserve a safe path around another active/tail wet
generator. An all-bypassed chain never exposes its raw send.

The FX rack/editor is deliberately available when `audio.graph.enabled=false`:
it validates and saves Project routing without processing or metering the
direct audio path. Only an enabled graph needs runtime replacement publication,
so only that mode blocks FX changes until transport and all recording are
stopped. Keep this distinction explicit in musician and architecture docs.

The Phase 3/4 Pi checkpoint on 2026-07-19 measured the combined eight source
inserts, two aux reverbs, and master compressor for 60 seconds. At 128 frames it
reported 313.572 us mean, 360 us p99, 540.108 us maximum, and zero
missed/oversized callbacks. At 64 frames it reported 158.527 us mean, 198 us
p99, 349.905 us maximum, and zero missed/oversized callbacks. No sustained-run
xrun occurred; the known synth teardown-only xruns remain. `/etc/jackdrc` was
restored byte-for-byte to 48 kHz/128 frames/3 periods, JACK is active, SHR is
stopped, graph enablement remains absent/default-false, and no owned ports
remain. The consolidated KEEP/IMPROVE/DROP listening sheet is still open.

At installation time the AudioBox USB 96 was disconnected. Consequently the
pre-existing `jack.service` remained failed because ALSA could not resolve
`hw:A96`; this is a hardware-availability issue, not a tuning failure. Do not
start or restart JACK merely to validate the affinity profile. At the
2026-07-18 graph checkpoint the AudioBox was connected and JACK was active at
48 kHz, 128 frames, and 3 periods.

The product and Cargo package are named `shr-daw`. The regular installer
provides `shr`, `shr-setup`, and `shr-audio-tune`; `shs` and `synth-player` are
compatibility aliases to the same Rust `shr` binary and therefore share its
engine ownership and shutdown behavior. The repository helpers above are
intentionally development/local-checkout commands. The `shsynth` state, data,
configuration, and shared-data paths remain unchanged for compatibility.

The controller menu uses a four-page spatial contract on every screen and
modal context: page 1 is `OPS`; on child screens and editors, `EXIT` is always
page 4/item 4 and returns one level. MIDI never quits the application. Empty
items/pages are invisible, silent, and skipped. The visible control strip is
centered and capped at 40 columns. The full map is in
`docs/CONTROLLER_INTERFACE.md`; README carries only the overview and link.

Playback shows chord and held-note text above a continuous two-row keyboard
state. At 40 columns it covers C2–G7 without octave gaps: natural notes color
the white upper background and lower full block red, while sharps color the
upper `└` foreground red. `display.note_names=german` is the B/H default;
`english` selects A#/B for both chord and note text. Buffer tests lock the
natural/sharp color ownership and the gapless octave boundary. Recognized
major triads use an explicit spaced `maj` suffix (`C maj`), including before a
slash bass (`C maj/E`); a single held C remains `C`.

Presets NAV item 1 opens the passive `MTR` performance screen. CPU0–CPU3 come
from bounded UI-side `/proc/stat` deltas, with the configured temperature when
available. Live stereo smoothed RMS, a short decaying peak marker, independent
non-decaying L/R `MAX` peaks, and clip state come only from the active owned
graph's dedicated post-master meter. Direct mode, stopped engines, WAV loops,
hardware returns, recorder inputs, and unrelated JACK clients are never
presented as final-output activity; direct/stopped output is explicitly
unavailable and lifecycle changes clear stale maxima. MTR RESET clears
presentation holds only. Every downward movement of the mapped synthv1 Volume
control clears both numeric maxima before pickup acceptance, while increases,
equal values, and unrelated controls do not. CPU rows are whole-core load: they
do not measure synth/graph process CPU, callback timing, scheduling jitter, or
xruns. Its deterministic README screenshot says that it uses presentation
data.

FT2 real-time REC is hardware-page-only: it refuses `ActiveInstrument`,
consumes notes before the loaded synth, auditions through the selected page's
MIDI destination/channel, and writes only that page in the selected looping
pattern. Pattern setup supports 4/4 sizes 8/16/32/64/128 and corresponding 3/4
sizes 6/12/24/48/96. Projects retain distinct Patterns plus their Arrangement.

FT2 has one Play/Rec/Edit/N00B mode state. N00B maps live input to the nearest
selected major/natural-minor scale tone with downward tie-breaking and exact
per-channel/source-note release ownership. The Tools child opens the private
WAV loop player. Loop imports live below the XDG user-data `loops/` directory;
Projects keep optional meter, filename, BPM interpretation, and beat-region
settings plus a signed beat offset for one-bar placement shifts. The loop
ALIGN child can run offline pulse/duration analysis, snap length to Project bars,
and move placement by whole bars. JACK loop client/output names and the import
inbox are configuration. Tempo matching sets the current Pattern tempo from the
interpreted WAV BPM; the WAV is not stretched or pitch-shifted to fit the old
tempo. The loop player requires the JACK server sample rate to match the WAV
sample rate, so use JACK setup/restart at 44100 Hz for 44.1 kHz loops when
needed.

FT2 Edit has a persistent 1/2/4/8-row ADD value used by note/chord entry,
blank, erase, and note-off. Project Files has a Pattern child for lifecycle,
clipboard, and atomic melody-only ±1/±12 transpose. Its Drums child loads the
70-plus authored common-rhythm library into the first percussion page without
changing routing, and saves user `.shdrum` files below the XDG user-data
`drum-patterns/` directory. Filters select 10 genres, 3/4 or 4/4, and
24/48/96 or 32/64/128 rows. Loading may resize an empty melodic Pattern, but
refuses shape changes once melodic data exists. Bundled drum patterns and the
compact `.shrdrums` catalog are read-only.

Deferred external-MIDI routing ideas are recorded in
`docs/FUTURE_IMPROVEMENTS.md`: opt-in multi-target live thru and stable aliases
for otherwise indistinguishable USB-MIDI adapters. Current FT2 page playback
already supports simultaneous distinct output targets, including the same MIDI
channel on different ports; do not broaden normal live thru implicitly.

External MIDI sound names are data-driven. JSON profiles live in
`midi-devices/` (installed below `share/shsynth/midi-devices/`), while private
overrides can live below `${XDG_DATA_HOME}/shsynth/midi-devices/` or
`SHSYNTH_DEVICE_PROFILE_DIR`. `roland-d-50` is the first bundled profile, not a
hardcoded tracker mode. Each FT2 page has one destination and four persisted
column channel/bank/program setups. Project format 3 stores those setups plus
the source insert rack, two aux routes, and master rack. Formats 0 and 1 migrate
with empty effects routing; format 2 retains its source rack and gains empty
aux/master routing; format 0 page-wide setups also migrate into four identical
columns. Unknown newer formats and invalid fields are refused. Compatible
shared channels require identical master selections. FT2 Program cell editing
uses the selected column for named live audition; devices without a profile
retain numeric 0–127 access.

Project display names are editable and saved renames publish without replacing
collisions. Pattern cleanup deletes only zero-reference records and never
rewrites Arrangement steps. Remove Loop remains detach-only; the separate
private loop library refuses referenced files, symlinks, and unsafe paths and
requires confirmation before physical deletion.

USB input-controller setup is also data-driven. Reviewed JSON entries live in
`controller-profiles/catalog.json`; installed and user-updated search paths
mirror the external-device profile model. The generic `controller.conf` is
empty so unknown hardware never inherits MiniLab commands. `shr-setup` runs
`shr pads auto` for the selected ALSA input and offers the non-audible `shr
pads learn` wizard when no profile matches. Selecting an unknown controller
clears the previous device mapping after making a backup. Learning supports
absolute CCs, both relative-encoder directions, CC or note buttons, and
note-based encoder presses while rejecting conflicts. The existing MiniLab 3
mapping moved to the reviewed catalog. `shr pads update` downloads only SHR's
validated catalog;
Ardour, Mixxx, Zynthian, and Pencil Research data are documented research
sources and are not redistributed.

## Preset provenance decision

Only the 21 cleared synthv1 presets listed in
`presets/synthv1/cleared-presets.txt` and documented in `THIRD_PARTY.md` belong
in the tracked directory or public installation. The manifest is the packaging
allowlist and schema-test source of truth. They are MIT with the project.
`scripts/generate_cleared_presets.sh` records their authored recipes.

The private LinuxSynths archive is:

- source: <https://linuxsynths.com/Synthv1PatchesDemos/392Synthv1Patches.tar.gz>;
- local path: `user/downloads/392Synthv1Patches.tar.gz`;
- SHA-256: `f4f9157cf5d245f7371a702584e28a90d1cf92b9a1eec9fa38c43fad584016ea`;
- contents: 392 preset files with no licence or verified authorship notice.

It was extracted locally without overwriting existing files. The private
directory currently contains 424 unique synthv1 presets after merging cleared
and pre-existing local sounds. It is available for this checkout's private
use, but must not be committed, packaged, mirrored, relabelled as MIT, or
downloaded by the public installer. `user/SOURCE.txt` retains the local note.
No stated licence does not mean public domain.

The public bank contains 21 files and the private bank 424 files. Check this
boundary with:

```sh
find presets/synthv1 -maxdepth 1 -type f -name '*.synthv1' | wc -l
find user/presets/synthv1 -maxdepth 1 -type f -name '*.synthv1' | wc -l
git check-ignore -v user/downloads/392Synthv1Patches.tar.gz
git ls-files | rg '^user/'
```

The final command must produce no output.

## Installed tools and validation

Read `docs/MAINTAINER_HELPERS.md` before changing or replacing anything below
`scripts/` or its related Make targets. It is the durable reference for helper
arguments, environment overrides, side effects, safety boundaries, and design
rationale. In particular, the screenshot renderer's explicit pixel-copy loops
and exhaustive 2×2 check are intentionally slower than a library resize.

On this Raspberry Pi, `gh`, `libxml2-utils` (`xmllint`), and `shellcheck` are
installed. If a
required validation or publishing tool is missing, install it instead of
silently weakening or skipping the check. Validate all local and public preset
XML with:

```sh
find presets/synthv1 user/presets/synthv1 -maxdepth 1 \
  -type f -name '*.synthv1' -print0 | xargs -0 -n1 xmllint --noout
```

Use the repository-required Rust 1.85 toolchain for changes that touch Rust,
Cargo metadata, installer behavior, runtime configuration, preset validation,
or application behavior:

```sh
export PATH=/home/patch/.rustup/toolchains/1.85.0-aarch64-unknown-linux-gnu/bin:$PATH
cargo fmt -- --check
cargo test --locked
cargo clippy --locked -- -D warnings
cargo build --release --locked
```

At the time this handoff was written, all 445 public-plus-private XML files
validated, 133 Rust tests passed, the bundled MIDI-device JSON parsed and
installed correctly, Clippy passed with warnings denied, formatting passed,
and the release build succeeded. Run the checks again after changes;
this statement is history, not a substitute for current verification.

After the Phase 1 metrics/recovery changes on 2026-07-18, formatting, all 283
Rust tests, warning-denied Clippy, and the optimized locked release build passed
again with Rust 1.85. At the Phase 2 final checkpoint, all 333 Rust tests,
formatting, warning-denied Clippy, and the optimized locked release build passed
with Rust 1.85.

After Phase 3/4 implementation, all 356 Rust tests, formatting,
warning-denied Clippy, and the optimized locked release build passed with Rust
1.85 before the documentation-only measurement update.

For docs, README, screenshot, or image-only changes, keep validation scoped to
the files changed instead of running the Rust suite mechanically. Examples:
check links/references, verify image dimensions and byte sizes, compile Python
helpers with `python3 -m py_compile`, and run `git diff --check`. Run the full
Rust checks only when code, Cargo files, runtime behavior, or install/runtime
scripts changed.

The documentation information architecture is deliberate: root `README.md` is
the concise product overview; `docs/README.md` is the wiki-style home;
`HOW_IT_WORKS.md` explains ownership and behavior; `AUDIO_GRAPH.md` and
`CONFIGURATION.md` hold detailed implementation/persistence contracts;
`USING_SHR_DAW.md`, `TRACKER.md`, `HELP.md`, and the visual manual speak to the
musician; measurement and Build Week documents retain dated evidence; and
future-plan documents must not present proposals as current behavior. Keep
terms consistent: Project, Pattern, page, column, Arrangement, Idea, audio
recording, source insert, aux send/return, and master rack. Link to the detailed
contract instead of copying a long feature inventory into every guide.

For a documentation-only refresh, enumerate every tracked Markdown file, check
local Markdown paths, image paths, and heading fragments, run the exhaustive
screenshot check when images/navigation are referenced, run targeted Rust
tests only for documentation embedded in the binary, then finish with:

```sh
python3 scripts/render-readme-screenshots.py --check
git diff --check
git ls-files | rg '^user/'
```

The last command must produce no output. A comment-only correction to the
configuration template does not change runtime behavior; validate its syntax
and relevant parser/help tests rather than mechanically running the full Rust
suite. Any actual Rust, parser behavior, installer, Makefile, or runtime script
change still requires the full Rust 1.85 validation above.

The complete visual interface reference starts at `docs/MENU_MANUAL.md` and is
split into focused chapters below `docs/menu/`. Its 80 menu-page images come
from 25 populated deterministic scenarios in `src/ui.rs`; available pages and
labels are read from the canonical `src/navigation.rs` tables. The renderer
draws the real 40×20 ratatui UI with the PSF console font at 480×320, then uses
explicit integer copying to make a 960×640 PNG in which every source pixel is
an exact 2×2 square. Do not substitute antialiased font or resize rendering.
After a fixture/renderer change, inspect one frame before the batch, then run:

```sh
python3 scripts/render-readme-screenshots.py --only menu/ft2-step-edit-add.png
python3 scripts/render-readme-screenshots.py
python3 scripts/render-readme-screenshots.py --check
```

The reusable Codex workflow is private and ignored at
`user/codex/skills/shr-menu-documentation/`; it must never be staged or moved
into the public project without explicit user direction.

## Safety and fresh-session checklist

1. Read `AGENTS.md`, this file, `THIRD_PARTY.md`, and the relevant source docs.
2. Run `git status -sb`; preserve existing and ignored user data.
3. Use `scripts/local.sh` for the self-contained checkout.
4. Never start an audible synth or JACK test without explicit permission.
5. Never manage, kill, or layer processes outside SHR-DAW's ownership rules.
6. Keep hardware routes and executable/client names in configuration.
7. Keep the 12 mapped synthv1 controls and pickup/reset invariants intact.
8. Validate only what changed, inspect the staged tree, then push. Run all Rust
   checks for code/runtime changes, not for docs or image-only commits.
