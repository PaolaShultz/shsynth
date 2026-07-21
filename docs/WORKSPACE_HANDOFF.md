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

On the current Pi account, the plain `shr` command targets this checkout's
`scripts/local.sh` through both the interactive Bash alias in
`/home/patch/.bash_aliases` and the user-local symlink at
`/home/patch/.local/bin/shr`. The alias previously hardcoded an old
`target/release/shr` and therefore hid newer debug workflow builds; do not
restore a build-specific alias. The launcher keeps all writable state below
`user/` and always selects `target/debug/shr`; release and installed binaries
are used only through an explicit `SHSYNTH_BIN` override. The debug build is
incremental and displays a `DEV` badge in the TUI (`REL` identifies an explicit
release build). Rebuilding the debug binary therefore updates what plain `shr`
runs without changing shell startup files. This is machine-local state, not a
tracked installation; verify or restore both entry points after moving the
checkout or replacing the development system.

`SHSYNTH_USER_DIR` may replace `user/`. The launchers set `XDG_STATE_HOME`,
`XDG_DATA_HOME`, `SHSYNTH_PRESET_DIR`, and `SHSYNTH_LOOP_INBOX`; do not replace
this with hardcoded Rust paths. The important local paths are:

- `user/state/shsynth/`: runtime/controller configuration, backups, PID/log
  state, and generated engine configuration;
- `user/data/shsynth/ideas/`: recorded MIDI ideas;
- `user/data/shsynth/songs/`: tracker Projects (`.shsong`);
- `user/data/shsynth/ft2-routing-defaults.shsong`: private new-Pattern routing
  template, created only after the musician confirms the empty-Pattern prompt;
- `user/data/shsynth/demos/`: missing-only cleared MIDI/Project demo copies and
  their public manifest;
- `user/data/shsynth/recordings/`: synchronized take directories, mono stems,
  manifests, recovered/incomplete takes, and legacy stereo WAV recordings;
- `user/data/shsynth/loop-inbox/`: missing-only public starter seeds and any
  private source loops offered for import;
- `user/data/shsynth/loops/`: privately imported FT2 WAV loops;
- `user/data/shsynth/drum-patterns/`: user-saved reusable four-lane drum patterns;
- `user/presets/synthv1/`: cleared copies plus local/legacy presets;
- `user/downloads/`: private source archives.

The tracked `loops/cleared-loops.txt` is the public WAV packaging allowlist.
Its four CC0 stereo 48 kHz/24-bit files are documented and hashed in
`loops/SOURCES.md`; setup copies missing names into the inbox. MusicRadar's
optional 80s drum download is royalty-free for music but forbids raw-sample
redistribution. It must remain below user data and must never be committed or
packaged; setup downloads it directly and keeps a source/terms note beside the
four extracted tempo examples.

The tracked `demos/cleared-demos.json` is the only public demo-song packaging
manifest. It records title/BPM/meter/key/parts, descriptions and restyle ideas,
per-composition public-domain reasoning and institutional links, arrangement
licence, exact filenames, and hashes. `scripts/generate_demo_songs.py` owns the
deterministic 10 MIDI plus 10 format-4 Project outputs and rejects changes or
extras by default. Setup copies demo Projects into `songs/` without replacing
user files and keeps the matching corpus below `demos/`.

The local setup currently selects the MiniLab3 MIDI controller, JACK
`system:playback_1`/`system:playback_2`, AudioBox USB 96 stereo capture through
`system:capture_1`/`system:capture_2`, and the AudioBox MIDI port as the external
hardware destination. These are configuration values, not Rust constants.
Rerun `scripts/setup-local.sh` when hardware or JACK port names change. The
wizard did not replace `~/.jackdrc` and never starts or restarts JACK.

