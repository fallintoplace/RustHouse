import tempfile
import unittest
from pathlib import Path
from bench.scale.billion_matrix import load_cases


class BillionMatrixStrictJsonTests(unittest.TestCase):
    def test_rejects_duplicate_case_fields(self):
        with tempfile.TemporaryDirectory() as root:
            case = Path(root) / "01-case.json"
            case.write_text('{"name":"a","name":"b","kind":"modulo"}', encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "duplicate benchmark case JSON"):
                load_cases(root)


if __name__ == "__main__":
    unittest.main()
