# Phase 3/4 time effects, reverb, and buses measurement

This is the objective evidence record for the expanded internal effects graph.
The user explicitly chose to complete the bounded effects set before one
consolidated listening and repair pass. Technical conformance can identify a
wrong response or artifact; it does not by itself decide musical taste.

## Software measurement checkpoint

Measured on 2026-07-19 with the installed Rust 1.85 aarch64 toolchain. The
complete suite contains 356 tests. Every new processor covers silence, impulse
or bounded program input, parameter limits and rapid movement, reset,
non-finite recovery, 8–384 kHz sample-rate limits, chunk invariance, bypass,
and callback-allocation detection.

| Processor/topology | Declared target | Objective result or enforced tolerance |
| --- | --- | --- |
| Stereo delay | Free 1–2,000 ms or tempo-synced divisions; stereo, ping-pong, and mono-stereo; feedback no higher than 92% | Free and sync impulses land on the exact declared sample; ping-pong crosses channels on successive echoes and decays; smoothed time changes remain finite; tail bypass drains while ordinary bypass clears and reaches exact dry |
| Chorus | 5–30 ms base delay with bounded modulation and stereo phase | Zero-depth 15 ms impulse lands at sample 720 at 48 kHz; every modulated read head remains within allocated history; stereo phase produces distinct channels |
| Flanger | 0.2–8 ms base delay with bounded signed feedback | Positive and negative feedback are measurably distinct; the delay read head remains at least one sample behind the writer and within allocated history |
| Phaser | Four or six strictly stable first-order all-pass stages | Every precomputed coefficient is finite with magnitude below 1 at all supported rates and parameter extremes; four/six-stage and stereo-phase responses are distinct |
| Tremolo/autopan | Sine, triangle, or 5 ms-smoothed square LFO; constant-power pan law | Zero depth is unity; shapes are bounded and distinct; every autopan table entry keeps squared left-plus-right gain at 2 within 0.00001, preserving the rack's unity-at-center convention without callback trigonometry |
| Reverb | Original four-line Hadamard feedback-delay network; room, plate, and hall voicings; predelay, damping, width, and bounded RT60 | Every line feedback is strictly between 0 and 1; reconstructed RT60 attenuation is −60 dB within 0.01 dB; a 20 ms room impulse first arrives in samples 2,070–2,090 at 48 kHz; all three voicings have distinct signatures, decorrelated stereo output, and late energy below one fifth of early energy |
| Aux sum | Two independent pre/post sends into forced-wet chains, each returned once | A −6.0206 dB send followed by a −6.0206 dB return produces 0.25 within 0.001 and the return meter agrees; dry-only auxes, a third bus, a third reverb, cycles, and duplicate/global-ID overflow are rejected |
| Master | Dry source plus aux returns summed once, then one ordered chain | Deterministic topology tests place the master effects after the sum and before the single sink; the master fader/meter processes without allocation |

The delay is intentional wet-path time, not hidden graph latency. Source and
master processors otherwise run in the current JACK callback without a
lookahead block. Chorus/flanger likewise create only their declared delayed
component; their dry component is not block-delayed.

## Product integration checkpoint

- Project format 3 persists the source rack, two aux buses, independent send
  level and pre/post point, return gain, and master rack. Formats 0/1 migrate to
  empty racks/routing; format 2 retains its source rack and gains empty routing.
  Unknown fields, malformed values, and newer formats are refused before
  overwrite.
- Effect IDs are global and stable across source, aux, and master chains.
  Add/remove/reorder operations validate transactionally and keep compatible
  runtime state by ID.
- Aux delay, reverb, chorus, flanger, and phaser instances are forced to 100%
  wet and 0% dry. Empty/dry-only active auxes are rejected.
- Structural publication remains stopped-transport/no-recording only. The exact
  owned-client rollback and direct fallback from Phase 1 are unchanged.
- The compact FX screen selects `SOURCE`, `AUX 1`, `AUX 2`, or `MASTER`; exposes
  send, point, and return controls; keeps `OPS` on page 1 and `EXIT` at page
  4/item 4; and displays effect, return, and master meters.

## Raspberry Pi performance checkpoint

The release-mode low-gain checkpoint uses the same Raspberry Pi 4, dedicated
audio core, AudioBox USB 96, JACK 48 kHz/3 periods, and low-velocity `Compact
Bass` source as Phase 2. `phase4-full` deliberately combines eight source
inserts, two independently fed reverbs, and one master compressor. It is a
capacity/topology stress case, not a proposed musical preset.

| JACK setting/profile | Window | Callbacks | Mean | p95 | p99 | Maximum | Deadline misses / oversized |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 128 frames, dry | 10.028 s | 3,835 | 5.491 µs | 11 µs | 13 µs | 57.666 µs | 0 / 0 |
| 128 frames, time-full ×8 | 30.068 s | 11,359 | 193.749 µs | 227 µs | 254 µs | 496.275 µs | 0 / 0 |
| 128 frames, room reverb ×1 | 30.072 s | 11,352 | 62.035 µs | 74 µs | 95 µs | 190.314 µs | 0 / 0 |
| 128 frames, two aux reverbs | 30.070 s | 11,358 | 123.651 µs | 146 µs | 176 µs | 486.775 µs | 0 / 0 |
| 128 frames, phase4-full ×11 | 60.052 s | 22,599 | 313.572 µs | 345 µs | 360 µs | 540.108 µs | 0 / 0 |
| 64 frames, dry | 10.027 s | 7,687 | 3.611 µs | 7 µs | 10 µs | 53.629 µs | 0 / 0 |
| 64 frames, phase4-full ×11 | 60.051 s | 45,232 | 158.527 µs | 182 µs | 198 µs | 349.905 µs | 0 / 0 |

