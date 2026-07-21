# Physical connections

Every device in this guide is optional. Start with the equipment you already
have and add one connection at a time.

## Smallest setup

A Raspberry Pi, terminal, and audio output are enough to use software
instruments and the pattern editor. The terminal can be local or connected
over SSH. Audio can use any Pi, HDMI, or USB output exposed through JACK.

Add a MIDI keyboard when you want live velocity and chord input. Add a physical
control surface when you want to operate SHR-DAW without a computer keyboard or
mouse.

## Expanded setup

![Physical SHR-DAW connections: controller, Raspberry Pi, display, USB audio/MIDI interface, chained MIDI instruments, mixer, monitors, and headphones](images/shr-daw-physical-connections.jpg)

The main paths are:

- controller or computer keyboard → SHR-DAW;
- SHR-DAW → synthv1, Yoshimi, or FluidSynth → JACK audio output;
- tracker pages → optional external MIDI instruments;
- optional audio inputs → direct monitoring and/or synchronized stem recording.

The diagram is an example, not a shopping list. The original test rig used an
Arturia MiniLab 3, a PreSonus AudioBox USB 96, and a Casio Casiotone MT-240.
Those devices proved the workflow, but SHR-DAW is not designed around them.

## MIDI input

SHR-DAW treats a control surface and performance keyboards as separate roles.
The controller supplies menu buttons, the main encoder, pads, and mapped synth
controls. Every configured performance input bypasses those mappings, so a
keyboard note or CC cannot accidentally invoke a menu command. The controller
can either pass its unmatched musical messages (the compatible default for one
combined device) or be explicitly control-only.

If both roles resolve to the same exact ALSA port, SHR opens it once and
classifies its messages as the established combined-device path. If they
resolve to different ports, SHR opens separate connections. Repeated
performance inputs are simultaneous sources; repeated legacy `midi.input`
entries remain ordered controller fallbacks. Missing inputs fail independently,
and computer-keyboard control remains available.

Source identity remains attached to active notes. Two keyboards may hold the
same channel/note without either Note Off releasing the other's note. Route
changes, source loss, panic, stop, and shutdown release the affected ownership;
All Notes Off, All Sound Off, and sustain are also isolated per source/channel.

Do not also connect the controller directly to the same synth with a desktop
MIDI patching tool. Two paths can cause doubled notes. Use `shr-setup` or the
configuration files to choose the route in one place.

A USB audio/MIDI interface may be full-duplex: its MIDI input can be a
performance source while its MIDI output remains an FT2 destination. Input and
`external_midi.output` are independent directions.

Routing uses one stable ALSA identity policy everywhere. A live RtMidi name
such as `AudioBox USB 96:AudioBox USB 96 MIDI 1 32:0` is persisted as
`AudioBox USB 96:AudioBox USB 96 MIDI 1`; only the volatile trailing numeric
address is removed. The older whitespace form is accepted only when it resolves
uniquely. Partial or ambiguous matches stay offline rather than selecting an
arbitrary port.

Some distributions enable a standalone FluidSynth daemon or `amidiminder`,
which connects hardware and application MIDI ports broadly. Accept the
recommended exclusive-routing cleanup in `shr-setup`; SHR can still launch its
own configured FluidSynth when selected.

## Software instruments

SHR-DAW supports synthv1, Yoshimi, and FluidSynth as separately installed
programs. Only one SHR-DAW-managed software synth runs at a time. The
standalone Software Synth workspace keeps its sound while moving between its
Presets and Playback screens, then sends All Notes Off and unloads it on the
top-level return to Home. FT2 separately loads the engine/instrument pair saved
by its current Pattern when a software note is auditioned or scheduled; opening
an empty FT2 Pattern does not start it. Replacement and exit stop only a process
SHR-DAW owns.

Each engine has a configured MIDI input and JACK audio output. See
[Configuration and routing](CONFIGURATION.md) for the settings.

By default, the active engine connects directly to the configured playback
pair. With the opt-in owned effects graph enabled, that same one engine instead
passes through source inserts, two aux returns, the master rack, and final
meter. Activation is transactional and restores the direct path on failure.
The separate loop player and recorder do not pass through this graph.

## External MIDI instruments

Each tracker page can use its own MIDI output, with four independent column
channels/banks/programs. Several pages can play several hardware instruments
at the same time. All channels 1–16 and programs 0–127 remain raw-editable
without a device profile. A page can instead store a software engine and one of
that engine's instruments; it never inherits the standalone workspace selection.

Portable `AUTO` pages save no output or channel and follow the machine default.
Explicit pages remember their exact port. If it is disconnected, the page
shows `OFFLINE` and does not silently send to another port. Its notes and
preference are kept; reconnecting it makes
the original mapping usable on the next play without rewriting the Project.

Named sound lists for supported external instruments come from optional
[MIDI device profiles](MIDI_DEVICE_PROFILES.md). Profiles are convenience
metadata, not permission or detection. Instruments without one still expose
the normal MIDI programs 1–128 (stored/sent as values 0–127).

ALSA can report that an AudioBox MIDI output port is online, but a one-way DIN
output cannot report whether the downstream Roland D-50 is connected or
powered. Routing therefore shows the interface availability and the configured
device profile separately, for example `AudioBox · ONLINE` and
`D-50 · UNVERIFIED`. It never says `D-50 connected`.
SHR-DAW does not probe downstream DIN hardware. Advanced users may construct
arbitrary experimental channel/program chains behind one configured output.

## Audio output and recording

The setup wizard selects the left and right JACK playback ports and can retain
the older left/right recording pair. The recorder screen and repeated
`capture.track` entries configure any larger set of exact capture sources.

When an audio interface has direct monitoring, connect an external instrument
to its inputs and use the interface's monitor balance. This mixes the external
sound with SHR-DAW software instruments without a second software-monitoring
path or its additional CPU work.

Those JACK capture inputs remain available to the raw multitrack recorder. The
optional owned final bus additionally resolves exactly one configured stereo
capture pair and software-monitors it alongside the managed synth and owned WAV
loop. The resulting limited stereo samples feed both playback and the dedicated
24-bit final-mix recorder. This is not a free-routing mixer and does not add
per-interface-channel processing.

Do not also direct-monitor the same capture pair at the interface unless the
doubled path is deliberate: otherwise the two latencies can cause excess level
and comb filtering. SHR-DAW refuses a configuration that declares both modes
unless explicit confirmation is set. See
[Final performance bus](FINAL_PERFORMANCE_BUS.md).

For exact routes and configuration keys, read
[Configuration and routing](CONFIGURATION.md).
For source assignment, manifests, recovery, and interfaces with any channel
count, read [Multitrack recording](MULTITRACK_RECORDING.md).
