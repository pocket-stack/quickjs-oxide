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

    def test_tail_stack_canary_reaches_private_and_published_descriptor(self):
        """The real shell negative must mutate the moved helper, then fail its seal."""
        import codecs
        import re
        from binary_object.context import ScanContext
        from binary_object.rules import source_setup

        checker = Path(__file__).resolve().parents[1]
        repository = Path(__file__).resolve().parents[4]
        shell = (checker / 'canaries/translation.sh').read_text()
        block = shell.split('expect_full_rewrite_rejected stage3c-stack-effect-guarded-bypass', 1)[1].split('expect_full_rewrite_rejected', 1)[0]
        strings = re.findall(r"\$'((?:\\.|[^'])*)'", block)
        self.assertEqual(len(strings), 2)
        before, after = [codecs.decode(text, 'unicode_escape') for text in strings]
        original = (repository / 'src/engine/code/instruction.rs').read_text()
        declaration = r'(?:pub\(crate\) )?const fn nominal_stack_effect'
        self.assertEqual(len(re.findall(declaration, original)), 1)
        for visibility in ('', 'pub(crate) '):
            with self.subTest(visibility=visibility), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                base = root / 'src/engine/code/bytecode.rs'
                owner = root / 'src/engine/code/instruction.rs'
                owner.parent.mkdir(parents=True)
                base.write_text('// descriptor lives in its explicit owner\n')
                source = re.sub(declaration, visibility + 'const fn nominal_stack_effect', original)
                owner.write_text(source)
                result = subprocess.run(
                    ['python3', str(REWRITE), str(base), before, after, '', '', str(root), '', ''],
                    capture_output=True, text=True,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                modified = owner.read_text()
                self.assertNotEqual(modified, source)
                self.assertIn(visibility + after, modified)
                self.assertEqual(base.read_text(), '// descriptor lives in its explicit owner\n')
                ctx = ScanContext(root)
                source_setup.check(ctx)
                original_item = ctx.unique_braced_item(
                    ctx.rust_code_only(source),
                    re.compile(r'\bfn\s+nominal_stack_effect\b[^{};]*\{'),
                    'stage3c-tail-verifier', 'unmodified nominal stack effect',
                )[0]
                self.assertEqual(ctx.errors, [])
                self.assertEqual(
                    ctx.normalized_code_sha256(original_item),
                    '7fa361fbe20e888631a7c8330138fc00ad162c05f8cbcca476d1b235149dd957',
                )
                item = ctx.unique_braced_item(
                    ctx.rust_code_only(modified),
                    re.compile(r'\bfn\s+nominal_stack_effect\b[^{};]*\{'),
                    'stage3c-tail-verifier', 'shared nominal stack effect',
                )[0]
                ctx.require_normalized_code_sha256(
                    'stage3c-tail-verifier', 'shared exhaustive nominal model', item,
                    '7fa361fbe20e888631a7c8330138fc00ad162c05f8cbcca476d1b235149dd957',
                )
                self.assertTrue(any('stage3c-tail-verifier' in error for error in ctx.errors))
