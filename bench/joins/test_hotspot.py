import tempfile
import unittest

from bench.joins.hotspot import benchmark, fact_id, fixture_copy_sql, verify_rows, write_fixture


class SkewedJoinTests(unittest.TestCase):
    def test_fixture_has_configured_hot_key_and_exact_multiplicities(self):
        with tempfile.TemporaryDirectory() as root:
            expected = write_fixture(root, 100, 10, 90)
            self.assertEqual(sum(expected.values()), 100)
            self.assertEqual(expected[(0, "group-0")], 90)
            self.assertEqual(fact_id(95, 10, 90), 1 + 95 % 9)
            output = b"".join(
                f"{key}\t{label}\n".encode() * count
                for (key, label), count in expected.items()
            )
            verify_rows(output, expected)
            with self.assertRaisesRegex(ValueError, "multiset"):
                verify_rows(output[:-len(b"0\tgroup-0\n")], expected)

    def test_rejects_symlinked_hotspot_fixture_outputs(self):
        from pathlib import Path
        for name in ("facts.tsv", "dimensions.tsv"):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as root:
                directory = Path(root) / "fixture"
                directory.mkdir()
                protected = Path(root) / "protected"
                protected.write_text("keep")
                (directory / name).symlink_to(protected)
                with self.assertRaisesRegex(ValueError, "symlinked hotspot"):
                    write_fixture(directory, 10, 2, 50)
                self.assertEqual(protected.read_text(), "keep")
                self.assertFalse((directory / ("dimensions.tsv" if name == "facts.tsv" else "facts.tsv")).exists())

    def test_rejects_invalid_input_and_wrong_join_rows(self):
        with tempfile.TemporaryDirectory() as root:
            for probe, build, hot in [(0, 10, 90), (131073, 10, 90),
                                      (100, 32769, 90), (100, 10, -1),
                                      (100, 10, 101)]:
                with self.assertRaises(ValueError):
                    write_fixture(root, probe, build, hot)
            expected = write_fixture(root, 4, 2, 100)
            with self.assertRaisesRegex(ValueError, "multiset"):
                verify_rows(b"0\tincorrect\n" * 4, expected)
            with self.assertRaisesRegex(ValueError, "final newline"):
                verify_rows(b"0\tgroup-0", expected)




class BoundedHotspotControlsTests(unittest.TestCase):
    def test_invalid_dimensions_fail_before_writing_files(self):
        with tempfile.TemporaryDirectory() as root:
            for dims in ((True, 10, 90), (4.5, 10, 90),
                         (100, True, 90), (100, 10, True),
                         (100, 10, 90.5)):
                with self.subTest(dims=dims), self.assertRaises(ValueError):
                    write_fixture(root, *dims)

    def test_invalid_run_counts_fail_before_launching_sql(self):
        for values in ((True, 0, 1), (1, True, 1), (101, 0, 1),
                       (50, 51, 1), (1, 0, float("nan")),
                       (1, 0, float("inf"))):
            with self.subTest(values=values), self.assertRaisesRegex(ValueError, "invalid benchmark"):
                benchmark("no-binary", 10, 3, 50, *values)

class HotJoinCopyEscapingTests(unittest.TestCase):
    def test_copy_path_with_apostrophe_is_one_sql_literal(self):
        from pathlib import Path
        self.assertEqual(
            fixture_copy_sql("dimensions", Path("/tmp/a'b/dimensions.tsv")),
            "COPY dimensions FROM '/tmp/a''b/dimensions.tsv' WITH (FORMAT TSV, HEADER TRUE)",
        )
        with self.assertRaisesRegex(ValueError, "unsupported fixture"):
            fixture_copy_sql("invalid", Path("data.tsv"))


if __name__ == "__main__":
    unittest.main()
