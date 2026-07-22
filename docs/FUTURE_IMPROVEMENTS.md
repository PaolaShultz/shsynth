# Future improvements

This file records useful extensions that are deliberately not part of the
current behavior. They are not required for separate FT2 pages to sequence
multiple hardware instruments simultaneously.

These entries do not enter the current 0.4, 0.5, or 0.6 scope automatically.
The [release roadmap](RELEASE_ROADMAP.md) owns development order; an item here
becomes a milestone requirement only when the owner explicitly moves it there.

## Safe fallback for unknown USB MIDI devices

When a USB MIDI input is connected without a saved or reviewed controller
profile, SHR should eventually offer or apply a useful fallback mapping instead
of leaving the device entirely unmapped. Reviewed profiles must remain the
preferred source, and fallback discovery must never silently overwrite the
user's controller configuration.

This needs a deliberately conservative design. Arbitrary notes and CCs must not
accidentally become transport, record, panic, or navigation commands. Musical
notes must continue to pass through unless the user deliberately assigns them
as channel-qualified commands. Safe continuous-control discovery should be
separate from command-button assignment: a knob or encoder can be proposed from
observed continuous traffic, while transport and other command buttons require
clear review or explicit learning before activation.

## Raspberry Pi 5 Headroom pass

An unscheduled post-Build Week pass will compare the current Raspberry Pi 4
development host with an ordered but not-yet-present 2 GB Raspberry Pi 5 and
NVMe setup. It will measure clean and incremental builds, memory, storage,
thermal/power behavior, private-cache benefits from real-time core placement,
effect state and callback cost, and dependency/library footprint before making
any optimization claim.

The proposal keeps one effects rack. Effects that later pass fixed low-state
and low-callback-cost gates may receive the compact `» PRESTO` mark; unmarked
effects remain normal first-class choices. No hardware result, marker, library
change, or release date is implied today. The complete boundaries and planned
experiment matrix are in the [Raspberry Pi 5 headroom and footprint
plan](PI5_HEADROOM_PLAN.md).

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

The current narrow performance bus now sums exactly the managed instrument,
owned loop, and one configured stereo input before the master, dedicated
limiter, final meter, recorder, and playback. The broader proposed migration to
a bounded multi-strip mixer with genuinely shared multi-source aux buses is in the
[post-competition mixer and shared-aux plan](POST_COMPETITION_MIXER_AUX_PLAN.md).
It also records the current dry/wet behavior, the audio-source boundary behind
tracker lanes, Project migration, and recording taps. The two narrow meter and
aux-bypass findings from that routing audit have also been repaired. The final
bus does not implement the broader strip/aux design.

The managed graph now includes the essential source inserts, delay/modulation,
three reverb voicings, two independently scaled pre/post aux sends and returns,
and an ordered master rack. It retains strict Project persistence, stopped
structural publication, compact editors, and meters. Evidence is in the
[Phase 2 insert-effects measurement](PHASE2_AUDIO_GRAPH_MEASUREMENT.md) and
[Phase 3/4 effects measurement](PHASE3_4_AUDIO_GRAPH_MEASUREMENT.md). The
three-source path has hardware-independent evidence, while full-duplex physical
interface acceptance remains deliberately deferred.

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

SHR-DAW now owns a bounded three-source stereo sum. The managed source's dry
path and two wet returns meet the owned loop and configured live-input pair,
then pass through the master, final limiter/meter/recorder and playback. The raw
synchronized multitrack recorder remains a separate workflow.

The graph uses internal preallocated mixer, send-tap, and return nodes rather
than relying on implicit JACK summing. That makes independent send/return gain,
pre/post placement, return metering, and exactly-once mixing explicit and
testable. The final bus adds only smoothed level/mute per source and master
level. A fuller mixer would still be needed for pan, solo, per-input inserts,
or shared aux sends, none of which is current product scope.

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

### Implemented foundation and remaining choices

Earlier revisions of this plan compared per-source, JACK-summed, and owned-mix
topologies and proposed a first small effect set. That selection work is now
historical: the current bounded implementation uses an owned exactly-once sum,
13 effect types, source/master serial racks, two wet-only aux racks, a
post-master meter, and transactional direct fallback. The authoritative
current behavior and limits live in the [audio graph contract](AUDIO_GRAPH.md),
not in this future-work page.

The choices still open here are genuinely future ones: how independently owned
loop, live-input, and hardware-return sources become mixer strips; how
monitoring and recording taps remain unambiguous; whether external hardware
inserts are safe and worthwhile; and how a validated free-wiring UI fits 40×13.
Chain order, bypass/tails, client loss, Project migration, and publication must
retain the current safety guarantees during that expansion. Objective DSP and
performance measurements can establish engineering fitness, but final
low-gain, level-matched musical curation remains a human listening decision.

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
- The 40×13 workflow exposes only the controls needed to understand and perform
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
