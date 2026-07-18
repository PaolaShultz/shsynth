# Future improvements

This file records useful extensions that are deliberately not part of the
current behavior. They are not required for separate FT2 pages to sequence
multiple hardware instruments simultaneously.

## Irregular Patterns, swing, and groove timing

The post-competition plan for arbitrary Pattern shortening/growing, individual
early/late hits, Pattern-wide swing, groove tools, expressive tracker capture,
and optional formal odd-meter metadata is in [Post-competition rhythm
plan](POST_COMPETITION_RHYTHM_PLAN.md). The first planned release uses the
existing arbitrary row-count scheduler and a transactional Length editor, so a
loaded drum groove can be cropped freely and committed once without repetitive
confirmation dialogs.

## Unreasonable but useful challenges

These are moonshots, not release promises. They exist because a tiny music box
should occasionally attempt something delightfully excessive, and because a
good stunt can expose weaknesses that polite test material never finds.

### The Space Shuttle challenge: decode a Danny Carey performance

Privately import a short, legally obtained excerpt from a Tool recording that
features Danny Carey, then ask SHR-DAW to analyze the full mix and build an
editable drum track from what it hears. The name is intentionally unserious;
the engineering challenge is not.

The experiment should:

1. detect likely drum transients without assuming a steady 4/4 grid;
2. propose tempo regions, irregular phrase ends, and groupings such as
   `2+2+3`, while showing uncertainty rather than inventing certainty;
3. separate kick, snare, hat/cymbal, and other-percussion candidates into the
   four tracker lanes;
4. retain estimated velocity and early/late timing instead of flattening the
   performance onto a rigid grid;
5. let the user audition, correct, shorten, grow, regroup, and simplify the
   result; and
6. derive a new, playable SHR-DAW groove that demonstrates what the analysis
   taught us without requiring the source recording during playback.

This challenge depends on the arbitrary-length, microtiming, swing, groove,
and expressive-capture work in the [post-competition rhythm
plan](POST_COMPETITION_RHYTHM_PLAN.md). Offline analysis must remain separate
from the real-time audio callback and must have bounded input length, memory,
and CPU use. A mixed commercial master may prevent reliable instrument
separation, so a useful partial transcription with visible confidence is a
valid result; pretending it is exact is not.

The imported audio and an exact derived transcription remain private below the
user-data boundary unless their redistribution rights are established. They
must not be committed, packaged, embedded in a demo, or presented as project
content. A public result must use newly authored/cleared audio and a genuinely
original groove rather than redistributing the Tool excerpt or a note-for-note
copy of the performance.

Success is not “replace Danny Carey.” Success is that SHR-DAW can inspect one
famously demanding rhythmic performance, explain its best hypothesis in plain
language, turn that hypothesis into editable tracker data, and help a musician
make something new. If the little box survives the Space Shuttle challenge,
ordinary drum loops should feel like a pleasant afternoon.

## Audio effects graph: inserts, sends, and returns

Phase 2 now includes the managed-source ordered insert rack, its essential
processors, strict Project persistence, stopped structural publication, compact
editors, and meters. Its software evidence and still-pending Pi/listening gate
are in [Phase 2 insert-effects measurement](PHASE2_AUDIO_GRAPH_MEASUREMENT.md).
Master inserts, auxes, hardware loops, and Phase 3 time/modulation processors
below remain deferred until their phase gates pass.

### Product idea

Effects should be reusable audio processors that can be placed deliberately in
the signal path, not hard-wired decorations on one synth. The routing model
should eventually support:

- an ordered **source insert chain**, such as synth → filter → drive → output;
- a **master insert**, such as all SHR-DAW sources → compressor/EQ → output;
- a shared **aux send/return**, where multiple sources retain their dry path
  while feeding a 100%-wet delay or reverb at independent send levels; and
- an optional **external hardware insert/send**, where a spare interface output
  feeds a pedal/rack processor and a capture input returns it; and
- a **live input strip**, where a physical JACK capture pair becomes a
  first-class source that can pass through the same inserts, sends, master,
  monitor, and recording choices as software sources while playback continues.

Insert and send are musically different. An insert replaces the source path and
usually exposes a wet/dry or bypass control. A send copies some of the source to
a shared effect while the dry signal continues to the master; its return must be
mixed exactly once. The UI and Project format should use those words rather than
hiding both behaviors behind a generic “effect” switch.

### Current architecture boundary