The active development machine is still the Raspberry Pi 4 with 4 GB RAM and
microSD storage described by the dated measurement documents below. A
Raspberry Pi 5 with 2 GB RAM, active cooler, 27 W supply, bottom-mounted NVMe
adapter, and 128 GB NVMe has been ordered but is not present, installed,
configured, or measured. Do not retitle Pi 4 results, change current tuning, or
make Pi 5/2 GB support claims in current product documentation. The unscheduled
hardware migration, footprint audit, isolated-core measurements, and proposed
one-rack `» PRESTO` effect mark are strictly post-Build Week future work in
`docs/PI5_HEADROOM_PLAN.md`. All measurements begin after the Build Week
deadline and require a clean development checkpoint.

The recorder is now generic and synchronized rather than fixed stereo. Repeated
`capture.track=ID|LABEL|GROUP|ROLE|ARMED|JACK_SOURCE` entries remember exact
machine-local sources; blank or missing sources never fall back to another JACK
port. Old `capture.input` pairs remain compatible and appear as one linked
stereo group until deliberately edited. One shared preallocated interleaved ring
transfers all armed channels from the JACK process callback to a non-real-time
writer, so every accepted callback is whole-take and every mono stem has the
same start, stop, rate, and frame count. The callback performs no file I/O,
allocation, logging, or waiting. A take is published as a unique `.take`
directory with 24-bit mono WAVs and format-1 `session.json`; faults publish only
an explicitly incomplete take or retain a recoverable `.take.part`.

The compact recorder screen can select/name/assign tracks, arm one/all resolved/
none, refresh discovery without changing preferences, record one synchronized
take, and show missing inputs, elapsed time, selected activity, drop/xrun/high-
water status, and its final path or failure. The safety cap is 64 tracks, not an
MR18-specific limit. `shr recorder-stress DEST [SECONDS] [CHANNELS] [RATE]
[CALLBACK]` exercises the production ring/writer/publication path without JACK,
MIDI, a synth, or sound. Its primary target is 18 mono channels at 48 kHz and
128 frames/callback. See `docs/MULTITRACK_RECORDING.md` and the fully explicit
helper contract in `docs/MAINTAINER_HELPERS.md`.

The planned first hardware acceptance target is a Midas M AIR MR18, but no
Midas, ALSA-client, USB-card, or JACK-port name is compiled into Rust or public
configuration. `docs/MR18_TEST_PLAN.md` separates official facts from tomorrow's
required observations, preserves exact discovered identifiers only in private
machine configuration, and progresses through 2/4/8/12/16/18 inputs at 48 kHz.
Do not claim a hardware pass until that procedure has been completed. The
three-minute truthful hardware/synthetic fallback script is in
`docs/MULTITRACK_PRESENTATION.md`.

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

The dependency install deliberately uses `--no-install-recommends`: the
FluidSynth CLI's Qsynth recommendation otherwise pulls in the roughly 142 MiB
FluidR3 GM bank despite SHR explicitly using the small TimGM bank. Interactive
setup offers a recommended exclusive-routing cleanup when needed. It stops and
masks only the per-user distribution `fluidsynth.service` and system
`amidiminder.service`, preserving the FluidSynth binary for SHR's owned
on-demand JACK process. The managed FluidSynth command uses its piped shell and
does not enable the unused TCP server. Dependency installation always applies
and verifies the per-user FluidSynth mask; non-interactive setup makes no
additional system-wide service-policy change.

On the current Pi, `/home/patch/.config/systemd/user/fluidsynth.service` and
`/etc/systemd/system/amidiminder.service` are persistent `/dev/null` masks. The
unowned daemon and blanket MIDI patcher are stopped, TCP port 9800 is closed,
and the auto-installed `fluid-soundfont-gm` and Qsynth packages were purged
without running `autoremove`. `/usr/bin/fluidsynth` and the 5.7 MiB TimGM bank
remain for SHR's managed on-demand engine. JACK was not restarted.

