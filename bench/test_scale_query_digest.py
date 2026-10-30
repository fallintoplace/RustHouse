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

    def test_rejects_wrong_output_size_and_accepts_exact_size(self):
        def execute(command, **kwargs):
            kwargs["stdout"].write(b"123\n")
            return SimpleNamespace(returncode=0, stderr=b"")
        correct = query_digest("binary", "part", "SELECT 123",
                               expected_bytes=4, run=execute)
        self.assertTrue(correct["bytes_verified"])
        self.assertFalse(correct["verified"])
        with self.assertRaisesRegex(ValueError, "byte count differs"):
            query_digest("binary", "part", "SELECT 123", expected_bytes=3,
                         run=execute)
        with self.assertRaisesRegex(ValueError, "expected_bytes"):
            query_digest("binary", "part", "SELECT 123", expected_bytes=True,
                         run=execute)

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

    def test_posix_child_receives_an_online_output_size_limit(self):
        import os
        from unittest.mock import patch
        if os.name != "posix":
            self.skipTest("POSIX resource limits required")
        def execute(command, **kwargs):
            self.assertIn("preexec_fn", kwargs)
            kwargs["stdout"].write(b"ok\n")
            return SimpleNamespace(returncode=0, stderr=b"")
        with patch("bench.scale.query_digest.subprocess.run", side_effect=execute):
            self.assertEqual(query_digest("binary", "part", "SELECT 1")["bytes"], 3)


    def test_refuses_mutations_and_multiple_statements(self):
        def never_run(*_args, **_kwargs):
            self.fail("mutating SQL must be rejected before execution")
        for sql in ("DELETE FROM t", "DROP TABLE t", "SELECT 1; DELETE FROM t", "WITH x AS (SELECT 1) DELETE FROM t"):
            with self.subTest(sql=sql), self.assertRaisesRegex(ValueError, "read-only SELECT"):
                query_digest("rusthouse", "parts", sql, run=never_run)

if __name__ == "__main__":
    unittest.main()
