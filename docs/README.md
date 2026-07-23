# SHR-DAW documentation

README is the short product overview. This index routes each kind of detail to
one maintained home so setup instructions, implementation contracts,
measurements, history, and future plans do not become repeated walls of text.

For current behavior, begin in **Start and use**, then follow implementation
details into **Install and configure** or **Architecture and safety**.
Measurement pages are dated evidence from particular checkpoints, not a second
current specification. Development records preserve what was true during the
build. Planned-work pages describe proposals and do not override current code,
configuration, or the architecture contracts.

## Start and use

- [First run](FIRST_RUN.md) — configure hardware and open SHR-DAW.
- [Using SHR-DAW](USING_SHR_DAW.md) — instruments, screens, MIDI Ideas,
  source/aux/master effects, performance meters, recording, and commands.
- [Complete screen and menu manual](MENU_MANUAL.md) — 95 deterministic images
  covering every populated 40×13 screen, contextual editor, and controller
  menu page, with explanations.
- [In-app help](HELP.md) — the compact help text shown by `?` or F1.
- [Tracker guide](TRACKER.md) — FT2 editing, pages, routing, Arrangement,
  drums, loops, and Project files.
- [Public-domain demo songs](DEMO_SONGS.md) — tempos, keys, parts, restyle
  ideas, clearance records, and installed discovery.
- [Controller interface](CONTROLLER_INTERFACE.md) — complete four-page action
  inventory and hardware navigation contract.
- [Physical connections](CONNECTIONS.md) — MIDI and audio wiring examples.

## Install and configure

- [Installation](INSTALLATION.md) — dependencies, install/uninstall boundaries,
  repository-local evaluation, JACK, and optional CPU tuning.
- [Configuration and routing](CONFIGURATION.md) — every runtime setting and
  persisted route.
- [MIDI device profiles](MIDI_DEVICE_PROFILES.md) — external-instrument bank
  and program names.
- [Controller profiles](CONTROLLER_PROFILES.md) — automatic matching and the
  non-audible MIDI learner.
- [Codex-assisted setup](CODEX_ASSISTED_SETUP.md) — optional assistance for
  unusual hardware and recovery.

## Architecture and safety

- [How SHR-DAW works](HOW_IT_WORKS.md) — end-to-end MIDI/audio routing,
  instrument ownership, Ideas, FT2 Projects, loops, recording, the full effect
  palette, graph safety, persistence, and honest current limits.
- [Audio graph and DSP contract](AUDIO_GRAPH.md) — Project effects data, exact
  parameter schemas, real-time limits, routing publication, meters, bypass,
  tails, topology limits, and curation gates.
- [Final stereo performance bus](FINAL_PERFORMANCE_BUS.md) — exact three-source
  topology, limiter, monitoring safety, final WAV capture, and hardware acceptance.
- [Synchronized multitrack recording](MULTITRACK_RECORDING.md) — exact JACK
  source mapping, shared callback timeline, mono stems, manifests, recovery,
  and the non-audible stress helper.
- [MR18 acceptance plan](MR18_TEST_PLAN.md) — readiness gates and a printable,
  safety-first 18×18 full-duplex 48 kHz hardware procedure.
- [Three-minute multitrack presentation](MULTITRACK_PRESENTATION.md) — truthful
  hardware and synthetic versions with exact on-screen evidence.
- [Third-party software and sounds](../THIRD_PARTY.md) — licences, credits,
  provenance, and redistribution rules.
- [New patches and sounds](NEW_PATCHES.md) — synthv1 schema and authoring
  workflow.

## Measurements and audits

- [Phase 1 dry graph](PHASE1_AUDIO_GRAPH_MEASUREMENT.md) — owned-routing and
  bit-exact fallback checkpoint.
- [Phase 2 insert effects](PHASE2_AUDIO_GRAPH_MEASUREMENT.md) — deterministic
  processor evidence and Raspberry Pi measurements.
- [Phase 3/4 effects and buses](PHASE3_4_AUDIO_GRAPH_MEASUREMENT.md) — time and
  modulation effects, reverb, aux/master routing, and consolidated curation.
- [Preset audit](PRESET_AUDIT.md) — cleared public synthv1 bank review.
- [Drum-pattern audit](DRUM_PATTERN_AUDIT.md) — bundled rhythm structure,
  limitations, and listening shortlist.

## Development record

- [Maintainer helper scripts](MAINTAINER_HELPERS.md) — parameters, side
  effects, safety boundaries, and design decisions for every repository helper
  and the related Make targets.
- [Development story and Build Week record](BUILD_WEEK.md) — chronology,
  model provenance, division of work, target-native workflow, and evaluation.
- [Feature and quirk matrix](BUILD_WEEK_FEATURE_MATRIX.md) — subsystem-level
  access, persistence, failure, safety, architecture, and test inventory.
- [Workflow audit and repair handoff](WORKFLOW_AUDIT_HANDOFF.md) — complete
  musician/operator workflow coverage, first repair queue, deferred decisions,
  evidence gaps, and persistent fix tracking.
- [Workspace handoff](WORKSPACE_HANDOFF.md) — current checkout, local hardware,
  private/public boundary, and validation state for maintainers.

`WORKSPACE_HANDOFF.md` describes one development machine and is not an end-user
setup guide.

## Planned work

- [Release roadmap](RELEASE_ROADMAP.md) — ordered 0.4, 0.5, and 0.6 acceptance
  gates and the rule that keeps unrelated ideas out of the current milestone.
- [Future improvements](FUTURE_IMPROVEMENTS.md) — deferred routing and product
  ideas, including the deliberately unreasonable challenges.
- [Raspberry Pi 5 headroom and footprint plan](PI5_HEADROOM_PLAN.md) —
  release 0.4 Raspberry Pi OS Lite acceptance followed by the later
  dependency/footprint, real-time-core, and PRESTO experiments.
- [Post-competition mixer and aux plan](POST_COMPETITION_MIXER_AUX_PLAN.md) —
  multi-strip mixer and shared-aux migration.
- [Post-competition rhythm plan](POST_COMPETITION_RHYTHM_PLAN.md) — arbitrary
  Pattern length, microtiming, swing, groove tools, and optional formal meter.
