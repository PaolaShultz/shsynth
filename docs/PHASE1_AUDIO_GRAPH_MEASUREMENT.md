# Phase 1 dry audio graph measurement

This is the first authorized low-gain Raspberry Pi checkpoint for SHR-DAW's
owned dry graph. It covers one managed synth source only. It does not measure
creative effects, auxes, live input, or recording taps and is not evidence that
those later phases are Raspberry Pi safe.

## Test system

- Date: 2026-07-18
- Device: Raspberry Pi 4 Model B Rev 1.4, four Cortex-A72 cores
- Kernel: Linux 6.12.93+rpt-rpi-v8, aarch64
- Interface: PreSonus AudioBox USB 96 (`hw:A96`)
- JACK: 1.9.21, real-time priority 95, 48,000 Hz, 128 frames, 3 periods
- Period deadline: 2,666.7 microseconds
- CPU profile: performance governor; JACK and the managed synth on isolated
  CPU 3; the owned graph callback was observed on CPU 0
- Build: Rust 1.85 optimized release, through commit `573c6ad`
- Source: the MIT-cleared `Compact Bass` synthv1 preset
- Signal: five one-second low-velocity bass/chord gestures at MIDI velocity 24

The AudioBox output level was lowered before the authorized audible run. The
same preset, phrase, sample rate, period, periods, and physical outputs were
used for direct and graph routing. No JACK buffer setting was raised to hide a
failure.

## Routing and signal result

The direct baseline contained exactly:

```text
shs-synthv1:out_1 -> system:playback_1
shs-synthv1:out_2 -> system:playback_2
```

The graph run contained exactly one path per channel:

```text
shs-synthv1:out_1 -> shr-graph:managed_in_l
shr-graph:main_out_l -> system:playback_1
shs-synthv1:out_2 -> shr-graph:managed_in_r
shr-graph:main_out_r -> system:playback_2
```

There was no simultaneous managed direct connection during the graph run. An
eight-second, 384,000-frame, four-channel 32-bit PCM capture recorded the synth
source pair and graph output pair simultaneously. Every graph output sample
equalled its corresponding source sample: zero differing left samples, zero
differing right samples, and a maximum difference of zero integer least
significant bits. No extra sample of latency was present. Left and right were
not collapsed: they differed on 263,490 of 384,000 frames.

Separate-run levels were close despite free-running synth phase:

| Route | Left peak | Right peak | Left RMS | Right RMS |
| --- | ---: | ---: | ---: | ---: |
| Direct | 0.237065 | 0.179341 | 0.032877 | 0.019855 |
| Graph source/output | 0.236698 | 0.178479 | 0.032709 | 0.019798 |

The simultaneous bit-exact comparison, rather than the small difference
between separate synth runs, establishes dry level and stereo equivalence.

## Performance result

The main graph run lasted 29,039 callbacks, about 77.4 seconds at the active
period. The callback-owned fixed histogram was read after callback
deactivation:

| Metric | Result |
| --- | ---: |
| Callback mean | 8.246 us |
| Callback p95 | 13.000 us |
| Callback p99 | 28.000 us |
| Callback maximum | 151.907 us |
| Missed callback deadlines | 0 |
| Oversized callbacks | 0 |

`jack_cpu_load` samples taken during equivalent eight-second captures were:

| Route | Mean | Minimum | Maximum |
| --- | ---: | ---: | ---: |
| Direct | 5.215% | 4.266% | 6.798% |
| Dry graph | 6.149% | 5.131% | 7.540% |

The graph capture recorded four channels (source plus graph output) while the
direct capture recorded two, so the small `jack_cpu_load` delta includes the
extra recorder-channel work and is not attributed entirely to the dry graph.

Process samples showed the direct SHR daemon at 0.1-0.2% CPU and about 7.1 MiB
RSS, and the graph-owning daemon at about 1.0% CPU and 113.2 MiB RSS. The graph
daemon's rollup was 38.7 MiB PSS with about 1.7 MiB private dirty memory; most
of the large RSS was shared JACK memory, including about 34.1 MiB locked. The
synth stayed near 3.0-3.6% process CPU and 126.0 MiB RSS in both cases. These
are short `ps` samples, not promises for a later full rack.

No JACK xrun or process error occurred during either valid direct or graph
capture. JACK reported synth process errors only during deliberate managed
synth termination and full server restart, outside the sustained measurement
window.

## Fallback, loss, and ownership result

- Normal graph shutdown deactivated the callback first. A 50 ms topology poll
  then observed the exact direct left route on poll 4 before the managed synth
  exited. The matching right route is created by the same checked operation.
- A separate `system:capture_1 -> jackrec:input1` connection remained present
  through graph activation and graph shutdown. SHR removed only its graph and
  managed-synth resources.
- A deliberate JACK service restart exercised whole-server loss. The owner
  reported graph loss and an unavailable direct restore while the server was
  down, with 6,145 callbacks, 6.865 us mean, 11 us p95, 13 us p99, 54.481 us
  maximum, zero missed deadlines, and zero oversized callbacks. It left no
  stale SHR resources.
- After JACK returned at the unchanged 48,000 Hz / 128-frame setting, a
  graph-disabled managed start created exactly the two conservative direct
  links and shut down cleanly.

The repository's non-audible rollback, allocation, client-loss, and exact
ownership regression tests remain part of the gate. The ignored local graph
flag was returned to `false` after measurement, and no physical wiring or JACK
buffer configuration was changed.

## Phase decision

Phase 1 passes its one-managed-source dry-path checkpoint: routing is single,
dry output is bit exact and stereo-preserving, callback timing stayed far below
the observed whole-period deadline, fallback was observable, and unrelated
resources survived. This result permits Phase 2 planning or implementation;
it does not pre-approve any creative effect or effect default. Each later phase
still requires its own deterministic tests, level-matched listening, Pi
measurements, and `KEEP` / `IMPROVE` / `DROP` curation.
