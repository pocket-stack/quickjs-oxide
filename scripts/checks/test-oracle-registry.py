#!/usr/bin/env python3
"""Regression tests for source-tree registration, independent of the real corpus."""
import contextlib
import importlib.util
import io
import tempfile
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

sys.dont_write_bytecode = True

spec = importlib.util.spec_from_file_location(
    'oracle_registry', Path(__file__).with_name('check-oracle-registry.py')
)
registry = importlib.util.module_from_spec(spec)
spec.loader.exec_module(registry)


class OracleRegistryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='qjo-oracle-registry-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.oracle = self.root / 'apps/cli/tests/oracle'
        self.oracle.mkdir(parents=True)
        self.write('main.rs', 'mod ordinary;\n' + ''.join(
            '#[cfg(feature = "test262-host")]\nmod ' + name + ';\n'
            for name in sorted(registry.HOST_MODULES)
        ))
        self.write('ordinary.rs', '#[test]\nfn ordinary_case() {}\n')
        for name in registry.HOST_MODULES:
            self.write(name + '.rs', '#[test]\nfn host_case() {}\n')
        self.addCleanup(patch.stopall)
        patch.object(registry, 'ROOT', self.root).start()
        patch.object(registry, 'ORACLE', self.oracle).start()

    def write(self, name, source):
        path = self.oracle / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(source)

    def inventory(self):
        with contextlib.redirect_stdout(io.StringIO()):
            return registry.inventory()

    def test_new_registered_topic_needs_no_fixed_count_update(self):
        self.write('ordinary.rs', 'mod nested;\n#[test]\nfn first() {}\n')
        self.write('ordinary/nested.rs', '#[test]\nfn second() {}\n')
        self.assertEqual(self.inventory(), {False: 2, True: 3})

    def test_unregistered_source_is_rejected(self):
        self.write('forgotten.rs', '#[test]\nfn lost() {}\n')
        with self.assertRaisesRegex(SystemExit, 'unregistered oracle sources'):
            self.inventory()

    def test_missing_host_gate_is_rejected(self):
        main = self.oracle / 'main.rs'
        main.write_text(main.read_text().replace('#[cfg(feature = "test262-host")]\n', '', 1))
        with self.assertRaisesRegex(SystemExit, 'must be gated'):
            self.inventory()

    def test_ambiguous_module_is_rejected(self):
        self.write('ordinary/mod.rs', '')
        with self.assertRaisesRegex(SystemExit, 'exactly one source'):
            self.inventory()

    def test_missing_module_is_rejected(self):
        (self.oracle / 'ordinary.rs').unlink()
        with self.assertRaisesRegex(SystemExit, 'exactly one source'):
            self.inventory()

    def test_conditional_exclusion_is_rejected(self):
        main = self.oracle / 'main.rs'
        main.write_text('#[cfg(any())]\n' + main.read_text())
        with self.assertRaisesRegex(SystemExit, 'unexpected module gate'):
            self.inventory()

    def test_symlinked_source_is_rejected(self):
        (self.oracle / 'ordinary.rs').unlink()
        (self.root / 'outside.rs').write_text('')
        (self.oracle / 'ordinary.rs').symlink_to(self.root / 'outside.rs')
        with self.assertRaisesRegex(SystemExit, 'regular file'):
            self.inventory()

    def test_legacy_top_level_wrapper_is_rejected(self):
        (self.root / 'apps/cli/tests/oracle_legacy.rs').write_text('')
        with self.assertRaisesRegex(SystemExit, 'must live under apps/cli/tests/oracle'):
            self.inventory()


if __name__ == '__main__':
    unittest.main()
