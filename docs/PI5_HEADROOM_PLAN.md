# Raspberry Pi 5 headroom and footprint plan

> The clean Raspberry Pi OS Lite installation and setup acceptance is a release
> 0.4 gate. The later headroom, dependency, PRESTO, and optimization experiments
> remain unscheduled future work. Nothing here describes measured Raspberry Pi
> 5 performance while the ordered hardware is absent.

## Purpose

Release 0.4 uses the new Pi first to prove a reproducible clean installation,
setup, and return from failures on normal Raspberry Pi OS Lite. That platform
gate does not require or assume an optimization. Only after it passes does the
future Headroom pass compare and tune performance.

SHR-DAW grew experimentally by adding useful musical capabilities. The future
**Headroom pass** deliberately reverses direction for a while: stop adding,
measure what exists, remove unnecessary weight, improve real-time locality, and
make inexpensive effect choices visible without dividing the product into
different racks.

Headroom has both meanings here. It is spare audio level before clipping and
spare processing time before a JACK deadline. The work should:

- keep the real-time audio instructions and active state warm on an isolated
  core where the platform and JACK topology permit it;
- reduce callback jitter, migrations, cache contention, and worst-case time;
- identify effects with genuinely small state and low measured callback cost;
- reduce dependency features, binary footprint, clean-build work, and local
  Cargo storage where evidence justifies it;
- retain one effects rack and every existing sound choice; and
- preserve engine ownership, All Notes Off, recording publication, direct
  fallback, and the other live-audio safety boundaries.

This is not a challenge to force the complete executable below an arbitrary
cache size. Cold setup, XML, JSON, screenshot, and file-management paths do not
need to remain in CPU cache while the audio callback runs. The useful target is
the hot working set: the instructions and data repeatedly touched together by
the active callback.

## Planned development hardware

The ordered package is a Raspberry Pi 5 with 2 GB RAM, active cooler, 27 W
power supply, a bottom-mounted PCIe-to-NVMe adapter, and a 128 GB NVMe drive.
The complete package cost about EUR 120; the small NVMe cost EUR 15. Neither
price is a product requirement or an evergreen buying guide.

The 2 GB model was chosen deliberately for a CLI/TUI appliance rather than a
desktop workload. That choice is a hypothesis until the compiler, linker,
Codex CLI, JACK, synth, graph, recorder, and normal OS memory peaks have been
measured together. Do not publish 2 GB as a minimum or recommended capacity in
installation documentation before that acceptance passes.

The NVMe adapter sits below the Pi because the GPIO display needs the space
above it. A future printed enclosure should angle the display toward the
player, retain cooler airflow, avoid cable strain, expose the required
connectors, and keep the drive serviceable. Exact dimensions and CAD work wait
until the physical board, adapter, drive, cooler, and screen can be measured.

Begin the release 0.4 platform path from a fresh official 64-bit Raspberry Pi
OS Lite image and record its exact filename, release, architecture, checksum,
kernel, firmware, and first-boot choices. Do not install Patchbox OS or copy the
Pi 4 root filesystem as a shortcut.

