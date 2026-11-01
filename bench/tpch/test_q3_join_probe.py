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


class Q3BoundedJoinInputTests(unittest.TestCase):
    def test_oversized_join_row_fails_before_materializing(self):
        from unittest.mock import patch
        from q3_join_probe import read_table
        with tempfile.TemporaryDirectory() as root:
            orders = Path(root) / "orders.tsv"
            orders.write_text("o_orderkey\to_custkey\n1\t" + "7" * 100 + "\n")
            with patch("q3_join_probe.MAX_REFERENCE_LINE_CHARS", 40):
                with self.assertRaisesRegex(ValueError, "exceeds"):
                    read_table(orders, "o_orderkey")

    def test_rejects_symlinked_join_input(self):
        from q3_join_probe import read_table
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "customers.tsv"
            path.write_text("c_custkey\n1\n")
            link = Path(root) / "alias.tsv"
            try:
                link.symlink_to(path)
            except (OSError, NotImplementedError):
                self.skipTest("symlinks unavailable")
            with self.assertRaisesRegex(ValueError, "non-symlinked"):
                read_table(link, "c_custkey")


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
