"""Fit generated hand cutouts to the original seven-cell Hand.png layout."""

from __future__ import annotations

from pathlib import Path

import cv2
import numpy as np
from PIL import Image, ImageDraw, ImageFont

HERE = Path(__file__).resolve().parent
SOURCE = HERE.parents[2] / "planet/Graphics.c4g/Hand.png"
SCALE = 8


def transparent_hand(path: Path) -> Image.Image:
    generated = Image.open(path)
    if generated.mode == "RGBA" and generated.getchannel("A").getextrema()[0] < 255:
        return generated

    rgb = np.asarray(generated.convert("RGB"))
    red, green, blue = (rgb[:, :, channel].astype(np.int16) for channel in range(3))
    candidate = ((red - green > 18) & (green - blue > 7) & (red > 90)).astype(np.uint8)
    count, labels, stats, _ = cv2.connectedComponentsWithStats(candidate, 8)
    assert count > 1, path
    largest = 1 + np.argmax(stats[1:, cv2.CC_STAT_AREA])
    foreground = (labels == largest).astype(np.uint8)
    contours, _ = cv2.findContours(foreground, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)
    cv2.drawContours(foreground, contours, -1, 1, cv2.FILLED)

    # The generator rendered a gray checkerboard despite the alpha instruction.
    # Carry the closest interior skin color into antialiased edge pixels.
    interior = cv2.erode(foreground, np.ones((5, 5), np.uint8))
    _, nearest = cv2.distanceTransformWithLabels(
        1 - interior, cv2.DIST_L2, 5, labelType=cv2.DIST_LABEL_PIXEL
    )
    palette = np.zeros((nearest.max() + 1, 3), np.uint8)
    palette[nearest[interior > 0]] = rgb[interior > 0]
    color = palette[nearest]
    alpha = cv2.GaussianBlur(foreground.astype(np.float32), (0, 0), 1.1)
    alpha = np.uint8(np.clip(alpha * 255, 0, 255))
    return Image.fromarray(np.dstack((color, alpha)))


def main() -> None:
    source = Image.open(SOURCE).convert("RGBA")
    assert source.size == (448, 64)
    for phase in range(7):
        before = source.crop((phase * 64, 0, (phase + 1) * 64, 64))
        before.save(HERE / "before" / f"hand-{phase}.png")
        bounds = before.getchannel("A").getbbox()
        assert bounds is not None
        cutout = transparent_hand(HERE / "support" / f"hand-{phase}-generated.png")
        visible = cutout.getchannel("A").point(lambda alpha: 255 if alpha >= 8 else 0)
        cutout = cutout.crop(visible.getbbox())
        destination = (
            (bounds[2] - bounds[0]) * SCALE,
            (bounds[3] - bounds[1]) * SCALE,
        )
        cutout = cutout.resize(destination, Image.Resampling.LANCZOS)
        after = Image.new("RGBA", (512, 512))
        after.alpha_composite(cutout, (bounds[0] * SCALE, bounds[1] * SCALE))
        after.save(HERE / "after" / f"hand-{phase}.png")
        after.resize((64, 64), Image.Resampling.LANCZOS).save(
            HERE / "game-size" / f"hand-{phase}.png"
        )

    width, height = 1540, 7 * 218 + 70
    overview = Image.new("RGB", (width, height), "#efede7")
    draw = ImageDraw.Draw(overview)
    title_font = ImageFont.load_default(size=26)
    label_font = ImageFont.load_default(size=18)
    draw.text(
        (38, 18),
        "Clonk hand gestures — batch 5: original / high-resolution preview",
        font=title_font,
        fill="#242725",
    )
    for phase in range(7):
        top = 66 + phase * 218
        draw.rounded_rectangle((24, top, width - 24, top + 198), radius=12, fill="#fffdf8")
        draw.text(
            (42, top + 12),
            f"HAND-{phase + 1:02} · phase {phase}",
            font=label_font,
            fill="#393936",
        )
        before = Image.open(HERE / "before" / f"hand-{phase}.png").convert("RGBA")
        after = Image.open(HERE / "after" / f"hand-{phase}.png").convert("RGBA")
        before = before.resize((160, 160), Image.Resampling.NEAREST)
        after = after.resize((160, 160), Image.Resampling.LANCZOS)
        draw.text((388, top + 12), "ORIGINAL 64 × 64", font=label_font, fill="#66645e")
        draw.text((898, top + 12), "PROPOSED 512 × 512", font=label_font, fill="#66645e")
        overview.paste(before, (370, top + 34), before)
        overview.paste(after, (884, top + 34), after)
    overview.save(HERE / "overview.png")


if __name__ == "__main__":
    main()
