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
- **Ideas** loads, plays, saves, and deletes MIDI ideas.
- **Help** shows compact user help; turn the encoder through rows and press a
  highlighted link to jump sections. When possible, it also shows a temporary
  LAN URL for the same help page.
- **FT2** edits and plays patterns.
- **Pages** adds four-lane pages and chooses each destination and channel.
- **Files** manages songs, patterns, and the song order.
- **Audio Recorder** records the configured stereo JACK input.

The display shows the current screen, menu page, and four available actions.
Empty actions are hidden. The main encoder moves through lists, rows, pages,
and values. Its press selects or confirms.

Physical menu layouts with four, five, or eight buttons are supported. Read
the [Controller interface](CONTROLLER_INTERFACE.md) for every action and menu.
Press `?` or F1 from the keyboard to open the same in-app help. The Help screen
tries to start `http://<LAN-IP>/help` on port 80 only while Help is open; if the
port or network is unavailable, the local Help screen keeps working.

## MIDI ideas

Ideas capture free playing as MIDI. Each saved idea keeps its timing and
instrument reference. When synthv1 is used, it also keeps the mapped control
values. An idea can later be loaded and played through any active engine.

## Stereo audio

The Audio Recorder writes the selected JACK stereo pair as a 24-bit WAV file.
The screen shows recording time, sample rate, file size, dropped frames, and
errors.

If recording is interrupted, the temporary file remains with a `.wav.part`
name. SHR-DAW tries to recover it when the next recording starts.

External line input is intended to use the audio interface's direct-monitor
feature. See [Physical connections](CONNECTIONS.md) for the audio path.

## Command line

The main program also provides these commands:

```sh
shr menu
shr list
shr status
shr start "synthv1:Velvet Tines"
shr start "Yoshimi:Fat Bass"
shr stop
shr log 80
shr casio diagnostic
```

`shr casio diagnostic` keeps an old name from the first hardware test. It does
not send MIDI. It lists output ports and shows the messages that would be used.
The tracker itself is device-neutral.

For pattern editing, continue with the [Tracker guide](TRACKER.md).
