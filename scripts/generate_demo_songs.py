#!/usr/bin/env python3
"""Generate SHR-DAW's original, cleared public-domain demo arrangements."""

from __future__ import annotations

import argparse
import hashlib
import json
import struct
from dataclasses import dataclass
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
DEMO_DIR = ROOT / "demos"
PPQN = 480
STEPS = 4


@dataclass(frozen=True)
class Demo:
    slug: str
    title: str
    bpm: int
    meter: tuple[int, int]
    key: str
    key_signature: tuple[int, int]
    melody: tuple[tuple[float, int], ...]
    chords: tuple[tuple[int, ...], ...]
    provenance: str
    source_urls: tuple[str, ...]
    description: str
    styles: tuple[str, ...]


def phrase(*items: tuple[float, int]) -> tuple[tuple[float, int], ...]:
    return items


DEMOS = (
    Demo(
        "house-of-the-rising-sun", "House of the Rising Sun", 84, (6, 8), "A minor", (0, 1),
        phrase((.5, 64), (.5, 64), (1, 69), (.5, 71), (.5, 72), (1, 69), (1, 64),
               (.5, 64), (.5, 67), (1, 69), (1, 72), (1, 69), (1, 69), (.5, 69),
               (.5, 72), (1, 74), (1, 77), (1, 74), (1, 72), (1, 69), (1, 64),
               (1, 69), (2, 69)),
        ((57, 60, 64), (60, 64, 67), (62, 66, 69), (65, 69, 72),
         (57, 60, 64), (60, 64, 67), (64, 68, 71), (57, 60, 64)),
        "Traditional Appalachian song. Library of Congress documentation traces circulation before the 1933 commercial recording and a 1937 field recording. This is a new block-chord arrangement and does not reproduce the Animals or another modern arrangement.",
        ("https://www.loc.gov/static/events/concerts-from-the-library-of-congress/documents/programs/2324-Kronos-Quartet-April18-program.pdf", "https://blogs.loc.gov/folklife/2015/10/lomax-kentucky-recordings-go-online/"),
        "A restrained 6/8 folk pulse with melody, block pad, bass, sparse drums, and a new high answer line.",
        ("folk rock", "dark synthwave", "ambient"),
    ),
    Demo(
        "whiskey-in-the-jar", "Whiskey in the Jar", 108, (4, 4), "D major", (2, 0),
        phrase((.5, 62), (.5, 62), (.5, 62), (.5, 64), (1, 66), (.5, 64), (.5, 62),
               (1, 59), (.5, 62), (.5, 62), (.5, 64), (.5, 66), (1, 69), (.5, 66),
               (.5, 64), (1, 62), (.5, 69), (.5, 69), (.5, 71), (.5, 69), (1, 66),
               (.5, 62), (.5, 64), (1, 66), (1, 64), (2, 62)),
        ((62, 66, 69), (59, 62, 66), (67, 71, 74), (62, 66, 69)),
        "Traditional Irish song represented in P. W. Joyce's Old Irish Folk Music and Songs. The melody and story predate modern commercial versions; this arrangement was authored from the traditional tune without copying a recording.",
        ("https://www.itma.ie/scores/pw-joyce-oifms-686/",),
        "Straight four-beat folk groove with singable lead, open-fifth bass, chord pad, drums, and a fiddle-like response.",
        ("Celtic rock", "acoustic", "festival EDM"),
    ),
    Demo(
        "cotton-eyed-joe", "Cotton-Eyed Joe", 126, (4, 4), "G major", (1, 0),
        phrase((.5, 62), (.5, 62), (.5, 67), (.5, 67), (.5, 67), (.5, 69), (.5, 71), (.5, 67),
               (.5, 71), (.5, 71), (.5, 71), (.5, 69), (.5, 67), (.5, 64), (1, 62),
               (.5, 62), (.5, 67), (.5, 67), (.5, 69), (.5, 71), (.5, 67), (.5, 62), (.5, 64),
               (1, 67), (1, 64), (2, 67)),
        ((55, 59, 62), (60, 64, 67), (62, 66, 69), (55, 59, 62)),
        "Traditional American fiddle/dance song. The Library of Congress catalogs a 1939 Lomax field recording and states that Cotton-Eyed Joe is not among the restricted titles in that collection. This is a newly programmed arrangement, not the 1990s dance production.",
        ("https://www.loc.gov/item/lomaxbib000040/", "https://citizen-dj.labs.loc.gov/items/loc-musicbox/20100917222547/"),
        "Compact hoedown melody over bass, offbeat pad, four-lane dance drums, and a fresh octave counterline.",
        ("country dance", "chiptune", "electro-folk"),
    ),
    Demo(
        "scarborough-fair", "Scarborough Fair", 92, (3, 4), "D Dorian", (0, 0),
        phrase((1, 69), (1, 69), (1, 64), (1, 64), (.5, 71), (.5, 72), (1, 71),
               (1, 69), (1, 64), (1, 67), (1, 69), (1, 71), (1, 69), (1, 67),
               (1, 66), (1, 64), (1, 62), (1, 64), (1, 62), (2, 62)),
        ((62, 65, 69), (60, 64, 67), (62, 65, 69), (57, 60, 64)),
        "Traditional English ballad. Library of Congress describes Scarborough Fair as traditional and documents its oral-revival transmission. No harmony or counterpoint from a modern recording is used here.",
        ("https://www.loc.gov/collections/songs-of-america/articles-and-essays/musical-styles/traditional/traditional-ballads/", "https://blogs.loc.gov/folklife/2023/04/homegrown-plus-martin-carthy-master-folksinger-and-guitarist-from-england/"),
        "Modal 3/4 melody with drone-like bass, slow pad, frame-drum pattern, and an independently written descending answer.",
        ("medieval ambient", "trip-hop", "orchestral"),
    ),
    Demo(
        "greensleeves", "Greensleeves", 96, (3, 4), "A minor", (0, 1),
        phrase((1, 64), (1, 69), (1, 72), (1, 74), (1, 76), (.5, 74), (.5, 72),
               (1, 71), (1, 67), (1, 69), (1, 71), (1, 72), (1, 69),
               (1, 68), (1, 69), (1, 64), (1, 69), (1, 72), (1, 74),
               (1, 76), (1, 74), (1, 72), (1, 71), (1, 68), (2, 69)),
        ((57, 60, 64), (55, 59, 62), (60, 64, 67), (52, 56, 59),
         (57, 60, 64), (55, 59, 62), (52, 56, 59), (57, 60, 64)),
        "Anonymous Renaissance tune; a Library of Congress concert note identifies an extant English print naming it in 1588. The arrangement uses a newly authored pulse and countermelody over the old romanesca-derived ground.",
        ("https://www.loc.gov/static/events/concerts-from-the-library-of-congress/documents/programs/2324-Jordi-Savall-Hesperion-Apr2-program.pdf",),
        "Three-beat Renaissance melody, grounded bass, soft triads, restrained percussion, and a new bell response.",
        ("Renaissance", "dream pop", "cinematic"),
    ),
    Demo(
        "amazing-grace", "Amazing Grace", 76, (3, 4), "G major", (1, 0),
        phrase((1, 62), (2, 67), (1, 71), (1, 67), (2, 71), (1, 69), (2, 67),
               (1, 64), (1, 62), (2, 62), (1, 67), (1, 71), (1, 67),
               (2, 71), (1, 69), (1, 74), (2, 71), (1, 74), (1, 71),
               (1, 67), (2, 67)),
        ((55, 59, 62), (60, 64, 67), (55, 59, 62), (62, 66, 69)),
        "John Newton's text appeared in 1779; the Library of Congress documents William Walker's 1835 publication of the associated New Britain tune. Both are long out of copyright; this instrumental setting is newly arranged.",
        ("https://www.loc.gov/collections/amazing-grace/articles-and-essays/dissemination-of-amazing-grace.html/",),
        "Spacious hymn melody with root bass, warm pad, gentle 3/4 percussion, and an original upper echo.",
        ("gospel", "ambient", "orchestral build"),
    ),
    Demo(
        "drunken-sailor", "Drunken Sailor", 116, (4, 4), "D minor", (-1, 1),
        phrase((.5, 69), (.5, 69), (.5, 69), (.5, 69), (.5, 69), (.5, 69), (1, 65), (1, 62),
               (.5, 69), (.5, 69), (.5, 69), (.5, 69), (.5, 69), (.5, 69), (1, 65), (1, 62),
               (.5, 69), (.5, 69), (.5, 69), (.5, 69), (.5, 72), (.5, 74), (.5, 72), (.5, 69),
               (1, 67), (1, 64), (2, 62)),
        ((62, 65, 69), (60, 64, 67), (62, 65, 69), (57, 61, 64)),
        "Traditional sailor work song from the nineteenth-century shanty tradition. Library of Congress sources document the early-to-mid-nineteenth-century genre and archival traditional material. This new syncopated arrangement does not copy a revival recording.",
        ("https://blogs.loc.gov/folklife/2021/01/a-deep-dive-into-sea-shanties/", "https://www.loc.gov/collections/traditional-music-and-spoken-word/about-this-collection/rights-and-access/"),
        "Call-and-response lead with stomping bass, minor pad, deck-like drums, and a newly written response line.",
        ("punk shanty", "industrial", "festival folk"),
    ),
    Demo(
        "wellerman", "Wellerman", 104, (4, 4), "E minor", (1, 1),
        phrase((.5, 59), (.5, 64), (.5, 64), (.5, 64), (1, 67), (1, 71),
               (.5, 71), (.5, 69), (.5, 67), (.5, 66), (1, 66), (1, 64),
               (.5, 67), (.5, 72), (1, 71), (.5, 69), (.5, 67), (1, 66),
               (.5, 64), (.5, 66), (1, 67), (1, 64), (2, 64)),
        ((52, 55, 59), (60, 64, 67), (55, 59, 62), (59, 63, 66)),
        "Anonymous New Zealand whaling song. The National Library of New Zealand records an unknown composer and likely 1830s Otago whaling origin; the Library of Congress distinguishes it as an occupational sea song. This arrangement predates no modern performance and copies none.",
        ("https://natlib.govt.nz/records/45059227", "https://blogs.loc.gov/folklife/2021/01/a-deep-dive-into-sea-shanties/"),
        "Firm bass and communal melody over dry drums and pad, with an original offbeat upper response.",
        ("sea folk", "synth-pop", "drum and bass halftime"),
    ),
    Demo(
        "auld-lang-syne", "Auld Lang Syne", 88, (4, 4), "D major", (2, 0),
        phrase((1, 57), (1, 62), (1, 62), (.5, 62), (.5, 66), (1, 64), (1, 62),
               (1, 64), (1, 66), (1, 62), (1, 66), (1, 69), (1, 71), (.5, 71), (.5, 69),
               (1, 66), (1, 66), (1, 62), (1, 64), (1, 62), (2, 62)),
        ((62, 66, 69), (57, 61, 64), (59, 62, 66), (55, 59, 62)),
        "Robert Burns adapted older traditional words in the late eighteenth century; the National Library of Scotland documents publication in the Scots Musical Museum and the traditional tune history. Burns died in 1796 and the source material is public domain.",
        ("https://www.nls.uk/collections/stories/literature-and-poetry/robert-burns-and-his-history-of-myself/", "https://wee-windaes.nls.uk/the-scots-musical-museum/"),
        "Clear communal melody with simple bass, broad pad, understated drums, and an original farewell counterline.",
        ("piano ballad", "post-rock", "New Year synthwave"),
    ),
    Demo(
        "danny-boy", "Danny Boy", 72, (4, 4), "D major", (2, 0),
        phrase((1, 57), (1, 62), (1, 64), (1, 66), (1, 67), (.5, 66), (.5, 64), (1, 62),
               (1, 71), (1, 69), (1, 66), (1, 62), (1, 64), (1, 66), (1, 67), (1, 69),
               (1, 74), (1, 73), (1, 71), (1, 69), (1, 66), (1, 69), (1, 74), (2, 74)),
        ((62, 66, 69), (55, 59, 62), (59, 62, 66), (57, 61, 64)),
        "Frederic Weatherly's song was published in 1913 over the traditional Londonderry Air; library catalogs identify the 1913 score and Weatherly's 1848–1929 life dates. Publication-age and life-plus-70 terms have expired. This new instrumental arrangement uses no later orchestration.",
        ("https://digitalcommons.library.umaine.edu/mmb-vp/267/", "https://scholarsjunction.msstate.edu/cht-sheet-music/12024/"),
        "Long lyrical lead with slow bass, sustained pad, brushes-style drums, and a newly composed answering voice.",
        ("cinematic", "ambient", "slow rock"),
    ),
)


