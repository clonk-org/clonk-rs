"""Fit generated navigation-arrow cutouts to the original sprite cells."""

from __future__ import annotations

from pathlib import Path

import cv2
import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

HERE = Path(__file__).resolve().parent
GRAPHICS = HERE.parents[2] / "planet/Graphics.c4g"
SHEETS = (
    ("red-arrow", "Arrow.png", 64, 64),
    ("wood-arrow", "GUIBigArrows.png", 19, 40),
)
SCALE = 8


def transparent_cutout(path: Path, stem: str) -> Image.Image:
    generated = Image.open(path)
    if generated.mode == "RGBA" and generated.getchannel("A").getextrema()[0] < 255:
        return generated

    rgb = np.asarray(generated.convert("RGB"))
    red, green, blue = (rgb[:, :, channel].astype(np.int16) for channel in range(3))
    if stem == "red-arrow":
        colored = (red - green > 22) & (red - blue > 22) & (red > 65)
        outline = (red < 95) & (green < 95) & (blue < 95)
        candidate = (colored | outline).astype(np.uint8)
    else:
        candidate = ((red - green > 8) & (green - blue > 4) & (red > 43)).astype(
            np.uint8
        )

    count, labels, stats, _ = cv2.connectedComponentsWithStats(candidate, 8)
    assert count > 1, path
    largest = 1 + np.argmax(stats[1:, cv2.CC_STAT_AREA])
    foreground = (labels == largest).astype(np.uint8)
    contours, _ = cv2.findContours(foreground, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)
    cv2.drawContours(foreground, contours, -1, 1, cv2.FILLED)
    foreground = cv2.dilate(foreground, np.ones((3, 3), np.uint8))

    interior = cv2.erode(foreground, np.ones((5, 5), np.uint8))
    _, nearest = cv2.distanceTransformWithLabels(
        1 - interior, cv2.DIST_L2, 5, labelType=cv2.DIST_LABEL_PIXEL
    )
    palette = np.zeros((nearest.max() + 1, 3), np.uint8)
    palette[nearest[interior > 0]] = rgb[interior > 0]
    color = palette[nearest]
    alpha = cv2.GaussianBlur(foreground.astype(np.float32), (0, 0), 1.0)
    return Image.fromarray(np.dstack((color, np.uint8(np.clip(alpha * 255, 0, 255)))))


def main() -> None:
    for stem, filename, width, height in SHEETS:
        source = Image.open(GRAPHICS / filename).convert("RGBA")
        assert source.size == (width * 4, height)
        for phase in range(4):
            before = source.crop((phase * width, 0, (phase + 1) * width, height))
            before.save(HERE / "before" / f"{stem}-{phase}.png")
            bounds = before.getchannel("A").getbbox()
            assert bounds is not None

            if stem == "red-arrow":
                original = np.asarray(before)
                red = original[:, :, 0].astype(np.int16)
                green = original[:, :, 1].astype(np.int16)
                blue = original[:, :, 2].astype(np.int16)
                shape = (
                    (red - green > 35)
                    & (red - blue > 35)
                    & (red > 85)
                    & (original[:, :, 3] > 70)
                )
                y, x = np.where(shape)
                bounds = (
                    max(0, int(x.min()) - 1),
                    max(0, int(y.min()) - 1),
                    min(width, int(x.max()) + 2),
                    min(height, int(y.max()) + 2),
                )

            cutout = transparent_cutout(
                HERE / "support" / f"{stem}-{phase}-generated.png", stem
            )
            visible = cutout.getchannel("A").point(lambda alpha: 255 if alpha >= 8 else 0)
            cutout = cutout.crop(visible.getbbox())
            cutout = cutout.resize(
                ((bounds[2] - bounds[0]) * SCALE, (bounds[3] - bounds[1]) * SCALE),
                Image.Resampling.LANCZOS,
            )
            after = Image.new("RGBA", (width * SCALE, height * SCALE))
            if stem == "red-arrow":
                shadow = Image.new("L", after.size)
                shadow.paste(
                    cutout.getchannel("A"),
                    ((bounds[0] + 2) * SCALE, (bounds[1] + 3) * SCALE),
                )
                shadow = shadow.filter(ImageFilter.GaussianBlur(SCALE * 1.05))
                shadow = shadow.point(lambda alpha: round(alpha * 0.58))
                after.paste((12, 8, 7, 255), (0, 0, *after.size), shadow)
            after.alpha_composite(cutout, (bounds[0] * SCALE, bounds[1] * SCALE))
            after.save(HERE / "after" / f"{stem}-{phase}.png")
            after.resize((width, height), Image.Resampling.LANCZOS).save(
                HERE / "game-size" / f"{stem}-{phase}.png"
            )

    canvas = Image.new("RGB", (1600, 8 * 220 + 68), "#f0ece5")
    draw = ImageDraw.Draw(canvas)
    title_font = ImageFont.load_default(size=27)
    label_font = ImageFont.load_default(size=19)
    draw.text(
        (30, 16),
        "Clonk navigation arrows - batch 6: original / high-resolution preview",
        fill="#292824",
        font=title_font,
    )
    for index, (stem, _, width, height) in enumerate(SHEETS):
        for phase in range(4):
            row = index * 4 + phase
            top = 64 + row * 220
            draw.rounded_rectangle((20, top, 1580, top + 202), 11, fill="#fffdfa")
            identifier = f"{'ARROW' if index == 0 else 'WOOD'}-{phase + 1:02}"
            draw.text((38, top + 14), f"{identifier} / phase {phase}", font=label_font, fill="#393832")
            before = Image.open(HERE / "before" / f"{stem}-{phase}.png").convert("RGBA")
            after = Image.open(HERE / "after" / f"{stem}-{phase}.png").convert("RGBA")
            before_scale = min(170 / before.width, 155 / before.height)
            after_scale = min(170 / after.width, 155 / after.height)
            before = before.resize(
                (round(before.width * before_scale), round(before.height * before_scale)),
                Image.Resampling.NEAREST,
            )
            after = after.resize(
                (round(after.width * after_scale), round(after.height * after_scale)),
                Image.Resampling.LANCZOS,
            )
            draw.text((400, top + 12), f"ORIGINAL {width} x {height}", font=label_font, fill="#65635c")
            draw.text((920, top + 12), f"PROPOSED {width * SCALE} x {height * SCALE}", font=label_font, fill="#65635c")
            canvas.paste(before, (400, top + 38), before)
            canvas.paste(after, (920, top + 38), after)
    canvas.save(HERE / "overview.png")


if __name__ == "__main__":
    main()
