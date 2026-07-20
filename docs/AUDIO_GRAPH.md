# Audio graph and DSP contract

This document is the current implementation contract for SHR-DAW's owned audio
graph and effects racks. The graph is implemented and Raspberry Pi measured,
but remains opt-in and disabled by default. Direct JACK routing is both the
default and the conservative fallback. The current product compiler instantiates
exactly three stereo sources while retaining the general four-source bound: the
managed software instrument, the owned WAV loop, and one exact configured JACK
capture pair.

The active signal flow is:

```text
managed instrument -> SOURCE inserts -------------------------------\
       |                    |                                            ^
       |                    +-> POST send gain -> wet AUX 1/2 -> return -+
       +----------------------> PRE send gain  -> wet AUX 1/2 -> return -+
owned WAV loop ------------------------------------------------------+-> dry sum
configured stereo JACK input --------------------------------------/

dry sum -> MASTER inserts -> master level -> linked limiter
        -> FINAL OUT meter -> final stereo WAV tap -> playback L/R
```

Each aux bus has its own send level, pre/post source-insert tap, forced-wet
serial rack, return level, and return meter. Each return is mixed exactly once.
The complete dry-plus-wet sum then passes through the master rack and a
dedicated final limiter and post-limiter meter immediately before the recorder
tap and playback. The final WAV and JACK playback buffers contain the same
limited samples. The exact final-bus contract is in
[Final stereo performance bus](FINAL_PERFORMANCE_BUS.md). The measured history
is in the [Phase 1 dry](PHASE1_AUDIO_GRAPH_MEASUREMENT.md),
[Phase 2 insert](PHASE2_AUDIO_GRAPH_MEASUREMENT.md), and
[Phase 3/4 bus](PHASE3_4_AUDIO_GRAPH_MEASUREMENT.md) records.

Send and return gain are each bounded to -60..+12 dB. The compact UI changes
sends by 3 dB and treats below -60 dB as `OFF`; a newly created aux starts with
a -18 dB post-insert send. Return changes also use 3 dB steps and wrap from
-60 to +12 dB. These are Project values, not JACK port gains.

## Ownership boundary

One SHR-owned JACK client contains the current stereo graph. Effects are
internal processors, not separate JACK clients or processes. That client has
three stereo input boundaries and one stereo output boundary to the
runtime-resolved pair. The saved preferred `audio.output` pair,
ordered internal pairs, and final headphone pair remain machine configuration.
Exact JACK and hardware
names come from configuration, never Rust constants.

The typed graph model reserves a fourth source plus hardware returns/sends, but
the current graph client instantiates only `ManagedEngine`, `LoopPlayer`, and
`LiveInput`. The loop remains its own rendering client; when the final bus is
active its output is moved off direct playback and into the owned sum. The raw
synchronized multitrack recorder remains a separate capture client. External
instruments return only through the configured stereo mix. There is no hardware
insert or per-interface-channel processing.

The graph may connect and disconnect only SHR-owned endpoints. It must not
alter unrelated JACK connections or terminate a client/process it does not
own. Until the graph is activated successfully, the managed engine and loop use
their exact direct routes. The raw recorder routes are unchanged.

For every current managed-engine start, direct playback is connected first. The
owned callback publishes silence while its eight boundary links are prepared;
the two synth and two loop direct links are then removed in the same
rollback-capable transaction.
Only after that commit does an atomic flag publish graph output at a block
boundary. A rejected graph, activation failure, ambiguous engine-output pair,
or connection failure leaves or restores the exact direct topology.

On normal shutdown, the publish flag is cleared and JACK deactivation joins the
owned callback before the synth and loop direct links are restored. Closing that client then
releases only its registered ports and their graph-boundary connections. This
ordering prevents a final already-started graph block from overlapping the
restored direct path.

`src/jack.rs` is the shared dynamic-loading and lifetime boundary for current
and future JACK clients. A caller owns its callback allocation and keeps it
alive until client deactivation returns.

## Project data and typed graph model

