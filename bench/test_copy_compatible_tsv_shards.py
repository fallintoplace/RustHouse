"""Validate COPY TSV syntax before publishing immutable shard manifests."""
import tempfile
import unittest
from pathlib import Path

from bench.scale.tsv_shards import split_tsv


class CopyCompatibleShards(unittest.TestCase):
    def test_bad_final_record_never_publishes_earlier_shards(self):
        for bad in (b"bad\\q\n", b"bad\\xQ0\n", b"bad\\x0\n",
                    b"bad\\xFF\n", b"bad\x00row\n", b"bad\xff\n",
                    b"bad\rrow\n", b"1\t" * 128 + b"1\n"):
            with self.subTest(bad=bad), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                source = root / "input.tsv"
                source.write_bytes(b"valid\n" + bad)
                dest = root / "shards"
                with self.assertRaises(ValueError):
                    split_tsv(source, dest, rows_per_chunk=1)
                self.assertFalse(dest.exists())
                self.assertFalse(list(root.glob(".rusthouse-shards-*")))

    def test_supported_escapes_utf8_and_128_columns_stay_lossless(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "input.tsv"
            source.write_bytes(b"caf\\xC3\\xA9\t\\N\t\\t\\n\\r\\0\\b\\f\\v\\\\\n"
                               + b"\t".join([b"x"] * 128) + b"\r\n")
            dest = root / "shards"
            manifest = split_tsv(source, dest, rows_per_chunk=1)
            self.assertEqual(manifest["total_rows"], 2)
            self.assertEqual(b"".join((dest / s["path"]).read_bytes()
                                      for s in manifest["shards"]), source.read_bytes())


if __name__ == "__main__":
    unittest.main()
