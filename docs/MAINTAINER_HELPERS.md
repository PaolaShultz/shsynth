# Maintainer helper scripts

This document is the source-of-truth guide to the repository helpers in
`scripts/`: their arguments, environment variables, files changed, safety
boundaries, and the reasons they work the way they do. End-user commands remain
in the normal setup guides; this page explains the maintenance machinery.

All shell helpers use `set -euo pipefail`. An unhandled failure, unset variable,
or failed pipeline stops the operation instead of continuing with partial
assumptions.

## At a glance

| Helper | Intended use | Main side effects |
|---|---|---|
| `setup-local.sh` | Configure this checkout inside ignored private storage | Writes below `user/` by default; may run the interactive hardware wizard |
| `local.sh` | Run the checkout without using normal home-directory state | Writes runtime data below `user/` by default |
| `setup.sh` / installed `shr-setup` | Seed loops/demos and configure display/MIDI/JACK choices | Backs up and rewrites owned configuration; optionally downloads private loops, writes `~/.jackdrc`, and installs CPU tuning after confirmation |
| `install.sh` | Install dependencies and SHR-DAW on Debian/Raspberry Pi OS | May use `sudo apt-get`, rustup, `sudo make install-files`, and the setup wizard |
| `audio-performance.sh` / installed `shr-audio-tune` | Reversibly reserve one CPU for audio | Manages specific boot, systemd, governor, and JACK-affinity settings; requires reboot for isolation |
| `render-readme-screenshots.py` | Regenerate or validate real TUI documentation images | Writes tracked PNGs below `docs/images/` only |
| `generate_cleared_presets.sh` | Reproduce the authored public synthv1 bank | Creates named preset files only when they do not already exist |
| `generate_demo_songs.py` | Reproduce or validate cleared public-domain demos | `--write` replaces only tracked demo outputs; normal mode is read-only and rejects changes/extras |

None of the setup, tuning, preset, or screenshot helpers starts JACK, a synth
engine, MIDI playback, or an audible test. `local.sh` is the exception only in
the ordinary sense that it launches the application the user explicitly asked
to run; what the application subsequently starts depends on that user action.

## Repository-local setup: `setup-local.sh`

### Invocation

```sh
./scripts/setup-local.sh
SHSYNTH_USER_DIR=/absolute/private/path ./scripts/setup-local.sh
```

Environment:

- `SHSYNTH_USER_DIR` selects the private root. It defaults to the repository's
  ignored `user/` directory.
- `SHSYNTH_BIN` may select an already-built `shr` executable. It defaults to
  `target/release/shr` in this checkout.

The wrapper exports:

- `XDG_STATE_HOME=$SHSYNTH_USER_DIR/state`;
- `XDG_DATA_HOME=$SHSYNTH_USER_DIR/data`;
- `SHSYNTH_PRESET_DIR=$SHSYNTH_USER_DIR/presets/synthv1`;
- `SHSYNTH_LOOP_INBOX=$SHSYNTH_USER_DIR/data/shsynth/loop-inbox`.

It requires an executable SHR-DAW binary, creates the private preset directory,
copies only missing public presets into it, and then replaces itself with
`setup.sh --state-dir "$XDG_STATE_HOME/shsynth"`. The shared wizard seeds only
the missing WAV names in `loops/cleared-loops.txt` and missing cleared demo
Projects. Matching demo MIDI/manifest files live in the private XDG demo tree.

### Why it exists

The regular setup command belongs to an installed application and therefore
uses normal XDG user directories. A checkout needs a hard, visible boundary
between public repository files and local Projects, Ideas, recordings,
downloads, routes, and uncleared sounds. This thin wrapper establishes that
boundary while reusing the exact same setup wizard. It never overwrites a
same-named private preset because a private edited sound takes precedence over
the public seed copy.

It deliberately refuses to compile automatically: configuration should use the
binary that was explicitly built and tested, not silently change code or wait
through an unexpected build.

## Repository-local launcher: `local.sh`

### Invocation

```sh
./scripts/local.sh
./scripts/local.sh doctor
./scripts/local.sh screenshots
SHSYNTH_USER_DIR=/absolute/private/path ./scripts/local.sh
```