def vlq(value: int) -> bytes:
    data = [value & 0x7F]
    value >>= 7
    while value:
        data.append(0x80 | (value & 0x7F))
        value >>= 7
    return bytes(reversed(data))


def midi_track(events: list[tuple[int, bytes]]) -> bytes:
    def event_priority(event: bytes) -> int:
        status = event[0] & 0xF0
        if status == 0x80 or (status == 0x90 and len(event) > 2 and event[2] == 0):
            return 1
        if status == 0x90:
            return 2
        return 0

    body = bytearray()
    previous = 0
    for tick, event in sorted(events, key=lambda item: (item[0], event_priority(item[1]))):
        body += vlq(tick - previous) + event
        previous = tick
    body += b"\x00\xff\x2f\x00"
    return b"MTrk" + struct.pack(">I", len(body)) + body


def notes_from_phrase(items: tuple[tuple[float, int], ...], repeats: int = 2) -> list[tuple[float, float, int]]:
    result: list[tuple[float, float, int]] = []
    at = 0.0
    for _ in range(repeats):
        for duration, note in items:
            result.append((at, duration, note))
            at += duration
    return result


def arrangement(demo: Demo) -> tuple[float, dict[str, list[tuple[float, float, int]]]]:
    lead = notes_from_phrase(demo.melody)
    length = max(32.0, lead[-1][0] + lead[-1][1])
    chord_span = length / max(1, len(demo.chords))
    chords: list[tuple[float, float, int]] = []
    bass: list[tuple[float, float, int]] = []
    counter: list[tuple[float, float, int]] = []
    for index, chord in enumerate(demo.chords):
        start = index * chord_span
        for note in chord[:3]:
            chords.append((start, chord_span * .9, note + 12))
        for beat in range(max(1, round(chord_span / 2))):
            bass.append((start + beat * 2, 1.5, chord[0] - 12))
        counter.append((start + chord_span / 2, min(1.0, chord_span / 3), chord[-1] + 12))
    drums: list[tuple[float, float, int]] = []
    beat = 0.0
    while beat < length:
        drums.append((beat, .2, 42))
        if int(beat) % 4 in (0, 2):
            drums.append((beat, .25, 36))
        if int(beat) % 4 in (1, 3):
            drums.append((beat, .25, 38))
        beat += .5
    return length, {"Drums": drums, "Bass": bass, "Pad": chords, "Lead": lead, "Counter": counter}


