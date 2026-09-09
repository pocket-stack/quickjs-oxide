"""Pinned data and expected shapes for receipts."""

STAGE3F_STATUS_CORRIDORS = (('the status document must retain the exact Stage3F raw177 one-to-one Nop chain, branch index, '
  'verifier fallthrough, and VM no-effect boundary',
  'Stage 3F admits raw 177 only as the exact one-to-one typed chain',
  'adds no public surface, source syntax, Test262 admission, or Feature Parity claim.',
  '068db26e6d2a60b8336443ace78bc6bf165b37733e1bce10282abbcb6b646671'),
 ('the status document must retain the exact Stage3F 41-byte wire, metadata, realm, pending, '
  'malformed-fallthrough rollback, and branch-index evidence',
  'Stage-3F Rust evidence uses the exact 41-byte property-free strict function wire',
  'a separate branch fixture proves `Goto(1)` still lands on `Instruction::Nop`.',
  'd4af12e9a2e3b169ad8f8d2061647630d71eaa473a80862690df896f3e9cb0f9'),
 ('the status document must retain the exact Stage3F compiler-natural baseline, mechanical raw177 '
  'derivation, Stage3G Object, Stage3H ToObject, Stage3I PushThis, and Stage3J ToPropKey evidence, '
  'C execution boundary, and frozen C hashes',
  'Stage 3F authenticates the compiler-natural strict empty-function baseline',
  '`e9f74aaa094cc4fb30b4a159d239d7e311622e55cd4c78f75723057a03569ee7`.',
  '291a4ca6a4a5b9dd43f728e1a9d54223719e8313b5d3f9900095c7fb5891e5eb'))

STAGE3G_STATUS_CORRIDORS = (('the status document must retain the exact Stage3G raw11 one-to-one Object chain, publisher, '
  'verifier, VM, defining-realm, raw47, and no-new-surface boundaries',
  'Stage 3G admits raw 11 only as the exact one-to-one typed chain',
  'Stage 3G exposes no new source syntax, public API, Test262 admission, or Feature Parity claim.',
  'c2cd9c362a26adfbf9854a9d1bd924ecf6a732391c51e879c02b200b81c4fbf3'),
 ('the status document must retain the exact Stage3G 41-byte natural wire, metadata, fresh '
  'defining-realm objects, transaction rollback/retry, and branch-index evidence',
  'Stage-3G Rust evidence uses the compiler-natural exact 41-byte strict object-return wire',
  'typed raw11 index, fresh identity, defining prototype, and clean pending state.',
  '467a973c533455fd3956a967cae843ccb04e1817e01dd4468afe64a60f7ecec3'))

STAGE3H_STATUS_CORRIDORS = (('the status document must retain the exact Stage3H raw111 one-to-one ToObject chain, verifier, '
  'VM, defining-realm boxing, no-coercion, blocked-neighbor, and no-new-surface boundaries',
  'Stage 3H admits raw 111 only as the exact one-to-one typed chain',
  'Stage 3H changes neither production bytecode nor VM implementation and adds no source syntax, '
  'public API, Test262 admission, or Feature Parity claim.',
  '40c7eb0c3221b0e01bac7000d884d57080c65ba83c248e2e685d0abc6877373e'),
 ('the status document must retain the exact Stage3H natural/manual wires, metadata/raw sets, '
  'identity/boxing/realm/nullish semantics, rollback/retry, and branch-index evidence',
  'Stage-3H Rust evidence distinguishes the compiler-natural exact 56-byte strict',
  'Boolean boxing prototype, and clean pending state.',
  '04ff9df60b6dab1e8bbab1ffb4b5e1463a71e5963ccf3392a3d27528c1c0ebe6'),
 ('the status document must retain honest compiler-natural versus mechanical C provenance, exact '
  'raw111/raw8/raw112 execution evidence, and current frozen C hashes',
  'Stage 3H then compiler-naturally emits the exact 56-byte strict source',
  '`e9f74aaa094cc4fb30b4a159d239d7e311622e55cd4c78f75723057a03569ee7`.',
  'c19562aa539c039a82394d2ee4443e4ce5479ab4870d793e9a74ab3d153faecc'))

