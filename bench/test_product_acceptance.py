import unittest
from bench.product_acceptance import check_rows

class AcceptanceTests(unittest.TestCase):
    def test_exact_reference(self):
        check_rows("id\towner\tamount\n1\tAda\t20\n2\tGrace\t30\n")

    def test_detects_missing_row(self):
        with self.assertRaises(AssertionError):
            check_rows("id\towner\tamount\n1\tAda\t20\n")

    def test_detects_wrong_value(self):
        with self.assertRaises(AssertionError):
            check_rows("id\towner\tamount\n1\tAda\t20\n2\tGrace\t99\n")


if __name__ == "__main__":
    unittest.main()
