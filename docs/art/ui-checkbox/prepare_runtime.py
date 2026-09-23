"""Prepare the runtime checkbox sprites from the approved checkbox artwork.

`approved/` holds the checked and unchecked sprites approved on 2026-09-22.
They share one base, so the checkmark layer is recovered from the pair. Its
coverage is exact wherever the base is not opaque. Where its antialiased rim
covers the opaque base, the coverage is solved from the colour of the nearest
solidly covered checkmark pixel. The base is fitted to the classic box bounds
at 8x, and both checked states are composited over it again. The disabled
state converts only the checkmark to Rec. 709 luma.
"""

from __future__ import annotations

from pathlib import Path

import numpy as np
from PIL import Image
from scipy import ndimage

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
OUT = ROOT / "crates/clonk-app/assets/ui-icons"
CLASSIC = ROOT / "planet/Graphics.c4g/GUICheckbox.png"
SCALE = 8
SIZE = 32 * SCALE
# Resampled margin around the box, for its antialiased edge.
MARGIN = 6


def load(path: Path) -> np.ndarray:
    return np.asarray(Image.open(path).convert("RGBA")).astype(float) / 255


def box_bounds(rgba: np.ndarray) -> tuple[int, int, int, int]:
    """Bounds of the opaque, non-black pixels: the box without the classic
    box's pure black drop shadow. `hd_ui_icons.rs` applies the same rule."""
    inside = (rgba[..., 3] >= 128 / 255) & (rgba[..., :3].max(axis=2) >= 8 / 255)
    ys, xs = np.nonzero(inside)
    return int(xs.min()), int(ys.min()), int(xs.max()) + 1, int(ys.max()) + 1


def premultiply(rgba: np.ndarray) -> np.ndarray:
    return rgba[..., :3] * rgba[..., 3:4]


def recover_checkmark(checked: np.ndarray, unchecked: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    """Premultiplied colour and coverage of the checkmark composited over the
    unchecked base in the approved checked sprite."""
    covered = np.any(checked != unchecked, axis=2)
    solid = ndimage.binary_erosion(covered, iterations=2)
    _, nearest = ndimage.distance_transform_edt(~solid, return_indices=True)
    rim = checked[..., :3][nearest[0], nearest[1]]
    out, base = premultiply(checked), premultiply(unchecked)
    # out = coverage * rim + base * (1 - coverage), in the least-squares sense.
    delta = rim - base
    spread = (delta**2).sum(axis=2)
    estimate = ((out - base) * delta).sum(axis=2) / np.maximum(spread, 1e-9)
    estimate = np.where(spread > (40 / 255) ** 2, estimate, 1)
    # Coverage below these bounds would need a colour outside [0, 1].
    brighter = np.where(out > base, (out - base) / np.maximum(1 - base, 1e-9), 0).max(axis=2)
    darker = np.where(base > 0, 1 - out / np.maximum(base, 1e-9), 0).max(axis=2)
    from_colour = np.where(solid, 1, np.clip(np.maximum(estimate, np.maximum(brighter, darker)), 0, 1))
    base_alpha = unchecked[..., 3]
    from_alpha = np.clip((checked[..., 3] - base_alpha) / np.maximum(1 - base_alpha, 1e-9), 0, 1)
    coverage = np.where(covered, np.where(base_alpha <= 0.9, from_alpha, from_colour), 0)
    colour = np.clip(out - base * (1 - coverage[..., None]), 0, coverage[..., None])
    return colour, coverage


def fit_base(unchecked: Path, source: tuple[int, ...], target: tuple[int, ...]) -> np.ndarray:
    """Resample the approved base so its box spans the target bounds."""
    left, top, right, bottom = source
    x0, y0, x1, y1 = target
    scale_x, scale_y = (x1 - x0) / (right - left), (y1 - y0) / (bottom - top)
    region = (
        left - MARGIN / scale_x,
        top - MARGIN / scale_y,
        right + MARGIN / scale_x,
        bottom + MARGIN / scale_y,
    )
    size = (x1 - x0 + 2 * MARGIN, y1 - y0 + 2 * MARGIN)
    fitted = Image.open(unchecked).convert("RGBa").resize(size, Image.Resampling.LANCZOS, box=region)
    base = Image.new("RGBa", (SIZE, SIZE))
    base.paste(fitted, (x0 - MARGIN, y0 - MARGIN))
    return np.asarray(base.convert("RGBA")).astype(float) / 255


def over(colour: np.ndarray, coverage: np.ndarray, base: np.ndarray) -> np.ndarray:
    alpha = coverage + base[..., 3] * (1 - coverage)
    out = colour + premultiply(base) * (1 - coverage[..., None])
    straight = out / np.maximum(alpha[..., None], 1e-9)
    return np.concatenate([straight, alpha[..., None]], axis=2)


def save(rgba: np.ndarray, name: str) -> None:
    pixels = np.clip(np.round(rgba * 255), 0, 255).astype(np.uint8)
    pixels[pixels[..., 3] == 0] = 0
    Image.fromarray(pixels).save(OUT / name, optimize=True)


def main() -> None:
    checked = load(HERE / "approved/checkbox-on.png")
    unchecked = load(HERE / "approved/checkbox-off.png")
    colour, coverage = recover_checkmark(checked, unchecked)
    classic = box_bounds(load(CLASSIC)[:, :32])
    target = tuple(bound * SCALE for bound in classic)
    base = fit_base(HERE / "approved/checkbox-off.png", box_bounds(unchecked), target)
    gray = 0.2126 * colour[..., 0] + 0.7152 * colour[..., 1] + 0.0722 * colour[..., 2]
    save(base, "checkbox-off.png")
    save(over(colour, coverage, base), "checkbox-on.png")
    save(over(np.repeat(gray[..., None], 3, axis=2), coverage, base), "checkbox-disabled.png")
    print(f"classic box {classic}; approved box {box_bounds(unchecked)} fitted to {target}")


if __name__ == "__main__":
    main()
