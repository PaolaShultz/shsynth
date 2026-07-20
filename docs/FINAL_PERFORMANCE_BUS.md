# Final stereo performance bus

The opt-in owned audio graph has one deliberately small final bus. It is not a
free-wiring view or a general-purpose mixer. Exactly three stereo sources are
instantiated:

```text
managed software instrument -> source inserts/optional aux sends --\
owned native-rate WAV loop -----------------------------------------+-> stereo sum
configured JACK capture L/R ---------------------------------------/
    -> optional aux returns, where routed from the managed source
    -> master insert rack
    -> master level
    -> linked sample-peak limiter
    -> FINAL meter
    -> final 24-bit stereo WAV tap
    -> configured JACK playback L/R
```

The loop and external-input strips do not gain individual insert racks, aux
sends, pan, solo, automation, or waveform editing. Their only controls are a
smoothed level and mute. The managed source keeps its existing Project-owned
insert/aux routing. Master level follows the complete sum. Source gain is
bounded to -60..+6 dB, master gain to -60..0 dB, and all level/mute transitions
use a 10 ms sample ramp. New runtime buses start each source at -6 dB to leave
basic three-source summing headroom. These live performance controls are not
Project data; Project format remains 4. JACK assignments remain machine
configuration.

## Exact routing and availability

The configured input is `audio.graph.input=LABEL|LEFT|RIGHT`. When that optional
new key is blank, the first legacy `capture.input` pair is reused so older
runtime configuration remains useful. Both exact names must exist and be
distinct. A similar-looking or adjacent port is never substituted. The owned
loop client must be loaded and expose its exact configured output ports before
the bus can activate. The MTR screen reports the three sources as READY or
OFFLINE.

Before activation, the synth and loop each have their ordinary direct stereo
routes. The graph publishes silence while all six source connections and the
final playback pair are connected. It then removes the exact two synth and two
loop direct connections as one rollback-capable transaction and publishes the
callback at a block boundary. A failed connection restores the exact prior
topology and leaves unrelated JACK connections untouched. Normal shutdown,
loop replacement, JACK loss, or source disappearance deactivates the callback
before restoring the direct routes. A missing source remains missing; recovery
does not invent a replacement.

`audio.graph.input_direct_monitoring` describes whether the interface's own
zero/low-latency direct monitor is also audible. The final bus is software
monitoring. Enabling both without
`audio.graph.confirm_doubled_monitoring=true` refuses graph activation because
the delayed software copy and direct copy can comb-filter or sound doubled.
Confirmation is deliberately explicit; it does not change interface hardware.

## Limiter

The last processor is a dedicated stereo-linked lookahead limiter, not a
compressor preset:

- sample-peak ceiling: -1.0 dBFS;
- soft knee: 3.0 dB;
- detector: the larger absolute left/right sample, with one shared gain so the
  stereo image is preserved;
- lookahead target: 2.5 ms, exactly 120 samples at 48 kHz and 110 samples at
  44.1 kHz (2.494331 ms at that integer rate);
- release time constant: 100 ms, with an attack/hold covering the lookahead;
- automatic makeup: none; and
- bypass: not exposed, so the protection cannot be removed accidentally.

The delay is allocated and cleared before callback use. Processing performs no
allocation, file access, locks, logging, formatting, sleeping, or unbounded
work. Non-finite input becomes silence; invalid internal indices or envelope
state reset deterministically. A final finite clamp guarantees that published
samples do not exceed the declared sample ceiling.

This is **not true-peak or inter-sample-peak limiting**. It has no oversampled
detector. The -1 dBFS sample ceiling provides pragmatic headroom, not a promise
about reconstructed analogue peaks. The MTR screen shows pre-limiter clips,
final clips, final L/R peak, and bounded maximum gain reduction for the latest
block.

The limiter adds the exact delay above. JACK and the interface add their own
buffering in addition: capture/playback periods, driver safety buffering, and
converter latency depend on the active server and hardware. SHR does not hide
those as part of the 2.5 ms figure. For example, a 128-frame period is 2.667 ms
at 48 kHz, but the number of periods and converter delays must be observed in a
real full-duplex test.

## Final recording

MTR `REC` arms one final-mix recording. Start and stop are sampled only at
whole callback boundaries. The callback gives the recorder the same final
limited `StereoFrame` slice that is then copied to JACK playback. A bounded
interleaved stereo ring transfers it to a non-real-time writer, which performs
24-bit conversion, file writes, flush, synchronization, and no-replace
publication.

The result is one conventional little-endian PCM RIFF/WAVE file: two
interleaved channels, 24 bits, and the active JACK sample rate. It includes the
three-source sum, managed-source aux returns, master rack, master level, and
limiter. It excludes raw recorder stems, unrelated JACK clients, interface
direct monitoring, hardware mixer/insert processing after JACK playback, and
any downstream speaker/headphone processing.

Classic RIFF has a 32-bit data size. Stereo 24-bit audio uses six bytes per
frame, so SHR stops before 715,827,876 frames instead of wrapping. That is about
4:08:33 at 48 kHz or 4:30:32 at 44.1 kHz. A zero-frame take is not published.
Overflow, writer failure, JACK shutdown/xrun, oversized callback, invalid
buffer, or required-source loss stops/faults the take visibly. A faulted
`*.wav.part` remains recoverable; it is never presented as a successful final
WAV. Existing raw multitrack sessions and legacy stereo recovery remain
unchanged.

## Generic interface setup and future MR18 acceptance

Use the setup wizard or edit private runtime configuration only after obtaining
the exact JACK names from the current machine. Choose one stereo capture pair
that already contains the desired external-gear mix. Keep interface direct
monitoring off for the normal software-monitored workflow, set conservative
hardware gains, and enable `audio.graph.enabled` only when the synth, loop,
input pair, and playback pair are all ready.

After an absent source returns, MTR `RESET` retries activation against those
same remembered exact names. It never rewrites the mapping or chooses another
port.

No MR18 name is compiled into SHR. A future MR18 acceptance must follow
[the hardware plan](MR18_TEST_PLAN.md): discover and record the actual names,
confirm the intended two-channel external mix, decide direct-monitor behavior,
then test full duplex at conservative level. It must verify channel identity,
source unplug/reconnect faults, no doubled direct/software path, final playback
versus WAV equality, xruns/dropouts, and teardown restoration. Synthetic tests
and the current AudioBox-era configuration are not an MR18 pass.

Maintainers can exercise the production hardware-independent path with:

```sh
shr final-mix-stress DEST [SECONDS] [RATE] [CALLBACK]
```

It uses three distinguishable stereo sources, the production faders/limiter,
bounded callback handoff, stereo writer, and full PCM equality check without
opening JACK, starting a synth, transmitting MIDI, or producing sound. See
[maintainer helpers](MAINTAINER_HELPERS.md#synthetic-final-mix-stress).
