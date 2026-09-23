"""Fit the approved gamepad previews into the game's 8x cell format."""

from pathlib import Path

from PIL import Image

HERE = Path(__file__).resolve().parent
OUT = HERE.parents[2] / "crates/clonk-app/assets/gamepad-icons"


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    for number in range(1, 5):
        preview = Image.open(HERE / f"after/gamepad-{number}.png").convert("RGBA")
        assert preview.size == (1280, 576)
        art = preview.resize((640, 288), Image.Resampling.LANCZOS)
        art.save(OUT / f"gamepad-{number}.png", optimize=True)


if __name__ == "__main__":
    main()
