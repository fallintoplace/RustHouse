import math
import tempfile
import unittest
from pathlib import Path

from bench.scale.keyset_export import export, page_sql


class KeysetParametersTests(unittest.TestCase):
    def test_rejects_noninteger_pages(self):
        for value in (True, 2.5):
            with self.subTest(value=value), self.assertRaises(ValueError):
                page_sql("events", "id", 0, value)
            with self.subTest(value=value), self.assertRaises(ValueError):
                page_sql("events", "id", value, 2)

    def test_rejects_nonfinite_timeouts_before_writing(self):
        with tempfile.TemporaryDirectory() as temp:
            dest = Path(temp) / "out.tsv"
            for timeout in (math.nan, math.inf, 0, -1, True):
                with self.subTest(timeout=timeout), self.assertRaisesRegex(ValueError, "timeout"):
                    export("unused", "unused", "events", dest, 0, 1, timeout=timeout,
                           execute=lambda _: self.fail("should not query"))
                self.assertFalse(dest.exists())

    def test_rejects_fractional_page_size_before_writing(self):
        with tempfile.TemporaryDirectory() as temp:
            dest = Path(temp) / "out.tsv"
            with self.assertRaisesRegex(ValueError, "page size"):
                export("unused", "unused", "events", dest, 0, 1, page_rows=1.5)


if __name__ == "__main__":
    unittest.main()
