"""Chunk COPY must fail on nonstandard JSON constants before any writes."""
import tempfile
import unittest
from pathlib import Path

from bench.scale.chunk_copy import load_chunks


class StrictCopyManifestJsonTests(unittest.TestCase):
    def test_nonfinite_extra_fields_fail_before_copy(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "manifest.json"
            for number in ("NaN", "Infinity", "-Infinity"):
                path.write_text(
                    '{"format":"rusthouse-tsv-shards-v1","header":false,'
                    '"shards":[],"extra":' + number + '}', encoding="ascii")
                with self.subTest(number=number):
                    with self.assertRaisesRegex(ValueError, "JSON constant"):
                        load_chunks("bin", "db", "events", root, dry_run=True)


if __name__ == "__main__":
    unittest.main()
