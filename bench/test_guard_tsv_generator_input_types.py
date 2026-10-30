import io
import tempfile
import unittest
from pathlib import Path

from bench.scale.tsv_generator import emit_rows, generate


class TsvGeneratorInputTypeTests(unittest.TestCase):
    def test_rejects_noninteger_and_boolean_counter_types(self):
        cases = [
            (True, {}), (1.5, {}), (1, {"start": True}),
            (1, {"start": 0.5}), (1, {"batch_rows": True}),
            (1, {"batch_rows": 2.5}), (1, {"header": 1}),
        ]
        for rows, options in cases:
            output = io.StringIO()
            with self.subTest(rows=rows, options=options), self.assertRaises(ValueError):
                emit_rows(output, rows, **options)
            self.assertEqual(output.getvalue(), "")

    def test_rejects_truthy_nonboolean_force_before_creating_file(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "data.tsv"
            with self.assertRaisesRegex(ValueError, "force"):
                generate(path, 2, force=1)
            self.assertFalse(path.exists())


if __name__ == "__main__":
    unittest.main()
