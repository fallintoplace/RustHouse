import math
import unittest
from smoke import run_smoke

class SmokeArgumentTests(unittest.TestCase):
    def test_invalid_inputs_fail_without_fixture(self):
        for kw in ({"rows": True}, {"rows": 1.5}, {"seed": 0.1},
                   {"runs": True}, {"warmups": -1}, {"warmups": 1.5},
                   {"timeout": math.inf}, {"timeout": math.nan}, {"timeout": True}):
            with self.subTest(kw=kw), self.assertRaises(ValueError):
                run_smoke("rusthouse", execute=lambda sql: self.fail("fixture setup"), **kw)

if __name__ == "__main__":
    unittest.main()