All arguments are passed unchanged to `shr`. The environment and private-preset
copy rules match `setup-local.sh`. The launcher prefers
`target/release/shr`; if it is absent, it uses an installed `shr` from `PATH`.
It refuses to run until the local `shsynth.conf` exists.

### Why it uses `exec`

`exec` makes SHR-DAW replace the wrapper process. Signals, exit status, terminal
ownership, and clean shutdown therefore reach the application directly instead
of passing through a redundant shell parent. This matters for All Notes Off and
owned engine shutdown.

The launcher does not recopy or reset the whole private tree. It validates the
demo corpus and creates only required directories and missing public preset,
loop, and demo seeds, preserving all user work.

## Hardware setup wizard: `setup.sh` / `shr-setup`

### Invocation and inputs

```sh
./scripts/setup.sh
./scripts/setup.sh --state-dir /absolute/state/shsynth
shr-setup
```

Options:

- `--state-dir DIR` overrides the runtime/controller configuration directory.
- `-h`, `--help` prints usage.

Environment:

- `XDG_STATE_HOME` changes the normal state root.
- `XDG_DATA_HOME` changes the recording/data root written into configuration.
- `SHSYNTH_BIN` selects the binary used for config initialization and controller
  profile commands.
- `SHSYNTH_PRESET_DIR`, when present, becomes the configured synthv1 preset
  directory.
- `SHSYNTH_LOOP_INBOX`, when present, becomes the configured and seeded loop
  import inbox.

The source-tree form reads templates from `config/`, MIDI-device profiles from
`midi-devices/`, allowlisted starter WAVs from `loops/`, and the cleared demo
manifest/files from `demos/`. The installed form resolves all four beneath
`share/shsynth/`. If configuration is missing in the
normal state directory it uses `shr config init`; for an explicit state
directory it copies only missing template files.

Setup always creates or preserves configuration, selects the active XDG/private
loop inbox for new configuration, copies missing allowlisted starter loops,
copies missing demo Projects to `songs/`, and mirrors the cleared demo corpus
under `demos/`. The manifest itself may be refreshed; user Projects are never
replaced.
If standard input is not a terminal it then stops; it never guesses display,
download, or hardware choices in automation.

### Interactive sequence

Before changing configuration, the wizard creates unique timestamped backups of
both `shsynth.conf` and `controller.conf`. It then:

1. asks whether note names use English `B` or German `H`/`B` spelling;
2. optionally selects an ALSA interface and writes a backed-up `~/.jackdrc` for
   the user's next manual JACK restart;
3. selects the controller input and writes the same exact match to runtime and
   controller configuration, or explicitly keeps keyboard-only operation;
4. runs non-audible `shr pads auto`, optionally followed by `shr pads learn` if
   no reviewed profile matches;
5. discovers physical JACK playback ports, writes the same preferred stereo
   pair for synth and loop playback, then optionally records a named internal
   fallback and a distinct final analogue-headphone fallback;
6. optionally downloads four MusicRadar 80s drum beats, converts them to the
   chosen WAV rate with SoX, and records their source/redistribution terms;
7. optionally configures a distinct stereo capture pair and label;
8. optionally configures an external MIDI destination and data-driven device
   profile;
9. on systems with at least four CPUs, optionally invokes `shr-audio-tune` and
   records the selected engine CPU.

### Design decisions

- Hardware/client names are written to configuration, never Rust constants.
- ALSA and JACK discovery is advisory. Manual exact values remain possible so
  setup can be completed while hardware or JACK is offline.
- System, Midi Through, and SHR-owned MIDI ports are filtered from controller
  candidates to avoid feedback and self-connection.
- JACK choices require distinct left/right ports.
- Configuration keys are replaced through a temporary same-directory file and
  `mv`, preserving file permissions when possible. This avoids leaving a
  half-written configuration.
- Values containing newline or carriage-return characters are rejected, and
  capture labels also reject the field separator `|`.
- The wizard may write `~/.jackdrc` only after an explicit opt-in and backup.
  It never starts or restarts JACK because doing that during a live session can
  interrupt or produce audible output.
