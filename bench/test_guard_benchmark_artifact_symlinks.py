import sys
import tempfile
import unittest
from pathlib import Path

from bench.test_validate_benchmark import sample_manifest, verify


class ArtifactSymlinkTests(unittest.TestCase):
    @unittest.skipIf(sys.platform == "win32", "Windows CI does not allow unprivileged symlinks")
    def test_rejects_measurement_symlink(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            manifest = sample_manifest(directory)
            measurement = directory / manifest["results"][0]["stdout_file"]
            target = directory / "other.txt"
            target.write_text("valid-looking measurement\n")
            measurement.unlink()
            measurement.symlink_to(target)
            with self.assertRaisesRegex(ValueError, "measurements"):
                verify(directory, manifest)

    @unittest.skipIf(sys.platform == "win32", "Windows CI does not allow unprivileged symlinks")
    def test_rejects_diagnostic_symlink(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            manifest = sample_manifest(directory)
            diagnostics = directory / manifest["results"][0]["stderr_file"]
            target = directory / "other.stderr"
            target.write_text("")
            diagnostics.unlink()
            diagnostics.symlink_to(target)
            with self.assertRaisesRegex(ValueError, "diagnostics"):
                verify(directory, manifest)


if __name__ == "__main__":
    unittest.main()
