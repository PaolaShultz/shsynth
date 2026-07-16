# Installation

SHR-DAW is developed and tested with real audio and MIDI hardware on Patchbox
OS based on Debian 12 (Bookworm).

A clean Debian 11 (Bullseye) ARM64 system can install the required packages,
build the locked project with Rust 1.85, and pass the test suite. Audio and MIDI
hardware have not been tested there. Raspberry Pi OS Bullseye and Debian or
Raspberry Pi OS 13 (Trixie) are expected to work, but are not hardware-tested.

## Install

From the project directory, run:

```sh
./scripts/install.sh
```

The installer:

- installs build, JACK, and ALSA tools;
- installs synthv1, Yoshimi, FluidSynth, and a small default SoundFont;
- builds the release version of SHR-DAW;
- opens the routing wizard.

Use `--no-deps` to keep the installer from installing system packages. Use
`--no-config` to skip the routing wizard:

```sh
./scripts/install.sh --no-deps
./scripts/install.sh --no-config
```

## Installed commands

- `shr` opens SHR-DAW and provides its command-line tools.
- `shr-setup` opens the routing wizard.
- `shr-audio-tune` manages optional Raspberry Pi audio CPU tuning.
- `shs` and `synth-player` are compatibility names for `shr`. They use the same
  Rust engine ownership, routing, and shutdown path as the main command.

The product and Cargo package are named `shr-daw`. The main command is `shr`.
Existing `shsynth` configuration and data paths are kept for compatibility.

## JACK

JACK must be running before loading a software synth, playing WAV loops, or
recording audio. The browser and external-MIDI tracker can start without JACK.

The setup wizard can create a backed-up `~/.jackdrc` for a selected Raspberry
Pi or USB audio device. It never starts or restarts JACK. This avoids changing
a live audio session without the user's control. Choose a JACK sample rate that
matches the WAV loops you intend to use, such as 44100 Hz for CD-rate loops or
48000 Hz for 48 kHz material.

## Optional dedicated audio CPU

On a Raspberry Pi with at least four cores, the setup wizard can reserve one
CPU for JACK and the one software synth managed by SHR-DAW. This is disabled by
default.

The optional profile:

- pins JACK and the managed synth to the selected CPU;
- keeps normal interrupt handling on the other CPUs;
- configures full-tickless and RCU offload at boot;
- uses the `performance` CPU governor while its service is active;
- backs up the boot command line;
- refuses to replace CPU isolation settings it did not create.

The wizard does not restart JACK or reboot the Pi. Check or remove the managed
settings with:

```sh
shr-audio-tune status
sudo shr-audio-tune remove
```

After removing them, clear `audio.engine_cpu` in `shsynth.conf` and reboot.
CPU isolation leaves fewer cores for normal system work. It can improve audio
scheduling, but it cannot prevent every xrun caused by hardware, firmware, or
an unsuitable JACK buffer size.

Continue with [First run](FIRST_RUN.md).
