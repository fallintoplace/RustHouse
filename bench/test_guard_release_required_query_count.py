import unittest
from bench.release_gate import parse_query_ids


class ReleaseQueryCountTests(unittest.TestCase):
    def test_rejects_many_individually_valid_ranges(self):
        with self.assertRaisesRegex(ValueError, "exceeds 10000"):
            parse_query_ids("1-6000,6001-12000")

    def test_accepts_exactly_ten_thousand_queries(self):
        self.assertEqual(len(parse_query_ids("1-6000,6001-10000")), 10000)


if __name__ == "__main__":
    unittest.main()
