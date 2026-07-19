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
- optional audio input → direct monitoring and/or stereo WAV recording.

The diagram is an example, not a shopping list. The original test rig used an
Arturia MiniLab 3, a PreSonus AudioBox USB 96, and a Casio Casiotone MT-240.
Those devices proved the workflow, but SHR-DAW is not designed around them.

## MIDI input

SHR-DAW opens the configured controller input once. It consumes menu buttons,
the main encoder, and mapped synth controls inside the application. Musical
notes pass to the selected destination.

Do not also connect the controller directly to the same synth with a desktop
MIDI patching tool. Two paths can cause doubled notes. Use `shr-setup` or the
configuration files to choose the route in one place.

## Software instruments

SHR-DAW supports synthv1, Yoshimi, and FluidSynth as separately installed
programs. Only one SHR-DAW-managed software synth runs at a time. Changing it
sends All Notes Off and stops only the process SHR-DAW started before opening
the next one.

Each engine has a configured MIDI input and JACK audio output. See
[Configuration and routing](CONFIGURATION.md) for the settings.

By default, the active engine connects directly to the configured playback
pair. With the opt-in owned effects graph enabled, that same one engine instead
passes through source inserts, two aux returns, the master rack, and final
meter. Activation is transactional and restores the direct path on failure.
The separate loop player and recorder do not pass through this graph.

## External MIDI instruments

Each tracker page can use its own MIDI output, with four independent column
channels/banks/programs. Several pages can
play several hardware instruments at the same time. A page can also target the
currently active SHR-DAW software instrument.

Portable `AUTO` pages save no output or channel and follow the machine default.
Explicit pages remember their preferred port. If it is disconnected, the page
shows `FALLBACK` while the configured output or loaded instrument is usable,
otherwise `OFFLINE`. Its notes and preference are kept; reconnecting it makes
the original mapping usable on the next play without rewriting the Project.

Named sound lists for supported external instruments come from
[MIDI device profiles](MIDI_DEVICE_PROFILES.md). Instruments without a profile
still have the normal MIDI program numbers 0–127.

## Audio output and recording

The setup wizard selects the left and right JACK playback ports and the left
and right recording ports.

When an audio interface has direct monitoring, connect an external instrument
to its inputs and use the interface's monitor balance. This mixes the external
sound with SHR-DAW software instruments without a second software-monitoring
path or its additional CPU work.

The same JACK capture inputs remain available to the stereo recorder. SHR-DAW
does not currently send capture audio back to playback for software monitoring.
External-instrument audio, hardware returns/sends, and the WAV loop are not
currently mixed or metered by SHR-DAW's master rack; combine them with hardware
direct monitoring or an external mixer. The WAV Loop screen's separate
`LOOP OUT` meter observes that loop alone and does not change this mixing
boundary.

For exact routes and configuration keys, read
[Configuration and routing](CONFIGURATION.md).
