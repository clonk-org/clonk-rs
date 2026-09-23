"""Redraw the checkbox runtime tiles of `ui-icons-sprites.png` from the
runtime sprites. Each tile shows its sprite over the panel beige, filling the
rounded panel with bicubic filtering, as the overview draws every runtime
sprite."""

from pathlib import Path

from PIL import Image, ImageDraw

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
OVERVIEW = HERE.parent / "ui-icons-sprites.png"
CARD = (255, 255, 255, 255)
PANEL = (238, 232, 220, 255)
RADIUS = 5
SUPERSAMPLE = 8
# Runtime sprite panels of UI-14 Checkbox unchecked and UI-15 Checkbox checked.
TILES = {
    "checkbox-off.png": (726, 1345, 932, 1552),
    "checkbox-on.png": (1195, 1345, 1401, 1552),
}


def panel_mask(size: tuple[int, int]) -> Image.Image:
    width, height = size
    large = Image.new("L", (width * SUPERSAMPLE, height * SUPERSAMPLE), 0)
    ImageDraw.Draw(large).rounded_rectangle(
        (0, 0, width * SUPERSAMPLE - 1, height * SUPERSAMPLE - 1),
        radius=RADIUS * SUPERSAMPLE,
        fill=255,
    )
    return large.resize(size, Image.Resampling.BOX)


def main() -> None:
    overview = Image.open(OVERVIEW).convert("RGBA")
    for name, (left, top, right, bottom) in TILES.items():
        size = (right - left, bottom - top)
        sprite = Image.open(ROOT / "crates/clonk-app/assets/ui-icons" / name).convert("RGBa")
        tile = Image.new("RGBA", size, PANEL)
        tile.alpha_composite(sprite.resize(size, Image.Resampling.BICUBIC).convert("RGBA"))
        overview.paste(Image.new("RGBA", size, CARD), (left, top))
        overview.paste(tile, (left, top), panel_mask(size))
    overview.convert("RGB").save(OVERVIEW, optimize=True)


if __name__ == "__main__":
    main()
