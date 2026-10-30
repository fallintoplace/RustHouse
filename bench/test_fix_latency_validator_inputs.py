import unittest
from bench.latency.validate_report import validate_report

class ValidatorArgumentsTests(unittest.TestCase):
    def test_invalid_dimensions_fail_before_suite_generation(self):
        for kw in ({"rows": True}, {"rows": 1.5}, {"rows": 0},
                   {"runs": True}, {"runs": 0}, {"runs": 2.5},
                   {"warmups": True}, {"warmups": -1}, {"warmups": 0.5}):
            args = {"rows": 100, "runs": 3, "warmups": 1}
            args.update(kw)
            with self.subTest(kw=kw), self.assertRaisesRegex(ValueError, "positive integers"):
                validate_report({}, **args)

    def test_valid_dimensions_reach_report_validation(self):
        with self.assertRaisesRegex(ValueError, "wrong SQL latency report"):
            validate_report({}, rows=100, runs=3, warmups=0)

if __name__ == "__main__":
    unittest.main()
