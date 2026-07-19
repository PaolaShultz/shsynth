# First run

You do not need a full hardware studio to start. SHR-DAW is designed around a
small paged MIDI control surface, with computer-keyboard controls kept as a
fallback for setup and development. External synths, an audio interface, mixer,
and dedicated display are optional additions.

## Configure and start

Run these commands after installation:

```sh
shr-setup
shr doctor
shr
```

The setup wizard finds ALSA MIDI ports and JACK audio ports. You can enter an
exact port name if automatic detection is not enough. Run `shr-setup` again
after changing a controller, MIDI interface, sound card, or JACK port layout.

`shr doctor` checks the complete audio/MIDI setup, so it reports missing JACK
as a problem. The preset browser and external-MIDI tracker can still open
without JACK; loading a software instrument, playing a WAV loop, and recording
audio require it. SHR-DAW does not start or restart JACK.

The owned effects graph is disabled by default, so a managed software
instrument initially uses the configured direct playback pair. Playback
**SOUND** → **FX** always opens the Project's source rack, two pre/post wet aux
buses, and master rack. With the graph disabled those controls design and save
the Project silently; direct playback cannot process or meter them. Enabling
the graph in `shsynth.conf` makes that route audible and adds the final meter.
The separate WAV loop and stereo capture paths do not pass through the current
master rack. Read [How SHR-DAW works](HOW_IT_WORKS.md) before changing that
routing boundary.

## Choose how to play

- Use the configured control surface for the primary four-page/four-button
  menus and synth controls.
- Add a MIDI keyboard to play velocity, chords, and live recordings.
- If the controller is not ready yet, use the computer keyboard fallback to
  navigate setup and enter tracker notes with `Z S X D C V G B H N J M`.
- Add external MIDI instruments and audio hardware only when useful.

Computer-keyboard step entry is available, but it is not the primary hardware
workflow. Free live performance of a software synth from the computer keyboard,
a wider keyboard range, and more performance bindings are planned fallback
features. `?` and F1 already open Help.

## Terminal size

SHR-DAW is designed for a 40×20 terminal. It adapts to the terminal cell size
and reports when the window is too small. The installer does not change the
font, desktop, display resolution, or fullscreen settings.

Pixel resolution does not determine the number of terminal cells by itself.
The terminal program, font, scaling, window borders, and fullscreen state all
matter. Change those settings yourself if fewer than 40 columns or 20 rows fit.

## Run from a development checkout

For a self-contained local setup, use:

```sh
cargo +1.85.0 build --release --locked
./scripts/setup-local.sh
./scripts/local.sh
```

This keeps configuration, logs, ideas, songs, recordings, downloads, and local
presets below the ignored `user/` directory. Existing private files are not
replaced. Set `SHSYNTH_USER_DIR` to use a different private directory. These
helpers neither install packages nor start JACK; `setup-local.sh` only runs the
routing wizard, and `local.sh` launches the already-built binary.

## If setup is unusual

The installer and setup wizard are the normal path. For uncommon controllers,
complex routing, or recovery, Codex CLI can use the project's assisted-setup
brief. Follow the official
[Codex CLI installation and sign-in guide](https://developers.openai.com/codex/cli/),
then run:

```sh
cd /path/to/shr-daw
codex -C . "$(cat docs/CODEX_ASSISTED_SETUP.md)"
```

This optional path can identify controller messages one physical control at a
time, inspect ALSA MIDI and JACK routes, repair setup problems, and help with
SoundFonts or complex external-instrument routing. Read the
[assisted-setup brief](CODEX_ASSISTED_SETUP.md) for its safety rules. Audible
tests and system-wide changes still require the user's permission.

Known USB controllers are matched during `shr-setup`; unknown devices can be
mapped immediately with the non-audible MIDI learner. Profiles remain ordinary
data and learned mappings remain private. See
[Automatic controller setup and MIDI learn](CONTROLLER_PROFILES.md). Assisted
discovery remains useful for proprietary modes, displays, LED feedback, or
deeply customized hardware.

Next, read [Using SHR-DAW](USING_SHR_DAW.md). For a larger hardware rig, see
[Physical connections](CONNECTIONS.md).
