import unittest
from copy import deepcopy

from canonical_gate import audit


def complete():
    return {"reference_checked": True,
            "results": [{"query": i, "status": "passed"} for i in range(1, 23)],
            "coverage": {"verified_queries": 22, "failed_queries": 0,
                         "unverified_queries": 0},
            "official_22_coverage": {"available_queries": 22,
                                    "missing_query_ids": [],
                                    "unexpected_query_ids": []}}


class CanonicalGateTests(unittest.TestCase):
    def test_full_verified_set_passes(self):
        self.assertEqual(audit(complete())["reference_verified_queries"], 22)

    def test_unverified_execution_is_never_a_pass(self):
        for kind in ("unverified", "error", "mismatch", "timeout", "missing_reference"):
            bad = complete()
            bad["results"][5]["status"] = kind
            with self.subTest(kind=kind):
                with self.assertRaisesRegex(ValueError, "incomplete"):
                    audit(bad)

    def test_rejects_missing_duplicate_or_lying_coverage(self):
        bad = complete()
        bad["results"][21]["query"] = 21
        with self.assertRaisesRegex(ValueError, "exactly once"):
            audit(bad)
        bad = complete()
        bad["results"].pop()
        with self.assertRaisesRegex(ValueError, "exactly 22"):
            audit(bad)
        bad = complete()
        bad["coverage"]["verified_queries"] = 21
        with self.assertRaisesRegex(ValueError, "does not attest"):
            audit(bad)

    def test_requires_independent_references(self):
        bad = complete()
        bad["reference_checked"] = False
        with self.assertRaisesRegex(ValueError, "independent"):
            audit(bad)


if __name__ == "__main__":
    unittest.main()
