import tempfile
import unittest
from pathlib import Path

from bench.common import file_change_time_ns
from bench.scale import snapshot_fingerprint, tsv_profile


class PortableOfflineFingerprintTests(unittest.TestCase):
    def test_both_offline_readers_share_the_portable_change_time(self):
        self.assertIs(snapshot_fingerprint.file_change_time_ns, file_change_time_ns)
        self.assertIs(tsv_profile.file_change_time_ns, file_change_time_ns)

    def test_offline_snapshot_and_tsv_profile_are_unchanged_on_posix(self):
        with tempfile.TemporaryDirectory() as root:
            base = Path(root)
            source = base / "rows.tsv"
            source.write_bytes(b"1\n2\n3\n")
            profile = tsv_profile.profile(source, verify_sorted_uint64=True)
            self.assertEqual(profile["rows"], 3)
            self.assertEqual(profile["bytes"], 6)
            snapshot = snapshot_fingerprint.fingerprint(base)
            self.assertEqual(snapshot["files"], 1)
            self.assertEqual(snapshot["bytes"], 6)


if __name__ == "__main__":
    unittest.main()
