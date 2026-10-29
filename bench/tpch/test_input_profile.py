import hashlib
import tempfile
import unittest
from pathlib import Path

from schema import names
from input_profile import profile_table, profile_directory


class InputProfileTests(unittest.TestCase):
    def test_sha256_rows_and_q6_selectivity(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "lineitem.tsv"
            header = names("lineitem")
            values = dict.fromkeys(header, "1")
            values.update(l_shipdate="1994-05-12", l_discount="600",
                          l_quantity="2300", l_extendedprice="1200")
            text = "\t".join(header) + "\n" + "\t".join(values[h] for h in header) + "\n"
            path.write_bytes(text.encode("utf-8"))
            result = profile_table(path, "lineitem")
            self.assertEqual(result["rows"], 1)
            self.assertEqual(result["q6_predicate_matches"], 1)
            self.assertEqual(result["q6_selectivity_percent"], 100.0)
            self.assertEqual(result["sha256"], hashlib.sha256(text.encode()).hexdigest())
            self.assertEqual(profile_directory(root, ["lineitem"])["total_rows"], 1)

    def test_rejects_empty_duplicate_and_unknown_selections(self):
        for selected in ([], ["region", "region"], ["not_a_table"]):
            with self.subTest(selected=selected):
                with self.assertRaisesRegex(ValueError, "unique"):
                    profile_directory("/nonexistent", selected)

    def test_rejects_unterminated_final_record(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "region.tsv"
            source.write_text("\t".join(names("region")) + "\n1\tEUROPE\tcomment")
            with self.assertRaisesRegex(ValueError, "truncated"):
                profile_table(source, "region")

    def test_rejects_wrong_schema(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "region.tsv"
            path.write_text("bad\n1\n")
            with self.assertRaisesRegex(ValueError, "header"):
                profile_table(path, "region")


if __name__ == "__main__":
    unittest.main()
