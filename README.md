<p align="center">
  <img src="docs/images/shr-daw-header.png" alt="SHR-DAW" width="100%">
</p>

SHR-DAW is a small Raspberry Pi music workstation. It runs in a 40×20 terminal
and can be used without a mouse.

Use it to play software instruments, control external MIDI instruments, build
FT2 Patterns, arrange complete Projects, save MIDI ideas, and record stereo
audio. Start with only a Raspberry Pi and a computer keyboard. Add a MIDI
controller, synthesizer, audio interface, or dedicated screen when you need one.

## App screens

SHR-DAW is split into focused 40x20 pages, so performance controls, FT2 pattern
editing, arrangement, routing, files, loops, and recording stay separate.

<table>
  <tr>
    <td align="center" width="50%">
      <img src="docs/images/shr-daw-presets.png" alt="Preset browser showing synthv1 sounds" width="360"><br>
      <strong>Presets</strong><br>
      Browse playable software-instrument sounds.
    </td>
    <td align="center" width="50%">
      <img src="docs/images/shr-daw-playback.png" alt="Playback screen with held notes, control indicators, and recorded MIDI status" width="360"><br>
      <strong>Playback</strong><br>
      Play, record, review, and save MIDI ideas.
    </td>
  </tr>
  <tr>
    <td align="center" width="50%">
      <img src="docs/images/shr-daw-ft2-pattern.png" alt="FT2 Pattern editor with four lanes of note data" width="360"><br>
      <strong>FT2 Pattern Editor</strong><br>
      Edit notes, velocity, programs, gates, and commands.
    </td>
    <td align="center" width="50%">
      <img src="docs/images/shr-daw-ft2-arrangement.png" alt="FT2 Arrangement screen listing ordered pattern steps" width="360"><br>
      <strong>FT2 Arrangement</strong><br>
      Chain Pattern IDs into the Project timeline.
    </td>
  </tr>
  <tr>
    <td align="center" width="50%">
      <img src="docs/images/shr-daw-ft2-pages.png" alt="FT2 Pattern page routing screen with MIDI channels and targets" width="360"><br>
      <strong>Pattern Pages</strong><br>
      Keep routes, channels, lanes, mutes, and targets inside each Pattern.
    </td>
    <td align="center" width="50%">
      <img src="docs/images/shr-daw-project-files.png" alt="Project Files screen listing saved Projects" width="360"><br>
      <strong>Project Files</strong><br>
      Save, load, preview, and delete whole Projects.
    </td>
  </tr>
  <tr>
    <td align="center" width="50%">
      <img src="docs/images/shr-daw-ft2-loop.png" alt="FT2 WAV Loop screen with tempo and beat-region controls" width="360"><br>
      <strong>FT2 WAV Loop</strong><br>
      Import private loops and align them to tracker tempo.
    </td>
    <td align="center" width="50%">
      <img src="docs/images/shr-daw-audio-recorder.png" alt="Stereo recorder screen with input ports and recording status" width="360"><br>
      <strong>Stereo Recorder</strong><br>
      Capture a JACK stereo input as a 24-bit WAV file.
    </td>
  </tr>
</table>

## Start simple, expand when you want

SHR-DAW can sit at the center of a larger music setup, but the devices in this
diagram are optional. Begin with a Raspberry Pi and an audio output. Add a MIDI
controller, external instruments, an audio interface, a mixer, or a dedicated
screen as your setup grows.

![SHR-DAW connected to an optional controller, display, audio interface, MIDI instruments, mixer, speakers, and headphones](docs/images/shr-daw-physical-connections.png)

The basic signal paths are controller → SHR-DAW, SHR-DAW → software or hardware
instruments, and audio → speakers or the stereo recorder. See
[Physical connections](docs/CONNECTIONS.md) for the detailed MIDI and audio
routes.

## What it does

- Plays synthv1, Yoshimi, and FluidSynth instruments.
- Routes one MIDI controller to software and hardware instruments.
- Builds Projects with self-contained FT2 Patterns and an FT2 Arrangement.
- Provides FT2 Play/Rec/Edit/N00B modes and scale-safe live MIDI input.
- Imports private WAV loops and synchronizes them with FT2 through JACK.
- Records free playing as reusable MIDI ideas.
- Records a stereo JACK input as a 24-bit WAV file.
- Works from a computer keyboard or a small physical controller.

SHR-DAW is designed as a portable music box. It is not tied to one controller,
synthesizer, or audio interface. Hardware names and routes are configured by
the user.

## Quick start

On Patchbox OS, Raspberry Pi OS, or Debian:

```sh
./scripts/install.sh
shr-setup
shr doctor
shr
```

JACK must be running before SHR-DAW starts. The setup wizard helps choose the
MIDI and audio ports, but it does not start or restart JACK.

Read [Installation](docs/INSTALLATION.md) for supported systems and installer
options. Then follow [First run](docs/FIRST_RUN.md) to configure and test your
setup.

## Documentation

### Use SHR-DAW

- [First run](docs/FIRST_RUN.md) — configure, check, and open SHR-DAW.
- [Using SHR-DAW](docs/USING_SHR_DAW.md) — instruments, screens, ideas, and
  audio recording.
- [In-app help](docs/HELP.md) — compact help text shown by `?`, F1, or the
  controller Help action. While Help is open, SHR-DAW also tries to serve the
  same page temporarily at `http://<LAN-IP>/help`.
- [Tracker guide](docs/TRACKER.md) — FT2 Patterns, pages, arrangement, step
  editing, live recording, and Project files.
- [Controller interface](docs/CONTROLLER_INTERFACE.md) — physical controls and
  the complete menu map.
- [Physical connections](docs/CONNECTIONS.md) — simple and expanded hardware
  setups, MIDI paths, and audio paths.

### Install and customize it

- [Installation](docs/INSTALLATION.md) — dependencies, installed commands, and
  optional Raspberry Pi audio tuning.
- [Configuration and routing](docs/CONFIGURATION.md) — configuration files,
  page targets, channels, and offline devices.
- [MIDI device profiles](docs/MIDI_DEVICE_PROFILES.md) — named sounds and bank
  data for external instruments.
- [Controller profiles and MIDI learn](docs/CONTROLLER_PROFILES.md) — automatic
  matching and non-audible setup for USB input controllers.
- [Codex-assisted setup](docs/CODEX_ASSISTED_SETUP.md) — optional help for
  unusual hardware or recovery.

### Understand or develop it

- [How it works](docs/HOW_IT_WORKS.md) — synth ownership, MIDI safety, pickup,
  recording, and data locations.
- [Add patches and sounds](docs/NEW_PATCHES.md) — create and validate synthv1
  presets.
- [Third-party software and sounds](THIRD_PARTY.md) — credits, licences, and
  redistribution rules.
- [Workspace handoff](docs/WORKSPACE_HANDOFF.md) — current development and
  local-machine context.

## License

SHR-DAW code and the 21 included presets are MIT licensed. See
[THIRD_PARTY.md](THIRD_PARTY.md) before packaging the project or adding sounds.

---

<p align="center">
While I was releasing the first version of this software, my uncle died. So I dedicate this project to him.<br>
Počivao u miru, striče Mile, puno te volim!
</p>
