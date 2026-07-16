# New patches and sounds

## Add a patch

Put a descriptively named `.synthv1` file in `presets/synthv1/`. The filename is
the browser name, so use readable title case and avoid numbered experiments.
For a new sound, copy a complete current preset such as `Velvet Tines.synthv1`
as a structural template, then change the sound parameters and root `name`.

Use synthv1's current 0.9.29 layout: 145 parameters indexed 0–144. Keep every
index unique and consecutive, but treat the `name` attribute as authoritative;
SHR-DAW deliberately reads values by name so old presets still work.

## Design notes

synthv1 contains two independent synth sections, each with two oscillators,
filter/LFO/amplifier envelopes, output, velocity, and controller settings. Their
combined signal passes through chorus, flanger, phaser, delay, reverb, and
dynamics. Use the second section for a transient, interval, sub layer, or stereo
counterpart instead of only duplicating the first.

The 12 panel values come from these parameters:

| Index | Name | Range |
|---:|---|---:|
| 17 | `DCF1_CUTOFF` | 0–1 |
| 18 | `DCF1_RESO` | 0–1 |
| 21 | `DCF1_ENVELOPE` | −1–1 |
| 30 | `LFO1_RATE` | 0–1 |
| 44 | `DCA1_VOLUME` | 0–1 |
| 45–48 | `DCA1_ATTACK` through `DCA1_RELEASE` | 0–1 |
| 132–134 | `DEL1_WET`, `DEL1_DELAY`, `DEL1_FEEDB` | 0–1 |

Choose their initial values carefully: they are the pickup targets, the
Playback-screen encoder reset values, and neutral reference for the colored
indicators. Keep output levels conservative and leave the limiter enabled
unless there is a reason not to.

## Validate

Do not overwrite an existing patch while experimenting. Check that the new
sound is discoverable and that the project still passes:

```sh
target/release/shr list
PATH=/home/patch/.rustup/toolchains/1.85.0-aarch64-unknown-linux-gnu/bin:$PATH cargo test --locked
```

If `xmllint` is installed, also run:

```sh
xmllint --noout "presets/synthv1/New Sound.synthv1"
```

Static validation cannot judge tone. Audition through the normal interface only
when authorized, compare soft and hard velocities across several octaves, test
chords for clipping, and confirm the in-place parameter reset plus all 12 pickup
controls afterward.

For imported patches, record their origin and license in this document or a
nearby note. Do not assume a preset found online is redistributable. Once a
patch is cleared for public packaging, add its exact filename to
`presets/synthv1/cleared-presets.txt`; unlisted files are deliberately not
installed and make the repository schema test fail.

## Cleared SHR-DAW collection

`Velvet Tines` and the 20 category-focused presets listed in `THIRD_PARTY.md`
are newly authored/cleared under MIT. Their exact parameter recipes are retained
in `scripts/generate_cleared_presets.sh`; that generator refuses to overwrite
an existing file. System Yoshimi instruments and SoundFonts must be configured
and indexed in place, not copied here or relabeled as MIT content.