def make_midi(demo: Demo) -> bytes:
    _, parts = arrangement(demo)
    denominator_power = {2: 1, 4: 2, 8: 3}[demo.meter[1]]
    tempo = round(60_000_000 / demo.bpm)
    conductor = [
        (0, b"\xff\x03\x09Conductor"),
        (0, b"\xff\x51\x03" + tempo.to_bytes(3, "big")),
        (0, bytes((0xFF, 0x58, 4, demo.meter[0], denominator_power, 24, 8))),
        (0, bytes((0xFF, 0x59, 2, demo.key_signature[0] & 0xFF, demo.key_signature[1]))),
    ]
    tracks = [midi_track(conductor)]
    programs = {"Bass": 32, "Pad": 88, "Lead": 40, "Counter": 10}
    channels = {"Lead": 0, "Counter": 1, "Pad": 2, "Bass": 3, "Drums": 9}
    for name in ("Drums", "Bass", "Pad", "Lead", "Counter"):
        channel = channels[name]
        events = [(0, b"\xff\x03" + bytes((len(name),)) + name.encode())]
        if name != "Drums":
            events.append((0, bytes((0xC0 | channel, programs[name]))))
        for start, duration, note in parts[name]:
            on = round(start * PPQN)
            off = round((start + duration) * PPQN)
            velocity = 92 if name == "Lead" else 76
            events.append((on, bytes((0x90 | channel, note, velocity))))
            events.append((off, bytes((0x80 | channel, note, 0))))
        tracks.append(midi_track(events))
    header = b"MThd" + struct.pack(">IHHH", 6, 1, len(tracks), PPQN)
    return header + b"".join(tracks)


