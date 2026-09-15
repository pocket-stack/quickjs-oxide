"""Manifest admission must reject stale or ambiguous fixed-work experiments."""
import json
import tempfile
import unittest
from pathlib import Path
from fixed import load_workloads
from run import digest


class ManifestTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.source = self.directory / 'loop.js'
        self.source.write_text('print(42);\n')
        self.item = dict(case='loop', path='/missing/loop.js', sha256=digest(self.source),
                         size=1, expected='42\n')
        self.report = self.directory / 'report.json'

    def write(self, items):
        self.report.write_text(json.dumps(dict(metadata=dict(workloads=dict(workloads=items)))))

    def test_relocation_preserves_byte_identity(self):
        self.write([self.item])
        result = load_workloads(self.report, self.directory, ['loop'])
        self.assertEqual(result[0]['path'], str(self.source))
        self.assertEqual(result[0]['expected'], '42\n')

    def test_changed_workload_is_rejected(self):
        self.write([self.item])
        self.source.write_text('print(41);\n')
        with self.assertRaisesRegex(ValueError, 'bytes changed'):
            load_workloads(self.report, self.directory)

    def test_missing_or_repeated_selection_is_rejected(self):
        self.write([self.item])
        for cases in [['missing'], ['loop', 'loop']]:
            with self.subTest(cases=cases), self.assertRaises(ValueError):
                load_workloads(self.report, self.directory, cases)

    def test_ambiguous_or_unsafe_manifest_is_rejected(self):
        for items in [[self.item, self.item], [dict(self.item, case='../loop')], []]:
            self.write(items)
            with self.subTest(items=items), self.assertRaises(ValueError):
                load_workloads(self.report, self.directory)
