import importlib.util
import struct
import tempfile
import unittest
import zlib
from pathlib import Path

REPOSITORY = Path(__file__).resolve().parents[2]
MODULE_PATH = REPOSITORY / "scripts" / "install_hd_definition_pack.py"

_spec = importlib.util.spec_from_file_location("install_hd_definition_pack", MODULE_PATH)
install_hd_definition_pack = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(install_hd_definition_pack)


def write_png(path, width, height):
    """A minimal opaque-black PNG; only the IHDR is ever read back."""

    def chunk(kind, payload):
        return (
            struct.pack(">I", len(payload))
            + kind
            + payload
            + struct.pack(">I", zlib.crc32(kind + payload) & 0xFFFFFFFF)
        )

    raw = b"".join(b"\x00" + b"\x00\x00\x00\xff" * width for _ in range(height))
    path.write_bytes(
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw))
        + chunk(b"IEND", b"")
    )


# The shipped Clonk, trimmed to the actions that matter here. Scale/ScaleDown
# and Dig/Bridge each share one facet rect, and only one of each pair is named
# by a rendered pack.
SHIPPED_ACT_MAP = """[Action]
Name=Walk
Procedure=WALK
Directions=2
FlipDir=1
Length=16
Delay=15
Facet=0,0,16,20
NextAction=Walk
StartCall=None
InLiquidAction=Swim

[Action]
Name=Scale
Procedure=SCALE
Directions=2
FlipDir=1
Length=16
Facet=0,20,16,20
NextAction=Scale
StartCall=Scaling

[Action]
Name=ScaleDown
Procedure=SCALE
Directions=2
FlipDir=1
Length=16
Reverse=1
Facet=0,20,16,20
NextAction=ScaleDown

[Action]
Name=Dig
Procedure=DIG
Directions=2
Length=16
Facet=0,60,16,20
NextAction=Dig

[Action]
Name=Bridge
Procedure=BRIDGE
Directions=2
Length=16
Facet=0,60,16,20
NextAction=Bridge
"""

SHIPPED_DEF_CORE = """[DefCore]
id=CLNK
Name=Clonk
Width=16
Height=20
Offset=-8,-10
Picture=192,100,32,40
ColorByOwner=1

[Physical]
Energy=50000
Scale=30000
"""

# A rendered pack: six-component facets, taller rows, and no alias actions.
HD_ACT_MAP = """[Action]
Name=Walk
Length=16
Facet=0,0,16,22,0,-2

[Action]
Name=Scale
Length=16
Facet=0,22,20,22,-2,-1

[Action]
Name=Dig
Length=16
Facet=0,66,20,22,-2,-2
"""

HD_DEF_CORE = """[DefCore]
Width=16
Height=20
Offset=-8, -10
Picture=280,242,32,40
Scale=300
"""


class InstallHdDefinitionPackTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        root = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        self.shipped = root / "Shipped.c4d"
        self.pack = root / "Pack"
        self.out = root / "Out.c4d"
        self.shipped.mkdir()
        self.pack.mkdir()
        (self.shipped / "ActMap.txt").write_text(SHIPPED_ACT_MAP, encoding="latin-1")
        (self.shipped / "DefCore.txt").write_text(SHIPPED_DEF_CORE, encoding="latin-1")
        (self.shipped / "Script.c").write_text("#strict\n", encoding="latin-1")
        write_png(self.shipped / "Graphics.png", 16, 16)
        write_png(self.shipped / "Overlay.png", 16, 16)
        (self.pack / "ActMap.txt").write_text(HD_ACT_MAP, encoding="latin-1")
        (self.pack / "DefCore.txt").write_text(HD_DEF_CORE, encoding="latin-1")
        # 320x308 logical at Scale=300.
        write_png(self.pack / "Graphics.png", 960, 924)
        write_png(self.pack / "Overlay.png", 960, 924)

    def install(self):
        return install_hd_definition_pack.install(self.shipped, self.pack, self.out)

    def actions(self):
        text = (self.out / "ActMap.txt").read_text(encoding="latin-1")
        return dict(install_hd_definition_pack.parse_actions(text))

    def test_alias_actions_follow_the_rect_they_share_without_being_named(self):
        self.assertEqual(self.install(), 0)
        actions = self.actions()
        # ScaleDown and Bridge appear in no rendered pack, but each shares a
        # facet rect with an action that does.
        self.assertEqual(actions["ScaleDown"], actions["Scale"])
        self.assertEqual(actions["ScaleDown"], "0,22,20,22,-2,-1")
        self.assertEqual(actions["Bridge"], actions["Dig"])
        self.assertEqual(actions["Bridge"], "0,66,20,22,-2,-2")

    def test_behaviour_keys_survive_the_splice(self):
        self.assertEqual(self.install(), 0)
        text = (self.out / "ActMap.txt").read_text(encoding="latin-1")
        # Only the facet changes; the pack has no opinion on any of these.
        for key in ("Procedure=WALK", "FlipDir=1", "InLiquidAction=Swim", "Reverse=1"):
            self.assertIn(key, text)
        self.assertIn("Facet=0,0,16,22,0,-2", text)
        self.assertNotIn("Facet=0,0,16,20\n", text)

    def test_def_core_takes_the_pack_scale_and_picture_but_keeps_the_shape(self):
        self.assertEqual(self.install(), 0)
        core = (self.out / "DefCore.txt").read_text(encoding="latin-1")
        self.assertIn("Scale=300", core)
        self.assertIn("Picture=280,242,32,40", core)
        # Shape is in game units and must not follow the art.
        self.assertIn("Width=16", core)
        self.assertIn("Height=20", core)
        self.assertIn("id=CLNK", core)
        # `Scale` must land in [DefCore], above [Physical].
        self.assertLess(core.index("Scale=300"), core.index("[Physical]"))
        # The physical Scale is a different key that must survive untouched.
        self.assertIn("Scale=30000", core)

    def test_unmapped_shipped_action_fails_instead_of_reading_stale_coordinates(self):
        # Drop Dig from the pack: Dig and Bridge now have no rect to map to,
        # and their shipped 0,60 would land mid-Walk-row on the new sheet.
        (self.pack / "ActMap.txt").write_text(
            HD_ACT_MAP.split("[Action]\nName=Dig")[0], encoding="latin-1"
        )
        self.assertEqual(self.install(), 1)

    def test_mismatched_overlay_dimensions_fail(self):
        # C++ rejects a ColorByOwner pair whose dimensions disagree, so the
        # installer must not produce one.
        write_png(self.pack / "Overlay.png", 960, 900)
        self.assertEqual(self.install(), 1)

    def test_facet_outside_the_new_sheet_fails(self):
        write_png(self.pack / "Graphics.png", 96, 96)
        write_png(self.pack / "Overlay.png", 96, 96)
        self.assertEqual(self.install(), 1)

    def test_shipped_crlf_line_endings_survive_the_splice(self):
        # The shipped definition files are CRLF. Rewriting them as LF changes
        # every line, which buries the real two-line facet change in a
        # whole-file diff.
        crlf = SHIPPED_ACT_MAP.replace("\n", "\r\n")
        (self.shipped / "ActMap.txt").write_bytes(crlf.encode("latin-1"))
        (self.shipped / "DefCore.txt").write_bytes(
            SHIPPED_DEF_CORE.replace("\n", "\r\n").encode("latin-1")
        )
        self.assertEqual(self.install(), 0)
        for name in ("ActMap.txt", "DefCore.txt"):
            raw = (self.out / name).read_bytes()
            self.assertNotIn(b"\n\n", raw, f"{name} gained a bare LF")
            self.assertEqual(
                raw.count(b"\r\n"),
                raw.count(b"\n"),
                f"{name} must stay entirely CRLF",
            )
        self.assertIn(b"Facet=0,0,16,22,0,-2\r\n", (self.out / "ActMap.txt").read_bytes())


