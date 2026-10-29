import hashlib
import tempfile
import unittest
from pathlib import Path

from canonical_integrity import git_blob_hash, source_bytes, verify


class CanonicalIntegrityTests(unittest.TestCase):
    def test_matches_pinned_hash_after_provenance(self):
        sql = b"SELECT 42;\n"
        header = (b"-- Full TPC-H Q1 validation query. Source: pinned\n"
                  b"-- Copyright 2018-2026 Stichting DuckDB Foundation. SPDX-License-Identifier: MIT\n")
        self.assertEqual(source_bytes(header + sql), sql)
        with tempfile.TemporaryDirectory() as root:
            file = Path(root) / "q01.sql"
            file.write_bytes(header + sql)
            result = verify(root, {"q01.sql": git_blob_hash(sql)})
            self.assertEqual(result["query_count"], 1)
            self.assertFalse(result["canonical_sql_verified"])

    def test_rejects_modified_sql_or_unexpected_query(self):
        sql = b"SELECT 42;\n"
        header = (b"-- Full TPC-H Q1 validation query. Source: pinned\n"
                  b"-- SPDX-License-Identifier: MIT\n")
        with tempfile.TemporaryDirectory() as root:
            (Path(root) / "q01.sql").write_bytes(header + b"SELECT 0;\n")
            with self.assertRaisesRegex(ValueError, "differs"):
                verify(root, {"q01.sql": git_blob_hash(sql)})
            (Path(root) / "q02.sql").write_bytes(header + sql)
            with self.assertRaisesRegex(ValueError, "extra"):
                verify(root, {"q01.sql": git_blob_hash(sql)})

    def test_rejects_missing_attribution(self):
        with self.assertRaisesRegex(ValueError, "provenance"):
            source_bytes(b"SELECT 42;\n")
        with self.assertRaisesRegex(ValueError, "license"):
            source_bytes(b"-- Full TPC-H Q1\n-- no license\nSELECT 42;\n")


if __name__ == "__main__":
    unittest.main()