The application opens on a minimal black Home list. Its labels are centered in
fixed 36-column bars spanning zero-based columns 2–37 on the 40×20 display,
and the visible block is vertically centered. Home is the navigation root for
Software Synths, FT2, Recorder, Performance, MIDI Learn, Routing, Effects,
Ideas, and Help. Routing reuses the read-only current-connection owner; Effects
opens the existing Project rack. A configured offline controller, controller
without a matching reviewed profile, or incomplete learned encoder recommends
and initially selects MIDI Learn with a plain reason. Learned encoder turn and
click are sufficient even when optional commands are absent. Home never starts
learning or sends MIDI. Top-level Exit returns Home; nested editors return to
their parent first. The controller menu uses a four-page spatial contract on
workspace and modal contexts: page 1 holds the primary workflow (FT2 uses
Page−/Page+/Track−/Track+); `EXIT` is page 4/item 4 and returns one level. MIDI
never quits the application. Physical pages contain no PageUp/PageDown or
unrelated top-level launchers; keyboard paging remains. Empty items/pages are
invisible, silent, and skipped. The visible control strip is centered and
capped at 40 columns. The full map is in
`docs/CONTROLLER_INTERFACE.md`; README carries only the overview and link.

Playback shows the chord, structured held-note columns, and each note's current
decimal MIDI Note On velocity directly beneath it, above a continuous two-row
keyboard state. The velocity is strike data from the controller, not measured
audio loudness. Note Off, velocity-zero Note On, and channel-specific All Notes
Off/All Sound Off remove only that channel's instance. If several channels hold
the same pitch, the display remains pitch-deduplicated and deterministically
shows the greatest still-held velocity, falling back when that instance is
released. At 40 columns the keyboard covers C2–G7 without octave gaps: natural
notes color the white upper background and lower full block red, while sharps
color the upper `└` foreground red. `display.note_names=german` is the B/H
default; `english` selects A#/B for both chord and note text. Buffer tests lock
velocity alignment, compact-layout safety, natural/sharp color ownership, and
the gapless octave boundary. Recognized major triads use an explicit spaced
`maj` suffix (`C maj`), including before a slash bass (`C maj/E`); a single
held C remains `C`.

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

The normal FT2 WAV Loop view has its own independently smoothed stereo
`LOOP OUT` RMS/peak/MAX/clip presentation. Its real-time accumulator taps the
loop player's generated left/right callback samples after loop rendering,
region/cut selection, interpolation, transport gating, and edge fades, just
before those samples enter the existing loop JACK output buffers. It therefore
measures only the independently routed WAV loop and never synths, owned source/
aux/master effects, external inputs, hardware gain, or unrelated JACK clients.
It adds no ports or connections and does not route the loop through the owned
effects graph. Stop, unload, load failure, inactive-client/shutdown, and
oversized-callback paths publish silence/unavailability so stale levels do not
remain; loop/load boundaries reset this meter's presentation lifecycle without
touching `FINAL OUT`.

FT2 real-time REC is hardware-page-only: it refuses Pattern-owned synthv1
pages, consumes notes before the loaded synth, auditions through the selected
page's MIDI destination/channel, and writes only that page in the selected
looping pattern. Pattern setup supports 4/4 sizes 8/16/32/64/128 and
corresponding 3/4 sizes 6/12/24/48/96. Projects retain distinct Patterns plus
their Arrangement.

