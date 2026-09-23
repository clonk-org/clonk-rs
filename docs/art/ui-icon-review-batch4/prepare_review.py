"""Prepare one imagegen controller and four matching numbered review sprites."""

from __future__ import annotations

from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFont
from scipy import ndimage

HERE = Path(__file__).resolve().parent
SOURCE = HERE.parents[2] / "planet/Graphics.c4g/Gamepad.png"
GENERATED = HERE / "support/controller-generated-opaque.png"
SIZE = (1280, 576)


def controller_cutout() -> Image.Image:
    rgb = np.asarray(Image.open(GENERATED).convert("RGB"))
    # The imagegen output contains a baked checkerboard. Its dark outline is a
    # closed component, so the largest dark component gives the controller
    # silhouette without modifying any artwork inside it.
    dark = np.max(rgb, axis=2) < 120
    components, _ = ndimage.label(dark)
    sizes = np.bincount(components.ravel())
    sizes[0] = 0
    silhouette = ndimage.binary_fill_holes(components == sizes.argmax())
    alpha = ndimage.gaussian_filter(
        ndimage.binary_dilation(silhouette).astype(np.float32), 0.7
    )
    alpha = (np.clip(alpha, 0, 1) * 255).astype(np.uint8)
    ys, xs = np.nonzero(alpha > 4)
    rgba = Image.fromarray(np.dstack((rgb, alpha)), "RGBA")
    return rgba.crop((int(xs.min()), int(ys.min()), int(xs.max()) + 1, int(ys.max()) + 1))


def original_digit_bounds(cell: Image.Image) -> tuple[int, int, int, int]:
    pixels = np.asarray(cell.convert("RGBA")).astype(np.int16)
    red, green, blue, alpha = [pixels[:, :, channel] for channel in range(4)]
    mask = (
        (red > 65)
        & (red > green * 1.25 + 20)
        & (red > blue * 1.25 + 20)
        & (alpha > 100)
    )
    mask[:18] = False
    mask[:, :56] = False
    components, _ = ndimage.label(mask)
    sizes = np.bincount(components.ravel())
    sizes[0] = 0
    ys, xs = np.nonzero(components == sizes.argmax())
    return int(xs.min()), int(ys.min()), int(xs.max()) + 1, int(ys.max()) + 1


def polished_digit(number: int) -> tuple[Image.Image, tuple[int, int, int, int]]:
    rgb = np.asarray(
        Image.open(HERE / f"support/digit-{number}-generated-opaque.png").convert("RGB")
    )
    red, green, blue = [rgb[:, :, channel].astype(np.int16) for channel in range(3)]
    # Only the lower-right numeral is used from each imagegen edit. Its red
    # material identifies it independently of the regenerated controller.
    red_material = (
        (red > 45)
        & (red > green * 1.18 + 10)
        & (red > blue * 1.18 + 10)
    )
    red_material[:350] = False
    red_material[:, :1200] = False
    components, _ = ndimage.label(red_material)
    sizes = np.bincount(components.ravel())
    sizes[0] = 0
    digit = components == sizes.argmax()
    ys, xs = np.nonzero(digit)
    bounds = (int(xs.min()), int(ys.min()), int(xs.max()) + 1, int(ys.max()) + 1)
    margin = 12
    box = (
        bounds[0] - margin,
        bounds[1] - margin,
        bounds[2] + margin,
        bounds[3] + margin,
    )
    digit = digit[box[1] : box[3], box[0] : box[2]]
    crop = rgb[box[1] : box[3], box[0] : box[2]]
    holes, count = ndimage.label(ndimage.binary_fill_holes(digit) & ~digit)
    for component in range(1, count + 1):
        # The model's glossy near-white highlights sit inside the red paint.
        # Keep the dark triangular counter of the 4 transparent.
        pixels = crop[holes == component]
        if pixels.size and pixels[:, 0].mean() > 150:
            digit[holes == component] = True
    alpha = (np.clip(ndimage.gaussian_filter(digit.astype(float), 1.2), 0, 1) * 255).astype(np.uint8)
    return Image.fromarray(np.dstack((crop, alpha)), "RGBA"), bounds


def numbered(base: Image.Image, cell: Image.Image, number: int) -> Image.Image:
    image = base.copy()
    digit, model_bounds = polished_digit(number)
    left, top, right, bottom = original_digit_bounds(cell)
    scale_x = (right - left) * 16 / (model_bounds[2] - model_bounds[0])
    scale_y = (bottom - top) * 16 / (model_bounds[3] - model_bounds[1])
    digit = digit.resize(
        (round(digit.width * scale_x), round(digit.height * scale_y)),
        Image.Resampling.LANCZOS,
    )
    image.alpha_composite(
        digit,
        (round(left * 16 - 12 * scale_x), round(top * 16 - 12 * scale_y)),
    )
    return image


def write_overview() -> None:
    canvas = Image.new("RGB", (1060, 970), "#f5f1e9")
    draw = ImageDraw.Draw(canvas)
    font_dir = Path("/System/Library/Fonts/Supplemental")
    regular = ImageFont.truetype(str(font_dir / "Arial.ttf"), 23)
    bold = ImageFont.truetype(str(font_dir / "Arial Bold.ttf"), 27)
    draw.text((28, 20), "Clonk gamepad icons — batch 4", font=bold, fill="#302b23")
    draw.text((190, 69), "Original 80×36", font=regular, fill="#6f665b")
    draw.text((635, 69), "High-resolution preview", font=regular, fill="#6f665b")
    for number in range(1, 5):
        top = 110 + (number - 1) * 208
        if number % 2:
            draw.rounded_rectangle((14, top - 4, 1046, top + 194), 11, fill="#fffcf6")
        draw.text((28, top + 69), f"PAD-0{number}", font=regular, fill="#8c785a")
        for x, source, resample in (
            (190, HERE / f"before/gamepad-{number}.png", Image.Resampling.NEAREST),
            (635, HERE / f"after/gamepad-{number}.png", Image.Resampling.LANCZOS),
        ):
            image = Image.open(source).convert("RGBA").resize((400, 180), resample)
            draw.rounded_rectangle((x - 5, top + 2, x + 405, top + 186), 8, fill="#e9e0d1")
            canvas.paste(image, (x, top + 4), image)
        if number < 4:
            draw.line((27, top + 202, 1035, top + 202), fill="#e0d7ca")
    canvas.save(HERE / "overview.png", optimize=True)


def main() -> None:
    sheet = Image.open(SOURCE).convert("RGBA")
    (HERE / "game-size").mkdir(exist_ok=True)
    cutout = controller_cutout().resize((1160, 520), Image.Resampling.LANCZOS)
    base = Image.new("RGBA", SIZE)
    base.alpha_composite(cutout, (30, 15))
    base.save(HERE / "support/controller-base.png", optimize=True)
    for number in range(1, 5):
        cell = sheet.crop(((number - 1) * 80, 0, number * 80, 36))
        cell.save(HERE / f"before/gamepad-{number}.png", optimize=True)
        preview = numbered(base, cell, number)
        preview.save(HERE / f"after/gamepad-{number}.png", optimize=True)
        preview.resize((80, 36), Image.Resampling.LANCZOS).save(
            HERE / f"game-size/gamepad-{number}.png", optimize=True
        )
    write_overview()


if __name__ == "__main__":
    main()