The Raspberry Pi 5 CPU provides private per-core L1 and 512 KB L2 caches plus a
2 MB shared L3 cache. Reserving a core cannot reserve the shared L3, but it can
leave that core's private caches largely to the real-time audio threads. Cache
lines are not cleared between JACK callbacks merely because the thread sleeps;
the expected benefit is warmer code/state and less worst-case jitter when
ordinary work and interrupts stay elsewhere. This remains a hypothesis to
measure, not a performance claim. See the official
[Raspberry Pi 5 product brief](https://pip-assets.raspberrypi.com/categories/892-raspberry-pi-5/documents/RP-008348-DS-1-raspberry-pi-5-product-brief)
and [Arm Cortex-A cache comparison](https://developer.arm.com/-/media/Arm%20Developer%20Community/PDF/Cortex-A%20R%20M%20datasheets/Arm%20Cortex-A%20Comparison%20Table_v4.pdf?hash=C816A56372483062F01ABFCFB500CDAF46CD82B3&revision=7c836998-353a-4601-80c3-d0f76021ae17).

## Measurement sequence

Use one committed source revision for the comparison. Begin only after the Pi 5
arrives, concurrent feature work has reached a clean checkpoint, and the user
explicitly requests the combined build-and-test pass. Do not clean, replace, or
repurpose the active checkout's Cargo cache; use a separate exact ignored
directory below `user/` as `CARGO_TARGET_DIR` for controlled clean builds.

Before headroom experiments, treat the Pi 5 as a genuinely new machine and run
the complete documented installation and local-setup path from its starting
state. Do not copy the Pi 4 installation, runtime configuration, tuning state,
or Cargo artifacts as a substitute. Record each required dependency, restart,
configuration decision, failure, retry, and successful return to the main
path. This clean-machine acceptance is a prerequisite for the next multi-day
MR18 loan and its [18×18 full-duplex acceptance](MR18_TEST_PLAN.md), not part of
the borrowed-hardware session itself.

### 1. Raspberry Pi 4 baseline

Record the current machine as it really is: board revision, RAM, microSD,
kernel, OS, Rust toolchain, power/cooling, governor, isolation settings, JACK
version, sample rate, period, and periods per buffer. Then record:

- clean and warm incremental formatting, test, warning-denied Clippy, and
  release-build wall time;
- peak RSS, available memory, swap activity, OOM events, final binary sections,
  and Cargo target size;
- storage throughput relevant to builds and recorder stems;
- temperature, clock, throttling, and power warnings; and
- audio callback mean, p95, p99, maximum, missed deadlines, oversized
  callbacks, xruns, and core migrations for the existing reference profiles.

### 2. Raspberry Pi 5 platform acceptance

Repeat the same committed revision, commands, profiles, rates, and callback
sizes on the NVMe-backed Pi 5. First use the supported PCIe mode and a clean
machine configuration; do not copy Pi 4 boot isolation files or tool-owned
system settings blindly. Install and inspect audio tuning on the new machine as
a new operation.

First measure the clean system without optional SHR tuning. Compare the Pi 4
state and results, then apply the managed audio profile only if the new kernel,
boot layout, and measured need justify it. Use `shr-audio-tune`; do not
reproduce its owned boot tokens, service files, or affinity settings by hand.

Record the same build, memory, storage, thermal, power, and callback results.
If Linux exposes trustworthy PMU counters, also record instruction, data, L2,
and last-level cache misses. If a counter is unsupported or ambiguous, say so
rather than substituting an estimate.

### 3. Real-time placement experiment

Compare at least:

1. normal scheduling;
2. the supported isolated-audio-core profile; and
3. specific placement of SHR's graph callback thread with the TUI, file writer,
   normal interrupts, and ordinary processes kept on other cores.

The target is an isolated set of real-time audio threads, not a misleading
claim that the JACK server alone produces all client audio. Account for JACK,
the managed synth, SHR's graph callback, and any other active JACK client.
Compare p99/maximum duration and migrations as carefully as average time.

No audible or physical audio test is implied by this plan. Use existing
non-audible paths first; JACK, synth, interface, listening, or hardware-loop
tests still require the user's explicit authorization.

## Dependency and binary-footprint audit

Run this work after the platform baselines so an optimization cannot erase the
comparison it is meant to inform.

1. Produce crate-level and function-level release-size reports, a linker map,
   the complete feature graph, clean-build timings, and per-crate build cost.
2. Test unnecessary default features individually. Initial questions include
   Crossterm's bracketed-paste default and Signal Hook's channel/iterator
   defaults; they are candidates to measure, not predetermined removals.
3. Inspect Ratatui/layout/Unicode, Serde/JSON, Quick XML, Hound, MIDI/ALSA, and
   error-handling contributions without assuming that a large source package
   produces an equally large linked result.
4. Try a smaller library, fixed 40×13 implementation, or separate live/offline
   binary only in an isolated branch or worktree and only when the measured
   contributor is large enough to justify the experiment.
5. Compare capability, correctness, clean-build time, incremental-build time,
   binary sections, runtime memory, and callback timing. Size alone does not
   choose the winner.

The existing release profile already uses LTO, one code-generation unit, and
symbol stripping, allowing LLVM and the linker to remove unreachable code.
Unused dependencies and features can still waste compilation, disk, review,
and supply-chain surface even when they barely change the executable. Record
those gains honestly.

Do not adopt `panic=abort` merely to shrink unwind data. A smaller program does
not justify bypassing All Notes Off, owned-engine shutdown, route restoration,
or recording cleanup.

## One rack and the PRESTO mark

There will be one effects rack. Do not create a separate fast, light, gaming,
or performance rack and do not hide time-based effects from the normal list.

The proposed compact mark is `»`, named **PRESTO** in Help and documentation.
The installed `Lat15-VGA16` console font and deterministic screenshot renderer
map this glyph, it occupies one cell, and it does not reuse SHR's existing `▶`
play or `●` record symbols.

An eventual explanation should remain plain:

> `» PRESTO` marks an effect with measured low callback cost and small
> persistent state. It predicts processing headroom, not sound quality.

Before assigning the mark, publish fixed acceptance thresholds derived from
the reference baseline. Test every effect at supported rates, 64- and
128-frame callbacks, parameter extremes, rapid movement, bypass, and repeated
instances. Record at least:

- persistent state bytes per instance;
- active data touched per callback where it can be measured meaningfully;
- added mean, p95, p99, and maximum callback time over the matched baseline;
- cache/core behavior when trustworthy counters exist;
- finite output, discontinuity, deadline, and xrun results; and
- the repeated-instance capacity on both the Pi 4 and Pi 5 reference systems.

The mark is expected to suit processors with tiny fixed state, such as gain,
EQ, dynamics, distortion, filtering, crushing, and some modulation, but the
measurement decides. Long delay histories, chorus/flanger histories, and
reverbs are likely to remain unmarked even when they perform comfortably.
Unmarked means only that an effect spends more retained state or callback
headroom; it is not a warning and says nothing about musical quality.

The FX rack may show the one-cell mark beside a kind and explain it in the
status/help path. Any color must remain secondary to the glyph and must not
conflict with existing green/yellow/red parameter meaning. The complete 40×13
layout and deterministic screenshot set must pass before the mark ships.

## Release and documentation gates

The Headroom pass may enter current documentation only after its behavior and
evidence exist. Until then:

- this plan and short links to it remain under **Planned work**;
- old Raspberry Pi 4 measurement pages remain unchanged dated evidence;
- installation docs do not claim Pi 5 or 2 GB acceptance;
- `AUDIO_GRAPH.md`, Help, the visual manual, and musician guides do not describe
  PRESTO as available; and
- the root README does not advertise the planned work as a feature.

Publish later Pi 4/Pi 5 results in a separate dated measurement document, not
by converting hypotheses in this plan into unlabelled facts. Any Rust, Cargo,
runtime, helper, installer, or behavior change still requires the full locked
Rust validation and relevant real-time acceptance. Documentation-only planning
changes use the repository's targeted documentation checks.
