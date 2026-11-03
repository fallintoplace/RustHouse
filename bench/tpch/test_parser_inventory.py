import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from subprocess import CompletedProcess

from parser_inventory import inventory, parse_statement


class ParserInventoryTests(unittest.TestCase):
    def test_counts_only_parsed_queries_and_preserves_order(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root)
            (source / "q01.sql").write_text("-- provenance\nSELECT 1;\n")
            (source / "q06.sql").write_text("SELECT failure;")
            seen = []

            def parse(sql):
                seen.append(sql)
                return (sql == "SELECT 1", "expected parser error")

            report = inventory("unused", source, parse=parse)
            self.assertEqual(report["parsed_queries"], 1)
            self.assertEqual(report["total_queries"], 22)
            self.assertFalse(report["official_tpch_score"])
            self.assertEqual([r["query"] for r in report["results"]], list(range(1, 23)))
            self.assertEqual([report["results"][0]["status"], report["results"][5]["status"]],
                             ["parsed", "parse_error"])
            self.assertEqual(report["results"][5]["error"], "expected parser error")
            self.assertEqual(report["results"][1]["status"], "missing_query")
            self.assertEqual(seen, ["SELECT 1", "SELECT failure"])

    def test_rejects_symlinked_and_empty_queries_without_parser_call(self):
        with tempfile.TemporaryDirectory() as root:
            directory = Path(root)
            (directory / "source.sql").write_text("SELECT 1")
            (directory / "q01.sql").symlink_to(directory / "source.sql")
            (directory / "q02.sql").write_text("   ; ")
            seen = []
            report = inventory("unused", directory,
                               parse=lambda sql: seen.append(sql) or (True, None))
            self.assertEqual(report["results"][0]["status"], "invalid_query")
            self.assertEqual(report["results"][1]["status"], "invalid_query")
            self.assertEqual(seen, [])

    def test_parse_statement_rejects_empty_success_output(self):
        with patch("parser_inventory.subprocess.run", return_value=CompletedProcess([], 0)):
            parsed, error = parse_statement("unused", "SELECT 1")
        self.assertFalse(parsed)
        self.assertIn("without an AST", error)

    def test_parse_statement_refuses_invalid_timeout(self):
        for timeout in (0, -1, float("nan"), float("inf"), True):
            with self.subTest(timeout=timeout):
                with self.assertRaisesRegex(ValueError, "timeout"):
                    parse_statement("unused", "SELECT 1", timeout=timeout)


if __name__ == "__main__":
    unittest.main()
