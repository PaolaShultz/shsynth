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
Arturia MiniLab, an AudioBox USB interface, and a Casio Casiotone MT-240. Those
devices proved the workflow, but SHR-DAW is not designed around them.

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

## External MIDI instruments

Each tracker page can use its own MIDI output, with four independent column
channels/banks/programs. Several pages can
play several hardware instruments at the same time. A page can also target the
currently active SHR-DAW software instrument.

Songs save exact output port names. If a saved device is disconnected, the
page is shown as `OFFLINE`. Its notes and route are kept. Connect the device or
choose a different page target; SHR-DAW does not silently rewrite the song.

Named sound lists for supported external instruments come from
[MIDI device profiles](MIDI_DEVICE_PROFILES.md). Instruments without a profile
still have the normal MIDI program numbers 0–127.

## Audio output and recording

The setup wizard selects the left and right JACK playback ports and the left
and right recording ports.

When an audio interface has direct monitoring, connect an external instrument
to its inputs and use the interface's monitor balance. This mixes the external
sound with SHR-DAW software instruments without a second software-monitoring
path, added latency, or added CPU use.

The same JACK capture inputs remain available to the stereo recorder. SHR-DAW
does not currently send capture audio back to playback for software monitoring.

For exact routes and configuration keys, read
[Configuration and routing](CONFIGURATION.md).
