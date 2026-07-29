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


def splice_act_map(shipped_text, rect_map):
    """Rewrite every `Facet=` the map covers; leave all other keys alone."""

    def rewrite(block):
        current = re.search(r"^Facet=(.+)$", block, re.M)
        if not current or current.group(1).strip() not in rect_map:
            return block
        return re.sub(
            r"^Facet=.+$",
            "Facet=" + rect_map[current.group(1).strip()],
            block,
            count=1,
            flags=re.M,
        )

    head, *blocks = shipped_text.split("[Action]")
    return head + "".join("[Action]" + rewrite(block) for block in blocks)


def splice_def_core(shipped_core, hd_core):
    """Adopt the pack's `Scale` and `Picture`, keeping every other field."""
    scale = re.search(r"^Scale=(\d+)$", hd_core, re.M)
    if not scale:
        raise ValueError("high-resolution pack DefCore.txt has no Scale=")
    picture = re.search(r"^Picture=(.+)$", hd_core, re.M)
    core = shipped_core
    if picture:
        core = re.sub(
            r"^Picture=.+$", "Picture=" + picture.group(1).strip(), core, count=1, flags=re.M
        )
    # Scale belongs to [DefCore]; anchor it to Picture so it cannot land in a
    # later section such as [Physical].
    core = re.sub(r"^(Picture=.+)$", r"\1\nScale=" + scale.group(1), core, count=1, flags=re.M)
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


def install(shipped, hd, out):
    if out.exists():
        shutil.rmtree(out)
    shutil.copytree(shipped, out)

    hd_actions = dict(parse_actions((hd / "ActMap.txt").read_text(encoding=ENCODING)))
    shipped_text = (shipped / "ActMap.txt").read_text(encoding=ENCODING)
    shipped_actions = parse_actions(shipped_text)
    rect_map = facet_rect_map(shipped_actions, hd_actions)

    unmapped = [name for name, rect in shipped_actions if rect not in rect_map]
    aliases = [
        name for name, rect in shipped_actions if name not in hd_actions and rect in rect_map
    ]

    spliced = splice_act_map(shipped_text, rect_map)
    (out / "ActMap.txt").write_text(spliced, encoding=ENCODING)

    core, scale = splice_def_core(
        (shipped / "DefCore.txt").read_text(encoding=ENCODING),
        (hd / "DefCore.txt").read_text(encoding=ENCODING),
    )
    (out / "DefCore.txt").write_text(core, encoding=ENCODING)

    for art in ("Graphics.png", "Overlay.png"):
        source = hd / art
        if source.exists():
            shutil.copy(source, out / art)

    graphics = png_size(out / "Graphics.png")
    overlay = out / "Overlay.png"
    print(f"installed {out}")
    print(f"  Graphics {graphics[0]}x{graphics[1]}  Scale={scale}")
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
    args = parser.parse_args(argv)
    return install(args.shipped, args.pack, args.out)


if __name__ == "__main__":
    sys.exit(main())
