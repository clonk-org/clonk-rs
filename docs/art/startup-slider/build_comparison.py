"""Compare unscaled slider crops from native GPU captures.

    python3 build_comparison.py <before.png> <after.png> <output.png>
"""

from pathlib import Path
import sys

from PIL import Image, ImageDraw, ImageFont


CROP = (1260, 800, 2420, 1200)  # Both Audio sliders at 3840x2160 and 3x.
GAP = 16
HEADER = 80
BACKGROUND = (245, 240, 231)
INK = (42, 37, 29)


def main() -> None:
    before, after, output = (Path(argument) for argument in sys.argv[1:4])
    width = CROP[2] - CROP[0]
    height = CROP[3] - CROP[1]
    sheet = Image.new("RGB", (width * 2 + GAP * 3, height + HEADER), BACKGROUND)
    draw = ImageDraw.Draw(sheet)
    font = ImageFont.load_default(size=36)
    for index, (label, path) in enumerate((("Before", before), ("After", after))):
        frame = Image.open(path).convert("RGB")
        if frame.size != (3840, 2160):
            raise ValueError(f"{path} must be a native 3840x2160 GPU capture")
        x = GAP + index * (width + GAP)
        draw.text((x, 12), label, fill=INK, font=font)
        sheet.paste(frame.crop(CROP), (x, HEADER))
    sheet.save(output, optimize=True)


if __name__ == "__main__":
    main()