STAGE3I_STATUS_CORRIDORS = (('the status document must retain the exact Stage3I raw8 PushThis typed chain, archive protocol, '
  'VM receiver semantics, blocked neighbors, and no-new-surface boundary',
  'Stage 3I admits raw 8 only as the exact one-to-one typed chain',
  'Stage 3I changes neither the engine Instruction set nor VM implementation and adds no source '
  'syntax, public API, Test262 admission, or Feature Parity claim.',
  '004e01121a61cb598b558a77f02861a4d5dd3a77b0be8349d5a803323ac4d8e2'),
 ('the status document must retain the exact Stage3I natural/manual wires, typed output, '
  'receiver/realm/boxing semantics, protocol negatives, rollback/retry, and raw8-absent '
  'compatibility evidence',
  'Stage-3I Rust evidence pins compiler-natural strict and sloppy 47-byte',
  'protocol does not narrow older ordinary bodies.',
  'b2dc822ee1a0cdb05fb9dd87d06773d5d5a6f5326468a0d64c370fdab8822dc5'),
 ('the status document must retain honest Stage3I compiler-natural versus mechanical C provenance '
  'and adversarial raw8 execution evidence',
  'Stage 3I additionally compiler-naturally emits strict and sloppy',
  'exact-once and no-target-zero predicates.',
  '5160023e5cd0d27b649b139ec314fb9d4f8310dfa32ed69721b74e834cff7a18'),
 ('the status document must retain the exact Stage3I source-current receipt, inherited '
  'Stage3H/Stage3G/Stage3F coverage, and no-new-conformance lifecycle boundary',
  'This promoted receipt is source-current for Stage 3I',
  'raw-177 coverage, and makes no new conformance claim.',
  '83659ec1f3f12a5db78ca0595f43b2da887bef0d0200cdab16c7523998e67e6a'))

STAGE3J_STATUS_CORRIDORS = (('the status document must retain the exact Stage3J scalar/ordinary/union cohorts, admitted '
  'milestones, blocked raw47 frontier, and blocker vector',
  'The scalar policy remains 30 opcodes; the stage-3J ordinary policy is 133,',
  '`1, 4, 2, 1, 3, 7, 16, 15, 25, 4, 9, 11, 5, 4, 3`.',
  '6a7e96b15406438584c36926017a41e99b3d0b794d860d872dfc190701b6c033'),
 ('the status document must retain the exact Stage3J raw112 ToPropKey typed chain, existing '
  'verifier/VM semantics, duplicate/backedge acceptance, raw47 boundary, and no-new-surface claim',
  'Stage 3J admits raw 112 only as the exact one-to-one typed chain',
  'metric, or Feature Parity claim.',
  'aa442f78f09ea16861e3063d970a1c438277f5e101614e4783bb987df1b39919'),
 ('the status document must retain honest Stage3J compiler-natural versus mechanical C provenance, '
  'all pinned raw112 wires and semantics, and current frozen C hashes',
  'Stage 3J compiler-naturally emits',
  '`e9f74aaa094cc4fb30b4a159d239d7e311622e55cd4c78f75723057a03569ee7`.',
  '9d8622f4c3cd6de4a7811d0e491a72826edabb965935f49688d52ba461357bf5'),
 ('the status document must retain the exact Stage3J source-ahead, not-covered, not-authenticated, '
  'no-receipt, and no-new-conformance lifecycle boundary',
  'The Stage 3J raw112 `ToPropKey` Rust6/C3 working tree described above is',
  'and makes no new conformance or Feature Parity claim.',
  '7a37ebfb6e4e97c9d426c188a6f7349fcde8c349515f50ba2828e393f37c87c5'))

STAGE3F_DEFAULT_IGNORABLE_RANGES = ((847, 847),
 (4447, 4448),
 (6068, 6069),
 (6155, 6159),
 (8293, 8293),
 (12644, 12644),
 (65024, 65039),
 (65440, 65440),
 (65520, 65528),
 (917504, 921599))

