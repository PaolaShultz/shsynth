# Audio graph and DSP contract

This document is the implementation contract for SHR-DAW's owned audio graph
and lightweight effects rack. It records the stable model and real-time limits
before the graph becomes responsible for playback. The owned client now compiles
the managed-source, aux-return, and master racks but remains disabled by default. Its first
authorized Raspberry Pi dry-path checkpoint passed; direct JACK routing remains
the default and conservative fallback. See
[Phase 1 dry audio graph measurement](PHASE1_AUDIO_GRAPH_MEASUREMENT.md).

## Ownership boundary

One SHR-owned JACK client will contain the stereo graph. Effects are internal
processors, not separate JACK clients or processes. JACK ports exist only at
the graph boundary:

- inputs from the managed engine, loop player, configured live capture, and
  configured hardware returns;
- outputs to configured main playback, hardware sends, and recording taps; and
- exact physical port names supplied by configuration, never compiled into
  Rust.

The graph may connect and disconnect only SHR-owned endpoints. It must not
alter unrelated JACK connections or terminate a client/process it does not
own. Until the graph is activated successfully, the existing engine, loop, and
recorder routes remain the conservative fallback.

For the Phase 1 managed-engine path, direct playback is connected first. The
owned callback publishes silence while its four boundary links are prepared;
the two direct links are then removed in the same rollback-capable transaction.
Only after that commit does an atomic flag publish dry graph output at a block
boundary. A rejected graph, activation failure, ambiguous engine-output pair,
or connection failure leaves or restores the exact direct topology. The loop
player remains on its existing direct route for this checkpoint.

On normal shutdown, the publish flag is cleared and JACK deactivation joins the
owned callback before either direct link is restored. Closing that client then
releases only its registered ports and their graph-boundary connections. This
ordering prevents a final already-started graph block from overlapping the
restored direct path.

`src/jack.rs` is the shared dynamic-loading and lifetime boundary for current
and future JACK clients. A caller owns its callback allocation and keeps it
alive until client deactivation returns.

## Persisted model

The initial graph document has its own format version in addition to the
Project format. IDs are stable non-zero integers and are never inferred from
array positions. Unknown newer graph or effect versions are rejected; a failed
load must not be written back over the Project.

Each graph stores:

```text
format_version
enabled
nodes[] { id, kind, channel_layout, configuration }
edges[] { id, from_node, from_port, to_node, to_port }
effects[] { instance_id, kind, version, bypass, parameters-by-name }
source_chains[] { source_id, ordered effect instance IDs }
master_chain { ordered effect instance IDs }
aux_buses[] { id, ordered effect instance IDs, return_gain }
sends[] { source_id, aux_id, level, pre_or_post }
monitoring_mode
recording_tap
```

Effect parameters use stable names and physical units rather than positional
arrays. Interactive controls clamp to their visible range. Persisted
parameters must already be finite and valid or the whole proposed graph is
rejected.

Project format 3 stores a managed-source `InsertRack` and `ProjectAuxRouting`
as strict JSON inside the versioned `.shsong` line format. Formats 0 and 1
migrate to an empty rack and routing; format 2 keeps its source rack and adds
empty aux/master routing.
Unknown current fields, malformed rack data, and newer Project/effect versions
are refused on load and on overwrite. Rack order is a separate list of stable
effect IDs, so moving an effect does not recreate its identity.

### Typed nodes

Source node kinds are `ManagedEngine`, `LoopPlayer`, `LiveInput`, and
`HardwareReturn`. Processor kinds are `Utility`, `Eq`, `Compressor`,
`Distortion`, `Delay`, `Reverb`, `Chorus`, `Flanger`, `Phaser`, `TremoloPan`,
`Filter`, `Gate`, and `Crusher`. Bus kinds are `StereoMixer`, `SendTap`, and
`AuxReturn`. Sink/tap kinds are `MainPlayback`, `HardwareSend`, `RecordPreFx`,
`RecordPostFx`, and `RecordMaster`.

Edges carry stereo audio. A mono source requires an explicit adapter node; it
is never silently duplicated. A hardware send and its own return may not form
a path, and a master/sink may not feed a source.

## Validation before publication

A complete proposed graph is built and validated away from the JACK callback.
Validation checks:

- unique node, edge, port, and effect-instance IDs and references to existing
  objects;
- compatible channel layouts or an explicit adapter;
- an acyclic graph with deterministic topological order and no self-edge;
- no hardware-send/own-return, master/input, or other feedback path;
- exact, unambiguous configured JACK boundary ports;
- no unintended sink reachability or duplicate aux return;
- no simultaneous direct and software monitoring unless the user explicitly
  accepts the doubled-path warning;
- dry level forced to zero for delay, reverb, or modulation on an aux return;
- structural edits only while transport and recording are stopped; and
- every capacity and memory bound below.

The graph plan, filter coefficients, port resolution, delay memory, and all
callback buffers are prepared before activation. External connection failure
or rejected validation leaves the old graph and direct route unchanged.

## Initial hard bounds

These are rejection limits, not targets and not silent truncation points:

| Resource | Bound |
| --- | ---: |
| Stereo source strips | 4 |
| Aux buses | 2 |
| Effects in one serial chain | 8 |
| Active effect instances | 16 |
| Typed nodes | 32 |
| Edges | 64 |
| Simultaneous reverbs | 2 |
| Delay per delay instance | 2 seconds at active rate |
| JACK frames per callback | 4096 |
| Total owned delay/effect memory | 16 MiB |

The meter RMS window is also bounded to 4096 frames. The graph may initially
reserve one preallocated stereo block per node. Buffer-liveness reuse is a
measurement-led optimization, not permission to raise a bound.