Project format 4 stores the managed-source `InsertRack` and
`ProjectAuxRouting` as strict JSON inside the versioned `.shsong` line format.
Formats 0 and 1 migrate to an empty rack and routing; format 2 keeps its source
rack and adds empty aux/master routing; format 3 retains explicit routes.
Unknown current fields, malformed rack
data, and newer Project/effect versions are refused on load and on overwrite.
Rack order is a separate list of stable effect IDs, so moving an effect does
not recreate its identity.

The runtime compiler expands those Project fields into a typed graph definition
with stable non-zero IDs:

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
- every non-empty aux rack contains a wet generator; delay, reverb, chorus,
  flanger, and phaser are forced to 100% wet with zero dry signal;
- runtime plan publication only while transport and recording are stopped;
  graph-disabled Project edits validate and persist without touching audio; and
- every capacity and memory bound below.

The graph plan, filter coefficients, port resolution, delay memory, and all
callback buffers are prepared before activation. External connection failure
or rejected validation leaves the old graph and direct route unchanged.

## Initial hard bounds

These are rejection limits, not targets and not silent truncation points:

| Resource | Bound |
| --- | ---: |
| Stereo source strips | 4 in the general model; exactly 3 instantiated |
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
fixed runtime slots with stable instance identity, placement-safe finite
fallback, click-conscious bypass, reset, and separate input/output peak/RMS,
clip, and non-finite meters. The EQ, compressor, distortion, crusher/reducer, gate,
multimode filter, delay, chorus, flanger, phaser, tremolo/autopan, and
shared-topology reverb have passed their deterministic software response gates.
They are available in source and master racks; delay/reverb/modulation effects
on an aux are validated as 100% wet. Two independently scaled pre/post sends
feed two metered returns, which are mixed exactly once before the ordered master
chain. Source and master bypass retain dry passthrough. Aux bypass tracks wet
generators explicitly: an all-bypassed bus returns silence, a delay may drain a
wet-only tail with muted input, and serial conditioning may pass an already-wet
signal. A dedicated meter after the final master insert and immediately before
playback supplies the `MASTER`/`FINAL OUT` reading. The compact rack/editor
uses four controller pages, with `OPS` first and `EXIT` at page 4/item 4, and
shows input and output peak/RMS, clipping, non-finite counts, and compressor
gain reduction. Raspberry Pi whole-chain evidence is documented in the
[Phase 2 measurement](PHASE2_AUDIO_GRAPH_MEASUREMENT.md) and
[Phase 3/4 measurement](PHASE3_4_AUDIO_GRAPH_MEASUREMENT.md); the consolidated
human-curation gate remains open in the latter.

The source/master effect meters observe each processor's input and output;
return meters observe the wet bus after return gain; `FINAL OUT` observes the
complete owned graph after master processing. All publish bounded peak/RMS,
clip, and non-finite state through atomics. They do not observe the separate
loop client, recorder capture, hardware, or unrelated JACK clients. The MTR
keeps its non-decaying numeric L/R maxima entirely in UI presentation state;
the audio callback continues to publish only the bounded lock-free snapshots.
The bar's short peak marker retains its existing hold and decay behavior. The
numeric maxima reset on MTR RESET, every downward mapped synthv1 Volume movement
even before pickup, and meter/engine session boundaries. The MTR
CPU bars are whole-core `/proc/stat` load and deliberately cannot diagnose
per-process DSP cost, callback deadlines, scheduling jitter, or xruns; those
belong to the explicit checkpoint counters and JACK evidence.

### Effect parameter schemas

These stable names and physical ranges are the persistence and control
contract. Values in parentheses are defaults. Discrete numeric modes are
listed in their current order.

