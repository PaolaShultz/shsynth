# Documentation index

README is the short product overview. This index routes each kind of detail to
one maintained home so setup instructions, implementation contracts,
measurements, history, and future plans do not become repeated walls of text.

## Start and use

- [First run](FIRST_RUN.md) — configure hardware and open SHR-DAW.
- [Using SHR-DAW](USING_SHR_DAW.md) — instruments, screens, MIDI Ideas,
  performance meters, recording, and commands.
- [Complete screen and menu manual](MENU_MANUAL.md) — every populated 40×20
  screen, contextual editor, and controller-menu page with explanations.
- [In-app help](HELP.md) — the compact help text shown by `?` or F1.
- [Tracker guide](TRACKER.md) — FT2 editing, pages, routing, Arrangement,
  drums, loops, and Project files.
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

- [How SHR-DAW works](HOW_IT_WORKS.md) — concise ownership, MIDI, pickup,
  recording, audio-graph, and data boundaries.
- [Audio graph and DSP contract](AUDIO_GRAPH.md) — persisted graph model,
  real-time limits, routing publication, effects, and curation gates.
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
- [Workspace handoff](WORKSPACE_HANDOFF.md) — current checkout, local hardware,
  private/public boundary, and validation state for maintainers.

`WORKSPACE_HANDOFF.md` describes one development machine and is not an end-user
setup guide.

## Planned work

- [Future improvements](FUTURE_IMPROVEMENTS.md) — deferred routing and product
  ideas, including the deliberately unreasonable challenges.
- [Post-competition mixer and aux plan](POST_COMPETITION_MIXER_AUX_PLAN.md) —
  multi-strip mixer and shared-aux migration.
- [Post-competition rhythm plan](POST_COMPETITION_RHYTHM_PLAN.md) — arbitrary
  Pattern length, microtiming, swing, groove tools, and optional formal meter.