FT2 has one Play/Rec/Edit/N00B mode state. N00B is the beginner duration-entry
surface for melodic pages: the selected page owns the sound, the one rotary
selector offers 1/1–1/32 with 1/16 as default, and entry uses existing
gate/explicit note-off cells before advancing. N00B is refused on percussion
pages; page/lane movement onto one returns to Play without rewriting cells. Its
context keeps page/track, delete, note-off, play/save/files, Normal, and Exit
controls. Mode switches never rewrite cells.
The Tools child opens the private
WAV loop player. Loop imports live below the XDG user-data `loops/` directory;
Projects keep optional meter, filename, BPM interpretation, and beat-region
settings plus a signed beat offset for one-bar placement shifts. The loop
ALIGN child can run offline pulse/duration analysis, snap length to Project bars,
and move placement by whole bars. JACK loop client/output names and the import
inbox are configuration. Tempo matching sets the current Pattern tempo from the
interpreted WAV BPM; the WAV is not stretched or pitch-shifted to fit the old
tempo. The loop player requires the JACK server sample rate to match the WAV
sample rate, so use JACK setup/restart at 44100 Hz for 44.1 kHz loops when
needed. Fresh setup uses the 48 kHz inbox seeds, writes the selected stereo JACK
playback pair to both `audio.output` and `loop.output`, and asks explicitly for
English `C D E F G A B C` or German `C D E F G A H C` note spelling. The
deterministic loop fixture seeds useful stereo `LOOP OUT` display
data without starting JACK or reading private loop files; mono decoding already
duplicates samples into left/right values, so mono meter readings match.

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
hardcoded tracker mode. Each FT2 page has one destination and four columns.
Project format 4 adds the canonical portable `default` target, whose
device/channel/bank/program route is blank and resolved from active machine
configuration. It also accepts `synthv1:<preset name>` without changing the
format version. Explicit pages still persist their four setups. Format 3 loads
with every route remaining explicit and stores the source insert rack, two aux
routes, and master rack. Formats 0 and 1 migrate
with empty effects routing; format 2 retains its source rack and gains empty
aux/master routing; format 0 page-wide setups also migrate into four identical
columns. Unknown newer formats and invalid fields are refused. Compatible
shared channels require identical master selections. FT2 Program cell editing
uses the selected column for named live audition; devices without a profile
retain zero-based MIDI storage while every musician-facing screen shows
programs 1–128 and channels 1–16.

New FT2 Projects and Patterns use three routing pages: Software Synth with the
first available synthv1 preset, MIDI on channel 1/program 1, and Drums on
channel 10. The current private template may replace those values. Saving a routing-changed
but note-empty Pattern asks exactly whether to save its routing as the new
default; confirm updates the private template, cancel preserves it, and a
Pattern containing notes never changes defaults implicitly. Older
`ActiveInstrument` Projects are upgraded only in memory until explicit save.

Software Synth and FT2 now have separate engine ownership. Presets/Playback
keeps one standalone synth alive while moving within that workflow, sends All
Notes Off and drops only its owned engine on the top-level return to Home, and
never touches an unowned process. FT2 loads the synthv1 preset named by the
current Pattern instead of consulting the standalone selection. Page, channel,
program, destination, and preset changes cancel the old live route first; pad
messages remain consumed and ordinary MIDI auditions the selected page. One
host cannot represent two synth presets simultaneously, so an Arrangement with
multiple enabled synthv1 preset names is refused rather than misrouted.

Hardware-independent validation for this redesign used pinned Rust 1.85:
formatting, `cargo check --locked`, focused ownership/routing/defaults/N00B/MIDI
and 40×20 controller-render tests, an incremental `cargo build --locked`, all
498 Rust tests, and the optimized locked release build passed. The broader test
and release runs were made only after the user explicitly requested them; the
full suite included its JACK-free synthetic stress cases. No JACK client, synth
process, MIDI transmission/playback, audible or physical-hardware test, or
Clippy run was used. The established screenshot set was not regenerated during
the physical-controller review freeze.

### Next hands-on session: 2026-07-21

Resume from the current working tree based on commit `0ea6a41` on `main`.
Plain `shr` resolves through `scripts/local.sh` to the current
`target/debug/shr`; the incremental debug binary was rebuilt after the smart
drum-column work below and should show the red `DEV` badge. There is no known
compile or hardware-independent test failure. The next work is physical
acceptance and debugging, not another implementation pass in advance of
observation.

Use a new empty FT2 Project so existing user music remains untouched. Check,
in order:

1. Software Synth loads the chosen preset, keeps it while moving between its
   Presets and Playback screens, and ends notes/unloads it only on top-level
   Exit to Home.
2. A new FT2 Project shows Software Synth, MIDI channel 1/program 1, and Drums
   channel 10; FT2 must not inherit the last standalone preset.
3. Keyboard and ordinary musical MIDI audition the selected FT2 page. Change
   page, channel, program, destination, and synth preset while listening for
   stuck notes; command-pad presses/releases must remain silent and consumed.
