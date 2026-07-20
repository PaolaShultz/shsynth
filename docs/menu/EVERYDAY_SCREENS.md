# Everyday screens

[Manual home](../MENU_MANUAL.md) · [FT2 and Projects](TRACKER_AND_PROJECTS.md) ·
[Loops and effects](LOOPS_AND_EFFECTS.md)

All values shown below are deterministic presentation data. The screens are
real SHR-DAW renders, but no instrument, MIDI take, recorder, or meter was live
while the images were made.

## Presets

Home is the navigation root; **Software Synths** opens Presets. Turn the main
encoder, use the arrow keys, or use the mouse wheel to choose a sound. Loading replaces the one managed software
instrument; it never layers engines. synthv1, Yoshimi, and FluidSynth remain
separate catalogs selected with the Engine page.

Presets and Playback form one Software Synth workspace, so moving between them
does not reload the sound. Exiting top-level Presets to Home sends All Notes Off
and unloads only that SHR-owned engine. FT2 does not inherit the selection.

### OPS — browse and load

![Populated Presets screen with the OPS controller page](../images/menu/presets-ops.png)

`LOAD` starts the highlighted sound. `FIRST` and `LAST` jump to the ends of the
current engine's catalog. Keyboard PageUp/PageDown still move by ten sounds;
physical command pads deliberately do not duplicate that coarse scrolling.

### ENGINE — change instrument host

![Populated Presets screen with the ENGINE controller page](../images/menu/presets-engine.png)

`ENGINE-` and `ENGINE+` move among synthv1, Yoshimi, and FluidSynth. Changing catalogs does not
load a sound until `LOAD` is used.

### SYS — safety and help

![Populated Presets screen with the SYS controller page](../images/menu/presets-sys.png)

`PANIC` stops owned playback and notes. `HELP` opens the local help reader.
`EXIT` returns to Home. MIDI never quits the application; quitting remains
computer-keyboard-only from Home.

## Playback

Playback appears after a sound is loaded. The body shows the held chord and
notes, each note's decimal MIDI strike velocity directly beneath it, a
continuous two-row keyboard state, and the 12 mapped synthv1 controls. The
aligned velocity row helps with gentle/strong control, consistent chord
attacks, and bass-plus-chord balance. It is MIDI input data, not an audio
loudness meter; controller and instrument response determine the audible
result.
On the keyboard, red white-key areas are held natural notes and red upper `└`
marks are held sharps. Parameter colors are relative to the loaded preset:
green below the original value, bright yellow near it, and red above it. The
main encoder press resets only these mapped controls and re-arms pickup; it does
not restart the synth.

### OPS — capture a MIDI take

![Populated Playback screen with the OPS controller page](../images/menu/playback-ops.png)

`PLAY` plays or stops the captured take. `RECORD` starts or stops free-time MIDI
capture.

### SOUND — reset, finish, tempo, and effects

![Populated Playback screen with the SOUND controller page](../images/menu/playback-sound.png)

`RESET` restores the 12 mapped parameters in place and re-arms hardware pickup.
`SAVE` publishes a new non-overwriting Idea.

### SYS — stop and return

![Populated Playback screen with the SYS controller page](../images/menu/playback-sys.png)

`PANIC` performs the global owned stop. `HELP` opens help and returns here
afterward. `EXIT` returns to Presets, then Presets `EXIT` returns Home.

## Ideas

Ideas are timestamped or numbered free-time MIDI takes. A synthv1 Idea carries
a private preset snapshot; external-engine Ideas retain their sound identity
instead. Turn the encoder to select an entry.

### OPS — inspect, load, play, or delete

![Populated Ideas screen with the OPS controller page](../images/menu/ideas-ops.png)

`INSPECT` shows the Idea's sound and recording metadata. `PLAY` plays or stops
the take. `RECORD` starts or stops capture. `DELETE` requires a repeated
confirmation and only removes the selected Idea.

### FILE — load or save an Idea