- Controller learning is non-audible: learned MIDI is not forwarded to a synth.
- Existing configuration is backed up rather than silently discarded.
- Hardware discovery never overwrites a remembered route merely because that
  hardware is absent. The user must explicitly choose a changed/disabled route.
- Public and downloaded-private loop seeds never replace a same-named inbox
  file. Public packaging is constrained by `loops/cleared-loops.txt`.
- Cleared demo Projects never replace same-named user songs. Demo source
  packaging is constrained by `cleared-demos.json` and deterministic validation.
- The optional 78 MB archive is fetched directly from MusicRadar into a
  temporary directory and deleted after extracting four tempo-labelled beats.
  Those raw WAVs remain private because MusicRadar forbids redistribution.

## Installer: `install.sh`

### Invocation

```sh
./scripts/install.sh
./scripts/install.sh --no-deps
./scripts/install.sh --no-config
./scripts/install.sh --no-deps --no-config
```

Options:

- `--no-deps` skips `apt-get update` and dependency installation.
- `--no-config` skips the final interactive `shr-setup` run.
- `-h`, `--help` prints usage.

With dependencies enabled, it requires a Debian-style `apt-get` system and uses
`sudo` to install the build toolchain, ALSA/JACK utilities and headers, SoX and
unzip for optional loop installation, Python 3 for demo validation/seeding, the
three supported software instruments,
and their packaged data. It requires
Rust 1.85 or newer; when necessary it installs the official minimal rustup
toolchain for the current user and runs Cargo as `cargo +1.85.0`.

It then runs locked tests, creates a locked release build, installs the files
with `sudo make install-files`, and normally opens `shr-setup`.

### Why install is explicit and relatively heavy

SHR-DAW is a live-audio program. Installing an untested binary or silently using
an old distro compiler is a worse failure mode than spending time on a locked
test/build. Dependencies are installed rather than quietly skipping parts of
the application. `--no-deps` and `--no-config` exist for maintainers and package
builders who have already satisfied those responsibilities.

The installer does not start JACK or any synth. Package installation may enable
distribution services according to the OS packages, but SHR-DAW itself does
not assume that the audio interface is connected or safe to restart.

## Audio CPU tuning: `audio-performance.sh` / `shr-audio-tune`

### Invocation

```sh
sudo shr-audio-tune install
sudo shr-audio-tune install 3
shr-audio-tune status
sudo shr-audio-tune remove
```

Commands:

- `install [CPU]` reserves the zero-based CPU; the default is the highest
  online CPU.
- `status` reports the managed CPU, whether reboot-time isolation is active,
  current readable governors, and JACK's affinity drop-in.
- `remove` reverses only the settings installed by this helper and keeps the
  original boot-command-line backup.
- `runtime-start` and `runtime-stop` are internal systemd-service entry points,
  not normal maintainer commands.

Environment:

- `SHR_TUNE_ROOT=/fixture/root` prefixes all managed absolute paths and disables
  real `systemctl` calls. It exists for isolated tests and inspection; the
  fixture still needs representative `/sys`, `/proc`, `/boot`, and `/etc`
  paths.

### Managed state

Installation requires at least four online CPUs and refuses non-contiguous or
unusual online-CPU layouts instead of inventing a mask. It records ownership
beneath `/var/lib/shr-audio-tune/`, backs up the Raspberry Pi boot command line,
and manages only:

- `isolcpus=domain,managed_irq,<CPU>`;
- `nohz_full=<CPU>`;
- `rcu_nocbs=<CPU>`;
- `irqaffinity=<housekeeping CPUs>`;
- `/etc/systemd/system/jack.service.d/90-shr-audio-cpu.conf`;
- `/etc/systemd/system/shr-audio-performance.service`;
- `/usr/local/libexec/shr-audio-tune-runtime`.

The runtime service records each existing CPU governor before selecting
`performance` where supported, then restores the recorded values when stopped.
The JACK drop-in applies the audio CPU affinity, real-time priority limit, and
unlimited memory lock on JACK's next start.

### Safety rationale

- Pre-existing kernel keys or managed-path collisions are refused unless this
  helper already owns the installation.
- A different already-installed CPU must be removed before changing CPUs.
- `remove` deletes only exact tokens and files owned by this helper; it does not
  restore an entire possibly-stale command line over later administrator work.
