"""Compare unscaled tab crops from before and after native GPU captures.

    python3 build_comparison.py <before.png> <after.png> <output.png>
"""

from pathlib import Path
import sys

from PIL import Image, ImageDraw, ImageFont


CROP = (735, 225, 1145, 730)  # Program and Graphics tabs, 3840x2160 at 3x.
GAP = 16
HEADER = 50
BACKGROUND = (245, 240, 230)
INK = (42, 37, 29)


def main() -> None:
    before, after, output = (Path(argument) for argument in sys.argv[1:4])
    width = CROP[2] - CROP[0]
    height = CROP[3] - CROP[1]
    sheet = Image.new("RGB", (width * 2 + GAP * 3, height + HEADER + GAP), BACKGROUND)
    draw = ImageDraw.Draw(sheet)
    font = ImageFont.load_default(size=25)
    for index, (label, path) in enumerate((("Before", before), ("After", after))):
        frame = Image.open(path).convert("RGB")
        if frame.size != (3840, 2160):
            raise ValueError(f"{path} must be a native 3840x2160 GPU capture")
        x = GAP + index * (width + GAP)
        draw.text((x, 10), label, fill=INK, font=font)
        sheet.paste(frame.crop(CROP), (x, HEADER))
    sheet.save(output, optimize=True)


if __name__ == "__main__":
    main()
