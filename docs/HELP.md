# SHR-DAW Help

[Controller basics](#controller-basics)
[Presets and playback](#presets-and-playback)
[MIDI ideas](#midi-ideas)
[FT2 tracker](#ft2-tracker)
[Pages and hardware MIDI](#pages-and-hardware-midi)
[Loops and audio](#loops-and-audio)
[Trouble spots](#trouble-spots)

## Controller basics

The main encoder moves one visible row or value at a time. Press it to select
the highlighted row, confirm a field, or follow a help link.

The controller menu has four pages. Page 1 is OPS. On child screens, page
4 item 4 is EXIT and returns one level. Empty buttons are hidden and silent.

Four-button controllers use encoder press to enter page-select mode. Turn to
choose a menu page, then press again to return the encoder to row movement.

## Presets and playback

Presets chooses the instrument engine and sound. Loading a sound starts or
reuses only the engine owned by SHR-DAW; unrelated synth processes are left
alone.

Synthv1 controls use pickup. After loading, idea load, or RESET, mapped CCs are
blocked until the physical control reaches the stored value. This prevents
jumps during live audio.

The dots beside synthv1 values compare the current sound to the loaded preset:
green is lower, yellow is near original, red is higher.

## MIDI ideas

Ideas record musical MIDI while a sound is loaded. STOP REC ends the take; TAKE
plays it back through the loaded engine; SAVE stores it for later.

Loading an idea can replace the current sound. If a sound is already active,
choose LOAD twice to confirm.

Ideas are MIDI, not audio. Use the audio recorder when you need a WAV of the
actual JACK input.

## FT2 tracker

FT2 is a pattern sequencer. PLAY starts at the cursor, START plays from the
song beginning, and STOP stops only the tracker transport.

EDIT turns incoming notes into pattern data. Encoder press inserts a blank row.
ERASE clears the selected cell and advances one row. N-OFF writes a note-off.

CELL edit is transactional. Confirm commits the draft cell; EXIT cancels and
restores the original value.

N00B mode maps live notes to the nearest selected major or natural-minor scale
tone. Equal-distance ties go downward, and note releases stay owned by the
source note that created them.

## Pages and hardware MIDI

Each tracker page has four lanes, one destination, and one MIDI channel. Pages
can target the active software instrument, the configured external output, or
a named MIDI port.

Real-time REC is hardware-output only. It refuses Active Instrument so a loaded
software synth is not doubled or rewritten by live capture.

Offline targets keep their data. If hardware is unplugged, fix the page target
or reconnect the device; the song does not discard the route.

## Loops and audio

Loop import copies WAV files from the configured inbox into private storage.
Source BPM is manual unless AUTO align can measure a useful pulse.

Tempo matching currently changes playback rate, so pitch changes with tempo.
Use UNIT to edit by beat or bar, and ALIGN to snap/move placement by bars.

The audio recorder writes the configured JACK stereo input as 24-bit WAV. If it
is interrupted, the unfinished `.wav.part` file is recovered on the next start.

## Trouble spots

If nothing sounds, check JACK first, then the page or preset target. Setup does
not start or restart JACK for you.

If controls do not move a synthv1 parameter, pickup is probably armed. Move the
physical control through the loaded value once.

PANIC sends all-notes-off, stops owned playback/recording, and shuts down the
managed engine. It does not kill synth processes SHR-DAW did not start.

Pad lock lets command pads play as musical notes. Turn pad lock off when menu
buttons appear to do nothing.
