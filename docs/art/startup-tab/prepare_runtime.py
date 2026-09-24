"""Fit the selected imagegen tab to the classic silhouette at 8x resolution."""

from pathlib import Path

from PIL import Image, ImageFilter


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
CLASSIC = ROOT / "planet/Graphics.c4g/StartupTabClip.png"
GENERATED = HERE / "generated.png"
RUNTIME = ROOT / "crates/clonk-app/assets/StartupTabClipHD.png"
SOURCE_SIZE = (120, 80)
GENERATED_SIZE = (1536, 1024)
RUNTIME_SIZE = (960, 640)


def main() -> None:
    source = Image.open(CLASSIC).convert("RGBA")
    generated = Image.open(GENERATED).convert("RGB")
    if source.size != SOURCE_SIZE or generated.size != GENERATED_SIZE:
        raise ValueError("the tab source or selected imagegen output changed size")

    # The model added an opaque backdrop. The source alpha fixes the complete
    # paper, hinge, and loop silhouette, including the open centre of the loop.
    # Thresholding away the source's old shadow keeps the generated backdrop
    # out of antialiased edges; a restrained 8x shadow is added separately.
    core = source.getchannel("A").resize(RUNTIME_SIZE, Image.Resampling.LANCZOS)
    core = core.point(lambda value: 255 if value >= 210 else 0)
    core = core.filter(ImageFilter.GaussianBlur(1.4))

    tab = generated.resize(RUNTIME_SIZE, Image.Resampling.LANCZOS).convert("RGBA")
    tab.putalpha(core)
    shadow_mask = Image.new("L", RUNTIME_SIZE)
    shadow_mask.paste(core, (4, 6))
    shadow_mask = shadow_mask.filter(ImageFilter.GaussianBlur(9))
    shadow_mask = shadow_mask.point(lambda value: round(value * 0.55))
    shadow = Image.new("RGBA", RUNTIME_SIZE)
    shadow.putalpha(shadow_mask)
    Image.alpha_composite(shadow, tab).save(RUNTIME, optimize=True)


if __name__ == "__main__":
    main()
