"""Prepare the runtime checkbox sprites from the approved checkmark and the
classic box.

`approved/` holds the checked and unchecked sprites approved on 2026-09-22.
They share one base, so the checkmark layer is recovered from the pair. Its
coverage is exact wherever the base is not opaque. Where its antialiased rim
covers the opaque base, the coverage is solved from the colour of the nearest
solidly covered checkmark pixel.

The box is the flat classic box of `GUICheckbox.png`, rebuilt at 8x. The
classic cell is a box over a pure black drop shadow. The shadow is fitted as
a blurred, offset box, which gives the box's own coverage and colour. Each
edge band keeps its colour along the edge, relative to the face, and the face
mottling is upsampled smoothly. Both checked states are composited over the
box. The disabled state converts only the checkmark to Rec. 709 luma.
"""

from __future__ import annotations

from pathlib import Path

import numpy as np
from PIL import Image
from scipy import ndimage, optimize
from scipy.special import erf

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
OUT = ROOT / "crates/clonk-app/assets/ui-icons"
CLASSIC = ROOT / "planet/Graphics.c4g/GUICheckbox.png"
SCALE = 8
SIZE = 32 * SCALE
# Classic pixels of the unchecked cell: those the box touches, its face, and
# the straight runs of its edges, as (left, top, right, bottom).
BOX = (4, 5, 26, 27)
FACE = (8, 9, 23, 24)
RUN_X, RUN_Y = slice(9, 22), slice(10, 23)


def load(path: Path) -> np.ndarray:
    return np.asarray(Image.open(path).convert("RGBA")).astype(float) / 255


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


def blurred_span(position: np.ndarray, low: float, high: float, sigma: float) -> np.ndarray:
    """Coverage of [low, high) blurred by a Gaussian, at position."""
    root = sigma * np.sqrt(2)
    return 0.5 * (erf((high - position) / root) - erf((low - position) / root))


def shadow_alpha(shadow: np.ndarray, xs: np.ndarray, ys: np.ndarray) -> np.ndarray:
    left, top, right, bottom, sigma, peak = shadow
    return peak * blurred_span(xs, left, right, sigma) * blurred_span(ys, top, bottom, sigma)


def fit_shadow(cell: np.ndarray) -> np.ndarray:
    """The drop shadow as a blurred box, fitted to the pure black shadow
    pixels and the empty pixels around the box."""
    black = (cell[..., :3].max(axis=2) == 0) & (cell[..., 3] > 0)
    ys, xs = np.nonzero(black | (cell[..., 3] == 0))
    target = cell[ys, xs, 3]
    # Accelerate's matrix products raise spurious floating-point flags on
    # macOS; the fit itself is finite.
    with np.errstate(divide="ignore", over="ignore", invalid="ignore"):
        return optimize.least_squares(
            lambda shadow: shadow_alpha(shadow, xs + 0.5, ys + 0.5) - target,
            x0=[5.5, 6.5, 27.5, 28.5, 1.5, 0.8],
        ).x


def upsample(face: np.ndarray) -> np.ndarray:
    height, width = face.shape[:2]
    channels = (
        Image.fromarray(face[..., channel].astype(np.float32)).resize(
            (width * SCALE, height * SCALE), Image.Resampling.BICUBIC
        )
        for channel in range(3)
    )
    return np.stack([np.asarray(channel) for channel in channels], axis=2)


