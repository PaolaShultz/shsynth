# Workspace handoff

This file contains only current machine state and decisions that must survive a
new thread in `/home/patch/p/shsynth`. Durable repository policy is in
`AGENTS.md`; detailed helper behavior is in `docs/MAINTAINER_HELPERS.md`. Never
record credentials, GitHub device codes, or private file contents here.

## Current priority and shared checkout

The Build Week snapshot is preserved by its tag; the repository itself is
unfrozen and ordinary development continues on `main`. Do not keep or recreate
a standing `dev` branch before the owner opens the planned 0.6 milestone. The
temporary combined build-and-test gate in `AGENTS.md` still applies; an
unfrozen repository is not implicit permission to compile.

The current ordered targets are owned by `docs/RELEASE_ROADMAP.md`: 0.4 checks
all existing menus/workflows plus clean install/setup on Raspberry Pi OS Lite;
0.5 completes the owner-specified FT2 behavior without pulling random future
features into scope; 0.6 implements and physically accepts simultaneous
18-channel playback and 18-channel recording. Package version `0.3.92` is the
corrected starting point; the current checked-progress version is `0.3.94`.

The complete first musician/operator workflow review and its persistent repair
ledger are in `docs/WORKFLOW_AUDIT_HANDOFF.md`. Its R01–R15 `READY` queue has
completed the source/docs/test repair pass and is recorded there as
`SOURCE DONE`; the combined build/test and screenshot evidence still requires
explicit authorization. Its `DECISION` questions remain for the owner pass,
and its physical queue remains untouched. Neither the audit nor source pass
performed a compile, physical test, private-data inspection, or
hardware/audio/MIDI action.

The complete deterministic documentation screenshot set is reconciled to the
current UI; physical approval remains the next gate for UI/controller work.
The repository-only release pass on 2026-07-22 completed at package version
`0.3.94`: all 655 tests ran with 651 passing and four intentionally ignored;
locked check, debug, and release builds and warning-denied Clippy passed; the
105-image screenshot set regenerated and passed its exhaustive drift check;
and the isolated install fixture contained only the expected public package
tree. No JACK, synth, MIDI, playback, recording, or hardware-changing test was
started for that pass.

Multiple workers use this checkout and commit their own changes independently.
Branch tips, commit messages, and clean/dirty snapshots are intentionally not
recorded here. Inspect live Git state, preserve concurrent work, commit only
your own scope, and do not wait for unrelated workers to finish; follow the
canonical collaboration rule in `AGENTS.md`.

Plain `shr` resolves to this checkout's `scripts/local.sh` through both
`/home/patch/.bash_aliases` and `/home/patch/.local/bin/shr`. The launcher uses
`target/debug/shr` unless `SHSYNTH_BIN` is explicitly set; the debug TUI shows
`DEV`. Do not restore the obsolete release-binary alias.

## Active DSP/JACK continuation (2026-07-22)

The current DSP closure pass must be continued, not recreated. It
adds validated FFT/alias analyzers; centered four-point Lagrange interpolation
for delay, chorus, and flanger; first-order ADAA on the filter cubic pre-drive;
short reverb input all-pass diffusion; comprehensive nonlinear/interpolation/
reverb tests; and private level-matched audition renders. Distortion retains
first-order ADAA after multi-bin characterization. The implementation and
focused provenance are in `src/dsp/`, `src/effects/`, `src/effect_schema.rs`,
`src/main.rs`, `docs/AUDIO_GRAPH.md`, and `docs/CONFIGURATION.md`. Do not edit
the roadmap or historical Phase 2/3/4 measurements for this work.

The earlier DSP-focused offline validation was coherent: its complete suite
passed 648 tests with zero failures and four intentionally ignored private
renderers; the later checkpoint-only diagnostic/panic change passed its focused
parser test and `cargo check --locked`. Locked release builds succeeded.
Private raw evidence and audition files are in the ignored
`user/dsp-lab/20260722T151647Z/`; do not overwrite, stage, publish, or copy that
directory into tracked documentation.

The amplifier was confirmed off only for the completed connected tests in the
originating session; fresh physical work still requires fresh explicit safety
authorization. JACK was left running exactly as found at 48 kHz, 128 frames,
three periods, RT priority 95 on `hw:A96`. Starting and final snapshots both had
18 ports, zero connections, no SHR/synthv1 process, and identical routes. No
persistent audio configuration or tuning changed.

