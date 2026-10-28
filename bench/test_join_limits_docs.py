"""Prevent README JOIN limit claims from drifting behind Rust executor constants."""
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def source_limit(name):
    path = ROOT / "crates" / "rusthouse-query" / "src" / "join.rs"
    source = path.read_text(encoding="utf-8")
    match = re.search(rf"pub\(crate\) const {re.escape(name)}:\s*\w+\s*=\s*([\d_]+);", source)
    if not match:
        raise AssertionError(f"missing JOIN limit constant: {name}")
    return int(match.group(1).replace("_", ""))


class JoinDocumentedLimitsTests(unittest.TestCase):
    def test_join_limits_match_executor_source(self):
        document = (ROOT / "README.md").read_text(encoding="utf-8")
        section = document.split("## Bounded SQL joins", 1)[1].split("## Query frozen parts", 1)[0]
        self.assertIn(f"{source_limit('MAX_STREAM_JOIN_BUILD_ROWS'):,} build-side rows", section)
        self.assertIn(f"{source_limit('MAX_BUFFERED_JOIN_ROWS'):,} rows per input", section)
        self.assertIn(f"{source_limit('MAX_JOIN_RESULT_ROWS'):,} matched rows", section)
        mib = 1024 * 1024
        self.assertIn(f"{source_limit('MAX_JOIN_BUILD_BYTES') // mib} MiB", section)
        self.assertIn(f"{source_limit('MAX_JOIN_CHAIN_BYTES') // mib} MiB", section)

    def test_source_limits_have_valid_relationships(self):
        self.assertGreater(
            source_limit('MAX_STREAM_JOIN_BUILD_ROWS'),
            source_limit('MAX_BUFFERED_JOIN_ROWS')
        )
        self.assertGreaterEqual(
            source_limit('MAX_JOIN_CHAIN_BYTES'),
            source_limit('MAX_JOIN_BUILD_BYTES')
        )


if __name__ == "__main__":
    unittest.main()
