import hashlib
import unittest
from types import SimpleNamespace

from bench.scale.query_digest import query_digest


class QueryDigestTests(unittest.TestCase):
    def test_spools_and_verifies_query_output(self):
        raw = b"1\n2\n3\n"
        def execute(command, **kwargs):
            kwargs["stdout"].write(raw)
            return SimpleNamespace(returncode=0, stderr=b"")
        report = query_digest("binary", "part", "SELECT id FROM part",
                              expected_sha256=hashlib.sha256(raw).hexdigest(),
                              run=execute)
        self.assertEqual(report["bytes"], len(raw))
        self.assertTrue(report["verified"])

    def test_rejects_wrong_reference(self):
        def execute(command, **kwargs):
            kwargs["stdout"].write(b"1\n")
            return SimpleNamespace(returncode=0, stderr=b"")
        with self.assertRaisesRegex(ValueError, "differs"):
            query_digest("binary", "part", "SELECT 1",
                         expected_sha256="0" * 64, run=execute)

    def test_enforces_output_limit(self):
        def execute(command, **kwargs):
            kwargs["stdout"].write(b"abc")
            return SimpleNamespace(returncode=0, stderr=b"")
        with self.assertRaisesRegex(ValueError, "max_output_bytes"):
            query_digest("binary", "part", "SELECT 1",
                         max_output_bytes=2, run=execute)


if __name__ == "__main__":
    unittest.main()