Connected release results were healthy during sustained processing:

- `soft-cubic`, 10.027 s: 3,810 callbacks, mean 53.416 us, p99 98 us,
  maximum 224.222 us, zero misses/oversized callbacks, owner/synth CPU
  3.09%/5.09%, owner/synth RSS 119,388/129,284 KiB.
- `phase4-full`, 20.050 s and eleven effects: 7,576 callbacks, mean
  437.601 us, p99 532 us, maximum 1,013.125 us, zero misses/oversized
  callbacks, owner/synth CPU 17.51%/5.44%, owner/synth RSS
  121,752/129,324 KiB, 1,860,804 bytes effect storage and 589,824 bytes graph
  buffers.
- The final five-second `soft-cubic` diagnostic had zero meter clips and
  non-finite samples, zero limiter reduction, 51.847 us mean, 96 us p99 and
  253.202 us maximum callback time.

The checkpoint now emits Unix-microsecond control-thread events and final-bus
meters. Timestamp evidence demonstrated that the old checkpoint sent the
48-message all-channel panic twice: explicitly before graph restore and again
inside `Engine::drop`. Removing only the duplicate explicit panic reduced an
otherwise identical five-second run from four teardown xruns to two without
changing routes, signal policy, timeout, or callback work. `Engine::drop`
remains the guaranteed All Notes Off/panic path.

Two transition xruns remain and both name `shs-synthv1`: the final run began
engine drop at Unix 1784739033026652 us; JACK reported the first miss at
2026-07-22 17:50:33.032472+01:00 and the second at
17:50:35.035115+01:00; engine drop completed at Unix 1784739035050091 us.
Thus one miss follows synth termination startup by about 5.8 ms and the other
lands at the existing two-second SIGTERM-to-forced-kill boundary. Do not hide
these events, narrow the journal window, weaken exact route restoration, or
extend/shorten the timeout speculatively. The next focused experiment should
establish whether synthv1 0.9.29 has a usable graceful JACK-client exit path;
the inspected upstream project is Rui Nuno Capela's `rncbc/synthv1`, GPL-2.0-
or-later, but no upstream code was copied. Preserve unowned synth processes.

## Publishing state

Public remote: <https://github.com/PaolaShultz/shr-daw>.
GitHub CLI is installed and authenticated as `PaolaShultz` over HTTPS. This
repository's local identity is `PaolaShultz` with GitHub's numeric no-reply
address. Keep those values; if authentication expires, use `gh auth login
--hostname github.com --git-protocol https --web` and let the user complete the
device flow. The repository is public, so apply the publishing boundary in
`AGENTS.md` before any requested commit or push.

## Private runtime and public packaging

The ignored `user/` tree is the private boundary for this checkout. The local
wrappers redirect XDG state/data, presets, and the loop inbox there. Important
roots are:

- `user/state/shsynth/`: runtime/controller configuration, backups, logs, and
  generated engine state;
- `user/data/shsynth/`: Ideas, Projects, demos, recordings, loops, loop inbox,
  and drum patterns;
- `user/presets/synthv1/`: cleared copies plus private/local presets;
- `user/downloads/`: private source archives.

Never replace this boundary with hardcoded Rust paths. Setup seeds only missing
cleared content and must preserve same-named user files. The only public
packaging authorities are `presets/synthv1/cleared-presets.txt`,
`loops/cleared-loops.txt`, and `demos/cleared-demos.json`.

The LinuxSynths archive at
`user/downloads/392Synthv1Patches.tar.gz` has SHA-256
`f4f9157cf5d245f7371a702584e28a90d1cf92b9a1eec9fa38c43fad584016ea`.
Its 392 files have no verified licence/authorship notice. They are available
for private use only and must never be committed, packaged, mirrored,
downloaded by the public installer, or described as MIT/public domain. Only the
21 manifest-cleared project presets are public and MIT. MusicRadar's optional
drum download is also private: its terms permit musical use but prohibit raw
sample redistribution.

## Current machine and hardware state

- The active development system is a Raspberry Pi 4 with 4 GB RAM and microSD.
  A Pi 5 with 2 GB RAM, active cooler, 27 W supply, bottom NVMe adapter, and
  128 GB NVMe was ordered but is not installed or measured. Keep Pi 4 evidence
  labelled accurately; migration and Pi 5 claims remain deferred to
  `docs/PI5_HEADROOM_PLAN.md` after a clean checkpoint.
