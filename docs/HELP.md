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

The main encoder moves one visible row or value at a time except on the FT2
grid, where Play/REC turns select columns and Edit turns move rows. Press it to
select the highlighted row, confirm a field, or follow a help link.

Home centers every label in one equal-width bar. MIDI Learn, Routing, and
Effects are separate destinations. Routing reports current controller,
performance-input, MIDI-output,
clock, and audio connections and edits them transactionally. Browsing is
read-only; press to edit a detached field, press again to validate/save, or
Back to cancel. Use `shr-setup` for initial machine setup.

If a configured controller is offline, has no reviewed profile, or has not
learned encoder turn and click, Home highlights MIDI Learn and explains why.
Keyboard Up/Down/Enter still work. Optional command buttons may be skipped once
the learned encoder can turn and click. Home does not learn or send MIDI by
itself; Learn keeps selected-controller messages isolated until an explicit
save or cancel. Separate performance inputs continue to bypass controller
interpretation.

The controller menu has four pages. Page 1 is OPS. On child screens, page
4 item 4 is EXIT and returns one level. Empty buttons are hidden and silent.

Home is the only screen without the shared working-screen status row. On every
other screen the two controller rows sit immediately above it. The first status
cell is steady green `>` for play, steady white `■` for stop, steady white `‖`
for pause, or red `●` for record; record alone pulses between red and bright
red without hiding the circle.

Four-button controllers use encoder press to enter page-select mode. Turn to
choose a menu page, then press again to return the encoder to screen control.
In Help, use OPS OPEN to follow the highlighted link. In target/channel
editors, use OPS CONFIRM; SYS EXIT cancels the field.

Some navigation actions open a master overlay instead of replacing the current
workspace. The workspace remains visible around a 38×11 border; its usable
inside is 36×9 on a 40×13 display. While the overlay is open, its bottom
border shows only the highlighted launcher near the same physical position;
the final row remains the shared status row. Turn the master rotary or use
Up/Down, then click or press Enter. Press that same menu
item again, or use Back/Esc, to close. The controller strip has no separate
Back item while an overlay is open. Back first cancels an active field, then
cancels any unconfirmed draft and closes the overlay.

## Presets and playback

Presets chooses the instrument engine and sound. Loading a sound starts or
reuses only the engine owned by SHR-DAW; unrelated synth processes are left
alone. Presets and Playback share that owned sound, and leaving those screens
keeps it running. Global panic, shutdown, replacement, or an explicit different
FT2 software route ends it safely. A genuinely new, empty, unsaved default FT2
Project adopts the current engine/instrument on page 1 without restarting it;
without a Player instrument, FT2 loads the first available synthv1 preset.
Saved or explicitly changed Projects keep their routes.

Synthv1 controls use pickup. After loading, idea load, or RESET, mapped CCs are
blocked until the physical control reaches the stored value. This prevents
jumps during live audio.

The dots beside synthv1 values compare the current sound to the loaded preset:
green is lower, yellow is near original, red is higher.

Playback names the held chord and notes, with each note's decimal MIDI Note On
velocity (1–127) directly beneath it. Use the rows to practise gentle/strong
strikes, even chord attacks, or bass-versus-chord balance. Velocity comes from
MIDI and is not an audio volume measurement; the controller and instrument
response matter. On terminals taller than the native 40×13 layout, the spare
space adds a continuous two-row keyboard from C2 through G7 at 40 columns. A
red white-key area means its natural note is held; a red upper `└` means the
following sharp is held. Major triads show `maj` explicitly, such as `C maj`.
`display.note_names=german` uses B/H spelling; `english` uses A#/B.

Playback N00B toggles the filter on the existing Player screen. While on, its
compact SCALE rotary appears below the normal controls; turn the master encoder
to cycle every root plus MAJOR or natural MINOR choice. Notes in the chosen
scale keep their pitch and sound normally; notes outside it stay silent.
Pressing N00B again restores all chromatic notes. Changing or leaving the
filter releases held notes first.

## Effects graph

Playback SYS FX or FT2 Tools OPS FX opens the current Project's FX rack. In
FT2, uppercase F opens it directly. Back returns to the calling Player or FT2
screen while its instrument remains active. TARGET cycles SOURCE,
AUX 1, AUX 2, and MASTER. Source effects change the instrument in series.
Each aux makes a parallel wet copy: SEND sets how much enters it, POINT chooses
before or after source effects, and RETURN sets how much comes back. Master
effects change the final dry-plus-aux mix.

ADD inserts a provisional processor and opens TYPE. EDIT changes the selected
processor's type; PARM opens its named values; DEL removes it. ORDER moves the
same stable instance and BYPASS fades a source or master effect toward dry. A
fully bypassed aux returns silence, so it never doubles the dry source; a delay
tail can be allowed to fade with new input muted. Aux effects are forced wet.

