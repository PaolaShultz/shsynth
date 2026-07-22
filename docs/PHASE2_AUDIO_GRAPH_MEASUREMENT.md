# Phase 2 insert-effects measurement

> Historical checkpoint: the measurements and 333-test count below describe
> this phase on 2026-07-18/19. Later time/modulation, reverb, aux, master, and
> final-meter work is recorded in
> [Phase 3/4](PHASE3_4_AUDIO_GRAPH_MEASUREMENT.md); current behavior is specified
> in [Audio graph and DSP contract](AUDIO_GRAPH.md).

This is the evidence record for the essential insert rack. It separates
objective DSP conformance from the final musical decision. A technically
correct response can still be uninspiring; an effect with a measurable defect
cannot be excused as merely subjective.

## Software measurement checkpoint

Measured on 2026-07-18/19 with the installed Rust 1.85 aarch64 toolchain. The
complete suite contains 333 tests at the final checkpoint.
Every Phase 2 processor has deterministic silence, impulse or step, bounded
maximum input, randomized finite input, rapid valid parameter movement, reset,
non-finite recovery, 8–384 kHz sample-rate limits, chunk invariance, bypass,
and callback-allocation coverage.

| Processor | Declared target | Objective result or enforced tolerance |
| --- | --- | --- |
| EQ | Two-section fourth-order Butterworth high-pass, Q 0.5411961 and 1.306563 | −3.0103 dB at cutoff within 0.01 dB; 50–100 Hz octave slope 24 dB within 0.25 dB; 10×-cutoff passband within 0.001 dB |
| EQ shelves/bells | Cookbook shelf and broad Q=0.9 bell responses | Low shelf +12 dB within 0.02 dB; bell −9 dB within 0.01 dB; high shelf +6 dB within 0.1 dB; unity setup is sample exact |
| Compressor | Stereo-linked feed-forward, soft/hard knee, no lookahead | Hard-knee 4:1 gives −15 dB gain at 0 dBFS over a −20 dBFS threshold; 12 dB knee center is −1.125 dB; lookup error below 0.001 dB; attack reaches the declared one-time-constant value within 0.01; both channels receive equal gain; first impulse sample remains above 0.99 |
| Soft cubic | Bounded symmetric cubic transfer | Output stays within ±1; third harmonic exceeds 0.02 in the normalized test while second and fourth remain below 0.00001; a bin-exact 10.55 kHz/0.8 test at 48 kHz measures third-harmonic foldback at −23.9 dBc |
| Hard clip | Literal bounded clamp | ±1 bound and exact 0.5→0.5 within the unclipped region |
| Asymmetric diode-like | Intentionally asymmetric bounded transfer with automatic DC rejection | Second harmonic exceeds 0.01; automatic 10 Hz DC blocker settles residual below 0.001 |
| Gate/expander | Linked detector, open/close hysteresis, exact hold, bounded range | Opens exactly at threshold, stays open throughout the configured hold, closes on the following eligible sample; attack/release are monotonic; −40 dB range settles within 0.01 dB; channel gain ratio matches within 0.000001 |
| Multimode filter | Topology-preserving state-variable LP/BP/HP, Q bounded to 8 | LP low response above 0.98 and high below 0.02; HP inverse bounds; BP center exceeds both edge responses by 5×; exhaustive max-resonance impulse/random tests remain finite at every supported rate and cutoff extreme |
| Crusher/reducer | Signed PCM quantizer plus exact sample holds | Four-bit steps are exactly 0.125, with −1 and +0.875 endpoints and exact zero; 16-bit +1 maps to 32767/32768; hold factor owns exact 1–32-sample windows; deterministic TPDF dither remains bounded and chunk invariant |
| Shared slot | Finite recovery, metering, smoothing, bypass | Callback processing allocates nothing; bad input is counted and replaced with finite output; bypass reaches exact dry after 5 ms and its measured constant-input sample step remains below 0.002 |

The distortion modes are deliberately inexpensive and do not claim
oversampling or alias suppression. Their names describe their actual transfer
functions. The measured foldback makes aliasing on high-frequency driven
material a concrete curation point: if objectionable, mark it `IMPROVE` rather
than treating it as an unmeasured promise.

## Product integration checkpoint

- Project format 2 persists a strict eight-slot managed-source rack with stable
  IDs, kind/version, bypass, and named physical-unit parameters.
- Project formats 0 and 1 migrate to an empty rack. Unknown current fields,
  malformed values, and newer versions are refused before overwrite.
- Add, remove, and reorder are atomic model operations. Reorder keeps compatible
  DSP state and meter handles by stable ID.
- Structural publication requires stopped transport and no active recording.
  JACK callback execution is joined before the plan changes, then the same
  owned client and exact boundary are reactivated. Direct and graph output are
  never intentionally active together.
- The 40×20 rack and editor have four controller pages. `OPS` is page 1 and
  `EXIT` is page 4/item 4. The editor shows input/output peak and RMS, output
  clip/non-finite counts, and compressor gain reduction.

That 40×20 statement is a dated checkpoint fact. Current screens and generated
documentation use the later 40×13 physical layout.

