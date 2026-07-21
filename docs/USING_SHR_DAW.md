# Using SHR-DAW

SHR-DAW is designed to feel like a small hardware music appliance. After setup,
the main performance and sequencing workflow can be used without a mouse or
computer keyboard.

## Instruments

The Presets screen browses three kinds of software instruments:

- synthv1 presets;
- Yoshimi `.xiz` banks;
- FluidSynth `.sf2` and `.sf3` SoundFonts.

Changing engine does not mix their files or control rules. The Playback screen
shows held note names with each note's decimal MIDI velocity directly beneath
it, chord names, a continuous keyboard-state strip, MIDI idea recording, and
the 12 mapped synthv1 controls. This is a practical way to practise soft/loud
key control, even chord attacks, and bass-plus-chord balance. It reports the
controller's MIDI strike velocity (1–127), not measured audio loudness; the
instrument and its settings decide how strongly that velocity changes sound.
`display.note_names` in
`shsynth.conf` selects German `B`/`H` spelling or English `A#`/`B` spelling.
Recognized major triads use the explicit `maj` label, such as `C maj`, so the
display does not rely on an omitted suffix to teach the chord quality.

The loaded standalone instrument belongs to the Software Synth workspace.
Moving between Presets and Playback keeps it running; leaving Presets for Home
sends All Notes Off and unloads it. FT2 has separate Pattern-owned synth routing
and never treats this last standalone selection as its instrument.

## Learning by exploration

The beginner-friendly parts of SHR-DAW form one experience, but they do
different jobs. Playback is the immediate feedback surface. In N00B, the
learner chooses a chromatic root plus major or natural minor. Notes in that
scale sound; notes outside it are consumed and stay silent rather than being
shifted to another pitch. The screen names the chord currently held, lists the
allowed sounding notes and their MIDI strike velocities, and lights those
notes on the continuous keyboard. NORMAL restores unrestricted chromatic play.

For example, a learner can select C-sharp natural minor, find and hear `C#m`,
change one allowed note, watch the displayed chord name change, and compare the
result. Trying `E maj`, `A maj`, an added seventh, or an unfamiliar allowed
combination can lead to a personally meaningful question: why does this feel
settled, related, tense, or surprising? The display supplies names and visible
note shapes for discoveries the learner has already heard.

FT2 applies the same N00B scale gate as a switch layered over Play, real-time
Record, and Step Edit on the selected melodic page. In Play it filters what is
heard; in Record and Edit it also filters what reaches Pattern cells. Step Edit
still accepts one note or a gesture of up to four allowed notes and gives the
entry a familiar 1/1–1/32 length. Length and the 1/2/4/8-row cursor advance are
independent. A new player can plan a phrase, listen to it, preserve an
interesting accident, and revise it without first learning tracker command
syntax. N00B turns off when moving to a percussion page.

This is a product philosophy, not a promise about educational outcomes or a
claim that every child learns alike. The intended loop is simple:

```text
press -> hear -> see notes -> read a chord name -> change -> compare -> ask why
```

Theory is still valuable. SHR-DAW tries to let it arrive as an explanation for
something a person already cares about. That invitation is for children and
new music makers, but also for returning adults—including the creator—and for
Raspberry Pi experimenters learning a new musical or technical vocabulary.

## Screens

- **Home** is the startup and navigation root. Turn the master rotary or use
  Up/Down, then press it or Enter to open Software Synths, FT2, Recorder,
  Performance, MIDI Learn, Routing, Effects, Ideas, or Help. Its centered
  labels all use one fixed-width selection bar. Esc quits only from Home;
  controller MIDI never quits SHR.
- **Presets** chooses an engine and sound.
- **Playback** shows played notes and keyboard state, changes synthv1 controls,
  and records ideas.
- **MTR** is the compact three-source final-bus surface when the graph is
  enabled; otherwise it retains the passive CPU/legacy meter view.
- **Ideas** loads, plays, saves, and deletes MIDI ideas.
- **Help** shows compact user help; turn the encoder through rows and press a
  highlighted link to jump sections. When possible, it also shows a temporary
  LAN URL for the same help page.
- **FT2** edits and plays patterns.
- **Tools** opens page, file, arrangement, loop, and clipboard workflows.
- **Tracks** (opened with **Tools** → **Pages**) adds four-lane pages, chooses one destination, and edits each
  column's channel, bank, master program, and profile-provided instrument name.
- **Files** manages Projects; its **Pattern** child groups pattern editing,
  melody-only transpose, and the separate reusable drum-pattern library. Drum
  filters choose genre, meter, and 2/4/8-bar phrase size. It also names and
  renames Projects and cleans only zero-reference Pattern records.

Ten cleared demo Projects appear in **Files** after setup. Their five `AUTO`
pages follow this machine's current MIDI defaults, and matching MIDI exports
plus provenance live in the XDG demo directory.
- **Arrange** edits the ordered pattern steps separately from pattern data.
- **Loop** imports, trims, aligns, and plays a private WAV with the tracker,
  with a separate stereo `LOOP OUT` meter for that WAV alone;
  **Library** separately deletes only unreferenced regular WAV files.