- The untouched original command line remains as a recovery artifact.
- Installation and removal never start or restart JACK. Kernel isolation waits
  for an explicit reboot, and the JACK drop-in waits for the user's next safe
  service start.
- `audio.engine_cpu` belongs to `shsynth.conf`; removal tells the user to clear
  it rather than modifying an unknown runtime configuration path as root.

## TUI screenshot renderer: `render-readme-screenshots.py`

### Invocation

```sh
# Render every README and menu-manual image.
python3 scripts/render-readme-screenshots.py

# Render one exact manifest name for visual inspection.
python3 scripts/render-readme-screenshots.py \
  --only menu/ft2-step-edit-add.png

# Validate presence, dimensions, and integer scaling without rewriting images.
python3 scripts/render-readme-screenshots.py --check
```

Options:

- no option renders every frame returned by the Rust manifest;
- `--only NAME` renders only an exact output name from that manifest;
- `--check` requires every manifest image, verifies 960×640 dimensions, and
  checks every 2×2 output block for identical pixels.

Environment:

- `CARGO` overrides the Cargo executable used for `cargo run --locked`.
- `SHR_SCREENSHOT_COMMAND` replaces the complete manifest-producing command;
  it is parsed with shell-style quoting but run directly, not through a shell.

The default command uses the installed Rust 1.85 toolchain when present and
runs `shr screenshots`. Rust renders the real application `draw` function into
40×20 ratatui test buffers seeded by the deterministic `ScreenshotScenario`
fixtures in `src/ui.rs`. JSON supplies each cell's symbol, foreground,
background, and bold state. No JACK server, engine, MIDI port, or private user
file is involved.

### Image parameters

- terminal geometry: 40 columns × 20 rows;
- cell geometry: 12×16 pixels;
- native raster: 480×320 pixels;
- final scale: exactly 2;
- final PNG: 960×640 pixels;
- primary font: `/usr/share/consolefonts/Lat15-VGA16.psf.gz`;
- fallback font: `target/Lat15-VGA16.psf`;
- output roots: `docs/images/shr-daw-*.png` and `docs/images/menu/*.png`.

The PSF glyph is eight bits wide. Each 12-pixel cell column samples it with
`source_x = out_x * 8 // 12`, matching the established wide terminal look.
Ratatui's ANSI colors and bold modifier are converted through a fixed palette.
Unsupported Unicode falls back to the font's question-mark glyph instead of a
host-dependent replacement.

### Why the renderer is intentionally slow

The final enlargement uses explicit nested loops that copy each native pixel
into an exact 2×2 square. A library resize could be faster, but the explicit
operation makes the contract obvious in code and cannot silently acquire
interpolation, antialiasing, color blending, or a version-dependent sampling
rule. This preserves the pixel font and makes mobile/browser display crisp
without pretending the application has more than 40×20 cells.

`--check` is also deliberately exhaustive. It opens every expected image and
checks every 2×2 block instead of trusting file metadata or the name of a resize
filter. On the Raspberry Pi, rendering or validating all 80 menu frames takes
noticeable time. That time is an accepted documentation-integrity cost, not an
optimization bug. Do not replace the scaler or weaken the check merely to make
the command faster. First render one representative image and inspect it; then
run the complete batch.

Pillow is used as a bitmap container and PNG writer, not for font rendering.
Using a TTF, browser screenshot, GUI terminal, or Pillow text API would make
glyph metrics dependent on a desktop font and could introduce smoothing.

## Cleared-preset generator: `generate_cleared_presets.sh`

### Invocation

```sh
./scripts/generate_cleared_presets.sh
```

There are no command-line options or environment controls. The script uses
`presets/synthv1/Velvet Tines.synthv1` as the complete current-schema template.
Its internal helper has the conceptual form:

```text
make_preset NAME PARAMETER=VALUE ...
```

For each authored sound, it copies the template, changes the XML preset name,
and replaces selected `<param>` values by exact parameter name with Perl. Values
not listed in the recipe remain inherited from the known template.

### Why it refuses to overwrite

