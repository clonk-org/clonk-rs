#!/usr/bin/env python3
"""Splice a high-resolution sprite pack into a shipped definition folder.

    install_hd_definition_pack.py <shipped.c4d> <hd-pack-dir> <out.c4d>

A pack rendered from the 3D crew rigs carries only `Name`, `Length` and
`Facet` per action. Everything that makes an action *behave* -- `Procedure`,
`Directions`, `FlipDir`, `Delay`, `NextAction`, `StartCall`, `Reverse`,
`InLiquidAction` -- exists only in the shipped `ActMap.txt`, so installing a
pack is a splice and never a replace.

The facet mapping is keyed on the SHIPPED FACET RECT rather than the action
name. Several shipped actions deliberately share one rect, which is how the
original sprite compositor authored them:

    ScaleDown = the Scale rect, played backwards (`Reverse=1`)
    Bridge    = the Dig rect
    RideStill = the Ride rect, held for one frame

Keying on the rect carries those aliases across for free. Keying on the name
would leave every unnamed alias pointing at coordinates that mean nothing on
the new sheet -- at `Scale=300` the shipped Scale row's `0,20` lands in the
middle of the high-resolution Walk row.

`Scale` and the `Picture` rect move to the pack's values: the rendered rows
are taller than the shipped ones, so the picture sits elsewhere on the sheet.
"""

import argparse
import re
import shutil
import struct
import sys
from pathlib import Path

ENCODING = "latin-1"


def png_size(path):
    """Width and height from the IHDR, without decoding the image."""
    header = path.read_bytes()[16:24]
    return struct.unpack(">II", header)


def parse_actions(text):
    """[(name, facet)] in file order, skipping actions without a facet."""
    actions = []
    for block in text.split("[Action]")[1:]:
        name = re.search(r"^Name=(.+)$", block, re.M)
        facet = re.search(r"^Facet=(.+)$", block, re.M)
        if name and facet:
            actions.append((name.group(1).strip(), facet.group(1).strip()))
    return actions


def facet_rect_map(shipped_actions, hd_actions):
    """Shipped facet rect -> high-resolution facet rect.

    Built from the actions both files name, so alias actions the pack never
    mentions still resolve through the rect they share.
    """
    shipped_rect_of = dict(shipped_actions)
    return {
        shipped_rect_of[name]: hd_facet
        for name, hd_facet in hd_actions.items()
        if name in shipped_rect_of
    }


def action_strips(text):
    """[(name, facet, length, rows)] — enough to locate an action's whole strip.

    `Directions=2` normally stacks a second row of frames, but `FlipDir=1`
    mirrors the first row instead and consumes no extra row.
    """
    strips = []
    for blk in text.split("[Action]")[1:]:
        name = re.search(rf"^Name=({VALUE})", blk, re.M)
        facet = re.search(rf"^Facet=({VALUE})", blk, re.M)
        if not (name and facet):
            continue

        def num(key, default=1):
            m = re.search(rf"^{key}=(-?\d+)", blk, re.M)
            return int(m.group(1)) if m else default

        rows = 1 if num("FlipDir", 0) else max(num("Directions", 1), 1)
        strips.append((name.group(1).strip(), facet.group(1).strip(), max(num("Length", 1), 1), rows))
    return strips


