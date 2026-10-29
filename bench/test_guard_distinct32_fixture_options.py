import tempfile
import unittest
from pathlib import Path

from bench.scale.distinct32_memory import benchmark, distinct_values, write_tsv


class Distinct32OptionGuardsTests(unittest.TestCase):
    def test_rejects_noninteger_fixture_dimensions_before_file_creation(self):
        with tempfile.TemporaryDirectory() as root:
            target = Path(root) / "fixture.tsv"
            for rows, duplicates, layout in (
                (True, 1, "sparse"), (2.5, 1, "sparse"),
                (2, True, "sparse"), (2, 1.5, "sparse"),
                (2, 1, "unknown"),
            ):
                with self.subTest(rows=rows, duplicates=duplicates, layout=layout):
                    with self.assertRaises(ValueError):
                        write_tsv(target, rows, layout, duplicates)
                    self.assertFalse(target.exists())

    def test_generator_rejects_boolean_row_counts(self):
        with self.assertRaisesRegex(ValueError, "rows"):
            list(distinct_values(True, "dense"))

    def test_rejects_bad_benchmark_controls_before_spawning_binary(self):
        for kwargs in (
            {"runs": True}, {"runs": 1.5}, {"timeout": True},
            {"timeout": "60"}, {"sample_ms": True}, {"sample_ms": 2.5},
        ):
            with self.subTest(kwargs=kwargs), self.assertRaises(ValueError):
                benchmark("unused", **kwargs)


if __name__ == "__main__":
    unittest.main()
