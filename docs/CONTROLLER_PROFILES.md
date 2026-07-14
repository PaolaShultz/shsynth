# Automatic controller setup and MIDI learn

SHR-DAW uses a small reviewed input-controller catalog plus MIDI learn. A
controller profile describes messages produced by physical knobs, encoders,
and buttons. It is different from an external-instrument profile, which
describes messages accepted by a synthesizer.

Run the normal setup wizard after connecting a controller:

```sh
shr-setup
```

The wizard selects an ALSA MIDI input, loads a matching known profile, or
offers non-audible MIDI learn for the missing controls. Learning never forwards
messages to a synth. It can identify absolute knob/fader CCs, either direction
convention for a relative encoder, a CC or note encoder press, and command
buttons that send either notes or CCs. Conflicting assignments are rejected.

The generic installed `controller.conf` is deliberately empty. An unknown
device therefore remains a normal musical MIDI input instead of accidentally
inheriting another controller's command notes.

## Commands

```sh
shr pads ports                 # list detected MIDI inputs
shr pads profiles              # list installed known profiles
shr pads auto [PORT_MATCH]     # select input and apply a known profile
shr pads learn [PORT_MATCH]    # learn only what remains unassigned
shr pads update                # download the reviewed SHR catalog
shr pads list                  # show the resulting mapping
```

The bundled catalog lives in `controller-profiles/catalog.json`. Installation
copies it below `share/shsynth/controller-profiles/`. `shr pads update`
downloads the current catalog from the SHR-DAW public repository, validates it
fully, and atomically installs it below
`${XDG_DATA_HOME}/shsynth/controller-profiles/`. Set
`SHSYNTH_CONTROLLER_PROFILE_DIR` for a private override. Machine-specific
learned mappings remain in the private state directory as `controller.conf`.
The setup helper uses `SHSYNTH_STATE_DIR` internally when an explicit
`--state-dir` is supplied.

## Upstream mapping sources

There is no universal controller-description standard. These projects provide
useful input-controller knowledge:

- [Ardour MIDI maps](https://github.com/Ardour/ardour/tree/master/share/midi_maps)
  cover many keyboard controllers and control surfaces.
- [Mixxx controller mappings](https://github.com/mixxxdj/mixxx/tree/main/res/controllers)
  cover many DJ and grid controllers, including USB identifiers and scripts.
- [Zynthian controller drivers](https://github.com/zynthian/zynthian-ui/tree/master/zyngine/ctrldev)
  demonstrate matched plug-and-play drivers plus MIDI learn.

Their mappings bind hardware to application-specific actions and may execute
device setup or LED scripts. SHR-DAW does not download or run those files.
Reviewed profiles may use their documentation as a source, but raw note/CC
facts must be verified on hardware and recorded with provenance. This keeps a
foreign transport command from silently becoming an SHR panic, record, or
navigation command. Those upstream repositories use copyleft licences; none
of their mapping data is included in the MIT SHR catalog.

The [Pencil Research MIDI dataset](https://github.com/pencilresearch/midi) is
CC BY-SA 4.0 and is valuable for external synth CC/NRPN and drum-note profiles.
It does not describe the physical controls emitted by USB input controllers,
so it is not used for controller autoloading.

## Catalog profile format

Each JSON entry has stable `id`, display `name`, normalized ALSA
`match_names`, a 4/5/8-button layout, and any known mappings. `controls` maps
incoming physical CC numbers to the twelve canonical synthv1 CCs.
`note_buttons` and `cc_buttons` map physical messages to page/item roles.
Encoder, press, and optional lock messages are separate so they cannot collide
with continuous controls.

Profiles may be partial. After one is loaded, `shr pads learn` asks only for
continuous controls and encoder functions that are still empty; command-button
learning is optional and replaces the chosen button layout as one verified
set.