- **Audio Recorder** maps, names, and arms exact JACK inputs for one synchronized
  multistem take.
- **FX Rack** shapes the managed instrument with source inserts, two parallel
  pre/post aux sends and wet returns, then a master rack and final meter.
- **Routing** shows the current controller, tracker output/profile,
  controller-clock, and audio-output state. It is deliberately read-only:
  leave SHR and run `shr-setup` to change hardware routes safely.
- **MIDI Learn** opens directly from Home and learns rotary left, right, then
  click before browsing optional mappings.

MIDI Learn is highlighted initially when a configured controller is offline,
has no matching reviewed profile, or has an incomplete learned encoder. The
reason appears directly on Home. A learned controller is considered navigable
once encoder turn and click work; skipped optional command mappings do not make
it a failure. Opening Home never starts learning or sends MIDI, and saving a
new mapping still requires confirmation in the isolated Learn workflow.

Home is a plain centered inverted-selection list. Other screens show their current
controller page and available actions; empty actions and pages are hidden. The
main encoder moves through lists, rows, pages, and values, and its press selects
or confirms. Back from a top-level workspace returns Home. Nested tools and
editors return one level at a time, without stopping unrelated playback or
recording state.

On large sound, Project, drum-pattern, and WAV lists, an otherwise unassigned
letter jumps to the first matching name without taking input away from naming,
numeric entry, or editor/modal fields. Keyboard PageUp/PageDown keep their
existing list/tracker behavior. Physical command pages omit PageUp/PageDown so
pads stay focused on the current musical workflow.

Physical menu layouts with four, five, or eight buttons are supported. Read
the [screen and menu manual](MENU_MANUAL.md) for the complete visual tour and
the [Controller interface](CONTROLLER_INTERFACE.md) for the implementation
contract behind every action and menu.
Press `?` or F1 from the keyboard to open the same in-app help. The Help screen
tries to start `http://<LAN-IP>/help` on port 80 only while Help is open; if the
port or network is unavailable, the local Help screen keeps working.

For the MiniLab 3, the reviewed factory Arturia/DAW pads use notes 36–43 on
channel 10 for page 1–4 and item 1–4. Keyboard/channel-1 notes—including the
captured User 1 pads—remain musical. DAW Shift/CC27 is not pad lock.

