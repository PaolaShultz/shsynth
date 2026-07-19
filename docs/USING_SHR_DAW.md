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
shows held notes, chord names, MIDI idea recording, and the 12 mapped synthv1
controls.

## Screens

- **Presets** chooses an engine and sound.
- **Playback** shows played notes, changes synthv1 controls, and records ideas.
- **MTR** passively shows CPU-core activity and, when available, the owned
  graph's stereo final-output level.
- **Ideas** loads, plays, saves, and deletes MIDI ideas.
- **Help** shows compact user help; turn the encoder through rows and press a
  highlighted link to jump sections. When possible, it also shows a temporary
  LAN URL for the same help page.
- **FT2** edits and plays patterns.
- **Tools** opens page, file, arrangement, loop, and clipboard workflows.
- **Pages/Tracks** adds four-lane pages, chooses one destination, and edits each
  column's channel, bank, master program, and profile-provided instrument name.
- **Files** manages Projects; its **Pattern** child groups pattern editing,
  melody-only transpose, and the separate reusable drum-pattern library. Drum
  filters choose genre, meter, and 2/4/8-bar phrase size. It also names and
  renames Projects and cleans only zero-reference Pattern records.
- **Arrange** edits the ordered pattern steps separately from pattern data.
- **Loop** imports, trims, aligns, and plays a private WAV with the tracker;
  **Library** separately deletes only unreferenced regular WAV files.
- **Audio Recorder** records the configured stereo JACK input.

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

The friendly stereo VU display is labelled in dBFS: its solid body is smoothed
RMS, the thin marker is peak hold, and `CLIP!` is held visibly after a full-scale
sample. The scale runs from −60 to 0 dBFS; green is below −12 dBFS, yellow is
−12 through −3 dBFS, and red is above −3 dBFS. RESET clears only the visual
peak and clip holds. It does not touch audio, effects, engines, or JACK routes.

`FINAL OUT` is truthful only while the owned audio graph is active. It is the
graph master after the managed software-instrument source, its wet aux returns,
and master inserts. The current graph does not include the separate WAV loop
player, hardware returns, recorder inputs, or unrelated JACK clients. In direct
playback MTR explicitly reports that output metering is unavailable because
there is no safe owned tap; it never enables the graph or fakes movement.

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
shr pads update
shr casio diagnostic
```

`shr casio diagnostic` keeps an old name from the first hardware test. It does
not send MIDI. It lists output ports and shows the messages that would be used.
The tracker itself is device-neutral. Command-line idea playback restores the
saved instrument and can be stopped with Ctrl+C; deletion always requires the
explicit `--yes` argument.

For pattern editing, continue with the [Tracker guide](TRACKER.md).
