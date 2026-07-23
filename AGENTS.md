# Repository instructions

Read `docs/WORKSPACE_HANDOFF.md` completely at the start of work in this
checkout. It records current machine state, private/public boundaries, open
work, and decisions that must survive a new thread. Before changing or
questioning a repository helper or related Make target, also read
`docs/MAINTAINER_HELPERS.md`; that file alone owns detailed helper arguments,
side effects, safety boundaries, and rationale.

## Priority and scope

Follow the user's priorities, make the requested path work quickly, and fix
observed blockers. The user decides how to inspect results and owns music,
demo, and video work. Do not turn a focused task into a broad testing,
documentation, cleanup, or handoff campaign unless the user requests it or an
observed failure requires diagnosis.

### Default execution contract

Unless the user explicitly broadens a request, treat the concrete outcome as
the exact scope boundary. Complete every ordinary step required to achieve and
proportionately verify that outcome without making the user repeatedly
authorize routine in-scope work. Do not add adjacent features, redesigns,
refactors, cleanup, publishing, or process work merely because they appear
useful. Required safety fixes and focused documentation for behavior changed by
the request remain in scope; unrelated improvements do not.

Check required named apps, connectors, hardware, credentials, and specialist
tools at the start of the path that depends on them. If a required capability
is absent or inaccessible, tell the user immediately, name the exact blocked
action, and give the shortest recovery step such as exiting and resuming to
restore a connector. Do not silently substitute a browser, shell workflow,
manual instructions, or a different tool for the requested mechanism. A
workaround requires the user's choice when it materially changes their work.

Keep the `openaiDeveloperDocs` MCP server on demand rather than configured
globally. It has previously caused MCP loading problems during normal Codex
startup. Do not persistently reinstall it merely because a skill prefers that
source. When a task genuinely requires it, explain the temporary addition,
add it for that task, account for any required Codex restart or resume, and
remove it with `codex mcp remove openaiDeveloperDocs` when the lookup is done.
Use official OpenAI web sources when they are sufficient and permitted by the
task instead of imposing this MCP on every session.

This checkout is shared by multiple workers who edit and commit independently.
Inspect live Git state before editing and again before committing; preserve
other workers' changes and stage/commit only your own scoped paths or hunks. Do
not wait for unrelated workers to finish. Never record a branch tip, last
commit, or clean/dirty snapshot in the handoff because it becomes stale during
parallel work.

### Completion and publication

Classify repository-changing work by its intended outcome, not by whether the
diff adds new lines:

- **Repair work** restores or completes existing intended behavior: bug fixes,
  contract corrections, regressions, broken workflows, safety fixes, and
  implementation required to make an already-promised path work. A completed
  repair task ends with a scoped commit and push to the current shared branch
  unless the user explicitly says not to commit or push, the task is review-only,
  or credentials/network/server policy prevents publication. Commit and push are
  ordinary completion steps for repair work and do not require a separate user
  prompt.
- **New work** introduces a capability, feature, experiment, prototype, or
  product direction that was not already part of the intended behavior. Do not
  commit or push new work unless the user explicitly asks. Keep it available in
  the working tree for review.

When one request mixes repair and new work, commit and push only independently
stageable repair paths or hunks; leave the new work uncommitted unless the user
authorizes it. If the changes cannot be separated safely, do not publish the
mixed commit without the user's direction. Documentation and tests required to
describe or verify a repair belong with that repair. A review or audit that
makes no repository changes has nothing to commit.

Use `main` for ordinary development. The Build Week submission is preserved by
its tag; do not keep or recreate a standing `dev` branch before the repository
owner opens the planned 0.6 milestone. A short-lived branch or worktree for an
explicitly requested isolated experiment is not a standing integration branch.

Do not alter private user data unless explicitly requested. Physical equipment
is borrowed; do not start JACK, a synth, MIDI transmission/playback, recording,
or any other audible or hardware-changing test without explicit permission.

## Product and development

SHR-DAW is a responsive 40×13 Raspberry Pi mini DAW with a Rust TUI, FT2-style
tracker, synth hosts, MIDI routing, loops, recording, JACK/ALSA integration,
and a small controller. Keep live-audio paths bounded and responsive.

Treat the 40×13 display as content-first and do not change its established TTY
font. Home is the only screen without the shared working-screen layout. Every
other screen reserves row 13, the final terminal row, for one shared status
renderer. The two controller rows sit immediately above it. Screen bodies,
screen-specific footers, overlays, and later cleanup passes must not draw or
clear the final row. Remove or fold redundant gray status-like lines in screen
bodies instead of stacking local commentary above the shared status row. Omit
healthy or obvious labels such as `AVAILABLE`, `ONLINE`, `CONNECTED`, and
`IDLE`; assume the musician understands the current screen and controls. Keep
navigation actions literal: changing a page or order must preserve the selected
lane/column/cursor unless the requested behavior explicitly says otherwise.

The status row begins with exactly one transport cell. Use `>` in steady green
for play, `■` in steady white for stop, `‖` in steady white for pause, and
`●` in red for record. Record is the only transport state that pulses: alternate
normal and bright red without ever hiding or replacing the circle. After one
space, show only current, useful state or fault text; do not invent per-screen
gray messages merely to fill the row.

