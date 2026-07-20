# Post-competition mixer and shared-aux plan

> **Historical proposal boundary:** SHR-DAW now has a deliberately narrow
> three-source final performance bus and Project format 4. The general
> multi-strip/shared-aux, pan/solo, hardware-insert, and free-routing work below
> remains a proposal, not current behavior. See
> [Final performance bus](FINAL_PERFORMANCE_BUS.md).

This plan was written after the competition appliance release. Do not treat its
historical format/topology descriptions as the current implementation.

Two narrow correctness issues found during the routing audit are listed under
the release boundary below. They can be repaired independently of this plan.

## Historical baseline when this plan was written

At the time, the owned graph had this signal flow:

```text
managed synth -> SOURCE inserts ---------------------------> sum
       |              |                                      |
       |              +-> post-insert send -> AUX 1/2 -> return
       +-----------------> pre-insert send  -> AUX 1/2 -> return

sum (source dry path plus each return exactly once)
    -> MASTER inserts
    -> configured stereo playback
```

This is not merely a master insert. It has one source rack, two independent
pre/post sends, two return gains, two aux racks, and one master rack. Delay and
reverb expose independent dry and wet percentages in SOURCE and MASTER. On an
aux they are forced to 0% dry and 100% wet; chorus, flanger, and phaser use the
equivalent 0% dry and 100% effect mix.

The important historical boundary was that the source was exactly one managed software
instrument. The WAV loop player remains directly connected, the recorder
captures its separately configured stereo input, and external instruments have
no owned audio return unless the user has arranged one outside this graph.
Project aux sends consequently have no source/strip ID.

Tracker lanes are MIDI lanes, not isolated audio channels. Several lanes sent
to one synth or one stereo hardware return have already been mixed by that
device, so SHR cannot give those lanes different audio sends afterward. A lane
can acquire its own mixer strip only when it has an independently owned audio
output or return.

## Release boundary and completed narrow corrections

This release boundary is retained as history; it no longer specifies the
current Project format or final-bus topology.

The audit found two correctness issues. They were repaired in the competition
graph without changing Project format 3, its one-managed-source topology, its
effect limits, or its JACK ownership/fallback behavior:

1. A dedicated post-master meter node now follows the final master effect and
   directly feeds the playback sink. An empty master rack still passes through
   that node, so `MASTER` and `FINAL OUT` measure the signal actually published
   for playback rather than the pre-insert sum.
2. Runtime aux state now records effect placement, wet-generator activity, and
   tail eligibility. A bypassed sole wet generator fades toward silence rather
   than dry. Tail-enabled delay drains only its wet history while its input
   fades closed. A bypassed processor may pass through when another active or
   draining wet generator keeps the bus wet-safe; this preserves an already-wet
   signal through downstream conditioning and lets another active generator in
   a serial chain receive the send. When every generator is bypassed and no
   tail is draining, the return is silent. SOURCE and MASTER keep their former
   click-conscious dry-passthrough bypass.

Deterministic topology, DSP, tail, serial-chain, Project-load, and callback
allocation tests cover these semantics. The post-competition phases below
still describe future work and remain outside the competition release.

## Product decisions

Use two global stereo aux buses. Each eligible audio strip gets an independent
send to each bus; each bus owns one shared effect chain and one return to the
master mix. The normal musical defaults are:

- source strip: tone, dynamics, utility, distortion, and other genuinely
  source-specific inserts;
- AUX 1: shared 100%-wet reverb;
- AUX 2: shared 100%-wet delay, or another deliberately chosen send effect;
- master: utility, corrective EQ, bus compression, and an optional measured
  safety limiter.

Reverb and delay can technically be used as inserts when they provide dry/wet
controls, which the current processors do. The normal UI should nevertheless
guide shared ambience and echoes to AUX 1/2. Do not automatically put a reverb
or delay on MASTER. Existing Project data must remain loadable even if the new
UI treats an old master time effect as an advanced/legacy placement.

An aux may need ordinary conditioning around its wet generator. Allow safe
processors such as EQ, filter, compressor, or utility before/after it, while
requiring at least one enabled wet generator and forcing every time/modulation
generator on that bus to wet-only operation.

## Stable Project model

Introduce a new Project format only when the whole migration is implemented.
Use stable non-zero IDs rather than vector positions.

```text
MixerProject
  strips[]
    id
    source_binding
    name
    trim_db, pan, mute, solo
    insert_rack
    sends[2] { aux_id, level_db, tap }
  aux_buses[2]
    id, name
    rack
    return_gain_db, return_pan, mute
  master
    gain_db
    rack
  recording_tap
```

`source_binding` initially covers the managed instrument, owned WAV loop,
configured live stereo input, and configured hardware return. Exact JACK names
remain in runtime configuration, not Project data or Rust constants.

