# Synchronized multitrack recording

SHR-DAW's raw recorder records a deliberately configured collection of JACK audio source
ports as one synchronized take. It is interface-neutral: an MR18, a two-input
sound card, a virtual JACK source, or another multichannel USB interface uses
the same recorder. It does not monitor, mix, process, overdub, or edit these
inputs. It remains distinct from the owned performance bus and its one-file
post-limiter [final stereo recording](FINAL_PERFORMANCE_BUS.md#final-recording).
Use the interface or mixer for safe low-latency raw-input monitoring.

## What one take guarantees

All armed tracks enter and leave capture on the same JACK callback boundary.
One callback copies every channel into one preallocated interleaved SPSC ring,
or rejects the whole callback. The callback performs no file I/O, allocation,
logging, sleep, mutex/condition wait, or collection growth. One ordinary
worker thread drains the ring and writes the stems.

A successful take has one JACK sample rate, one frame count, stable track
order, and one mono 24-bit PCM WAV per armed track. A mono file keeps physical
input identity obvious and gives every channel its own RIFF data limit rather
than sharing the short 4 GiB ceiling of one large interleaved file.

The configured global safety bound is 64 tracks. It is not an MR18 limit: it
bounds callback work and preallocated memory on this small appliance while
leaving ample room above the 18-channel acceptance target. `capture.ring_frames`
is a per-timeline capacity shared by every channel; at 18 channels and 262144
frames the sample storage is about 18 MiB. `capture.maximum_callback_frames`
must be at least the largest JACK period the machine will use.

## Storage planning

Raw stem storage grows independently of the callback-ring capacity. For
24-bit mono PCM without compression, the approximate payload rate is:

```text
channels × sample_rate × 3 bytes per second
```

At 48 kHz, WAV headers and the session manifest are negligible beside the
audio payload:

| Armed mono channels | Approximate payload per hour |
|---:|---:|
| 1 | 0.52 GB |
| 18 | 9.33 GB |
| 32 | 16.59 GB |
| 64 | 33.18 GB |

These are decimal drive-manufacturer gigabytes. A nominal empty 128 GB drive
therefore holds less than 7.8 hours at 32 channels, and the real recording
allowance is lower after the OS, builds, Projects, filesystem reserve, and a
deliberate free-space safety margin. Choose capacity from channel count and
retained hours rather than treating any drive size as universally excessive.

## Configuration and exact routing

New tracks use repeated lines in the private `shsynth.conf`:

```ini
capture.directory=~/.local/share/shsynth/recordings
capture.client=shs-recorder
capture.ring_frames=262144
capture.maximum_callback_frames=4096
capture.track=input-1|Input 1 · Vocal||mono|true|exact-client:exact-port
capture.track=input-2|Input 2 · Guitar||mono|true|exact-client:another-port
capture.track=line-l|Line L|line-pair|left|false|
capture.track=line-r|Line R|line-pair|right|false|
```

Fields are `stable ID|musician label|optional group|role|armed|preferred
source`. Roles are `mono`, `left`, or `right`. Left/right roles with the same
group describe a logical stereo pair in the manifest; the WAVs remain mono.
IDs are safe, unique names and remain stable when the visible label changes.

The preferred source is an exact JACK name owned by this machine. Blank means
unassigned. It never means port zero, silence, the first discovered port, or a
nearby name. Runtime discovery marks that exact preference `ready` or
`missing`; it never rewrites it. An armed missing track blocks start. Disarm it
or deliberately assign the correct discovered source. `REFRESH` resolves the
same remembered name again when the interface returns.

Old `capture.input=NAME|LEFT|RIGHT` configuration remains supported. When no
new `capture.track` lines exist, the first legacy pair becomes one armed linked
left/right pair in memory. Existing stereo recording configuration therefore
continues to produce two synchronized mono stems in one take directory.

## The 40×13 workflow

Open **AUDIO**. The list shows an arm dot, track number, label, and `ready` or
`missing`. Only the selected track has a compact level readout.

The body ends above the two controller rows. The final terminal row is the
shared status row; its first cell is steady white `■` while stopped or a red
`●` pulsing only between normal and bright red while a take is active. Useful
recorder state or faults may follow after one space.

- **RECORD:** `RECORD` starts/stops the synchronized take; `ARM` toggles the
  selected track.
- **TRACK:** select `PREV`/`NEXT`, cycle a deliberate `SOURCE` assignment, or
  edit the track `NAME`.
- **SETUP:** `ALL` arms every resolved track, `NONE` disarms everything, and
  `REFRESH` discovers JACK sources without rewriting remembered assignments.
- **SYS:** `PANIC` safely stops owned activity, `HELP` explains the screen, and
  `EXIT` returns Home.

Keyboard equivalents are `R`, arrows or `J`/`K`, Space, `A`, `X`, `S`, `N`,
and `F`. Track edits save to private runtime configuration. They are refused
during a take.

## Session directory and manifest

The writer first owns a unique `*.take.part` directory. Complete publication
uses a no-replace rename to `*.take`; a detected damaged take can only become
`*.incomplete.take`. A successful directory resembles:

```text
recording-…​.take/
  01-Input-1---Vocal.wav
  02-Input-2---Guitar.wav
  session.json
```

`session.json` is format version 1. It records the take name, rate, total
frames and duration; stable IDs and labels; grouping/role; preferred and
actual JACK source; WAV filenames and per-file frame/finalization state;
completeness; drop, overflow, callback and xrun counts; writer high-water
mark; and recovery notes. Unknown manifest versions are not rewritten.

Ring overflow, a dropped callback, JACK xrun/shutdown, oversized callback,
null JACK buffer, source disconnect, unequal final frames, storage error, WAV
limit, flush/fsync error, or publication failure prevents a complete result.
The UI reports the reason and location instead of presenting damaged stems as
successful.

On the next start, a recognized interrupted session is recovered without
following symlinks. Each valid partial mono WAV is truncated to the shortest
whole-frame count shared by all stems, finalized, and published only as
`recovered-incomplete`; its manifest says that review is required. The old
single stereo `.wav.part` recovery path is retained.

## Hardware-free 18-channel soak

This command does not open or start JACK and cannot make sound:

```sh
shr recorder-stress /explicit/temp/destination 60 18 48000 128
```

Arguments are destination, seconds, channel count, sample rate, and callback
frames. Defaults after the required destination are `10 18 48000 128`. It
paces deterministic distinguishable channel data in real time through the
production ring/writer, publishes a real take below only that destination,
then verifies completion, equal frame counts, and channel identity. Its report
includes elapsed time, write throughput, high-water frames, drops, overflows,
and the exact session path. It never deletes the destination or unrelated
files.

## Platform references

JACK explicitly forbids I/O, allocation, `printf`, mutex locking, sleeping,
waiting, polling, joining, and condition waits in the process callback; its
shutdown callback must be async-safe. See the official [JACK callback
contract](https://jackaudio.org/api/group__ClientCallbacks.html). Linux exposes
USB-audio stream capabilities and state in `/proc/asound/card*/stream*`, which
the kernel documentation calls useful for debugging; see [ALSA proc
files](https://www.kernel.org/doc/html/latest/sound/designs/procfile.html) and
the [`snd-usb-audio` driver parameters](https://www.kernel.org/doc/html/latest/sound/alsa-configuration.html#module-snd-usb-audio).
