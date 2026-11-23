#!/usr/bin/env python3
"""Compress a QA walkthrough video into a small shareable MP4."""

from __future__ import annotations

import argparse
import math
import os
import pathlib
import shutil
import subprocess
import sys

DEFAULT_MAX_BYTES = 300 * 1024 * 1024


def parse_size(raw: str) -> int:
    text = raw.strip().lower().replace("_", "")
    suffixes = {
        "gib": 1024 * 1024 * 1024,
        "gb": 1000 * 1000 * 1000,
        "mib": 1024 * 1024,
        "mb": 1000 * 1000,
        "kib": 1024,
        "kb": 1000,
    }
    for suffix, multiplier in suffixes.items():
        if text.endswith(suffix):
            return int(float(text[: -len(suffix)]) * multiplier)
    return int(text)


def human_size(num_bytes: int) -> str:
    value = float(num_bytes)
    for unit in ("B", "KiB", "MiB", "GiB"):
        if value < 1024 or unit == "GiB":
            return f"{value:.1f} {unit}" if unit != "B" else f"{int(value)} {unit}"
        value /= 1024
    return f"{num_bytes} B"


def run(cmd: list[str], input_text: str | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        cmd,
        text=True,
        input=input_text,
        capture_output=True,
        check=False,
    )


def binary_candidates(name: str) -> list[str]:
    candidates = [
        shutil.which(name),
        f"/opt/homebrew/bin/{name}",
        f"/usr/local/bin/{name}",
    ]
    deduped = []
    for candidate in candidates:
        if candidate and candidate not in deduped:
            deduped.append(candidate)
    return deduped


def find_working_binary(name: str) -> str | None:
    for candidate in binary_candidates(name):
        if not pathlib.Path(candidate).is_file():
            continue
        try:
            result = run([candidate, "-version"])
        except FileNotFoundError:
            continue
        if result.returncode == 0:
            return candidate
    return None


def ffprobe_duration(path: pathlib.Path) -> float | None:
    ffprobe = find_working_binary("ffprobe")
    if ffprobe is None:
        return None
    result = run(
        [
            ffprobe,
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            str(path),
        ]
    )
    if result.returncode != 0:
        return None
    try:
        duration = float(result.stdout.strip())
    except ValueError:
        return None
    return duration if duration > 0 else None


def choose_bitrate_kbps(
    duration_seconds: float | None,
    max_bytes: int,
    requested: int | None,
    min_bitrate: int,
    max_bitrate: int,
) -> int:
    if requested is not None:
        return requested
    if duration_seconds is None:
        return max_bitrate
    reserved_bits = max_bytes * 8 * 0.85
    target = math.floor(reserved_bits / duration_seconds / 1000)
    return max(min_bitrate, min(max_bitrate, target))


def default_output_path(input_path: pathlib.Path) -> pathlib.Path:
    return input_path.with_name(f"{input_path.stem}.compressed.mp4")


def compress_video(args: argparse.Namespace) -> pathlib.Path:
    ffmpeg = find_working_binary("ffmpeg")
    if ffmpeg is None:
        raise SystemExit("ffmpeg is required to compress QA recordings")

    input_path = args.input.resolve()
    if not input_path.is_file():
        raise SystemExit(f"input video does not exist: {input_path}")

    output_path = (args.output or default_output_path(input_path)).resolve()
    if output_path == input_path:
        raise SystemExit("output path must differ from input path")
    output_path.parent.mkdir(parents=True, exist_ok=True)

    duration = ffprobe_duration(input_path)
    bitrate = choose_bitrate_kbps(
        duration,
        args.max_bytes,
        args.bitrate_kbps,
        args.min_bitrate_kbps,
        args.max_bitrate_kbps,
    )
    maxrate = max(bitrate, math.ceil(bitrate * 1.25))
    bufsize = max(bitrate, bitrate * 2)
    video_filter = f"scale=w='min({args.width},iw)':h=-2,fps={args.fps}"

    command = [
        ffmpeg,
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-i",
        str(input_path),
        "-map",
        "0:v:0",
        "-vf",
        video_filter,
        "-c:v",
        "libx264",
        "-preset",
        args.preset,
        "-pix_fmt",
        "yuv420p",
        "-b:v",
        f"{bitrate}k",
        "-maxrate",
        f"{maxrate}k",
        "-bufsize",
        f"{bufsize}k",
        "-an",
        "-movflags",
        "+faststart",
        str(output_path),
    ]
    result = run(command)
    if result.returncode != 0:
        raise SystemExit(result.stderr.strip() or "ffmpeg failed")

    size = output_path.stat().st_size
    if size > args.max_bytes:
        raise SystemExit(
            "compressed video is "
            f"{human_size(size)}, over the {human_size(args.max_bytes)} cap. "
            "Use a shorter clip, lower --min-bitrate-kbps, or split the run."
        )

    print(f"compressed_path={output_path}")
    print(f"compressed_size={human_size(size)}")
    print(f"video_bitrate_kbps={bitrate}")
    if duration is not None:
        print(f"duration_seconds={duration:.1f}")
    return output_path


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=pathlib.Path, help="Raw .mov or other QA recording")
    parser.add_argument("--output", type=pathlib.Path, help="Compressed MP4 output path")
    parser.add_argument(
        "--max-bytes",
        type=parse_size,
        default=parse_size(os.environ.get("QA_VIDEO_MAX_BYTES", str(DEFAULT_MAX_BYTES))),
        help="Maximum compressed artifact size, default 300 MiB",
    )
    parser.add_argument("--fps", type=int, default=10, help="Output frame rate")
    parser.add_argument("--width", type=int, default=1280, help="Maximum output width")
    parser.add_argument("--preset", default="veryfast", help="ffmpeg x264 preset")
    parser.add_argument("--bitrate-kbps", type=int, help="Fixed target video bitrate")
    parser.add_argument("--min-bitrate-kbps", type=int, default=250)
    parser.add_argument("--max-bitrate-kbps", type=int, default=1200)
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    if args.min_bitrate_kbps > args.max_bitrate_kbps:
        raise SystemExit("--min-bitrate-kbps must be <= --max-bitrate-kbps")
    compress_video(args)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
