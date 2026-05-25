#!/usr/bin/env python3
"""Preview algorithmic front-fin cycles for bigbert.kdl."""

from __future__ import annotations

import argparse
import curses
import re
import time
from pathlib import Path


POSE_RE = re.compile(r'(?ms)^({pose})\s+#+"""\n(.*?)\n"""#+\s*$')


def extract_pose(path: Path, pose: str) -> list[str]:
    source = path.read_text()
    pattern = re.compile(POSE_RE.pattern.format(pose=re.escape(pose)), POSE_RE.flags)
    match = pattern.search(source)
    if not match:
        raise SystemExit(f"could not find pose {pose!r} in {path}")
    return match.group(2).splitlines()


def rightmost_segment(row: str) -> tuple[int, str] | None:
    matches = list(re.finditer(r"\S+", row))
    if not matches:
        return None
    match = matches[-1]
    return match.start(), match.group(0)


def shift_segment(row: str, start: int, text: str, offset: int, width: int) -> str:
    if offset == 0:
        return row.ljust(width)

    canvas = list(row.ljust(width))
    for col in range(start, start + len(text)):
        if 0 <= col < len(canvas):
            canvas[col] = " "

    new_start = max(0, min(width - len(text), start + offset))
    for index, char in enumerate(text):
        canvas[new_start + index] = char
    return "".join(canvas)


def round_away_from_zero(value: float) -> int:
    if value < 0:
        return -round_away_from_zero(-value)
    return int(value + 0.5)


def weighted_offsets(tip_offset: int) -> tuple[int, int, int]:
    return (
        round_away_from_zero(tip_offset * 0.30),
        round_away_from_zero(tip_offset * 0.50),
        tip_offset,
    )


def pulse_offsets(tip_offset: int, bend: bool, tug_root: bool) -> tuple[int, int, int]:
    direction = 1 if tip_offset > 0 else -1 if tip_offset < 0 else 0
    if direction == 0:
        return (0, 0, 0)
    return (
        direction if tug_root else 0,
        direction if bend else 0,
        direction,
    )


def place_front_segment(base: str, width: int, start: int, text: str) -> str:
    canvas = list(base.ljust(width))
    segment = rightmost_segment(base)
    if segment is not None:
        old_start, old_text = segment
        for col in range(old_start, old_start + len(old_text)):
            if 0 <= col < width:
                canvas[col] = " "

    new_start = max(0, min(width - len(text), start))
    for index, char in enumerate(text):
        canvas[new_start + index] = char
    return "".join(canvas)


def build_oar_frames(lines: list[str], amplitude: int) -> list[tuple[str, list[str]]]:
    width = max(len(line) for line in lines) + amplitude + 4
    shaft = rightmost_segment(lines[-2])
    tip = rightmost_segment(lines[-1])
    if shaft is None or tip is None:
        raise SystemExit("could not locate bigbert front-fin shaft and tip")

    shaft_text = shaft[1]
    tip_text = tip[1]
    tight_tip = "/,/" if tip_text == "/,,/" else tip_text
    fin_rows = [len(lines) - 4, len(lines) - 3, len(lines) - 2, len(lines) - 1]
    rest_rows = [lines[row_index].ljust(width) for row_index in fin_rows]

    def oar_rows(specs: list[tuple[int, str]]) -> list[str]:
        return [
            place_front_segment(lines[row_index], width, start, text)
            for row_index, (start, text) in zip(fin_rows, specs, strict=True)
        ]

    shapes = [
        ("as-is", rest_rows),
        (
            "forward catch",
            oar_rows([(16, shaft_text), (16, shaft_text), (14, shaft_text), (12, tip_text)]),
        ),
        (
            "forward pull",
            oar_rows([(16, shaft_text), (15, shaft_text), (13, shaft_text), (11, tight_tip)]),
        ),
        (
            "forward catch",
            oar_rows([(16, shaft_text), (16, shaft_text), (14, shaft_text), (12, tip_text)]),
        ),
        ("as-is", rest_rows),
        (
            "back recovery",
            oar_rows([(17, shaft_text), (17, shaft_text), (16, shaft_text), (15, tip_text)]),
        ),
        (
            "back sweep",
            oar_rows([(17, shaft_text), (17, shaft_text), (17, shaft_text), (16, tip_text)]),
        ),
        (
            "back sweep hold",
            oar_rows([(17, shaft_text), (17, shaft_text), (17, shaft_text), (16, tip_text)]),
        ),
        (
            "back extension",
            oar_rows([(17, shaft_text), (18, shaft_text), (18, shaft_text), (18, tight_tip)]),
        ),
    ]

    frames = []
    for label, fin_shape in shapes:
        frame = [line.ljust(width) for line in lines]
        for row_index, row in zip(fin_rows, fin_shape, strict=True):
            frame[row_index] = row
        frames.append((label, frame))
    return frames


