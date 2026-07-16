#!/usr/bin/env python3
"""Render small README screenshots without starting SHR-DAW or JACK."""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "docs" / "images"
FONT = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"

FONT_SIZE = 13
CELL_H = 15
PAD_X = 12
PAD_Y = 9
COLS = 40
ROWS = 20
W = 480
H = 320

BG = (17, 19, 22)
PANEL = (24, 27, 31)
FG = (220, 226, 232)
DIM = (124, 133, 143)
GREEN = (78, 213, 133)
YELLOW = (247, 216, 92)
RED = (245, 111, 111)
BLUE = (119, 180, 255)
SELECT_BG = (232, 190, 64)
SELECT_FG = (13, 15, 18)


def fit(line: str) -> str:
    return line[:COLS].ljust(COLS)


def rows(*lines: str) -> list[str]:
    value = [fit(line) for line in lines]
    value.extend(" " * COLS for _ in range(ROWS - len(value)))
    return value[:ROWS]


SCREENS = {
    "shr-daw-presets.png": rows(
        "synthv1 · Velvet Tines              LCK",
        "> 00  Velvet Tines",
        "  01  Hollow Brass",
        "  02  Soft Fifths",
        "  03  Juniper Lead",
        "  04  Dust Pad",
        "  05  Square Bass",
        "",
        "Sound engine: synthv1",
        "MIDI ready · pickup armed",
        "",
        "",
        "",
        "",
        "",
        "Ready",
        " 1:OPS   2:ENGINE 3:NAV    4:SYS",
        "[LOAD]  [PG UP] [PG DOWN] [FIRST]",
        " PRESETS P1 STOP IDLE        --- BPM",
    ),
    "shr-daw-playback.png": rows(
        "             synthv1 · Velvet Tines",
        "",
        "Held: C4 E4 G4",
        "Chord: C major",
        "",
        "Cut  [green]     Res [yellow]",
        "Sus  [red]       Rel [green]",
        "Mod  [green]     Pan [yellow]",
        "",
        "recorded 48 MIDI events",
        "Playback to review",
        "",
        "",
        "",
        "",
        "",
        " 1:OPS   2:SOUND 3:NAV    4:SYS",
        "[RECORD][REC END][TAKE]  [SAVE]",
        " PLAYBACK P1 RUN PLAY",
    ),
    "shr-daw-ft2-pattern.png": rows(
        "MELODY · dusk-project EDIT",
        "ord 01/04 pat 00 · ONLINE",
        "ROW      L1       L2       L3       L4",
        ">00 C-4 60D E-4 58  G-4 5A  ... ..",
        " 01 ... ..  ... ..  ... ..  ... ..",
        " 02 C-5 70  ... ..  OFF ..  ... ..",
        " 03 ... ..  ... ..  ... ..  ... ..",
        " 04 D-4 62T F-4 50  A-4 55  ... ..",
        " 05 ... ..  ... ..  ... ..  ... ..",
        " 06 ... ..  ... ..  ... ..  ... ..",
        " 07 G-3 6A  ... ..  ... ..  ... ..",
        " 08 C-4 60  E-4 60  G-4 60  B-4 60",
        "",
        "P1/2 MELODY L1 ch1 Configured ON",
        "step edit on",
        " 1:OPS   2:MODE  3:MOVE   4:SYS",
        "[PLAY]  [START] [STEP]  [CELL]",
        " FT2 P1 STOP IDLE            120 BPM",
    ),
    "shr-daw-ft2-arrangement.png": rows(
        "FT2 ARRANGEMENT",
        "┌ 4 steps ───────────────────────────┐",
        "> 01  pat 00  064 rows 120 BPM 4/4 2p",
        "  02  pat 01  032 rows 92 BPM  4/4 3p",
        "  03  pat 00  064 rows 120 BPM 4/4 2p",
        "  04  pat 02  024 rows 135 BPM 3/4 1p",
        "└────────────────────────────────────┘",
        "",
        "Repeat uses the same pattern ID.",
        "Clone or paste creates a new pattern.",
        "",
        "",
        "",
        "FT2 arrangement · chain pattern steps",
        " 1:OPS   2:STEP  3:       4:SYS",
        "[PLAY]  [JUMP]  [APPEND][INSERT]",
        " ARRANGE P1 STOP IDLE        120 BPM",
    ),
    "shr-daw-ft2-pages.png": rows(
        "FT2 PATTERN PAGES · 4 LANES",
        "┌ 3 pages ───────────────────────────┐",
        ">01 MELODY   ch01 Configured",
        " 02 DRUMS    ch10 Configured",
        " 03 D-50     ch03 Roland D-50",
        "└────────────────────────────────────┘",
        "",
        "Page setup belongs to this pattern.",
        "Targets can be exact hardware ports.",
        "",
        "",
        "",
        "",
        "page route updated · DONE to keep",
        " 1:OPS   2:PAGE  3:       4:SYS",
        "[ADD]   [TARGET][CHANNEL][DONE]",
        " TRACKS P1 STOP IDLE        120 BPM",
    ),
    "shr-daw-project-files.png": rows(
        "PROJECT FILES",
        "┌ saved Projects · 5 ────────────────┐",
        "> dusk-project",
        "  sunday-sketch",
        "  mt240-drums",
        "  d50-pad-study",
        "  live-set-a",
        "└────────────────────────────────────┘",
        "",
        "Files save/load/delete the whole Project.",
        "Pattern tools stay on PATTERNS.",
        "",
        "",
        "Project files · select an action",
        " 1:OPS   2:PATTERN 3:EDIT  4:SYS",
        "[LOAD]  [SAVE] [PREVIEW][DELETE]",
        " FILES P1 STOP IDLE         120 BPM",
    ),
    "shr-daw-ft2-loop.png": rows(
        "FT2 WAV LOOP",
        "breakbeat-96.wav",
        "",
        "Source  96.00 BPM  1x",
        "Target 120 BPM     ratio 1.250",
        "Region beat 0 +16",
        "Offset +0 bar(s)",
        "Cut BAR · meter 4/4",
        "",
        "PLAY  00:03 / 00:08",
        "48000 Hz · 2ch",
        "Pitch changes with tempo",
        "",
        "",
        " 1:OPS   2:BPM   3:CUT    4:SYS",
        "[IMPORT][HERE] [START] [STOP]",
        " FT2 LOOP P1 STOP IDLE      120 BPM",
    ),
    "shr-daw-audio-recorder.png": rows(
        "             STEREO RECORDER",
        "",
        "AudioBox USB 96",
        "L system:capture_1",
        "R system:capture_2",
        "",
        "Time 00:02:14",
        "Rate 48000 Hz · 24-bit stereo",
        "Size 36.8 MiB",
        "Dropped 0",
        "",
        "recordings/dusk-project-001.wav",
        "R/REC start · STOP finalize",
        "",
        " 1:OPS   2:      3:NAV    4:SYS",
        "[RECORD]",
        " AUDIO P1 STOP IDLE          --- BPM",
    ),
}


