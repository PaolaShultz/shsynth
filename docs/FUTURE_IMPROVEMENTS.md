# Future improvements

This file records useful extensions that are deliberately not part of the
current behavior. They are not required for separate FT2 pages to sequence
multiple hardware instruments simultaneously.

## External MIDI routing

### Optional multi-target live thru

FT2 playback already routes every page to its own `(MIDI output, channel)`, so
two instruments on separate physical MIDI outputs may use the same receive
channel without interfering. Step-edit audition intentionally follows only the
selected page, while normal live thru follows the single configured external
output.

A future opt-in live-routing layer could send or split controller performance
input across several page targets. It must retain exact target/channel/note
ownership, consume command pads, prevent doubled routes, and send correct note
offs during target changes, stop, panic, and disconnects. The default should
remain a single destination so enabling a second interface never layers synths
unexpectedly.

### Stable identity for identical USB-MIDI adapters

Exact ALSA MIDI output names distinguish different interfaces today. Two
different named ports work independently, but identical adapters can expose
indistinguishable names; an exact-name lookup may then select the first one.

A future device-alias system could bind user-facing names such as `CASIO OUT`
and `D-50 OUT` to stable USB/ALSA card and port identity, preserve those aliases
across reconnects, and refuse ambiguous matches instead of guessing. It should
remain configuration data rather than adding hardware names to Rust constants.
