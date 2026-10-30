import tempfile
import unittest
from pathlib import Path

from q6_dbgen import Q6, expected_revenue, run_q6


class Q6OracleTests(unittest.TestCase):
    def test_fixed_point_revenue_and_boundaries(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "lineitem.tsv"
            path.write_text(
                "l_shipdate\tl_discount\tl_quantity\tl_extendedprice\n"
                "1994-01-01\t500\t2300\t12345\n"
                "1994-12-31\t700\t2399\t200\n"
                "1995-01-01\t600\t2000\t9999\n"
                "1994-05-01\t600\t2400\t9999\n"
            )
            expected = 12345 * 500 + 200 * 700
            self.assertEqual(expected_revenue(path), expected)
            report = run_q6(Path("fake"), Path("db"), path, runs=2, execute=lambda _: f"{expected}\n".encode())
            self.assertEqual(report["results"][0]["status"], "passed")
            self.assertEqual(report["coverage"]["verified_correct_percent"], 100.0)
            self.assertIn("l_quantity < 2400", Q6)

    def test_rejects_missing_or_duplicate_q6_columns(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "lineitem.tsv"
            for header in ("l_shipdate\tl_discount\n",
                           "l_shipdate\tl_discount\tl_quantity\tl_quantity\tl_extendedprice\n"):
                with self.subTest(header=header):
                    path.write_text(header)
                    with self.assertRaisesRegex(ValueError, "columns"):
                        expected_revenue(path)

    def test_rejects_extra_q6_data_fields(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "lineitem.tsv"
            path.write_text(
                "l_shipdate\tl_discount\tl_quantity\tl_extendedprice\n"
                "1994-01-01\t500\t2300\t100\textra\n"
            )
            with self.assertRaisesRegex(ValueError, "extra"):
                expected_revenue(path)

    def test_zero_revenue_from_qualifying_row_is_not_null(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "lineitem.tsv"
            path.write_text(
                "l_shipdate\tl_discount\tl_quantity\tl_extendedprice\n"
                "1994-01-01\t500\t1000\t0\n"
            )
            report = run_q6(Path("fake"), Path("db"), path, runs=1, warmups=0,
                            execute=lambda _: b"0\n")
            self.assertEqual(report["expected_revenue_integer"], 0)
            self.assertEqual(report["matched_rows"], 1)
            self.assertEqual(report["results"][0]["status"], "passed")

    def test_does_not_count_mismatch_as_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "lineitem.tsv"
            path.write_text("l_shipdate\tl_discount\tl_quantity\tl_extendedprice\n")
            report = run_q6(Path("fake"), Path("db"), path, execute=lambda _: b"0\n")
            self.assertEqual(report["results"][0]["status"], "mismatch")


class Q6TimingValidationTests(unittest.TestCase):
    def test_rejects_nonfinite_timeout_and_boolean_runs(self):
        for options in ({"timeout": float("nan")}, {"timeout": float("inf")},
                        {"runs": True}, {"warmups": False}):
            with self.subTest(options=options):
                with self.assertRaisesRegex(ValueError, "invalid runs"):
                    run_q6(None, None, Path("nonexistent"), **options)

if __name__ == "__main__":
    unittest.main()
