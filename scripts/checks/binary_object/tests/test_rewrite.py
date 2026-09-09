from pathlib import Path
import subprocess
import tempfile
import unittest

REWRITE = Path(__file__).resolve().parents[1] / 'canaries/rewrite_source.py'


class RewriteTests(unittest.TestCase):
    def test_explicit_file_wins_over_identical_child_expression(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            base = root / 'module.rs'
            child = root / 'module/child.rs'
            child.parent.mkdir()
            base.write_text('target')
            child.write_text('target')
            result = subprocess.run(
                ['python3', str(REWRITE), str(base), 'target', 'changed',
                 '', '', str(root), '', ''], capture_output=True, text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(base.read_text(), 'changed')
            self.assertEqual(child.read_text(), 'target')

    def test_relocated_expression_requires_one_child(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            base = root / 'module.rs'
            child = root / 'module/child.rs'
            child.parent.mkdir()
            base.write_text('mod child;')
            child.write_text('target')
            arguments = ['python3', str(REWRITE), str(base), 'target', 'changed',
                         '', '', str(root), '', '']
            result = subprocess.run(arguments, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(child.read_text(), 'changed')
            child.write_text('target')
            (child.parent / 'other.rs').write_text('target')
            result = subprocess.run(arguments, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('expected one occurrence', result.stderr)
            self.assertEqual(child.read_text(), 'target')
