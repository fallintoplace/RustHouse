import tempfile
import unittest
from pathlib import Path
from q1_global import oracle, benchmark, SQL


class Q1GlobalTests(unittest.TestCase):
    def test_real_integer_totals_exclude_late_rows(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "lineitem.tsv"
            source.write_text("l_shipdate\tl_quantity\tl_extendedprice\n"
                              "1998-09-02\t100\t400\n"
                              "1998-09-03\t500\t2000\n"
                              "1994-01-01\t200\t600\n")
            self.assertEqual(oracle(source), b"2\t300\t1000\n")
            report = benchmark("fake", "db", source, execute=lambda sql: b"2\t300\t1000\n")
            self.assertEqual(report["results"][0]["status"], "passed")
            self.assertFalse(report["official_tpch"])

    def test_unmatched_returns_null_sums(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "lineitem.tsv"
            path.write_text("l_shipdate\tl_quantity\tl_extendedprice\n1999-01-01\t1\t2\n")
            self.assertEqual(oracle(path), b"0\tNULL\tNULL\n")


class Q1InputShapeTests(unittest.TestCase):
    def test_rejects_malformed_columns_in_reference_data(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "lineitem.tsv"
            for payload in ("l_shipdate\tl_quantity\n1994-01-01\t1\n",
                            "l_shipdate\tl_quantity\tl_extendedprice\n1994-01-01\t1\n",
                            "l_shipdate\tl_quantity\tl_extendedprice\n1994-01-01\t1\t2\textra\n"):
                path.write_text(payload)
                with self.subTest(payload=payload):
                    with self.assertRaisesRegex(ValueError, "columns|data fields"):
                        oracle(path)

if __name__ == "__main__":
    unittest.main()
