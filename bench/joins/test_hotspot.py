import tempfile
import unittest

from bench.joins.hotspot import fact_id, verify_rows, write_fixture


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


if __name__ == "__main__":
    unittest.main()