The editor selects named parameters and adjusts values in physical units. One
compact meter row appears when the owned graph has data. The compressor uses a
dark-red 0.5–24 dB LED row whose bright-red lights show live gain reduction;
bypass leaves every LED dim. Other effects show compact input/output values.
Rack size and total effect count are bounded. With the graph active, stop
transport and all recording before an FX change can publish a replacement
plan. With the graph disabled, the same editor can design and save the Project
silently, but direct playback will not process or meter it.

## Performance meters

Home PERFORMANCE, or keyboard m, opens the meter/mix surface. With the
owned graph disabled it retains the passive CPU and legacy output view. With
the graph enabled it shows Synth, Loop, and Input readiness, level and mute;
master level; final L/R peak and clip; limiter gain reduction; and final-record
elapsed time, size, drop/error state, and path.

Stereo bars use circular `●` LEDs for live smoothed RMS and a brighter,
decaying held peak on a −60 to 0 dBFS scale. Unlit circles are dark gray; safe
active circles use one green, while yellow and red appear only at their active
thresholds. Each channel's `MAX` number separately holds its highest peak
without decay. CLIP is held in red. RESET clears `MAX`, the bright peaks, and
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
In Play and REC, turn the physical main encoder to select columns across page
boundaries; keyboard Up/Down still moves rows. Edit keeps encoder row movement.
The shaded selected column does not move the row, playhead, Arrangement Step,
or transport. During REC, turns made while recorded notes are held are ignored
until all of those notes receive Note Off.

FT2 `SELECT` contains `PAGE`, `PATTERN`, `SONG`, and `ROUTE`. PAGE selects one of
the current Pattern's four-column locations and can open the full Tracks
manager. PATTERN selects an existing Pattern or opens Pattern/Project tools.
SONG selects an Arrangement step and can open detailed Arrangement or Loop/
page tools. ROUTE shows the active page destination and all four columns'
channel, bank, program, profile name, and availability. ROUTE changes remain a
draft until `APPLY ROUTING`; closing it with its highlighted launcher or Back
cancels the draft safely.

With controller clock enabled, SHR sends the current/default tempo at 24 PPQN
to one exact controller MIDI port while the app is open; tracker transport adds
Start/Stop. An empty Pattern may run for a live external-sync arpeggiator;
tracker pages never send notes or programs back through the clock-only route.

EDIT turns incoming notes into pattern data. Encoder press inserts a blank row.
Edit `ADD` opens a rotary overlay choosing any advance from 0 through 32
rows; 0 keeps the cursor on the current row. The FT2 heading shows the active
value. N-OFF writes a note-off.

On a Drums page, EDIT reuses each voice's column from earlier rows. New bass
drums prefer column 1, new snares prefer column 2, and other new drums begin in
columns 3–4. Existing cells are not replaced to force that layout.

CELL edit is transactional. DONE `SAVE` commits the draft cell; `EXIT` cancels
and restores the original value. `PANIC` remains available without introducing
a second partial-commit path.

FT2 N00B toggles the Player-selected scale directly over Play, Record, and Step
Edit on the selected melodic page; it does not open another screen or change
the current mode. Out-of-scale keys stay silent and are never moved to another
pitch. Play can use it without writing; Record and Edit write only the
allowed notes. It turns off automatically on Drums, where the current mode
remains active.

Edit **LENGTH** opens a rotary overlay choosing 1/1, 1/2, 1/4, 1/8, 1/16,
1/32, 1/64, or 1/128 for melodic notes. The independent 0–32-row **ADD**
overlay controls where the next entry goes.

## Pages and hardware MIDI

Each tracker page has four lanes and one destination. New Patterns start with
Software Synth (first synthv1 preset), MIDI (channel 1/program 1), and Drums
(channel 10). Explicit columns show MIDI channel 1–16 and program 1–128. Pages can target
a named synthv1 preset, configured external output, or named MIDI port. Live
keyboard and musical MIDI audition whichever page is selected.
Sharing a destination/channel requires the same master instrument.

Real-time REC is hardware-output only. It refuses software-instrument pages so
an owned synth is not doubled or rewritten by live capture.

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
by beat or bar, and ALIGN to snap/move placement by bars. Loop Player PLAY
`REMOVE` is confirmed and detaches the Project loop without deleting the WAV.
Loop Player SYS `LIBRARY` opens an overlay over the loop page; turn to browse
inbox and private WAVs, then press to import or attach and load the selected
file.
`INBOX` imports; `PRIVATE`, `CURRENT`, and `SAVED` attach the existing file.
The browser does not delete WAVs.

The loop reports READY, NOT READY, or OUTPUT FAULT. A valid decoded region
keeps its white position bar and green playhead even during an output fault.

The normal Loop screen's circular-LED `LOOP OUT` bars show only that WAV after its cut,
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