When `controller_clock.enabled=true`, leave the MiniLab arpeggiator at External
Sync and start normal FT2 transport. While SHR is open it sends the
current/default tempo at 24 PPQN; transport adds the required Start/Stop state
through the dedicated standard MIDI endpoint. An empty Pattern is allowed for
live arpeggiator playing. Stop sends Stop but clock continues so the MiniLab is
ready before the next Play; enabling the feature is the explicit clock-run
switch. See
[Configuration and routing](CONFIGURATION.md#dedicated-controller-clock-and-transport)
for the Raspberry Pi setup, backup, disable, and rollback procedure.

## MIDI ideas

Ideas capture free playing as MIDI. Each saved idea keeps its timing and
instrument identity. A synthv1 idea includes its own private preset snapshot;
other engines retain the external instrument reference. Mapped synthv1 control
values are saved too. Loading an idea can replace the current managed engine,
then TAKE plays through the restored idea instrument rather than an arbitrary
active engine.

## Effects and routing

From Playback, open **SOUND** → **FX**. `TARGET` moves among SOURCE, AUX 1,
AUX 2, and MASTER. A source rack changes the managed instrument in series; it
is the natural place for EQ, compression, filtering, distortion, gating, or
other processing that belongs to that sound. The master rack processes the
complete managed-source-plus-aux mix immediately before `FINAL OUT`.

An aux is a parallel path for space or motion. `SEND` controls how much of the
instrument enters it, `POINT` chooses before or after the source inserts, and
`RETURN` controls how much processed sound comes back. The aux editor offers
Delay, Reverb, Chorus, Flanger, and Phaser and forces them wet-only, so the
return cannot accidentally add a second dry instrument. A new aux begins with
a conservative -18 dB post-insert send.

Source and master racks also offer Utility, EQ, Compressor, Distortion,
Tremolo/Pan, Gate, Filter, and Crusher as well as the five time/space effects.
Each rack is ordered: filter before distortion sounds different from distortion
before filter. Source/master bypass fades toward clean passthrough. A fully
bypassed aux fades to silence; a delay can optionally stop new input and let
its already-created wet tail decay.

The rack remains editable and saved when the opt-in graph is disabled, but the
direct audio path cannot process or meter it. With the graph active, stop
transport and all recording before publishing an FX change. Source/aux effects
belong only to the managed instrument. The loop and configured stereo input
join it before the master rack, so master effects and the protected final stage
cover the complete three-source sum. Read
[How SHR-DAW works](HOW_IT_WORKS.md#the-managed-audio-graph) for the complete
route and sound-oriented effect guide.

## Synchronized audio stems

The Audio Recorder writes every armed exact source as a separate mono 24-bit
WAV with one shared timeline and manifest. Select a musician-friendly track,
assign a discovered source deliberately, name it, and arm it. A missing exact
preference remains `missing` and blocks start until assigned or disarmed.
The screen shows elapsed time, armed count, selected-track level, writer
high-water, drops, overflows, xruns, saved path, and errors.

If recording is interrupted, the temporary `*.take.part` session remains. On
the next start, recognized mono stems recover only their common complete frames
and publish as `recovered-incomplete`; unknown or unsafe data is reported and
not silently deleted.

Each mono file has its own RIFF limit. Any overflow, callback violation, source
loss, xrun, writer/storage error, or mismatched finalization prevents the take
from appearing complete.

This raw-stem workflow is separate from MTR's final-mix recorder. The latter
writes one 24-bit interleaved stereo WAV containing the exact limited playback
samples. See [Final performance bus](FINAL_PERFORMANCE_BUS.md).
See [Synchronized multitrack recording](MULTITRACK_RECORDING.md) for exact
configuration, session layout, recovery, and hardware-free stress validation.

## Performance meters

Open MTR from the first item on Presets NAV, or press `m` on Presets. When the
owned graph is disabled, its four CPU rows retain the legacy passive display.
When enabled, the 40×20 surface instead shows the three-source performance bus:
selected source level/mute/readiness, master level, final L/R meter and clip,
limiter gain reduction, and final recording time/size/error/path.

The friendly stereo VU display is labelled in dBFS. Its solid body is live,
smoothed RMS; the thin marker is a short peak hold that later decays. The
`MAX` number is separate: left and right independently retain their highest
detected peak and never decay merely because the signal becomes quieter or
time passes. `CLIP!` is held visibly after a full-scale sample. The scale runs
from −60 to 0 dBFS; green is below −12 dBFS, yellow is −12 through −3 dBFS,
and red is above −3 dBFS.

RESET clears the two `MAX` numbers, short peak markers, and clip hold without
touching audio, effects, engines, or JACK routes. Turning the mapped synthv1
Volume control down also clears both `MAX` numbers. This happens on every
downward physical movement, even while pickup is still waiting and the Volume
change is blocked; increases, unchanged values, and other controls do not clear
them. A new sound/engine session, a stopped engine, direct unmetered playback,
or a lost meter starts with no maximum from the previous session.

`FINAL OUT` is truthful only while the owned graph is active. It follows the
managed instrument (including its wet aux returns), owned WAV loop, configured
stereo input, master rack/level, and linked limiter. Its samples are the exact
buffer sent to both the final recorder and playback. It excludes unrelated
JACK clients and downstream interface processing. Direct playback reports the
bus unavailable; it never enables the graph or fakes movement.

The normal FT2 WAV Loop screen keeps its independent `LOOP OUT` meter for the
loop alone. An active bus moves those same owned loop outputs away from their
direct playback links and includes them exactly once in `FINAL OUT`.

Use SOURCE-/SOURCE+ to select Synth, Loop, or Input; LEVEL-/LEVEL+ changes the
selected source by 1 dB and MUTE toggles it. REC starts/stops one final stereo
WAV at callback boundaries. All faders are smoothed. There is deliberately no
pan, solo, arbitrary routing, or per-input processing. The exact limiter,
monitoring, latency, and recording rules are in
[Final performance bus](FINAL_PERFORMANCE_BUS.md).

The CPU bars are whole-core system load, not CPU used by the synth or graph.
MTR deliberately does not measure JACK callback duration, xruns, scheduling
latency, or whether a particular effects chain is safe. Those require the
maintainer performance checkpoint and JACK evidence described in the
[audio graph contract](AUDIO_GRAPH.md).

## Command line

The main program also provides these commands:

```sh
shr menu
shr list
shr status
shr doctor
shr start "synthv1:Velvet Tines"
shr start "Yoshimi:Fat Bass"
shr stop
shr log 80
shr ideas list
shr ideas inspect "idea-name"
shr ideas play "idea-name"
shr ideas delete "idea-name" --yes
shr pads list
shr pads ports
shr pads profiles
shr pads auto [PORT_MATCH]
shr pads learn [PORT_MATCH]
shr pads update
shr clock ports
shr casio diagnostic
shr config init
shr effects-checkpoint ENGINE:PRESET [PROFILE] [SECONDS]
```

`shr casio diagnostic` keeps an old name from the first hardware test. It does
not send MIDI. It lists output ports and shows the messages that would be used.
The tracker itself is device-neutral. Command-line idea playback restores the
saved instrument and can be stopped with Ctrl+C; deletion always requires the
explicit `--yes` argument.

`config init` creates missing configuration from templates without replacing
existing files. `effects-checkpoint` is a maintainer-only, bounded, low-gain
measurement workflow; it requires explicit authorization and a prepared JACK
session, and normal use should not run it. See
[Configuration and routing](CONFIGURATION.md#owned-audio-graph).

For pattern editing, continue with the [Tracker guide](TRACKER.md).
