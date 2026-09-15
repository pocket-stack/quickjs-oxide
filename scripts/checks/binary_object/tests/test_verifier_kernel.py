"""Mutation canaries for the wrapper -> generic kernel -> compact state seam."""
from pathlib import Path
import tempfile
import unittest

from binary_object.context import ScanContext
from binary_object.rules import source_setup, verifier_kernel

ROOT = Path(__file__).resolve().parents[4]
SOURCE = 'src/engine/code/bytecode.rs'


class VerifierKernelContracts(unittest.TestCase):
    def scan(self, before=None, after=None):
        code = (ROOT / SOURCE).read_text()
        if before is not None:
            self.assertIn(before, code)
            code = code.replace(before, after)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / SOURCE
            target.parent.mkdir(parents=True)
            target.write_text(code)
            ctx = ScanContext(root)
            source_setup.check(ctx)
            ctx.read_source = lambda relative: (root / relative).read_text()
            kernel = verifier_kernel.resolve(ctx)
            ctx.require_normalized_code_sha256('typed-kernel', 'full control-flow kernel', kernel,
                ('055bb6802b9422b7ab996d0506ff7f75b5b1aa9251cdadb22e335df7135974e3'
                 if ctx.verifier_kernel_kind == 'compact' else
                 'ef9fe333359c127175f3a83bd4c20996702ca11d581096a205ffeb4e30fafe41'))
            return ctx.errors

    def test_current_wrapper_resolves_actual_kernel(self):
        self.assertEqual(self.scan(), [])

    def test_bypass_or_misrouted_inputs_are_rejected(self):
        # Both frozen stages run this same test module. Mutate the actual
        # representation, preserving join/history/FIFO/terminal coverage in each.
        compact = 'fn verify_parts_with_visits' in (ROOT / SOURCE).read_text()
        mutations = [
            ('worklist.pop_front()', 'worklist.pop_back()'),
            ('| Instruction::Throw\n            | Instruction::Ret => {}', '| Instruction::Throw\n            | Instruction::Ret => { enqueue_fallthrough(&mut worklist, pc, state.clone(), code.len())?; }'),
            ('Instruction::ThrowReadOnly(_)\n            | Instruction::ThrowRedeclaration(_)', 'Instruction::Nop\n            | Instruction::ThrowReadOnly(_)\n            | Instruction::ThrowRedeclaration(_)'),
        ]
        if compact:
            mutations += [
                ('verify_parts_with_visits::<CompactVisits>(code, constant_count, declared_max_stack)', 'Ok(VerifiedBytecode { max_stack: 0 })'),
                ('verify_parts_with_visits::<CompactVisits>(code, constant_count, declared_max_stack)', 'verify_parts_with_visits::<CompactVisits>(&[], constant_count, declared_max_stack)'),
                ('if !states.visit(pc, &state)? {', 'if false {'),
                ('depths: vec![usize::MAX; length]', 'depths: vec![0; length]'),
                ('if *depth != state.depth {', 'if false {'),
                ('!= state.return_addresses.as_slice()', '== state.return_addresses.as_slice()'),
                ('!= state.super_call_bases.as_slice()', '== state.super_call_bases.as_slice()'),
                ('states[pc] = Some(state.clone());', 'states[pc] = None;'),
            ]
        else:
            mutations += [
                ('pub fn verify_parts(', 'pub fn unused_verify_parts('),
                ('if previous != &state {', 'if false {'),
                ('previous.depth != state.depth', 'previous.depth == state.depth'),
                ('previous.regions != state.regions', 'previous.regions == state.regions'),
                ('previous.return_addresses != state.return_addresses', 'previous.return_addresses == state.return_addresses'),
                ('*slot = Some(state.clone());', '*slot = None;'),
            ]
        for before, after in mutations:
            with self.subTest(before=before):
                self.assertTrue(self.scan(before, after))