Replace the current ambiguous pre/post wording with explicit tap points:

- `PreInsert` for a special unaffected feed;
- `PostInsertPreFader` for a monitor-style send independent of strip fader;
- `PostFader` as the normal musical default, following strip level and pan.

The mix equation is explicit and testable:

```text
strip post-fader dry paths
  + AUX 1 return exactly once
  + AUX 2 return exactly once
  -> master gain/rack
  -> final meter
  -> playback and optional post-master recorder tap
```

Project format 3 migration creates one managed-instrument strip, moves the
existing source rack onto it, attaches its existing two sends to that strip,
and preserves the aux and master effect instance IDs and parameters. Unknown
newer formats and malformed source bindings remain non-overwriting failures.

## Implementation phases

### 1. Pure model and graph compiler

- Add `AudioSourceId`, bounded strip state, per-strip inserts, and two
  per-strip sends.
- Keep graph construction independent of JACK and test it with two, then four,
  synthetic stereo sources.
- Add real strip gain/pan/mute/solo and master gain nodes with smoothed changes.
- Add a dedicated post-master meter node. Do not infer final output from a
  convenient upstream mixer or effect meter.
- Retain global stable effect IDs and compatible DSP state across moves.
- Reassess the current 4-source, 16-effect, 32-node, and 16 MiB bounds from a
  derived worst-case graph; raise a bound only with measured Pi evidence.

### 2. Owned source boundaries

- Give the owned JACK client one explicit stereo boundary pair per enabled
  source strip and one main output pair.
- Bring the managed synth through the first strip without changing its current
  fallback and ownership guarantees.
- Bring the owned WAV loop through another strip, replacing its direct route
  transactionally and restoring that exact route on failure.
- Add configured live input and hardware return only after monitoring mode,
  feedback prevention, and full-duplex hardware behavior are explicit.
- A single source loss must not rewire unrelated clients or double surviving
  dry paths.

### 3. Compact mixer and aux UI

- Add a `MIX` screen that selects a strip, AUX 1, AUX 2, or MASTER within the
  40x20 and four-page controller contract.
- Show strip source/name, pre/post level, pan, mute/solo, and both send levels.
- Show each aux's wet-only status, return level/mute, and return meter.
- Keep the existing FX editor as the chain editor for the selected strip/bus;
  make placement labels unambiguous.
- Make unavailable MIDI-only lanes say `NO AUDIO RETURN` instead of presenting
  a send that cannot affect their sound.

### 4. Tracker/source binding

- Bind a tracker page or destination to an audio strip only when its audio is
  independently observable.
- Treat multiple MIDI lanes feeding one stereo synth/return as one audio strip;
  never claim per-lane sends after the device has mixed them.
- Decide multi-engine or multi-output instrument hosting as its own ownership
  project. Do not weaken the current no-layering and clean-shutdown invariants
  merely to make every MIDI lane appear mixable.
- For external MIDI, require an explicit configured hardware-return binding;
  two devices sharing one return remain one strip.

### 5. Recording taps

- Connect recorder choices to real graph nodes: source pre-insert, source
  post-insert/pre-fader, source post-fader, selected aux return, or post-master.
- Keep record-only, direct monitoring, and software monitoring visibly
  distinct. Refuse accidental direct-plus-software doubling and feedback.
- Preserve existing recordings and configured capture behavior during Project
  migration.

### 6. Measurement and listening gate

- Prove dry identity, independent per-strip sends, one shared processor instance
  per aux, exactly-once returns, tap-point behavior, mute/solo, and truthful
  post-master metering with deterministic tests.
- Cover bypass and tail drain for one-effect and serial aux chains; no state may
  leak a raw send into a wet return.
- Prove allocation-free callbacks, finite recovery, client loss, partial route
  failure, stopped publication, and exact fallback restoration.
- Run release-mode Pi tests at 48 kHz with 128 and 64 frames using the maximum
  supported source count, both auxes, representative inserts, loop traffic,
  recording traffic, and realistic polyphony. Record p95/p99/max, deadline
  misses, xruns, per-core CPU, RSS, memory, and teardown behavior.
- Finish with low-gain, level-matched listening: source separation, send feel,
  reverb/delay return balance, pan law, mute/solo transitions, master headroom,
  and recorded-tap agreement.

## Completion criteria

The work is complete when every independently owned audio strip can feed either
of the same two aux processors at its own level, its dry path reaches the mix
once, each wet return reaches the mix once, the master processes the complete
mix, and the displayed/recorded post-master signal agrees with playback. MIDI
lanes without isolated audio must remain honestly identified rather than
receiving controls that cannot work.
