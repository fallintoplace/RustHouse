"""UInt64 fixture dimensions must not silently accept boolean values."""
import io
import unittest

from bench.scale.tsv_generator import emit_rows


class TypedTsvGeneratorTests(unittest.TestCase):
    def test_rejects_boolean_dimensions_without_writing(self):
        for rows, kwargs in (
            (True, {}),
            (3, {"start": False}),
            (3, {"batch_rows": True}),
            (3, {"header": "false"}),
        ):
            with self.subTest(rows=rows, kwargs=kwargs):
                output = io.StringIO()
                with self.assertRaises(ValueError):
                    emit_rows(output, rows, **kwargs)
                self.assertEqual(output.getvalue(), "")

    def test_valid_batched_sequence_is_exact(self):
        output = io.StringIO()
        self.assertEqual(emit_rows(output, 5, start=8, batch_rows=2, header=True), 5)
        self.assertEqual(output.getvalue(), "id\n8\n9\n10\n11\n12\n")
