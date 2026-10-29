import io
import unittest

from bench.scale.grouped_physical import emit, reference


class GroupedPhysicalDomainTests(unittest.TestCase):
    def test_rejects_noninteger_row_or_group_domain(self):
        for rows, groups in (
            (True, 2), (10.5, 2), (10, True),
            (10, 2.5), (0, 2), (10, 1), (10, 65_537),
        ):
            with self.subTest(rows=rows, groups=groups):
                stream = io.StringIO()
                with self.assertRaisesRegex(ValueError, "row count or UInt16"):
                    emit(stream, rows, groups)
                self.assertEqual(stream.getvalue(), "")
                with self.assertRaisesRegex(ValueError, "row count or UInt16"):
                    reference(rows, groups)

    def test_valid_domain_matches_generated_rows(self):
        output = io.StringIO()
        emit(output, 15, 4, batch_rows=2)
        count, amount = reference(15, 4)
        self.assertEqual(sum(count), 15)
        self.assertEqual(sum(amount), sum(1 + n % 97 for n in range(15)))
        self.assertEqual(len(output.getvalue().splitlines()), 15)


if __name__ == "__main__":
    unittest.main()
