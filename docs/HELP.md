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
alone. Presets and Playback share that owned sound. Returning from top-level
Presets to Home sends All Notes Off and unloads it; FT2 loads its Pattern's
synthv1 preset separately.

Synthv1 controls use pickup. After loading, idea load, or RESET, mapped CCs are
blocked until the physical control reaches the stored value. This prevents
jumps during live audio.

The dots beside synthv1 values compare the current sound to the loaded preset:
green is lower, yellow is near original, red is higher.

Playback names the held chord and notes above a continuous two-row keyboard.
Each displayed note has its decimal MIDI Note On velocity (1–127) directly
beneath it. Use the row to practise gentle/strong strikes, even chord attacks,
or bass-versus-chord balance. Velocity comes from MIDI and is not an audio
volume measurement; the controller and instrument response matter.
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

Presets NAV MTR, or keyboard m on Presets, opens the meter/mix surface. With the
owned graph disabled it retains the passive CPU and legacy output view. With
the graph enabled it shows Synth, Loop, and Input readiness, level and mute;
master level; final L/R peak and clip; limiter gain reduction; and final-record
elapsed time, size, drop/error state, and path.

Stereo bars show live smoothed RMS and a short, decaying peak marker on a −60
to 0 dBFS scale. Each channel's `MAX` number separately holds its highest peak
without decay. CLIP is held in red. RESET clears `MAX`, the short markers, and
CLIP. Any downward movement of the mapped synthv1 Volume control clears both
`MAX` values even when pickup blocks the actual Volume change; increases,
equal values, and other controls leave them alone. Stopped, unavailable, and
new meter sessions cannot carry an old `MAX` forward.

FINAL OUT is available only for the active owned graph. It measures after all
three required sources, master inserts, master level, and linked limiter. The
same final limited buffer feeds the stereo recorder and playback. Direct
playback reports this final-bus meter unavailable and stays direct.

The FT2 WAV Loop screen's `LOOP OUT` still measures only the rendered loop. When
the final bus is active, that loop is one of the three sources in `FINAL OUT`.

On MTR, SOURCE-/SOURCE+ choose a source, LEVEL-/LEVEL+ change it in 1 dB steps,
MUTE toggles it, and REC starts/stops the final stereo WAV at callback
boundaries. RESET clears presentation holds and, when the bus is unavailable,
retries the same exact remembered source mapping. Source and master changes are
smoothed; there are no pan, solo, aux, or per-input effect controls.

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

With controller clock enabled, SHR sends the current/default tempo at 24 PPQN
to one exact controller MIDI port while the app is open; tracker transport adds
Start/Stop. An empty Pattern may run for a live external-sync arpeggiator;
tracker pages never send notes or programs back through the clock-only route.

EDIT turns incoming notes into pattern data. Encoder press inserts a blank row.
The ADD page chooses whether note entry, blank, erase, and note-off advance by
1, 2, 4, or 8 rows; the FT2 heading shows the active value. N-OFF writes a
note-off.

CELL edit is transactional. Confirm commits the draft cell; EXIT cancels and
restores the original value. STOP stops transport without discarding the draft.

N00B mode keeps the selected page as the instrument and enters notes with one
visible length. LENGTH opens a rotary selector for 1/1, 1/2, 1/4, 1/8, 1/16,
or 1/32; 1/16 is the default. Entry writes the note and its end, then advances.
NORMAL restores detailed tracker editing without changing existing notes.

## Pages and hardware MIDI

Each tracker page has four lanes and one destination. New Patterns start with
Software Synth (first synthv1 preset), MIDI (channel 1/program 1), and Drums
(channel 10). Explicit columns show MIDI channel 1–16 and program 1–128. Pages can target
a named synthv1 preset, configured external output, or named MIDI port. Live
keyboard and musical MIDI audition whichever page is selected.
Sharing a destination/channel requires the same master instrument.

Real-time REC is hardware-output only. It refuses synthv1 pages so a software
synth is not doubled or rewritten by live capture.

Missing preferred targets keep their data and show FALLBACK or OFFLINE. A
runtime substitute never replaces the saved route; reconnect and play again.

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

The normal Loop screen's `LOOP OUT` bars show only that WAV after its cut,
position, interpolation, transport gate, and edge fades. They do not include
the loaded synth, effects, recorder input, hardware gain, or other JACK clients.

The audio recorder arms independently named JACK inputs and writes one 24-bit
mono WAV per input in a synchronized take directory. Select a track, assign an
exact discovered source, name it, then arm it; a missing remembered source stays
missing and blocks recording instead of being replaced. ARM ALL includes only
resolved tracks, and NONE disarms all. Every armed stem starts and stops on the
same JACK callback boundary.

Each take has a `session.json` manifest recording the sample rate, shared frame
count, source identities, grouping, errors, and finalization state. Recognized
interrupted take directories recover conservatively on the next start. Existing
two-port `capture.input` configuration still appears as a linked stereo pair.

## Trouble spots

If nothing sounds, check JACK first, then the page or preset target. Setup does
not start or restart JACK for you.

If controls do not move a synthv1 parameter, pickup is probably armed. Move the
physical control through the loaded value once.

PANIC sends all-notes-off, stops owned playback/recording, and shuts down the
managed engine. It does not kill synth processes SHR-DAW did not start.

Pad lock lets command pads play as musical notes. Turn pad lock off when menu
buttons appear to do nothing.
