"""Skewed TSV generation must not silently accept bools as integers."""
import io
import unittest

from bench.scale.skewed_fixture import emit


class SkewedFixtureInputTests(unittest.TestCase):
    def test_rejects_booleans_before_writing(self):
        for rows, kwargs in (
            (True, {}),
            (5, {"groups": True}),
            (5, {"hot_percent": False}),
            (5, {"batch_rows": True}),
        ):
            with self.subTest(rows=rows, kwargs=kwargs):
                output = io.StringIO()
                with self.assertRaises(ValueError):
                    emit(output, rows, **kwargs)
                self.assertEqual(output.getvalue(), "")

    def test_valid_chunks_keep_skew_and_row_order(self):
        output = io.StringIO()
        emit(output, 5, groups=2, hot_percent=0, batch_rows=2)
        self.assertEqual(output.getvalue(), "0\t1\n1\t2\n2\t1\n3\t2\n4\t1\n")