| Effect | Parameters |
| --- | --- |
| Utility | `trim_db` -60..12 (0); `pan` -1..1 (0); `width_percent` 0..200 (100); `invert_left`, `invert_right`, `mute` toggles |
| EQ | `low_cut_enabled`; `low_cut_hz` 20..500 (80); `low_shelf_hz` 40..800 (120), `low_shelf_db` -18..18 (0); `low_mid_hz` 80..3000 (500), `low_mid_db` -18..18 (0); `high_mid_hz` 400..12000 (3000), `high_mid_db` -18..18 (0); `high_shelf_hz` 1500..20000 (8000), `high_shelf_db` -18..18 (0); `output_trim_db` -18..12 (0) |
| Compressor | `threshold_db` -48..0 (-18); `ratio` 1..20 (4); `knee_db` 0..12 (6); `attack_ms` 0.1..100 (10); `release_ms` 20..1500 (150); `makeup_db` -12..18 (0); `mix_percent` 0..100 (100); `sidechain_highpass_hz` 20..250 (20) |
| Distortion | `mode` 0 soft cubic, 1 hard, 2 asymmetric (0); `drive_db` 0..30 (6); `bias` -0.5..0.5 (0); `tone_hz` 800..18000 (12000); `output_db` -24..0 (-6); `mix_percent` 0..100 (100) |
| Delay | `mode` 0 stereo, 1 ping-pong, 2 mono-to-stereo (0); `tempo_sync`; `tempo_bpm` 20..300 (120); `division` 0..7 (4); `time_ms` 1..2000 (375); `feedback_percent` 0..92 (30); `stereo_ratio` 0.5..2 (1); `tone_hz` 500..18000 (8000); `wet_percent` 0..100 (25); `dry_percent` 0..100 (100); `tail_on_bypass` |
| Reverb | `type` 0 room, 1 plate, 2 hall (0); `predelay_ms` 0..200 (20); `decay_seconds` 0.2..8 (1.5); `size_percent` 0..100 (50); `damping_percent` 0..100 (50); `input_low_cut_hz` 20..500 (80); `width_percent` 0..100 (100); `wet_percent` 0..100 (25); `dry_percent` 0..100 (100) |
| Chorus | `base_delay_ms` 5..30 (15); `rate_hz` 0.05..5 (0.5); `depth_percent` 0..100 (35); `stereo_phase_degrees` 0..180 (90); `feedback_percent` 0..35 (0); `mix_percent` 0..100 (35); `dry_percent` 0..100 (100) |
| Flanger | `base_delay_ms` 0.2..8 (2); `rate_hz` 0.03..5 (0.25); `depth_percent` 0..100 (50); `feedback_percent` -80..80 (25); `stereo_phase_degrees` 0..180 (90); `mix_percent` 0..100 (50); `dry_percent` 0..100 (100) |
| Phaser | `stages` 4 or 6 (4); `rate_hz` 0.03..5 (0.25); `center_hz` 100..5000 (1000); `range_octaves` 0.5..6 (3); `feedback_percent` -75..75 (0); `stereo_phase_degrees` 0..180 (90); `mix_percent` 0..100 (50); `dry_percent` 0..100 (100) |
| Tremolo/Pan | `mode` 0 tremolo, 1 pan (0); `rate_hz` 0.05..15 (4); `depth_percent` 0..100 (50); `shape` 0 sine, 1 triangle, 2 smoothed square (0); `stereo_phase_degrees` 0..180 (180); `output_trim_db` -18..12 (0) |
| Filter | `mode` 0 low-pass, 1 band-pass, 2 high-pass (0); `cutoff_hz` 20..20000 (1000); `resonance` 0..90 (20); `drive_db` 0..12 (0); `mix_percent` 0..100 (100) |
| Gate | `threshold_db` -80..0 (-48); `hysteresis_db` 0..24 (6); `range_db` -80..0 (-60); `attack_ms` 0.1..100 (2); `hold_ms` 0..500 (40); `release_ms` 5..2000 (150) |
| Crusher | `bit_depth` 4..16 (12); `hold_factor` 1..32 (1); `dither`; `mix_percent` 0..100 (100) |

Delay sync divisions 0..7 are 1/16, 1/8, 1/4, 1/2, 1, 2, 4, and 8
beats. The source and master racks allow all 13 types. The normal aux editor
offers Delay, Reverb, Chorus, Flanger, and Phaser and forces their wet/dry
values; conditioning effects can exist in a loaded aux chain only when the
chain also contains one of those wet generators.

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
