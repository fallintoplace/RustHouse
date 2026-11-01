import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from bench.analytics.fixture import generate


class FixtureRollbackTests(unittest.TestCase):
    def test_generator_failure_removes_partial_database_and_tsv(self):
        def broken_records():
            yield (1, 320, 1, "term-1", "2013-07-01", 2, "model-1")
            raise RuntimeError("fixture input failed")

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "fixture"
            with patch("bench.analytics.fixture.records", return_value=broken_records()):
                with self.assertRaisesRegex(RuntimeError, "fixture input failed"):
                    generate(root, rows=2, seed=0)
            for name in ("hits.tsv", "hits.sqlite", "create.sql", "copy.sql"):
                self.assertFalse((root / name).exists(), name)


if __name__ == "__main__":
    unittest.main()
