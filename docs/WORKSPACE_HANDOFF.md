# Workspace handoff

This file contains only current machine state and decisions that must survive a
new thread in `/home/patch/p/shsynth`. Durable repository policy is in
`AGENTS.md`; detailed helper behavior is in `docs/MAINTAINER_HELPERS.md`. Never
record credentials, GitHub device codes, or private file contents here.

## Current priority and shared checkout

The competition fast-iteration policy in `AGENTS.md` is active. The complete
deterministic documentation screenshot set is reconciled to the current UI;
physical approval remains the next gate for UI/controller work.

Multiple workers use this checkout and commit their own changes independently.
Branch tips, commit messages, and clean/dirty snapshots are intentionally not
recorded here. Inspect live Git state, preserve concurrent work, commit only
your own scope, and do not wait for unrelated workers to finish; follow the
canonical collaboration rule in `AGENTS.md`.

Plain `shr` resolves to this checkout's `scripts/local.sh` through both
`/home/patch/.bash_aliases` and `/home/patch/.local/bin/shr`. The launcher uses
`target/debug/shr` unless `SHSYNTH_BIN` is explicitly set; the debug TUI shows
`DEV`. Do not restore the obsolete release-binary alias.

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
  acceptance. Follow `docs/MR18_TEST_PLAN.md` before claiming a hardware pass.
- The established tracked screenshot set now covers every normal controller
  context plus Home, MIDI Learn, and all master overlays. Keep its exact
  scenarios, font, 40×20 geometry, integer scaler, and validation contract in
  `docs/MAINTAINER_HELPERS.md`; do not hand-edit generated PNGs.

The open hands-on review is non-audible and must use a new empty FT2 Project.
Keep transport/recording stopped and do not attach routes. Verify the shared
38×18 overlay at `(1,1)`, its caller reveal and single highlighted launcher;
encoder/keyboard parity and wrap behavior; silent hidden launchers; two-step
Back behavior; ROUTE draft cancellation without Project mutation; the Loop
Library's inbox/private selection and return behavior; and every entered screen,
including an MTR FX caller return, starting on controller-menu page 1. Record
observed failures before changing behavior.

A later user-authorized musical/hardware pass should exercise the
standalone/FT2 synth ownership split, N00B versus Play/REC/Edit, independent
Step Edit length/ADD values, routing-default confirmation, and percussion smart
column reuse. Do not start that pass merely because the overlay review is
complete. Detailed UI contracts live in `docs/CONTROLLER_INTERFACE.md`,
`docs/TRACKER.md`, and the focused routing/effects documents linked from
`docs/README.md`.

## Installed tools and current validation boundary

Rust 1.85, `gh`, `xmllint` (`libxml2-utils`), and `shellcheck` are installed.
Use the scoped competition validation policy in `AGENTS.md`; historical full
suites, release builds, benchmarks, and screenshot batches are evidence in
their dated documents, not instructions to repeat them. No current physical or
audible acceptance should be inferred from synthetic or hardware-independent
checks.
