# Development story and OpenAI Build Week record

SHR-DAW is being prepared for the OpenAI Build Week Challenge. The official
[Build Week page](https://openai.com/build-week/),
[Devpost overview](https://openai.devpost.com/), and
[rules](https://openai.devpost.com/rules) were checked on 2026-07-18. The
submission deadline is July 21, 2026 at 5:00 PM Pacific Time. This is the
project's development story: it preserves the truthful timeline, working
method, human/AI boundary, and a reproducible snapshot without turning the main
README into a development journal. It is not an implementation specification.

SHR-DAW is a weekend/free-time side project, sometimes developed in parallel
with the creator's primary `bee247.hr` portal work. Its first public commit was
the moment the creator released the initial version and dedicated it to their
uncle, who died while the software was being released; it was not the beginning
of the code or of the Codex collaboration.

For eligibility, SHR-DAW is treated plainly as a pre-existing project. The
initial `4e779b55` commit is timestamped
`2026-07-13T16:31:23+01:00`, and the last pre-opening handoff commit,
`1dad8087`, is timestamped `2026-07-13T16:33:49+01:00`. The latter is the
comparison baseline. Only meaningful work after that baseline is presented as
Build Week development.

The development story is not only “Codex wrote Rust.”
[GPT-5.6](https://openai.com/index/gpt-5-6/) used through Codex also acted as a
technical and music-workflow navigator: it inspected
MIDI and audio topology, translated hardware actions into configuration,
helped reason about safe wiring and gain, organized presets and device data,
designed MIDI rhythms, designed synthv1 sounds, validated artifacts, and kept
licensing and private data out of the public repository.

## Model provenance for the first release

The creator's account is that all code leading to the first public commit was
developed with GPT-5.6 Sol through Codex CLI. A privacy-preserving review of the
local Codex CLI session metadata on the Raspberry Pi corroborates that account:
for this checkout, 144 recorded pre-commit turns across 12 local session files
span 2026-07-12 13:23 BST through 2026-07-13 16:30:53 BST. Every one of
those turn records names `gpt-5.6-sol`; none has a missing or different model
label. The final recorded turn is about 29 seconds before commit `4e779b55`.

This is evidence for the model used throughout the recorded pre-release Codex
work, not a claim that private platform logs establish line-by-line authorship.
Raw prompts, responses, Session IDs, and local log files remain private and are
not copied into the public repository.

## Division of work

The user supplied the musical goals, the available hardware, physical access,
and final taste. The user connected cables, moved controls when asked, and is
the authority for audible listening and whether a sound or groove is good.

The repository owner name `PaolaShultz` is the creator's gaming name and an
occasional online nickname, inspired by the empty tombstone used in the
buried-alive sequence in *Kill Bill: Volume 2*. It is not a company or another
contributor. This personal project is built by the same person who owns the
hardware, makes the musical/product decisions, and works primarily on the
`bee247.hr` portal outside this weekend/free-time project.

Codex CLI ran directly on the target Raspberry Pi throughout this work. Cargo
compilation, tests, warning-denied Clippy, and optimized release builds were
executed on the Pi itself. The source was not developed or cross-compiled on a
desktop PC and then deployed to the target; Codex-assisted editing, inspection,
compilation, QA, and release work all happened on the instrument's Raspberry
Pi. The creator also reports a working development session with two active
Codex CLI instances while SHR-DAW's managed synth was running. The workstation
remained operational in that observed session. This is useful qualitative
evidence of the real development workflow, not a measured CPU, latency, or
concurrency benchmark.

The creator reports that all Codex CLI work in this development push used the
literal `--yolo` invocation, with little terminal-screen reading or
command-by-command control. The human set the goal, supplied detailed durable
`AGENTS.md` constraints, reviewed meaningful checkpoints/outcomes, and retained
authority over hardware, audible judgment, destructive/system actions,
publishing, and product/music choices. This was a deliberate high-autonomy
workflow on an owned Pi and checkout, not a claim that unrestricted execution
is a safe default for other environments.

Codex performed or guided the following non-code work during development:

- inspected ALSA MIDI ports, JACK playback/capture ports, processes, preset
  locations, and existing configuration before changing routes;
- guided controller inspection one control at a time, including the 12 mapped
  controls, relative main encoder, encoder press, lock control, and command
  pads, then encoded the observed input mapping;
- selected and wrote machine-local MIDI input, MIDI output, stereo playback,
  and stereo capture settings while keeping device names out of Rust;
- documented the physical path from controller to SHR-DAW, software engines to
  JACK, and sound-card MIDI output to an external instrument, including safe
  gain and doubled-route warnings;
- discovered and organized synthv1, Yoshimi, FluidSynth, private preset, and
  external-device data without publishing the uncleared preset archive;
- authored 72 reusable MIDI drum grooves across ten genre groups, 3/4 and 4/4,
  with kick, snare/clap, hat, percussion, velocity, and gate choices;
- began with the authored `Velvet Tines` synthv1 sound and expanded the cleared
  bank with 20 original parameter designs for basses, leads, pads, plucks,
  bells, organs, drones, and effects. All are XML/schema validated; the
  20-preset expansion still needs an authorized listening and curation pass;
- researched and structured named external-instrument program data, diagnosed
  setup constraints, ran non-audible checks, maintained documentation, and
  prepared public/private and licensing boundaries.

This record is based on the user's account, Git history, tracked artifacts, and
`docs/WORKSPACE_HANDOFF.md`. It does not claim that Codex physically connected
hardware or heard audio. Those actions require the user.

## Development snapshot

Snapshot date: 2026-07-18, including the audit fixes and submission
documentation prepared for this release.

| Measure | Snapshot value | Consistent counting rule |
| --- | ---: | --- |
| Rust physical LOC | 24,165 | `find src -name '*.rs' -print0 \| xargs -0 wc -l` |
| Rust source modules | 22 | `.rs` files below `src/` |
| Source test functions | 251 | `#[test]` annotations below `src/` and `tests/` |
| Git commits | 35 | commits reachable after publishing this release |
| Active development dates | 4 | unique commit dates; a lower bound on sessions |
| Cleared synthv1 presets | 21 | public packaging allowlist |
| Bundled drum patterns | 72 | 60 compact-catalog plus 12 standalone patterns |
| User/developer Markdown guides | 19 | `.md` files directly below `docs/` |
| Tracked README visuals | 12 | PNG/JPEG files directly below `docs/images/` |

The major subsystem inventory is maintained in
[`BUILD_WEEK_FEATURE_MATRIX.md`](BUILD_WEEK_FEATURE_MATRIX.md), where every row
also names configuration, persistent data, offline behavior, safety rules,
architecture, tests, and the best demonstration shot. Counts are inventory
checks rather than proof of product quality.

Historical bug counts and session-type counts were not recorded consistently,
so they are not invented here. Detailed working records remain private rather
than turning the product documentation into a submission-planning archive.

## Pre-existing baseline and Build Week extensions

The pre-opening SHSynth baseline already included a 40×20 terminal, one-managed
engine hosting for synthv1/Yoshimi/FluidSynth, sound browsing, pickup-safe
synthv1 controls, MIDI Ideas, an initial external Casio tracker, stereo JACK
capture, setup scripts, the MiniLab controller workflow, and 21 public cleared
presets. This was substantial prior work and is not relabelled as a Build Week
result.

The dated feature diffs, rather than raw change volume, are the evidence:

- **2026-07-14:** SHR-DAW product framing, configurable tracker pages,
  controller auto-detection/non-audible learn, external device profiles, live
  FT2 recording and modes, private WAV-loop playback, and local web help.
- **2026-07-16:** wider Pattern and Arrangement architecture, loop beat/tempo
  alignment without time-stretching, per-column program/channel routing,
  Project-storage hardening, controller navigation, and 40×20 presentation
  screenshots.
- **2026-07-18:** expanded rhythm editing, reusable drum-pattern workflow, and
  72 authored grooves, followed by safety/content auditing, Raspberry Pi
  validation, and submission preparation.
- **2026-07-18/19:** an opt-in owned audio graph progressed from a measured dry
  path through bounded source inserts, time/modulation effects, reverb, two aux
  returns, master processing, and a passive CPU/final-output meter. Exact
  topology, measurements, and the still-open musical curation sheet remain in
  the audio-graph and phase documents rather than this narrative.

The detailed audit ledger, submission copy, video script, fallback, checklist,
and working journal remain below the ignored private `user/build-week/` tree.
They are not installed or published as product documentation.

## Presentation priorities

The clearest near-term story uses what already works:

1. configure a controller and routes without hard-coded hardware;
2. choose or shape a sound with pickup-safe controls;
3. load or edit a drum groove and record a melodic idea;
4. arrange Patterns, add a private loop if useful, and record the stereo result;
5. show the same workflow at 40x20 with the physical controller.

The selected remaining work is an audible curation of the 21 presets and drum
shortlist, one short original demo Project, one verified stereo take, and a
public end-to-end video below three minutes. These human listening, performance,
and publishing tasks are submission blockers; they are not marked complete in
advance.

## Continuing development

The README intentionally stops at the current product boundary. Deferred ideas
and their safety requirements live in
[`FUTURE_IMPROVEMENTS.md`](FUTURE_IMPROVEMENTS.md). The selected
post-competition work is split into executable [mixer/aux](POST_COMPETITION_MIXER_AUX_PLAN.md)
and [rhythm](POST_COMPETITION_RHYTHM_PLAN.md) plans. This record should grow
only when there is a meaningful development event to preserve; feature manuals,
benchmark tables, and speculative implementation detail belong in their
dedicated documents.