def reflow_auxiliary_sheet(source, target, strips, rect_map, scale):
    """Re-lay an auxiliary sheet onto the new facet layout.

    A definition can carry extra sheets besides Graphics.png -- the Knights'
    `GraphicsShield.png` is an ExtraGraphics overlay drawn through the CLONK's
    own definition, so it inherits that definition's `Scale`. Replacing the
    base art without touching it leaves its cells at the old coordinates while
    every facet now points at the new ones, and at `Scale=300` it reads far
    outside the sheet.

    Re-flowing copies each action's strip from its old rect to its new one,
    scaled to match. The art is upscaled rather than re-rendered, so it is not
    truly high-resolution -- but it lines up, which is the difference between
    "not remastered yet" and "renders garbage".
    """
    from PIL import Image  # imported lazily: CI runs the tests without Pillow

    old = Image.open(source).convert("RGBA")
    canvas = Image.new("RGBA", target[1], (0, 0, 0, 0))
    placed = 0
    for _name, facet, length, rows in strips:
        if facet not in rect_map:
            continue
        x, y, w, h = (int(v) for v in facet.split(",")[:4])
        nx, ny, nw, nh = (int(v) for v in rect_map[facet].split(",")[:4])
        src = old.crop((x, y, x + w * length, y + h * rows))
        if src.width == 0 or src.height == 0:
            continue
        dst = (nw * length * scale // 100, nh * rows * scale // 100)
        if dst[0] <= 0 or dst[1] <= 0:
            continue
        canvas.paste(src.resize(dst, Image.NEAREST), (nx * scale // 100, ny * scale // 100))
        placed += 1
    canvas.save(target[0])
    return placed


def newline_of(text):
    """The file's own line ending, so a rewrite does not convert it."""
    return "\r\n" if "\r\n" in text else "\n"


# `.+$` in MULTILINE swallows the CR of a CRLF line, so replacing a match
# drops it and silently converts that one line to LF. Matching the value with
# an explicit no-newline class leaves the terminator untouched.
VALUE = r"[^\r\n]*"


def splice_act_map(shipped_text, rect_map):
    """Rewrite every `Facet=` the map covers; leave all other keys alone."""

    def rewrite(block):
        current = re.search(rf"^Facet=({VALUE})", block, re.M)
        if not current or current.group(1).strip() not in rect_map:
            return block
        return re.sub(
            rf"^Facet={VALUE}",
            "Facet=" + rect_map[current.group(1).strip()],
            block,
            count=1,
            flags=re.M,
        )

    head, *blocks = shipped_text.split("[Action]")
    return head + "".join("[Action]" + rewrite(block) for block in blocks)


def splice_def_core(shipped_core, hd_core):
    """Adopt the pack's `Scale` and `Picture`, keeping every other field."""
    scale = re.search(rf"^Scale=(\d+){VALUE}", hd_core, re.M)
    if not scale:
        raise ValueError("high-resolution pack DefCore.txt has no Scale=")
    picture = re.search(rf"^Picture=({VALUE})", hd_core, re.M)
    core = shipped_core
    newline = newline_of(shipped_core)
    if picture:
        core = re.sub(
            rf"^Picture={VALUE}",
            "Picture=" + picture.group(1).strip(),
            core,
            count=1,
            flags=re.M,
        )
    # Scale belongs to [DefCore]; anchor it to Picture so it cannot land in a
    # later section such as [Physical].
    core = re.sub(
        rf"^(Picture={VALUE})",
        lambda m: m.group(1) + newline + "Scale=" + scale.group(1),
        core,
        count=1,
        flags=re.M,
    )
    return core, int(scale.group(1))


def out_of_bounds(actions, sheet, scale):
    """Facets that fall outside the sheet, measured in logical units.

    A facet is authored in game units; the sheet is `scale` percent larger.
    """
    logical = (sheet[0] * 100 // scale, sheet[1] * 100 // scale)
    offenders = []
    for name, facet in actions:
        x, y, width, height = (int(value) for value in facet.split(",")[:4])
        if x + width > logical[0] or y + height > logical[1]:
            offenders.append((name, facet))
    return logical, offenders


def read_ini(path):
    """Read a definition text file WITHOUT translating its line endings.

    The shipped files are CRLF. Reading them through Python's universal
    newlines and writing back gives an LF file that differs on every single
    line, which buries a two-line facet change in a whole-file diff.
    """
    with open(path, encoding=ENCODING, newline="") as handle:
        return handle.read()


def write_ini(path, text):
    with open(path, "w", encoding=ENCODING, newline="") as handle:
        handle.write(text)


def install(shipped, hd, out, variant=None):
    if out.exists():
        shutil.rmtree(out)
    shutil.copytree(shipped, out)

    # A graphics VARIANT (GraphicsArmored.png, GraphicsBlackKnight.png, ...) is
    # selected at runtime by SetGraphics and shares the base definition's
    # ActMap and DefCore. Rewriting those for a variant would apply one
    # costume's facets to every costume, so a variant install replaces only its
    # own sheet pair -- and therefore requires the variant to have been
    # rendered on the same layout as the base.
    if variant:
        graphics_name = f"Graphics{variant}.png"
        overlay_name = f"Overlay{variant}.png"
        for source, target in (
            (hd / "Graphics.png", out / graphics_name),
            (hd / "Overlay.png", out / overlay_name),
        ):
            if source.exists():
                shutil.copy(source, target)
        base = png_size(out / "Graphics.png")
        got = png_size(out / graphics_name)
        print(f"installed variant {variant} into {out}")
        print(f"  {graphics_name} {got[0]}x{got[1]}  base Graphics.png {base[0]}x{base[1]}")
        if got != base:
            # Variants index the base ActMap, so their sheets must be the same
            # size or every facet lands somewhere different on this costume.
            print("  FATAL: a variant sheet must match the base sheet exactly")
            return 1
        overlay = out / overlay_name
        if overlay.exists() and png_size(overlay) != got:
            print("  FATAL: ColorByOwner needs the variant Overlay to match its Graphics")
            return 1
        return 0

    hd_actions = dict(parse_actions(read_ini(hd / "ActMap.txt")))
    shipped_text = read_ini(shipped / "ActMap.txt")
    shipped_actions = parse_actions(shipped_text)
    rect_map = facet_rect_map(shipped_actions, hd_actions)

    unmapped = [name for name, rect in shipped_actions if rect not in rect_map]
    aliases = [
        name for name, rect in shipped_actions if name not in hd_actions and rect in rect_map
    ]

    spliced = splice_act_map(shipped_text, rect_map)
    write_ini(out / "ActMap.txt", spliced)

    core, scale = splice_def_core(
        read_ini(shipped / "DefCore.txt"),
        read_ini(hd / "DefCore.txt"),
    )
    write_ini(out / "DefCore.txt", core)

    for art in ("Graphics.png", "Overlay.png"):
        source = hd / art
        if source.exists():
            shutil.copy(source, out / art)

    graphics = png_size(out / "Graphics.png")
    overlay = out / "Overlay.png"
    print(f"installed {out}")
    print(f"  Graphics {graphics[0]}x{graphics[1]}  Scale={scale}")

    # Any OTHER action sheet in this definition still indexes the ActMap that
    # was just rewritten, so leaving it at the old layout makes it read out of
    # bounds. A sheet only counts if it is named GraphicsX.png -- an
    # OverlayX.png is re-flowed with its Graphics partner and never on its own,
    # because `Overlay1.png` pairs with `Portrait1.png`, not with the action
    # sheet, and resizing it to the action sheet breaks the portrait instead.
    strips = action_strips(shipped_text)
    for sheet in sorted(out.glob("Graphics*.png")):
        suffix = sheet.stem[len("Graphics"):]
        if not suffix or png_size(sheet) == graphics:
            continue
        pair = [(sheet.name, sheet)]
        companion = out / f"Overlay{suffix}.png"
        if companion.exists():
            pair.append((companion.name, companion))
        for name, target in pair:
            placed = reflow_auxiliary_sheet(
                shipped / name, (target, graphics), strips, rect_map, scale
            )
            print(f"  re-flowed {name} onto the new layout ({placed} actions, upscaled)")
    if overlay.exists() and png_size(overlay) != graphics:
        # ColorByOwner multiplies the overlay against the base by texel, so
        # the engine rejects a pair whose dimensions disagree.
        print("  FATAL: ColorByOwner needs Graphics.png and Overlay.png to match")
        return 1

    print(f"  {len(rect_map)} rects mapped; aliases resolved: {aliases or 'none'}")
    if unmapped:
        print(f"  FATAL: no high-resolution facet for {unmapped}")
        return 1

    logical, offenders = out_of_bounds(parse_actions(spliced), graphics, scale)
    print(f"  logical sheet {logical[0]}x{logical[1]}; out of bounds: {offenders or 'none'}")
    return 1 if offenders else 0


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("shipped", type=Path, help="the shipped .c4d to splice into")
    parser.add_argument("pack", type=Path, help="the rendered high-resolution pack")
    parser.add_argument("out", type=Path, help="where to write the installed .c4d")
    parser.add_argument(
        "--variant",
        help="install as a graphics variant (e.g. Armored -> GraphicsArmored.png), "
        "leaving the base ActMap and DefCore untouched",
    )
    args = parser.parse_args(argv)
    return install(args.shipped, args.pack, args.out, args.variant)


if __name__ == "__main__":
    sys.exit(main())