## Raspberry Pi performance and latency checkpoint

Measured on a Raspberry Pi 4 Model B Rev 1.4 with 3.7 GiB RAM, performance
governor, dedicated/isolation CPU 3, AudioBox USB 96, JACK 1.9.21 at 48 kHz and
3 periods, and the `Compact Bass` synth. The checkpoint command enabled the
graph only in memory, transmitted MIDI note 48 at velocity 8, and restored the
exact direct route before stopping the SHR-owned engine. The persisted
`audio.graph.enabled` value stayed `false`.

The `full` profile deliberately fills the eight-slot chain with EQ,
compressor, asymmetric distortion, crusher/reducer, gate, low-pass filter, a
second EQ, and a second compressor. It is a capacity/performance stress profile,
not a proposed musical preset.

| JACK setting/profile | Window | Callbacks | Mean | p95 | p99 | Maximum | Deadline misses / oversized |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 128 frames, dry | 10.025 s | 3,811 | 2.263 µs | 7 µs | 7 µs | 48.111 µs | 0 / 0 |
| 128 frames, full ×8 | 30.084 s | 11,335 | 164.390 µs | 185 µs | 202 µs | 266.313 µs | 0 / 0 |
| 64 frames, dry | 10.028 s | 7,622 | 1.422 µs | 5 µs | 6 µs | 27.982 µs | 0 / 0 |
| 64 frames, full ×8 | 60.038 s | 45,151 | 87.809 µs | 107 µs | 132 µs | 446.516 µs | 0 / 0 |

At 128 frames the callback deadline is 2,666.7 µs; the full-chain maximum used
9.99% of it. At 64 frames the deadline is 1,333.3 µs; the one maximum outlier
used 33.49%, while p99 used 9.9%. Owner CPU/RSS were 6.68%/116,976 KiB for the
128 full run and 7.56%/116,980 KiB for the 64 full run. Synth CPU/RSS were
4.92%/128,984 KiB and 5.56%/129,004 KiB respectively. The 128 dry/full owner
RSS difference was 1,124 KiB.

With the configured 4,096-frame safety capacity, the full profile derives a
minimum 540,680 bytes of effect meter/lookup arrays plus 327,680 bytes of graph
audio buffers. Validation now computes this minimum from kind and capacity;
persisted memory claims cannot reduce it.

No xrun occurred inside any sustained measurement window. JACK logged synth
client xruns starting at the deliberate teardown timestamp after each window;
these are recorded rather than counted as sustained graph deadline failures.
The graph itself reported zero missed or oversized callbacks throughout. A
future cleanup improvement should remove the teardown-only JACK errors.

The inserts process in place in the same callback and add no designed sample
latency. JACK's ALSA documentation defines one capture period as
`period / rate`, playback latency as `periods × period / rate`, recommends 3
periods for USB devices, and advises lowering the power-of-two period only as
far as operation remains xrun-free. On that model, 48 kHz/3-period basic JACK
capture-plus-playback latency falls from about 10.67 ms at 128 frames to 5.33
ms at 64 frames, before converter/USB latency. The 64-frame test passed and is a
credible lower-latency candidate, but the service was restored to its recorded
128-frame setting after measurement. See the [JACK ALSA parameter reference](https://manpages.debian.org/bookworm/jackd2/jackd.1.en.html)
and [JACK real-time scheduling guidance](https://jackaudio.org/faq/linux_rt_config.html).
Raspberry Pi's current studio guide likewise uses JACK2 for low-latency audio
and warns that processor-heavy synth voices/effects can affect DAW performance,
which is why this repository relies on measured local headroom rather than a
generic Pi claim; see the [official Raspberry Pi studio guide](https://www.raspberrypi.com/news/how-to-build-a-home-recording-studio-with-raspberry-pi-500-choose-and-install-your-software/).

The performance and latency checkpoint is complete. Controlled capture of the
software responses is covered by deterministic tests; final low-gain,
level-matched musical listening and the curation decisions below remain open.

## Human curation sheet

After the Pi evidence and low-gain level-matched listening pass, mark one choice
per row. Technical recommendations may inform this sheet but do not make the
creator's musical decision.

| Item | KEEP | IMPROVE | DROP | Notes |
| --- | :---: | :---: | :---: | --- |
| Five-section EQ | ☐ | ☐ | ☐ | |
| Compressor | ☐ | ☐ | ☐ | |
| Soft cubic distortion | ☐ | ☐ | ☐ | |
| Hard clip distortion | ☐ | ☐ | ☐ | |
| Asymmetric diode-like distortion | ☐ | ☐ | ☐ | |
| Gate/expander | ☐ | ☐ | ☐ | |
| Low-pass filter | ☐ | ☐ | ☐ | |
| Band-pass filter | ☐ | ☐ | ☐ | |
| High-pass filter | ☐ | ☐ | ☐ | |
| Bitcrusher/sample-rate reducer | ☐ | ☐ | ☐ | |

The user later authorized completing the bounded internal Phase 3/4 graph before
one consolidated listen-and-repair pass. The unchecked rows therefore remain
open and are repeated beside the newer effects in the consolidated curation
sheet.
