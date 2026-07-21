# Development story and OpenAI Build Week record

> Historical record: dates, counts, submission language, and measured state in
> this document are frozen to the stated Build Week snapshot. They are not the
> current repository inventory; use the documentation index and source for
> current behavior.

SHR-DAW is being prepared for the OpenAI Build Week Challenge. The official
[Build Week page](https://openai.com/build-week/),
[Devpost overview](https://openai.devpost.com/), and
[rules](https://openai.devpost.com/rules) were checked on 2026-07-18. The
submission deadline is July 21, 2026 at 5:00 PM Pacific Time. This is the
project's development story: it preserves the truthful timeline, working
method, human/AI boundary, and a reproducible snapshot without turning the main
README into a development journal. It is not an implementation specification.

## Why SHR-DAW exists

SHR-DAW began with a much smaller goal: play a few software synthesizers on a
Raspberry Pi without turning it into a desktop computer. The creator explored
existing music-focused systems, including Zynthian and Patchbox OS. The
Zynthian setup they considered centered on dedicated control hardware that did
not fit comfortably beside the GPIO display and four rotary controls they
already wanted to use. Designing more custom hardware was also not where they
wanted to spend their limited time.

Patchbox OS provided the audio-focused base, but not the exact instrument. The
experiment moved into a small terminal interface, avoiding a general desktop
GUI and unnecessary background weight on the Pi's four ARM64 cores. Several
synth engines were tried on the real hardware; some worked readily, some did
not, and the three reliable choices became the foundation of SHR-DAW's synth
support.

Once playing sounds worked, a new question appeared: could the same little box
provide an accompaniment? An FT2-style pattern editor fit the compact terminal
and physical-control idea naturally. Patterns led to pages, routing,
Arrangement, loops, recording, and effects—not as a plan to imitate a full
desktop DAW, but as a sequence of useful musical needs.

### The creator's explore-first learning idea

The learning angle is a personal product belief, not a factual claim that every
child or beginner learns in one preferred way. In the creator's words:

> Some children do not want their first contact with music to be a theory book.
> They want to press something, hear what happens, change it, and make their own
> conclusion. A child exploring around C-sharp minor can use Playback to see the
> exact sounding notes lit on the keyboard, the notes that remain unplayed, and
> the chord name. They might discover `C#m`, move or add a note, see another
> name appear, and ask why `E maj` or `A maj` still feels connected. The program
> does not need to declare an answer right or wrong. It can give the discovery a
> sound, a visible shape, and a name, so theory later answers a question the
> child already owns.
>
> N00B lets the player choose a root and major or natural-minor scale. Notes in
> it sound and notes outside it stay silent, both in Playback and on a melodic
> FT2 page. In FT2 it is an independent switch: the same filter can stay on
> while playing, recording, or using Step Edit. Step Edit separately gives the
> allowed recorded tones a familiar note length. Someone can carefully enter a
> phrase or practically face-smash the keys, listen, keep an accident that
> sounds good, and revise it. That is useful for a young explorer, a complete
> music beginner, a Raspberry Pi enthusiast, or an older person returning to
> learning—like me. N00B names the optional filter, not the person using it.

The intended path is curiosity first: press, hear, see the sounding notes, read
the chord name, change something, compare, and ask why. Playback supplies the
immediate musical mirror; FT2 turns discoveries into repeatable Patterns. This
does not replace theory or claim measured educational results. It explains why
beginner readability, visible names, compact feedback, and fast experimentation
are central product requirements rather than incidental extras.

The OpenAI Build Week email arrived late in that process. SHR-DAW was not
started for the competition. After reading the rules, the creator treated it
openly as a pre-existing project and used the last pre-opening handoff as the
baseline. The late decision brought a different challenge: decide quickly what
could be completed and demonstrated before the deadline, what would be
overreach, where to freeze the submission, and what should wait for
post-competition development.

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

## The creator's programming background

SHR-DAW is not the creator's first encounter with programming, but it is their
first Rust and Raspberry Pi project. Their early experience was mainly assembly
language on the Commodore 64 and Apple IIc. They later programmed on PCs and
fell in love with Delphi, which was their last sustained general-purpose
programming environment. In the years after that they worked mostly with SQL,
apart from a few small sketches in tools such as Lazarus. This background
predates Git and today's common collaborative, build, and release workflows.

Before SHR-DAW, the creator had never owned or programmed a Raspberry Pi and
had never written Rust; their prior exposure to Rust was limited to briefly
glancing at—and liking—some syntax. All hardware used for hands-on SHR-DAW
testing is borrowed from friends. The honest description is therefore an
experienced returning programmer learning an unfamiliar language, platform,
toolchain, and hardware workflow with Codex—not someone with no programming
history, and not someone arriving with prior Rust or Raspberry Pi expertise.

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
contributor. This personal project is built by the same person who arranges
access to the borrowed hardware, makes the musical/product decisions, and works
primarily on the `bee247.hr` portal outside this weekend/free-time project.

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
workflow on a borrowed Pi made available for the project and a
creator-controlled checkout, not a claim that unrestricted execution is a safe
default for other environments.

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
documentation prepared for that release. Later features, documents, tests,
commits, and the 80-image visual manual are intentionally not folded into this
historical table.

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
and [rhythm](POST_COMPETITION_RHYTHM_PLAN.md) plans. A separate
[Pi 5 Headroom plan](PI5_HEADROOM_PLAN.md) records another unscheduled future
pass; its hardware is not present and none of its measurements or changes is
part of the Build Week submission. That work begins only after the deadline.

The hardware choice is part of the human development story even though its
benchmarks are not. With memory and NVMe prices climbing, server replacement
cycles also leave many unfashionably small 128 GB NVMe drives looking for a
new job—sometimes around EUR 5 when a buyer is lucky. For SHR-DAW, that old
server size is generous. The ordered drive cost EUR 15, and the complete Pi 5
2 GB, active-cooler, 27 W supply, bottom-NVMe package cost about EUR 120. The
adapter goes below because the GPIO screen owns the top; a measured, angled
printed enclosure is planned after the parts arrive.

The running `biiiig cache` thought comes from the creator's old 1 KB-demo
instinct: modern hardware offers an enormous budget compared with the tiny
procedural tunnels, space scenes, and music once squeezed into a kilobyte, so
every dependency and retained byte is still fair to question. It is not a
claim that the whole DAW will live permanently in L1. The future pass will
measure the hot audio working set, core isolation, library features, build
cost, effect state, and callback timing before deciding what to remove or mark
as especially light.

This record should grow only when there is a meaningful development event to
preserve; feature manuals, benchmark tables, and speculative implementation
detail belong in their dedicated documents.