def color_for(line: str, y: int) -> tuple[int, int, int]:
    stripped = line.strip()
    if y == 0 or stripped.startswith("FT2 ") or stripped.startswith("TRACKER"):
        return GREEN
    if stripped.startswith("[") or stripped.startswith("1:"):
        return YELLOW
    if "OFFLINE" in stripped or "Dropped" in stripped:
        return RED
    if stripped.startswith("Project") or stripped.startswith("Pattern") or "belongs" in stripped:
        return BLUE
    if y >= ROWS - 2 or not stripped:
        return DIM
    return FG


def render(name: str, content: list[str]) -> None:
    font = ImageFont.truetype(FONT, FONT_SIZE)
    image = Image.new("RGB", (W, H), BG)
    draw = ImageDraw.Draw(image)
    draw.rounded_rectangle((3, 3, W - 3, H - 3), radius=8, fill=PANEL, outline=(46, 51, 57), width=1)
    for y, line in enumerate(content):
        x0 = PAD_X
        y0 = PAD_Y + y * CELL_H
        if line.startswith(">"):
            draw.rectangle((PAD_X - 4, y0 - 2, W - PAD_X + 4, y0 + CELL_H - 3), fill=SELECT_BG)
            draw.text((x0, y0), line, font=font, fill=SELECT_FG)
        else:
            draw.text((x0, y0), line, font=font, fill=color_for(line, y))
    image.save(OUT / name, optimize=True)


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    for name, content in SCREENS.items():
        render(name, content)


if __name__ == "__main__":
    main()