At 128 frames the 2,666.7 µs callback deadline leaves the combined graph at
13.5% for p99 and 20.25% for its maximum. At 64 frames the 1,333.3 µs deadline
leaves it at 14.85% for p99 and 26.24% for its maximum. Owner CPU/RSS were
12.29%/118,640 KiB at 128 and 12.59%/118,644 KiB at 64; synth CPU/RSS were
4.90%/128,980 KiB and 5.43%/129,000 KiB respectively. The measured owner RSS
increase from matching dry runs was 2,704 KiB at 128 and 2,692 KiB at 64.

With the configured 4,096-frame safety capacity, `phase4-full` preallocates a
derived 1,848,420 bytes for effect state/meters/delay history and 589,824 bytes
for graph audio buffers, 2,438,244 bytes combined. This remains well below the
16 MiB owned effect-memory rejection limit.

No xrun occurred during any sustained measurement window. JACK again reported
synth-client xruns at the deliberate teardown timestamps, beginning when the
checkpoint restored direct routing and stopped the owned synth. These are not
hidden; they remain a shutdown-cleanup defect, while the graph's own counters
reported no missed or oversized callback in every window.

The 64-frame setting halves JACK's period contribution to latency relative to
128 while retaining ample headroom in this deliberately dense one-engine test.
It is therefore a credible low-latency operating candidate, not yet a universal
default: simultaneous synth engines, loop/recording traffic, and real musical
polyphony still deserve a later whole-product soak. `/etc/jackdrc` was restored
byte-for-byte to its original 48 kHz, 128-frame, 3-period setting (SHA-256
`abb060978b8cd03711eb85a4a393a374abe98849b4dfe96fadc1e5ab714cab62`), JACK is
active, SHR is stopped, no owned ports remain, and graph enablement is still
absent/default-false.

## Consolidated human curation sheet

Listen at low gain and roughly level-match bypass against active output. Mark
one choice per row. `IMPROVE` means keep the idea visible for repair; `DROP`
means remove it from the product rather than merely changing its default.

| Item | KEEP | IMPROVE | DROP | Primary objective listening cue / notes |
| --- | :---: | :---: | :---: | --- |
| Five-section EQ | ☐ | ☐ | ☐ | Sweep each band; low cut should clean lows without unexpected whistling |
| Compressor | ☐ | ☐ | ☐ | Linked stereo image, transient control, pumping, release |
| Soft cubic distortion | ☐ | ☐ | ☐ | Smoothness versus audible high-note aliasing |
| Hard clip distortion | ☐ | ☐ | ☐ | Deliberately sharp edge; decide whether useful rather than polite |
| Asymmetric diode-like distortion | ☐ | ☐ | ☐ | Even-harmonic color without audible DC thump |
| Gate/expander | ☐ | ☐ | ☐ | Chatter, note-tail truncation, stereo stability |
| Low-pass filter | ☐ | ☐ | ☐ | Resonant sweep stability and musical character |
| Band-pass filter | ☐ | ☐ | ☐ | Center emphasis and usable resonance range |
| High-pass filter | ☐ | ☐ | ☐ | Thinness versus useful motion |
| Bitcrusher/rate reducer | ☐ | ☐ | ☐ | Step character, high-frequency harshness, dither usefulness |
| Stereo delay | ☐ | ☐ | ☐ | Timing, ping-pong image, feedback decay, time-change artifacts |
| Chorus | ☐ | ☐ | ☐ | Width without pitch seasickness or level jump |
| Flanger | ☐ | ☐ | ☐ | Comb sweep, signed-feedback character, runaway impression |
| Phaser 4-stage | ☐ | ☐ | ☐ | Sweep shape and low-frequency loss |
| Phaser 6-stage | ☐ | ☐ | ☐ | Added depth versus excessive coloration |
| Tremolo sine | ☐ | ☐ | ☐ | Smooth pulse and perceived loudness |
| Tremolo triangle | ☐ | ☐ | ☐ | More obvious motion without clicks |
| Tremolo smoothed square | ☐ | ☐ | ☐ | Rhythmic edge versus 5 ms transition softness |
| Autopan | ☐ | ☐ | ☐ | Center loudness and stable stereo travel |
| Room reverb | ☐ | ☐ | ☐ | Early density, metallic ringing, believable small space |
| Plate reverb | ☐ | ☐ | ☐ | Bright sustained tail, vocal/synth usefulness |
| Hall reverb | ☐ | ☐ | ☐ | Long-tail smoothness and low-frequency buildup |
| Two-aux workflow | ☐ | ☐ | ☐ | Independent pre/post sends and return balance |
| Master insert workflow | ☐ | ☐ | ☐ | Predictable whole-mix order, meter, and bypass |

Full-duplex live inputs, external hardware sends/returns, and record-tap choices
are not part of this checkpoint. They cross the physical-interface monitoring
boundary and remain a later phase rather than being inferred from permission to
complete internal effects.
