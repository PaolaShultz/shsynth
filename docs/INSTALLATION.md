# Installation

SHR-DAW is developed and tested with real audio and MIDI hardware on Patchbox
OS based on Debian 12 (Bookworm).

A clean Debian 11 (Bullseye) ARM64 system can install the required packages,
build the locked project with Rust 1.85, and pass the test suite. Audio and MIDI
hardware have not been tested there. Raspberry Pi OS Bullseye and Debian or
Raspberry Pi OS 13 (Trixie) are expected to work, but are not hardware-tested.

The supported family is Debian-based Linux. Rust 1.85, Cargo, a C build
toolchain, `pkg-config`, Python 3, ALSA development/runtime tools, and JACK2 are required
to build the complete binary. A running JACK server is optional for browsing
and editing but required for software-instrument audio, WAV-loop playback, and
stereo recording. synthv1, Yoshimi, and FluidSynth/TimGM are separate optional
sound engines at runtime; the default installer includes all three so their
catalogs are useful immediately. MIDI controllers, external instruments, audio
interfaces, and a 480×320 display are optional hardware.

## Install

From the project directory, run:

```sh
./scripts/install.sh
```

The installer:

- installs build, JACK, and ALSA tools;
- installs synthv1, Yoshimi, FluidSynth, and a small default SoundFont;
- installs/selects the official Rust 1.85 toolchain when the current Cargo is
  older, runs the locked tests, and builds the locked release version;
- installs commands, templates, the 21 allowlisted presets, four allowlisted
  CC0 48 kHz loops, ten manifest-cleared demo Projects plus MIDI files,
  device/controller profiles, drum data, documentation, and
  all 80 menu-manual images below the selected prefix (normally `/usr/local`);
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

## Repository-local evaluation

Contributors can build and inspect the checkout without installing files:

```sh
cargo +1.85.0 test --locked
SHSYNTH_STATE_DIR=/tmp/shr-daw-judge-state cargo +1.85.0 run --locked -- config init
SHSYNTH_STATE_DIR=/tmp/shr-daw-judge-state cargo +1.85.0 run --locked -- list
SHSYNTH_STATE_DIR=/tmp/shr-daw-judge-state cargo +1.85.0 run --locked -- screenshots > /tmp/shr-daw-screens.json
```

This path does not start JACK or transmit MIDI. Delete the two explicit `/tmp`
paths afterward. For a persistent private development checkout,
`./scripts/setup-local.sh` and `./scripts/local.sh` redirect configuration,
Projects, Ideas, recordings, loops, and private presets below ignored `user/`.
They copy missing public presets, starter loops, and demo Projects without
replacing private files. Build the release binary first; neither helper installs packages or
builds the program.

## Upgrade and uninstall boundaries

Rerunning `./scripts/install.sh` builds the locked current checkout and replaces
installed program/shared documentation files. Existing XDG configuration,
controller learning, Projects, Ideas, loops, and recordings are not removed or
reset. Run `shr-setup` only when routes or hardware need to change.

For a default `/usr/local` source installation, remove installed SHR-DAW files
from this checkout with:

```sh
sudo make uninstall
```

This removes the installed commands, public presets, profiles, rhythms, and
documentation. It deliberately preserves user data under
`${XDG_STATE_HOME:-~/.local/state}/shsynth/` and
`${XDG_DATA_HOME:-~/.local/share}/shsynth/`, repository-local `user/`, system
packages, JACK policy, and setup backups. Optional CPU/audio tuning is also a
separate explicit system change; inspect/remove it with `shr-audio-tune` before
uninstalling the command if desired. Never delete those retained directories
unless their Projects, Ideas, recordings, loops, and private presets have been
reviewed and backed up.

The Makefile install/uninstall file boundary was validated in an isolated
`DESTDIR`: 21 allowlisted public presets and only manifest-cleared demos were
installed, no `user/` path was included, and staged uninstall removed only
staged product files.

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