SHR-DAW does not yet own a common audio mixer. The managed software engine
connects its stereo JACK output directly to configured playback destinations.
The WAV loop client independently connects its stereo output to destinations,
and the recorder captures one configured stereo input pair.

The official JACK API documents that when an input port has multiple inbound
connections, JACK mixes those buffers. This makes a useful first graph smaller
than a general mixer:

- a single-source insert can route that source through an owned effect client;
- a master insert can connect multiple SHR outputs to one stereo effect input,
  which JACK sums, then connect only the effect output to playback;
- a basic global aux can preserve each dry playback connection, also connect
  sources to a 100%-wet effect input, and connect its return to playback, where
  JACK mixes dry sources and the wet return; but
- independent per-source send levels still require scaled send outputs,
  per-source effect input pairs, or owned gain/tap clients before summing.

A fuller owned mixer is still needed for independent source gain/pan, metering,
mute/solo, stable record points, and more complex buses. Configuration alone
cannot own graph transitions, control headroom, or prevent feedback.

Primary source: [JACK 2 `jack_port_get_buffer` API
contract](https://github.com/jackaudio/jack2/blob/develop/common/jack/jack.h),
which specifies appropriate mixing for multiple inbound connections.

The graph owner must connect and disconnect only SHR-owned ports, refuse
ambiguous endpoints, restore a safe graph after client loss, and never alter
unrelated JACK connections.

### Free wiring and first-class inputs

The long-term model should be a validated audio patch bay rather than separate
special cases for synth, loop, and recorder:

- **sources:** managed engine, WAV loop, physical capture input, and owned
  effect/hardware returns;
- **processors:** gain/pan, meters, insert effects, send taps, wet effects, and
  optional master processing; and
- **sinks:** physical playback, pre-effect recording, post-effect recording,
  and explicitly configured hardware sends.

A proper live-input client would register stereo JACK inputs connected from the
configured capture ports and stereo JACK outputs that enter the same processor
graph as the other sources. With a full-duplex device and JACK configuration,
capture and playback can run simultaneously: an external synth, microphone, or
hardware return can be processed, monitored, and recorded through SHR routing.
That behavior must be proven on the actual device rather than inferred from the
presence of input and output names.

“Free wiring” should mean that users may compose any valid acyclic route from
available sources, processors, and sinks. It must not mean that SHR silently
accepts a feedback cycle, connects an output to itself, creates two monitor
paths, or rewires unrelated JACK clients. Validate the proposed graph before
publishing it, reject unsafe cycles and ambiguous ports, and switch from the old
graph to the new graph with a bounded mute/fade strategy so partial connection
failure does not leave a loud, doubled, or silent path.

Input monitoring must explicitly distinguish:

- **hardware/direct monitoring**, which bypasses SHR processing and has the
  lowest device latency;
- **software monitoring**, which routes capture through SHR effects and back to
  playback; and
- **record-only input**, which captures without returning audio to playback.

Enabling hardware and software monitoring together can double the dry input or
create feedback in an external loop. The UI and Project must make the selected
mode visible. Recording should explicitly choose pre-insert, post-insert, or
master output instead of silently changing what is captured.

### Candidate topologies

1. **Per-source software insert:** `source → effect chain → selected output`.
   This is the smallest useful topology and avoids a global mixer, but each
   source needs its own instance or shared-chain ownership rules.
2. **JACK-summed master insert:** `sources → stereo effect input → output`.
   JACK can sum the sources at the input, making this a plausible small spike.
   Graph switching must remove old dry connections without a doubled or silent
   intermediate state; explicit gain/metering is still needed for headroom.
3. **JACK-summed global aux:** keep `source → dry playback`, also connect
   sources to `wet effect → playback`. This proves send/return cheaply, but one
   effect input gain is a global send amount. Independent source send levels
   need extra ports or owned gain/tap stages. A delay/reverb return should be
   100% wet so dry audio is not doubled inside the effect.
4. **External hardware loop:** `owned send output → processor → owned return
   input → SHR mixer`. This consumes physical I/O, adds conversion/round-trip
   latency, needs line-level compatibility, and must detect or structurally
   prevent feedback. Direct monitoring must not create a second return path.
5. **Live input chain:** `capture input → inserts/sends → monitor/master`, with
   optional `capture → pre-FX recorder` or `processed output → post-FX
   recorder`. This proves simultaneous input/output routing and makes external
   instruments part of the same workflow rather than recorder-only signals.

Chain order must be explicit because filter → drive, drive → filter, and
compressor → delay are musically different. Bypass, reorder, add/remove, and
Project load must not click, lose ownership, or leave stale JACK routes.

### Small effect candidates

The first implementation should prove the graph and real-time contract rather
than attempt a whole suite. Candidates to compare are:

- gain/pan plus peak metering and a conservative safety limiter, primarily as
  routing/mixer infrastructure;
- a bounded stereo delay with feedback, time, tone, wet level, and guarded
  gain, which makes send/return behavior easy to demonstrate;
- a filter or restrained drive insert with parameter smoothing; and
- chorus or reverb only after the simpler graph is stable and measured.

Objective DSP measurement can establish whether an effect meets its declared
response, curve, timing, stability, artifact, and real-time targets. It cannot
decide the remaining musical preference question. Codex provides a technical
recommendation from those measurements; the human performs the final low-gain,
level-matched musical curation.

### Raspberry Pi metric plan

Desktop development can accelerate unit tests and DSP prototyping, but only
release-mode Raspberry Pi measurements count for the product claim. Establish
an idle/bypass baseline, then measure 1, 2, and 4 instances where the topology
allows it. At minimum record:

- JACK sample rate, period size, periods, and the callback time budget
  (`period_frames / sample_rate`);
- callback mean, p95, p99, and maximum duration using lock-free counters read
  outside the callback;
- JACK xruns and deadline misses over a sustained run;
- process/core CPU, isolated audio-core utilization, RSS, and bounded effect
  memory;
- added algorithmic latency and, for an authorized hardware loop, measured
  round-trip latency;
- sustained simultaneous capture/playback behavior, input-to-output latency,
  and whether direct monitoring creates a doubled path;
- peak/RMS before and after, NaN/non-finite protection, clipping, and feedback
  containment;
- bypass/reorder/load discontinuities and click risk;
- sample-rate changes, client loss/reconnect, panic, stop, and clean shutdown;
  and
- the final demo graph under the exact song workload rather than a silent
  microbenchmark alone.

At 48 kHz, a 128-frame JACK period is about 2.67 ms; 64 frames is about 1.33
ms, and 256 frames about 5.33 ms. Those are total callback deadlines, not CPU
budgets available entirely to one effect. Report observed settings and results
rather than promising latency in advance.

### Real-time acceptance gates

- No allocation, locks, file I/O, subprocess calls, logging, or panics in the
  JACK callback.
- Fixed/bounded buffers, finite-value guards, denormal handling where needed,
  parameter smoothing, and safe feedback limits.
- No connection to an effect means a predictable dry path; a crashed effect
  must not leave destructive feedback or an unrecoverable silent graph.
- Bypass and shutdown are click-conscious and release every owned JACK resource.
- Project/config migration is versioned and atomic; unknown newer formats are
  refused.
- The 40×20 workflow exposes only the controls needed to understand and perform
  the chain.
- Free-wiring publication is transactional: validate the complete graph first,
  then connect it without leaving a partial, cyclic, or doubled route.

### PC/Pi split

If DSP work begins on a development PC, keep it in a separate Git
branch/worktree so it cannot destabilize the submission checkout. Record that
development split truthfully. A feature may enter SHR-DAW only after the same
locked formatting, tests, warning-denied Clippy, optimized build, non-audible
graph tests, and authorized performance measurements pass on the Raspberry Pi.

## External MIDI routing

### Optional multi-target live thru

FT2 playback already routes every page to its own `(MIDI output, channel)`, so
two instruments on separate physical MIDI outputs may use the same receive
channel without interfering. Step-edit audition intentionally follows only the
selected page, while normal live thru follows the single configured external
output.

A future opt-in live-routing layer could send or split controller performance
input across several page targets. It must retain exact target/channel/note
ownership, consume command pads, prevent doubled routes, and send correct note
offs during target changes, stop, panic, and disconnects. The default should
remain a single destination so enabling a second interface never layers synths
unexpectedly.

### Stable identity for identical USB-MIDI adapters

Exact ALSA MIDI output names distinguish different interfaces today. Two
different named ports work independently, but identical adapters can expose
indistinguishable names. SHR-DAW now refuses ambiguous exact or partial matches
instead of selecting the first one.

A future device-alias system could bind user-facing names such as `CASIO OUT`
and `D-50 OUT` to stable USB/ALSA card and port identity, preserve those aliases
across reconnects, while preserving the current refusal to guess. It should
remain configuration data rather than adding hardware names to Rust constants.
