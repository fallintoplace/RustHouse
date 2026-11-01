"""Total range deadlines should fail with a clean nonzero exit."""
import contextlib
import io
import unittest
from unittest.mock import patch

from bench.scale import physical_range_oracle


class RangeOracleDeadlineCliTests(unittest.TestCase):
    def test_total_timeout_has_no_traceback(self):
        stderr = io.StringIO()
        args = ["--binary", "bin", "--database", "db", "--table", "events", "--rows", "1"]
        with patch.object(physical_range_oracle, "verify",
                          side_effect=TimeoutError("total deadline exceeded")):
            with contextlib.redirect_stderr(stderr), self.assertRaises(SystemExit) as raised:
                physical_range_oracle.main(args)
        self.assertEqual(raised.exception.code, 1)
        self.assertIn("total deadline exceeded", stderr.getvalue())
        self.assertNotIn("Traceback", stderr.getvalue())


if __name__ == "__main__":
    unittest.main()
