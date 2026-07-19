# Bundled loop sources

Only WAVs named in `cleared-loops.txt` are packaged. All are stereo, 48 kHz,
24-bit PCM and released under
[CC0 1.0](https://creativecommons.org/publicdomain/zero/1.0/).

| Bundled file | BPM / length | Source | Conversion | SHA-256 |
|---|---:|---|---|---|
| `starter-100-step-1.wav` | 100 / 4 bars | `level1-step1.wav` from [Music loop variations](https://opengameart.org/content/music-loop-variations), obscure music | SoX very-high-quality 44.1-to-48 kHz conversion, input gain 0.8, trimmed to exactly 460,800 frames | `9d2fdff6b039a50d702e947e822d5e8d1c136749f542349e3776f13f3c4ed6e8` |
| `starter-100-step-2.wav` | 100 / 4 bars | `level1-step2.wav` from the same pack | Same conversion | `5a376ca8cd08538cdca4d196458a630bd7f061a0a08d8fe26b64d6a6d022078f` |
| `starter-100-step-3.wav` | 100 / 4 bars | `level1-step3.wav` from the same pack | Same conversion | `cee489d22fc0436b1708e6cf4a235df2a799ff15f78a41d213a85b81da5fc81a` |
| `war-drums-130.wav` | 130 / 8 bars | [Horde War Drums loop](https://opengameart.org/content/horde-war-drums-loop), William Hector | SoX very-high-quality 44.1-to-48 kHz conversion; no level change | `bf528086ab7d2a51f83f3305f560af9f367bf98fed40dfdcc4dd32f898149b2d` |

The three 100 BPM files are progressive variations from one source pack. They
give a beginner simple material at a known tempo and give tests a precise BPM
reference. The 130 BPM percussion loop exercises a second known tempo.

Upstream verification hashes:

- `loops.zip`: `b155871e15cacdf8469b6030ab3b4b0b4d023bc8a115d77b099518047752c85c`
- `level1-step1.wav`: `f389769ead0220f07f38f4359922c62924303484d789cdd11645febc302d2e42`
- `level1-step2.wav`: `c709c5fe23ce43bf5b95db4ae10696b98c6aab309a3a9642090d63b2386e3a6f`
- `level1-step3.wav`: `bae4fd8e0ff2143a0153b9a7ad2865a5ba464c0ab37fd262f53903dfa8308e00`
- original Horde WAV: `a4d95514029dd928e5637c3b9edd659b8eaf14fa78d8afb2ab7ec4da064e4417`
