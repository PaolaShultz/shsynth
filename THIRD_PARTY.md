# Licensing and redistribution

SHR-DAW source code, `Velvet Tines.synthv1`, and the 20 newly authored presets
listed below are released under the MIT license in `LICENSE`. MIT permits commercial and non-commercial use,
modification, and redistribution, but the copyright and license notice must be
kept. No MIT/Apache/GPL license is literally obligation-free.

## Rust dependencies

The direct Rust crates are permissive:

- anyhow, libc, signal-hook: MIT or Apache-2.0;
- crossterm, midir, quick-xml, ratatui: MIT.

Their transitive dependencies reported by `cargo metadata --locked` are also
permissive (MIT, Apache-2.0, ISC-style combinations, Unicode-3.0, or Unlicense).
Before publishing binary releases, generate and ship the exact notices for the
locked target dependency set with a tool such as `cargo-about` or
`cargo-deny`.

## External system software

SHR-DAW launches these separately installed programs; it does not copy or link
their source into this project:

- synthv1: GPL-2.0-or-later;
- Yoshimi: GPL-2.0-or-later overall, with some source under compatible
  LGPL/CC/ISC terms as documented by its package;
- FluidSynth: LGPL-2.1-or-later;
- JACK2 tools/libraries: primarily LGPL-2.1-or-later;
- `aconnect` from alsa-utils: GPL-2.0;
- ALSA library used by the Rust MIDI backend: LGPL-2.1-or-later.

SHR-DAW indexes Yoshimi `.xiz` banks and FluidSynth `.sf2`/`.sf3` files at
their configured system/user paths; it does not copy them into this project.
The Debian `yoshimi-data` package is GPL-2.0-or-later overall (with a few bank
files under GPL-3), and the packaged TimGM6mb SoundFont is GPL-2-only. Their
copyright files remain the authority for redistribution of an appliance image.

Using system-installed executables/data does not force SHR-DAW itself to use GPL.
Anyone distributing an appliance image or bundling those binaries must comply
with each package separately and provide their notices/source offers as
required. This is a practical audit, not legal advice.

## Controller mapping references

SHR-DAW can update its own MIT-licensed, hardware-verified controller catalog.
It does not redistribute or execute mappings from Ardour, Mixxx, or Zynthian.
Those copyleft-licensed projects are documented as useful research sources in
`docs/CONTROLLER_PROFILES.md`; application-specific bindings are not copied
into this repository. Pencil Research's CC BY-SA 4.0 MIDI dataset documents
external instruments rather than USB controller surfaces and is likewise not
included.

## Preset bank

The origin and redistribution terms of the legacy presets in
`presets/synthv1/` are not recorded in the files and could not be verified.
They must not be included in a public release until their authors/licenses are
identified or they are replaced with newly authored presets. Do not infer
clearance merely because a legacy file is present in a local checkout.

The `392 Synthv1 Presets` archive published by LinuxSynths was inspected on
2026-07-13. It contains 392 `.synthv1` files but no README, license, author
notice, or redistribution grant. Twenty-eight filenames overlap this local
legacy bank, although the local files are modified rather than byte-identical,
so the collection is a likely source for part of the bank. The archive was not
imported. Source: <https://linuxsynths.com/Synthv1PatchesDemos/synthv1.html>.

## Newly authored cleared synthv1 presets

The following complete synthv1 0.9.29 presets were authored for SHR-DAW on
2026-07-13 from the MIT-cleared `Velvet Tines` schema/template, with new
parameter designs. They contain no imported samples or third-party preset
content and are released under this repository's MIT license:

- basses: Deep Sub, Liquid Acid, Rubber Circuit, Compact Bass;
- leads: Mono Pulse Lead, PWM Horizon, Glass Saw Lead;
- pads: Warm Cloud, Dark Canopy, Shimmer Veil;
- plucks/bells: Copper Pluck, Reed Pluck, Silver Bell, Soft Chime;
- organs: Drawbar Glow, Hollow Organ;
- drones/effects: Low Orbit Drone, Frozen Drone, Dust Delay, Restrained Sweep.

`scripts/generate_cleared_presets.sh` records the exact authored parameter
choices. Static tests verify all 145 indices and names against the current
schema. Sound quality still requires authorized listening tests; none were run
while adding this set.