STAGE3J_CONTRARY_CLAIMS = ('\\bStage[- ]*3J\\b[^.;!?]{0,100}\\b(?:is|remains)[ \\t]+source[- ]current\\b',
 '\\bStage[- ]*3J\\b[^.;!?]{0,100}\\b(?:is|has[ \\t]+been|remains)[ '
 '\\t]+(?:authenticated|certified|covered)\\b',
 '\\b(?:this[ \\t]+)?(?:promoted[ '
 '\\t]+)?receipt\\b[^.;!?]{0,100}\\b(?:authenticates|certifies|covers)\\b[^.;!?]{0,80}\\b(?:Stage[- '
 ']*3J|raw[- ]?112)\\b',
 '\\b(?:run|job|artifact|fingerprint|reports?)\\b[^.;!?]{0,100}\\b(?:authenticates|certifies|covers)\\b[^.;!?]{0,80}\\b(?:Stage[- '
 ']*3J|raw[- ]?112)\\b',
 '\\bStage[- ]*3J\\b[^.;!?]{0,100}\\b(?:not|never|no[ \\t]+longer)[^.;!?]{0,32}\\bsource[- '
 ']ahead\\b',
 '\\b(?:not|never|no[ \\t]+longer)[^.;!?]{0,32}\\bsource[- ]ahead\\b[^.;!?]{0,100}\\bStage[- '
 ']*3J\\b',
 '\\bStage[- ]*3J\\b[^.;!?]{0,120}\\b(?:adds|makes|establishes)[ \\t]+(?:a[ \\t]+)?new[ '
 '\\t]+(?:Test262[ \\t]+)?conformance\\b')

STAGE3J_REQUIRED_CLAIMS = ('\\bStage[- ]*3J\\b[^.;!?]{0,100}\\bis[ \\t]+source[- ]ahead\\b[^.;!?]{0,100}\\bStage[- ]*3I[ '
 '\\t]+receipt\\b',
 '\\bStage[- ]*3I[ \\t]+receipt\\b[^.;!?]{0,80}\\bdoes[ \\t]+not[ \\t]+cover[ \\t]+or[ '
 '\\t]+authenticate[ \\t]+Stage[- ]*3J\\b',
 '\\bno[ \\t]+Stage[- ]*3J[ \\t]+receipt[ \\t]+has[ \\t]+been[ \\t]+promoted\\b',
 '\\bStage[- ]*3J\\b[^.;!?]{0,100}\\badds[ \\t]+no[ \\t]+Test262[ '
 '\\t]+admission\\b[^.;!?]{0,100}\\bchanges[ \\t]+no[ \\t]+focused[ \\t]+or[ \\t]+full[ '
 '\\t]+metric\\b',
 '\\bmakes[ \\t]+no[ \\t]+new[ \\t]+conformance[ \\t]+or[ \\t]+Feature[ \\t]+Parity[ \\t]+claim\\b')

STAGE3E_RECEIPT_METRICS = {'milestone': 'r3fj',
 'focused_variants': '6844',
 'focused_eligible': '6844',
 'focused_runnable': '6844',
 'focused_passes': '6844',
 'focused_tsv_lines': '6857',
 'focused_jsonl_lines': '6846',
 'focused_summary': 'pass=6844',
 'full_variants': '102037',
 'full_eligible': '80032',
 'full_runnable': '80032',
 'full_passes': '79982',
 'full_tsv_lines': '102050',
 'full_jsonl_lines': '102039',
 'full_summary': 'fail-parse=7 fail-runtime=43 pass=79982 skipped-config-exclude=6700 '
                 'skipped-feature=11775 unsupported-feature=847 unsupported-module=121 '
                 'unsupported-negative-provenance=2562'}

STAGE3E_RECEIPT_HEX_FIELDS = {'engine_semantics_source': 40,
 'engine_semantics_sha256': 64,
 'focused_tsv_sha256': 64,
 'focused_jsonl_sha256': 64,
 'full_tsv_sha256': 64,
 'full_jsonl_sha256': 64}

STAGE3E_PROMOTED_RECEIPT_VALUES = {'engine_semantics_source': '022e7b4860ec9b6e6d2922f835ac694790880126',
 'engine_semantics_sha256': 'f61afc7314c09e4b507468ca9bffdeb920d3e9c896068b3a9f39b4587caa0333',
 'focused_tsv_sha256': 'fec3395e614b678fefcde53e880605126dfed3aab58e2c4e69cc046d70252b98',
 'focused_jsonl_sha256': 'f8aa7b03998c11d8cc29ae84bb9a8dcad512ac6d368af8819ed6bf9f7b9144d2',
 'full_tsv_sha256': '52a4288752e11855a5101c7a536717690bdcb2d582224defc58bbe1ebfd3da91',
 'full_jsonl_sha256': '6886651883250c460dde168d85d1d44524f3b9f4dc5398859a0c2071604e6a3d'}