4. On Software Synth or MIDI, N00b Mode opens one rotary length selector,
   defaults to 1/16, and enters 1/1, 1/2, 1/4, 1/8, 1/16, and 1/32 notes without
   changing existing cells when modes are switched. On Drums it must refuse to
   open; moving onto Drums from melodic N00b must return to Play unchanged.
5. On a note-empty Pattern, change routing and Save: Cancel must retain the old
   defaults, Confirm must seed the next new Pattern, unchanged routing must not
   prompt, and a Pattern containing notes must not alter defaults.
6. On the Drums page in Step Edit, establish kick and Crash Cymbal 2 in
   different columns, then play both on a later row. Each must return to its
   earlier column. Confirm new bass drums and snares begin in columns 1 and 2,
   a simultaneous collision falls to a free column without replacing an
   unrelated cell.

All physical equipment used for this project is borrowed from friends. Preserve
its configuration and do not start JACK, synth, MIDI transmission, or audible
tests without the creator's explicit go-ahead. Keep observed hardware names in
private configuration, never Rust or tracked documentation.

The 2026-07-21 non-audible smart drum-column implementation applies only to
pages marked as percussion. Step Edit computer-keyboard entry and incoming MIDI
gestures search earlier rows of the current Pattern across all four columns,
prefer exact-note history, then kick/snare family history and homes, then a free
column. Existing Patterns are never rearranged. Unrelated notes, commands, and
fallback note-offs are preserved; a note-off in the returning voice's own lane
may be replaced by its new hit. Melodic entry is unchanged. N00B is deliberately
unavailable on percussion pages, and real-time REC retains its active-note
allocator because overlapping note-on/off ownership is a different constraint.
Rust 1.85 formatting, focused smart-placement/N00B-exclusion tests, the existing
melodic chord/selected-column regressions, `cargo check --locked`, and the
incremental `cargo build --locked` passed. No release build, complete suite,
JACK client, synth, MIDI transmission, or audible test was used.

Preferred routing and resolved runtime routing are separate. MIDI targets are
re-resolved for every transport start. An unavailable exact MIDI target may use
the configured external hardware route but never falls into the Pattern's
software synth; its preference is not mutated, so a returned target is used on
a later play. Portable `AUTO` retains its machine-default compatibility path.
Controller
open failure leaves keyboard navigation/entry active. Audio activation tries
the preferred pair, ordered named internal pairs, and the separately configured
analogue headphone pair last; the status identifies both fallback and missing
preference. The in-memory loop player uses the same resolved pair. No fallback
is written back to `shsynth.conf`.

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

The MiniLab 3 factory Arturia and DAW pads were directly captured as notes
36–43 on MIDI channel 10; the enabled User 1 program used channel 1, colliding
with the keyboard. Controller commands now retain an optional channel
qualifier, and the reviewed MiniLab profile uses channel 10 for page 1–4 and
item 1–4. Matching Note On/Off, velocity-zero releases, and polyphonic pressure
are consumed; the same notes on channel 1 remain musical. The old DAW Shift
CC27 pad-lock binding was removed because ordinary Shift gestures toggled it.
The captured exact Arturia program-notification SysEx remains forwarded rather
than adding a broad manufacturer filter; profile-qualified metadata matching is
the safe future boundary if consuming it becomes necessary.

The 2026-07-20 controller/effects workflow rebuild was deliberately committed
before physical testing at the user's request. It unifies F5–F12 with physical
pads 1–8, unifies Up/Down/Enter with encoder rotation/click, adds in-app MIDI
Learn, and implements the serial effect-rack and parameter-editor workflows.
In-app MIDI Learn now bootstraps the master encoder left/right/click before any
optional mapping. The learned encoder browses control and command roles; click
saves the partial or complete learned profile and exits, while Esc cancels.
Command roles are optional and infer the four-, five-, or eight-button layout
instead of assuming the connected device has eight buttons.
The active private controller state is
`user/state/shsynth/controller.conf`, using reviewed profile
`arturia-minilab-3` and exact input `Minilab3:Minilab3 MIDI`; its pre-refresh
backup is `user/state/shsynth/controller.conf.bak-1784558588`. Keep this task
open until the user has exercised the debug build on the physical 40×20 Pi
display and MiniLab 3. Do not regenerate screenshots or broaden documentation
until that approval.

