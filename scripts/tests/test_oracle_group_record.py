import importlib.util
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
MODULE = ROOT / "parity/bridge/oracle_group_record.py"


class OracleGroupRecordTests(unittest.TestCase):
    def test_missing_current_tree_archive_and_changed_artifact_fail_closed(self):
        self.assertTrue(MODULE.is_file(), "group build records must be verified before a differential")
        spec = importlib.util.spec_from_file_location("oracle_group_record", MODULE)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            port = root / "port"
            build = root / "build"
            target = port / "target/release"
            target.mkdir(parents=True)
            link = build / "CMakeFiles/clonk.dir/link.txt"
            link.parent.mkdir(parents=True)
            archive = target / "liblc_resources.a"
            link.write_text(f"c++ main.o {archive} -o clonk\n")
            with self.assertRaisesRegex(ValueError, "missing.*liblc_resources"):
                module.linked_resources(port, build)
            archive.write_bytes(b"archive-one")
            self.assertEqual(module.linked_resources(port, build), archive.resolve())
            record = {"artifacts": {str(archive): module.digest(archive)}}
            module.verify_artifacts(record)
            archive.write_bytes(b"archive-two")
            with self.assertRaisesRegex(ValueError, "changed artifact"):
                module.verify_artifacts(record)
            outside = root / "other-tree/liblc_resources.a"
            outside.parent.mkdir()
            outside.write_bytes(b"archive-one")
            link.write_text(f"c++ main.o {outside} -o clonk\n")
            with self.assertRaisesRegex(ValueError, "current tree"):
                module.linked_resources(port, build)


if __name__ == "__main__":
    unittest.main()
