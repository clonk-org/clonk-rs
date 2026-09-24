"""Pack approved smooth slider facets into an 8x book-scroll atlas."""

from pathlib import Path

from PIL import Image, ImageOps


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
CLASSIC = ROOT / "planet/Graphics.c4g/StartupBookScroll.png"
OUTPUT = ROOT / "crates/clonk-app/assets/StartupBookScrollHD.png"
CELL = 128


def selected_image(name: str) -> Image.Image:
    image = Image.open(HERE / name).convert("RGBA")
    if image.size != (1254, 1254):
        raise ValueError(f"{name} must be the selected 1254x1254 imagegen result")
    return image


def arrow() -> Image.Image:
    generated = selected_image("left-arrow-generated.png")
    # The output has genuine alpha, but near-invisible pixels around its
    # silhouette make its bounding box much larger than the visible arrow.
    visible = generated.getchannel("A").point(lambda value: 255 if value > 64 else 0)
    left, top, right, bottom = visible.getbbox()
    generated = generated.crop((left - 8, top - 8, right + 8, bottom + 8))
    alpha = generated.getchannel("A").point(lambda value: 0 if value < 16 else value)
    generated.putalpha(alpha)
    arrow = Image.new("RGBA", (CELL, CELL))
    arrow.alpha_composite(generated.resize((104, 104), Image.Resampling.LANCZOS), (24, 16))
    # The generated tail ends in a rounded cap. Open that cap into a straight
    # cross-section so the repeated middle is literally the same material.
    tail = arrow.crop((112, 0, 113, CELL))
    for x in range(112, CELL):
        arrow.paste(tail, (x, 0))
    return arrow


def rail(arrow_head: Image.Image) -> Image.Image:
    tail = arrow_head.crop((112, 0, 113, CELL))
    strip = Image.new("RGBA", (CELL, CELL))
    for x in range(CELL):
        strip.paste(tail, (x, 0))
    return strip


def pressed(image: Image.Image) -> Image.Image:
    red, green, blue, alpha = image.split()
    return Image.merge(
        "RGBA",
        (
            red.point(lambda value: min(255, round(value * 1.13 + 8))),
            green.point(lambda value: min(255, round(value * 1.04 + 4))),
            blue.point(lambda value: round(value * 0.78)),
            alpha,
        ),
    )


def main() -> None:
    source = Image.open(CLASSIC).convert("RGBA")
    if source.size != (48, 48):
        raise ValueError("the classic book-scroll atlas changed size")
    atlas = Image.new("RGBA", (3 * CELL, 3 * CELL))
    for row in range(3):
        for column in range(3):
            original = source.crop(
                (column * 16, row * 16, (column + 1) * 16, (row + 1) * 16)
            )
            atlas.paste(
                original.resize((CELL, CELL), Image.Resampling.LANCZOS),
                (column * CELL, row * CELL),
            )

    left = arrow()
    right = ImageOps.mirror(left)
    facets = {
        (0, 0): left,
        (1, 0): pressed(left),
        (0, 1): rail(left),
        (0, 2): right,
        (1, 2): pressed(right),
    }
    for (column, row), horizontal in facets.items():
        # DrawHBarByVGfx rotates vertical source facets 90 degrees CCW.
        atlas.paste(horizontal.rotate(270, expand=False), (column * CELL, row * CELL))
    atlas.save(OUTPUT, optimize=True)


if __name__ == "__main__":
    main()
