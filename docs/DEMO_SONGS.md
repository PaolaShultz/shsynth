# Public-domain demo songs

SHR-DAW ships ten original, editable arrangements of traditional or otherwise
public-domain compositions. They are musical starting points, not copies or
imitations of modern recordings. Every arrangement has five independent parts:
`Drums`, `Bass`, `Pad`, `Lead`, and `Counter`.

| Composition | BPM | Meter | Key | Suggested restyles |
| --- | ---: | ---: | --- | --- |
| House of the Rising Sun | 84 | 6/8 | A minor | folk rock, dark synthwave, ambient |
| Whiskey in the Jar | 108 | 4/4 | D major | Celtic rock, acoustic, festival EDM |
| Cotton-Eyed Joe | 126 | 4/4 | G major | country dance, chiptune, electro-folk |
| Scarborough Fair | 92 | 3/4 | D Dorian | medieval ambient, trip-hop, orchestral |
| Greensleeves | 96 | 3/4 | A minor | Renaissance, dream pop, cinematic |
| Amazing Grace | 76 | 3/4 | G major | gospel, ambient, orchestral build |
| Drunken Sailor | 116 | 4/4 | D minor | punk shanty, industrial, festival folk |
| Wellerman | 104 | 4/4 | E minor | sea folk, synth-pop, drum-and-bass halftime |
| Auld Lang Syne | 88 | 4/4 | D major | piano ballad, post-rock, New Year synthwave |
| Danny Boy | 72 | 4/4 | D major | cinematic, ambient, slow rock |

Setup copies each native `.shsong` into the XDG song directory without
replacing an existing file, so it appears directly on FT2 **FILES**. Its pages
use portable `AUTO` routing. Matching format-1 `.mid` files and a copy of the
manifest are kept below `${XDG_DATA_HOME:-~/.local/share}/shsynth/demos/`.

The canonical metadata is [`demos/cleared-demos.json`](../demos/cleared-demos.json).
For every title it records the short arrangement description, exact filenames
and SHA-256 hashes, public-domain reasoning, institutional source links, and
the MIT licence for SHR-DAW's newly authored arrangement. House of the Rising
Sun remains 6/8 in MIDI; because the current tracker stores a 3- or 4-beat grid
without a denominator, its native Project uses the documented 3/4 compound
grid while preserving the two-beat feel.

Maintainers regenerate or validate the corpus with:

```sh
./scripts/generate_demo_songs.py --write
./scripts/generate_demo_songs.py
make check-demos
```

The normal command is validation-only. Packaging first requires an exact
deterministic match, then installs only paths emitted from the cleared manifest.
An altered, missing, malformed, or extra file therefore stops the package.
