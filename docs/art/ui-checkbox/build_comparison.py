"""Lay out unscaled Audio options crops of the classic, previous and current
checkbox artwork from two `CLONK_HD_UI_CAPTURE` directories.

    python3 build_comparison.py <previous-captures> <current-captures> <output.png>

The capture test writes `audio-before.png` with the classic sheets and
`audio-after.png` with the high-resolution artwork. The classic row comes from
the current directory; both directories hold identical classic frames.
"""

from __future__ import annotations

import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

# Both checked boxes of the first column, and the unchecked and checked boxes
# of the second, in the 3840x2160 frame at 3x display scaling.
CROPS = ((1236, 424, 1636, 656), (1962, 424, 2362, 656))
LABEL_WIDTH = 190
GAP = 12
BACKGROUND = (244, 241, 234)
INK = (40, 36, 30)


def main() -> None:
    previous, current, output = (Path(argument) for argument in sys.argv[1:4])
    rows = (
        ("Classic", current / "audio-before.png"),
        ("Before", previous / "audio-after.png"),
        ("After", current / "audio-after.png"),
    )
    crop_width = sum(right - left for left, _, right, _ in CROPS) + GAP * (len(CROPS) - 1)
    crop_height = max(bottom - top for _, top, _, bottom in CROPS)
    sheet = Image.new(
        "RGB",
        (LABEL_WIDTH + crop_width + GAP, len(rows) * (crop_height + GAP) + GAP),
        BACKGROUND,
    )
    draw = ImageDraw.Draw(sheet)
    font = ImageFont.load_default(size=30)
    for index, (label, path) in enumerate(rows):
        frame = Image.open(path).convert("RGB")
        top = GAP + index * (crop_height + GAP)
        draw.text((GAP, top + crop_height // 2), label, fill=INK, font=font, anchor="lm")
        left = LABEL_WIDTH
        for crop in CROPS:
            sheet.paste(frame.crop(crop), (left, top))
            left += crop[2] - crop[0] + GAP
    sheet.save(output, optimize=True)


if __name__ == "__main__":
    main()