def read_vlq(data: bytes, offset: int) -> tuple[int, int]:
    value = 0
    for _ in range(4):
        if offset >= len(data):
            raise ValueError("truncated MIDI variable-length value")
        byte = data[offset]
        offset += 1
        value = (value << 7) | (byte & 0x7F)
        if not byte & 0x80:
            return value, offset
    raise ValueError("overlong MIDI variable-length value")


def validate_midi(data: bytes, part_count: int) -> None:
    if len(data) < 14 or data[:8] != b"MThd\x00\x00\x00\x06":
        raise ValueError("invalid MIDI header")
    fmt, track_count, division = struct.unpack(">HHH", data[8:14])
    if fmt != 1 or track_count != part_count + 1 or not division:
        raise ValueError("demo MIDI must be format 1 with conductor plus parts")
    offset = 14
    parsed_tracks = 0
    while offset < len(data):
        if data[offset:offset + 4] != b"MTrk" or offset + 8 > len(data):
            raise ValueError("invalid MIDI track header")
        length = struct.unpack(">I", data[offset + 4:offset + 8])[0]
        body = data[offset + 8:offset + 8 + length]
        if len(body) != length:
            raise ValueError("truncated MIDI track")
        position = 0
        running_status = None
        ended = False
        while position < len(body):
            _, position = read_vlq(body, position)
            if position >= len(body):
                raise ValueError("MIDI event is missing a status byte")
            status = body[position]
            if status & 0x80:
                position += 1
                if status < 0xF0:
                    running_status = status
            elif running_status is None:
                raise ValueError("MIDI running status has no prior status")
            else:
                status = running_status
            if status == 0xFF:
                if position >= len(body):
                    raise ValueError("truncated MIDI meta event")
                meta_type = body[position]
                position += 1
                size, position = read_vlq(body, position)
                position += size
                running_status = None
                if meta_type == 0x2F:
                    if size or position != len(body):
                        raise ValueError("invalid MIDI end-of-track event")
                    ended = True
            elif status in (0xF0, 0xF7):
                size, position = read_vlq(body, position)
                position += size
                running_status = None
            elif status < 0xF0:
                position += 1 if status & 0xE0 == 0xC0 else 2
            else:
                raise ValueError("unsupported system MIDI event in demo")
            if position > len(body):
                raise ValueError("truncated MIDI event data")
        if not ended:
            raise ValueError("MIDI track is missing its end marker")
        parsed_tracks += 1
        offset += 8 + length
    if parsed_tracks != track_count or offset != len(data):
        raise ValueError("MIDI track count or trailing data is invalid")


