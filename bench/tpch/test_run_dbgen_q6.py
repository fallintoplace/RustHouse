import tempfile
import unittest
from pathlib import Path
from run_dbgen_q6 import run_pipeline


class DbgenPipelineTests(unittest.TestCase):
    def test_converts_then_loads_then_runs_reference_checked_q6(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root)
            (path / "lineitem.tbl").write_text("placeholder\n")
            calls = []
            def convert(source, converted, selected):
                calls.append(("convert", selected))
                return {"lineitem": 2}
            def load(binary, db, converted, selected, timeout):
                calls.append(("load", selected))
                return {"tables": [{"table": "lineitem", "rows": 2}]}
            def q6(binary, db, path, runs, warmups, timeout):
                calls.append(("q6", runs))
                self.assertEqual(path.name, "lineitem.tsv")
                return {"coverage": {"verified_queries": 1}}
            report = run_pipeline("binary", path / "db", path, path / "converted",
                                  converter=convert, loader=load, runner=q6)
            self.assertEqual(calls, [("convert", ["lineitem"]),
                                     ("load", ["lineitem"]), ("q6", 5)])
            self.assertEqual(report["rows"], 2)
            self.assertFalse(report["official_tpch_score"])

    def test_refuses_old_database_and_missing_input(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root)
            with self.assertRaisesRegex(ValueError, "lineitem.tbl"):
                run_pipeline("binary", path / "db", path, path / "converted")
            (path / "lineitem.tbl").write_text("row\n")
            (path / "db").mkdir()
            with self.assertRaisesRegex(ValueError, "fresh database"):
                run_pipeline("binary", path / "db", path, path / "converted")

    def test_rejects_inconsistent_counts(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root)
            (path / "lineitem.tbl").write_text("placeholder\n")
            with self.assertRaisesRegex(ValueError, "row count differs"):
                run_pipeline("binary", path / "db", path, path / "out",
                             converter=lambda *args, **kwargs: {"lineitem": 3},
                             loader=lambda *args, **kwargs: {"tables": [{"rows": 2}]})


if __name__ == "__main__":
    unittest.main()
