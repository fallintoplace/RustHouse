import io
import tempfile
import unittest
from pathlib import Path

from bench.scale.skewed_fixture import emit, generate


class SkewedFixtureTests(unittest.TestCase):
    def test_ninety_percent_hot_group(self):
        out = io.StringIO()
        emit(out, 100, groups=10, hot_percent=90, batch_rows=7)
        fields = [line.split("\t") for line in out.getvalue().splitlines()]
        self.assertEqual(len(fields), 100)
        self.assertEqual(sum(int(key) == 0 for _, key in fields), 90)
        self.assertEqual(fields[90], ["90", "1"])

    def test_all_hot_and_all_cold(self):
        hot, cold = io.StringIO(), io.StringIO()
        emit(hot, 4, hot_percent=100)
        emit(cold, 4, hot_percent=0, groups=2)
        self.assertTrue(all(row.endswith("\t0") for row in hot.getvalue().splitlines()))
        self.assertTrue(all(not row.endswith("\t0") for row in cold.getvalue().splitlines()))

    def test_atomic_output_and_validation(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "events.tsv"
            generate(path, 3)
            self.assertEqual(len(path.read_text().splitlines()), 3)
            with self.assertRaises(FileExistsError):
                generate(path, 4)
        with self.assertRaisesRegex(ValueError, "groups"):
            emit(io.StringIO(), 1, groups=0)

    def test_concurrent_create_cannot_clobber_existing_output(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "events.tsv"
            with patch("bench.scale.skewed_fixture.os.link", side_effect=FileExistsError):
                with self.assertRaises(FileExistsError):
                    generate(path, 10)
            self.assertFalse(path.exists())
            self.assertEqual(list(Path(root).iterdir()), [])


if __name__ == "__main__":
    unittest.main()
