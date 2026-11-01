import tempfile
import unittest
from pathlib import Path

from bench.release_gate import load_manifest


class ReleaseManifestJsonTests(unittest.TestCase):
    def test_refuses_nan_and_infinity_even_in_extra_metadata(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "manifest.json"
            for token in ("NaN", "Infinity", "-Infinity"):
                with self.subTest(token=token):
                    path.write_text('{"reference_checked":true,"metadata":' + token + '}', encoding="utf-8")
                    with self.assertRaisesRegex(ValueError, "nonstandard"):
                        load_manifest(path)

    def test_standard_json_is_unchanged(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "manifest.json"
            path.write_text('{"reference_checked":true,"runs":2}', encoding="utf-8")
            self.assertEqual(load_manifest(path)["runs"], 2)


if __name__ == "__main__":
    unittest.main()
