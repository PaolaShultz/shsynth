# Configuration and tracker routing

SHSynth is a Raspberry Pi mini DAW and MIDI routing hub. Hardware names belong
in `shsynth.conf`, `controller.conf`, or a saved song. They are not compiled
into the program.

## Tracker pages

Every FT2 page has four note lanes. Pages play at the same time and each page
stores:

- its target;
- MIDI channel 1–16;
- bank, program, velocity, mute, and percussion settings;
- four lane names and lane mute states;
- a reserved list of MIDI setup messages for later use.

Open **PAGES** on the tracker screen. Use the main encoder or **PAGE−** and
**PAGE+** to select a page. **ADD** creates another four-lane page. **TARGET**
chooses an ALSA MIDI output that is currently visible, the active SHSynth
software instrument, or the configured compatibility output. **CHANNEL**
chooses 1–16. Encoder press confirms a field. **DONE** keeps all page changes;
**CANCEL** restores the song from before page management opened.

The active-instrument choice always means the single software instrument that
SHSynth currently owns and monitors. It does not start another engine. It is
offline when no managed instrument is active.

An exact hardware port name is saved in the song. If that device is later
missing, the page shows `OFFLINE`. SHSynth keeps the name and pattern data,
does not rewrite the file, and continues playing pages whose targets are
available.

## Compatibility output

The `external_midi.*` settings remain the default route for new songs and the
safe route used when loading version-1 or version-2 songs. They also hold
tracker timing, gate, bank/program, transport, live-thru, and optional drum-map
defaults.

The most important compatibility keys are:

```text
external_midi.enabled=true
external_midi.client=shs-tracker
external_midi.output=part of the ALSA output port name
external_midi.melody_channel=1
external_midi.percussion_channel=10
```

These example values are not device requirements. Run `shr-setup` and choose
the ports present on the Raspberry Pi. The Casiotone profile in the bundled
example is only the original proof-of-concept profile.

## Song files

Songs are stored below
`${XDG_DATA_HOME:-~/.local/share}/shsynth/songs/`. Song format version 3 adds a
target and reserved setup messages to each page and allows any positive number
of four-lane pages (up to the validation limit).

Version-1 tracks are explicitly converted into four-lane `MELODY` and `DRUMS`
pages. Version-2 pages keep their lanes, channels, programs, and pattern cells.
Both old versions use the configured compatibility output because those files
did not store a device. A save writes version 3. Unsupported newer versions are
refused and are never overwritten or deleted by SHSynth.

## Note ownership

Tracker events are sent only to their page target and channel. Controller
command pads, the encoder, and mapped controls stay inside SHSynth. STOP, page
mute, lane mute, song replacement, route changes, and exit release notes only
on affected destinations. Lanes that share a device/channel keep separate note
ownership; a shared note is released only after its last lane owner ends.
