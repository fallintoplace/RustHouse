import tempfile
import unittest
from pathlib import Path

from parser_inventory import inventory


class GroupedParserDiagnosticTests(unittest.TestCase):
    def test_groups_errors_without_claiming_queries_are_parsed(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for n in (1, 3, 6, 12):
                (root / f"q{n:02d}.sql").write_text(f"SELECT {n};\n")

            def parse(sql):
                if sql == "SELECT 6":
                    return True, None
                if sql == "SELECT 12":
                    return False, "grouped CASE requires THEN"
                return False, "expected ')' after grouped aggregate"

            report = inventory("unused", root, parse=parse)
            self.assertEqual(report["parsed_queries"], 1)
            self.assertEqual(report["first_error_groups"], [
                {"error": "expected ')' after grouped aggregate", "count": 2, "queries": [1, 3]},
                {"error": "grouped CASE requires THEN", "count": 1, "queries": [12]},
            ])
            self.assertEqual(report["results"][0]["status"], "parse_error")
            self.assertEqual(report["results"][5]["status"], "parsed")


if __name__ == "__main__":
    unittest.main()
