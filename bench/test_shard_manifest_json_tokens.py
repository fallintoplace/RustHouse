import tempfile
import unittest
from pathlib import Path
from bench.scale.verify_id_shards import verify


class StrictCaseJsonTests(unittest.TestCase):
    def test_rejects_nonstandard_numbers_before_using_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "manifest.json"
            for token in ("NaN", "Infinity", "-Infinity"):
                with self.subTest(token=token):
                    path.write_text('{"schema_version":1,"unexpected":' + token + '}', encoding="utf-8")
                    with self.assertRaisesRegex(ValueError, "nonstandard"):
                        verify(path)


if __name__ == "__main__":
    unittest.main()