![Populated Ideas screen with the CAPTURE controller page](../images/menu/ideas-capture.png)

`LOAD` restores the selected Idea, asking for confirmation before replacing an
active sound. `SAVE` publishes a new non-overwriting Idea. `FIRST` and `LAST`
select the list boundaries.

### SYS — safety and list boundary

![Populated Ideas screen with the SYS controller page](../images/menu/ideas-sys.png)

`PANIC` stops owned notes and transports. `HELP` opens contextual help. `EXIT`
returns Home.

## Help

Help is always available locally with `?` or F1, even if the optional temporary
LAN page cannot bind. Turn the encoder one rendered row at a time. On eight- or
five-button layouts, encoder press follows a selected section link.

### OPS — read and follow links

![Populated Help screen with the OPS controller page](../images/menu/help-ops.png)

`OPEN` follows the highlighted internal link and is the required link action on
a four-button layout. `TOP` returns to the beginning. Keyboard
PageUp/PageDown retain page scrolling; physical pads do not.

### SYS — safety and return

![Populated Help screen with the SYS controller page](../images/menu/help-sys.png)

`PANIC` remains available while reading. `EXIT` returns to the exact screen
that opened Help.

## Audio recorder

The recorder captures any deliberately configured set of JACK source ports as
one synchronized take with a 24-bit mono WAV per input and a shared manifest.
The compact list shows named tracks as ready or missing, and the status rows
show armed count, elapsed time, sample rate, writer high-water mark, drop/xrun
counts, final path, or the failure reason. It never starts or restarts JACK.

### RECORD — arm and record

![Populated Audio recorder screen with the OPS controller page](../images/menu/audio-recorder-ops.png)

`RECORD` starts all armed tracks at one callback boundary. `ARM` toggles the
selected track. An armed missing source prevents a take from starting.

### TRACK — choose the inputs

![Populated Audio recorder screen with the TRACK controller page](../images/menu/audio-recorder-track.png)

`PREV` and `NEXT` select a track. `SOURCE` cycles deliberately through the
currently discovered sources (and blank); `NAME` edits the musician-facing
label. Runtime absence never overwrites a remembered source.

### SETUP — prepare tracks

![Populated Audio recorder screen with the NAV controller page](../images/menu/audio-recorder-nav.png)

`ALL` arms every resolved track, `NONE` disarms everything, and `REFRESH`
discovers current JACK audio sources without changing assignments.

### SYS — finalize safely

![Populated Audio recorder screen with the SYS controller page](../images/menu/audio-recorder-sys.png)

`PANIC` stops owned activity. `HELP` opens help. `EXIT` returns Home without
silently changing recorder state.

## Performance meter

MTR is passive: it never changes the route. CPU bars come from bounded UI-side
system readings. Live stereo RMS bars, a short decaying peak marker, and the
independent non-decaying `MAX` numbers are shown only for the final output of
SHR-DAW's active owned graph. Direct mode and stopped engines are explicitly
reported as unavailable instead of displaying unrelated audio.

CPU is whole-core `/proc/stat` activity, not synth or graph process CPU, JACK
callback timing, or xruns. The audio bars are post-master for the managed
instrument and its two returns; they deliberately exclude the separate WAV
loop, recorder input, external hardware, and unrelated JACK clients.

### OPS — clear presentation holds

![Populated performance meter with the OPS controller page](../images/menu/performance-meter-ops.png)

`RESET` clears both `MAX` numbers, the short peak markers, and the clip hold. It
does not reset audio, effects, CPU state, or transport. Moving the mapped
synthv1 Volume control downward clears both `MAX` numbers even before pickup
accepts the control; upward, equal, and unrelated control movements do not.

### SYS — safety and return

![Populated performance meter with the SYS controller page](../images/menu/performance-meter-sys.png)

`PANIC` remains available. `HELP` opens the explanation of meter scope. `EXIT`
returns to Home. The screenshot says `Presentation · no live audio` because
its meter values are seeded for documentation rather than measured.