def make_project(demo: Demo) -> str:
    length, parts = arrangement(demo)
    rows = min(256, max(1, round(length * STEPS)))
    # The tracker currently models 3- or 4-beat rows without a denominator.
    # A 3-beat grid preserves the two dotted-quarter pulses of the 6/8 demo.
    project_meter = 3 if demo.meter == (6, 8) else demo.meter[0]
    names = ("Drums", "Bass", "Pad", "Lead", "Counter")
    out = [
        "SHSYNTH-SONG 4", f"name={demo.title}", f"steps={STEPS}", "gate=80", "order=0",
        'insert_rack={"effects":[],"order":[]}',
        'aux_routing={"master_rack":{"effects":[],"order":[]},"buses":[],"sends":[]}',
        f"pattern=0|{rows}|{demo.bpm}|{project_meter}",
    ]
    for page, name in enumerate(names):
        percussion = 1 if name == "Drums" else 0
        out.append(f"pattern_page=0|{page}|{name.upper()}|1|96|{percussion}|default")
        for column in range(4):
            out.append(f"pattern_column=0|{page}|{column}|default|default|default|default")
            out.append(f"pattern_lane=0|{page}|{column}|{name[:3].upper()}{column + 1}|1")
    occupied: set[tuple[int, int]] = set()
    for page, name in enumerate(names):
        lane_ends = [0.0] * 4
        for start, duration, note in parts[name]:
            row = min(rows - 1, round(start * STEPS))
            choices = sorted(range(4), key=lambda lane: (lane_ends[lane] > start, lane_ends[lane], lane))
            lane = next((candidate for candidate in choices if (row, page * 4 + candidate) not in occupied), choices[0])
            track = page * 4 + lane
            if (row, track) in occupied:
                continue
            occupied.add((row, track))
            lane_ends[lane] = start + duration
            velocity = 96 if name == "Lead" else 78
            out.append(f"cell=0|{row}|{track}|{note}|{velocity}|-|-|-")
    return "\n".join(out) + "\n"


