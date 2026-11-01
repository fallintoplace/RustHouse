import os
import shutil
from pathlib import Path
import tempfile
import unittest
from offline_vacuum import active_label, plan, vacuum


class OfflineVacuumTests(unittest.TestCase):
    def fixture(self, root, active="g2"):
        database = Path(root)
        parts = database / "events.parts"
        versions = parts / ".rusthouse-versions"
        versions.mkdir(parents=True)
        (database / "events.schema").write_text("rusthouse-schema-v1\nid\tUInt64\n")
        (parts / ".rusthouse-active").write_text(active + "\n")
        for number in range(1, 6):
            (versions / f"g{number}").mkdir()
        return parts, versions

    def test_retains_active_and_newest_generations(self):
        with tempfile.TemporaryDirectory() as root:
            parts, versions = self.fixture(root)
            candidates = plan(root, keep=2)
            self.assertEqual([p.name for _, _, p in candidates], ["g1", "g3"])
            self.assertEqual(active_label(parts), "g2")
            self.assertTrue(all((versions / f"g{i}").exists() for i in range(1, 6)))

    def test_bad_pointers_fail_closed(self):
        with tempfile.TemporaryDirectory() as root:
            parts, _ = self.fixture(root)
            for invalid in ("g0\n", "g01\n", "g6\n", "g2", "../g2\n"):
                (parts / ".rusthouse-active").write_text(invalid)
                with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                    plan(root)

    def test_refuses_symlinked_generation_directories(self):
        if os.name != "posix":
            self.skipTest("POSIX symlink fixture")
        with tempfile.TemporaryDirectory() as root:
            _parts, versions = self.fixture(root)
            (versions / "g6").symlink_to(versions / "g5", target_is_directory=True)
            with self.assertRaisesRegex(ValueError, "unexpected"):
                plan(root)

    def test_generation_limit_fails_before_scanning_remaining_entries(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as root:
            _parts, versions = self.fixture(root)
            real_iterdir = Path.iterdir
            def bounded_entries(path):
                for index, entry in enumerate(real_iterdir(path)):
                    if path == versions and index >= 3:
                        raise AssertionError("generation scan exceeded early bound")
                    yield entry
            with patch("offline_vacuum.MAX_GENERATIONS", 2), patch(
                "offline_vacuum.Path.iterdir", autospec=True, side_effect=bounded_entries
            ):
                with self.assertRaisesRegex(ValueError, "invalid generation inventory"):
                    plan(root, keep=2)

    def test_pending_entries_consume_scan_budget(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as root:
            _parts, versions = self.fixture(root)
            (versions / ".pending-incomplete").mkdir()
            with patch("offline_vacuum.MAX_GENERATIONS", 5):
                with self.assertRaisesRegex(ValueError, "invalid generation inventory"):
                    plan(root, keep=2)

    @unittest.skipUnless(os.name == "posix" and shutil.rmtree.avoids_symlink_attacks,
                         "requires symlink-safe POSIX rmtree")
    def test_deletion_budget_blocks_all_candidates_before_mutation(self):
        with tempfile.TemporaryDirectory() as root:
            _parts, versions = self.fixture(root)
            (versions / "g1" / "payload.bin").write_bytes(b"x" * 10)
            before = sorted(path.name for path in versions.iterdir())
            with self.assertRaisesRegex(ValueError, "max_delete_bytes"):
                vacuum(root, keep=2, execute=True, offline_confirmed=True,
                       max_delete_bytes=9)
            self.assertEqual(before, sorted(path.name for path in versions.iterdir()))
            with self.assertRaisesRegex(ValueError, "max_delete_entries"):
                vacuum(root, keep=2, execute=True, offline_confirmed=True,
                       max_delete_entries=1)
            self.assertEqual(before, sorted(path.name for path in versions.iterdir()))

    def test_execute_needs_explicit_offline_confirmation(self):
        with tempfile.TemporaryDirectory() as root:
            self.fixture(root)
            with self.assertRaisesRegex(ValueError, "offline-confirmed"):
                vacuum(root, keep=2, execute=True)
            self.assertEqual(len(plan(root, keep=2)), 2)

    @unittest.skipUnless(os.name == "posix" and shutil.rmtree.avoids_symlink_attacks,
                         "requires symlink-safe POSIX rmtree")
    def test_vacuum_only_deletes_unpinned_obsolete_generations(self):
        with tempfile.TemporaryDirectory() as root:
            parts, versions = self.fixture(root)
            removed = vacuum(root, keep=2, execute=True, offline_confirmed=True)
            self.assertEqual([p.name for _, _, p in removed], ["g1", "g3"])
            self.assertEqual(sorted(e.name for e in versions.iterdir()), ["g2", "g4", "g5"])
            self.assertEqual(active_label(parts), "g2")


if __name__ == "__main__":
    unittest.main()
