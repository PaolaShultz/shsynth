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
buttons that send either notes or CCs. Each step keeps the first qualifying
gesture. The in-app learner visibly keeps `OK` on that role until the physical
gesture is finished: a button advances on its matching CC-off, Note Off, or
velocity-zero Note On, while a knob/fader or relative encoder advances
automatically after its CC stream has been quiet for the short settle period.
Extra values and encoder neutral/reset packets extend that same gesture instead
of becoming the next role. On entry, release the control that opened MIDI Learn
and wait for the ready indication; its release and already queued traffic are
quarantined.

First turn the master encoder left and let it settle, turn it right and let it
settle, then click and release it. The learned encoder then browses the optional
control and command-button roles. One rotary gesture moves by exactly one role,
regardless of how many packets it emits. Each learned absolute control advances
to the next control automatically after settling, and each learned command
button advances after release. The next clean encoder click saves the mappings
learned so far, makes a backup, activates the new file, and exits after release;
Esc cancels and keeps the previous file. Conflicting assignments from a
different already-mapped control are rejected without replacing an accepted
`OK` message with errors from trailing traffic. Relative encoders using either
the center-64 convention or high/low values such as 125–127 left, 1–3 right,
and neutral 0 are supported.

SHR does not guess how many buttons the controller has. Command roles are
optional: browse past roles that the hardware does not need. The page choices
are mutually exclusive. If any of page 1–4 is learned, the separate page-cycle
role is bypassed; after all four are skipped, page-cycle is offered as the
five-button alternative before item 1–4. Item buttons alone use the four-button
layout.

Page-cycle may be one dedicated button or a held modifier plus another control.
For a dedicated button, press and release it once, then press it again to
confirm; this prevents a single exploratory Shift press from becoming the
mapping. For a chord, hold the modifier and move or press the intended trigger,
then release the modifier. The trigger may reuse a normally mapped knob or
button because it cycles the page only while that learned modifier is held;
one held chord triggers once regardless of packet count. Partial layouts are
valid, so spare hardware buttons can remain musical or unassigned.

The generic installed `controller.conf` is deliberately empty. An unknown
device therefore remains a normal musical MIDI input instead of accidentally
inheriting another controller's command notes. Selecting a different unknown
device with `shr pads auto` clears the previous device's mappings before MIDI
learn begins; the old file is backed up first.

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
Optional `note_button_channels` and `cc_button_channels` objects map the same
note/CC keys to 1-based MIDI channels. Missing qualifiers preserve legacy
all-channel profiles. MIDI learn records the channel observed for every learned
note or CC command, and save/load retains it.
Encoder, press, and optional lock messages are separate so they cannot collide
with continuous controls. All physical note and CC numbers must be valid MIDI
data bytes (0–127), and an encoder press cannot reuse a command-button note.
Learned page-cycle chords are stored as `page_cycle.modifier` and
`page_cycle.trigger` values such as `cc.1.27`; the modifier and trigger must be
different messages, while the trigger may deliberately reuse a normal mapping.

Profiles may be partial. After one is loaded, `shr pads learn` asks only for
continuous controls and encoder functions that are still empty; command-button
learning is optional and replaces the chosen button layout as one verified
set.

The reviewed MiniLab 3 profile uses factory Arturia/DAW pad notes 36–43 on
channel 10. Direct capture on this unit found User 1 pads on channel 1, the same
channel as its keyboard, so User 1 pads are not safe command buttons: their
messages are indistinguishable from keyboard notes. DAW Shift emits CC27, but
the profile deliberately does not bind it as persistent pad lock; normal
arpeggiator, program, and bank gestures therefore cannot toggle SHR lock state.
Selecting the controller's DAW program does not itself require a proprietary
DAW script for these ordinary MIDI note commands. Arturia mode has the same
captured channel-10 pad notes, so use DAW mode only if another ordinary mapping
has been verified to be useful.