- Local configuration selects the MiniLab 3 controller, JACK
  `system:playback_1`/`system:playback_2`, AudioBox USB 96 stereo capture on
  `system:capture_1`/`system:capture_2`, and the AudioBox MIDI port as external
  output. These are private configuration values, not portable defaults.
- The reviewed controller profile is `arturia-minilab-3`; controller and
  performance MIDI roles are separate. Its configured eight-pad layout uses
  four page pads plus four item pads; the master rotary browses content and its
  press selects/confirms. The Routing screen reports live visibility, not
  merely remembered configuration.
- The optional audio profile reserves CPU 3. Boot isolation is active; the
  performance-governor service and JACK affinity drop-in are installed. Inspect
  with `shr-audio-tune status`; removal requires the helper's managed removal,
  clearing `audio.engine_cpu`, and reboot. Never edit around its ownership
  records in `/var/lib/shr-audio-tune/`.
- The per-user `fluidsynth.service` and system `amidiminder.service` are masked
  and stopped. `/usr/bin/fluidsynth` and the TimGM bank remain for SHR-owned
  on-demand use. Setup and tuning do not start or restart JACK.
- All project equipment is borrowed. Preserve its configuration and require
  explicit approval before any JACK, synth, MIDI, recording, audible, or other
  physical-hardware test.

Rerun `scripts/setup-local.sh` only when the user requests configuration or
hardware/JACK names change. Read `docs/MAINTAINER_HELPERS.md` first.

## Decisions and open acceptance

- The competition build keeps the current bounded one-managed-source effects
  topology. The post-competition multi-strip/two-aux redesign stays in
  `docs/POST_COMPETITION_MIXER_AUX_PLAN.md`; hardware loops and full-duplex live
  input remain deferred until physical monitoring choices are made.
- `audio.graph.enabled` remains opt-in/default-false in local state. FX editing
  may validate and save routing while the graph is disabled, but only an active
  owned graph provides final metering/processing. Dated performance evidence
  belongs in the Phase 1–4 measurement documents, not here.
- The generic synchronized recorder and final stereo performance bus are
  implemented, but synthetic stress is not physical-interface or MR18
  acceptance. The first borrowed MR18 remained packed and produced no hardware
  evidence. Development and physical acceptance for simultaneous independent
  18-channel playback and 18-channel recording are deferred until the Pi 5
  clean-machine flow and the other working flows are ready; the next MR18 loan
  should span several days. Follow `docs/MR18_TEST_PLAN.md` before claiming a
  hardware pass or a checked release.
- The established tracked screenshot set now covers every normal controller
  context plus Home, MIDI Learn, and all master overlays. Keep its exact
  scenarios, font, 40×13 geometry, integer scaler, and validation contract in
  `docs/MAINTAINER_HELPERS.md`; do not hand-edit generated PNGs.

The open hands-on review is non-audible and must use a new empty FT2 Project.
Keep transport/recording stopped and do not attach routes. On the physical
40×13 TTY, verify the shared 38×11 overlay at `(1,1)`, its one-cell reveal,
launcher inside the bottom border, and uninterrupted final status row;
encoder/keyboard parity and wrap behavior; silent hidden launchers; two-step
Back behavior; ROUTE draft cancellation without Project mutation; the Loop
Library's inbox/private selection and return behavior; and every entered screen,
including an MTR FX caller return, starting on controller-menu page 1. Record
observed failures before changing behavior.

A later user-authorized musical/hardware pass should exercise the
standalone/FT2 synth ownership split, N00B versus Play/REC/Edit, independent
Edit length/ADD values, routing-default confirmation, and percussion smart
column reuse. Do not start that pass merely because the overlay review is
complete. Detailed UI contracts live in `docs/CONTROLLER_INTERFACE.md`,
`docs/TRACKER.md`, and the focused routing/effects documents linked from
`docs/README.md`.

## Installed tools and current validation boundary

Rust 1.85, `gh`, `xmllint` (`libxml2-utils`), and `shellcheck` are installed.
Use the scoped validation policy in `AGENTS.md`; historical full
suites, release builds, benchmarks, and screenshot batches are evidence in
their dated documents, not instructions to repeat them. No current physical or
audible acceptance should be inferred from synthetic or hardware-independent
checks.