class ReflowAuxiliarySheetTests(unittest.TestCase):
    """An extra sheet indexes the SAME ActMap, so it must follow the new layout."""

    def setUp(self):
        try:
            import PIL  # noqa: F401
        except ImportError:
            self.skipTest("Pillow is not installed")
        base = InstallHdDefinitionPackTests("run")
        base.setUp()
        self.addCleanup(base._tmp.cleanup)
        self.shipped, self.pack, self.out = base.shipped, base.pack, base.out
        # The Knights' GraphicsShield.png is an ExtraGraphics overlay drawn
        # through the clonk's own definition, so it inherits its Scale.
        write_png(self.shipped / "GraphicsShield.png", 16, 16)
        write_png(self.shipped / "OverlayShield.png", 16, 16)

    def test_a_stale_auxiliary_sheet_is_reflowed_to_match_the_base(self):
        self.assertEqual(
            install_hd_definition_pack.install(self.shipped, self.pack, self.out), 0
        )
        base = install_hd_definition_pack.png_size(self.out / "Graphics.png")
        self.assertEqual(base, (960, 924))
        for name in ("GraphicsShield.png", "OverlayShield.png"):
            self.assertEqual(
                install_hd_definition_pack.png_size(self.out / name),
                base,
                f"{name} would index the rewritten ActMap at the old coordinates",
            )

    def test_a_portrait_overlay_is_not_mistaken_for_an_action_sheet(self):
        # Overlay1.png pairs with Portrait1.png, NOT with Graphics.png. Resizing
        # it to the action sheet makes the loader reject the definition with
        # "size ... does not match graphics".
        write_png(self.shipped / "Portrait1.png", 150, 150)
        write_png(self.shipped / "Overlay1.png", 150, 150)
        self.assertEqual(
            install_hd_definition_pack.install(self.shipped, self.pack, self.out), 0
        )
        self.assertEqual(
            install_hd_definition_pack.png_size(self.out / "Overlay1.png"),
            (150, 150),
            "the portrait overlay must keep its portrait dimensions",
        )

    def test_a_sheet_already_matching_the_base_is_left_alone(self):
        write_png(self.shipped / "GraphicsShield.png", 960, 924)
        write_png(self.shipped / "OverlayShield.png", 960, 924)
        before = (self.shipped / "GraphicsShield.png").read_bytes()
        self.assertEqual(
            install_hd_definition_pack.install(self.shipped, self.pack, self.out), 0
        )
        self.assertEqual((self.out / "GraphicsShield.png").read_bytes(), before)


class InstallGraphicsVariantTests(InstallHdDefinitionPackTests):
    """A variant sheet shares the base definition's ActMap and DefCore."""

    def setUp(self):
        super().setUp()
        write_png(self.shipped / "GraphicsArmored.png", 16, 16)
        write_png(self.shipped / "OverlayArmored.png", 16, 16)

    def install_variant(self, variant="Armored"):
        return install_hd_definition_pack.install(
            self.shipped, self.pack, self.out, variant
        )

    def test_variant_replaces_only_its_own_sheet_pair(self):
        # Install the base first so the variant has a matching base sheet.
        self.assertEqual(self.install(), 0)
        shutil_src = self.out
        base_actmap = (shipped_out := shutil_src / "ActMap.txt").read_bytes()
        base_defcore = (shutil_src / "DefCore.txt").read_bytes()
        self.shipped = shutil_src  # variant installs on top of the HD base
        self.out = self.out.parent / "Variant.c4d"
        self.assertEqual(self.install_variant(), 0)
        self.assertEqual((self.out / "ActMap.txt").read_bytes(), base_actmap)
        self.assertEqual((self.out / "DefCore.txt").read_bytes(), base_defcore)
        self.assertEqual(
            install_hd_definition_pack.png_size(self.out / "GraphicsArmored.png"),
            (960, 924),
        )
        self.assertEqual(
            install_hd_definition_pack.png_size(self.out / "OverlayArmored.png"),
            (960, 924),
        )

    def test_variant_sheet_that_does_not_match_the_base_fails(self):
        # Variants index the base ActMap, so a differently sized sheet would
        # put every facet somewhere else on this costume.
        self.assertEqual(self.install_variant(), 1)


if __name__ == "__main__":
    unittest.main()
