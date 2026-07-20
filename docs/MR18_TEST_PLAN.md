# Midas M AIR MR18 acceptance plan

This is the concrete checklist for the first full hardware test. The planned
unit is a Midas M AIR MR18; the recorder itself remains generic JACK capture.
Do not claim MR18 success until every applicable row below passes on the
attached unit.

## Verified facts and unit-specific checks

Verified from the current official [MR18 product
page](https://www.midasconsoles.com/en/products/0605-aaf), [MR18/MR12 user
manual](https://cdn-media.empowertribe.com/f4e89140f9994eed93e4f2a4e086c578/M_MI_0605-AAF_MR18-MR12_EN.pdf),
and [current quick-start
guide](https://cdn-media.empowertribe.com/3a0cd685a2a4450993e03a09ddb34dc7/QSG_MI_0605-AAF_MR18_WW_2025-06-24_Rev.0.pdf):

- The unit has 16 combo mic/line inputs with Midas PRO preamps plus balanced
  line inputs 17–18.
- Its computer connector is USB 2.0 Type-B. The interface is bidirectional
  18×18 audio and 16×16 MIDI; up to 18 audio channels can be recorded at once.
- The specification lists Linux as supported, 44.1 and 48 kHz operation, and
  24-bit A/D-D/A conversion at those rates. The current manual tells Windows
  users to obtain a driver; the official Downloads tab offers no Linux audio
  driver. This supports ordinary Linux USB-audio operation, but the current
  manual does not use the literal phrase “class compliant.” Tomorrow, confirm
  the actual unit binds to Linux `snd-usb-audio` without a vendor driver before
  recording “class-compliant Linux” as the observed result.
- The mixer can use 18×18 or 2×2 USB mode. Input/USB routing is freely
  assignable, USB sends/inputs are selectable, and the block diagram shows
  input tap choices including pre-HP, pre/post gate, pre/post EQ, pre/post
  fader, and post-pan. Choose and record the intended dry or processed tap;
  do not assume the current scene is 1:1 or dry.
- The manual says to mute Main LR before changing between 44.1 and 48 kHz
  because pops can occur.
- As checked on 2026-07-19, the official Downloads tab lists MR18 firmware
  `1.25.0` for both Hardware V1 and Hardware V2 (release filenames dated
  2025-11-18), older `1.22.0`, M-AIR Edit Linux `1.8.1`, Raspberry Pi 64-bit
  `1.8.2`, and the current manuals. Record hardware revision and installed
  firmware first. Do not update merely because a newer package exists.

Still assumptions until observed on this exact mixer/Pi/cable:

- the USB descriptors, vendor/product IDs, ALSA card ID and device/subdevice;
- whether `snd-usb-audio` binds cleanly and exposes 18 capture channels;
- the exact ALSA and JACK client/port names and their order;
- installed hardware revision, firmware, scene, clock, USB mode and send taps;
- stable operation at the chosen period count/buffer, storage target, and all
  18 channels on this Raspberry Pi.

## Safe step-by-step procedure

One person should call each change; another should watch outputs, JACK, storage,
and the result sheet.

1. Read the front/rear label and control app identification. Confirm the unit
   is **Midas M AIR MR18**, not MR12, XR18, X18, or another mixer.
2. With power disconnected, inspect the case, IEC inlet/cable, USB Type-B
   socket, network socket, connectors, vents, liquid/impact damage, and any
   unsafe cabling. Confirm the official auto-ranging 100–240 V, 50/60 Hz power
   requirement from the unit/manual.
3. Leave microphones disconnected. Do not enable phantom power blindly. Note
   every already connected source and whether it can safely receive 48 V.
4. Connect mixer control by wired Ethernet where practical. Audio is USB, not
   Ethernet/Wi-Fi. Do not rely on the built-in 2.4 GHz access point for control
   during acceptance when a wired path is available.
5. Open M-AIR Edit without changing settings. Record hardware revision,
   installed firmware, clock rate, USB mode, and scene/show name. Compare the
   installed version with the official Downloads tab; do not update unless a
   specific verified defect requires it and a rollback/recovery plan exists.
6. Save/export the existing scene/show to the control computer before any
   routing change. Record the backup filename and verify it can be seen.
7. Mute Main LR and make every physical output safe. Confirm monitor wedges,
   headphones, amps and PA cannot surprise anyone. The manual specifically
   warns of pops on clock-rate changes.
8. Use a known-good, short USB 2.0 Type-B **data** cable. Connect the MR18
   directly to a suitable Pi USB port for the baseline, avoiding an unverified
   hub. On Raspberry Pi 4, all USB 2.0 traffic shares the VL805 USB 2.0 hub;
   keep other USB traffic controlled and record the storage connection. See
   the official [Raspberry Pi USB
   documentation](https://www.raspberrypi.com/documentation/computers/raspberry-pi.html#universal-serial-bus-usb).
9. Set the MR18 to 48 kHz while outputs remain safe. Configure ALSA/JACK for
   the same 48 kHz only through the normal owned setup procedure. Do not let
   two layers disagree or resample unnoticed.
10. Confirm the MR18 is in **18×18**, not 2×2, USB mode. Save the observed
    setting in the results.
11. Inspect Linux read-only before changing SHR configuration:

    ```sh
    lsusb
    cat /proc/asound/cards
    arecord -l
    cat /proc/asound/card*/stream* 2>/dev/null
    jack_lsp -p -t
    ```

    Confirm the actual USB device, `snd-usb-audio` binding, supported 18-channel
    48 kHz stream, and JACK capture sources. Do not infer names from this plan.
12. While SHR is stopped, prepare 18 private logical slots if they do not
    already exist. This is a portable label template, not a device route; every
    blank final field must remain `missing` until deliberately assigned:

    ```ini
    capture.track=input-1|Input 1||mono|false|
    capture.track=input-2|Input 2||mono|false|
    capture.track=input-3|Input 3||mono|false|
    capture.track=input-4|Input 4||mono|false|
    capture.track=input-5|Input 5||mono|false|
    capture.track=input-6|Input 6||mono|false|
    capture.track=input-7|Input 7||mono|false|
    capture.track=input-8|Input 8||mono|false|
    capture.track=input-9|Input 9||mono|false|
    capture.track=input-10|Input 10||mono|false|
    capture.track=input-11|Input 11||mono|false|
    capture.track=input-12|Input 12||mono|false|
    capture.track=input-13|Input 13||mono|false|
    capture.track=input-14|Input 14||mono|false|
    capture.track=input-15|Input 15||mono|false|
    capture.track=input-16|Input 16||mono|false|
    capture.track=input-17|Input 17 · Line L|line-17-18|left|false|
    capture.track=input-18|Input 18 · Line R|line-17-18|right|false|
    ```

    Then copy the exact observed JACK source names into those private mappings
    using the AUDIO screen. Never put observed names in Rust, tracked examples,
    or public Projects. Refresh and verify each assigned preference says
    `ready`; leave every unassigned slot disarmed.
13. In M-AIR Edit, inspect Inputs/USB routing and USB sends. Configure USB
    channels 1–18 to the intended individual input sources and explicitly
    record each tap. For a dry-stem baseline choose the suitable pre-processing
    tap only after confirming its label on the actual firmware. Keep the scene
    backup untouched.
14. With outputs safe, inject one distinctive safe signal into **one input at a
    time**. Use a line-level generator for 17–18 and source-appropriate levels
    for 1–16. Never create a loop from mixer output to input. Never apply phantom
    power without confirming the connected device and cable support it.
15. For each input, confirm only the intended selected-track meter responds and
    record its physical input → USB channel → observed JACK port → SHR track ID.
    Stop immediately on a swap, duplicate, bleed, unexpected processing, clip,
    or missing channel.
16. Record progressive takes at 2, 4, 8, 12, 16, then 18 armed channels. Re-run
    the single-input identity check when a larger group first fails.
17. For every take, inspect all mono WAV headers and `session.json`. Require one
    rate, identical non-zero frame counts, expected duration, `complete` state,
    zero drop/overflow/callback/xrun counts, correct source identity, and no
    unintended signal in other stems.
18. During each run monitor JACK xruns, recorder drop/overflow and writer
    high-water, callback setting, CPU per core, temperature, memory, free space,
    and sustained storage throughput. Useful read-only views include
    `jack_iodelay` only when deliberately wired for it; do not alter this test's
    graph casually. Use ordinary system tools for CPU/thermal/storage.
19. Increase duration only after the previous channel count passes: 30 s map
    check, 5 min stability, then 30 min and a longer soak as time/storage allow.
20. Record sample rate, JACK frames/period and periods/buffer, storage target,
    filesystem/free space, cable/USB port, and all results. If latency changes,
    write the new value and rerun the lower-channel baseline.
21. Stop on corruption, channel swap/bleed, non-zero recorder drop/overflow,
    JACK xrun, source loss, thermal throttling/warning, power warning, low-space
    warning, write/finalization error, or unequal frames. Preserve failed take
    and logs; do not relabel it successful.
22. Restore the saved mixer scene and normal output safety state after testing.
    Confirm SHR has stopped recording and owns no stale JACK ports. Do not leave
    phantom power or routing changed without an explicit handoff.

## Pass/fail thresholds

A row passes only when:

- every armed source is exact and independently verified;
- `session.json` is `complete`, every WAV is mono 24-bit at the selected rate,
  and all frame counts equal the expected callback-bounded timeline;
- channel identity passes with no swap, duplicate, or measurable/visible
  unintended test signal in another digital stem;
- JACK xruns, recorder dropped frames, overflow events, callback violations,
  source-loss events, and write/finalization errors are all **zero**;
- no power/undervoltage or thermal warning occurs, temperature stays below the
  machine's throttling threshold, free storage retains at least the agreed
  reserve, and observed throughput has margin over the recorded stem rate;
- CPU and writer high-water remain stable rather than growing without bound.

Any violated item fails the row. Higher latency may be a valid diagnosed fix,
but only after recording the change and repeating the lower-channel baseline.

## Printable results

| Channels | Rate | JACK frames / periods | Duration | Xruns | Recorder dropped frames | Writer high-water | Max CPU | Max temp | Storage throughput | Frame counts agree | Channel identity | Pass/fail | Notes |
|---:|---:|---|---:|---:|---:|---:|---:|---:|---:|---|---|---|---|
| 2 | 48 kHz | | | | | | | | | | | | |
| 4 | 48 kHz | | | | | | | | | | | | |
| 8 | 48 kHz | | | | | | | | | | | | |
| 12 | 48 kHz | | | | | | | | | | | | |
| 16 | 48 kHz | | | | | | | | | | | | |
| 18 | 48 kHz | | | | | | | | | | | | |
| 18 long | 48 kHz | | | | | | | | | | | | |

Observed identifiers and routing:

| Physical input | Intended tap | USB channel | Observed ALSA/JACK source | SHR stable ID | Identity result / notes |
|---:|---|---:|---|---|---|
| 1–16 | | | | | |
| 17 L | | | | | |
| 18 R | | | | | |

## Future final-bus acceptance (not yet run)

After the raw multitrack matrix passes, obtain—not guess—the MR18 JACK capture
names for the deliberately mixed external stereo pair and place them in the
private `audio.graph.input` setting. With monitor/speaker level controlled,
verify synth-only, loop-only, input-only, and all-three identity; confirm there
is no parallel direct loop/synth path; and deliberately choose either interface
direct monitoring or the software path. Exercise source disconnect/reconnect,
transaction rollback, final-recording fault behavior, L/R identity, limiter
latency, JACK xruns/timing, and playback/file equality. Record exact observed
names, period count, converter latency, and results here. Synthetic tests do not
count as this physical acceptance.
