#!/usr/bin/env python3
"""Combine album-cover background off/on GUI captures into review images."""

from __future__ import annotations

import argparse
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("captures", type=Path, help="directory written by the GTK matrix test")
    parser.add_argument("output", type=Path, help="directory for paired comparison screenshots")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)

    font = ImageFont.load_default()
    off_paths = sorted(args.captures.glob("*-background-off.png"))
    if not off_paths:
        parser.error(f"no background-off captures found in {args.captures}")

    for off_path in off_paths:
        theme = off_path.name.removesuffix("-background-off.png")
        on_path = args.captures / f"{theme}-background-on.png"
        if not on_path.is_file():
            parser.error(f"missing matching background-on capture: {on_path}")
        off = Image.open(off_path).convert("RGB")
        on = Image.open(on_path).convert("RGB")
        if off.size != on.size:
            parser.error(f"capture sizes differ for {theme}: {off.size} vs {on.size}")

        width, height = off.size
        label_height = 34
        divider = 2
        comparison = Image.new(
            "RGB", (width * 2 + divider, height + label_height), (245, 245, 245)
        )
        draw = ImageDraw.Draw(comparison)
        draw.text((12, 10), f"{theme}: bookshelf background OFF", fill=(30, 30, 30), font=font)
        draw.text(
            (width + divider + 12, 10),
            f"{theme}: bookshelf background ON",
            fill=(30, 30, 30),
            font=font,
        )
        comparison.paste(off, (0, label_height))
        comparison.paste(on, (width + divider, label_height))
        draw.rectangle((width, 0, width + divider - 1, height + label_height), fill=(90, 90, 90))
        comparison.save(args.output / f"{theme}.jpg", quality=93, optimize=True)

    print(f"Created {len(off_paths)} paired screenshots in {args.output}")


if __name__ == "__main__":
    main()
