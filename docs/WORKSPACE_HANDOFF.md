# Workspace handoff

This document is the durable context for continuing work in
`/home/patch/p/shsynth` without the conversation that created the workspace.
Do not put credentials, GitHub device codes, or private preset files here.

## Repository and publishing

- The checkout is a Git repository on branch `main`.
- Public remote: <https://github.com/PaolaShultz/shr-daw>.
- `main` tracks `origin/main`; push with `git push` after committing.
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
- `user/data/shsynth/songs/`: tracker songs;
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
`docs/PHASE1_AUDIO_GRAPH_MEASUREMENT.md`. Phase 2 has not begun, and no creative
effect is approved by this dry-path result.

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

FT2 real-time REC is hardware-page-only: it refuses `ActiveInstrument`,
consumes notes before the loaded synth, auditions through the selected page's
MIDI destination/channel, and writes only that page in the selected looping
pattern. Pattern setup supports 4/4 sizes 8/16/32/64/128 and corresponding 3/4
sizes 6/12/24/48/96. Songs retain distinct patterns plus their order list.

FT2 has one Play/Rec/Edit/N00B mode state. N00B maps live input to the nearest
selected major/natural-minor scale tone with downward tie-breaking and exact
per-channel/source-note release ownership. The Tools child opens the private
WAV loop player. Loop imports live below the XDG user-data `loops/` directory;
songs keep optional meter, filename, BPM interpretation, and beat-region
settings plus a signed beat offset for one-bar placement shifts. The loop
ALIGN child can run offline pulse/duration analysis, snap length to song bars,
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
column channel/bank/program setups. Song format 1 stores them explicitly;
format 0 page-wide setups migrate into four identical columns, while unknown
newer formats are refused. Compatible shared channels require identical master
selections. FT2 Program cell editing uses the selected column for named live
audition; devices without a profile retain numeric 0–127 access.

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
again with Rust 1.85.

For docs, README, screenshot, or image-only changes, keep validation scoped to
the files changed instead of running the Rust suite mechanically. Examples:
check links/references, verify image dimensions and byte sizes, compile Python
helpers with `python3 -m py_compile`, and run `git diff --check`. Run the full
Rust checks only when code, Cargo files, runtime behavior, or install/runtime
scripts changed.

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
