import unittest
from clickbench_subset import parse_ids


class StrictClickBenchQuerySelectors(unittest.TestCase):
    def test_requires_ascii_query_ids(self):
        self.assertEqual(parse_ids("1, 2 - 4,9"), [1, 2, 3, 4, 9])
        for spec in ("١", "1,٢", "+1", "1-+3", "1e0"):
            with self.subTest(spec=spec), self.assertRaisesRegex(ValueError, "ASCII numeric"):
                parse_ids(spec)


if __name__ == "__main__":
    unittest.main()