def generated_files() -> dict[Path, bytes]:
    files: dict[Path, bytes] = {}
    entries = []
    for demo in DEMOS:
        midi_name = f"{demo.slug}.mid"
        project_name = f"{demo.slug}.shsong"
        midi = make_midi(demo)
        validate_midi(midi, 5)
        project = make_project(demo).encode()
        files[DEMO_DIR / midi_name] = midi
        files[DEMO_DIR / project_name] = project
        entries.append({
            "id": demo.slug, "title": demo.title, "composition": "traditional/public-domain source",
            "arrangement_license": "MIT", "bpm": demo.bpm,
            "time_signature": f"{demo.meter[0]}/{demo.meter[1]}", "key": demo.key,
            "tracks": ["Drums", "Bass", "Pad", "Lead", "Counter"],
            "description": demo.description, "style_ideas": list(demo.styles),
            "native_project_meter": (
                "3/4 tracker grid representing the two compound beats of 6/8"
                if demo.meter == (6, 8) else f"{demo.meter[0]}/4 tracker grid"
            ),
            "public_domain_reasoning": demo.provenance, "sources": list(demo.source_urls),
            "midi": midi_name, "project": project_name,
            "sha256": {"midi": hashlib.sha256(midi).hexdigest(), "project": hashlib.sha256(project).hexdigest()},
        })
    manifest = {"schema": 1, "policy": "Only files named here may be installed as demos.", "demos": entries}
    files[DEMO_DIR / "cleared-demos.json"] = (json.dumps(manifest, indent=2, ensure_ascii=False) + "\n").encode()
    return files


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--write", action="store_true", help="write the deterministic demo files")
    parser.add_argument(
        "--files", action="store_true",
        help="after validation, print only manifest-cleared files for packaging",
    )
    args = parser.parse_args()
    expected = generated_files()
    if args.write:
        DEMO_DIR.mkdir(parents=True, exist_ok=True)
        for path, data in expected.items():
            path.write_bytes(data)
        return 0
    mismatches = [str(path.relative_to(ROOT)) for path, data in expected.items() if not path.is_file() or path.read_bytes() != data]
    present = {path for path in DEMO_DIR.glob("*") if path.is_file()} if DEMO_DIR.is_dir() else set()
    extras = sorted(str(path.relative_to(ROOT)) for path in present - set(expected))
    if mismatches or extras:
        for path in mismatches:
            print(f"missing or changed: {path}")
        for path in extras:
            print(f"unlisted demo file: {path}")
        return 1
    if args.files:
        for path in sorted(expected):
            print(path.relative_to(ROOT))
    else:
        print(f"validated {len(DEMOS)} cleared demo arrangements")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
