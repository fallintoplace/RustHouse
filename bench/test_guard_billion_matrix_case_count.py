import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from bench.scale.billion_matrix import load_cases


class BillionMatrixCaseCountTests(unittest.TestCase):
    def test_rejects_too_many_numbered_cases(self):
        with tempfile.TemporaryDirectory() as root:
            for index in range(3):
                (Path(root) / f"{index:02d}-case.json").write_text(
                    f'{{"name":"case-{index}","kind":"modulo"}}', encoding="utf-8")
            with patch("bench.scale.billion_matrix.MAX_CASES", 2):
                with self.assertRaisesRegex(ValueError, "case count exceeds"):
                    load_cases(root)


if __name__ == "__main__":
    unittest.main()
