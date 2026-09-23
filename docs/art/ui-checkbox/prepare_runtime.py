"""Validate and install the approved 8x checkbox states.

The flat box and the complete red mark were reviewed together. Keep the box
identical in every state, and derive the disabled mark with Rec. 709 luma.
The checked and disabled PNGs are retained beside the layers so the generated
runtime images can be checked against the exact artwork that was approved.
"""

from __future__ import annotations

from pathlib import Path
from shutil import copyfile

import numpy as np
from PIL import Image

HERE = Path(__file__).resolve().parent
OUT = HERE.parents[2] / "crates/clonk-app/assets/ui-icons"
SIZE = (256, 256)


def approved(name: str) -> Image.Image:
    image = Image.open(HERE / "approved" / name)
    if image.size != SIZE or image.mode != "RGBA":
        raise ValueError(f"{name} must be a {SIZE[0]}x{SIZE[1]} RGBA sprite")
    return image


def main() -> None:
    base = approved("checkbox-off.png")
    mark = approved("checkmark.png")
    checked = Image.alpha_composite(base, mark)
    if not np.array_equal(np.asarray(checked), np.asarray(approved("checkbox-on.png"))):
        raise ValueError("approved checked icon differs from the approved box and mark")

    gray_pixels = np.array(mark)
    gray = np.uint8(
        np.round(
            0.2126 * gray_pixels[..., 0]
            + 0.7152 * gray_pixels[..., 1]
            + 0.0722 * gray_pixels[..., 2]
        )
    )
    gray_pixels[..., :3] = gray[..., None]
    disabled = Image.alpha_composite(base, Image.fromarray(gray_pixels))
    if not np.array_equal(np.asarray(disabled), np.asarray(approved("checkbox-disabled.png"))):
        raise ValueError("approved disabled icon differs from the gray checkmark")

    for name in ("checkbox-off.png", "checkbox-on.png", "checkbox-disabled.png"):
        copyfile(HERE / "approved" / name, OUT / name)
    print("validated and installed the approved checkbox states")


if __name__ == "__main__":
    main()
