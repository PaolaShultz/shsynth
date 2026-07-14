# Codex-assisted SHR-DAW setup

Help me install, recover, or customize SHR-DAW on this Raspberry Pi. First read
`AGENTS.md`, `docs/WORKSPACE_HANDOFF.md`, the README, and the existing user
configuration. Treat `install.sh` and `shr-setup` as the primary supported path;
this session is an optional diagnostic and customization layer around them.

Work interactively and explain any physical action I need to take. Diagnose the
OS, dependencies, terminal/display geometry, ALSA MIDI ports, JACK ports, audio
interface, controller, and current SHR-DAW configuration. Run the normal setup
and checks where safe. If a generic project defect prevents setup, patch the
project, add or update validation, and keep the normal installer usable for the
next person rather than making an undocumented local workaround.

When controller discovery is needed, listen to MIDI without forwarding it to a
synth and ask me to move or press exactly one control at a time. Identify and
verify the 12 continuous synth controls, main relative encoder, encoder press,
lock control, and available command pads. Detect relative-encoder direction and
value convention. Reject duplicate or conflicting assignments, preserve pickup
behavior, back up `controller.conf`, write the mapping, and show me a concise
summary before treating it as complete.

Help with complex JACK, ALSA, external-instrument, tracker-page, and SoundFont
configuration when requested. Keep machine-specific routes and hardware names
in configuration, not Rust constants. Put private downloads and user-specific
sound data outside the public repository, retain source and licence details,
and do not present unlicensed material as redistributable.

Preserve existing configuration, presets, ideas, songs, recordings, unrelated
processes, and repository changes. Never start or restart JACK, launch an
audible synth test, overwrite user data, publish, or make destructive/system-wide
changes without explaining the action and receiving my explicit permission.
Never kill a synth process SHR-DAW does not own. Back up files before changing
them and clearly separate project fixes from this machine's private settings.

Finish by running proportionate non-audible validation, including `shr doctor`
when JACK is already available, and report what was detected, changed, backed
up, verified, left untested, and what I should do next.
