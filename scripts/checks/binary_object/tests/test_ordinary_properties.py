from pathlib import Path
import tempfile
import unittest

from binary_object.context import ScanContext
from binary_object.rules import ordinary_properties, source_setup

ROOT = Path(__file__).resolve().parents[4]


class OrdinaryPropertyContracts(unittest.TestCase):
    def scan(self, edits=()):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for relative in ordinary_properties.FILES:
                source = (ROOT / relative).read_text()
                for path, before, after in edits:
                    if path == relative:
                        self.assertIn(before, source)
                        source = source.replace(before, after)
                target = root / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(source)
            context = ScanContext(root)
            source_setup.check(context)
            ordinary_properties.check(context)
            return context.errors

    def test_current_contracts(self):
        self.assertEqual(self.scan(), [])

    def test_bad_boundaries_are_rejected(self):
        storage, ordinary, dispatch, runtime, heap = ordinary_properties.FILES
        mutations = [
            (storage, "struct OwnSlot", "pub(crate) struct OwnSlot"),
            # Replace every occurrence to simulate removal of the shared class gate.
            (storage, "ObjectKind::Ordinary", "ObjectKind::ModuleNamespace"),
            (storage, "fn locate(", "fn bad() { self.call_internal(); } fn locate("),
            (dispatch, "impl Runtime {", "fn ordinary_set_fast_path_available() {} impl Runtime {"),
            (ordinary, "self.validate_value_domain(&value,", "self.skip_domain(&value,"),
            (ordinary, "rejected_object.as_ref().unwrap_or(receiver)", "receiver"),
            (runtime, "if !failure.published", "if failure.published"),
            (heap, ".retain_edges_transactionally(&new_edges)", ".skip_retain(&new_edges)"),
        ]
        for mutation in mutations:
            with self.subTest(mutation=mutation):
                self.assertTrue(self.scan([mutation]))
