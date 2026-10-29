import hashlib
import tempfile
import unittest
from pathlib import Path

from data_manifest import build_manifest
from schema import names


class DataManifestTests(unittest.TestCase):
    def test_hashes_exact_input_bytes_with_row_counts(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "region.tsv"
            data = ("\t".join(names("region")) + "\n" +
                    "1\tEUROPE\tcomment\n2\tASIA\tcomment\n").encode()
            path.write_bytes(data)
            result = build_manifest(root, ["region"])
            table = result["tables"][0]
            self.assertFalse(result["official_tpch_score"])
            self.assertEqual(table["rows"], 2)
            self.assertEqual(table["bytes"], len(data))
            self.assertEqual(table["sha256"], hashlib.sha256(data).hexdigest())
            path.write_bytes(data.replace(b"EUROPE", b"EUROPX"))
            self.assertNotEqual(build_manifest(root, ["region"])["tables"][0]["sha256"],
                                table["sha256"])

    def test_rejects_malformed_rows_and_missing_line_terminators(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "region.tsv"
            header = "\t".join(names("region")) + "\n"
            for row in ("1\tEUROPE\n", "1\tEUROPE\tcomment"):
                with self.subTest(row=row):
                    path.write_text(header + row)
                    with self.assertRaisesRegex(ValueError, "malformed"):
                        build_manifest(root, ["region"])

    def test_rejects_extra_header_line_terminators(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "region.tsv"
            for ending in (b"\r\r\n", b"\n\n"):
                path.write_bytes("\t".join(names("region")).encode() + ending + b"1\tEUROPE\tcomment\n")
                with self.assertRaisesRegex(ValueError, "header|blank record"):
                    build_manifest(root, ["region"])

    def test_rejects_missing_bad_header_and_duplicates(self):
        with tempfile.TemporaryDirectory() as root:
            with self.assertRaisesRegex(ValueError, "does not exist"):
                build_manifest(root, ["region"])
            (Path(root) / "region.tsv").write_bytes(b"wrong\n1\n")
            with self.assertRaisesRegex(ValueError, "header"):
                build_manifest(root, ["region"])
            with self.assertRaisesRegex(ValueError, "unique"):
                build_manifest(root, ["region", "region"])


class BoundedManifestInputTests(unittest.TestCase):
    def test_rejects_oversized_physical_record(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "region.tsv"
            path.write_bytes(("\t".join(names("region")) + "\n").encode() +
                             b"1\tEUROPE\t" + b"x" * 100 + b"\n")
            with patch("data_manifest.MAX_RECORD_BYTES", 64):
                with self.assertRaisesRegex(ValueError, "malformed"):
                    build_manifest(root, ["region"])

if __name__ == "__main__":
    unittest.main()
