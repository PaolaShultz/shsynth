# Licensing and redistribution

SHR-DAW source code, `Velvet Tines.synthv1`, the 20 newly authored presets, and
the bundled original drum-pattern data are released under the MIT license in
`LICENSE`. MIT permits commercial and non-commercial use, modification, and
redistribution, but the copyright and license notice must be kept. No
MIT/Apache/GPL license is literally obligation-free.

## Rust dependencies

The direct Rust crates are permissive:

- anyhow, libc, serde, serde_json, signal-hook: MIT or Apache-2.0;
- hound: Apache-2.0;
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

## Documentation rendering tools

The deterministic TUI screenshots are generated from SHR-DAW's own seeded
screen data by `scripts/render-readme-screenshots.py`. That maintainer helper
uses Pillow as a PNG container/writer and reads the host's
`Lat15-VGA16.psf.gz` Linux console font; it does not bundle Pillow or the PSF
font into the installed product-data directory. Anyone redistributing the
generated documentation or a fallback copy of that font should retain and
review the copyright/licence information supplied by the corresponding Pillow
and console-font packages. The images are presentation fixtures, not evidence
that JACK, MIDI hardware, playback, or recording was active.

## Preset bank boundary

The tracked `presets/synthv1/` directory and public installation contain only
the 21 MIT-cleared presets identified by
`presets/synthv1/cleared-presets.txt` and described in this file. That manifest
is the single packaging and schema-test allowlist. Legacy or downloaded
presets without verified authorship and redistribution terms belong only in the
ignored private `user/presets/synthv1/` tree. They must not be committed,
packaged, mirrored, or relabelled as MIT merely because a copy exists locally.

The `392 Synthv1 Presets` archive published by LinuxSynths was inspected on
2026-07-13. It contains 392 `.synthv1` files but no README, license, author
notice, or redistribution grant. Twenty-eight filenames overlapped the legacy
local bank, although those local files were modified rather than byte-identical,
so the collection is a likely source for part of that private bank. A private
extraction described in `docs/WORKSPACE_HANDOFF.md` remains below the ignored
`user/` tree; none of it was imported into the tracked or installed public
collection. Source:
<https://linuxsynths.com/Synthv1PatchesDemos/synthv1.html>.

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
choices. Add a newly cleared file to `presets/synthv1/cleared-presets.txt` only
after its provenance is recorded here. Static tests verify the manifest, all
145 indices and names, and mapped values against the current schema. Sound
quality still requires authorized listening tests; none were run while adding
this set.

## Original drum patterns

The 60 compact catalog entries and 12 standalone `.shdrum` files in
`drum-patterns/` were authored as editable SHR-DAW MIDI data and are released
under MIT. They use conventional General MIDI-style drum-note assignments, but
they do not copy or transcribe third-party MIDI files or protected recordings.
Genre names are creative navigation hints rather than claims of an
authoritative cultural transcription. The structural and naming audit is in
`docs/DRUM_PATTERN_AUDIT.md`; groove, feel, and final curation still require
human listening.

## Bundled and private-download WAV loops

The four WAVs named in `loops/cleared-loops.txt` are redistributable under CC0
1.0 rather than MIT. Their authors, source pages, conversions, exact tempo and
hashes are recorded in `loops/SOURCES.md`. The three `starter-100-step-*.wav`
files come from obscure music's CC0 **Music loop variations** pack; the
`war-drums-130.wav` file is William Hector's CC0 **Horde War Drums loop**.
Packaging must continue to use the manifest allowlist.

`shr-setup` can also download four tempo-labelled WAVs from MusicRadar's
**SampleRadar: 183 free 80s pop drums samples** directly into the user's loop
inbox. MusicRadar permits use in music but says not to redistribute the raw
samples, so neither the 78 MB archive nor extracted WAVs may be committed,
packaged, mirrored, or copied into a public release. Setup asks first, stores a
source/terms note beside the private files, and downloads from:
<https://www.musicradar.com/news/sampleradar-free-80s-pop-drums-samples>.
