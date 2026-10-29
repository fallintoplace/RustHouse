import unittest
from types import SimpleNamespace
from bench.scale.snapshot_gate import inspect_snapshot

class InspectorPayloadTests(unittest.TestCase):
    def test_nonobject_json_is_rejected(self):
        for payload in (b"[]", b"null", b"42", b'"text"'):
            with self.subTest(payload=payload), self.assertRaisesRegex(ValueError, "JSON object"):
                inspect_snapshot("rusthouse", ".", run=lambda *args, **kwargs:
                                 SimpleNamespace(returncode=0, stdout=payload, stderr=b""))

if __name__ == "__main__":
    unittest.main()
