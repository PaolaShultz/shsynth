# SHR-DAW Help

[Controller basics](#controller-basics)
[Presets and playback](#presets-and-playback)
[Effects graph](#effects-graph)
[Performance meters](#performance-meters)
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
In Help, use OPS OPEN to follow the highlighted link. In target/channel
editors, use OPS CONFIRM; SYS EXIT cancels the field.

## Presets and playback

Presets chooses the instrument engine and sound. Loading a sound starts or
reuses only the engine owned by SHR-DAW; unrelated synth processes are left
alone.

Synthv1 controls use pickup. After loading, idea load, or RESET, mapped CCs are
blocked until the physical control reaches the stored value. This prevents
jumps during live audio.

The dots beside synthv1 values compare the current sound to the loaded preset:
green is lower, yellow is near original, red is higher.

Playback names the held chord and notes above a continuous two-row keyboard.
At 40 columns it shows C2 through G7 when middle C is C4. A red white-key area
means its natural note is held; a red upper `└` means the following sharp is
held. Major triads show `maj` explicitly, such as `C maj`.
`display.note_names=german` uses B/H spelling; `english` uses A#/B.

## Effects graph

Playback SOUND FX opens the current Project's FX rack. TARGET cycles SOURCE,
AUX 1, AUX 2, and MASTER. Source effects change the instrument in series.
Each aux makes a parallel wet copy: SEND sets how much enters it, POINT chooses
before or after source effects, and RETURN sets how much comes back. Master
effects change the final dry-plus-aux mix.

KIND chooses what ADD creates. Select a row to EDIT it; BYPASS fades a source
or master effect toward dry. A fully bypassed aux returns silence, so it never
doubles the dry source; a delay tail can be allowed to fade with new input
muted. ORDER moves the same stable instance and REMOVE deletes it. Aux effects
are forced wet.

The editor selects named parameters and adjusts values in physical units. Its
bottom rows show input/output peak and RMS, clip/non-finite counts, and
compressor gain reduction when the owned graph is active. Rack size and total
effect count are bounded. With the graph active, stop transport and all
recording before an FX change can publish a replacement plan. With the graph
disabled, the same editor can design and save the Project silently, but direct
playback will not process or meter it.

## Performance meters

Presets NAV MTR, or keyboard m on Presets, opens a passive meter. CPU0–CPU3 use
whole-core Linux `/proc/stat` counter changes: green is below 60%, yellow is
60–85%, and red is above 85%. This is not synth/JACK process CPU, callback
timing, xrun detection, or proof of audio safety. Configured CPU temperature is
optional.

Stereo bars show live smoothed RMS and a short, decaying peak marker on a −60
to 0 dBFS scale. Each channel's `MAX` number separately holds its highest peak
without decay. CLIP is held in red. RESET clears `MAX`, the short markers, and
CLIP. Any downward movement of the mapped synthv1 Volume control clears both
`MAX` values even when pickup blocks the actual Volume change; increases,
equal values, and other controls leave them alone. Stopped, unavailable, and
new meter sessions cannot carry an old `MAX` forward.

FINAL OUT is available only for the active owned graph. It measures after all
master inserts, immediately before playback, and covers the managed source and
its wet returns—not WAV loops, inputs, hardware, or other JACK clients. Direct
playback reports the meter unavailable and stays direct.

## MIDI ideas

Ideas record musical MIDI while a sound is loaded. STOP REC ends the take; TAKE
plays it back through the loaded engine; SAVE stores it for later.

Recording timestamps come from the MIDI callback, and TAKE playback runs
independently of screen redraws. Stopping a take cancels it promptly and sends
all-notes-off cleanup.

Loading an idea can replace the current sound. If a sound is already active,
choose LOAD twice to confirm. Saved synthv1 control values are restored after
the sound loads, and pickup is armed against those restored values.

Ideas are MIDI, not audio. Use the audio recorder when you need a WAV of the
actual JACK input.

## FT2 tracker

FT2 is a Pattern sequencer. PLAY starts at the cursor, START plays from the
Project's Arrangement beginning, and STOP stops only the tracker transport.

EDIT turns incoming notes into pattern data. Encoder press inserts a blank row.
The ADD page chooses whether note entry, blank, erase, and note-off advance by
1, 2, 4, or 8 rows; the FT2 heading shows the active value. N-OFF writes a
note-off.

CELL edit is transactional. Confirm commits the draft cell; EXIT cancels and
restores the original value. STOP stops transport without discarding the draft.

N00B mode maps live notes to the nearest selected major or natural-minor scale
tone. Equal-distance ties go downward, and note releases stay owned by the
source note that created them.

## Pages and hardware MIDI

Each tracker page has four lanes and one destination. Every column independently
stores MIDI channel 1–16, bank MSB/LSB, and master program. Pages can target the
active software instrument, the configured external output, or a named MIDI
port. Sharing a destination/channel is allowed only when the columns select the
same master instrument.

Real-time REC is hardware-output only. It refuses Active Instrument so a loaded
software synth is not doubled or rewritten by live capture.

Offline targets keep their data. If hardware is unplugged, fix the page target
or reconnect the device; the Project does not discard the route.

FILES NEW PRJ requires a second press, clears the current unsaved Project, and
starts the next `project-001` style name. SAVE AS writes and switches to the
next non-overwriting `<name>-copy-001` file. Pattern Repeat/Remove operations
remain on the Arrange screen. FILES NAME accepts a display name and safely
publishes a rename. FILES PATTERN groups pattern create/clone/clear, clipboard,
and melody-only semitone/octave transpose actions. PATTERN DRUMS loads bundled
grooves into the percussion page without changing its MIDI route. FILTER picks
genre, meter, and 32/64/128-row length (24/48/96 in 3/4). Empty Patterns resize;
existing melody is protected. Saved drum patterns are separate `.shdrum` files;
only user saves can be deleted.
FILES CLEAN deletes only a zero-reference Pattern and never edits Arrangement
steps.

## Loops and audio

Loop import copies WAV files from the configured inbox into private storage.
Source BPM is manual unless AUTO align can measure a useful pulse.

Tempo matching sets the current Pattern tempo from the WAV; the WAV plays at
native speed and pitch. The loop sample rate must match JACK. Use UNIT to edit
by beat or bar, and ALIGN to snap/move placement by bars. TOOLS LOOP REMOVE is
confirmed and detaches the Project loop without deleting the private WAV.
TOOLS LIBRARY separately lists private WAVs and physically deletes only
unreferenced regular files after confirmation.

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
