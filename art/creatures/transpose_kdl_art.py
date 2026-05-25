#!/usr/bin/env python3
"""Mirror left/right ASCII-art pose nodes from a creature KDL file."""

from __future__ import annotations

import argparse
import os
import re
import sys
from dataclasses import dataclass
from pathlib import Path


POSE_RE = re.compile(
    r'(?ms)^(?P<name>[A-Za-z][A-Za-z0-9_-]*)\s+'
    r'(?P<hashes>#+)"""\n(?P<art>.*?)\n"""(?P=hashes)\s*$'
)

MIRROR_CHARS = str.maketrans(
    {
        "/": "\\",
        "\\": "/",
        "(": ")",
        ")": "(",
        "<": ">",
        ">": "<",
        "[": "]",
        "]": "[",
        "{": "}",
        "}": "{",
    }
)


@dataclass(frozen=True)
class Pose:
    name: str
    hashes: str
    art: str


def parse_poses(source: str) -> list[Pose]:
    return [
        Pose(
            name=match.group("name"),
            hashes=match.group("hashes"),
            art=match.group("art"),
        )
        for match in POSE_RE.finditer(source)
    ]


def pose_sort_key(name: str, prefix: str) -> int:
    if name == prefix:
        return 0
    return int(name.removeprefix(prefix))


def pose_suffix(name: str, prefix: str) -> str:
    if name == prefix:
        return ""
    return name.removeprefix(prefix)


def source_and_target(direction: str) -> tuple[str, str]:
    if direction == "ltr":
        return "left", "right"
    if direction == "rtl":
        return "right", "left"
    raise ValueError(f"unknown direction: {direction}")


def matching_poses(poses: list[Pose], prefix: str) -> list[Pose]:
    pattern = re.compile(rf"^{re.escape(prefix)}(?:[0-9]+)?$")
    matches = [pose for pose in poses if pattern.match(pose.name)]
    return sorted(matches, key=lambda pose: pose_sort_key(pose.name, prefix))


def clean_mirrored_line(line: str, direction: str) -> str:
    if direction == "rtl":
        line = line.replace(" o ''-'", " o `'-'")
        line = line.replace("/-''", "/-`'")
        line = line.replace("'----''\\", "'----'`\\")
        return re.sub(r"(?<=\s)'----'(?=\s)", "`----`", line)

    line = line.replace("_'-'`", "_'-''")
    line = line.replace("'`-", "''-")
    line = line.replace("/`'----'", "/''----'")
    return re.sub(r"(?<=\s)`----`(?=\s)", "'----'", line)


def mirror_art(art: str, direction: str) -> str:
    lines = art.splitlines()
    width = max((len(line) for line in lines), default=0)
    mirrored = [
        clean_mirrored_line(line.ljust(width)[::-1].translate(MIRROR_CHARS).rstrip(), direction)
        for line in lines
    ]
    return "\n".join(mirrored)


def emit_transposed(poses: list[Pose], direction: str) -> None:
    source_prefix, target_prefix = source_and_target(direction)
    source_poses = matching_poses(poses, source_prefix)
    if not source_poses:
        raise SystemExit(f"no {source_prefix!r} frames found")

    for index, pose in enumerate(source_poses):
        suffix = pose_suffix(pose.name, source_prefix)
        target_name = f"{target_prefix}{suffix}"
        if index > 0:
            print()
        print(f'{target_name} {pose.hashes}"""')
        print(mirror_art(pose.art, direction))
        print(f'"""{pose.hashes}')


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    direction = parser.add_mutually_exclusive_group(required=True)
    direction.add_argument("--ltr", action="store_const", const="ltr", dest="direction")
    direction.add_argument("--rtl", action="store_const", const="rtl", dest="direction")
    parser.add_argument("input", type=Path)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    try:
        source = args.input.read_text()
    except OSError as error:
        raise SystemExit(f"{args.input}: {error}") from error

    emit_transposed(parse_poses(source), args.direction)


if __name__ == "__main__":
    try:
        main()
    except BrokenPipeError:
        devnull = os.open(os.devnull, os.O_WRONLY)
        os.dup2(devnull, sys.stdout.fileno())
        sys.exit(1)