Optional `controller_clock.*` configuration owns a dedicated exact stable ALSA
standard-MIDI output and is off by default. It shares tracker transport tempo,
sends only `FA`, evenly timed `F8` at 24 PPQN, and `FC`, and never uses a musical
page.
The source port is ALSA `NO_EXPORT` and uses directly addressed events; this is
required because the Pi's JACK sequencer bridge otherwise auto-subscribes to a
normal RtMidi output and duplicates its route.
Clock runs while the enabled app is open, using default tempo before the first
run; direct MiniLab validation established that clock must be detected before
Start. Enabling it permits an empty Pattern for live external-sync
arpeggiation. SHR has no pause/resume state, so it sends Start for each fresh
run and does not send Continue or Song Position Pointer; Stop does not silence
Timing Clock.

The 2026-07-20 non-audible validation addressed only the current standard MIDI
endpoint (`Minilab3 MIDI`, then ALSA `32:0`; the number is volatile). USB wire
monitoring saw `FA`, repeated `F8` intervals around 20,833 microseconds at 120
BPM, and `FC`, with no channel messages or SysEx on that connection. With the
MiniLab in Arturia mode, Sync External, and its arpeggiator enabled, a fresh
Start after clock detection produced Note On/Off on the same standard port,
channel 1. The observed repeating pitches included 72, 79, 76, 84, 91, 88, 96,
103, and 100. The passive capture is
`/tmp/shr-minilab-clock-validation-20260720-003.log`. A second capture against
the final release build,
`/tmp/shr-minilab-clock-validation-20260720-004.log`, confirmed that
stopped-state clock before the first fresh Start produced the arpeggio
immediately, without the earlier manual transport restart. `/tmp` evidence is
not expected to survive a reboot. DAW and User 1 were not repeated because the
dedicated clock protocol and endpoint do not depend on controller program, and
their no-clock behavior plus pad channels were already captured separately.

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

Use the repository-required Rust 1.85 toolchain. Normal development uses fast
incremental debug checks and focused tests; build the debug binary when it is
ready for physical testing:

```sh
export PATH=/home/patch/.rustup/toolchains/1.85.0-aarch64-unknown-linux-gnu/bin:$PATH
cargo fmt -- --check
cargo check --locked
cargo test --locked FILTER
cargo build --locked
```

During the competition heavy-test phase, do not run the full test suite,
warning-denied Clippy, an optimized release build, or release stress
validation. This temporary fast-iteration rule remains in force until the
competition deadline; use formatting, `cargo check --locked`, and focused tests
for the exact changed behavior. A commit, handoff, or broad validation request
does not by itself override this phase rule. Historical release results below
remain evidence for their dated commits, not the current iteration policy.

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

After the loop-only meter and Playback velocity work on 2026-07-19, formatting,
all 394 Rust tests, warning-denied Clippy, and the optimized locked release
build passed with Rust 1.85. The deterministic screenshot set was regenerated;
the Playback and FT2 Loop root images plus their eight menu-page variants
changed, while the visual manual remained at 80 menu screenshots. Representative
40×20 Playback and Loop frames were inspected, the exhaustive screenshot check
passed, all 34 tracked Markdown files had valid local paths, image references,
and heading fragments, `git diff --check` passed, and no tracked path existed
below `user/`. No JACK server, synth engine, MIDI hardware, private loop file,
recording, or audible test was used.

After the starter-loop/setup work on 2026-07-19, all four public WAVs passed
stereo 48 kHz/24-bit, exact-frame, provenance-hash, local-seed, no-replacement,
and staged-package allowlist checks. The optional private MusicRadar path was
exercised end to end at 48 kHz for its 85/110/120/140 BPM selections. Shellcheck,
formatting, all 394 Rust tests, warning-denied Clippy, and the optimized locked
release build passed. No JACK client, synth engine, MIDI transmission, playback,
recording, or audible test was started.

