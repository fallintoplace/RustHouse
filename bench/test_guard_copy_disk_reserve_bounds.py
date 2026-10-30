import unittest

from bench.scale.copy_disk_pressure_guard import guard


class DiskPressureControlInputTests(unittest.TestCase):
    def test_rejects_invalid_disk_reserve_before_io(self):
        for invalid in (True, "1", float("nan"), float("inf"), 0, -1, 1e-30, 1e300):
            with self.subTest(reserve=invalid), self.assertRaisesRegex(
                    ValueError, "min-free-gib"):
                guard("binary", "db", "events", "/missing", "/missing",
                      min_free_gib=invalid)

    def test_rejects_invalid_time_and_poll_settings_before_io(self):
        for kwargs in (
            {"timeout": True}, {"timeout": "5"}, {"timeout": float("inf")},
            {"poll_seconds": True}, {"poll_seconds": "1"},
            {"poll_seconds": float("nan")}, {"poll_seconds": 0.01},
        ):
            with self.subTest(kwargs=kwargs), self.assertRaises(ValueError):
                guard("binary", "db", "events", "/missing", "/missing", **kwargs)


if __name__ == "__main__":
    unittest.main()
