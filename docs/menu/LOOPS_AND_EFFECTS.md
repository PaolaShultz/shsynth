# Loops and effects

[Manual home](../MENU_MANUAL.md) · [Everyday screens](EVERYDAY_SCREENS.md) ·
[FT2 and Projects](TRACKER_AND_PROJECTS.md)

WAV loops and the owned effects graph require a running JACK server, but the
screenshots below are deterministic presentation states and never start JACK.
The loop player and graph operate only on resources owned by SHR-DAW.

## FT2 WAV Loop

A Project may attach one privately imported mono or stereo WAV. Import copies
the selected inbox file beneath the user's SHR-DAW data directory. The player
uses native pitch and requires the WAV sample rate to match JACK; it does not
time-stretch or pitch-shift audio to force a fit.

### OPS — import and transport

![Populated FT2 WAV Loop screen with the OPS page](../images/menu/ft2-loop-ops.png)

`IMPORT` copies the selected inbox WAV into private storage and attaches it to
the Project. `HERE` plays from the current tracker position. `START` plays from
the Arrangement beginning. `STOP` stops tracker and loop transport.

### BPM — interpret source tempo

![Populated FT2 WAV Loop screen with the BPM page](../images/menu/ft2-loop-bpm.png)

`BPM-` and `BPM+` change the interpreted source tempo. `BPM x` cycles half,
normal, and double interpretation. `UNIT` switches cut adjustment between
single beats and whole bars. Tempo matching changes the current Pattern tempo;
it does not alter the WAV samples.

### CUT — choose the beat region

![Populated FT2 WAV Loop screen with the CUT page](../images/menu/ft2-loop-cut.png)

`START-` and `START+` move the region's first beat. `LEN-` and `LEN+` change
its length. The active `UNIT` determines whether each press means one beat or
one whole Project bar.

### SYS — align and return

![Populated FT2 WAV Loop screen with the SYS page](../images/menu/ft2-loop-sys.png)

`PANIC` and `STOP` remain reachable. `ALIGN` opens offline analysis and
placement adjustment. `EXIT` returns to FT2 Tools.

## Private loop library

The library distinguishes the WAV attached to the current edit (`CURRENT`),
files referenced by saved Projects (`SAVED`), and unreferenced files (`FREE`).
It rejects symlinks and unsafe paths.

### OPS — browse and delete an unused WAV

![Populated private loop library with the OPS page](../images/menu/loop-library-ops.png)

`DELETE` begins confirmed physical deletion. It succeeds only for a free,
unreferenced private WAV. `PG UP` and `PG DOWN` move through the list. This is
separate from Project `REMOVE`, which only detaches a loop.

### SYS — stop and return

![Populated private loop library with the SYS page](../images/menu/loop-library-sys.png)

`PANIC`, `STOP`, and `HELP` retain their normal meanings. `EXIT` leaves library
mode and returns to WAV Loop setup.

## Loop Align

Align performs bounded offline pulse/duration analysis, can snap interpreted
length to complete Project bars, and can shift placement without destructively
editing the audio file.

### OPS — analyze and place

![Populated Loop Align screen with the OPS page](../images/menu/loop-align-ops.png)

`AUTO` analyzes the attached file and proposes a bar-aligned beat length.
`BAR-` and `BAR+` move its placement by exactly one bar. `DONE` keeps the
settings and returns to WAV Loop.

### SYS — stop or leave

![Populated Loop Align screen with the SYS page](../images/menu/loop-align-sys.png)

`PANIC`, `STOP`, and `HELP` stay available. `EXIT` returns to WAV Loop without
performing another automatic analysis.

## FX Rack

The rack targets `SOURCE`, `AUX 1`, `AUX 2`, or `MASTER`. Source and master
racks are serial inserts. Aux buses have an independent send level, pre/post
source-insert point, wet-only processor rack, and return level. Each rack is
bounded to eight effects. With the graph active, FX changes are refused while
transport or recording makes publication unsafe. With it disabled, the same
controls edit saved Project data without touching audio.

The screenshot shows a populated source chain. Selecting another target keeps
the same menu but changes the body and which routing actions apply.

### OPS — edit rack contents

![Populated FX Rack with the OPS page](../images/menu/fx-rack-ops.png)

`EDIT` opens the selected processor's named parameters. `ADD` appends the
currently displayed effect kind. `BYPASS` fades the selected processor between
active and safe bypass. `REMOVE` removes only the selected owned processor.

### ORDER — reorder and choose add kind

![Populated FX Rack with the ORDER page](../images/menu/fx-rack-order.png)

`UP` and `DOWN` move the selected effect within this rack. `KIND-` and `KIND+`
choose the next effect type to add. Aux targets offer only supported wet
time/modulation effects.

### ROUTE — choose rack and aux send

![Populated FX Rack with the ROUTE page](../images/menu/fx-rack-route.png)

`TARGET` cycles Source, Aux 1, Aux 2, and Master. On an aux target, `SEND-` and
`SEND+` adjust its send level in dB and `POINT` toggles pre/post source inserts.
Those three controls report that an aux must be selected when used elsewhere.

### SYS — return level, help, and exit

![Populated FX Rack with the SYS page](../images/menu/fx-rack-sys.png)

`PANIC` remains available. On an aux target, `RETURN` cycles its independent
return level. `HELP` opens the local reference. `EXIT` returns one level.

## FX parameter editor

Parameters come from strict per-effect schemas and are displayed in physical
units such as dB, hertz, milliseconds, ratio, or percent. The demonstration
shows a compressor with its ratio selected. Live input/output, clip,
non-finite, and gain-reduction meters appear only when the owned graph supplies
them; the screenshot honestly reports the graph as inactive.

### OPS — select and adjust a parameter

![Populated FX editor with the OPS page](../images/menu/fx-editor-ops.png)

`PARAM-` and `PARAM+` choose a named parameter. `VALUE-` and `VALUE+` change it
by the schema's safe step and clamp to the validated range.

### STATE — bypass this processor

![Populated FX editor with the STATE page](../images/menu/fx-editor-state.png)

`BYPASS` toggles the edited processor without removing its ID, parameters, or
position. Bypass uses click-conscious smoothing in the active graph.

### NAV — return to the rack

![Populated FX editor with the NAV page](../images/menu/fx-editor-nav.png)

`RACK` returns to the parent rack while keeping valid parameter changes.

### SYS — safety and exit

![Populated FX editor with the SYS page](../images/menu/fx-editor-sys.png)

`PANIC` and `HELP` stay available. `EXIT` returns to the rack. Invalid or
non-finite parameter values are refused rather than published to audio.
