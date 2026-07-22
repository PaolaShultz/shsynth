# Release roadmap

This roadmap owns the current development order. Releases advance when their
acceptance gates pass, not on dates. Detailed behavior remains in the focused
product, installation, FT2, and hardware documents linked below.

## Version numbering

The package uses `major.minor.patch` numbering. Version `0.3.92` corrected the
unintended `0.392.0`; the current checked-progress version is `0.3.94`.

- Meaningful checked progress before the next milestone increments the patch:
  `0.3.95`, `0.3.96`, and so on.
- The completed 0.4 milestone becomes package version `0.4.0`. Later fixes or
  checked progress toward 0.5 use `0.4.1`, `0.4.2`, and so on.
- The completed 0.5 milestone becomes `0.5.0`; later checked progress toward
  0.6 uses `0.5.1`, `0.5.2`, and so on.
- The completed 0.6 milestone becomes `0.6.0`.

Do not assign a milestone version merely because its code exists. Its complete
gate, documentation, and required physical evidence must pass first.

## Scope rule

Work on the current milestone before pulling from later or speculative plans.
An observed defect or missing recovery path that blocks the current gate is in
scope. A new feature, redesign, optimization, or interesting experiment is not
in scope unless the owner explicitly moves it into the milestone.

The owner supplies product intent that has not yet been written down. Do not
infer missing FT2 requirements from nearby ideas or implement random entries
from [Future improvements](FUTURE_IMPROVEMENTS.md). Record an intended action,
result, state boundary, and acceptance path before implementing it.

## 0.4.0 — trust the existing product

Outcome: every existing menu entry is in the intended place and every current
workflow works as intended on the supported compact UI and a clean normal
Raspberry Pi OS Lite installation.

The release gate requires:

- every reachable menu entry and controller-menu item checked for its intended
  screen, page, order, label, and return location, using keyboard/controller
  parity where both are supported;
- every currently documented workflow checked through normal completion,
  cancellation or Back, repeated use, failure and retry, interruption or mode
  change, and preservation of existing Projects, configuration, selection, and
  other state that the action should not change;
- the complete installer and setup flow checked from a fresh 64-bit Raspberry
  Pi OS Lite image rather than treating Patchbox OS as the target platform;
- the exact Pi 4 development-system state captured as the comparison baseline,
  including OS/kernel, packages, services, boot/audio tuning, JACK, toolchain,
  storage, power/cooling, and relevant hardware configuration;
- the exact Pi 5 image and starting state recorded, with every dependency,
  prompt, restart, configuration decision, failure, retry, and successful
  return to the install/setup path;
- existing managed system optimizations evaluated on the new system and
  applied through their owner only when compatible and useful, never copied
  blindly from Pi 4 boot or service files; and
- focused documentation corrected to match the accepted behavior and platform.

Keep raw state captures, logs, routes, host/network identifiers, serials, and
runtime configuration below ignored `user/`. Promote only cleared, relevant
platform and measurement facts into public documentation.

The platform procedure and comparison fields live in the
[Pi 5 plan](PI5_HEADROOM_PLAN.md). Passing installation does not itself prove
audio hardware, musical quality, or 18×18 full duplex.

## 0.5.0 — complete the intended FT2 workflow

Outcome: FT2 has the complete functionality the owner intends, without the
current short-wired or partial flows and without unrelated future ideas
obstructing that work.

The exact functionality inventory is still owner input. Until it is stated and
captured, this roadmap deliberately does not invent it. For each supplied item:

1. record the intended musician action and visible/audible result;
2. identify how the current partial path differs;
3. preserve the required cursor, lane, column, page, route, mode, and Project
   state, making genuinely exclusive modes replace one another;
4. provide nearby cancellation, failure, and retry behavior without losing
   work; and
5. verify normal, repeated, interrupted, saved/reloaded, and existing-state
   paths that apply.

The release gate is the completed owner-approved FT2 inventory, its focused
tests and hands-on checks, and matching current documentation. Planned rhythm,
mixer, analysis, or other ideas are not 0.5 blockers unless the owner adds them
to that inventory.

## 0.6.0 — 18×18 full-duplex multichannel audio

Outcome: SHR can play 18 independent output channels while synchronously
recording 18 input channels through one multichannel interface, and that path
is physically proven rather than inferred from synthetic tests.

The release gate requires:

- the 18-output path implemented with exact configured JACK destinations and
  bounded live-audio behavior;
- the existing 18-input recorder integrated without weakening synchronized
  publication, recovery, or source-identity guarantees;
- hardware-independent playback identity, capture identity, failure, retry,
  and combined-load checks completed before borrowing the mixer;
- progressive capture-only, playback-only, and simultaneous 2×2 through 18×18
  checks on the Raspberry Pi 5 and MR18;
- exact bidirectional channel identity, zero required xrun/drop/overflow/fault
  counters, safe disconnect/reconnect and teardown, and a sustained 18×18 soak;
  and
- the borrowed mixer scene and physical safety state restored after testing.

The authoritative physical procedure and result sheets are in the
[MR18 acceptance plan](MR18_TEST_PLAN.md). The release is not marked checked
until that physical gate passes.
