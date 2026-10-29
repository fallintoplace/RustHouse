import importlib.util
import math
import tempfile
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("ingest_runner", HERE / "ingest.py")
runner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runner)


class IngestBenchmarkTests(unittest.TestCase):
    def test_data_is_reproducible_and_oracle_is_independent(self):
        with tempfile.TemporaryDirectory() as temp:
            a = Path(temp) / "first.tsv"
            b = Path(temp) / "second.tsv"
            expected = runner.write_input(a, 1000, 17)
            self.assertEqual(expected, runner.write_input(b, 1000, 17))
            self.assertEqual(a.read_bytes(), b.read_bytes())
            rows = [
                line.split("\t") for line in a.read_text().splitlines()
            ]
            self.assertEqual(len(rows), 1000)
            self.assertEqual(len({station for station, _ in rows}), 17)
            temperatures = [float(value) for _, value in rows]
            self.assertTrue(math.isclose(min(temperatures), expected[0]))
            self.assertTrue(math.isclose(max(temperatures), expected[1]))
            self.assertTrue(math.isclose(sum(temperatures), expected[2], abs_tol=1e-8))

    def test_verification_rejects_changed_sum_with_matching_count_and_extrema(self):
        valid = ["3", "2", "-1", "1", "0"]
        with mock.patch.object(runner, "sql", side_effect=valid):
            runner.verify(Path("/missing"), Path("/missing"), 3, 2, -1, 1, 0, 1)
        invalid = ["3", "2", "-1", "1", "0.5"]
        with mock.patch.object(runner, "sql", side_effect=invalid):
            with self.assertRaisesRegex(RuntimeError, "sum="):
                runner.verify(Path("/missing"), Path("/missing"), 3, 2, -1, 1, 0, 1)

    def test_large_sums_cannot_hide_small_absolute_errors(self):
        values = ["3", "2", "0", "100000000", "100000000.05"]
        with mock.patch.object(runner, "sql", side_effect=values):
            with self.assertRaisesRegex(RuntimeError, "sum="):
                runner.verify(Path("binary"), Path("db"), 3, 2, 0, 100000000,
                              100000000, 5)

    def test_semicolon_input_uses_the_same_oracle(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "readings.ssv"
            runner.write_input(path, 5, 2, ";")
            self.assertEqual(path.read_text().splitlines()[0].count(";"), 1)

    def test_verifier_rejects_collapsed_station_values(self):
        answers = {
            "SELECT count(*) FROM measurements": "3",
            "SELECT count(DISTINCT station) FROM measurements": "2",
            "SELECT min(temperature) FROM measurements": "-1.0",
            "SELECT max(temperature) FROM measurements": "2.0",
            "SELECT sum(temperature) FROM measurements": "1.0",
        }

        def fake_sql(_binary, _database, statement, _timeout):
            return answers[statement]

        with mock.patch.object(runner, "sql", side_effect=fake_sql):
            runner.verify(Path("binary"), Path("db"), 3, 2, -1.0, 2.0, 1.0, 5)
            answers["SELECT count(DISTINCT station) FROM measurements"] = "1"
            with self.assertRaisesRegex(RuntimeError, "stations=1"):
                runner.verify(Path("binary"), Path("db"), 3, 2, -1.0, 2.0, 1.0, 5)

    def test_ingest_part_inventory_requires_doctor_verification(self):
        healthy = mock.Mock(returncode=0, stdout=b"OK measurements: 5 rows, 2 parts\n")
        with mock.patch.object(runner.subprocess, "run", return_value=healthy):
            self.assertEqual(runner.verified_part_count("rusthouse", ".", 5, 1), 2)
        corrupt = mock.Mock(returncode=0, stdout=b"OK measurements: 4 rows, 2 parts\n")
        with mock.patch.object(runner.subprocess, "run", return_value=corrupt):
            with self.assertRaisesRegex(RuntimeError, "verification"):
                runner.verified_part_count("rusthouse", ".", 5, 1)

    def test_reports_nearest_rank_p90(self):
        result = runner.summarize([0.3, 0.1, 0.2])
        self.assertEqual(result["median_seconds"], 0.2)
        self.assertEqual(result["p90_seconds"], 0.3)

    def test_rejects_invalid_scale_without_running_binary(self):
        with self.assertRaises(ValueError):
            runner.benchmark(Path("/missing"), 2, 3, 3, 1, 1, "TSV")
        with self.assertRaises(ValueError):
            runner.benchmark(Path("/missing"), 3, 1, 0, 1, 1, "TSV")


    def test_buffered_fixture_matches_one_row_reference(self):
        with tempfile.TemporaryDirectory() as temp:
            single = Path(temp) / "single.tsv"
            batched = Path(temp) / "batched.tsv"
            baseline = runner.write_input(single, 10003, 17, batch_rows=1)
            generated = runner.write_input(batched, 10003, 17, batch_rows=4096)
            self.assertEqual(baseline, generated)
            self.assertEqual(single.read_bytes(), batched.read_bytes())
            with self.assertRaisesRegex(ValueError, "batch_rows"):
                runner.write_input(batched, 1, 1, batch_rows=0)


if __name__ == "__main__":
    unittest.main()
