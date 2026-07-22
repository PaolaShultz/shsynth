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

The loaded standalone instrument stays running when you leave Presets or
Playback, so it remains available for effects and other screens. Global panic,
application shutdown, loading a replacement, or entering FT2 with a different
explicit software route still ends it safely. Entering a genuinely new, empty,
unsaved default FT2 Project assigns the current Player engine/instrument to page
1 without restarting it. With no Player instrument loaded, FT2 loads the first
available synthv1 preset itself. Saved or explicitly changed Projects keep their
own routes, including saved Projects with empty note grids.

## Learning by exploration

Playback N00B toggles a root plus major or natural-minor scale filter without
leaving Player. While it is on, the same screen adds one compact `SCALE` rotary;
turning the master encoder cycles the scale while every normal Player control,
chord, note, velocity, and keyboard display remains visible. Allowed notes keep
their pitch, other notes stay silent, and pressing N00B again restores chromatic
play.

FT2 toggles the same selected scale directly in Play, Record, and Edit on
melodic pages; it never opens another workspace or changes the current mode.
Record and Edit write only allowed notes; Edit note length remains
independent from its row advance. Moving to Drums turns the filter off.

```text
press -> hear -> see notes -> read a chord name -> change -> compare -> ask why
```

## Screens

Home opens Software Synths, FT2, Recorder, Performance, MIDI Learn, Routing,
Effects, Ideas, or Help. The master rotary browses the current content; press
it to select or confirm. Back returns one level, and controller MIDI never
quits the application.

Software Synths leads from Presets to Playback and MIDI Ideas. FT2 owns
Patterns, pages, Arrangement, Projects, drums, and the fourth musician-facing
Loop Player page. Recorder captures raw synchronized stems; Performance owns
the optional final bus and stereo recording. Routing edits machine choices,
while MIDI Learn configures the controller without forwarding learned input.

Temporary master overlays keep their caller visible and cancel unconfirmed
drafts on close. For every screen, controller page, shortcut, and exact loop
workflow, use the [visual menu manual](MENU_MANUAL.md), [tracker
guide](TRACKER.md), and [controller interface](CONTROLLER_INTERFACE.md).
Controller-clock setup belongs in [Configuration and
routing](CONFIGURATION.md#dedicated-controller-clock-and-transport).

## MIDI ideas

Ideas capture free playing as MIDI. Each saved idea keeps its timing and
instrument identity. A synthv1 idea includes its own private preset snapshot;
other engines retain the external instrument reference. Mapped synthv1 control
values are saved too. Loading an idea can replace the current managed engine,
then TAKE plays through the restored idea instrument rather than an arbitrary
active engine.

## Effects and routing

Open Effects from Home, Playback **SYS** → **FX**, or FT2's page tools; Back
returns to the caller. `TARGET` selects SOURCE, AUX 1, AUX 2, or MASTER.

Source effects process the managed instrument in series. Aux sends make
parallel wet-only copies, and the master rack processes the complete final-bus
sum. Project edits remain available with the graph disabled, but direct audio
does not process or meter them. With the graph active, stop transport and all
recording before changing FX. See [How SHR-DAW
works](HOW_IT_WORKS.md#the-managed-audio-graph) for placement, effect choices,
bypass, and routing.

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

Open Performance from Home, or press `m` on Presets. With the graph disabled,
MTR shows passive whole-core CPU and legacy meter information. With it enabled,
MTR controls Synth/Loop/Input levels and mutes, master level, the linked
limiter, `FINAL OUT`, and final stereo recording.

`FINAL OUT` is the post-limiter buffer shared by recording and playback. The
Loop Player's separate `LOOP OUT` still measures only its WAV. RESET clears
presentation holds, not audio state. MTR does not report callback timing or
xruns; see [Final performance bus](FINAL_PERFORMANCE_BUS.md) for the exact
meter, limiter, monitoring, recording, and control contract.

All horizontal meters use circular `●` LEDs. Dark gray means unlit; one green
marks safe active level; yellow and red appear only at their active thresholds;
and a held peak is a brighter circle of the same threshold colour.

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
shr config paths
shr config init [--force]
shr effects-checkpoint ENGINE:PRESET [PROFILE] [SECONDS]
```

`shr config init` preserves existing configuration; `--force` deliberately
replaces both runtime and controller files with current defaults. The complete
CLI inventory, including maintenance stress and screenshot commands, is in
`shr --help`; their safety contracts are in the focused recording, final-bus,
and maintainer-helper documents.

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
