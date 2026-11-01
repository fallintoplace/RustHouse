import tempfile
import unittest
from pathlib import Path

from bench.clickbench.run import load_projection


class ProjectionJsonTests(unittest.TestCase):
    def test_rejects_nonstandard_numeric_metadata(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "projection.json"
            for token in ("NaN", "Infinity", "-Infinity"):
                with self.subTest(token=token):
                    path.write_text(
                        '{"available_columns":["id"],"query_columns":{"1":["id"]},"unexpected":' + token + '}',
                        encoding="utf-8",
                    )
                    with self.assertRaisesRegex(ValueError, "nonstandard"):
                        load_projection(path)


if __name__ == "__main__":
    unittest.main()
