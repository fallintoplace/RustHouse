import io
import unittest
from bench.scale.tsv_generator import emit_rows

class StrictFixtureInputTests(unittest.TestCase):
    def test_invalid_types_fail_without_output(self):
        for rows, start, batch_rows in ((True, 0, 1), (1.5, 0, 1),
                                         (1, True, 1), (1, 0.5, 1),
                                         (1, 0, True), (1, 0, 2.5)):
            output = io.StringIO()
            with self.subTest(rows=rows, start=start, batch_rows=batch_rows):
                with self.assertRaises(ValueError):
                    emit_rows(output, rows, start=start, batch_rows=batch_rows)
                self.assertEqual(output.getvalue(), "")

    def test_stride_feature_is_preserved(self):
        output = io.StringIO()
        emit_rows(output, 3, start=7, step=3, batch_rows=2)
        self.assertEqual(output.getvalue(), "7\n10\n13\n")

    def test_valid_range(self):
        output = io.StringIO()
        self.assertEqual(emit_rows(output, 3, start=7, batch_rows=2), 3)
        self.assertEqual(output.getvalue(), "7\n8\n9\n")

if __name__ == "__main__":
    unittest.main()
