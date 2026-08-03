import pathlib
import tomllib
import unittest


REPOSITORY = pathlib.Path(__file__).resolve().parents[2]


class ZipReleaseContractTests(unittest.TestCase):
    def test_release_writer_pins_unix_origin_metadata_with_supported_zip_floor(self):
        manifest = tomllib.loads(
            (REPOSITORY / "xtask/Cargo.toml").read_text(encoding="utf-8")
        )
        self.assertEqual(
            manifest["dependencies"]["zip"]["version"],
            "8.1",
            "FileOptions::system requires zip 8.1 or newer",
        )
        source = (REPOSITORY / "xtask/src/main.rs").read_text(encoding="utf-8")
        self.assertEqual(
            source.count(".system(zip::System::Unix)"),
            2,
            "release directory and file options must not inherit the host system",
        )


if __name__ == "__main__":
    unittest.main()