All horizontal LED meters use the existing one-cell `●` glyph, never square bar
glyphs. Unlit LEDs are dark gray. Lit safe LEDs use one consistent green;
yellow and red appear only at their documented active thresholds. A held peak
may use a brighter version of the same threshold colour, but not a different
shape. The startup splash uses three identical circular-LED rows per channel.

Master overlays preserve a one-cell reveal on the left, right, top, and bottom.
Their launcher stays inside the overlay border; the bottom reveal remains the
shared status row. Overlays own transient selection only and do not move,
replace, or clear that row.

Use the installed Rust 1.85 toolchain because the system Cargo may be too old:

```sh
PATH=/home/patch/.rustup/toolchains/1.85.0-aarch64-unknown-linux-gnu/bin:$PATH cargo check --locked
```

Temporary combined-pass rule: do not run builds or any command that compiles
the project until the user explicitly asks for the combined build-and-test
pass. This includes `cargo build`, `cargo check`, `cargo test`, Clippy, and
other build-producing validation. Add requested changes serially and limit
intermediate validation to formatting and source-level inspection.

During the current incremental-debug phase, use formatting and source
inspection, then—only after that explicit build authorization—use
`cargo check --locked` and focused tests for changed behavior. Run
`cargo build --locked` only when a binary is needed for user testing. Do
not run the complete test suite, warning-denied Clippy, optimized/release
builds, or release stress validation unless the user explicitly requests them.
A commit, handoff, or general validation request does not override this rule.
For documentation or image-only work, use only relevant link/reference,
format/dimension, helper syntax, and `git diff --check` checks. Install a tool
required for the requested validation rather than silently weakening it.

Plain `shr` must launch this checkout's `target/debug/shr`; debug and release
builds must remain visibly identified as `DEV` and `REL` in the TUI.

## Safety and ownership invariants

- Never layer managed synth engines, terminate a synthv1 process SHR-DAW does
  not own, alter unrelated processes/routes, or omit clean shutdown and All
  Notes Off.
- Put hardware names, client/executable names, preset paths, and audio/MIDI
  routes in `shsynth.conf` or `controller.conf`, never Rust constants.
- Block mapped CCs before synthv1 until pickup reaches or crosses the loaded
  value. Loading or resetting parameters must re-arm pickup.
- On Playback, main-encoder press resets only the 12 mapped parameters without
  restarting the engine; PLAY and keyboard `P` control MIDI-take playback.
- Keep synthv1 0.9.29 indices/ranges in `control.rs`; parse preset XML by name.
- Parameter indicators are relative to the original preset: green below
  −0.03, bright yellow within ±0.03, and red above +0.03.
- Consume command-pad note-on and note-off; pass musical MIDI through.

## Public, private, and publishing boundaries

Every tracked file is public. Keep all private runtime state, configuration,
logs, ideas, Projects, recordings, downloads, routes, and uncleared presets
below ignored `user/` (or an explicit `SHSYNTH_USER_DIR`). Use
`scripts/setup-local.sh` and `scripts/local.sh` for repository-local operation.
Never stage or publish a path below `user/`.

Publish repair work by default and new work only when asked, as defined in the
completion contract above. Use the existing GitHub CLI login and
repository-local Git identity; never invent an identity or expose credentials.
Before committing or pushing, inspect `git status --short`, confirm no `user/`
path is staged, and run `git diff --cached --check`.

Only presets listed in `presets/synthv1/cleared-presets.txt` and documented in
`THIRD_PARTY.md` may be public. Follow `docs/NEW_PATCHES.md`, validate current
schema/XML names, and retain source/licence evidence. The uncleared private
preset archive must not be committed, packaged, mirrored, or relabelled.
Tracked loops and demos are likewise limited to their cleared manifests.

## Documentation and collaboration

Keep `README.md` as a short landing page, never the complete manual. Keep it at
or below 900 words and 160 lines, with only a short product description,
compact feature summary, essential install/local-launch commands, concise
screenshot tour, canonical documentation links, and licence/third-party links.
Do not add detailed behavior, workflows, architecture, storage, failure models,
controller tables, history, plans, personal narrative, or repeated safety
contracts. Put current detail in the focused document that owns it and link
there in one sentence; remove README duplication instead of expanding it.

Update focused documentation when behavior, commands, mappings, storage, or
hardware assumptions change. Remove stale conflicting text. Use
`docs/README.md` to find the focused architecture, musician, measurement, and
future-plan documents.

Explain musical and hardware choices in plain language, recommend a safe
default, and connect parameters to what the user will hear or do. For physical
setup, give one concrete user action at a time and separate it from machine
inspection. Research unfamiliar or current details from authoritative sources
and preserve provenance when it affects configuration or redistribution.

For requested visual review over SSH, place temporary output in one exact
ignored subdirectory below `user/`, serve only that subdirectory over a
temporary LAN HTTP server, give the development-PC URL, and stop the server
after review. Never serve the repository root, all of `user/`, configuration,
credentials, or other private data.