def build_bendy_frames(lines: list[str], amplitude: int, pulse: bool) -> list[tuple[str, list[str]]]:
    width = max(len(line) for line in lines) + amplitude + 4
    fin_rows = [len(lines) - 3, len(lines) - 2, len(lines) - 1]
    fin_segments = {}
    for row_index in fin_rows:
        segment = rightmost_segment(lines[row_index])
        if segment is None:
            raise SystemExit(f"could not locate fin segment on art row {row_index + 1}")
        fin_segments[row_index] = segment

    if pulse:
        steps = [
            ("as-is", 0, False, False),
            ("forward tip", 1, False, False),
            ("forward bend", 1, True, False),
            ("forward tip", 1, False, False),
            ("forward bend", 1, True, False),
            ("forward root tug", 1, True, True),
            ("forward bend", 1, True, False),
            ("forward tip", 1, False, False),
            ("as-is", 0, False, False),
            ("back tip", -1, False, False),
            ("back bend", -1, True, False),
            ("back tip", -1, False, False),
            ("back bend", -1, True, False),
            ("back root tug", -1, True, True),
            ("back bend", -1, True, False),
            ("back tip", -1, False, False),
        ]
    else:
        steps = [
            ("as-is", 0, False, False),
            ("forward light", 1, False, False),
            ("forward full", amplitude, False, False),
            ("forward light", 1, False, False),
            ("forward full", amplitude, False, False),
            ("forward light", 1, False, False),
            ("forward full", amplitude, False, False),
            ("forward light", 1, False, False),
            ("as-is", 0, False, False),
            ("back light", -1, False, False),
            ("back full", -amplitude, False, False),
            ("back light", -1, False, False),
            ("back full", -amplitude, False, False),
            ("back light", -1, False, False),
            ("back full", -amplitude, False, False),
            ("back light", -1, False, False),
        ]

    frames = []
    for label, tip_offset, bend, tug_root in steps:
        frame = [line.ljust(width) for line in lines]
        offsets = (
            pulse_offsets(tip_offset, bend, tug_root)
            if pulse
            else weighted_offsets(tip_offset)
        )
        for row_index, offset in zip(fin_rows, offsets, strict=True):
            start, text = fin_segments[row_index]
            frame[row_index] = shift_segment(lines[row_index], start, text, offset, width)
        frames.append((label, frame))
    return frames


def build_frames(lines: list[str], amplitude: int, style: str) -> list[tuple[str, list[str]]]:
    if style == "oar":
        return build_oar_frames(lines, amplitude)
    if style == "weighted":
        return build_bendy_frames(lines, amplitude, pulse=False)
    if style == "pulse":
        return build_bendy_frames(lines, amplitude, pulse=True)
    raise SystemExit(f"unknown style {style!r}")


def dump_kdl(frames: list[tuple[str, list[str]]], pose: str) -> None:
    for index, (_, frame) in enumerate(frames[1:], start=1):
        print(f'{pose}{index} ###"""')
        print("\n".join(line.rstrip() for line in frame))
        print('"""###\n')


def run_tui(stdscr: curses.window, frames: list[tuple[str, list[str]]], pose: str, delay: float) -> None:
    curses.curs_set(0)
    stdscr.nodelay(True)
    frame_index = 0
    paused = False

    while True:
        key = stdscr.getch()
        if key in (ord("q"), 27):
            break
        if key == ord(" "):
            paused = not paused
        elif key in (ord("-"), ord("_")):
            delay = min(1.0, delay + 0.02)
        elif key in (ord("+"), ord("=")):
            delay = max(0.02, delay - 0.02)
        elif key in (curses.KEY_RIGHT, ord("l")):
            frame_index = (frame_index + 1) % len(frames)
            paused = True
        elif key in (curses.KEY_LEFT, ord("h")):
            frame_index = (frame_index - 1) % len(frames)
            paused = True

        label, frame = frames[frame_index]
        stdscr.erase()
        stdscr.addstr(0, 0, f"bigbert {pose} fin cycle | frame {frame_index + 1}/{len(frames)} | {label}")
        stdscr.addstr(1, 0, "space pause  h/l step  +/- speed  q quit")
        for row, line in enumerate(frame, start=3):
            stdscr.addstr(row, 0, line.rstrip())
        stdscr.refresh()

        if not paused:
            frame_index = (frame_index + 1) % len(frames)
        time.sleep(delay)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("path", nargs="?", default="bigbert.kdl", type=Path)
    parser.add_argument("--pose", default="right")
    parser.add_argument("--amplitude", default=2, type=int)
    parser.add_argument("--delay", default=0.12, type=float)
    parser.add_argument("--style", choices=("oar", "weighted", "pulse"), default="oar")
    parser.add_argument("--pulse", action="store_true", help="alias for --style pulse")
    parser.add_argument("--dump-kdl", action="store_true", help="print generated pose nodes instead of animating")
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    if args.amplitude < 1:
        raise SystemExit("--amplitude must be at least 1")

    style = "pulse" if args.pulse else args.style
    lines = extract_pose(args.path, args.pose)
    frames = build_frames(lines, args.amplitude, style)
    if args.dump_kdl:
        dump_kdl(frames, args.pose)
    else:
        curses.wrapper(run_tui, frames, args.pose, args.delay)


if __name__ == "__main__":
    main()