def classic_box(cell: np.ndarray) -> np.ndarray:
    """The classic unchecked box and its drop shadow, rebuilt at SCALE."""
    shadow = fit_shadow(cell)
    centres_y, centres_x = np.mgrid[0:32, 0:32] + 0.5
    beneath = shadow_alpha(shadow, centres_x, centres_y)
    # alpha = coverage + (1 - coverage) * shadow; the shadow adds no colour.
    body = cell[..., :3].max(axis=2) > 0
    coverage = np.where(body, np.clip((cell[..., 3] - beneath) / (1 - beneath), 0, 1), 0)
    colour = premultiply(cell) / np.maximum(coverage[..., None], 1e-9)

    left_pixel, top_pixel, right_pixel, bottom_pixel = BOX
    face_left, face_top, face_right, face_bottom = FACE
    left = left_pixel + 1 - coverage[RUN_Y, left_pixel].mean()
    right = right_pixel - 1 + coverage[RUN_Y, right_pixel - 1].mean()
    top = top_pixel + 1 - coverage[top_pixel, RUN_X].mean()
    bottom = bottom_pixel - 1 + coverage[bottom_pixel - 1, RUN_X].mean()

    face = colour[face_top:face_bottom, face_left:face_right]
    level = np.median(face.reshape(-1, 3), axis=0)
    rows, columns = np.ones((32, 3)), np.ones((32, 3))
    for y in (*range(top_pixel, face_top), *range(face_bottom, bottom_pixel)):
        rows[y] = np.median(colour[y, RUN_X], axis=0) / level
    for x in (*range(left_pixel, face_left), *range(face_right, right_pixel)):
        columns[x] = np.median(colour[RUN_Y, x], axis=0) / level
    row_factor = np.repeat(np.repeat(rows, SCALE, axis=0)[:, None], SIZE, axis=1)
    column_factor = np.repeat(np.repeat(columns, SCALE, axis=0)[None], SIZE, axis=0)
    # The left bands start below the top highlight rows, and the top inset
    # rows start at the left inset line, as in the classic box.
    column_factor[: (top_pixel + 2) * SCALE, : face_left * SCALE] = 1
    row_factor[(top_pixel + 2) * SCALE : face_top * SCALE, : (left_pixel + 2) * SCALE] = 1

    texture = upsample(face)
    pixel = np.arange(SIZE)
    texture_rows = np.clip(pixel - face_top * SCALE, 0, texture.shape[0] - 1)
    texture_columns = np.clip(pixel - face_left * SCALE, 0, texture.shape[1] - 1)
    box_colour = np.clip(texture[texture_rows][:, texture_columns] * row_factor * column_factor, 0, 1)

    def span(low: float, high: float) -> np.ndarray:
        return np.clip(np.minimum(pixel + 1, high * SCALE) - np.maximum(pixel, low * SCALE), 0, 1)

    box_coverage = span(top, bottom)[:, None] * span(left, right)[None, :]
    centres_y, centres_x = np.mgrid[0:SIZE, 0:SIZE] + 0.5
    scaled_shadow = np.array([*shadow[:5] * SCALE, shadow[5]])
    alpha = box_coverage + (1 - box_coverage) * shadow_alpha(scaled_shadow, centres_x, centres_y)
    straight = box_colour * box_coverage[..., None] / np.maximum(alpha[..., None], 1e-9)
    return np.concatenate([straight, alpha[..., None]], axis=2)


def over(colour: np.ndarray, coverage: np.ndarray, base: np.ndarray) -> np.ndarray:
    alpha = coverage + base[..., 3] * (1 - coverage)
    out = colour + premultiply(base) * (1 - coverage[..., None])
    straight = out / np.maximum(alpha[..., None], 1e-9)
    return np.concatenate([straight, alpha[..., None]], axis=2)


def quantize(rgba: np.ndarray) -> np.ndarray:
    pixels = np.clip(np.round(rgba * 255), 0, 255).astype(np.uint8)
    pixels[pixels[..., 3] == 0] = 0
    return pixels


def save(rgba: np.ndarray, name: str) -> None:
    Image.fromarray(quantize(rgba)).save(OUT / name, optimize=True)


def main() -> None:
    checked = load(HERE / "approved/checkbox-on.png")
    unchecked = load(HERE / "approved/checkbox-off.png")
    colour, coverage = recover_checkmark(checked, unchecked)
    # Quantized first, so every uncovered pixel of the checked states matches
    # the unchecked sprite exactly.
    base = quantize(classic_box(load(CLASSIC)[:, :32])).astype(float) / 255
    gray = 0.2126 * colour[..., 0] + 0.7152 * colour[..., 1] + 0.0722 * colour[..., 2]
    save(base, "checkbox-off.png")
    save(over(colour, coverage, base), "checkbox-on.png")
    save(over(np.repeat(gray[..., None], 3, axis=2), coverage, base), "checkbox-disabled.png")


if __name__ == "__main__":
    main()
