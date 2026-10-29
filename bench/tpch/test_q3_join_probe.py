import tempfile
import unittest
from pathlib import Path
from q3_join_probe import MAX_ROWS, benchmark, oracle


class JoinProbeTests(unittest.TestCase):
    def test_join_oracle_preserves_order_and_unmatched_rows(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root)
            (path / "orders.tsv").write_text(
                "o_orderkey\to_custkey\n3\t2\n1\t1\n2\t99\n")
            (path / "customer.tsv").write_text("c_custkey\n1\n2\n")
            expected = b"1\t1\n3\t2\n"
            self.assertEqual(oracle(path / "orders.tsv", path / "customer.tsv"), expected)
            run = benchmark("fake", "db", path / "orders.tsv", path / "customer.tsv",
                            execute=lambda _: expected)
            self.assertEqual(run["results"][0]["status"], "passed")
            self.assertFalse(run["official_tpch"])

    def test_reject_duplicate_customer_keys(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root)
            (path / "orders.tsv").write_text("o_orderkey\to_custkey\n1\t1\n")
            (path / "customer.tsv").write_text("c_custkey\n1\n1\n")
            with self.assertRaisesRegex(ValueError, "duplicate"):
                oracle(path / "orders.tsv", path / "customer.tsv")


class Q3InputShapeTests(unittest.TestCase):
    def test_rejects_missing_join_columns_and_extra_values(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root)
            orders = path / "orders.tsv"
            customers = path / "customer.tsv"
            customers.write_text("c_custkey\n1\n")
            orders.write_text("o_orderkey\n1\n")
            with self.assertRaisesRegex(ValueError, "join columns"):
                oracle(orders, customers)
            orders.write_text("o_orderkey\to_custkey\n1\t1\textra\n")
            with self.assertRaisesRegex(ValueError, "join fields"):
                oracle(orders, customers)

if __name__ == "__main__":
    unittest.main()
