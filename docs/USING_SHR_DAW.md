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

## Screens

- **Presets** chooses an engine and sound.
- **Playback** shows played notes and keyboard state, changes synthv1 controls,
  and records ideas.
- **MTR** passively shows CPU-core activity and, when available, the owned
  graph's stereo final-output level.
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
- **Audio Recorder** records the configured stereo JACK input.
- **FX Rack** shapes the managed instrument with source inserts, two parallel
  pre/post aux sends and wet returns, then a master rack and final meter.

The display shows the current screen, menu page, and four available actions.
Empty actions are hidden. The main encoder moves through lists, rows, pages,
and values. Its press selects or confirms.

Physical menu layouts with four, five, or eight buttons are supported. Read
the [screen and menu manual](MENU_MANUAL.md) for the complete visual tour and
the [Controller interface](CONTROLLER_INTERFACE.md) for the implementation
contract behind every action and menu.
Press `?` or F1 from the keyboard to open the same in-app help. The Help screen
tries to start `http://<LAN-IP>/help` on port 80 only while Help is open; if the
port or network is unavailable, the local Help screen keeps working.

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
transport and all recording before publishing an FX change. The current graph
contains only the managed software instrument; the WAV loop, recorder input,
and external-instrument audio do not pass through these effects. Read
[How SHR-DAW works](HOW_IT_WORKS.md#the-managed-audio-graph) for the complete
route and sound-oriented effect guide.

## Stereo audio

The Audio Recorder writes the selected JACK stereo pair as a 24-bit WAV file.
The screen shows recording time, sample rate, file size, dropped frames, and
errors.

If recording is interrupted, the temporary file remains with a `.wav.part`
name. On the next recording start, SHR-DAW recovers complete frames from a
recognized capture header and reports unrecognized partial files without
silently deleting them.

At RIFF's 4 GiB size limit, recording stops and finalizes the last complete
stereo frame instead of producing an invalid WAV file.

External line input is intended to use the audio interface's direct-monitor
feature. See [Physical connections](CONNECTIONS.md) for the audio path.

## Performance meters

Open MTR from the first item on Presets NAV, or press `m` on Presets. Its four
CPU rows normally show CPU0–CPU3 on a Raspberry Pi. They are calculated from
changes in Linux `/proc/stat`, not by running a command. Green is below 60%,
yellow is 60–85%, and red is above 85%. If fewer cores or no Linux statistics
are available, the missing rows say `n/a`. A configured CPU-temperature sensor
is shown too, but MTR does not require one.

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

`FINAL OUT` is truthful only while the owned audio graph is active. It is the
graph master after the managed software-instrument source, its wet aux returns,
and master inserts. The current graph does not include the separate WAV loop
player, hardware returns, recorder inputs, or unrelated JACK clients. In direct
playback MTR explicitly reports that output metering is unavailable because
there is no safe owned tap; it never enables the graph or fakes movement.

The normal FT2 WAV Loop screen has a second, independent stereo meter labelled
`LOOP OUT`. Its solid bars are smoothed RMS, its thin markers are short-held
peaks, `MAX` is the highest left/right loop peak in the current transport
session, and `CLIP!` is held visibly. It measures the loop callback after the
selected region, interpolation, transport gate, and edge fades, just before
the loop's existing JACK outputs. Stopping, unloading, a load failure, or a
lost loop client clears it. It does not include synths, effects, inputs,
hardware gain, or any other JACK client, and it does not make the loop part of
`FINAL OUT`.

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