Every destination is checked before copying, and the whole run stops if it
already exists. This prevents a reproducibility tool from overwriting a later
hand-edited or reviewed public sound. Consequently, run it in a clean temporary
checkout or against an intentionally absent generated bank when auditing
reproducibility; do not run it casually over the normal populated checkout.

The generator is an authorship recipe, not a licence grant. Adding or changing
a public preset still requires schema/XML validation, listening review when
authorized, an entry in `cleared-presets.txt`, and provenance in
`THIRD_PARTY.md` as described by `docs/NEW_PATCHES.md`.

## Cleared demo generator: `generate_demo_songs.py`

### Invocation

```sh
./scripts/generate_demo_songs.py
./scripts/generate_demo_songs.py --files
./scripts/generate_demo_songs.py --write
```

Normal mode regenerates all expected bytes in memory and validates the exact
`demos/` directory. It fails for a missing, changed, or extra regular file.
`--files` performs the same validation and then prints the manifest-cleared
repository paths used by `make install-files`. `--write` is the only mutating
mode: it creates `demos/` if needed and replaces the 10 MIDI files, 10 current
`.shsong` Projects, and `cleared-demos.json` with deterministic output. It does
not touch user/XDG song data.

The script uses only Python's standard library. Each format-1 MIDI contains a
conductor track and five named musical parts; each Project contains the same
parts with canonical format-4 `default` routes. The JSON manifest owns title,
tempo, meter, key, parts, description, style ideas, original-arrangement
licence, public-domain reasoning, institutional source URLs, filenames, and
SHA-256 hashes. `src/demo.rs` validates that manifest, MIDI chunk structure,
native Project loading/routing, metadata, and exact directory membership.

The generator's melody/harmony/event data are the original SHR-DAW
arrangements. Do not replace them with downloaded MIDI or a transcription of a
modern recording. Any new title needs its own public-domain analysis and source
record before `--write`; changing the source requires rerunning validation and
reviewing the regenerated hashes. No JACK client or MIDI output is opened.

## Related Make targets

The Makefile is not a script, but the installer delegates its final file layout
to it:

```sh
make build
make test
make check-demos
sudo make install
sudo make install-files
sudo make uninstall
```

Variables:

- `CARGO` selects Cargo;
- `PREFIX` defaults to `/usr/local`;
- `DESTDIR` prefixes the install tree for packaging or a non-root fixture.

`install-files` first runs `check-demos`, then installs only presets and demos
named by their cleared manifests, the
configuration and device/profile data, drum patterns, documentation, nested
menu chapters, and nested menu images. The public `shr` binary receives the
compatibility aliases `shs` and `synth-player`; no separate process binary is
installed for those names.

Use `DESTDIR` to inspect installation without touching the host:

```sh
fixture=$(mktemp -d)
make install-files DESTDIR="$fixture" PREFIX=/usr/local
find "$fixture/usr/local" -type f -o -type l
```

Choose a dedicated temporary directory and remove it only after confirming the
expanded path. `uninstall` is intentionally broad within the exact selected
`PREFIX`/`DESTDIR` application paths; never point those variables at an
unresolved or unintended root.

## Validation after helper changes

Match validation to the helper's effects:

- Shell helper: run `shellcheck` on each changed shell file.
- Python renderer: run `python3 -m py_compile`, inspect one image, render the
  full batch, and run `--check`.
- Documentation: check local links/image references and run `git diff --check`.
- Preset generator or output: validate every affected `.synthv1` with
  `xmllint`, confirm parameter names, manifest membership, and provenance.
- Demo generator or output: compile the Python helper, run its normal check,
  run the Rust structural test, inspect manifest provenance, and verify the
  staged package contains only `--files` output.
- Installer, setup, runtime, Makefile, Rust fixture, Cargo, or application
  behavior: run the complete pinned Rust format, test, warning-denied Clippy,
  and locked release-build suite required by `AGENTS.md`.
- Install layout: use a validated explicit `DESTDIR` fixture and confirm the
  nested manual chapters/images and cleared-only preset bank.

Before any commit, ensure `git ls-files | rg '^user/'` produces no output. Never
stage private configuration, Ideas, Projects, recordings, downloads, or the
private Codex skill below `user/`.