## Callback contract

The process callback may read and write only fixed memory, atomics, and
lock-free bounded queues. It must not allocate or free, take a lock, access a
file, run a subprocess, log, format text, panic, or calculate trigonometric
functions per sample.

Parameter targets are finite and range checked on the control side. Cheap
values are smoothed in the sample path. Biquad coefficients and LFO recurrence
steps are calculated before callback use. Every processor guards non-finite
input, output, and state; a poisoned processor resets and yields a bounded dry
or silent fallback instead of propagating NaN/infinity.

Structural publication is intentionally stopped-only. The publish flag
is cleared and JACK deactivation joins the callback before the control thread
rebuilds the plan. Compatible kind plus stable instance ID moves the existing
runtime slot into the replacement plan, retaining recursive DSP, smoothing,
dither, and meter state. The same client is reactivated and its exact boundary
transaction is rechecked before output is armed. A failure restores the direct
fallback; no old and new callback plan can run together. Live two-plan
crossfading remains future work and is not implied by the current stopped-only
workflow.

## Shared DSP foundation

`src/dsp/mod.rs` provides finite/denormal guards, stereo frames, smoothed
values, dB conversion, one-pole filters, a DC blocker, transposed-direct-form-II
biquads, bounded fractional delay, an envelope follower, a recurrence sine
LFO, and fixed-window peak/RMS metering with atomic publication. Construction
and configuration are control-thread work; their sample-processing methods are
allocation-free.

The biquad formulas are an original Rust implementation guided by the
[W3C Audio EQ Cookbook](https://www.w3.org/TR/audio-eq-cookbook/), which
documents Robert Bristow-Johnson's public coefficient formulas. JACK lifecycle
and port behavior follow the official
[JACK 2 API header](https://github.com/jackaudio/jack2/blob/develop/common/jack/jack.h).
No third-party DSP implementation code is copied into SHR-DAW.

Deterministic tests cover silence/step/impulse behavior, supported sample-rate
limits, reset and non-finite recovery, stereo independence, long-running
finite state, chunk-size invariance, and callback-path allocation detection.

The effect rack adds one canonical named parameter schema per effect kind in
`src/effect_schema.rs`. Persisted values may omit older/defaulted controls, but
unknown names, non-finite values, invalid discrete choices, and values outside
the declared physical range reject the complete graph. `src/effects/` provides
fixed runtime slots with stable instance identity, finite dry fallback,
click-conscious bypass, reset, and separate input/output peak/RMS, clip, and
non-finite meters. The EQ, compressor, distortion, crusher/reducer, gate,
multimode filter, delay, chorus, flanger, phaser, tremolo/autopan, and
shared-topology reverb have passed their deterministic software response gates.
They are available in source and master racks; delay/reverb/modulation effects
on an aux are validated as 100% wet. Two independently scaled pre/post sends
feed two metered returns, which are mixed exactly once before the ordered master
chain. The compact rack/editor
uses four controller pages, with `OPS` first and `EXIT` at page 4/item 4, and
shows input and output peak/RMS, clipping, non-finite counts, and compressor
gain reduction. Raspberry Pi whole-chain evidence is documented in the
[Phase 2 measurement](PHASE2_AUDIO_GRAPH_MEASUREMENT.md) and
[Phase 3/4 measurement](PHASE3_4_AUDIO_GRAPH_MEASUREMENT.md); the consolidated
human-curation gate remains open in the latter.

The dry client additionally publishes allocation-free callback count, total,
mean, maximum, missed-deadline, and oversized-callback counters. One fixed
one-microsecond histogram increment per callback lets the owner calculate p95
and p99 outside the callback. The headless daemon records the final timing
summary in its private engine log during an orderly stop; the full measurement
report remains owner-thread/Pi checkpoint work.

## Measurement and curation gates

Effect quality has an objective engineering part and a musical-curation part.
Tests and controlled captures must establish the technical part against a
declared design target rather than stopping at basic finite-output checks. Use
impulses, steps, steady and swept sines, multitone/noise, silence, and bounded
program material as appropriate. Measure response/cutoff/slope/resonance,
transfer and dynamics curves, harmonics/intermodulation/aliasing/DC, envelope
timing and stereo linking, transient overshoot/ringing, discontinuities and
zipper/bypass clicks, noise/headroom, latency, chunk/sample-rate consistency,
long-run recovery, and whole-chain callback cost where each applies. Report
deviations numerically and treat a measurable defect as an engineering issue,
not merely a subjective preference.

Those measurements can establish accuracy to the intended response, stability,
bounded artifacts, click-conscious operation, and Pi performance. They cannot
alone decide whether a deliberate coloration is inspiring or appropriate for
particular music. Each phase therefore also needs an authorized, low-gain,
level-matched release-mode listening checkpoint. Record JACK rate/period
settings, callback mean/p95/p99/max, missed deadlines, xruns, process/core CPU,
RSS, owned effect memory, meters, and shutdown/client-loss behavior. Provide an
evidence-based technical `KEEP`, `IMPROVE`, or `DROP` recommendation; the
creator makes the final musical curation decision, and only kept/improving
choices remain visible as product effects.

The first authorized graph checkpoint compared one dry source with the direct
fallback and verified bit-exact stereo output, one path, callback headroom, and
clean fallback/ownership behavior. The exact Pi evidence is recorded in
[Phase 1 dry audio graph measurement](PHASE1_AUDIO_GRAPH_MEASUREMENT.md). It
does not imply that an unmeasured creative effect or later graph phase is safe.
