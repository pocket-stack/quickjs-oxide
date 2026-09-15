"""Resolve and authenticate the public verifier's shared visitation kernel.

Terminal/ordinary-successor checks must inspect the kernel, not its public
wrapper. Compact state storage remains part of the trust boundary: a skipped
join check is rejected even when the control-flow kernel has not changed.
"""
import re


def resolve(ctx):
    code = ctx.rust_code_only(ctx.read_source('src/engine/code/bytecode.rs'))
    def item(pattern, description):
        return ctx.unique_braced_item(code, re.compile(pattern),
                                     'published-verifier-kernel', description)[0]
    wrapper = item(r'\bfn\s+verify_parts\b[^{};]*\{', 'public verifier wrapper')
    if not re.search(r'\bfn\s+verify_parts_with_visits\b', code):
        # S08 still owns the reviewed full visitation implementation directly.
        # An absent helper is not an exemption: the complete old verifier must
        # authenticate, including all joins and FIFO/terminal/fallthrough logic.
        ctx.verifier_kernel_kind = 'full'
        ctx.require_normalized_code_sha256(
            'published-verifier-kernel', 'S08 must retain the complete reviewed full-state verifier',
            wrapper, 'ef9fe333359c127175f3a83bd4c20996702ca11d581096a205ffeb4e30fafe41')
        return wrapper
    ctx.verifier_kernel_kind = 'compact'
    expected = ('fn verify_parts( code: &[Instruction], constant_count: usize, '
                'declared_max_stack: u16, ) -> Result<VerifiedBytecode, Error> { '
                'verify_parts_with_visits::<CompactVisits>(code, constant_count, declared_max_stack) }')
    if ' '.join(wrapper.split()) != expected:
        ctx.fail('published-verifier-kernel',
                 'verify_parts must forward all exact inputs to the CompactVisits kernel without bypass')
    kernel = item(r'\bfn\s+verify_parts_with_visits\s*<V:\s*VerificationVisits>[^{};]*\{',
                  'shared typed verifier kernel')
    ctx.require_ordered_fragments(
        'published-verifier-kernel', 'all instructions must be validated before FIFO reachability; every state must authenticate its depth and join before instruction indexing', kernel,
        ('for (pc, instruction) in code.iter().enumerate()',
         'let mut states = V::new(code.len());',
         'while let Some((pc, state)) = worklist.pop_front()',
         'record_maximum_depth(&mut maximum, state.depth, declared_max_stack)?;',
         'if !states.visit(pc, &state)? { continue; }',
         'let instruction = &code[pc];'))
    visits = item(r'\bimpl\s+VerificationVisits\s+for\s+CompactVisits\s*\{',
                  'production compact visits implementation')
    ctx.require_normalized_code_sha256(
        'published-verifier-kernel',
        'compact visits must retain unvisited sentinel, bounds checks, depth/unwind/gosub/super join precedence and exceptional-state retention', visits,
        '8dc1de689cf27ed97b29542ef52692be0339c7fe04e5ee025cd7c639490ae249')
    return kernel