STAGE3E_FOCUSED_RECEIPT_PATHS = {'focused_tsv': ('dev-support/test262/generated/test262-class-private-callables-b-global-candidate.tsv',
                 'focused_tsv_sha256',
                 1543969),
 'focused_jsonl': ('dev-support/test262/generated/test262-class-private-callables-b-global-candidate.jsonl',
                   'focused_jsonl_sha256',
                   3501190)}

STAGE3E_CONTRARY_CLAIMS = ('\\b(?:source[- ]stale|stale)[ \\t]+for[ \\t]+Stage[ \\t]*3I\\b',
 '\\b(?:does[ \\t]+not|cannot)[ \\t]+(?:certify|cover|authenticate)[ \\t]+(?:the[ \\t]+)?(?:raw[- '
 ']?8|Stage[ \\t]*3I)\\b',
 '\\bStage[ \\t]*3I\\b[^.]{0,100}\\b(?:is|remains)[ \\t]+(?:source[- ]ahead|source[- '
 ']stale|stale|uncertified|unauthenticated|not[ \\t]+authenticated|not[ \\t]+certified)\\b',
 '\\bStage[ \\t]*3I\\b[ \\t]+(?:is|remains)[ \\t]+(?:uncovered|not[ \\t]+covered)\\b',
 '\\bStage[ \\t]*3I\\b[^.]{0,100}\\bhas[ \\t]+(?:not[ \\t]+been|yet[ \\t]+to[ \\t]+be)[ '
 '\\t]+(?:authenticated|certified|covered)\\b',
 '\\bStage[ \\t]*3I\\b[^.]{0,100}\\b(?:pending|awaiting)[ \\t]+(?:a[ \\t]+)?(?:separate[ '
 '\\t]+)?(?:exact-source[ \\t]+)?receipt[ \\t]+promotion\\b',
 '\\bonly[ \\t]+Stage[ \\t]*3H\\b[^.]{0,120}\\b(?:is|was|has[ \\t]+been)[ '
 '\\t]+(?:authenticated|certified|covered)\\b[^.]{0,120}\\b(?:this[ \\t]+)?receipt\\b',
 '\\b(?:this[ \\t]+)?receipt\\b[^.]{0,100}\\b(?:authenticates|certifies|covers)[ \\t]+only[ '
 '\\t]+Stage[ \\t]*3H\\b',
 '\\b(?:this[ \\t]+)?receipt\\b[ \\t]+only[ \\t]+(?:authenticates|certifies|covers)[ \\t]+Stage[ '
 '\\t]*3H\\b',
 '\\b(?:source[- ]stale|stale)[ \\t]+for[ \\t]+Stage[ \\t]*3H\\b',
 '\\b(?:does[ \\t]+not|cannot)[ \\t]+(?:certify|cover|authenticate)[ \\t]+(?:the[ \\t]+)?(?:raw[- '
 ']?111|Stage[ \\t]*3H)\\b',
 '\\bStage[ \\t]*3H\\b[^.]{0,100}\\b(?:is|remains)[ \\t]+(?:source[- ]ahead|source[- '
 ']stale|stale|uncertified|unauthenticated|not[ \\t]+authenticated|not[ \\t]+certified)\\b',
 '\\bStage[ \\t]*3H\\b[ \\t]+(?:is|remains)[ \\t]+(?:uncovered|not[ \\t]+covered)\\b',
 '\\bStage[ \\t]*3H\\b[^.]{0,100}\\bhas[ \\t]+(?:not[ \\t]+been|yet[ \\t]+to[ \\t]+be)[ '
 '\\t]+(?:authenticated|certified|covered)\\b',
 '\\bStage[ \\t]*3H\\b[^.]{0,100}\\b(?:pending|awaiting)[ \\t]+(?:a[ \\t]+)?(?:separate[ '
 '\\t]+)?(?:exact-source[ \\t]+)?receipt[ \\t]+promotion\\b',
 '\\bonly[ \\t]+Stage[ \\t]*3G\\b[^.]{0,120}\\b(?:is|was|has[ \\t]+been)[ '
 '\\t]+(?:authenticated|certified|covered)\\b[^.]{0,120}\\b(?:this[ \\t]+)?receipt\\b',
 '\\b(?:this[ \\t]+)?receipt\\b[^.]{0,100}\\b(?:authenticates|certifies|covers)[ \\t]+only[ '
 '\\t]+Stage[ \\t]*3G\\b',
 '\\b(?:this[ \\t]+)?receipt\\b[ \\t]+only[ \\t]+(?:authenticates|certifies|covers)[ \\t]+Stage[ '
 '\\t]*3G\\b',
 '\\b(?:source[- ]stale|stale)[ \\t]+for[ \\t]+Stage[ \\t]*3G\\b',
 '\\b(?:does[ \\t]+not|cannot)[ \\t]+(?:certify|cover|authenticate)[ \\t]+(?:the[ \\t]+)?(?:raw[- '
 ']?11|Stage[ \\t]*3G)\\b',
 '\\bStage[ \\t]*3G\\b[^.]{0,100}\\b(?:is|remains)[ \\t]+(?:source[- ]ahead|source[- '
 ']stale|stale|uncertified|unauthenticated|not[ \\t]+authenticated|not[ \\t]+certified)\\b',
 '\\bStage[ \\t]*3G\\b[ \\t]+(?:is|remains)[ \\t]+(?:uncovered|not[ \\t]+covered)\\b',
 '\\bStage[ \\t]*3G\\b[^.]{0,100}\\bhas[ \\t]+(?:not[ \\t]+been|yet[ \\t]+to[ \\t]+be)[ '
 '\\t]+(?:authenticated|certified|covered)\\b',
 '\\bStage[ \\t]*3G\\b[^.]{0,100}\\b(?:pending|awaiting)[ \\t]+(?:a[ \\t]+)?(?:separate[ '
 '\\t]+)?(?:exact-source[ \\t]+)?receipt[ \\t]+promotion\\b',
 '\\bonly[ \\t]+Stage[ \\t]*3F\\b[^.]{0,120}\\b(?:is|was|has[ \\t]+been)[ '
 '\\t]+(?:authenticated|certified|covered)\\b[^.]{0,120}\\b(?:this[ \\t]+)?receipt\\b',
 '\\b(?:this[ \\t]+)?receipt\\b[^.]{0,100}\\b(?:authenticates|certifies|covers)[ \\t]+only[ '
 '\\t]+Stage[ \\t]*3F\\b',
 '\\b(?:this[ \\t]+)?receipt\\b[ \\t]+only[ \\t]+(?:authenticates|certifies|covers)[ \\t]+Stage[ '
 '\\t]*3F\\b',
 '\\b(?:source[- ]stale|stale)[ \\t]+for[ \\t]+Stage[ \\t]*3F\\b',
 '\\b(?:does[ \\t]+not|cannot)[ \\t]+(?:certify|cover|authenticate)[ \\t]+(?:the[ \\t]+)?(?:raw[- '
 ']?177|Stage[ \\t]*3F)\\b',
 '\\bStage[ \\t]*3F\\b[^.]{0,100}\\b(?:is|remains)[ \\t]+(?:source[- '
 ']stale|stale|uncertified|unauthenticated|not[ \\t]+authenticated|not[ '
 '\\t]+certified|uncovered|not[ \\t]+covered)\\b',
 '\\b(?:source[- ]stale|stale)[ \\t]+for[ \\t]+Stage[ \\t]*3E\\b',
 '\\b(?:does[ \\t]+not|cannot)[ \\t]+(?:certify|cover|authenticate)[ \\t]+(?:the[ \\t]+)?(?:raw[- '
 ']?49|Stage[ \\t]*3E)\\b',
 '\\bStage[ \\t]*3E\\b[^.]{0,100}\\b(?:is|remains)[ \\t]+(?:source[- '
 ']stale|stale|uncertified|unauthenticated|not[ \\t]+authenticated|not[ '
 '\\t]+certified|uncovered|not[ \\t]+covered)\\b')
