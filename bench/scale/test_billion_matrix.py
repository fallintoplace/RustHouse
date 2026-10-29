"""Unit checks for the billion-row query matrix SQL/oracles."""
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

from bench.scale.billion_matrix import (CASE_DIRECTORY, benchmark, compile_case,
                                        load_cases, verify_database)


class TestBillionMatrix(unittest.TestCase):
    def test_all_cases_compile_at_hundred_thousand_and_billion(self):
        cases = load_cases(CASE_DIRECTORY)
        self.assertGreaterEqual(len(cases), 1)
        self.assertEqual(len({case['name'] for case in cases}), len(cases))
        for rows in (100_000, 1_000_000_000, 2_000_000_000, 4_000_000_000):
            for case in cases:
                with self.subTest(case=case['name'], rows=rows):
                    sql, expected, matched = compile_case(case, rows)
                    self.assertTrue(sql.startswith('SELECT '))
                    self.assertTrue(expected.endswith(b'\n'))
                    self.assertGreaterEqual(matched, 0)

    def test_modulo_count_and_sum(self):
        count = dict(name='mod', kind='modulo', aggregate='count', modulus=3, residue=2)
        total = dict(count, aggregate='sum')
        self.assertEqual(compile_case(count, 100_000)[1], b'33333\n')
        self.assertEqual(compile_case(total, 100_000)[1], b'1666650000\n')

    def test_interval_and_range_modulo(self):
        case = dict(name='r', kind='range', aggregate='sum', lower_ppm=100_000,
                    upper_ppm=200_000)
        self.assertEqual(compile_case(case, 100_000)[1], b'149995000\n')
        case.update(kind='range-modulo', modulus=2, residue=0)
        self.assertEqual(compile_case(case, 100_000)[1], b'74995000\n')

    def test_membership_and_exclusion(self):
        case = dict(name='points', kind='membership', aggregate='count', ids=[0, 7, 65536])
        self.assertEqual(compile_case(case, 100_000)[1], b'3\n')
        case['exclude'] = True
        self.assertEqual(compile_case(case, 100_000)[1], b'99997\n')

    def test_filtered_descending_page(self):
        case = dict(name='page', kind='page', direction='DESC', limit=3, offset=2,
                    modulus=4, residue=3)
        self.assertEqual(compile_case(case, 100_000)[1], b'99991\n99987\n99983\n')

    def test_fail_closed_on_bad_inputs(self):
        cases = [
            dict(name='bad', kind='modulo', aggregate='sum', modulus=0, residue=0),
            dict(name='bad', kind='membership', aggregate='sum', ids=[3, 3]),
            dict(name='bad', kind='range', aggregate='count', lower_ppm=100, upper_ppm=100),
            dict(name='bad', kind='page', direction='DESC', limit=0, offset=0),
            dict(name='Bad Name', kind='range', aggregate='count', lower_ppm=0, upper_ppm=1_000_000),
            dict(name='bad', kind='unknown', aggregate='count'),
        ]
        for case in cases:
            with self.subTest(case=case):
                with self.assertRaises(ValueError):
                    compile_case(case, 100_000)

    def test_case_loader_bounds_json_before_reading(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "01-big.json"
            path.write_bytes(b"x" * (64 * 1024 + 1))
            with self.assertRaisesRegex(ValueError, "64 KiB"):
                load_cases(root)
            self.assertFalse(path.is_symlink())

    def test_doctor_requires_real_row_count(self):
        def doctor(command, **_):
            return SimpleNamespace(returncode=0, stdout=b'OK events: 100000 rows, 2 parts\n', stderr=b'')
        self.assertTrue(verify_database('bin', 'db', 100_000, execute=doctor))
        with self.assertRaises(ValueError):
            verify_database('bin', 'db', 1_000_000_000, execute=doctor)

    def test_benchmark_checks_each_query_result(self):
        case = dict(name='mod', kind='modulo', aggregate='count', modulus=2, residue=0)
        with tempfile.TemporaryDirectory() as root:
            Path(root, '01-mod.json').write_text(json.dumps(case))
            calls = []
            def execute(command, **_):
                calls.append(command)
                output = b'OK events: 100000 rows, 2 parts\n' if command[1] == 'doctor' else b'50000\n'
                return SimpleNamespace(returncode=0, stdout=output, stderr=b'')
            report = benchmark('bin', 'db', 100_000, root, runs=1, warmups=0, execute=execute)
            self.assertTrue(report['verified'])
            self.assertEqual(report['case_count'], 1)
            self.assertEqual(len(calls), 2)
            self.assertEqual(calls[1][-2:], ['--format', 'tsv-no-header'])

    def test_benchmark_fails_on_wrong_answer(self):
        case = dict(name='mod', kind='modulo', aggregate='count', modulus=2, residue=0)
        with tempfile.TemporaryDirectory() as root:
            Path(root, '01-mod.json').write_text(json.dumps(case))
            def execute(command, **_):
                output = b'OK events: 100000 rows, 2 parts\n' if command[1] == 'doctor' else b'49999\n'
                return SimpleNamespace(returncode=0, stdout=output, stderr=b'')
            with self.assertRaisesRegex(ValueError, 'wrong output'):
                benchmark('bin', 'db', 100_000, root, runs=1, warmups=0, execute=execute)


if __name__ == '__main__':
    unittest.main()