After the portable-routing, non-destructive-fallback, and cleared-demo work on
2026-07-19, Rust 1.85 formatting, all 401 tests, warning-denied Clippy, and the
optimized locked release build passed. The deterministic generator validated
10 format-1 MIDI files, 10 loadable format-4 portable Projects, their metadata,
provenance links, hashes, and exact manifest membership. A non-interactive XDG
fixture verified missing-only Project seeding, and an isolated `DESTDIR`
contained exactly the 21 manifest-cleared demo files. Shellcheck/bash syntax,
Python compilation, all 36 Markdown files' local targets, `git diff --check`,
and the no-tracked-`user/` boundary passed. No JACK start/restart, synth engine,
MIDI transmission, playback, recording, or audible test was used.

After the hardware-independent synchronized-recorder work on 2026-07-19, Rust
1.85 formatting, all 409 tests, warning-denied Clippy, and the optimized locked
release build passed. The final release helper produced and re-read an 18-mono-
stem, 48 kHz, 128-frame synthetic take: all 48,000 frames and identity probes
agreed, drops/overflows were zero, and writer high-water was 384 frames. This is
synthetic recorder/storage evidence, not an MR18 hardware result. Recovery,
future/malformed manifest refusal, traversal/symlink/no-replace safety, source
loss, missing/reconnected preference, callback violation, bounded overflow,
slow/failing writer, zero-frame and legacy-stereo paths passed. The regenerated
40×20 recorder and controller images passed the exhaustive renderer check; all
47 Markdown files passed local path and heading-fragment validation; `git diff
--check` passed; and a 203-file isolated install contained the new recorder
guides, configuration, and images before staged uninstall. No JACK start or
restart, live graph edit, synth, MIDI transmission, hardware configuration,
audible test, or MR18 claim was made.

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
8. Validate only what changed, inspect the staged tree, then push. During the
   competition heavy-test phase, keep code/runtime validation to formatting,
   `cargo check --locked`, and focused tests for the changed behavior.

## Final stereo performance bus (2026-07-20 implementation)

The opt-in owned graph now instantiates exactly the managed synth, owned WAV
loop, and one exact configured stereo capture pair. It transactionally replaces
the direct synth/loop routes, applies the existing managed-source/aux and master
DSP, then a dedicated -1 dBFS stereo-linked 3 dB-knee sample-peak limiter with
2.5 ms lookahead. The post-limiter buffer is shared exactly by the final meter,
new 24-bit interleaved stereo recorder tap, and JACK playback. See
`docs/FINAL_PERFORMANCE_BUS.md` for the full boundary and limitations.

Machine source names remain only in runtime configuration; Project format stays
4 and old configuration falls back to its first legacy capture pair. The raw
multitrack recorder and legacy `.wav.part` recovery remain separate and intact.
MTR provides the compact three-source level/mute/readiness, master, limiter,
meter, and final-record controls through its four controller pages.

Hardware-independent validation and release stress results belong in the
handoff report for this change. No full-duplex JACK/interface or MR18 acceptance
was run during implementation; `docs/MR18_TEST_PLAN.md` records the later
procedure without invented port names.

Final release-mode synthetic evidence used three distinguishable stereo sources
for one paced second per row. At 48 kHz, 64/128/256/1024-frame callbacks had
mean times of 15.294/29.111/58.912/203.826 microseconds; the worst single
callback was 295.517 microseconds. A 44.1 kHz, 128-frame run averaged 30.614
microseconds and peaked at 276.314 microseconds. Every run reported 2.60 dB
maximum limiter reduction, writer high-water no greater than one 1024-frame
callback (128/128/256/1024 and 128 frames respectively), zero dropped frames,
zero overflows, and complete playback/WAV PCM equality. This is synthetic
hardware-independent evidence only.
