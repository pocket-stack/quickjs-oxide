# Test262 progress baseline

Test262 is now a pinned progress instrument, not yet a completion claim. The
authoritative compatibility target remains QuickJS 2026-06-04; focused QuickJS
differentials still decide exact behavior inside each implemented slice.

## Pinned inputs

- Test262 commit: `5c8206929d81b2d3d727ca6aac56c18358c8d790`
- QuickJS patch SHA-256: `f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3`
- QuickJS config SHA-256: `79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b`
- quickjs-oxide capability profile SHA-256:
  `1860224ce1e828406f4869b66b3f1964f96fad85e4eab6ba7fecb256b4b6c2f2`
- 53,125 non-fixture metadata records SHA-256:
  `a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a`

`scripts/prepare-test262.sh` prepares and verifies that exact checkout and the
two harness changes carried by the QuickJS release. No Test262 source is
vendored into the product.

## Smoke baseline

`tests/test262-smoke.txt` fixes 100 synchronous script tests. They expand to
193 independent sloppy/strict variants:

- all 193 pass;
- 0 unsupported, failed, skipped, timed-out, crashed, or infrastructure-faulted
  variants.

The final two former frontiers are parse-negative tests which put class
declarations in a single-statement `if` position. They now pass for the
intended early `SyntaxError`, rather than because class parsing is absent.

This 193/193 result is a runner smoke baseline, not a project-wide 100%
estimate. The sample was selected from already implemented synchronous
surfaces. Module, async/jobs, most `$262` host hooks, advanced RegExp pattern
grammar, async class forms, TypedArrays, and many other broad layers remain
absent. Public fields, static blocks, private elements, and public/private
synchronous generator methods are measured separately by the scoped
R3g/R3h/R3i/R3j/R3k/R3l gates below.

Nineteen additional provenance variants guard the result: four audited negative
variants pass for the intended parse error, while 15 unsupported grammar
variants fail closed instead of passing because they happened to throw a
`SyntaxError`.

## Complete classified vector

The pinned suite expands to 102,037 sloppy/strict variants. The runner emits
every outcome in canonical order, and the checked-in baseline pins the complete
vector hashes and summary:

- 36,928 pass;
- 18,475 are outside the pinned QuickJS target configuration;
- 45,150 are classified as unsupported because of a feature, mode, host
  capability, parser/runtime/harness frontier, or unaudited negative-test
  provenance;
- 97 fail to parse, 1,284 fail at runtime, 97 fail in the harness, and six
  time out; there are no crashes or runner/engine infrastructure faults.

The runner admitted 38,483 variants to execution. That count includes variants
which then report a typed parser or harness frontier rather than an observed
non-unsupported outcome.

Three rates answer different questions:

- raw suite pass rate: 36.19% (`36,928 / 102,037`);
- conservative target-scope lower bound: 44.19%
  (`36,928 / (102,037 - 18,475)`);
- pass rate among variants with a non-unsupported observed outcome: 96.14%
  (`36,928 / 38,412`).

The 44.19% figure is the useful whole-project progress floor, not a claim that
the engine is 44.19% conformant. The 96.14% conditional rate measures quality
only on the currently exposed frontier and must not be read as overall
completion. It can move in either direction as classification improves: R2p
lowers it slightly by admitting 204 real, independent non-Symbol frontiers that
had previously failed closed as unsupported features; R2q then raises it
slightly as 31 untagged binding variants become real passes, R2t resolves two
more typed parser frontiers, R2u adds 15 array-assignment passes without
admitting additional jobs, R2v resolves 14 untagged object-assignment
frontiers, and R2w resolves 23 parser frontiers, 24 runtime frontiers, and two
ordinary runtime failures on the synchronous catch-binding surface. R2x then
adds 88 passes from the synchronous identifier-rest surface and its untagged
harness consumers without admitting additional jobs. R2y adds another 60
passes from synchronous identifier defaults and moves direct-eval,
destructuring, class, and missing-intrinsic consumers to their deeper explicit
frontiers, again without changing the runnable count. R2z then adds 22 passes
from synchronous no-default parameter BindingPatterns, while moving 11 old
failures to the deeper Parameter-Environment frontier and keeping the runnable
count fixed. R3a adds 12 passes from the combined parameter-expression and
BindingPattern path, moves two typed runtime frontiers to their already-known
adjacent failures, and again keeps the runnable count fixed. R3b adds 66 passes
from direct eval in non-simple Parameter Environments; one untagged staging
variant reaches its known implicit-`this` runtime mismatch and two reach the
generator-method typed runtime frontier, while the runnable count remains
fixed. R3e then adds 328 passes from the dependency-audited synchronous base
class slice, exposes adjacent derived/class-element and missing-intrinsic
frontiers, and again keeps the runnable count fixed. R3f adds 545 passes by
opening synchronous heritage/derived construction, while 88 adjacent variants
move from parser/harness frontiers to honest missing-intrinsic, optional-chain,
or pinned-target-error outcomes. The capability profile currently admits 71
reviewed Test262 feature tags and 423 reviewed
negative-test paths; all other feature-tagged or
negative-provenance cases fail closed. Expanding that profile as implementation
lands can only make the measurement more representative. Focused QuickJS
differential tests remain the semantic judge.

R3s separately admits `RegExp.escape` only in its checksum-bound complete
RegExp built-ins profile; the global profile remains fail-closed for that tag.
R3t likewise authenticates synchronous `generators` plus
`destructuring-binding` in a checksum-bound scoped profile while leaving both
tags fail-closed globally.

The complete TSV/JSONL reports are generated under `target/` rather than
committed (together they are tens of megabytes). Their complete hashes and
outcome summary are pinned in `tests/test262-full-baseline.txt`. Runner ordering
was cross-checked at five and eight workers through the scoped RegExp modifier
milestone. The canonical full gate now uses two workers because its 30-second
budget is wall-clock based and the untagged `Proxy/ownkeys-linear.js` performs
15,000 precondition property insertions before reaching the missing `Proxy`
frontier; higher worker contention can make those two already-failing variants
scheduler-dependent. Focused gates and the generic runner retain their existing
parallel defaults. The current byte expectations use a fixed
`TZ=America/Los_Angeles`; the hash gate therefore requires a Unix-like zoneinfo
installation, and Windows still lacks the corresponding IANA-zone backend.
The current TSV and JSONL SHA-256 values are
`6b2fb9219bad5f25bfcebc297ce9373798cd210140ebab0566a18e8dd83d052b`
and
`d2cf352f98f7d12b1ff734d7ff001c443c896be3c8adddd54951dd0a47f78eb2`.

## Milestone policy

Test262 is now the project-wide milestone scoreboard, while the pinned QuickJS
source and focused differential probes remain the semantic specification for
each feature slice. A substantial slice lands only after its Rust/unit and
QuickJS differential gates pass; the full Test262 vector then records pass
movement, regressions, newly exposed failures, and unsupported-frontier
movement. Small implementation commits do not need an independent full-suite
run.

The preceding simple-parameter `arguments` milestone moved 17,365 to 18,011
passes and exposed `Math.pow` as a common harness blocker. The Math milestone
moved the complete vector from 18,011 to 21,429 passes with no previous-pass
regression. Its exact old/new join matched all 102,037 keys: all 4,435 outcome
changes are inside the 4,589-variant reviewed set, with zero outcome drift
among the other 97,448 variants.

The reviewed set now has 3,420 passes and 1,169 non-pass outcomes. Every one of
the 568 runnable `built-ins/Math` variants passes; 86 more remain explicitly
unsupported because they also require other unimplemented feature tags. The
3,770 `propertyHelper.js` variants now split into 2,755 passes, 897 runtime
failures, four harness errors, 52 parse failures, and 62 explicit parser
frontiers.

The keyed transition audit records 2,763 `harness-error -> pass`, 897
`harness-error -> fail-runtime`, 639 `fail-runtime -> pass`, 62
`harness-error -> unsupported-parser`, 56 `harness-error -> fail-parse`, 16
`unsupported-feature -> pass`, and two `fail-runtime -> timeout`. Those two
timeouts are the sloppy and strict variants of
`staging/sm/String/fromCodePoint.js`: implementing `Math.pow` lets them reach
their 49,152-argument `apply` stress path, so they record a performance
frontier rather than a Math semantic regression.

The Reflect milestone moves the complete vector from 21,429 to 21,740 passes
and from 31,873 to 32,227 runnable variants. An exact keyed join again matched
all 102,037 variants: precisely 371 outcomes changed, every one inside the
427-variant reviewed Reflect manifest, with no previous-pass regression and no
outside-manifest drift. The transitions are 294 `unsupported-feature -> pass`,
38 `unsupported-feature -> fail-runtime`, ten `unsupported-feature ->
fail-parse`, six `unsupported-feature -> harness-error`, six
`unsupported-feature -> unsupported-parser`, and 17 `fail-runtime -> pass`.

The current 427-variant focused Reflect vector admits 405 variants: 377 pass,
28 fail at runtime, and 22 remain gated by adjacent features. R2f moved four
concise-method parser frontiers to runtime assertions; R2g then made four
independent getter consumers pass. Later aggregate refreshes exposed the
already-landed downstream fixes, including both variants of the Reflect.apply
rest-parameter case. The other non-pass results continue to expose
ArrayBuffer, async/generator, JSON, TypedArray, parser, or adjacent-feature
frontiers rather than being hidden from the scoreboard. Current focused
TSV/JSONL SHA-256 values are
`186ee55e1981e8ca036c7e828dd74749d0ec0c6bf4a93a11a352f8c5500abc21`
and
`e382d2f60ce5146949ad3850330672bc03a37e13cd344cf66e32eed38c3d5f48`.

The observable Date milestone moves the complete vector from 21,740 to 23,016
passes without changing its 32,227 admitted jobs. An exact keyed join across
all 102,037 variants records exactly 1,276 `fail-runtime -> pass` transitions,
no previous-pass regression, and no outcome change outside the reviewed Date
manifest. Five- and eight-worker full reports are byte-identical.

The Date-focused review corpus contains 799 paths and 1,598 sloppy/strict
variants. Its Date-owned subset contains all 646 paths and 1,292 variants from
`built-ins/Date`, `annexB/built-ins/Date`, and `staging/sm/Date`; 153 adjacent
paths expose Date through globals, reflection, constructors, or indirect
dependencies. The current focused outcome vector has 1,552 passes, two runtime
failures, 34 configured/feature skips, and ten explicit `create-realm` host
frontiers. The runner admits 1,554 jobs, all of which now have an observed
non-unsupported outcome, for a 99.87% pass rate on that frontier (97.12% of the
complete focused vector). R2f resolved 62 former concise-method parser
frontiers; R2g then resolved the final ten accessor variants. Later milestones
lifted additional downstream blockers without changing the Date-owned surface.
The remaining runtime and host frontiers stay explicit.
Current focused TSV/JSONL SHA-256 values are
`751cdacad364af8f324df0ddaa5aa446a28963e0933a836f90c23b5a0600364e`
and
`e9d2e46bc6cccde539ae8b5950837f469862f3e1462cb966b346ca37034745ea`.
The six grouped QuickJS differentials, one oracle vector self-check, two
cross-realm/GC integration tests, and 44 Date unit tests pass. Reproduce the
hash-pinned focused vector with `scripts/test-test262-date.sh`; both it and the
full-vector command fix `TZ=America/Los_Angeles`. The Date-landing focused and
full reports were byte-identical at five and eight workers on the required
Unix-like zoneinfo host.

The generic `String.prototype.split` milestone moves the complete vector from
23,016 to 23,190 passes and from 32,227 to 32,289 admitted jobs. Its exact keyed
join matches all 102,037 variants and records 220 changes with no previous-pass
regression: 158 `fail-runtime -> pass`, 16 `unsupported-feature -> pass`, 30
`unsupported-feature -> fail-parse`, 12 `unsupported-feature -> fail-runtime`,
and four `unsupported-feature -> unsupported-parser`. Of those changes, 172
are inside the focused manifest and become passes; outside it, the existing
`Symbol.split` descriptor test contributes two more passing variants and 46
`RegExp.prototype[Symbol.split]` variants move from feature-gated to explicit
RegExp parser/runtime/parser-frontier outcomes.

The focused split corpus contains 127 paths and 254 sloppy/strict variants:
all 120 `built-ins/String/prototype/split` paths, one Annex-B IsHTMLDDA path,
and six direct consumers selected from the previous full vector. At the
generic-split landing it had 186 passes, 52 runtime failures, eight
feature-gated outcomes, six typed parser frontiers, and two host-capability
outcomes. Declaring `Symbol.split` meant that the well-known symbol and
generic/custom-splitter delegation were audited, not that the then-unpublished
RegExp protocol was complete; exposing those outcomes made the next semantic
frontier visible.

R1e activates that existing delegation through
`RegExp.prototype[Symbol.split]`. At that landing, the same frozen vector
admitted 244 variants and recorded 234 passes, four runtime failures at the
independent missing-global-`eval` frontier, eight adjacent feature outcomes,
two IsHTMLDDA host outcomes, and six typed parser frontiers. Its TSV and JSONL
SHA-256 values are
`ad66315d9b6d285240d9f0628a899ab71b64496ea451f153bcf4916d7ffeccdb`
and
`c0182c6f56c9df1cb4b1e991f60aa94aa5c8173e01f7882e7fa4031e966eaebc`.
The capability profile remains unchanged at 18 reviewed tags.
R1p's Annex B named-backreference parser resolves the two
`separator-regexp.js` variants. R1x then executes the two eval consumers. The
R2c Arrow and R2f concise-method slices resolve the remaining parser consumers.
The current gate admits and passes 252 variants; two require IsHTMLDDA.
Current TSV/JSONL hashes are
`13f8c26ce2c9cd93904ce420cc00010e06e60f1eedccd7e22cc2f1e98fdb1303`
and
`eb88da8a2773b80e436c9311ba39f0868c623555e6679aeff4761ef631e5f26d`.

The RegExp R0 foundation deliberately did not increase the pass count. It
added the internal UTF-16 compiler/executor and heap brand while `%RegExp%`
remained unavailable. A static RegExp-core manifest froze 225 untagged
`built-ins/RegExp` paths and 450 sloppy/strict variants as a zero-pass named
implementation queue rather than a feature claim.

The parser now selects the RegExp lexical goal when `/` or `/=` begins a
primary expression. An exact join across all 102,037 full-vector keys records
1,312 classification-only changes and no pass regression: 1,209
`fail-parse -> unsupported-parser` transitions plus 103
`harness-error -> unsupported-harness-parser` transitions. Every old result
was `SyntaxError: unexpected '/'`; every new result is the typed RegExp-literal
frontier, including 73 harness users of `nativeFunctionMatcher.js` and 30 of
`sm/non262-Math-shell.js`. No flags, feature metadata, expected phase or
actual phase changed. The same reclassification moves 2 Reflect, 8 Date and 38
String-split focused variants without changing their 321, 1,282 and 174 pass
counts. Five- and eight-worker R0 full reports were byte-identical at 23,190
passes and 32,289 admitted jobs.

The RegExp R1a observable shell publishes the constructor, ordinary prototype,
species, source/flag accessors, `exec`, abstract RegExpExec/`test`, `toString`,
`lastIndex`, captures and `d` indices while continuing to reject advanced
grammar explicitly. At the R1a landing, the 450-variant core vector recorded
430 passes, ten `fail-runtime` outcomes caused only by the separate
missing-`eval` frontier, and ten `unsupported-runtime` outcomes. The later R1f
Unicode decimal-escape classification refinement moves both variants of
`unicode_restricted_octal_escape.js` to pass, so the core vector at R1f had
432 passes and eight typed advanced-pattern outcomes. The R1a full vector moves
from 23,190 to 23,859 passes, reduces `fail-runtime` from 4,540 to 3,861, and
adds ten typed `unsupported-runtime` outcomes. RegExp literals, legacy
`compile`, `RegExp.escape`, and Symbol protocols were not claimed by that
slice.
An exact join matches all 102,037 `(path, variant)` keys with no duplicates or
missing rows and zero previous-pass regressions. Its only 679 transitions are
669 `fail-runtime -> pass` and ten
`fail-runtime -> unsupported-runtime`. The new passes span 462 RegExp, 132
Object, 42 Array, 12 String, nine language-expression and 12 adjacent global,
literal or staging variants; those collateral groups construct or consume
regular expressions rather than representing unrelated feature work.
Subsequent RegExp grammar slices moved the same core gate to 436 passes; R1p's
Unicode bare-`\k` diagnostic resolves two more, and R1x executes the five eval
consumers. R3s resolves the final two typed legacy-control frontiers, so the
current core vector passes all 450 variants. Its TSV/JSONL hashes are
`ec6298bec9cd1f268a5e36ef725ea196d44a13a6d7ed0e3b53791edb853c1021`
and
`7702a505d3ad53624cd6f7975bb55973c89eac5b1b6edcc9fdb0d6dc1fd693e9`.
The frozen core vector is reproduced by
`scripts/test-test262-regexp-core.sh`.

The RegExp R1b literal slice follows QuickJS's compile-once/instantiate-many
model: a typed compiled-pattern constant is linked into bytecode, and
`Instruction::RegExp` creates a fresh RegExp with the execution realm's
canonical shape and prototype on every evaluation. Pattern diagnostics remain
at compile time. `tests/test262-regexp-literals.txt` freezes 48 paths and 96
sloppy/strict variants; `tests/test262-regexp-literals-baseline.txt` pins the
classified TSV/JSONL hashes plus the R1a selection provenance, and
`scripts/run-test262-regexp-literals.sh` reproduces both checks. At the R1b
landing, the focused vector had 88 passes, two `fail-runtime` outcomes and six
typed `unsupported-parser` outcomes: two lookaround and four backreference
variants. Relative to R1a, all 88 passes move from the typed RegExp-literal
parser frontier. The two runtime variants still stop at an earlier
`String.prototype.match` call; R1d later makes both pass, moving the vector
from 88 to 90 passes. R1k resolves four linked backreference variants and R1l
the final two lookahead variants, so the current focused gate passes all 96.
The complete R1b vector moves
from 23,859 to 24,699 passes while the 18,475 target exclusions and 32,289
admitted jobs stay unchanged. Its exact 102,037-key join has 1,193 transitions:
840 `unsupported-parser -> pass`, 226 `unsupported-parser -> fail-runtime`, 24
`unsupported-parser -> fail-parse`, and 103
`unsupported-harness-parser -> harness-error`. There are no previous-pass
regressions. The focused vector remains an independent, faster reproduction
gate rather than a substitute for that full baseline.

The RegExp R1c search slice publishes `String.prototype.search` and
`RegExp.prototype[Symbol.search]` with the QuickJS conversion, delegation,
abstract-RegExpExec, `lastIndex` SameValue reset/restore, result-index and realm
boundaries locked by eight Rust tests, including nine QuickJS differential
vectors and one cross-realm runtime test. `tests/test262-regexp-search.txt`
freezes all 66 search paths and their
132 sloppy/strict variants from the R1b report;
`tests/test262-regexp-search-baseline.txt` pins both the R1b selection
provenance and current outcome hashes, and
`scripts/run-test262-regexp-search.sh` reproduces the gate. It now admits and
passes 128 variants, while four outcomes remain gated by adjacent feature
requirements. R2g resolves the final 12 accessor consumers. At R1b the same
keys were 2 passes, 60
runtime failures and 70 feature-gated outcomes, so the focused slice contributes
110 new passes. Eight more search-enabled variants outside the frozen manifest
pass, for 118 new full-vector passes in total.

The complete R1c vector moves from 24,699 to 24,817 passes and from 32,289 to
32,353 admitted jobs. Its exact old/new join matches all 102,037 keys, has zero
previous-pass regressions, and records only 66 `fail-runtime -> pass`, 52
`unsupported-feature -> pass`, and 12 `unsupported-feature ->
unsupported-parser` transitions. The parser transitions are the explicitly
bounded object-literal grammar frontier, not search algorithm drift.

The RegExp R1d match slice publishes `String.prototype.match` and
`RegExp.prototype[Symbol.match]` with QuickJS 2026-06-04 delegation,
conversion, abstract-RegExpExec, global-loop, empty-match UTF-16 advance,
mutation and realm boundaries locked by 11 passing Rust
oracle/differential/cross-realm/recursion tests. The String entry shares the
isolated generic protocol helper with search; the 155-line RegExp algorithm
lives in `runtime/intrinsics/regexp/match_protocol.rs` rather than the runtime
facade. The shared four-active-frame native recursion guard remains an explicit
non-parity frontier: the fifth mixed match/search frame throws `InternalError`,
where pinned QuickJS continues.

`tests/test262-regexp-match.txt` freezes all 104 match paths and their 208
sloppy/strict variants from the R1c report;
`tests/test262-regexp-match-baseline.txt` pins both the R1c selection provenance
and current outcome hashes, and `scripts/run-test262-regexp-match.sh`
reproduces the gate. It now admits and passes 206 variants, while two outcomes
remain gated by `regexp-v-flag`. R1x executes the
legacy eval consumer. At R1c the
same keys were two passes, 76 runtime failures and 130 feature-gated outcomes.
The focused TSV and JSONL SHA-256 values are
`5aa6b8b6c61a48acf72417d583f3439b8fbfc5dde9020b8c8341e31759a790a6`
and
`5f3e63c0d709819e47a57e4bfbb3929a565b615d74a6a95966b3dc19c90948e2`.

The complete R1d vector moves from 24,817 to 25,029 passes and from 32,353 to
32,497 admitted jobs. Its exact old/new join matches all 102,037 keys with no
missing, extra or duplicate rows and zero previous-pass regressions. The only
230 transitions are 86 `fail-runtime -> pass`, 126 `unsupported-feature ->
pass`, 16 `unsupported-feature -> unsupported-parser`, and two
`unsupported-feature -> fail-runtime`. Those two are the sloppy/strict variants
of one Annex-B path that at R1d reaches the then-unimplemented
`RegExp.prototype[Symbol.split]`. The two literal-focused variants noted at R1b
now pass, independently moving that gate from 88 to 90 passes. The full
eight-worker TSV/JSONL hashes are
`a695d6299b44e4298b553c28c12983b6b12fc9d8522f1216e18e16a6bad28012`
and
`fb305cd709b2af1bf28de5fc82b440f836a0567ff8ed3e36af967723e3beb64b`.

The RegExp R1e split slice publishes
`RegExp.prototype[Symbol.split]` and activates the existing generic
`String.prototype.split` delegation for RegExp separators. Its dedicated
237-line algorithm and reusable SpeciesConstructor helper follow QuickJS
2026-06-04 construction, flags/sticky handling, abstract RegExpExec, UTF-16
advance, capture insertion, limit, mutation, abrupt-completion and realm
boundaries. Only four facade lines are added to `runtime.rs`. Eight Rust tests
cover 19 QuickJS differential vectors.

`tests/test262-regexp-split.txt` freezes 46 direct paths and their 92
sloppy/strict variants from the R1d report;
`tests/test262-regexp-split-baseline.txt` pins the R1d selection provenance and
current outcome hashes, and `scripts/run-test262-regexp-split.sh` reproduces the
gate. It now admits and passes 50 variants. Forty core variants
remain conservatively gated by the undeclared `Symbol.species` profile tag, two
require the create-realm host hook, and R2g resolves the four former accessor
parser frontiers. The QuickJS differential suite separately locks
SpeciesConstructor semantics
without widening the full-suite capability profile. Its current TSV and JSONL
SHA-256 values are
`377746133482618291d3948d5a2da8a30f2cd7c6a7ca9cf3fce3589f426b8be5`
and
`853e1dcd3353307b0c6e2b71f4acfa3df3014f9c1dd516caad6d3f62a3f51629`.
The independent 127-path String split gate now records 252 passes. Its current
TSV/JSONL hashes are
`13f8c26ce2c9cd93904ce420cc00010e06e60f1eedccd7e22cc2f1e98fdb1303`
and
`eb88da8a2773b80e436c9311ba39f0868c623555e6679aeff4761ef631e5f26d`.

The complete R1e vector moves from 25,029 to 25,119 passes while admitted jobs
remain at 32,497. Its exact R1d/R1e join matches all 102,037 keys with no
missing, extra or duplicate rows and zero previous-pass regressions. The only
outcome transitions are 90 `fail-runtime -> pass`; five- and eight-worker TSV
and JSONL reports are byte-identical. The full hashes are
`5673ac15896bab5b1665bf8930db517447012c3d63d69bfbb1da9b8e7f9574c1`
and
`fe98f9fdb5f4c21c25cd045d8b1824fe34e3481e26c8661376d7afe78596fa64`.
The summary now has 3,847 runtime failures, four timeouts and 2,251 typed parser
frontiers; all other outcome counts are unchanged. The two variants of
`staging/sm/RegExp/split.js` retain their `fail-runtime` classification but now
reach the independent missing-JSON-global frontier, so that detail change is
not an outcome transition. The capability profile remains at 18 tags with
SHA-256
`cc10293aa847f5a449ac2b039709dff98d264b672dddc8828b8e17d8b7e12d9a`.

The RegExp R1f slice publishes the pinned legacy
`RegExp.prototype.compile` mutation. A dedicated 35-path/70-variant vector
freezes the complete Annex-B compile directory and every pinned-suite source
which directly invokes the method. It records 44 passes: all executable core
compile variants plus the four linked RegExp split variants. The sloppy/strict
variants of one staging replace path still stop first at the independent
missing `@@replace` protocol; feature, host, arrow and object-method parser
frontiers remain explicitly classified.
The focused TSV/JSONL SHA-256 values are
`1f1fb2ff6dfe5cd5dde0445e60daa310fa5b8056dfeeddac83bf3a81f0d74874`
and
`60fbf6017a8302242f5d8fa9de929e7fe39d59d7a7993631d69cc05030c56f43`.

R1f also refines Unicode decimal-escape classification at the pure RegExp
compiler boundary. The two variants of
`unicode_restricted_octal_escape.js` move from typed Unsupported to pass, so
the 450-variant RegExp-core gate now records 432 passes, ten missing-`eval`
runtime failures and eight advanced-pattern frontiers. Its TSV/JSONL SHA-256
values are
`a650f0855a4585f81c3b4c3d8df2da8ab2b9f4771ad1f94f90be0390db5c6b2b`
and
`123eae124abc4ff59475df4a028f1aafef5bb16b10c12e88d0b2a5bb2ce10e90`.

The exact R1e/R1f full join covers all 102,037 keys with no missing, extra, or
duplicate rows and zero previous-pass regressions. Its only transitions are 44
`fail-runtime -> pass` and two `unsupported-runtime -> pass`. The full vector
therefore moves to 25,165 passes while admitted jobs remain 32,497;
`fail-runtime` falls to 3,803 and `unsupported-runtime` to eight. Five- and
eight-worker reports are byte-identical. The full TSV/JSONL hashes are
`57caefa97b579fafeb6b56ba45da7daf9cbe5e168849e4ab0459b87452d4745e`
and
`613a396d850698fff9472991e547946eac6bc9bc4f3b95cf90ce57d85953dee0`.

The RegExp R1g slice ports the pinned scoped modifier grammar
`(?ims-ims:...)`. The focused manifest freezes every Test262 path whose sole
feature tag is `regexp-modifiers`: 230 paths and 460 sloppy/strict variants.
All 460 are admitted, 448 pass, and the remaining 12 stop at existing typed
frontiers: four backreference variants and eight Unicode property-escape
variants. There are no modifier-owned parse or runtime failures. The focused
TSV/JSONL SHA-256 values are
`b9baafd9e3d49b1cda6a6a5b99bbddc5ae938aa494c35bd31e1a1ceccb545c68`
and
`cf2e6a818da59c66735d46f429b885c916454cf4a2b160f6b2d10dd2b40b8e86`.

Publishing this feature also audits exactly 83 modifier-owned literal
parse-negative paths. The capability profile therefore moves to 19 feature
tags and 101 exact negative paths, with SHA-256
`0d26aedd5b5d7fa00b6c2551a93c7d776f22e2934b790615d6dc58c454156d5f`.
Because that hash is part of every report header, all earlier focused report
hashes change mechanically. Their manifests have zero overlap with the new
feature, and replacing only the R1g profile hash with the R1f value reconstructs
every previous TSV/JSONL hash exactly; their outcome rows, summaries, and
historical provenance are unchanged.

The exact R1f/R1g full join covers all 102,037 keys with no missing, extra, or
duplicate rows, no change outside the `regexp-modifiers` feature, and zero
previous-pass regressions. Its only transitions are 448
`unsupported-feature -> pass` and 12
`unsupported-feature -> unsupported-parser`. The complete vector moves to
25,613 passes and 32,957 admitted jobs. Five- and eight-worker reports are
byte-identical. The full TSV/JSONL hashes are
`5ece50a681fcb4fe97779002b179174930d2cdbdb4bd2120e0679678bd96b161`
and
`83539d1bcea789f87853cdc6d9862dd2741d61a5b6696e8513e551318c9e5df8`.

The R1h replacement slice publishes `String.prototype.replace`,
`String.prototype.replaceAll`, and the generic
`RegExp.prototype[Symbol.replace]` path. Its frozen manifest contains 191 paths
and 376 variants. At the R1h landing the profile admitted 332 variants and
recorded 286 passes with zero runtime failures. Six variants failed to parse,
40 stopped at typed parser frontiers, 38 at other undeclared features, and six
at host capabilities. The R1h focused TSV/JSONL hashes are
`055d52219998a0863a4241b3c5b374b917c1503d93b0715048ee2e171db3d012`
and
`dffcdbd8260a3d6e1c277d76797ba7187e40a971860ff802efaf8b3c6e65c0ad`.
R1i's direct standard-RegExp route preserved that outcome vector. At R1p the
gate admitted 348 variants and recorded 300 passes. The current vector admits
and passes 362 variants; eight retain adjacent feature requirements, two
require create-realm, and four require IsHTMLDDA. Current focused TSV/JSONL
hashes are
`0dccee6d3228b5c665a9f2c42890e46345d865bb0905020224e04e1b35589a94`
and
`facaadcafe19ae3444b8aa0ae2b7467519037f9c4ee4dc0bfa6f1bd07e8c98a2`.

Publishing `String.prototype.replaceAll` and `Symbol.replace` moves the
capability profile to 21 reviewed feature tags, with SHA-256
`921df0ef452f4d1286162093ebdf81a74d0805eb7c04601c86abd6ec7347ed7f`.
The Test262 worker also installs the pinned qjs-compatible `print` host surface
before raw or harness scripts, while raw tests still receive no Test262
harness.

The exact R1g/R1h full join covers all 102,037 keys with no missing, extra, or
duplicate rows and zero previous-pass regressions. Its transitions are 110
`fail-runtime -> pass`, 170 `unsupported-feature -> pass`, four
`unsupported-feature -> fail-parse`, and 38
`unsupported-feature -> unsupported-parser`. The complete vector moves to
25,893 passes and 33,169 admitted jobs. The full TSV/JSONL hashes are
`2895a8d2ddbe5857e83b573827e46b4a60a97d89b5882727c85ff75d2ff9d368`
and
`64fed7fd3bb722d470bbd420e42995e138aed5d6f3588b7d2657973cb3968419`.
Earlier focused vectors retain their outcome rows and update only their profile
metadata hashes, except the compile vector, whose two linked staging replace
variants now pass and move that focused result from 44 to 46 passes.

R1i implements the branded standard-RegExp direct replacement matcher and its
raw, AutoInit-sensitive predicate. This changes observable getter traffic on
already-passing programs but does not add a Test262 capability, manifest path,
or runnable variant. The focused replacement gate remains byte-identical at
286/376, with TSV/JSONL hashes
`055d52219998a0863a4241b3c5b374b917c1503d93b0715048ee2e171db3d012`
and
`dffcdbd8260a3d6e1c277d76797ba7187e40a971860ff802efaf8b3c6e65c0ad`.
The complete gate likewise remains byte-identical at 25,893/102,037, with
TSV/JSONL hashes
`2895a8d2ddbe5857e83b573827e46b4a60a97d89b5882727c85ff75d2ff9d368`
and
`64fed7fd3bb722d470bbd420e42995e138aed5d6f3588b7d2657973cb3968419`.
The exact R1h/R1i join therefore has zero transitions and zero previous-pass
regressions; focused QuickJS differentials, rather than pass-count movement,
are the acceptance evidence for this semantic-path milestone.

R1j publishes `Symbol.matchAll` and `String.prototype.matchAll` together with a
QuickJS-shaped RegExp String Iterator. Its static manifest is the complete
68-path union of the RegExp protocol, iterator prototype, and String entry
directories, expanding to 136 variants. The post-implementation vector admits
112 variants and records 64 passes; the other 72 remain explicitly classified
at unrelated feature, parser, or harness frontiers. The focused TSV/JSONL
hashes are
`03def26414f02bf5056ebb1421a28d28178c29946b07fc8d0e085fdbb9bfe72b`
and
`b020aa4bd8cd878a8b96aa66b1736eee991df4fc87b6adda3510101a0a911fd8`.

The exact R1i/R1j full join covers all 102,037 keys with no previous-pass
regression. Sixty-six variants move from `unsupported-feature` to pass; 20
reach an existing harness-parser frontier, 28 reach an existing parser
frontier. The complete vector moves to 25,959 passes and 33,283 admitted jobs.
Its
TSV/JSONL hashes are
`5f0e4601ce6b0212dacdd5c98fc1ba4cb2c8c217e3f0eb6c91411ad6e3f243fa`
and
`a829007d38ffe4bd84b7420200b0fef505671808e1a003326c2fccb6383edcd6`.
At R1j the capability profile contained 23 reviewed feature tags and had
SHA-256
`5aaca9f98ddca05a2bcb3bb6dfdc297f3f27a8314cb6efde61b25c2944548fd9`.
Earlier focused outcome rows remain unchanged; their whole-report hashes move
mechanically because this profile hash is part of every report header.

R1k ports numeric RegExp backreferences together with QuickJS's inseparable
non-Unicode Annex B decimal/octal fallback. The static focused manifest covers
49 paths and 98 variants, including syntax-priority canaries and linked
lookaround/named-group frontiers. At R1k, 74 variants were admitted; R1l
resolved four linked lookahead variants, R1o resolved 14 linked lookbehind
variants, and R1p admits the final six named-group variants. Later object
binding support resolves the last four parser frontiers, so the current gate
passes all 98 variants. Current focused TSV/JSONL hashes are
`fc91f2bc073844d86dc5b4c4b739da40e41a21267fde6f61d8fc6792d2b6c9a4`
and
`7ab11b9287f97ea7faf73331501b7fff2624a7892467b8f68879da2e155a1d8c`.

The exact R1j/R1k full join adds 68 passes with no previous-pass regression:
62 variants move from `unsupported-parser`, two from `unsupported-runtime`,
and four audited Unicode parse-negative variants from
`unsupported-negative-provenance`. The complete vector reaches 26,027 passes
and 33,287 admitted jobs. Its TSV/JSONL hashes are
`0bdf4955b2a9060279d0ad4232f653adb2018e9864654148f068caf22c0aabd6`
and
`7fcfbcd8157fa1d21d52af7df7e3b2226db7be08bfe42254994a28d56a5b9857`.
The profile still has 23 feature tags, now with 103 exact audited negative
paths and SHA-256
`6f27d9fcfa5a13423796ad48fe8ccbf8d5edcd49118ad7f0f64cc5a936090645`.

R1l ports forward positive and negative lookahead using QuickJS-shaped paired
assertion instructions and typed control frames on the existing non-recursive
executor stack. Positive success discards internal alternatives while
retaining captures and compacting their undo records into any surviving outer
transaction; negative completion always rolls assertion-local state back.
Non-Unicode assertions retain Annex B quantification, while `u` mode preserves
the distinct `*`/`+`/`?` versus brace-quantifier syntax priorities.

The static focused manifest covers 26 paths and 52 variants. All 52 are
admitted and pass. Its TSV/JSONL hashes are
`f4087df9d8fb3a91b9f92e733ba4568c62c6c083a340a27b449ecec54deb025b`
and
`18551f6e79bc933a9337b5709011657b9c94e46be7f77120049a63e9753761fb`.
The exact R1k/R1l full join converts 50 `unsupported-parser` and two
`unsupported-runtime` variants to pass, with no other category movement and no
previous-pass regression. The complete vector reaches 26,079 passes while
admitted jobs remain 33,287. Its TSV/JSONL hashes are
`9a60ea477bb8d383b316b9418683865031b43b3609400d7bcacb448cb535a85b`
and
`b69f3de1d2e61d3cb7667e6de1ffe2f5a811569df83b1cf34929008aaf8e393a`.

R1m ports `u`-mode Unicode property escapes from pinned QuickJS. The generated
Rust catalog contains 38 General_Category sets, 176 Script sets, 176
Script_Extensions sets, and 55 accepted binary properties. Exact aliases,
errors, non-`u` identity behavior, `\P` inversion-before-folding under `iu`,
scoped modifiers, astral code points, lone surrogates, and class-range
priorities are locked by 37 match and 28 compile/error oracle vectors.

The focused Test262 manifest contains the 144 direct property-escape paths
which do not require the generated helper corpus, plus four scoped-modifier
canaries. All 296 variants pass. Its TSV/JSONL hashes are
`66a129065346b23b454c6275b15301508bc8a4afaf6dacd8a473d6a948b7c392`
and
`87b704d71d7d8e33403abd81445cfd302c136fc2de30308c7f7caf9ceed9d869`.
The profile now contains 24 feature tags and 245 exact audited negative paths,
with SHA-256
`6d5bb9a92d00babb6a4a0bcb19334fbcfcd532bb5382ce278ce85a960d40d781`.

The exact R1l/R1m full join adds 298 passes and admits 1,170 more jobs:
288 variants move from `unsupported-feature` to pass, ten move from
`unsupported-parser` to pass, and 882 generated Unicode-property variants move
from `unsupported-feature` to the existing harness-parser frontier. There are
no previous-pass regressions or other category changes. The complete vector
reaches 26,377 passes and 34,457 admitted jobs. Its TSV/JSONL hashes are
`275fd8b3f6b1e5f078b6aad58bfc33797abaf6637179f47cc52228bc8f52feda`
and
`c2e14d42cfbb933946d9ce738d27c371e15fa3b9865131c2a6160cfe70b480f9`.

R1n completes that generated Unicode-property data tranche without claiming
general destructuring support. The compiler lowers identifier-only
`const`/`let`/`var` array BindingPatterns in synchronous `for-in`/`for-of`
through nested QuickJS-shaped iterator records; holes, empty patterns, trailing
commas, early exhaustion, abrupt close order, and fresh lexical cells are
covered. Assignment patterns, object patterns, defaults, rest, and nested
patterns remain typed frontiers.

The Test262 worker now publishes only the QuickJS
`js_string_codePointRange` native helper needed by the pinned harness, with
exact `ToUint32`, UTF-16, realm, function-metadata, and non-constructor
behavior. Other `$262` hooks remain absent. RegExp normalized range lookup uses
binary search, matching the upstream data-plane shape instead of scanning up
to 1,372 intervals for every input code point.

The cumulative Unicode-property gate is now 589 paths and 1,178 variants: the
original 148 paths plus all 441 generated code-point property files. Every
variant passes. Its TSV/JSONL hashes are
`1cc6e3fec21a989c4a916a5dcfd069c9600efaa03883611a7dc5888ead73dd48`
and
`8b0dd3a9e76c7795f945631987f4dbd1ab3c5596dfda921993ea4594cb2f072e`.
The 28 generated properties-of-strings files remain behind the unimplemented
RegExp `v`-mode frontier.

The same harness change advances 20 already-tracked MatchAll variants:
14 pass and six reach the object-literal method/accessor parser frontier. The
complete R1m/R1n join matches all 102,037 keys. Its 930 outcome changes are
896 `unsupported-harness-parser -> pass`, six
`unsupported-harness-parser -> unsupported-parser`, 20
`unsupported-parser -> pass`, six `unsupported-parser -> fail-runtime`, and
two `unsupported-parser -> fail-parse`. All 935 changed complete rows are
inside the pre-audited 475-path set; there are no previous-pass regressions or
outside-set changes. Admitted jobs remain 34,457, while the complete vector
reaches 27,293 passes. Its TSV/JSONL hashes are
`6035ae86888c4db9e99b73be65e706bf7b90ee83c108082a3e7931f2000edc61`
and
`fb37235d0d651a2d424cb4f63c16b6662813183f25fd2126e970bacb3506c50d`.

R1o ports positive and negative variable-length lookbehind through the same
non-recursive assertion controls used by R1l. Code generation retains
alternative priority while reversing each alternative's terms, emits
QuickJS-shaped `Prev` instructions around ordinary consuming atoms, swaps
capture boundaries, and compares participating numeric backreferences
right-to-left without crossing the capture start. Nested lookahead/lookbehind,
greedy and lazy captures, assertion atomicity and rollback, anchors, word
boundaries, scoped case folding, and UTF-16/Unicode reverse movement are
covered by 42 match and ten compile/error vectors against pinned QuickJS.

The frozen focused gate contains 27 paths and 54 variants. At R1o, its 17 pure
lookbehind paths and eight audited parse-negative paths contributed 50 passing
variants while four co-tagged named-group variants stayed gated. R1p resolves
those four, so the current gate passes all 54. Focused TSV/JSONL hashes are
`590b466885fe087bc30cb02e1adc1b1076af0322e229a998af8cda3a680131dd`
and
`5aca0c7d11afea0d6c1facd893663ad2000f7a95860703112c641dd8a8fa914c`.

The exact R1n/R1o full join matches all 102,037 keys. It records 34
`unsupported-feature -> pass` and 16
`unsupported-negative-provenance -> pass` transitions; all 50 outcome changes
and 54 complete-row changes are inside the frozen set, with no previous-pass
regression or outside-set drift. The complete vector reaches 27,343 passes and
34,507 admitted jobs. Its TSV/JSONL hashes are
`50fe24e393c2532e2c25fc2113e6bbb48c163678a6bc8a0991f8c6ad0d8273c1`
and
`c997357b861109bfd17c46ad0c8059004f2b797cf9254394b90892dca078810b`.

R1p ports ordinary named captures and named backreferences from pinned
QuickJS. The runtime-independent compiled program stores normalized
`JsString` names aligned to captures 1..N, while the matcher reuses the
existing multi-capture forward/backward backreference instructions. The
parser preserves QuickJS's fixed group-name buffer, Unicode 17 identifier
rules, Annex B `\k` fallback, wrapping global alternative scope, and
forward-name scan cursor quirk. Match `groups` and `indices.groups` are
null-prototype C/W/E objects with QuickJS duplicate-name value and insertion
order. Named replacement deliberately leaves the direct helper before any
state mutation and uses the generic `$<name>` path.

Fifty-nine execution, grammar, construction, result, replacement, and
QuickJS-quirk vectors match the pinned oracle; a separate Rust test locks
defining-realm ownership. At the R1p landing, the frozen focused gate is 101
paths and 202 variants: 184 are admitted and 158 pass. Six reach pre-existing
arrow-function parse failures, 20 reach typed
class/object-method/destructuring parser frontiers, and 18 stay behind
`regexp-match-indices`, `Symbol.iterator`, or class syntax. At that landing,
the 19 paths tagged with
`regexp-duplicate-named-groups` remained outside that declaration even though
the lower-level QuickJS duplicate selection behavior was implemented; R1q
audits them below. R1p focused TSV/JSONL hashes are
`505845ba54ec78ae1a636f91f7285e447444d3ffca8b66a03592591573a15d26`
and
`5daec58cf49af34cdf2ad8e70d5a945513e6490180ab4c74e9e996f39d4fa234`.
Later object-binding and rest-parameter milestones move the frozen gate to 194
passes; R3f derived construction resolves its remaining four class frontiers,
so the current gate has 198 passes, two feature-gated variants, and two at an
unrelated runtime frontier.
Current TSV/JSONL hashes are
`37d54ae152bd48b0fc35625d4776e082c3baa2b4024382bd274f0633ea2323e3`
and
`b96318614cf6bd6a9d0d8b1c360cccd0a2f12131f59988baba24002201aff846`.

The exact R1o/R1p full join again matches all 102,037 keys. It records 158
`unsupported-feature -> pass`, six `unsupported-feature -> fail-parse`, 20
`unsupported-feature -> unsupported-parser`, two `unsupported-parser -> pass`,
and two `unsupported-runtime -> pass` transitions. There are 188 outcome and
204 complete-row changes with no previous-pass regression. Four changes lie
outside the frozen manifest and are explicit linked `\k` canaries: the
Unicode restricted-identity-escape test now receives the required
`SyntaxError`, and the String split separator test now receives Annex B
identity-escape behavior when no named capture exists. The vector reaches
27,505 passes and 34,691 admitted jobs. Full TSV/JSONL hashes are
`ff31a5f63b2b9e27f5650dd99c301cbff9c863314cce48e592f97b6ca1df2704`
and
`e1766ea22ab3e33ef610310a6d83ce101eb66dcfa598d581ebaed257295e9402`.

R1q declares the duplicate-named-capture feature after a separate pinned
QuickJS source and adversarial-probe audit confirmed that R1p already mirrors
the target's global wrapping 8-bit scope, nested-alternative leakage,
multi-capture backreference selection, reset behavior, result order, indices,
and replacement semantics. No production engine change is needed.

The frozen focused gate is the complete 19-path/38-variant feature set. It
admits 32 variants and passes 26 at the R1q landing. Six variants in the
constructor,
`RegExp.prototype.compile`, and matchAll syntax tests reach the existing arrow
parser frontier; the six match-indices co-tagged variants remain gated in that
historical report and are admitted by R1r.
Focused TSV/JSONL hashes are
`bd55aacd10c14cf1f0f7a38e11a610ad3763bce8c4f326c9a6ae3ad548a8ef30`
and
`1b9dc971d9c965910b7e0bd88573e80553d17b74651c0ef4762dd34d998cc666`.

The exact R1p/R1q full join matches all 102,037 keys. It records 26
`unsupported-feature -> pass` and six
`unsupported-feature -> fail-parse` transitions. All 32 outcome changes and
38 complete-row changes stay inside the focused manifest, with no
previous-pass regression. The complete vector reaches 27,531 passes and
34,723 admitted jobs. Full TSV/JSONL hashes are
`16759de6e768905a3feae8dc96889936668838f42b64217bd70776cb6e56db96`
and
`36b947828eda57d0216d84e623b6af51143d26586860db3639cc3875765fc7e0`.

R1r declares `regexp-match-indices` after pinned QuickJS source review and
focused probes confirmed that the existing production engine already matches
the target's `d` flag and canonical flag order, `hasIndices`, UTF-16 match
ranges, unmatched-capture `undefined` values, null-prototype named
`indices.groups`, duplicate-name selection, construction/legacy-compile
behavior, and observable descriptors. No production engine change is needed.
Seven dedicated differential tests lock result/pair descriptors,
low-surrogate `lastIndex`, protocol propagation, replacement non-observation,
and nested defining realms against the pinned oracle.

The frozen focused gate contains 31 paths and 62 variants. At the R1r landing,
it admits 50 and passes 38; two variants fail at the existing arrow-function
parser frontier, four stop in the existing `deepEqual.js` harness frontier,
and six reach the typed object-setter parser frontier. Ten remain behind the
independently gated `regexp-dotall` feature in that historical report and are
admitted by R1s, while two retain the missing `$262.createRealm` host
requirement. Focused TSV/JSONL hashes are
`b626f453c4a22402c9bf35f0b6a95ad3cf54cb2095ff21c023a150ec6904a230`
and
`edc7cb06eb9d18596202ae4d6f9faa4e56c1e2c4a6a81b51a54a26b0b34cd31f`.
Later binding and rest-parameter milestones move the current gate to 58
passes; two variants remain feature-gated and two require
`$262.createRealm`. Current TSV/JSONL hashes are
`da103588eaf15c8864b2aff5966f5e7a60fe533ca85be14607956695cf193b1d`
and
`ec5f84df5135174cbe78b91218a13879a53e758d97c583120c32b7a8026b5f7a`.

The exact R1q/R1r full join matches all 102,037 keys. It records 38
`unsupported-feature -> pass`, two `unsupported-feature -> fail-parse`, four
`unsupported-feature -> harness-error`, and six `unsupported-feature ->
unsupported-parser` transitions. All 50 outcome changes and ten detail-only
changes stay inside the focused manifest, for 60 complete-row changes and no
previous-pass regression. The complete vector reaches 27,569 passes and
34,773 admitted jobs. Full TSV/JSONL hashes are
`e09478accaf05c27e39555c5a4c1889617c97ce5c1454ddf945c7f675ea3d2ef`
and
`95ea74491558035ac02af4f60c3a2d202120798fc2ab08c41c7050a6031e950b`.
The capability profile now contains 28 reviewed feature tags and 307 audited
negative paths, with SHA-256
`b39bee15a2aaa88e00c8f7ca6cb0736313456d43a77e176a8c5cf7844e9ea718`.

R1s declares `regexp-dotall` after a pinned QuickJS source review and focused
probes confirmed that the existing Rust implementation already follows the
target end to end. The `s` flag uses QuickJS's bit, the compiler selects the
all-character instruction instead of ordinary dot, UTF-16 and Unicode width
come from the shared executor, scoped modifiers restore their enclosing state,
and the constructor, legacy `compile`, accessors, canonical flags, protocols,
species paths, and defining-realm brand checks retain the flag exactly. No
production engine change is needed. Six dedicated differential tests cover the
oracle-vector self-check, line-terminator and UTF-16 matching, the public and
construction surface, nested scoped modifiers, matchAll/split species flags,
and cross-realm getter brands and error realms.

The frozen focused gate contains all 17 paths and 34 variants tagged with
`regexp-dotall`. At R1s it admitted 26 and passed 18, with linked Arrow,
accessor, `u180e`, `regexp-v-flag`, and create-realm frontiers kept visible.
Later slices resolve Arrow and `u180e`; R2g resolves the final four accessor
consumers. The current gate admits and passes 30 variants, while two remain
behind `regexp-v-flag` and two retain the missing `$262.createRealm` host
requirement. Its exact outcome summary is
`pass=30 unsupported-feature=2 unsupported-host-create-realm=2`. Focused
TSV/JSONL hashes are
`3d5bda20dece92150f0398cb6f2d70a4114ff46fea69c7326ef056e439c7e246`
and
`a584c2db7b136338cb5ea9ca5116572f17ce2121740b5670889ab035e979bd23`.

The exact R1r/R1s join matches all 102,037 keys. It records 18
`unsupported-feature -> pass`, four `unsupported-feature -> fail-parse`, and
four `unsupported-feature -> unsupported-parser` transitions. All 26 outcome
changes and six detail-only changes stay inside the frozen manifest, for 32
complete-row changes and no previous-pass regression. The complete vector
reaches 27,587 passes and 34,799 admitted jobs. Full TSV/JSONL hashes are
`44f7ee3d6de6c97962c4b372da2f492882b8834d76663b334dd46265fae9e69f`
and
`fa263cbcd0483000f0645f017d486e4a4403d5227b97ce3bf5e812bf8a6857ce`.
The capability profile now contains 29 reviewed feature tags and 307 audited
negative paths, with SHA-256
`84fe6615092829a107e66beb49ac54b00a1910616424494f47e5f75c8ccc7880`.
The admission and differential locks add no production code; `runtime.rs`
remains 9,677 lines.

R1t declares `u180e` after pinned QuickJS source review and focused probes
confirmed that the existing Rust implementation already matches the target
where that semantics is implemented. U+180E is not ECMAScript whitespace or a
line terminator; it is preserved in comments and literals, rejected between
tokens, rejected by Number conversion, honored as a prefix-parser stopping
point, retained by trim, skipped as Case_Ignorable for Final Sigma, excluded
from RegExp `\s`, and matched by dot and `\S`. Seven dedicated differential
tests lock these lexer, conversion, string, casing, and RegExp boundaries.
Global `eval` and JSON are recorded as independent subsystem frontiers rather
than receiving U+180E-specific production code.

At the R1t landing, the frozen focused gate contained all 25 paths and 50
variants tagged with `u180e`. All 50 were admitted and 40 passed. Ten variants
failed at runtime because the five `*-eval.js` paths required the then-missing
JavaScript global `eval`; the exact parse-negative path was separately
provenance-audited and passed as a real SyntaxError. Its historical summary was
`fail-runtime=10 pass=40`. R1t focused TSV/JSONL hashes are
`3e42dd0c0e7272d51f02a03f95c1d907218b9f3ee5e29a20c0c6760565fbaf0c`
and
`4d6e6d514c9a4e6108f828b57b53507e24564df2d0a670a31132a878dbbc8d5c`.

The exact R1s/R1t join matches all 102,037 keys. It records 40
`unsupported-feature -> pass` and ten `unsupported-feature -> fail-runtime`
transitions. All 50 outcome and complete-row changes stay inside the frozen
manifest, with no detail-only changes or previous-pass regression. The
complete vector reaches 27,627 passes and 34,849 admitted jobs. Full
TSV/JSONL hashes are
`7ea006b596e26f56712c9618f74cd8a5af9aada88702d08f855e6bc8eb313424`
and
`6d1d42c46ff6ff145dd72890c90abf6047d11910545599186e5f285028a21fc4`.
The capability profile now contains 30 reviewed feature tags and 308 audited
negative paths, with SHA-256
`3c5dee6fa18c428a45556488873ab216dd99e9f8859875ce2e4d1475d307aca6`.
The admission and differential locks add no production code; `runtime.rs`
remains 9,677 lines.

R1u installs the global eval intrinsic shell while keeping primitive String
source execution fail-closed. Pinned QuickJS differentials cover the callable
metadata and descriptors, lack of a prototype and constructor protocol,
no-argument behavior, exact non-String identity with no coercion, global
delete/replacement with a held alias, and cross-realm calls. The original
callable is retained as a realm-local root independently of the mutable global
property, matching the identity model that QuickJS's direct-eval opcode uses.
Primitive String input returns the uncatchable engine-level `Unsupported`
frontier rather than being run through the host Script evaluator.

Before R1u, a source-and-diagnostic inventory identified 1,085 eval-bearing
paths / 1,517 fail-runtime variants as the audit ceiling. Of those, 1,056 paths
/ 1,465 variants execute String source through direct, indirect, or mixed eval;
the remaining 29 / 52 only depend on the callable surface. The exact frozen
join moves 1,503 of those variants, while 14 remain unchanged fail-runtime
behind earlier or secondary independent failures. This is an architectural
work queue, not a predicted pass delta, because String execution will expose
further parser and runtime gaps. The independent `$262.evalScript` host hook
accounts for another 31 paths / 44 variants and is not global eval.

The complete positive focused gate contains 31 paths and 55 variants, and all
55 pass. Its manifest, TSV, and JSONL SHA-256 values are
`ae398ca6148d5babf468e7ba1cdcf956f454d35cdb6f612a3c4444d2b3c97cea`,
`9d364c24169423efa49ecfa384c86280f94011b430fa787f72a8214fe867a6f6`,
and
`63d5717d85f57c19705196aee0333c18cc270242b37e431622a035a8c34cf2fd`.
The R1u versions of the U+180E, RegExp-core, RegExp-match, and String-split
focused gates now classify their String eval consumers as
`unsupported-runtime`; the Date gate gains two linked passes. No other existing
focused manifest changes.

The exact R1t/R1u full join matches all 102,037 keys. It records 55
`fail-runtime -> pass`, 1,448 `fail-runtime -> unsupported-runtime`, and 41
`pass -> unsupported-runtime` transitions, with 1,544 outcome and complete-row
changes and no detail-only changes. The 41 former passes were all audited as
missing-eval false positives: 31 accepted the wrong outer `ReferenceError`,
while ten swallowed it and asserted state left untouched because the eval
source never ran. This correction makes the scoreboard more truthful even
though the net gain is only 14 passes. The full vector reaches 27,641 passes
and 34,849 admitted jobs. Full TSV/JSONL hashes are
`59736a4a4f63122a458a33374d2afd873a706aeb7ff271b52f9fa4aa2aa71fbe`
and
`c4849aecc54afcc7c73bb182cd240bc9cf35634bc74bc4d5558d6951898af2f2`.
The capability profile remains byte-identical at
`3c5dee6fa18c428a45556488873ab216dd99e9f8859875ce2e4d1475d307aca6`.

R1v adds the QuickJS-shaped direct-eval opcode path but intentionally changes
no Test262 classification. The compiler recognizes only a syntactic
IdentifierReference named `eval`, retaining the call-site scope in parser IR;
the VM then compares the resolved callee with the current realm's cached
original `%eval%`. Identity mismatch remains an ordinary call with an
undefined receiver and all evaluated arguments. Identity match bypasses the
native callable frame and forwards only the first argument (or `undefined`) to
the existing non-String/typed-Unsupported shell. This is the execution shape
required before String source can receive a linked caller environment.

The 31-path/55-variant focused report is byte-identical to R1u: 55 pass, zero
fail, unsupported, or skipped outcomes, with TSV/JSONL SHA-256
`9d364c24169423efa49ecfa384c86280f94011b430fa787f72a8214fe867a6f6`
and
`63d5717d85f57c19705196aee0333c18cc270242b37e431622a035a8c34cf2fd`.
The complete 102,037-key report is also byte-identical, with zero outcome,
complete-row, detail-only, missing, extra, or duplicate changes. It remains at
27,641 passes and 34,849 admitted jobs; full TSV/JSONL SHA-256 are
`59736a4a4f63122a458a33374d2afd873a706aeb7ff271b52f9fa4aa2aa71fbe`
and
`c4849aecc54afcc7c73bb182cd240bc9cf35634bc74bc4d5558d6951898af2f2`.
This zero movement is the acceptance result for R1v, not a claim that direct
String eval is complete. Spread arguments (`OP_apply_eval`), optional calls,
and the immutable eval-environment descriptor table remain later milestones.

R1w adds that immutable direct-eval caller-environment descriptor table without
opening String source execution. Descriptors walk the exact inner-to-outer
scope chain, divide it into current and ancestor function segments, retain
authoritative names on Local/Argument/Closure sources, force eval-visible
`arguments` and private function-name bindings, and reuse existing closure
slots. Publication checks the segment count against function-tree depth,
Body/Root topology, source partition, bounds, flags, parent-relay names, global
exclusion, and atom ownership. For primitive String input the VM validates the
complete descriptor and materializes live caller VarRefs before returning the
existing typed Unsupported error; non-String input still returns before scope
inspection or `this` normalization.

The R1w focused run remains 55/55 and is byte-identical to R1v, with TSV/JSONL
SHA-256
`9d364c24169423efa49ecfa384c86280f94011b430fa787f72a8214fe867a6f6`
and
`63d5717d85f57c19705196aee0333c18cc270242b37e431622a035a8c34cf2fd`.
The complete report also remains byte-identical: 27,641 passes among 102,037
variants, 34,849 runnable jobs, and TSV/JSONL SHA-256
`59736a4a4f63122a458a33374d2afd873a706aeb7ff271b52f9fa4aa2aa71fbe`
and
`c4849aecc54afcc7c73bb182cd240bc9cf35634bc74bc4d5558d6951898af2f2`.
That zero movement is the required result: the next compiler/runtime milestone
must add QuickJS-shaped eval bytecode publication and explicit defining-realm
ownership before any bounded String-execution slice can be enabled. Persistent
sloppy dynamic variables remain a separate declaration-environment milestone.

R1x opens that bounded primitive-String slice with a dedicated Eval root rather
than reusing the Script root. Direct eval imports the caller descriptor as an
ordered authenticated external closure prefix, while indirect eval has no
caller bindings and executes in the original `%eval%` callable's defining
realm. Eval-local lexical declarations, expression/statement completion,
strict inheritance, caller-cell writes, returned closures and catchable parser
errors now execute. Dynamic `var`, FunctionDeclaration instantiation, nested
syntactic direct eval, direct `new.target` and ill-formed UTF-16 source remain
typed frontiers rather than being approximated.

The focused eval gate grows from 31 paths / 55 variants to 74 paths / 138
variants, all passing. The 43 added paths account for 83 added passing
variants. Manifest, TSV and JSONL SHA-256 values are
`99aa8af497946369babf6f639f5ccfb4c8da5bffb7587f75825ead076556c314`,
`2b3f87db4ae4333cee6ff896c3d0ead2e061fd98000b0673a6fa32ff4acd7ad4`
and
`29e965a24abdd74d70ea0970a8c2afd6ce20f5b52153239f1b15bb7ec651b34e`.
The capability profile remains byte-identical; eight Test262 eval-lexical
paths are therefore covered by focused Rust/QuickJS tests but not added to the
gate because globally declaring the suite's `let`/`const` feature tags would
reclassify a much broader surface. One runtime-negative indirect parse case is
likewise left for a coordinated negative-provenance profile update.
Opening String eval also moves already-frozen collateral gates without changing
their manifests: RegExp core rises from 438 to 448 passes, RegExp match from
184 to 186, generic String split from 236 to 240, and U+180E from 40 to 50.

The exact R1w/R1x full join matches all 102,037 keys with no additions,
removals or previous-pass regressions. It records 575
`unsupported-runtime -> pass` and 13 `unsupported-runtime -> fail-runtime`
transitions. Ten exposed failures stop at existing arrow, async, generator or
non-simple-parameter parser frontiers. The remaining three are pinned QuickJS
behaviors already recorded as SpiderMonkey staging failures: the two
`try-completion.js` variants and `regress-602621.js`. Changing them here would
move away from the declared QuickJS target. The full vector reaches 28,216
passes while runnable remains 34,849; TSV/JSONL SHA-256 values are
`c62f104a2a3801c9b3eca38362fa5075f1fc21564395c58f45dfb23153ef1530`
and
`526c00942821ff5f153e08d3056627bbe35e7e12e4cde3702a55c220351bbd09`.

R1y opens QuickJS-shaped eval `var`, ordinary FunctionDeclaration, and Annex B
declaration environments without broadening the Test262 capability profile.
The new bytewise-sorted manifest freezes 497 paths: 54 core eval-declaration
paths and 443 Annex B consumers. They expand to 519 runnable variants, all of
which pass. Nested direct eval, `with`, generator/async declarations, and the
shared-profile lexical-feature surface remain outside this focused vector.
The manifest, TSV and JSONL SHA-256 values are
`ecc3cb3b50f8b59cae548fa9c1017dfd1d71878644bf204146d4002015c2bd70`,
`1b9cfacfe80671d5e2579865b7efb1478b5d7c1da70b240b71a1cccc3cf1c80a`
and
`0a0e7db1f1c80431302b14b66148f34efa998f38811e965f126c2d548ab6dd6d`.
The gate also pins a separate 15-path hash for collateral Test262 failures
which reproduce on QuickJS 2026-06-04, so target behavior is not mislabeled as
an Oxide regression.

The exact R1x/R1y join has the same 102,037 unique keys, with no missing,
extra, duplicate, or previous-pass rows. Outcome movement is:

- 752 `unsupported-runtime -> pass`;
- 16 `fail-runtime -> pass`;
- 16 `unsupported-runtime -> fail-runtime`.

Fifteen of the newly exposed failures are the pinned QuickJS collateral set;
the remaining test reaches the existing generator/async declaration frontier.
One additional row remains `unsupported-runtime` but now stops at the narrower
nested-direct-eval frontier after its preceding labelled FunctionDeclarations
execute. Net growth is 768 passes. The complete report reaches 28,984 passes,
keeps 34,849 runnable jobs, and contains no engine or runner fault. Full
TSV/JSONL SHA-256 values are
`cca9eadc35c3c5f9acdf24b00cb9d65b0a2ca20a65860e137185f4f7fa48c4e4`
and
`348e25af619fcf81ef534b82f57571889c1d2ab7f06cad3d5233e7d49fae240f`.

R1z removes the recursive direct-eval environment frontier without broadening
the capability profile or runnable set. Its bytewise-sorted manifest freezes
all 25 formerly blocked paths / 30 variants. Twenty-nine pass; the remaining
`staging/sm/global/eval-in-strict-eval-in-normal-function.js` sloppy variant
reaches the independent `with statements are not implemented yet` frontier.
The manifest, focused TSV and JSONL SHA-256 values are
`0b5e9ab5d51376e66a3b5b28614803fc32843649bbf6494747892de20c9032fc`,
`3a6dd32c7f3d0154b36946c6894f9cdba79a12d7086bf5602a210360b90f5248`
and
`23f4e2115b5a1ed322eac39faa51517912825562e71965a73261b3f4ad86a1fb`.

The exact R1y/R1z full join retains all 102,037 unique keys. It records 29
`unsupported-runtime -> pass` transitions and one detail-only refinement,
strictly inside the frozen manifest, with no missing, extra, duplicate, or
previous-pass row. Passes rise from 28,984 to 29,013; runnable remains 34,849
and `unsupported-runtime` falls from 135 to 106. Full TSV/JSONL SHA-256 values
are
`2ba53703827155be4ce36f11a52b48c3ac1bb4efc8f61da9cc31b6b1ca8e125a`
and
`c9369e14acb1469b20aea4caab2c0a880cb7f040a72718d629f38e1301582650`.

R2a fixes QuickJS-specific precedence between a named function expression's
private self binding and same-named direct/nested eval declarations. Authored
caller code keeps the private FunctionName binding, while eval's ordered
external scan still sees the nearest `<var>` property first. The accompanying
pinned-QuickJS differential also freezes the target's `add_eval_variables`
metadata-loss quirk, including entry-before-children ordering with source-keyed
first-flags/kind-wins closure slots, plain-leaf FunctionName restoration, and
the contrasting Eval-root relay behavior.

The pinned Test262 tree contains no exact instance of this declaration shape:
that declaration-shape cohort is 0 paths / 0 variants. R2a therefore adds no
empty manifest and records no Test262 progress increase. The full gate remains
byte-identical across all 102,037 keys: 29,013 pass, 34,849 are runnable, and
the TSV/JSONL hashes remain
`2ba53703827155be4ce36f11a52b48c3ac1bb4efc8f61da9cc31b6b1ca8e125a`
and
`c9369e14acb1469b20aea4caab2c0a880cb7f040a72718d629f38e1301582650`.

## R2b `with` statement gate

The dynamic-environment cohort remains reproducible independently of the full
vector. `tests/test262-with.txt` preserves every R2a path whose execution
stopped at the exact typed frontier
`with statements are not implemented yet`: 203 bytewise-sorted paths expand
to 205 positive, synchronous script variants. R2b removes that parser/runtime
frontier completely. The focused result is 198 passes, five parse failures and
two runtime failures. The five parse failures all reach the existing arrow
function grammar gap; one runtime failure reaches generator syntax through
String-source eval and the other reaches arrow syntax through direct eval.
They remain in the stable cohort so later adjacent milestones can turn them
into passes without rewriting this evidence boundary.

The manifest and `(path, variant)` key-set SHA-256 values are
`8f43b8f924d127814ea157637acebbb4e37fc89f97e6a76789e5e329d10250d6`
and
`1c04aebebd7c6e575113ca1466832c92096fef90af088aa1f3d317561aed0d4e`.
The R2b focused TSV/JSONL SHA-256 values are
`e22e130dfd23e5509aab68cf4ac244ecb6f5a827067c3622dc34014f9cf9d65d`
and
`cfc1aeeaf7fd6cc8ab1a3741cdbfe17db50b8a2817a054bf182838108cf22129`.
Reproduce and validate the complete vector with
`scripts/run-test262-with.sh`; the entry point derives the repository root and
pinned suite location at runtime and does not encode a workstation path.

This is a focused progress gate, not a full-parity claim. The implementation
uses the hidden with-object scope binding and its closure/eval relay,
`ToObject`, prototype-aware `HasProperty`, `Symbol.unscopables`, and the
get/put/delete/make-reference/get-reference paths which preserve the implicit
receiver of a call. The relevant upstream anchors are
`quickjs.c::resolve_scope_var`, `var_object_test`, the `TOK_WITH` statement
case, `JS_GetGlobalVarRef`, and `OP_with_*`. The eval-variable object remains a
distinct environment source with different ownership and Reference timing.

The exact R2a/R2b full join retains all 102,037 unique keys and changes only
the 205 frozen rows. It records 173 `unsupported-parser -> pass`, five
`unsupported-parser -> fail-parse`, one `unsupported-parser -> fail-runtime`,
25 `unsupported-runtime -> pass`, and one `unsupported-runtime ->
fail-runtime` transition. There are no missing, extra, duplicate,
detail-only, outside-manifest, or previous-pass changes. The complete vector
therefore rises by 198 passes to 29,211 while runnable remains 34,849. Full
TSV/JSONL SHA-256 values are
`8eba52564839d3a11a92ac28c883494cfc51d1f49785b07e7d3ac62ec867965c`
and
`54122f8b86f8cdbea6f3de6aa9532f770b72df1f6bf28bdc7cd62ec665b32ca1`.

## R2c synchronous ArrowFunction gate

R2c implements the synchronous, simple-parameter ArrowFunction slice on the
QuickJS compiler path. The frozen differential covers 34 cases: identifier and
parenthesized heads, line-terminator lookahead, reserved-word errors,
expression and block bodies, strictness, `name`/`length`/source metadata,
lexical `this`/`arguments`/`new.target`, nested closures, `with`, direct and
nested eval, `typeof this`, missing `prototype`, and non-constructability.
At the R2c landing, async Arrow, default/rest/destructuring parameters,
class/`super`, and method/accessor adjacency remained typed independent
frontiers.

The focused manifest fixes 40 paths / 66 positive synchronous variants. All 66
are admitted and pass. Its manifest and key-set SHA-256 values are
`75c1e7e8c12a493eb1b2f38b662ca51c2a20bbe68434900b2a890573ad8d4360`
and
`52684eee5c0df05893b6f6d00376669f2b845ec35a7f01ac0c4bea96cc324384`;
focused TSV/JSONL SHA-256 values are
`fd5b76fb8cb81bcebe786abc6c7992e318b0b7bf8ce9e5b7b58c2a75111b5108`
and
`d363b03a69f71bf760d8366e4b565b743d85a7f3127ea401e45aeb51b0aa50e4`.
Reproduce it with `scripts/run-test262-arrow.sh`.

Declaring `arrow-function` changes the shared capability-profile SHA-256 to
`5c3c11f7c7c81fd54b706d6d50b5f28f6dddbd915c7b3543af9e5e6b5fb08aae`
and admits 575 more full-suite jobs. Of those, 534 pass, two fail to parse, 28
reach runtime failures, and 11 stop at typed parser frontiers. The explicit
feature-tag cohort contains 1,800 variants in total: another 522 remain gated
by other feature tags, 496 are excluded by the pinned QuickJS configuration,
195 are async, and 12 require detach-array-buffer host support. The profile
declaration therefore exposes the broad queue; it does not claim the remaining
Arrow-adjacent grammar or dependencies are implemented.

Arrow syntax also appears in untagged tests. The exact R2b/R2c join retains all
102,037 keys and every previous pass while adding 1,043 passes:

- 474 `fail-parse -> pass`;
- 5 `fail-runtime -> pass`;
- 30 `harness-error -> pass`;
- 534 `unsupported-feature -> pass`.

The first full run caught one old-pass regression in strict direct eval:
`typeof this` had promoted the authenticated pseudo read to
`GetOrUndefined`, which the new resolver initially rejected as a non-read.
The dedicated QuickJS differential and the original Test262 path now pin that
case. The final join has zero previous-pass regressions, 30,254 passes and
35,424 runnable variants. Full TSV/JSONL SHA-256 values are
`c28acb10ae63e46e8aad1372f679c3be3b283322c2f690e0296bf0a77e243345`
and
`e82fbff1bdd49b300ea561d7ad21b9c3d62ed4d640f7080c3375bc9044bf32f9`.

## R2e capability-profile truth-up

R2e first audits already-implemented surface area before adding another grammar
slice. Direct single-variant runs in quickjs-oxide and the pinned QuickJS
2026-06-04 oracle prove 22 feature cohorts that the conservative profile had
continued to reject:

- `Array.prototype.at`, `Array.prototype.includes`, `array-find-from-last`;
- `Object.fromEntries`, `Object.hasOwn`;
- `String.fromCodePoint`, `String.prototype.includes`,
  `String.prototype.isWellFormed`, `String.prototype.toWellFormed`,
  `String.prototype.trimEnd`, `String.prototype.trimStart`, `string-trimming`;
- `__getter__`, `__setter__`;
- `coalesce-expression`, `logical-assignment-operators`, `new.target`,
  `numeric-separator-literal`, `object-spread`;
- `const`, `let`, `optional-catch-binding`.

The same audit admits 95 exact negative-test paths only after both engines
produce the expected phase and error type. The runner also re-reads those paths
from the pinned suite and rejects the profile if any no longer carries negative
metadata. Together these additions move the profile from 31/308 to 53 reviewed
feature tags and 403 audited negative paths, with SHA-256
`e2043efeaa2d8b4420d0c82550f7ba42d53588897ec14ac87f6f03c4358a8218`.
No engine semantic code changes in this step.

All 28 focused, non-full Test262 gates preserve their existing key sets,
runnable counts, pass counts and outcome summaries. The 26 metadata baselines
and four direct TSV/JSONL baselines are regenerated only because the canonical
report header embeds the capability-profile hash.

The complete 102,037-key join admits 1,342 more variants and reaches 31,459
passes: 1,205 rows move from `unsupported-feature` to pass and 137 move to an
existing typed parser frontier. Another 507 rows retain their outcome while
their unsupported-feature detail loses one or more newly reviewed tags. The
join has no missing, extra, duplicate, or previous-pass rows, and all 1,849
complete-row changes carry at least one of the 22 new tags. Runnable jobs rise
from 35,424 to 36,766. Full TSV/JSONL SHA-256 values are
`7e05dd58a0387d8639d09b3896917ad38fd8fd8fdecef85a3f0bcd26f730a22a`
and
`c9faabfd53bd125b3f7e4f3f6cbce884e0ce3172de320a1056398de60aa73ab6`.

## R2f synchronous ObjectLiteral concise-method gate

R2f implements synchronous, simple-parameter ObjectLiteral concise methods on
the QuickJS-shaped compiler/VM path. Fixed identifier/keyword/String/numeric
keys and computed String/numeric/Symbol keys, contextual `get`/`set`/`async`
identifiers before `(`, inferred names, source/name/length metadata, C/W/E
property descriptors, dynamic `this`,
owned `arguments`/`new.target`/direct-eval environments, strictness inheritance,
trailing commas, duplicate-parameter early errors, non-constructability,
missing `prototype`, and ordinary `__proto__()` data-property behavior are
pinned against QuickJS 2026-06-04. Accessors, async/generator methods,
non-simple parameters, and home-object/`super` semantics remain typed
independent frontiers.

The focused manifest freezes 74 paths and 144 sloppy/strict variants. All 144
are admitted and pass. Its manifest and key-set SHA-256 values are
`e9f877f938d52a5f5ccbe13af35822b0cb94a9486bb0857156f254a4b532ae75`
and
`ebba13cb8173521639bc12b78f2d5acb498893984f8e42e744a57f6c82f08b9a`;
R2f-landing TSV/JSONL SHA-256 values are
`41a1812b56f74b21967c155f33f93261c767aed6338562535faaded4227e7c4c`
and
`5dbf57993c5c4c1dd47f31769e20bbde16c31bc41d486edd8f1999c19d91e16b`.
`scripts/run-test262-object-methods.sh` reproduces the same 144-pass manifest
against the current profile; its regenerated report hashes are pinned in the
checked-in baseline.

Ten exact parse-negative paths are admitted only after quickjs-oxide and the
pinned oracle both produce the expected phase and error type. The capability
profile therefore keeps 53 reviewed feature tags, moves from 403 to 413 audited
negative paths, and has SHA-256
`1a5258a57285ff43149d8377692b5f1a3939ed19c790cbee81abab6912d21e51`.

The same grammar slice advances existing frozen focused gates without widening
their manifests: Date reaches 1,478 passes (+62), String split 248 (+6), RegExp
match 192 (+2), compile 58 (+2), replacement 326 (+18), matchAll 108 (+26),
named groups 172 (+4), and match indices 48 (+4). Reflect keeps 365 passes while
four parser frontiers advance to runtime assertions; dotAll keeps 26 passes.
These focused manifests overlap, so their movements are not a full-suite pass
delta. The checked-in focused baselines pin each resulting outcome vector
independently.

The exact R2e/R2f full-vector join matches all 102,037 unique keys with no
missing, extra, or duplicate keys and no previous-pass regression. It adds 492
passes: 472
`unsupported-parser -> pass` transitions plus 20
`unsupported-negative-provenance -> pass` transitions from the ten newly
audited negative paths. The remaining exposed parser consumers split into 38
`unsupported-parser -> fail-parse`, 89 `unsupported-parser -> fail-runtime`,
and six `unsupported-parser -> unsupported-runtime` transitions; every other
outcome is unchanged. The join records 625 outcome changes and 631 detail-only
changes. Runnable jobs rise from 36,766 to 36,786 and the complete vector reaches
31,951 passes. Full TSV/JSONL SHA-256 values are
`b63cd00601ea67854cd837a023d1ee14d0b7bdcd02b5e337c0f3eb14f4aa9a67`
and
`4196b714970aae9710d76d07e169c1f96ce80afe65cf37d4677ec2da20e3fe2d`.
The conditional observed rate falls from 91.79% to 91.57% because the new
grammar honestly exposes 127 ordinary parse/runtime failures that were
previously typed parser frontiers; no formerly passing variant regresses.

## R2g synchronous ObjectLiteral accessor gate

R2g ports synchronous ObjectLiteral getters and simple-parameter setters on
the same QuickJS-shaped define-method path. Fixed and computed String, numeric,
keyword, and Symbol keys; one-time `ToPropertyKey`; getter/setter half merging
and replacement; data/accessor conversion; inferred names; descriptors;
dynamic `this`, `arguments`, `new.target`, and direct eval; strictness;
non-constructability; source spans; and ordinary accessor-named `__proto__`
properties are pinned against QuickJS 2026-06-04. QuickJS error priority is
also preserved for accessor arity and strict reserved-word diagnostics.
Non-simple setter parameters, HomeObject/`super`, and async/generator methods
remain independent typed frontiers.

The focused manifest freezes 70 paths and 128 sloppy/strict variants. All 128
are admitted and pass. Its manifest and key-set SHA-256 values are
`02e2810fd012d7f2191cfd2a14d0ae54425c82717c9b8aacd5460e65f9d72175`
and
`2b70d0e1d0054705fe4da193374a67ad664c5f5027d17fb21e1873bb3f8fc1e3`;
the empty non-pass projection has SHA-256
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
Focused TSV/JSONL SHA-256 values are
`fec46a88e750f33f59085a09386a0f05bd563a5c11ed1310bbd19f8de18cb70a`
and
`51f232d679e7045da9634cc0d417cf74815d0f9a1af6064eb1385e6aafa260bd`.
Reproduce it with `scripts/run-test262-object-accessors.sh`.

Nine independently audited parse-negative paths move the capability profile
to 53 reviewed feature tags and 422 exact negative paths, with SHA-256
`73da0ef92820d81935e2f784a2f0e9ce565ccd10c302d8905c4bd4353c3a81ef`.
All 23 existing script-focused gates remain green after regeneration. Nine of
them gain 76 overlapping passes: dotAll +4, compile +2, match indices +4,
RegExp split +4, replacement +24, match +14, matchAll +8, named groups +4,
and search +12. The separately frozen Reflect and Date vectors add four and
eight passes respectively; the latter also exposes two existing missing-JSON
runtime failures. String split and the RegExp-core vector retain their outcome
summaries and change only because the report header embeds the profile hash.

The exact R2f/R2g full-vector join matches all 102,037 unique keys with no
missing, extra, or duplicate keys and no previous-pass regressions. It adds
447 passes across
267 paths: 436 accessor consumers, two strict reserved-word consumers, and
nine newly audited negative variants. The outcome transitions are two
`fail-runtime -> pass`, nine
`unsupported-negative-provenance -> pass`, 414
`unsupported-parser -> pass`, and 22 `unsupported-runtime -> pass`.
Ten former parser frontiers now report ordinary parse failures and 42 reach
ordinary runtime failures at downstream Proxy, JSON, TypedArray, and other
unimplemented surfaces. There are 499 outcome changes and 42 detail-only
changes. Runnable jobs rise from 36,786 to 36,795 and the complete vector
reaches 32,398 passes. Full TSV/JSONL SHA-256 values are
`8510e4117dd3854cd3c428548e36e0bba13a31abd66a875decf5f774850302d3`
and
`71cba68a097d685638b4f77f5e77676ea161e4212410724937ab9804d3c43cb8`.

## R2h direct ObjectLiteral `super` gate

R2h adds QuickJS-shaped HomeObject state and direct SuperProperty Reference
semantics to synchronous ObjectLiteral methods, getters, and setters. The
HomeObject is installed after inferred naming and before property definition;
the super base is the HomeObject's current prototype, while ordinary reads and
writes use the current method receiver. When `super.x()` resolves through an
accessor, pinned QuickJS invokes the getter with the frozen super base and then
calls the returned function with the current method receiver. Fixed and
computed reads,
calls, simple/compound/logical assignments, prefix/postfix updates,
`for-in`/`for-of` assignment targets, strict-versus-sloppy rejected writes,
key-coercion ordering, null-base diagnostics, and `delete super.x` are pinned to
QuickJS 2026-06-04. `super()` remains an early error in ObjectLiteral methods.

The focused manifest freezes 26 paths and 48 sloppy/strict variants. All 48 are
admitted and pass. Its manifest/key-set SHA-256 values are
`75a8d27edff0f6add47f2538a1d44b07509353c1352e759427d4ef93dffd0210`
and
`e25ea45b40345ed6e368d2010f3a48b46364f822845094546a658526b530d41a`;
the non-pass projection is empty. Focused TSV/JSONL SHA-256 values are
`f9d39c6ecbbd768899ad6d9a0962a87271c35a3af8fef16f7a375d82139bb28d`
and
`501107f4cb1dd6f8db6a5e7a43b127a244abce810626fde34c2342e89fe1309e`.
Reproduce it with `scripts/run-test262-object-super.sh`.

Declaring the reviewed `super` feature and one independently audited negative
path moves the capability profile to 54 feature tags and 423 exact negative
paths, with SHA-256
`85cec5c2713df52c631ed38b96621e253baf9e1fafc06eceeea19e9eba64c6f9`.
All existing focused gates remain green after regeneration. The smoke vector
also advances two early-error variants, from 189 to 191 passes, because a
top-level function-body `super` now produces the intended `SyntaxError` rather
than a typed parser frontier.

The exact R2g/R2h full-vector join matches all 102,037 unique keys with no
missing, extra, duplicate, or previous-pass rows. It adds 82 passes: 52
`unsupported-parser -> pass`, 24 `unsupported-feature -> pass`, four
`unsupported-runtime -> pass`, and two
`unsupported-negative-provenance -> pass`. Eighteen other rows expose honest
downstream frontiers or failures, and nine retain their outcome with a more
specific detail, for 100 outcome changes and nine detail-only changes. Runnable
jobs rise from 36,795 to 36,825. The complete vector reaches 32,480 passes;
full TSV/JSONL SHA-256 values are
`44f6f555cc8f72a6d0ff5ed392468a315b44d8c2cd289f7b72a65adde8c58a78`
and
`4d220f27199ee71757e368eb863a535264cc9914a85efaa90d69d54813dd575c`.

R2i below resolves ArrowFunction inheritance and R2j resolves direct-eval
inheritance. Parameter initializers, classes and derived constructors, and
async/generator methods remain explicit follow-up slices rather than being
inferred from the direct ObjectLiteral result.

## R2i ObjectLiteral arrow `super` gate

R2i extends SuperProperty Reference semantics through synchronous arrows nested
in ObjectLiteral methods and accessors. Arrows capture neither a fresh `this`
nor a fresh HomeObject: the enclosing method lazily owns both authenticated
pseudo locals and nested or escaped arrows relay those cells through ordinary
closure slots. An 11-case pinned QuickJS differential covers live HomeObject
prototype changes, lexical receivers, accessor and nested-arrow inheritance,
computed writes, updates, strictness, getter-call receiver behavior, deletion,
and grammar boundaries.

The focused manifest freezes four paths and eight sloppy/strict variants. All
eight are runnable and pass. Its manifest/key-set SHA-256 values are
`d29f77c5920b21a92f61b0022eb186b5ba24e100f6ffa52b4d952347c9aaad90`
and
`4ac13c25ee6b84ee9019b53f5119fb2d7dc3154eb9785eda8800f725bbf32eba`;
the empty non-pass projection has SHA-256
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
Focused TSV/JSONL SHA-256 values are
`afa0f32205ef75af6aae165a3b2e74023d4408cef423333cad63454f9c402872`
and
`0c35ca795fc6b8329bcc6a3af0bbe7878d9819e22bf8b590f2634c79fbba4cbc`.
Reproduce it with `scripts/run-test262-object-super-arrow.sh`. The capability
profile remains unchanged at 54 feature tags and 423 audited negative paths.

The exact R2h/R2i full-vector join matches all 102,037 unique keys with no
missing, extra, duplicate, or detail-only rows and no previous-pass
regressions. Exactly four `unsupported-parser -> pass` transitions occur: the
sloppy/strict variants of
`prop-dot-obj-val-from-arrow.js` and `prop-expr-obj-val-from-arrow.js`.
Runnable jobs remain 36,825 and the complete vector reaches 32,484 passes.
Full TSV/JSONL SHA-256 values are
`dcc079d5c819b066703046136bfe2bdb17a6f02723796c6a8020680db0bb3acb`
and
`c82f264111cd4d0526f2f607ead97aab0e2776b49410b58d25425b8491df2664`.

R2j below resolves direct-eval inheritance of HomeObject. Parameter
initializers, classes and derived constructors, and async/generator methods
remain explicit follow-up slices.

## R2j ObjectLiteral direct-eval `super` gate

R2j extends ObjectLiteral SuperProperty Reference semantics through syntactic
Direct Eval inside synchronous methods, getters, setters, and their synchronous
arrows. Following QuickJS 2026-06-04, the bytecode and eval descriptors persist
independent `super_call_allowed` and `super_allowed` capabilities. ObjectLiteral
methods, getters, and setters carry `false/true`; ordinary functions, scripts,
and indirect eval carry `false/false`; arrows inherit both flags exactly; and
Direct Eval inherits the exact authenticated caller descriptor. HomeObject
pseudo locals and closure cells provide storage, not authority, so merely
finding a captured HomeObject cannot enable `super` across an ordinary-function
boundary.

A 16-case pinned QuickJS differential covers live HomeObject prototype changes,
method/getter/setter receivers, reads, calls, writes, updates, deletion,
strictness, authored and eval-created arrows, nested eval, ordinary/global/
indirect cutoffs, and `super()` argument-order boundaries. An unconditional
Rust expectation test runs the same vector without `QJS_ORACLE`; oracle-enabled
runs independently verify both the pinned expected vector and the Rust/QuickJS
differential. ObjectLiteral descriptors keep `super_call_allowed=false`, so
their `super()` forms remain early errors before argument evaluation. Execution
with an authenticated call capability remains a typed Unsupported boundary for
the future derived-constructor slice.

The focused manifest freezes 12 paths and 24 sloppy/strict variants. All 24 are
runnable and pass. Its manifest and key-set SHA-256 values are
`8643870c3932da98f7ba60cb4e7d4499b02783853f4154f096122796bd998b0f`
and
`6f193e1ebf25a09717fe1c9bbd032d3f1b9cc38eb602870e551f50d5e82277fa`;
the empty non-pass projection has SHA-256
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
Focused TSV/JSONL SHA-256 values are
`5fa67acef400c5525df9eace328219a30539a1661776ebc964e9ac6c4d38a470`
and
`5274231bdedc8c3d99f159626cdeef92fe4cf1fe6a9427d70b6f81f9928fbf0a`.
Reproduce it with `scripts/run-test262-object-super-eval.sh`. The capability
profile remains unchanged at 54 feature tags and 423 audited negative paths.

The exact R2i/R2j full-vector join matches all 102,037 unique keys with no
missing, extra, or duplicate keys, no metadata drift or detail-only rows, and
no previous-pass regressions.
Its only outcome changes are six `fail-runtime -> pass` transitions: the
sloppy/strict variants of `super-prop-method.js`,
`prop-dot-obj-val-from-eval.js`, and `prop-expr-obj-val-from-eval.js`. Runnable
jobs remain 36,825, runtime failures fall from 2,431 to 2,425, and the complete
vector reaches 32,490 passes. Full TSV/JSONL SHA-256 values are
`8a1633a0d527bc77926124f3a6e1fa5ef340e6e79626a22ed171f37dafb8c6e0`
and
`b904278dd9c8cc5d3cf54babd037723ec7e52d015a636fe0d19ef5a4b0f36cfb`.

## R2k tagged-template gate

R2k ports tagged-template parsing and runtime publication from QuickJS
2026-06-04. Each bytecode site owns one frozen cooked Array and one frozen
`raw` Array in its compilation realm; the cooked constant retains that identity
through repeated closure calls and GC. Invalid escapes become `undefined` only
in the cooked Array, raw UTF-16 text is preserved, substitutions remain full
comma expressions, and dot/computed/`with`/`super` tags keep the same Reference
receiver as ordinary calls. Tagged `eval` is deliberately an ordinary call.
Constructor precedence, chained tags, direct-eval HomeObject relay, dynamic
eval/Function site separation, newline continuation, descriptors, abrupt order,
and receiver behavior are pinned by 16 QuickJS differential vectors. A
separate Rust lifecycle test locks site identity across StripDebug publication
and cycle collection.

The focused manifest freezes 48 paths and 89 variants. Of its 85 executed
variants, 83 pass and two stop at the existing PrivateName literal runtime
frontier. Two `create-realm` variants remain host-unsupported and two TCO
variants remain excluded by the pinned QuickJS configuration. Manifest,
key-set, and non-pass SHA-256 values are
`d3a7e597a049e9a78830ee089a90db27c6b6b0b8b2d049cd76b30f5515e6d23a`,
`91852cd5c970debac2ef05af2715198736757b1276a34e6a73722df86bd80356`,
and
`981d8dba14c5cad2481e890d2dfc0925fd5ef03139aca7109d52891166a2c4aa`.
Focused TSV/JSONL SHA-256 values are
`62322ceafcf309aedb8ee6a0b155fef9f24a67356a5408a496647a6f93ed353d`
and
`c91514b3d5b4500ec88d491e19719b139422bd7910876993fbb6a36a9cb70230`.
Reproduce it with `scripts/test-test262-tagged-template.sh`.

Declaring `template` moves the capability profile to 55 feature tags and 423
audited negative paths, with SHA-256
`d146a337c9bab8b171aaddfe31d404073a9d3cbb65fd7ac7d6ab46fdefe69ef7`.
The exact R2j/R2k full join retains all 102,037 unique keys with no missing,
extra, duplicate, or detail-only rows and no previous-pass regressions. It
records 79 `unsupported-parser -> pass`, two `unsupported-runtime -> pass`,
and two `unsupported-feature -> pass` transitions. Two PrivateName staging variants
advance from the parser frontier to the existing typed runtime frontier. The
complete vector reaches 32,573 passes and 36,827 runnable variants. Full
TSV/JSONL SHA-256 values are
`96dfb48f8887e525ff2813e4f8ac9ab7cf191f9e0fedd0d8724ee52943ce60e9`
and
`799be95a11b86d2b1efdfa694cd88971a600c64992fd07b03d61d913377f2e23`.

## R2l strict JSON parse and reviver gate

R2l ports the pinned QuickJS JSON grammar and post-order reviver walk instead
of reusing the JavaScript lexer or an external serializer. Parsing preserves
arbitrary UTF-16 code units, allocates Arrays and ordinary objects in the
method's defining realm, defines `__proto__` as data, and retains exact
primitive source spans only when a callable reviver needs them. The walk
snapshots own keys, keeps QuickJS's duplicate-key parse-record behavior, and
observes mutations through ordinary property operations. It passes the third
reviver context argument with `source` only when the parsed primitive is still
unchanged.

The focused manifest freezes 84 paths and 168 variants. All 168 run: 166 pass,
and the sloppy/strict forms of the 2,097,153-element dense-array stress case
time out at the existing object-model performance frontier. Nothing is skipped
or reported unsupported. Manifest, key-set, and non-pass SHA-256 values are
`16b919d34d9eebcc60a92e038e0a6fd565e9306c1ba17cffc6f62ce0f05f23c4`,
`36e19d071bb8ad9e4982ae85a5f32a3205925b6bf68fe335cfd1cbdfb429cff9`,
and
`2436785b58ef14db6e47d65537af5a9edf58e33bec81837eaf2f3b36f1eee4d0`.
Landing TSV/JSONL hashes under the R2k profile were
`31d01dbc119767d5eb9e2be69c9054f97ca78a3b4ca5e5ae60faf9ed1f29b8e9`
and
`7ed6c23a8b94dfb2854f9be793c4aba388d64a432e0a931d6d8d81dbb7c38dbf`.
Under R2m's profile metadata, the gate hashes were
`22377dfabe093c798ec712be77ab06ca600e11725666945e523b68410d6927cb`
and
`2fa563ffd36405eee7433e0aada0abe1a1474e64b31228949f5a0dc04af2da04`.
Reproduce the current outcome vector and its checked-in
`tests/test262-json-parse-baseline.txt` byte hashes with
`scripts/test-test262-json-parse.sh`.

## R2m JSON stringify and Raw JSON gate

R2m completes the pinned JSON intrinsic family. `JSON.stringify` normalizes
the replacer before `space`, creates the root holder afterward in the defining
realm, invokes `toJSON` then the replacer, unwraps supported primitive wrappers,
snapshots object keys and Array length at the corresponding QuickJS points,
uses a path-only ancestor stack for cycle detection, quotes exact UTF-16
including lone surrogates, rejects unspecialized BigInts, and preserves
QuickJS's indentation and omission/null substitution rules. An explicit task
stack preserves the observable recursive traversal order without imposing the
old 256-level Rust cutoff; differential cases lock 257 and 4,096 nested Arrays.

Its focused manifest deliberately selects the direct stringify semantic
surface, excluding cases whose formatter usage is incidental. All 160 variants
across 80 paths pass. Manifest, key-set, and empty non-pass SHA-256 values are
`001d8337407a2689dc181120160bc6d45d6b03765ec5ca0c2c7f3421f9705f11`,
`ab8b0bdfa3895693115c79579f936d2559806dbc95f2588537267a73d6039892`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
R2m-landing focused TSV/JSONL hashes are
`38ebfa11ff63d080072eb93845711ff4f90bd6753a70fa793edc0c128f89bd82`
and
`1ff4e957792cf2f1702f21df30bd7656d5448a71f5cf9fcc6f37c9cd48fa445b`.
Reproduce the current outcome vector and its checked-in
`tests/test262-json-stringify-baseline.txt` byte hashes with
`scripts/test-test262-json-stringify.sh`.

`JSON.rawJSON` first converts and validates the exact source text through the
same strict parser, then creates a null-prototype, non-extensible object with a
runtime-wide unforgeable heap brand and one frozen enumerable `rawJSON` data
property. `JSON.isRawJSON` tests that brand directly without traps or coercion;
stringify recognizes it after `toJSON`/replacer processing and splices its
validated lexeme before cycle handling. The raw manifest freezes 22 paths and
44 variants. At the R2m landing, 42 were runnable: 36 passed, four parse
failures required unrelated rest/spread syntax, and two typed parser frontiers
required unrelated arrow destructuring. Refreshed through R3d, the current gate
passes all 42 runnable variants. The pinned staging path remains config-excluded
in both modes. R2m-landing manifest, key-set, and non-pass hashes were
`8e4d1fa6f59eae77cf1a35668ea02002de4d4f4cae146bb9ea6bde1c849b1df4`,
`c5be0b3a9dd6c106d9e1c19cd15726b7a6756ac5ee464d4279fd835d520ddee7`,
and
`2c8fb7640ded74e86d6e5b8990dcaf8650ec0eccbc855cb2dcbef808e8caae8a`.
R2m-landing focused TSV/JSONL hashes are
`bb3792c4b565855a533a56db306f9fb465b6f899ca739db3a0ceb92979a0cf34`
and
`4d76fd54f0d4878a816f452170f1b7436fec0c86a0c601d925f86aca1ae16264`.
Reproduce the current outcome vector and its checked-in
`tests/test262-json-raw-baseline.txt` byte hashes with
`scripts/test-test262-json-raw.sh`.

Declaring `json-parse-with-source` and `well-formed-json-stringify` moves the
capability profile to 57 feature tags and 423 audited negative paths, with
SHA-256
`0c6b9ef80d683bd69a97f87bbee10e7029432deb25d23695a96c251e9dfc9f66`.
Because every profile-aware report pins that hash in its header, R2m-era
baselines for older focused gates were re-emitted with metadata-only byte/hash
changes; their outcomes and key sets remained unchanged, while the historical
sections retained each gate's landing hashes.
The exact R2k/R2m full join keeps all 102,037 unique keys with no missing,
extra, duplicate, or previous-pass-regression rows. Of 518 outcome changes,
472 are `fail-runtime -> pass`, 38 are `unsupported-feature -> pass`, two are
`unsupported-feature -> unsupported-parser`, four are
`unsupported-feature -> fail-parse`, and the dense-array pair is
`fail-runtime -> timeout`; nine more rows change detail only. Runnable variants
reach 36,871 and passes reach 33,083, a net gain of 510. Full TSV/JSONL hashes
are
`63d5a44dd8d057e220882d02abebb1b221fdb1a419ce1fc691e1ed084d2b0a3e`
and
`0b8eedcae7d427a6bf7fbbcefb412d9f2691c0bdf00c4bc2229bbfd1a8212fb2`.

## R2n strong Map gate

R2n ports the pinned strong `Map` family through realm-local constructor,
prototype, and iterator graphs. Ordered records use `SameValueZero`, normalize
negative zero, and preserve live mutation behavior for iterators and
`forEach`. Construction follows QuickJS's cached-adder and `IteratorClose`
ordering; the implemented surface includes `set`, `get`, `has`, `delete`,
`clear`, `size`, `forEach`, `keys`, `values`, `entries`, `getOrInsert`,
`getOrInsertComputed`, species, tags, and `Map.groupBy`.

The dependency-audited focused gate freezes 186 paths and 370 variants; all
370 pass. `Symbol.iterator` and `upsert` are admitted only by its runner-bound
scoped profile, whose SHA-256 is
`16ab6bfe18540aae398c847905f492491e81500045b45a6bfb21f447fd537ea2`;
this is not a global feature claim for Set, WeakMap, or other consumers.
Manifest, key-set, and empty non-pass SHA-256 values are
`50387c488c3ade2aafbbe2cd4cecc387bc0c97a76808831d74b634407b990cd1`,
`2704f0c3407fa65dec9297df89f3643eba808f72347b530c71f091be15b14d81`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
Focused TSV/JSONL hashes are
`10e2e4ca4f285eaaf345c1231b7707951e72882e1d603dc144cdde50eb8ed645`
and
`e8645afd72aec2e917fbc11ae4c9502bbb4473897414cc9882027d79082cda69`.
Reproduce the gate with `scripts/test-test262-map.sh`.

Declaring only `Map` and `array-grouping` globally moves the capability profile
to 59 feature tags and 423 audited negative paths, with SHA-256
`0f4617ff1678710c97620aa1257c4868b2a4daf0f4f917f9d7393566ee549c45`.
The exact R2m/R2n full join retains all 102,037 unique keys and records 234
`fail-runtime -> pass`, 80 `unsupported-feature -> pass`, eight
`unsupported-feature -> fail-runtime`, and four
`unsupported-feature -> unsupported-parser` transitions. The eight runtime
failures expose four WeakMap receiver-brand paths in both modes; the four
parser frontiers are two subclass-Map class paths in both modes. They are newly
admitted gaps, not regressions of previously runnable tests. Eighteen further
rows change detail only. There is no previous-pass regression or outcome drift
outside the reviewed admission set: the focused Map manifest plus rows gated
by the newly global `Map` or `array-grouping` tags. Runnable variants reach
36,963 and passes reach 33,397, a net gain of 314. Full TSV/JSONL hashes are
`5a0502380cb281bb089fe229cb1ec806228dd70e75987f852476984cb4d30271`
and
`2370d923625dc76d0a89c8314ed16875a402bccde665b6e45e30948e7526a2f8`.
All global-profile focused reports are re-emitted because the profile header
changed; their key sets remain stable. Older aggregate gates may also change
outcomes or details when the newly installed Map surface removes a downstream
blocker.

Parameter initializers, classes and derived constructors, and async/generator
methods remain explicit follow-up slices.

## R2o strong Set gate

R2o ports the pinned observable strong `Set` family through realm-local
constructor, prototype, and independent Set-iterator graphs. Ordered records
use `SameValueZero`, normalize negative zero, and preserve live mutation for
iterators and `forEach`. Construction follows QuickJS's cached-adder and
`IteratorClose` ordering. The implemented surface includes `add`, `has`,
`delete`, `clear`, `size`, `forEach`, the exact keys/values alias, `entries`,
species, tags, `Set.groupBy`, and `isDisjointFrom`, `isSubsetOf`,
`isSupersetOf`, `intersection`, `difference`, `symmetricDifference`, and
`union`. Set-producing methods follow the pinned set-like protocol and allocate
a base Set in their defining realm without consulting subclass species or an
overridden `add`.

The dependency-audited focused gate freezes 322 paths and 642 variants; all
642 pass. The global profile already admits `Set` and `set-methods`; the
runner-bound scoped profile adds only the exact well-known-Symbol dependencies
required by the frozen manifest. Its SHA-256 is
`6869e9d28fff1d5bd4e5b698dcdf6ee677b9134a91781ad7abe226200d669455`.
Class, generator/object-generator, rest-parameter, lexical-destructuring,
WeakSet, and `$262.createRealm` dependencies remain separate frontiers.
Manifest, key-set, and empty non-pass SHA-256 values are
`44c6b6b599e7fe48324aaa693fa684649469c35209bc5c1edb34f0eebe2085b9`,
`5b4959128a9fb34b72b83950fd329f8a98bbbb2b08f256d5ff8bc3f7bc73a0ac`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
Focused TSV/JSONL hashes are
`b45345b024a33560f2244b69bcdd181e2c5f07add1a04d9fe474169117cb222b`
and
`de7d718b67a1bae7d8031345ce55ba7f32aa8a5d6bcefd745ac2c4401ae65e3f`.
Reproduce the gate with `scripts/test-test262-set.sh`.

Declaring only `Set` and `set-methods` globally moves the capability profile to
61 feature tags and 423 audited negative paths, with SHA-256
`086b4964eebc8dd8960b33aaa333b0adaeefb1447cbf63f893042ab269a5a17b`.
The exact R2n/R2o full join retains all 102,037 unique keys and records 342
`fail-runtime -> pass`, 302 `unsupported-feature -> pass`, 82
`unsupported-feature -> unsupported-parser`, 50
`unsupported-feature -> fail-parse`, and 14
`unsupported-feature -> fail-runtime` transitions. Of the 644 new passes, 602
are inside the focused manifest and 42 are linked Map-brand, for-of, and
staging variants outside it. The focused gate's other 40 variants remain
fail-closed under the global profile because their well-known-Symbol tags are
deliberately scoped. The 14 newly exposed runtime failures are WeakMap/WeakSet
receiver-brand cases; the parser and parse failures expose existing class,
generator/object-method, and parameter-syntax frontiers. There is no
previous-pass regression or outcome drift outside the focused manifest and
rows selected by the newly global tags. Runnable variants reach 37,411 and
passes reach 34,041, a net gain of 644. Full TSV/JSONL SHA-256 values are
`14f8412069dc7ba2a648c2facead1cbcd79ccf2cc5116832602f50decd5f95ab`
and
`c29229ceeee55db836e701d8a2984ef0ba9eb9396d6deca8a5166026b58bb71b`.
All global-profile focused reports are re-emitted because the profile header
changed; their key sets remain stable.

The stable-vector storage shared with Map deliberately retains tombstones and
uses linear lookup. That preserves the tested observable semantics but does
not yet match QuickJS's hash lookup and reclaimable zombie records. WeakMap and
WeakSet additionally require genuine weak-reference/GC infrastructure rather
than another strong-record wrapper. Both remain explicit resource-parity or
feature frontiers rather than part of this milestone.

## R2p well-known Symbol protocol admission

R2p audits the already-implemented realm-local well-known Symbol graph and
admits its eight remaining protocol tags globally: `Symbol.asyncIterator`,
`Symbol.hasInstance`, `Symbol.iterator`, `Symbol.prototype.description`,
`Symbol.species`, `Symbol.toPrimitive`, `Symbol.toStringTag`, and
`Symbol.unscopables`. The focused QuickJS differential suite continues to pin
intrinsic identity, descriptors, descriptions, coercion, iteration, species,
instance checks, tags, and unscopables behavior; this milestone changes the
runner's audited capability boundary rather than production semantics.

The dependency-audited focused gate freezes 517 paths and 1,010 variants under
an exact 30-feature scoped profile. At the R2p landing, all 806 protocol-ready
variants passed. The remaining 204 outcomes were 60 parse failures, 98 runtime
failures, 18 harness failures, and 28 typed parser frontiers caused by
independent class, rest/spread, Promise, buffer/TypedArray, Proxy, and
weak-collection dependencies; the source/result audit found no Symbol protocol
mismatch. R3e brought the gate to 864 passes while refining the remaining
class diagnostics; R3f resolves all 28 derived-class parser frontiers, so the
current gate passes 892 of 1,010 variants. Its other 118 outcomes are the
independent two parse, 98 runtime, and 18 harness failures. The
scoped profile SHA-256 is
`ff674aafc4b1b61b0c40042f831b44c600b1f741e06b8c8c35863b876919aa7b`.
R2p-landing normalized-manifest, manifest-file, key-set, and non-pass SHA-256
values were
`eaf2a48408b6b1f5673389335cda73cb66bed062636a669c655460d9fef99a4b`,
`6147636f7950b899f7c0eea25078e2f4c9c4c7fda2977181dd7c9671aa0bcde2`,
`e87d58ad7a8be3e60b5545129a70a1abd70ee350654092a4aa066d17dc69e450`,
and
`4783b1a8bb909a6e4706138265c477cfa3979bb6821f09f590e4c8c66a0dd5d2`.
R2p-landing focused TSV/JSONL hashes were
`ed0363676e7efdfc6bb24ee396739cf67d49a4ce685c3bd37d98569a60a96267`
and
`75c40ff9adf28f0b9120c23af44268b4660189ff815e3f4c2ba0b74786ede048`.
The current non-pass/TSV/JSONL SHA-256 values are
`831fea4c50b0ffcf14e073a75fa75a4c6855bbadc5c7ed58fbc988c8b33cdf73`,
`310560aa182de2df22b3a261157e92e6f94810a51adda918bea6e9f45fba5209`,
and
`d2fc654e57792e6670d21383e2cbc2c71d7638684ede17db28813dc126e9a409`.
Reproduce the gate with `scripts/test-test262-symbol-protocols.sh`.

The global profile now contains 69 reviewed feature tags and 423 audited
negative paths, with SHA-256
`a1a347d2d74c946a50f1e26fca6c1756c0e9948f087de3aed2339b3a4c7d6677`.
The exact R2o/R2p full join retains all 102,037 unique keys. Its 1,010 outcome
changes exactly equal the focused key set: 806 move from
`unsupported-feature` to pass, 98 to runtime failure, 60 to parse failure, 28
to a typed parser frontier, and 18 to harness failure. Another 1,954 rows change
feature-detail only. Every changed row carries at least one newly admitted tag;
there are no missing/extra keys, previous-pass regressions, or unrelated
outcome changes. Runnable variants reach 38,421 and passes reach 34,847, an
exact net gain of 806. Full TSV/JSONL SHA-256 values are
`a56285e53591df1d2026da4d6334d42e374a107cbcc7744e87f1d8b4c49d865d`
and
`0f1b3899b73d990575b8ee1f4cb11e308847c5fd3fb728b13b3e3e583e08f15e`.

The next high-yield semantic line is binding/destructuring rather than weak
collections: it unlocks several thousand immediately classifiable variants,
while WeakMap and WeakSet first require genuine weak heap edges and collection
semantics.

## R2q flat array binding declarations

R2q implements flat ArrayBindingPattern declarations for `var`, `let`, and
`const` in Program code, ordinary-function bodies, nested blocks, shared
switch scopes, classic `for` heads, and synchronous `for-in`/`for-of` heads.
The shared lowering accepts identifier leaves, empty patterns, elisions,
trailing commas, undefined-only defaults with NamedEvaluation, and a terminal
rest binding. Direct declarations use QuickJS-shaped control-flow inversion:
the binding fragment is emitted first, execution jumps to the right-hand side,
and then returns to the iterator-driven assignment fragment. Iterator records,
abrupt unwind, and `IteratorClose` therefore stay on the same VM path as
synchronous `for-of`. For `var` under `with`, the destination Reference is now
prepared before `IteratorStep`, preserving the binding target even when
observable iterator side effects mutate the object environment.

The dependency-audited gate freezes the clean identifier-leaf projection
across direct declarations, classic `for`, and synchronous `for-of`: 90 paths
and 180 sloppy/strict variants, all runnable and all passing. Its runner-bound
profile admits only `destructuring-binding` and the already-implemented
`Symbol.iterator`; it is deliberately not a global claim for nested or object
patterns, destructuring assignment, parameters, catch bindings, or
async/generator contexts. Normalized-manifest, manifest-file, key-set, and
empty non-pass SHA-256 values are
`257af4e4f08f01ed33c0d88a7c64b44dd29adee6bbc64d87cb0213402e72c048`,
`db17670a1f7715a325a07087b766f6e64cf2bb24cec727278db05db3f79ee679`,
`fdceb7f320989a25165bd37ec41b2b3d2cdd616695979a1a0db92a5415537325`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
The scoped profile SHA-256 is
`8232e2c11e908f7cbf5a9e0f34fbd5223a9551b49ae64647f2a72b2314bcaf84`;
focused TSV/JSONL hashes are
`f0a66030c0a650874b003639775cb87149a4fcd221a1cfd80f603ab8d86f0dde`
and
`ca54eb7e1763501e130fff72dd67ec90469ab8fbc580e12809b6e6cda88e2f35`.
Reproduce the gate with `scripts/test-test262-array-binding-flat.sh`.

The broad `destructuring-binding` tag remains absent from the global profile,
but the full vector is not byte-identical: several Test262 and staging paths do
not carry that metadata tag, so the newly implemented syntax is reached
naturally. The exact R2p/R2q join retains all 102,037 keys and changes 37
outcomes: 23 `unsupported-parser -> pass`, eight `fail-parse -> pass`, two
`unsupported-parser -> fail-parse`, and four
`fail-parse -> unsupported-parser`. The two new parse failures are both modes
of one still-unsupported destructuring-assignment staging path now reaching its
generic syntax frontier; the four typed parser outcomes are nested patterns.
Two further rows keep `fail-parse` but move from the old declaration diagnostic
to that same assignment diagnostic, so 39 data rows change bytes in total.
There are zero previous-pass regressions. Passes rise by 31 to 34,878 while
runnable variants remain 38,421; the full summary now contains 552 parse
failures and 1,204 typed parser frontiers, with every other category unchanged.
Full TSV/JSONL hashes are
`bc9e6f71acbad459fabfcd2838c691cf318a781dea3dc2239161eced7c065c2f`
and
`b0b99d49bec652fa0b686a8d9af4296a5b156db6fec849c56168fb1dc41e6b7e`.
Wider declaration contexts and destructuring consumers must still land behind
their own audited projections before the global capability boundary can move.

## R2r recursive nested array binding declarations

R2r extends the shared declaration lowering from flat identifier leaves to
recursively nested ArrayBindingPatterns. The same path now handles direct
`var`/`let`/`const` declarations, classic `for` declarations, and synchronous
`for-in`/`for-of` declaration heads. Nested defaults, terminal rest patterns,
elisions, and abrupt completion use the existing iterator-region machinery, so
each active iterator receives QuickJS-compatible `IteratorClose` treatment.
The lowering also preserves dynamic `with` References, restores AllowIn for a
whole-pattern initializer in a classic-for NoIn head, and pins QuickJS error
priority for malformed nested and rest patterns.

The dependency-audited R2r gate freezes 72 paths / 144 sloppy/strict variants;
all 144 are runnable and pass. Its runner-bound profile admits only
`destructuring-binding`, so object patterns, destructuring assignment,
parameters, catch bindings, async/generator contexts, and modules remain
outside this claim. The scoped profile SHA-256 is
`c770387473b6ba2e273ab635182b5f07ae80ad902f48057ba5e2fb4f036c723e`.
Normalized-manifest, manifest-file, key-set, and empty non-pass SHA-256 values
are
`84d3c39bb9dcc81f16d92e8b30045a7b5c5d8c2fa6b24151a849633ae087d269`,
`f7c7c181cdde65c84dfcb677cbe45f77884990666a774f952bc165df89f5e8a5`,
`a95c253cbdaf997e9b6d4ed38a48c63e4ffc7400204137c5f4fdd693a815ca7f`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
Focused TSV/JSONL hashes are
`39abfe594755acdeb26375bce7c173544bc9404ad5e96b7c6c4b0dd3f48b1c89`
and
`d4f25a4495c080fd36c237077f323e9686a99b7b9dfdf192c93c18643467f187`.
Reproduce the gate with `scripts/test-test262-array-binding-nested.sh`.

The exact R2q/R2r full join retains all 102,037 unique keys and records only
the sloppy and strict variants of `staging/sm/regress/regress-469625-03.js`
moving from `unsupported-parser` to pass. There are no previous-pass
regressions or other outcome changes. Passes therefore rise by two to 34,880,
runnable variants remain 38,421, and typed parser frontiers fall from 1,204 to
1,202. Full TSV/JSONL SHA-256 values are
`10704652e6a0f24369203c0830bf8e70c7cf3ecd6e158823ee70dc5130d91214`
and
`53590c254bbb591279dc86b4bb8c668dd5f84098fb8eaa0410318e6f42e924d8`.

## R2s fixed/computed recursive object binding declarations

R2s extends the shared declaration lowering to fixed and computed recursive
ObjectBindingPatterns, following QuickJS 2026-06-04. Direct
`var`/`let`/`const`, classic `for`, and synchronous `for-in`/`for-of`
declaration heads accept identifier, String, numeric, keyword, computed String,
and computed Symbol property keys. Defaults use undefined-only selection and
NamedEvaluation, and object and array patterns recurse into each other.
Property-key conversion, sloppy `var` Reference preparation, getters,
initializers, and writes preserve QuickJS's observable `with` ordering.
Abrupt nested patterns retain inner-to-outer iterator unwind and the pending
original-error priority.

The dependency-audited R2s gate freezes 36 generated positive templates across
nine direct, classic-for, and synchronous for-of declaration surfaces: 324
paths / 648 sloppy/strict variants. All 648 are runnable and pass. The global
`destructuring-binding` capability remains closed; the gate's exact
one-feature scoped profile has SHA-256
`aa6cdca241b5f0be7eb202461ba80e44132f917a66480f1c04225cedc410d0d7`.
Normalized-manifest, manifest-file, key-set, and empty non-pass SHA-256 values
are
`f6d9bda32460f3d16bd8084186c05b163e0d44a8788515fe20bf58a0f32d5c2d`,
`ab9974676a1f15442875d6b9de607a27a94a76896a949c8b9cf86b05dbac18dc`,
`bf712cfc7a3c455a2c8188baf82032876ba0321d3bf70d4c4281e00f4b945731`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
Focused TSV/JSONL hashes are
`70d85400fb852c831a1088a8a53e52f8a693eea660f14fc2429983f499858d09`
and
`27218697cb5950df31ae2ef0610ca57d39ee531f4e33ab757a3145c72fafae52`.
Reproduce the gate with `scripts/test-test262-object-binding.sh`.

The exact R2r/R2s full join retains all 102,037 unique keys. Forty-nine
outcomes change across 25 paths, another 71 rows change detail only, and no
previous pass regresses. The transitions are nine `fail-parse -> fail-runtime`,
two `fail-parse -> pass`, two `fail-runtime -> pass`, two
`unsupported-parser -> fail-parse`, two `unsupported-parser -> fail-runtime`,
30 `unsupported-parser -> pass`, and two `unsupported-runtime -> pass`.
Passes rise by 36 to 34,916 while runnable variants remain 38,421. Parse
failures become 543, runtime failures 1,504, typed parser frontiers 1,168, and
typed runtime frontiers 74. Full TSV/JSONL SHA-256 values are
`616026d35b7b86f6b4e6c24d22456db9ca50b64fcc00e787472e75aeebc3e3c2`
and
`a3f633ac23d0fe6d22dcec563ec7f2296f46b2be00738176b543079b7da283e6`.

Object rest remains a typed frontier because it still needs exclusion-aware
`CopyDataProperties`. Its `Unsupported` result is now deferred until the whole
source has completed syntax and declaration scanning, preserving the priority
of later syntax errors and declaration conflicts. Exclusion-aware object rest
is the next binding slice.

## R2t object-rest binding declarations

R2t implements exclusion-aware ObjectBindingPattern rest declarations against
QuickJS 2026-06-04. Direct `var`/`let`/`const`, classic `for`, and synchronous
`for-in`/`for-of` declarations share the recursive object/array lowering. The
new depth-addressed `CopyDataPropertiesExcluded` bytecode preserves its stack
operands. After source `ToObject`, a fresh exclusion object is created before
any computed-key conversion or getter and records fixed and computed
String/Symbol keys before the copy. Computed keys receive exactly one
`ToPropertyKey`; excluded accessors are not read again; ordinary own enumerable
keys are snapshotted in String/Symbol order and then read live into fresh
writable, enumerable, configurable own data properties. Differential tests also
pin primitive boxing, sloppy `with` Reference preparation, nested patterns,
parser skip-scanning, and copy/Put failures under iterator unwind.

The dependency-audited Test262 cohort selects the three available object-rest
semantic templates across direct, classic-for, and synchronous for-of
`var`/`let`/`const` declarations: 27 paths / 54 sloppy/strict variants. All 54
are runnable and pass. Synchronous for-in rest is covered by the focused
QuickJS differential rather than this Test262 cohort. The scoped profile admits
only `destructuring-binding` and `object-rest`; its SHA-256 is
`122a2b055aaf40672a0540441861ecd1e6c09b65e88d45b947bc27a691afc45e`.
Normalized-manifest, manifest-file, key-set, and empty non-pass SHA-256 values
are
`381dc052af426d6d73e498600660d479c843dee1333896958b73176e23b705d7`,
`fc75564488d2ae45a015fa8b07989f3a178f08978221d87ffdeeca0a9359fe57`,
`4b1f4177d308124eb74c0eff3a8028c4bf09b5cf713392467f635e05b03f7e7e`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
Focused TSV/JSONL hashes are
`9a1a364218204b9d6aede93dadd52cb97256b1504a0f016e8d41d46cca3b26be`
and
`53d8920bf0b160e0899a56af3a64fa50be354a899d78a8ec6864be96b3c79694`.
Reproduce the gate with `scripts/test-test262-object-rest-binding.sh`.

The exact R2s/R2t full join retains all 102,037 unique keys and changes only
the sloppy and strict variants of
`test/staging/sm/expressions/destructuring-object-__proto__-1.js` from
`unsupported-parser` to pass. There are no previous-pass regressions,
missing/extra keys, detail-only changes, or other outcome changes. Passes rise
by two to 34,918, runnable variants remain 38,421, and typed parser frontiers
fall from 1,168 to 1,166. Full TSV/JSONL SHA-256 values are
`0c4e7a6e1939aaee3926e8cd2b91e05af0f61a4bfb0cf0c932827e49ea7bb95c`
and
`512e97b82df170c24e262968c6ebf73fa450be92fb1f0db14aaa58d50c17d7f6`.

Destructuring assignment, parameter patterns, and catch patterns remain
separate compiler surfaces. Destructuring assignment is the next high-yield
binding slice.

## R2u array destructuring assignment

R2u implements ArrayAssignmentPattern for direct AssignmentExpression and
synchronous `for-in`/`for-of` assignment heads against QuickJS 2026-06-04.
Direct assignments retain the original RHS as their expression result while a
separate copy feeds the pattern. Identifier, fixed, computed, and `super`
targets prepare their References before `IteratorStep`; defaults, terminal
rest, elisions, empty patterns, recursive arrays, and abrupt completion reuse
the existing iterator-region path. Matching-closer lookahead distinguishes a
real for-head pattern from valid leading literal member targets such as
`for ([].x of values)`. ObjectAssignmentPattern remains typed Unsupported, but
its frontier validates the pattern first so malformed targets retain QuickJS's
SyntaxError and source location.

The dependency-audited Test262 projection selects direct, non-nested,
non-rest `array-*` paths under `expressions/assignment/dstr`: 70 paths / 131
sloppy/strict variants, all runnable and all passing. Its runner-bound profile
admits exactly `Symbol`, `Symbol.iterator`, `const`, `destructuring-binding`,
and `let`, plus exactly three audited parse-negative paths. This is deliberately
not a global `destructuring-binding` admission; Test262 labels much of this
assignment corpus with that broader binding tag. The scoped profile SHA-256 is
`b2133d90974566c72ab788525254de68d260b44756a8c5981111873fb38727af`.
Normalized-manifest, manifest-file, key-set, and empty non-pass SHA-256 values
are
`ee0b310ee20a89e3cff58469a4a7020a4a73980f5086fe189964a2c6c10c120f`,
`046679bd745132066b4982770f13236bfecdbd953b70bdba98afa60424c599c8`,
`093abb8f2b240a97cd1bcf5728cbd720203e91b5ed9df00d22f0394cd86ef4cb`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
Focused TSV/JSONL hashes are
`e3b579aacafa0f63e1e17857b242311ca2512481e86f8ddbe55fcbf28267df51`
and
`832eebb660ad3f50771c60348d203cb5eaef7055098d2a07098f86d04a1b5fc8`.
Reproduce the gate with `scripts/test-test262-array-assignment-flat.sh`.

Nested, rest, synchronous loop, `super`, `with`, IteratorClose, and exact
diagnostic behavior remain covered by the separate 12/12 pinned-QuickJS
differential: 31 semantic sources, 23 exact parser CLI diagnostics, eleven exact
iterator-origin stack traces, plus Rust-only frontier and smoke checks. Object
assignment, generator/async forms, optional
chaining, parameters, and catch patterns remain separate surfaces. Nested
iterator acquisition in a synchronous for-head also retains the existing
for-of control marker instead of QuickJS's RHS value site; behavior matches,
but that debug-frame provenance remains a separate source-map follow-up.

The exact R2t/R2u full join retains all 102,037 unique keys and changes exactly
33 outcomes: 14 `fail-parse -> pass`, one `unsupported-parser -> pass`, 14
`fail-parse -> unsupported-parser`, and four `fail-parse -> fail-runtime`.
There are no previous-pass regressions, missing/extra/duplicate keys, or
detail-only changes. The newly exposed non-pass variants stop at the explicit
object-assignment frontier, missing Proxy, or an already-known staging semantic
frontier. Passes rise by 15 to 34,933 while runnable variants remain 38,421;
parse failures fall to 511, runtime failures become 1,508, and typed parser
frontiers become 1,179. Full TSV/JSONL SHA-256 values are
`17c3c36e73ad8d098ae9d3bd3fc5c5d372187830d5e11f8532bc28471fbb4da3`
and
`e9cb57c7616c27e01e156e7754b9cbc606c40100ea632bcc651c411d10c6c8e9`.

## R2v object destructuring assignment

R2v implements ObjectAssignmentPattern for direct AssignmentExpression and
synchronous `for-in`/`for-of` assignment heads. It shares the direct array
path's control inversion so the original RHS remains the expression result,
then follows QuickJS's object-specific order: `ToObject`, source key
canonicalization, complete target Reference, source Get, undefined-only
default/NamedEvaluation, and NOKEEP Put. Nested patterns read the outer
property before preparing inner References. Object rest prepares its arbitrary
identifier/member/`super` target before exclusion-aware CopyDataProperties;
computed exclusions are canonicalized once and shared by Get and copy. Arrays
and objects recurse through each other without adding a VM opcode.

The pinned-QuickJS gate passes all nine Rust tests: 35 eval differentials, five
exact CLI stack traces, 14 exact parser diagnostics, and a Rust-only smoke.
Nested source-marker inheritance remains a documented non-exact source-map
surface rather than a false stack-parity assertion.

The Test262 projection is split by semantic owner:

- flat: 67 paths / 118 variants, all pass;
- nested object/array recursion: 14 paths / 24 variants, all pass;
- rest: 26 paths / 51 variants, all pass.

The three runner-bound profiles admit only their audited features and 6/4/1
negative paths. Profile SHA-256 values are
`989f5617484d5c12a15fb26a447121fa3436b19f05cd998cf400b5d3d7179a51`,
`18411f3d674a9493806bbf6a601bda903e859395aeec572e466c4a59470ceb12`,
and
`4b9f50b982dc5c3af1466d425a1665448c4a00165d465a74fd4057ef6e414206`.
Normalized-manifest/manifest-file/key-set hashes are respectively
`51eda576685e7a42d734c789f83a3a39efd9614f59e583afb179da4aec8b053a` /
`92089af97dcc157d557061120dfdb68c868f2a8823288290a227a22bfadb285b` /
`f4f62e06502ac316a37ad3b9a55c80a48be6c12fa61b51701b04fbc510994808`,
`925359ce13f9f03e82c2357e5b8ccf1d4024a712445455237fa78f4bba328be6` /
`0e5a594cee6e1c021f310c8e9d88e8b253d789171c97511aec4adcfd346d7d27` /
`ffd426c04c9d96bcae249d576811d2eae1d9a68c455b396769db145212113010`,
and
`014a3e85c43f1ceabdc49379bd502444bc1ca93da163ad25a7ed1ad9f32f899f` /
`931d743e7e2f46d78e66baf7c7c83fcf33208fd8ced6f6c72619ec5948971226` /
`6e574b6e8c3450e0ddb29aaa3d51fe892ad086d718f062858c48f2d115e91595`.
Every non-pass hash is the empty SHA-256. Focused TSV/JSONL hashes are
`f0cd537e2349ce952828c6c61c073636b8631ca27750c7decbc4a8cd634087c6` /
`27456fb05f0015a01c37f2d6c35a0d2b44e49a20578b9e0eabe5c57d53c546d9`,
`430391c59cb61029ecdb1b7f2d81b0ec7054cba76f6bbfdab8b0840baf438669` /
`cad849b67be5b15bbe7fd63b1fa635c5f74f4d2e05c8b65941fe076bb762a37a`,
and
`14d7dba398df75de6aa4583fe126ffc3aca871890121a7f6d53df71d8da4e4de` /
`b6cb010459de59ffaab193fb7ad5fddc9fb73b1f8e437f8041fd2a56ba358964`.
Reproduce them with the three
`scripts/test-test262-object-assignment-{flat,nested,rest}.sh` entry points.

The exact R2u/R2v full join retains all 102,037 keys. All 14 former
ObjectAssignmentPattern `unsupported-parser` variants move to pass; no prior
pass regresses and there are no missing/extra keys or detail-only changes.
Passes reach 34,947 among 38,421 runnable variants and typed parser frontiers
fall from 1,179 to 1,165. Both modes of the unrelated
`staging/sm/Proxy/ownkeys-linear.js` also move from their eventual missing-Proxy
runtime failure to the 30-second timeout while constructing 15,000 properties;
that performance-only movement is kept explicit in the vector. Full TSV/JSONL
SHA-256 values are
`bbc5babdb70a470ff6d937dde2771cb7de270bc6971bfc7597e1f5bf0b24e5da`
and
`2839c0d58d8661b6cec4f6e606d297625343756dbbd656224013c17f992743fe`.

## R2w synchronous catch binding patterns

R2w implements recursive ArrayBindingPattern and ObjectBindingPattern catch
parameters against QuickJS 2026-06-04. Identifier leaves, elisions, defaults
with NamedEvaluation, terminal array rest, fixed and computed object keys,
object rest, and arbitrary array/object recursion reuse the declaration
binding owner. The thrown value is initialized inside the catch lexical scope;
iterator and property abrupt completions therefore reach the surrounding
handler/finally machinery through the same verified paths as other binding
contexts. Pattern leaves are ordinary mutable catch-scope lexicals. Only a
simple catch identifier carries the private catch-parameter marker used by
direct-eval `var` redeclaration rules.

The dependency-audited gate freezes 97 paths / 177 variants, all runnable and
all passing. It covers the synchronous `language/statements/try/dstr` corpus
whose dependencies are implemented, six audited parse-negative rest cases,
the Annex B catch-body early-error integrations, and four untagged catch-scope
paths. Class- and generator-valued defaults remain independent frontiers. The
runner-bound profile admits only `Symbol.iterator`, `destructuring-binding`,
`let`, and `object-rest`; the broad binding/rest tags remain absent from the
global profile.

The scoped profile SHA-256 is
`a654327057a974e0feab6799f3c99a3104884a403cbc41bbc85f3fc226328718`.
Normalized-manifest, manifest-file, key-set, and empty non-pass SHA-256 values
are
`50c326ca60fdfa0cd5d3683df265e730c1947801db6e0892645b9bcfcd450927`,
`e3fb469169b069c185a7d9ea6b8cdce2fdb54d49181b7e87e33cff59a27c212e`,
`1f66a5b898cf1f0cb4a3dc333ee3bb4e7d5dc1361dd5a06b7c1c4be2b0573784`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
Focused TSV/JSONL hashes are
`c1a01134926200028f476ca165ed8127566725bab5faa1a174e77b9f4f460557`
and
`4215e94bb7c8435345542d80ebfcad56ff91567cb4c45582c3cf8426f66dc3da`.
Reproduce the gate with `scripts/test-test262-catch-binding.sh`.

The exact R2v/R2w full join retains all 102,037 keys and adds 49 passes: 24
`unsupported-runtime -> pass`, 23 `unsupported-parser -> pass`, and two
`fail-runtime -> pass`. No previous pass regresses. The two modes of the
unrelated `staging/sm/Proxy/ownkeys-linear.js` move from timeout back to their
eventual missing-Proxy runtime failure; this is recorded performance noise, not
a catch-binding regression. Passes reach 34,996 among 38,421 runnable variants;
typed parser frontiers fall from 1,165 to 1,142, typed runtime frontiers fall
from 74 to 50, and timeouts fall from eight to six. Full TSV/JSONL SHA-256
values are
`e00e85d148fcc5d03ff7830b0e730af0a64b478c498eaad8d018d0bf1c96898a`
and
`ace137cda9b5f55762b2e729a172adbed3715659c981c53bd809f9099fcf20ae`.

## R2x synchronous identifier-rest parameters

R2x implements the identifier-only synchronous rest-parameter slice against
QuickJS 2026-06-04. Ordinary function declarations and expressions,
synchronous object methods, arrows, and the `Function` constructor share the
same formal metadata and entry initialization. Rest collects only actual
trailing arguments into a fresh Array in the callee realm; formal padding does
not leak into that Array. The first rest position also becomes the public
`length`, and a sloppy function with rest receives an unmapped `arguments`
object which snapshots the raw arguments before the rest slot is initialized.

The entry prefix creates `arguments`, initializes rest, and only then installs
body function hoists. This preserves rest under a body `var` declaration while
allowing a body function declaration to replace it at the QuickJS-compatible
point. The bytecode publication boundary authenticates the rest operand,
formal metadata, and prologue shape before the VM may slice the active frame.
The parser also pins duplicate names, non-simple-body `"use strict"`, rest
position, trailing comma, initializer, and getter/setter diagnostics across the
four admitted function forms.

This is not complete rest-parameter or FormalParameters support. Parameter
Environment creation and its direct-eval interactions, default parameters,
parameter destructuring, rest BindingPatterns, and async, generator, and class
forms remain explicit later frontiers.

The runner-bound Test262 gate freezes 34 paths / 65 variants. All 65 are
runnable and pass. Its six-feature profile admits only `Reflect`,
`String.prototype.replaceAll`, `Symbol`, `arrow-function`, `rest-parameters`,
and `set-methods` for this exact manifest, together with 11 audited negative
paths; `rest-parameters` remains absent from the global profile.

The scoped profile SHA-256 is
`da6a76cb6338019f5c233e252bf6d40b7f3eb5c4235a6967cf78f9a74917dced`.
Normalized-manifest, manifest-file, key-set, and empty non-pass SHA-256 values
are
`5cfb4770e35f128a3481a15dcff70dc4733657072fe9cf7a185c91624c355b43`,
`cc326a73c13d2cd90726150e77ad5f5a247074f12a233fe9efa382b3ec6c420e`,
`5a3751688f145e0eda20738258675c1ee27f86fc7808a8a2654dae88d3917c1a`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
Focused TSV/JSONL hashes are
`7b28768f2bb46974d563728cda36e025bc5123f8d3749a32bf83a490e0ac691f`
and
`0a2d3aa3518bc8ab10c5f2bbf768bbd94bc88e809202837416849c63dfa14065`.

Reproduce both gates with:

```sh
./scripts/test-test262-identifier-rest.sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_rest_parameters -- --nocapture
```

The exact R2w/R2x full join retains all 102,037 keys and every previous pass.
It adds 88 passes: 31 `fail-parse -> pass`, nine `unsupported-parser -> pass`,
three `unsupported-runtime -> pass`, and 45 `harness-error -> pass`. There are
61 other outcome changes and ten same-outcome detail changes, with no missing,
extra, or duplicate keys. Passes reach 35,084 among 38,421 runnable variants.
Full TSV/JSONL SHA-256 values are
`1ff253545ba69824b686e23d40998645a57330d83fa01a8bf9a39fa2994e4959`
and
`6a1971269b694b9c5e344884714f9f2234619a3200b6ff2e25a69e2b45e26fb9`.

## R2y synchronous identifier-default parameters

R2y implements `BindingIdentifier = Initializer` for synchronous ordinary
function declarations and expressions, object methods, arrows, and the
`Function` constructor against QuickJS 2026-06-04. The parser establishes the
callee before parsing its formals and creates a parentless Parameter
Environment at the first default. All parameter lexical cells begin in TDZ and
initialize left-to-right; earlier cells, outer bindings, `arguments`, `this`,
`new.target`, `super`, and the private function name retain their applicable
visibility while body declarations do not leak into initializers.

The implementation intentionally preserves a pinned QuickJS behavior which
differs from current Node/spec behavior: initializer closures retain the
lexical parameter cell, while the authored function body reads and writes the
raw argument slot. Thus assigning `a = 2` in the body after an initializer
captured default `a = 1` produces the differential result `2|1`. Default
substitution also updates the raw slot before lexical initialization,
`arguments` is unmapped, `length` stops before the first default,
NamedEvaluation names anonymous functions/arrows, body hoists run after the
Parameter Environment closes, and a default composes with terminal identifier
rest.

The immutable function metadata carries the leading Parameter-local count.
Unlinked publication and final heap allocation share one structural validator
for the exact reverse TDZ reset, left-to-right single initialization,
default-plus-rest ABI, and fixed-order pseudo-binding prologue. The unlinked
boundary additionally authenticates lexical definitions and pseudo-binding
names, and binds each `FClosure` capture source to its bytecode segment:
initializer closures may capture Parameter cells but not raw argument slots,
while body closures use raw argument slots and cannot recover a closed
Parameter cell. Direct eval remains deliberately unsupported in or below a
Parameter Environment: matching the target requires independent `<arg_var>`
and body `<var>` objects plus function-segment topology, so this milestone does
not substitute a one-environment approximation.

The runner-bound scoped gate freezes 76 paths / 143 sloppy/strict variants.
All 143 are runnable and pass. Its profile admits only `default-parameters`
and the required `super` surface, together with 19 audited negative paths;
`default-parameters` remains absent from the repository-wide profile. The
15-case pinned QuickJS oracle separately covers undefined/supplied values,
initializer skipping, all four parser surfaces, later/self TDZ, unmapped
`arguments`, `length`, body hoists and initializer closures, NamedEvaluation,
default-plus-rest, the target-specific raw-argument split, and private named
function bindings across direct/captured reads and strict/sloppy writes.

Profile, normalized-manifest, manifest-file, key-set, and empty non-pass
SHA-256 values are
`5c98d19ccb72c7e2c577ddc98ee4ac83d43a0ba7d49175a8ebe271866d0feab6`,
`8427bc44409269c8edbcef0c1615c7c0c37c6fbbe270c2beb119a9deb3a85bf7`,
`264bb2b25e7502eed86f8a5df1b3fe8c0ccdeecd43171af390764b5e053a6472`,
`26c1a2ac0ab8da8cfa6aca04b724cd4dece1205dfb65b093cd7888343c7c0174`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
Focused TSV/JSONL hashes are
`f1775881f89d5b76f7a46f1a89391a60b213508becec9df244e2fb0d9a937bc7`
and
`dc1edd9121ce27142df0e499a8e4ccdca1e6ff43ca178a35ea40981d45538a23`.

Reproduce the focused gates with:

```sh
./scripts/test-test262-identifier-defaults.sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_identifier_default_parameters -- --nocapture
```

The exact R2x/R2y full join retains all 102,037 unique keys and every previous
pass. It adds 60 passes: 35 `fail-parse -> pass` and 25
`unsupported-parser -> pass`. The 54 other outcome changes are 38
`fail-parse -> unsupported-parser` transitions at the explicit direct-eval,
destructuring, and class boundaries plus 16 `fail-parse -> fail-runtime`
transitions at already-known runtime frontiers. Sixty-four same-outcome rows
now expose a deeper diagnostic; there are no missing, extra, or duplicate
keys. Passes reach 35,144 among 38,421 runnable variants. Full TSV/JSONL
SHA-256 values are
`e02a1e768065e63af6908932dc7ba8e5ff9ec552c3dc6adbce55db91a74eb866`
and
`b762e44abbca482419b5e24ed4479a1726a8c7d25232907538c71780829d4def`.

## R2z synchronous no-default parameter BindingPatterns

R2z implements synchronous FormalParameters BindingPatterns on QuickJS's
`SKIP_HAS_ASSIGNMENT == 0` path. Ordinary function declarations and
expressions, object methods, arrows, the `Function` constructor, and
one-argument setters share recursive array/object/rest lowering. A standalone
`=` anywhere in FormalParameters deliberately stays on the later Parameter
Environment path, including nested defaults and computed-key expressions.

Ordinary patterns reserve anonymous physical argument slots. A terminal rest
pattern reserves no slot and preserves QuickJS's observable `length` behavior,
including the zero-initialized bytecode-record result for an otherwise empty
function. Pattern initialization runs in FunctionRoot before body lexical
entry and before body function hoists. Unmapped `arguments`, direct eval,
computed keys, HomeObject/`super`, iterator closing, and closures created by
the pattern follow the pinned QuickJS ordering and visibility rules.

Both bytecode publication boundaries authenticate anonymous argument reads,
the rest start, the initialization marker, its control-flow boundary, the
arguments prologue, and the absence of direct body-lexical access during the
pattern phase. The complete-tree publisher additionally authenticates child
closure instantiation and rejects a pattern-phase closure which captures a
body lexical cell.

The runner-bound gate derives 37 dependency-clean generated paths from each of
four synchronous surfaces and adds one direct unmapped-arguments consumer: 149
paths / 298 sloppy/strict variants, all runnable and passing. Its scoped
profile admits only `Symbol.iterator`, `destructuring-binding`, and
`object-rest`, together with 12 audited negative paths; these scoped
admissions do not widen the repository-wide profile.

Profile, normalized-manifest, manifest-file, key-set, and empty non-pass
SHA-256 values are
`1f25a0648044b6cb3027e23bc58032b2b2fc3517cd0a29b35d5e4d0844fc6e5e`,
`9cb9662c3c5860e05ba2199be6d3818091e64780ccf7ef61c6d63276a6747f60`,
`9cb9662c3c5860e05ba2199be6d3818091e64780ccf7ef61c6d63276a6747f60`,
`3dbed4631c1c6670bae9256f82773b62ad7a82facda80dac0fb72187fd546e92`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
Focused TSV/JSONL hashes are
`9ef03e119426a2f65dadf3898e63fa48af05469e2f194f1d6c3ab20a3d8cc9db`
and
`0a23a3e1252ddfa2cf0d8fd708b1c0646f13a8d5ccf45098b4ed102c0f3814c1`.

Reproduce both gates with:

```sh
./scripts/test-test262-parameter-binding-patterns.sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_parameter_binding_patterns -- --nocapture
```

The exact R2y/R2z full join retains all 102,037 unique keys and every previous
pass. It adds 22 passes: 12 `fail-parse -> pass`, four
`fail-runtime -> pass`, four `unsupported-parser -> pass`, and two
`unsupported-runtime -> pass`. Nine former parse failures and two former
runtime failures move to the explicit Parameter-Environment frontier; 14
other rows keep their unsupported-parser outcome while exposing a deeper
diagnostic. There are no missing, extra, or duplicate keys. Passes reach
35,166 among 38,421 runnable variants. Full TSV/JSONL SHA-256 values are
`5d85f32719d07937a0e352cc665911c94014ae1f910292100821692c9cbe4546`
and
`2818623121c2991151fdb0c055090283fd5f131e5dcfdd135b97fcdb77df708c`.
BindingPatterns whose FormalParameters contain a standalone `=` are the next
R3a milestone; async, generator, and class forms remain later callable slices.

## R3a synchronous parameter-expression BindingPatterns

R3a completes synchronous BindingPatterns on QuickJS's Parameter Environment
path. A standalone `=` token anywhere in FormalParameters now pre-creates the
parentless argument scope before parsing the first parameter. The bounded
QuickJS-style lookahead retains an assignment already observed if its
256-delimiter safety limit is reached. Every identifier and pattern BoundName
is allocated in that lexical scope in source order, including the meaningful
zero-cell Parameter Environment.

Named parameters still keep their physical argument slot for body reads.
BindingPatterns use anonymous physical sources, then copy their initialized
cells into fresh ordinary body locals only after every parameter initializer
has run. Initializer closures therefore capture the Parameter Environment,
while body closures and body bytecode observe the copied locals. Whole-pattern
defaults, leaf defaults, mixed named/pattern parameters, terminal rest
patterns, function `length`, duplicate-name diagnostics, implicit `arguments`,
getter/setter arity quirks, and QuickJS's accepted but unreachable rest-pattern
initializer are all covered by focused differentials.

An immutable `ParameterEnvironmentLayout` crosses unlinked publication,
complete-tree publication, and heap installation. It records the initialization
boundary, named argument cells, pattern-copy map, raw default sources, future
synthetic-arguments and eval variable-object slots, and authenticates the exact
TDZ, initialization, default-branch, reverse-copy, body-access, closure-capture,
and control-flow topology. R3a deliberately made future direct eval use a typed
extension point instead of silently reusing this ABI. R3b below fills that
`<arg_var>` extension.

The runner-bound R3a gate derives 117 dependency-clean generated paths from
each of four synchronous surfaces: 468 paths / 936 sloppy/strict variants, all
runnable and passing. Its scoped profile admits only `Symbol.iterator`,
`default-parameters`, `destructuring-binding`, and `object-rest`, together with
36 audited negative paths. Profile, normalized-manifest/manifest-file, key-set,
focused TSV, and focused JSONL SHA-256 values are
`0addc7345b6576e1944afc3d5d84cffe16e299e44af09245e78c08cb29207f7b`,
`1db4662456a3ea231c7ce3f629d5224a8cb19d38d13d69c83e43f6407aac21c0`,
`5d4d801025b940f11608d4110169daf6f15427a063e26ca0b1770587a11f464b`,
`e7292d11cc347daf9016b28a987626ee648fc64e4740161ce843058a6fe7265c`,
and
`e6ad140b2e960920c4586455ee9905b4c982ba63e4aa7a9cfc102542c0de8827`.
The QuickJS oracle target contains 20 semantic/early-error vectors and passes
all four Rust integration tests. The pinned Test262 tree contains no exact
BindingPattern + standalone `=` + terminal identifier-rest combination, so
three of those oracle vectors explicitly freeze that cross-feature entry ABI
across ordinary functions, arrows, and object methods.

Reproduce the focused gates with:

```sh
./scripts/test-test262-parameter-expression-binding-patterns.sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_parameter_expression_binding_patterns -- --nocapture
```

The exact R2z/R3a full join retains all 102,037 unique keys and every previous
pass. Twelve `unsupported-parser` variants become passes, while two untagged
staging variants advance from the old typed runtime frontier to already-known
generator/async/class runtime failures. Fifteen same-outcome rows expose deeper
diagnostics. There are no missing, extra, duplicate, or previous-pass-regressed
keys. Passes reach 35,178 among 38,421 runnable variants; full TSV/JSONL hashes
are
`a529e8bc7556be32188fa20dd9a2db121e7feba4cc0dede5d4a1882b4ba363ec`
and
`78839d051f03908350eded05b8ea99c6d9843f4668ec4aa3673b50ca60e710da`.
At the R3a landing, async, generator, and class callables remained later
callable milestones; R3e now covers synchronous base classes.

## R3b direct eval in Parameter Environments

R3b implements sloppy direct eval in and below a synchronous non-simple
Parameter Environment with the two hidden variable objects used by pinned
QuickJS. The body environment owns `<var>` and resolves static body bindings,
then `<var>`, then `<arg_var>`, then outer scopes. Parameter initializers own
`<arg_var>` and resolve static parameter cells, then `<arg_var>`, then outer
scopes. Strict eval still uses a local declaration target.

The cross-layer ABI is explicit: `ClosureVariableKind` distinguishes the
parameter variable object, `EvalScopeKind` distinguishes the Parameter scope,
and `EvalVariableEnvironment` carries the exact scope and source selected for
declarations. Compiler, both publication boundaries, Heap, and VM authenticate
the target role and sentinel rather than guessing from closure order.

The implementation also reproduces QuickJS's synthetic parameter `arguments`
cell. It remains separate from a named `arguments` formal and from the body
binding, is initialized before either variable object, and is available to
BindingPattern expressions and closures. A descendant arrow receives a late
body-arguments closure suffix only for a real authored capture; eval alone does
not synthesize one. Body closures may retain the authenticated `<arg_var>`
object after the outer activation returns.

The QuickJS oracle freezes 42 cases across parameter declaration targets,
body/parameter object separation, deletion and lifetime, `arguments`, entry
ordering, computed/default scope selection, and strict eval. All four oracle
integration tests pass. The dependency-audited Test262 gate contains 71
`noStrict` paths / 71 sloppy variants: 48 arguments/direct-eval cases, 16
scope-open/close cases, 4 redeclaration negatives, 2 computed/default cases,
and 1 staging composite. Oxide and pinned QuickJS `run-test262 -a -m` both run
and pass all 71.

Profile, manifest, key-set, focused TSV, and focused JSONL SHA-256 values are
`98b5e323db1b4be493c1e05b8937a1060b71f7a1cc126087d05e88e7c2a2b335`,
`3df66805796888dd41acbc007b2a958aba5751e9694c0deffa5f0efba19c61a1`,
`08aeb2a3e23a3a3e1bb6e03262d730cd0bbaec1d9aff0f9cc744ebc3ce003938`,
`e2759eb05400218abb31e257fe60bedfcb321e05bbffc0018d9042b60c87ec12`,
and
`a25aaf9087fc356b4b5b3d8437a52cf19166c76ec09aeefc5569f4297a93844d`.

The exact R3a/R3b full join matches all 102,037 unique keys with no duplicates,
missing or extra rows, previous-pass regressions, or same-outcome detail drift.
It records 69 outcome changes: all 66 focused `unsupported-parser` variants
become passes; outside the manifest,
`staging/sm/Function/implicit-this-in-parameter-expression.js` advances to its
known runtime mismatch and the sloppy/strict variants of
`staging/sm/Function/function-name-method.js` advance to the generator-method
typed runtime frontier. Passes reach 35,244 among 38,421 runnable variants.
Full TSV/JSONL SHA-256 values are
`41ef0f16cbae0aa05cdc0bfb13e38130b9b87b1ac958fe6e807541140cda918a`
and
`ecd12b154863534e80f5ac0f40ee6615f1a8743856e9e4f9ca98b44e00a793a0`.

Reproduce the focused gates with:

```sh
./scripts/test-test262-parameter-direct-eval.sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_parameter_direct_eval -- --nocapture
```

At the R3b landing, async, generator, and class callables remained later
callable milestones; R3e now covers synchronous base classes.

## R3c AggregateError and Error cause

R3c publishes `%AggregateError%` on the existing NativeError substrate and
moves the Error intrinsic family into `runtime/intrinsics/error.rs`. The
constructor follows pinned QuickJS order: resolve `newTarget` and allocate the
branded object, convert `message`, perform the completion-aware `cause`
HasProperty/Get sequence, consume `errors`, define the own `errors` Array, and
only then snapshot `stack`. The iterable path caches `next`, allocates the
Array in the constructor's defining realm, closes after abrupt step/done/value
or indexed definition, and preserves the original throw when IteratorClose is
itself abrupt. Primitive `newTarget.prototype` falls back to the AggregateError
prototype belonging to the newTarget realm.

The QuickJS oracle freezes 19 vectors covering the intrinsic graph and
descriptors, call/construct behavior, custom and fallback newTarget prototypes,
message/cause/iterator ordering, genuine Array materialization, normal and
abrupt iterator completion, stack capture, and Error branding. Its expected,
pinned-QuickJS self-check, and Oxide/QuickJS differential tests all pass.

The complete focused feature cohort contains 28 paths / 56 variants. Fifty
pass. Six variants stop at the independent missing-Proxy frontier: the
sloppy/strict modes of
`AggregateError/newtarget-proto-custom.js`,
`AggregateError/newtarget-proto-fallback.js`, and `Error/cause_abrupt.js` use
`Proxy` in their bodies without declaring that dependency in Test262 metadata.
The gate pins those exact `ReferenceError: 'Proxy' is not defined` results so
they cannot masquerade as AggregateError failures or passes. Pinned QuickJS
passes all 28 source paths.

Profile, manifest, path/variant key-set, focused TSV, and focused JSONL SHA-256
values are
`ad9e38f7b1b42445a848ee01437e925fc23f5525276bc45dd15c5ae7a1454d7a`,
`f54979cc3881fd7d361dda7ffbbe75a5bf846e233512c7428711c1091b8474c5`,
`81e86c6e47fcc63ab2063814e34125de57fbc2ed14a8802186db5caa1be6bf5d`,
`40ee7c2976c4319b09457e311ed103bd3851a5a82ae11587794aa3dbc457b537`,
and
`019abe8aedfd1c82ee283aeb976a2364b1e124f91cb401c67407bb17556bd01b`.

The exact R3b/R3c full join matches all 102,037 unique keys with no missing,
extra, duplicate, or previous-pass-regressed rows. It records 62 outcome
changes: 52 `unsupported-feature -> pass`, six `unsupported-feature ->
fail-runtime` at the undeclared Proxy dependency, and four
`unsupported-feature -> unsupported-parser` at the existing class frontier.
The 52 passes include both modes of
`Object/seal/seal-aggregateerror.js`, which correctly consume the new feature
outside the focused intrinsic directory. Passes reach 35,296 among 38,483
runnable variants. Full TSV/JSONL SHA-256 values are
`8579dc70c2b02843b3b0e7680be35d48807bf24f17e3a6b3b2d7daabe6cfb71e`
and
`72296c8615ac07f1de8305445ff7fd9b170eb00b37e616e35679051a90536525`.

Reproduce the focused gates with:

```sh
./scripts/test-test262-aggregate-error.sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_aggregate_error -- --nocapture
```

The six Proxy-dependent variants, cross-realm host fixture, class subclasses,
and Promise consumers remain assigned to their independent milestones.

## R3d argument spread calls

R3d lowers spread calls through typed `Apply(Call)`, `Apply(Construct)`, and
`ApplyEval` bytecode instead of widening the fixed-argument call ABI. Ordinary,
method, constructor, and direct-eval calls share the QuickJS-shaped temporary
dense argument-list path while retaining method receivers and authenticated
eval environments. The VM preserves QuickJS's callable/list/constructor and
eval-identity error order, and keeps the spread source and materialized values
rooted across every observable iterator and call step.

The append path reproduces QuickJS's two observable `@@iterator` Gets. It also
pins the target's fast-Array quirk: when the first Get classifies a genuine
dense Array and the iterator record's cached `next` is the direct built-in
Array iterator-next function, values are copied from the original Array
without advancing or brand-checking that second iterator.

The dependency-audited focused gate freezes 67 paths / 134 variants. It records
122 passes and an exact adjacent-feature frontier of twelve runtime failures.
Fifteen automated Oxide/QuickJS semantic differentials all
pass. Three dense 65K Oxide stress vectors remain ignored for routine
automation and are run manually because immutable shape growth is currently
O(n²); their pinned QuickJS expectations are self-checked, while the shared
65,534/65,535 argument limit is checked quickly by `oracle_function_apply`.

The exact R3c/R3d full join retains all 102,037 unique keys and every prior
pass. It records 122 `fail-parse -> pass`, ten `fail-parse -> fail-runtime`,
and two `fail-runtime -> pass` transitions, plus 13 `fail-parse` detail-only
refinements: 147 complete rows change. Passes reach 35,420 among 38,483
runnable variants; full TSV/JSONL SHA-256 values are
`8fe66b2478571da55c1061a56ca521fbc8f3926591eb6093d3ac537f4cdccf60`
and
`e6ae2522eb1790119f95537d946c90fb529222e9d649710ea8e1c07fd715a89b`.
The refreshed Symbol protocol gate now passes 864 / 1,010 variants, and all 42
runnable Raw JSON variants pass.

Reproduce the focused gates with:

```sh
./scripts/test-test262-aggregate-error.sh
./scripts/test-test262-argument-spread.sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_argument_spread -- --nocapture
```

## R3e base classes

R3e ports the base-only path through QuickJS `js_parse_class`,
`js_op_define_class`, and `OP_define_class`. Class declarations and
expressions now have distinct outer declaration and immutable inner-name
bindings with TDZ behavior. Explicit and synthesized base constructors are
construct-only, preserve parameter/default/rest ordering and constructor
return validation, and publish the exact constructor/prototype descriptor
cycle. Synchronous instance/static methods and accessors support fixed and
computed names, inferred names, strict bodies, non-constructability, and
HomeObject-backed `super` property access.

The pinned QuickJS differential covers constructor errors, descriptors,
computed-key ordering, lexical/direct-eval behavior, HomeObject, source text,
and return handling; all five Rust integration tests pass. The
dependency-audited Test262 gate freezes 157 paths / 294 variants and
passes all 294, while pinned QuickJS passes all 157 paths. Its scoped profile
admits `class` only for that frozen manifest: the global capability profile
deliberately does not claim the whole feature. Profile, manifest, key-set,
TSV, and JSONL SHA-256 values are
`df73a1ac299cce6ade0b0638f0a4c3322310aa2db8e15a28039f483328e69f00`,
`0894fc15cf840a8897ad1b9243324c6312f28fd90e78cdafa377170d15b79f5f`,
`bb0c150613a6e85b4699f612b1c4755f04cd55a60384e8e3ac5b21e543e8de8b`,
`6049119789bd02e1d7848ec661a693c4161b769592b6567e567b21a17122703c`,
and
`7a10a6964629fdb96ed239be78587d9d1ebfdb6fd856549fbe813e5d28352521`.

The exact R3d/R3e full join retains all 102,037 unique keys with no missing,
extra, duplicate, or previous-pass-regressed key. It records 324
`unsupported-parser -> pass` and four `unsupported-runtime -> pass`
transitions. Another 50 outcomes move to deeper honest failures/frontiers, and
719 rows retain their outcome while refining the diagnostic; 1,097 complete
rows change. Passes reach 35,748 among the same 38,483 runnable variants. Full
TSV/JSONL SHA-256 values are
`10e3fee1e93b3491b4c97041990cd17a7f1051dbcd2d0d13c6514961934200ae`
and
`b863a62f5e7dbfcff8975fae28251731b80103f63b3c039d62f1f98271720ada`.

The full run also exposed a named class captured from a BindingPattern default
inside parameter initialization. Its local is now carried as explicit
initializer-scope provenance and authenticated at both publication layers;
forged TDZ lifecycles and body-side access/capture remain rejected. Class
heritage/derived constructors and `super()`, fields/private elements, static
blocks, and generator/async class methods remain typed frontiers.

Reproduce the focused gates with:

```sh
./scripts/test-test262-class-base.sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_class_base -- --nocapture
```

## R3f derived classes and `super()`

R3f was frozen before implementation so the class milestone started from an
authenticated dependency closure rather than a path-name sample. At the R3e
full-vector baseline, 294 paths / 588 variants stopped at the exact typed parser
detail `class heritage and derived constructors are not implemented yet`. The
focused cohort also carries already-passing `super` regressions, class-tagged
paths which the global profile intentionally keeps closed, and exact
parse-negative provenance checks.

The implementation ports the pinned QuickJS heritage and derived-constructor
path: LeftHandSideExpression heritage evaluation, constructor validation before
one observable `prototype` read, `extends null`, constructor/prototype wiring,
raw-argument default forwarding, and explicit fixed/spread `super()` with the
live superclass snapshot taken before argument evaluation. Derived `this`
remains a one-shot TDZ cell; object, undefined, and primitive returns follow the
distinct constructor protocol; `new.target` is preserved through
`Reflect.construct`; and arrows, parameter initializers, and nested direct eval
relay the same authenticated cells.

The bytecode boundary makes that authority explicit. `MarkSuperCall` protects
the authenticated active-function/new-target pair through argument control
flow, only `ConstructSuper`/`ApplySuper` results may initialize derived `this`,
and publication traces all three pseudo bindings through ParentLocal,
ParentClosure, and EvalEnvironment origins. The synthesized default constructor
also has an exact fail-closed shape rather than a name-based privilege.

The first audit draft reported 376 paths. That number omitted the 18 class
declaration paths under
`test/language/statements/class/subclass-builtins/` while retaining their
class-expression mirrors; all 18 depend only on already-present intrinsics and
pass pinned QuickJS, so the omission was corrected to an intermediate 394
paths. A subsequent source-body audit removed three paths whose metadata does
not advertise async support but whose programs contain async methods:

- `test/language/expressions/object/method-definition/early-errors-object-method-formals-contains-super-call.js`;
- `test/language/statements/class/definition/early-errors-class-method-body-contains-super-call.js`;
- `test/language/statements/class/definition/early-errors-class-method-formals-contains-super-call.js`.

Those are async-grammar frontiers, not evidence for synchronous derived-class
support, leaving a provisional 391 paths / 777 variants. A second,
execution-backed source audit then removed five more paths whose metadata does
not declare their adjacent intrinsic dependency: the statement-side
`subclass/builtins.js` directly extends `Uint8Array`, while
`superCallBadNewTargetPrototype.js`, `superCallBaseInvoked.js`,
`superPropDelete.js`, and `destructuring/order-super.js` directly require
`Proxy`. Whole Test262 files cannot be partially admitted, so their otherwise
useful derived-class assertions remain outside this gate until those globals
exist.

The final dependency-audited cohort is therefore 386 paths / 767 variants.
Its R3e global-profile outcomes are 95 pass, 544
`unsupported-parser`, 104 `unsupported-feature`, and 24
`unsupported-negative-provenance`. The focused profile contains exactly the
17 metadata feature tags used by those paths and all 29 parse-negative paths;
it adds `class` only inside this frozen gate. The global profile must continue
to omit whole-feature `class` until fields, private elements, static blocks,
and async/generator class forms are complete.

The 19 immediate heritage-frontier paths intentionally excluded from this
gate require ArrayBuffer/DataView/TypedArray (seven), Promise (two), Proxy
(six), WeakMap/WeakSet (one), private elements (two), or optional chaining
(one). Broader source-linked adjacent populations remain feature-gated: 212
public-field paths / 421 variants, 175 private-element paths / 346 variants,
two static-block paths / four variants, 95 async paths / 139 variants, 40
generator paths / 56 variants, and 15 host-dependent paths / 30 variants.
Those counts overlap where a test combines features and are an adjacency
inventory, not a proposed manifest.

Three otherwise in-scope staging paths are also excluded from the all-pass
oracle gate because pinned QuickJS 2026-06-04 itself records them in
`test262_errors.txt`: `boundFunctionSubclassing.js`, `strictExecution.js`, and
`superPropOrdering.js`. They remain separate target-known-error evidence rather
than being hidden or misreported as derived-class dependencies.

Pinned QuickJS passes all 386 selected paths, and Oxide passes all 767 variants
with no failure, unsupported result, timeout, crash, or infrastructure fault.
The manifest, focused profile, variant-key, TSV, and JSONL SHA-256 values are
`c9c477104d7f538c4b3fa58a108171be866273bedf19825bedf682afc9d00366`,
`1aa167fef279273185060224bd8a65765283d95fe1e08986c5c4ea197657e160`,
`366f33fe39e2980a2a7e6c94e4e20896cd415b8e93b0118f69bc33c39c07e1e5`,
`69467d4d2f8c76ec299e97ce9c88bf74cee35e5cdae42e029377761aa25e4b8a`,
and
`abbe6c64c2fe250f477cf95085c9201a9b9654a2ef01deaa826dff1fea9b1193`.

Two overlapping existing scoreboards independently record the same class
progress: the named-groups gate moves four derived-RegExp variants to pass and
reaches 198/202, while the Symbol-protocol gate moves 28 derived-class and
spread-`super()` variants to pass and reaches 892/1,010. Neither loses a prior
pass.

The exact R3e/R3f full join retains all 102,037 unique keys, has no duplicate,
missing, extra, or previous-pass-regressed row, and records 633 outcome changes:
545 `unsupported-parser -> pass`, 37 `unsupported-parser -> fail-runtime`, two
`unsupported-parser -> fail-parse`, and 49
`unsupported-harness-parser -> harness-error`. Six more rows refine only their
diagnostic. The 544 pass transitions inside the focused manifest are exact; one
excluded pinned-known-error strict variant also passes, while the 88 other
outside-manifest transitions expose missing ArrayBuffer/TypedArray/Promise/
Proxy/WeakMap support, optional chaining, or pinned QuickJS staging
differences. Passes reach 36,293 among the unchanged 38,483 runnable variants.
Full TSV/JSONL SHA-256 values are
`018c55de6e745b35eae7bb8f7d1c3b7680579a58d8bbb241641d860c723a0e34`
and
`995cce2dc58694f8728e1ad12602b2ec5c65169f650cff5047e45d84bc4b407a`.

Reproduce the complete focused vector and the dedicated QuickJS differential
with:

```sh
./scripts/test-test262-class-derived.sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_class_derived -- --nocapture
```

Use `--check` to authenticate only the frozen inputs and pinned QuickJS oracle.
Global `class` remains disabled; host realms, Proxy, fields/private
initialization, static blocks, and async/generator methods remain separate
probes.

## R3g public fields and static blocks

R3g freezes public instance fields, public static fields, and static blocks as
a separate dependency-audited cohort. It contains 386 paths / 767 variants;
the matching R3f path and variant counts are coincidental, not a reused
manifest. Its overlapping inventory includes 305 public-instance-field paths,
51 public-static-field paths, 54 static-block paths, and 119 parse-negative
paths. Ten source-audited adjacent cases are excluded because they require
Proxy or generator/async grammar not declared reliably by their metadata.

Before implementation, all 767 variants failed closed under the global
profile. With the manifest-scoped admission profile, Oxide now passes all 767
variants with no failure, unsupported result, skip, timeout, crash, or runner
fault; pinned QuickJS 2026-06-04 passes all 386 paths. This is a focused
scoreboard, not a full-Test262 percentage or a whole-feature `class` claim.
The global profile remains closed while private elements, async/generator class
forms, host hooks, Proxy, and other adjacent dependencies are incomplete.

The cohort checks computed-key and initialization order, base-versus-derived
constructor timing, public-field descriptor creation, inferred names,
HomeObject-backed `super`, direct eval, static-block scope, and abrupt
completion. A pinned transcript separately compares representative combined
observations byte-for-byte with QuickJS.

Reproduce the focused gate and differential with:

```sh
./scripts/test-test262-class-public-init.sh
cargo build --bin qjs
./scripts/test-r3g-class-public-init-oracle.sh --oxide ./target/debug/qjs
```

Use `--check` on either script to authenticate its frozen inputs without
claiming a new full-vector baseline.

## R3h private data fields

R3h is deliberately field-only: private instance data fields, private static
data fields, their read/write/update References, and `#name in value`. Private
methods, private accessors, and their shared brand operations are not included
in this checkpoint.
The semantic anchors are QuickJS 2026-06-04's own-field operations at
`quickjs.c` 8365-8460, private-`in` operator at 15964-15999, class-field parser
and initialization path at 24314-24330 and 25049-25629, and private-reference
resolution at 33281-33466. The adjacent `JS_AddBrand`/`JS_CheckBrand` path at
8462-8550 was the next methods/accessors milestone at the R3h checkpoint.

The dependency-audited cohort contains 630 paths / 1,260 sloppy-and-strict
variants: 405 positive paths and 225 parse-negative `SyntaxError` paths. Oxide
passes 1,260/1,260 and pinned QuickJS passes 630/630: 100% of this focused
cohort, with zero failure, unsupported, skip, timeout, crash, or runner fault.
This is not a full-Test262 percentage or a claim for all private elements.

The focused profile is hash-authenticated to the exact manifest. Its profile,
manifest path stream, variant-key stream, TSV, and JSONL SHA-256 values are
`c03c22a7ea0d767536c77f1720b5c87766b06759d8a42a6e7b9ec3069633ffa2`,
`8ae21223239ac757bad085913f11f0d86f0b371d66131843932824eb69744f78`,
`dc8a4cd362471eb05abc94b29a5c0ffcb967e5224ab0a75eb50446083015c6ac`,
`755120cd0d3222bf2ec26d43813470dcab31a0ecb6a9f25b904d121df4e35b78`,
and
`f391809104b47e5e05609e625321df4a2759339a080a17d98a34f7be2f181ec4`.
The global profile continues to reject the three private-field tags; ordinary
public observer methods and function-valued data fields do not widen this gate
to private method syntax.

Reproduce the frozen inventory and pinned QuickJS oracle with:

```sh
./scripts/test-test262-class-private-fields.sh --check
```

Run the same command without `--check` for the authenticated Oxide vector.

## R3i ordinary synchronous private methods

R3i adds ordinary synchronous private instance and static methods and their
per-class-side QuickJS brand semantics. It deliberately does not admit private
accessors or async/generator private forms. Each class evaluation creates fresh
instance-side and static-side brands; method callables are shared within that
evaluation, non-constructible, named `#method`, HomeObject-backed for `super`,
and read-only. Hidden own receiver markers remain outside public reflection and
ordinary extensibility, while `#method in value` and wrong-brand diagnostics
follow the same typed brand path. An initialized method with no published brand
also preserves QuickJS's priority: `expecting <brand> private field` is thrown
before a primitive receiver can produce `not an object`. For forward
`#name in object` before a field or method cell initializes, the fixture also
locks QuickJS's internal `[unsupported type]` own-property atom behavior.

The differential fixture also authenticates nested arrows/functions/direct
eval, nested classes, forward names, computed-key and initializer order,
inheritance, non-extensible replacement receivers, reevaluation, exact error
priority, and QuickJS's abrupt computed-key reentry behavior. If the key throws
before class-scope closure, escaped closures keep the captured private VarRef
and the next reentry reuses and resets that cell; normal closure still creates
a fresh identity for the next evaluation.

The dependency-audited manifest at Test262 commit
`5c8206929d81b2d3d727ca6aac56c18358c8d790` contains 267 paths / 534
sloppy-and-strict variants: 219 positive paths and 48 parse-negative paths.
Oxide passes 534/534, pinned QuickJS passes 267/267, and the non-pass report is
empty. This remains a focused manifest-scoped result, not a whole-feature or
full-Test262 percentage.

The profile, manifest file, manifest path stream, TSV, JSONL, and non-pass
SHA-256 values are
`76b0fcc5610e2ceee386469344fd727a8c359abe884befccec1ab435fed93315`,
`af3047bf66c6477f34d4229b03493a2c4247cc3f6f2b5dc4bf26e40c3ed4c7b6`,
`7ea0bbef5d3b5b27aa5e661574fbb0f53cc65fa785874bd1baabb1d83339b375`,
`89dacb36c99d9266e65dd7b0614d93d593007bac3cf0398b1ed0cb1a2258b357`,
`a7a32da2995f30bb21646817d21a2389da92e5b2b17e0c3922179d4e52dd637a`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
The differential JavaScript and expected transcript hashes are
`23053aea3d41c9ee72a61007c713a17d7082dd418c9b06433a03800173b77567`
and
`7e87481d5b8a4202554d7c50264bb8063547512468f8c2df22bf05d06965e452`.

Reproduce the authenticated gate and pinned differential with:

```sh
./scripts/test-test262-class-private-methods.sh
cargo build --bin qjs
./scripts/test-r3i-class-private-methods-oracle.sh --oxide ./target/debug/qjs
```

Use `--check` on either script to authenticate only the frozen inputs.

## R3j synchronous private accessors

R3j implements the synchronous private getter/setter admission target
separately from R3i. Starting from the 651-path
metadata-minimal private-method inventory, the shared source audit removes 79
adjacent paths and leaves 572 minimum synchronous paths. The accessor selector
then partitions those paths exactly into 305 private-accessor paths and the 267
ordinary-private-method paths already admitted by R3i.

All 305 accessor paths have both sloppy and strict variants, producing 610
variants: 229 positive paths and 76 parse-negative `SyntaxError` paths. Oxide
passes 610/610 and pinned QuickJS 2026-06-04 passes 305/305. The non-pass
vector is empty. This remains a focused manifest-scoped result, not a
whole-feature or full-Test262 percentage.

The accessor manifest includes only the same audited class/private-field
dependency tags as R3i. It excludes module/raw/async flags, other feature tags,
and the shared eval, Function-constructor, Proxy, await/yield, async,
generator, static-block, and optional-chaining source frontiers. Within the
pre-filter accessor inventory, 18 paths / 28 variants are excluded: 14 eval
paths and four Function-constructor paths; the other source categories have no
accessor overlap.

The profile, manifest file, manifest path stream, positive stream, and negative
stream SHA-256 values are
`1040d156877d88f6aae651f90b8fae472a8a4054d21f49bbbf2162d280afd884`,
`f8d7b7cb065cf15bae4066ec0790d1c7f0da513b83c8166aef20b3ad7e024cf4`,
`ca77913172666cbe4e74a6476f7f4d87383e801260b2c5b80932dc15e8e98cd6`,
`8ef30d5843d48aaee66a55834c79d710ed8f8d0afa89ea368dee89fef75d897c`,
and
`9d0e56fa4e6fd1ac21a075733fdd327d41f3107500506fbff5987960be1a5901`.
The variant-key stream, TSV, JSONL, and empty non-pass SHA-256 values are
`6c72f931034ee9e2e4b13910c5d88f4d06b527ff49cf6fa6211c751ad28b40a1`,
`aa54c8da45ac9a32aaeb9202ee5aae375a1b42dca0ac59928d78fd11042a02f0`,
`655a02032e50f63b281dce8cc5364d3c6aeff210a1bd3f69adae27c4c053c491`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.

The pinned differential covers getter/setter pairs and one-sided accessors,
including QuickJS's setter-only `#name in value` internal-tag quirk; partial
getter/setter initialization; instance/static brands and diagnostics;
initializer and `super()` ordering; HomeObject `super`; nested function,
arrow, class, and direct-eval capture; fresh class reevaluation; duplicate
brand insertion; abrupt computed-key VarRef reentry; and duplicate-name parser
rules. Its JavaScript and expected transcript SHA-256 values are
`0ee124bbd77f45ae9cd81bc6203cedd03e03b5e78640460abc9670ca77ffca12`
and
`c2656658102e7bfd9ee8da51848e18519afccb9a9ec02cc094d27cb6646d834a`.

Reproduce the authenticated gate and differential with:

```sh
./scripts/test-test262-class-private-accessors.sh
cargo build --bin qjs
./scripts/test-r3j-class-private-accessors-oracle.sh --oxide ./target/debug/qjs
```

Use `--check` on either script to authenticate only the frozen inputs.

## R3k synchronous generators

R3k implements synchronous generator declarations and expressions, public
object/class generator methods, `yield`, and synchronous `yield*`. The first
authenticated Test262 gate deliberately freezes the smallest directly audited
public class-generator inventory before widening feature admission: 82 paths /
160 sloppy/strict variants, comprising 44 positive paths and 38 parse-negative
`SyntaxError` paths. Four `onlyStrict` paths account for the two-variant
difference from a full 164-row expansion.

Oxide passes 160/160 variants and pinned QuickJS 2026-06-04 passes all 82
paths. There are no failures, unsupported results, skips, timeouts, crashes, or
runner faults, and the non-pass vector is empty. The gate includes instance and
static class generator methods, definition/name behavior, parameter/body var
scope, early errors, and two direct `yield*` paths. Runtime Rust tests and the
pinned differential separately cover function and object forms, every
`next`/`return`/`throw` state, catch/finally and reentry, delegation forwarding
and close behavior, first-`next` arguments, closures/direct eval,
`this`/`arguments`/`super`, multiple instances, GC, dynamic GeneratorFunction
calls, prototype selection, descriptors, binding, and non-constructibility.

The manifest, profile, variant-key stream, TSV, JSONL, and empty non-pass
SHA-256 values are
`30857ac44aa29bf86925b72b14da28c9215fb3bc29f81fc6b950694fa0d70b0f`,
`eab79cc5f8ba041e93b7ea04bc391bed8fa249eaf5cbb11857d533fe27028c52`,
`184f80aeb39690da69a802db371fe30cd1678726797181b4a660bf25a9996256`,
`b51f9551d8cc50c1daf1db6f919605c759e13ce57cdfa532b870105eafca89f3`,
`24634a50a1587b22631d09570a562421885137059857beaf9980e961847a340c`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
The checked-in baseline also authenticates the pinned Test262 patch/config/
metadata and the global fail-closed profile.

The global profile intentionally does not admit the `generators` feature tag.
R3k kept private class generator methods separate; R3l measures that slice
below. Async functions/generators, Proxy-dependent paths, and unrelated
unsupported dependencies remain separate frontiers, so this focused gate is
not a whole-Test262 percentage or a full generator-parity claim.

A supplemental fresh-tree dependency audit broadens the selection to 1,203
paths / 2,378 variants. Oxide passes 2,376 with zero engine faults or skips.
The only two non-passes are the sloppy/strict variants of
`test/language/statements/class/static-init-arguments-methods.js`: that file's
unrelated ordinary async method reaches the explicit
`async class methods are not implemented yet` parser frontier. Every
generator-dependent row in the expanded selection passes. Its path stream,
variant-key stream, profile, TSV, and JSONL SHA-256 values are
`8aaa256a04dd6b8b4d0ebfb6c49f70fa21efe0abdff9f8dfc591858539891c80`,
`cdf4ec0a992ec3d034111871945f14f0c488c2d114610d48174565a0d890a360`,
`d3cc7178cf10be7166ec3dcb8d690ce487fa85dd697c74ad0b7cecfa5663f0fa`,
`42d06dde909a48d6f961697c68d32a4809a01778075be79a4a15bde599412d93`,
and
`50108d91e551c71c9659487aaec997324099e13f8c6422e8302b549c588a5378`.
This is breadth evidence rather than a second checked-in acceptance gate.

Reproduce the authenticated gate with:

```sh
./scripts/test-test262-class-generator-methods.sh
```

Use `--check` to authenticate only the frozen manifest/profile and pinned
QuickJS oracle.

## R3l private synchronous class generators

R3l closes the private instance/static class-generator slice without adding a
parallel callable or private-element representation. Private generator methods
retain the existing authenticated `PrivateMethod` cell and class-side brand;
their child bytecode carries the orthogonal generator execution kind. The
unlinked publisher, linked heap verifier, and runtime cell reader accept only
the two legal method shapes—ordinary without an own prototype or generator
with one—while private accessors remain ordinary-only.

The authenticated bootstrap gate starts from a 90-path candidate
universe, excludes eight instance/static expression/statement paths whose
declared `object-spread` dependency is outside this slice, and freezes the
remaining 82 paths / 160 sloppy/strict variants. The manifest contains 16
positive paths and 66 parse-negative `SyntaxError` paths: 36 direct private
generator paths, eight core name/production/valid paths, and 38 early-error
paths. Four `onlyStrict` paths account for the two-variant difference from a
full 164-row expansion.

Oxide passes 160/160 variants and pinned QuickJS 2026-06-04 passes all 82
paths. There are no failures, unsupported results, skips, timeouts, crashes, or
runner faults. The manifest, profile, variant-key stream, TSV, JSONL, and empty
non-pass SHA-256 values are
`b7b2c71cab374f9bcc6754bd9a80506d273d2e135e3f66eb373f325c94d33685`,
`e3732db0b47608265f4f950c1c72929e782eb507597c5f0b336896e51874133e`,
`74f827bf644507c0f0101d6597a8c5560de82b8d2303ef236beef1f3ac9de22d`,
`13e751e511245db9f2ed3db749dbb58cc107161684b62bc7d9806431b79ccf94`,
`c4fcd1c4b3cdfeffaddd633a6f5676286fa99266ba103ec64c86a718b4ec27be`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.

The pinned differential independently locks parameter evaluation before the
first `next`, resumable instance/static bodies, brand-check ordering, callable
identity and extraction, reflection and source text, dynamic `super`, direct
eval/private capture, `yield*`, fresh class evaluations, static subclass
separation, and class evaluation resumed from an outer generator. Its source
and expected-transcript SHA-256 values are
`5af87d8181536da15ba5458ab97698e40d5df953955751bb74656a95a5dd382f`
and
`ff79f3ed6798a77b04e1baec6a6e022a46538f0e463707298cee894487c1a2dc`.
Rust white-box tests additionally lock all three callable-shape defenses and
the GC/realm lifetime of a suspended private generator.

A broader dependency inventory contains 714 primary private-generator paths /
1,420 variants. Oxide passes all 1,388 runnable synchronous variants; the
remaining 32 variants are 16 async-adjacency paths rejected at selection as
`unsupported-async`, with zero engine fault, crash, timeout, or skip. Pinned
QuickJS passes all 714 paths. The groups are 160 frozen-gate passes, 1,072
destructuring passes, 40 arguments-object passes, 12 object-spread passes, and
104 adjacency/name passes plus the 32 async selections. Inventory, variant-key,
normalized-report, and non-pass-stream SHA-256 values are
`84434292de9506822d95c5afef5590d78db2cbb4d0bddeeb3acb9e9e7d1399b1`,
`5fbee112b9ea46b5ba4002b0398e5b7045e97c9d2120a23e524f971a907b0c6c`,
`f48961f1d6223eccabaa2a17726898f8abd76081bf91769a8f9503e4851d3355`,
and
`867ef271b2a97d5de723276b22ce7ec50f36c01f2cddc05aeab19eb515ec6658`.
This is breadth evidence, not a second acceptance gate. The global capability
profile still deliberately keeps `generators` fail-closed, so neither focused
gate changes the whole-suite percentage.

Reproduce both gates with:

```sh
./scripts/test-test262-class-private-generator-methods.sh
cargo build --bin qjs
./scripts/test-r3l-class-private-generators-oracle.sh --oxide ./target/debug/qjs
```

Use `--check` on either script to authenticate only the frozen inputs and
pinned QuickJS oracle.

## R3m Promise constructor and jobs

R3m establishes the first Promise/microtask acceptance boundary without
claiming the rest of the 652-path Promise tree. The frozen candidate universe
is the 58 JavaScript files directly under `test/built-ins/Promise/`; the single
`proto-from-ctor-realm.js` path remains excluded because it requires the
separate `$262.createRealm` host capability. The resulting gate contains 57
paths / 112 sloppy/strict variants: 26 async paths, 31 synchronous paths, and
one each of `noStrict` and `onlyStrict`.

Oxide passes 112/112 variants and pinned QuickJS 2026-06-04 passes all 57
paths. There are no failures, unsupported results, skips, timeouts, crashes, or
runner faults. The manifest, scoped profile, variant-key stream, TSV, JSONL,
and empty non-pass SHA-256 values are
`6cd3564883d5c0e459872b835e19ee7bb8c7f13716824fa2617ca1e698d5ed25`,
`f3a07d4c1c839b4d252ed65f8fb9cadc1862cd31280002caa4656d581007eb71`,
`0290f32ed1fe1968adf0e039748011f30588f4c1ac4b99719c5ce95d1ed9623c`,
`ae6c2454e0aba85f1ce89e1216007c863bcefbf3ce092b2f231549e544b689cf`,
`0d0c92b15448bf8ef94f040ff36c970e1c1d795bfdc99a720e1dff45d1071c18`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.

The scoped profile declares only the metadata features actually present
(`Reflect`, `Reflect.construct`, and `arrow-function`) and opts into the
Test262 async host through `[execution] async=true`. That opt-in loads
`doneprintHandle.js`, captures every string argument passed to `print` behind a
read-only snapshot boundary, drains the FIFO runtime job queue, and requires
exactly one `$DONE` report. The global profile has no execution section, so
async tests remain fail-closed outside this pinned manifest.

Reproduce the gate with:

```sh
./scripts/test-test262-promise-constructor-jobs.sh
```

Use `--check` to authenticate only the frozen manifest/profile and pinned
QuickJS oracle.

## R3n Promise.try, Promise.withResolvers, and Promise.race

At its landing checkpoint, R3n freezes every file directly under the pinned
Test262 `built-ins/Promise/race`, `Promise/try`, and
`Promise/withResolvers` directories: 112 complete paths / 224 sloppy and
strict variants. The inventory contains 94 race paths, 12 try paths, six
withResolvers paths, 66 async paths, and 46 synchronous paths. No negative test
or unrelated Promise directory is admitted.

At R3n landing, Oxide passes 214 variants. The remaining ten are
`fail-runtime`; there are zero
unsupported results and zero skips. Pinned QuickJS 2026-06-04 passes all
112/112 paths. Every failure is the sloppy and strict mode of one of these five
explicit `Promise.all`/`Promise.prototype.finally` adjacency consumers:

- `test/built-ins/Promise/race/resolved-sequence-extra-ticks.js`
- `test/built-ins/Promise/race/resolved-sequence-mixed.js`
- `test/built-ins/Promise/race/resolved-sequence-with-rejections.js`
- `test/built-ins/Promise/race/resolved-sequence.js`
- `test/built-ins/Promise/race/resolved-then-catch-finally.js`

The passing rows cover method descriptors and generic/custom constructors;
`Promise.try` argument forwarding, synchronous callback invocation, return and
throw routing; `withResolvers` result shape and first-call settlement; and
`race` empty-input pending state, FIFO resolution, one-time resolve lookup,
pinned iterator-next no-close behavior, abrupt resolve/then IteratorClose, and
job-graph lifetime across GC. Thus the ten adjacent failures do not widen the
implemented R3n semantic frontier.

Manifest, scoped-profile, variant-key, adjacency-inventory, non-pass, TSV, and
JSONL SHA-256 values are
`be545aefd5f2029faae9745d859a43de176ec9865599a916f15ec465bf84d340`,
`8548d12a4d7f3141583b986c8e3ffcae4e1afb93476ae8a444f64b940bb44654`,
`bfe113d1c47283c84f5fc5f97e30cc74e3fea8d5975a3b87129e5b51eb05d7db`,
`9383382995694ab1f7356f23541c00e5f99910dfd6d80ab6f38662117043e7ae`,
`2fb9eb8c655158ba09dffcad4c9e50f96584cb218ad5e2e5d43a4216b90d3790`,
`faf0b4f680edab60b560e54a62ad0b9ba242c7b85abe92c9714b4152c87324cf`,
and
`fc10101195f430cd4c382c84a4a1a7bd84bb05daff24cd3e7d62351e7dda0968`.
The independent pinned QuickJS static-method fixture/transcript hashes are
`2bc2a52869d42f314614905f4ac750b87064d6e44cbcfdcb20b3703522bdd0b2`
and
`0da636dbcf08f6d6ec112b439a54ec3d6b0816fff34f1381516a5cad3789f16d`.

The scoped profile declares exactly its eight observed feature tags and
`[execution] async=true`. The global profile remains byte-identical at
`1860224ce1e828406f4869b66b3f1964f96fad85e4eab6ba7fecb256b4b6c2f2`;
it has no execution section, so async execution and the two new Promise feature
tags continue to fail closed outside this authenticated manifest.

Reproduce both locks with:

```sh
./scripts/test-r3n-promise-static-oracle.sh --check
./scripts/test-test262-promise-race-try-with-resolvers.sh
```

Use `--check` on the Test262 command to authenticate only its frozen
manifest/profile and pinned QuickJS result.

At the R3o checkpoint, this same frozen R3n inventory passed 216/224 variants.
Its eight `fail-runtime` rows, with zero unsupported results and zero skips,
were the sloppy and strict variants of these four `Promise.all`
consumers:

- `test/built-ins/Promise/race/resolved-sequence-extra-ticks.js`
- `test/built-ins/Promise/race/resolved-sequence-mixed.js`
- `test/built-ins/Promise/race/resolved-sequence-with-rejections.js`
- `test/built-ins/Promise/race/resolved-sequence.js`

Both variants of `resolved-then-catch-finally.js` passed at that checkpoint.
The R3o-checkpoint non-pass, TSV, and JSONL SHA-256 values are
`0865a76b4a9760298b3725c3b1e46559dabeb69e097b07cd9098882f595e64ba`,
`b37787f5024f9132fb4148e6b87a247c05e9439302dd19069c18e44dd1858469`,
and
`21dd45dcc42d79af81e1ff9c979690cbacca86fe1e24e2728edffc104bc300a0`.
The manifest, scoped profile, variant keys, adjacency inventory, and
static-method fixture/transcript remained byte-identical. This was an R3o
cross-milestone result, not a rewrite of the 214/224 authenticated R3n landing
checkpoint and its hashes above. R3p's current result is recorded below.

## R3o Promise.prototype.finally

R3o freezes all 29 files directly under the pinned Test262
`built-ins/Promise/prototype/finally` directory, producing 58 sloppy and strict
variants. The complete cohort contains 12 async paths / 24 variants and 17
synchronous paths / 34 variants. It has no negative tests or unrelated Promise
directories; its sole Proxy path contributes two variants.

Oxide passes 56/58 variants. The only failures are the sloppy and strict modes
of `test/built-ins/Promise/prototype/finally/this-value-proxy.js`, both
classified `fail-runtime` because `Proxy` is not yet defined. There are zero
unsupported results and zero skips. Pinned QuickJS 2026-06-04 passes all 29/29
paths. The scoped profile admits exactly the observed feature tags `Promise`,
`Promise.prototype.finally`, `Reflect.construct`, `Symbol`, `arrow-function`,
and `class`, plus `[execution] async=true`; the global profile remains
fail-closed.

The implementation follows pinned QuickJS `quickjs.c` 54057-54135. It requires
an object receiver, performs `SpeciesConstructor` before testing whether
`onFinally` is callable, and preserves QuickJS's `undefined`
default-constructor sentinel instead of eagerly substituting the intrinsic
Promise. That sentinel makes the later
`PromiseResolve(undefined, cleanupResult)` TypeError observable. A
non-callable argument is forwarded twice to the receiver's dynamic `then`.

Callable cleanup is represented by typed
`PromiseFinallyHandler(Fulfill|Reject)` native functions carrying
`InternalCallableData::PromiseFinallyHandler { constructor, on_finally }`.
Each handler invokes `onFinally` with an `undefined` receiver and no arguments,
routes a thrown cleanup error directly, performs `PromiseResolve` with the
captured constructor and cleanup result, then attaches a typed
`PromiseFinallyThunk(Fulfill|Reject)` through a dynamic `then`. Its
`InternalCallableData::PromiseFinallyThunk { value }` returns the original
fulfillment value or throws the original rejection reason. This locks the
QuickJS sequence of species lookup, callback, resolve, and dynamic `then`, as
well as the rule that failed cleanup overrides the original settlement while
successful cleanup preserves it. Heap validation checks that each native ID
has the matching typed capture, a constructible constructor when present, a
callable cleanup function, and a storable thunk value.

Pinned QuickJS runs Promise resolving class callbacks and its
`JS_NewCFunctionData` capability/finally callbacks in the calling Context,
while ordinary C built-ins switch to their defining realm. Oxide models this
as a typed dispatch policy covering the resolving pair, capability executor,
finally handlers, and finally thunks. A two-Context regression exposes a
finally handler from one Context, calls it from another, and verifies that its
TypeError uses the caller's `TypeError.prototype`. The pinned context-routing
anchors are `quickjs.c` 6025-6044, 17588-17612, 17742-17750,
53352-53357, 53508-53515, and 54070-54121.

GC tracing follows those typed captures: the handler owns constructor and
callback object edges, while the thunk owns raw settlement edges. A Symbol
settlement additionally goes through `retain_raw_value_atoms` during internal
function allocation and through the heap's raw-value atom enumerator;
allocation failure releases the acquired shape. The differential transcript's
`symbol-thunk-thrower-gc=value:true|thrower:true` and `finally-gc=42` rows lock
both value/thrower thunks and the complete finally graph across forced GC.

The manifest and manifest-file hash are both
`9c24a81143fc4d3dcaa8251a2ed98e381f07cb7969f30427a60e9ca931941464`.
The scoped-profile and global-profile SHA-256 values are
`fa10d45a7ddd3924e9124cfc42239e296847223c6c9686beb2a8435e9c83bf04`
and
`1860224ce1e828406f4869b66b3f1964f96fad85e4eab6ba7fecb256b4b6c2f2`.
The variant-key, async, synchronous, Proxy, feature, and include inventory
hashes are
`d468c957b3132cb0dcfb0f9ab2d76237cbefc2b5b86a8ba387c072345be70a9f`,
`72cf44a63ba76996ec5950307c6d79cbac4eeb917389399cdece903bc96f028b`,
`e4a96c0de4f8bda904c8c84868d3f4c51227526290f88cf8ff26961f9a8df6c3`,
`115c53865f31eb747b22e877e8e41154b0e1276618467c595250cf42d730ac8d`,
`38ad367b90ca8661fef8c0ba91e8dd308ddb8aa9afca2301ed6e7e22e9212fed`,
and
`0df478d04b840824e8f175d0e7fbb2e4a29afecce716f6ca7728163d406b0ea2`.
The non-pass, TSV, and JSONL hashes are
`f8155380318e12c8fcf6fef09db3b7628f8934c761279a066a772f6c675a9400`,
`80beabb219bb0a04830f7c2b40e47549234e20b458bd04e27998df7b64cb335d`,
and
`0375fb338a4fe87345f0406c5ce2ff05cb27c2779d2a7260989521cf44444cf8`.
The pinned Test262 patch, config, and metadata hashes are
`f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3`,
`79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b`,
and
`a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a`.
The independent pinned QuickJS differential fixture/transcript hashes are
`720b53338045bd65c70337c3d43678b52e8c7d3e0ce0b0ef1210f512b7d7a53a`
and
`9b30fc689ebac8bb116d18a87460fb9bd987f5c7b40dfabe508f787c249c10fe`.

The Promise facade remains 9,803 lines in `runtime.rs`; the finally algorithm
lives in the dedicated 203-line
`runtime/intrinsics/promise/finally.rs` module. At the R3o checkpoint, the
remaining explicit Promise frontiers were `Promise.all`,
`Promise.allSettled`, and `Promise.any`.

Reproduce both locks with:

```sh
./scripts/test-r3o-promise-finally-oracle.sh --check
./scripts/test-test262-promise-finally.sh
```

Pass `--oxide target/debug/qjs` to the oracle script for the byte-for-byte
QuickJS/Oxide transcript comparison. Use `--check` on the Test262 command to
authenticate only its frozen manifest/profile and pinned QuickJS result.

## R3p Promise.all

R3p freezes all 98 files directly under the pinned Test262
`built-ins/Promise/all` directory, producing 196 sloppy and strict variants.
The complete cohort contains 57 async paths / 114 variants and 41 synchronous
paths / 82 variants. It has no negative, Proxy, or `$262` host tests. Oxide
passes 196/196 with zero failures, unsupported results, or skips; pinned
QuickJS 2026-06-04 passes all 98/98 paths.

The scoped profile admits exactly `Reflect.construct`, `Symbol`,
`Symbol.iterator`, `Symbol.species`, `arrow-function`, and `class`, plus
`[execution] async=true`. The global profile remains byte-identical and has no
execution section. The manifest and manifest-file SHA-256 values are both
`293639a6d0e3f1937535997a4f61613fd40b2b10267d1d27cc5faa231865c1e5`;
the scoped profile and global profile hashes are
`83b69f80efbe0aa1c1273c646595424d4e3cda01f65ccc1e7400495a6779bb21`
and
`1860224ce1e828406f4869b66b3f1964f96fad85e4eab6ba7fecb256b4b6c2f2`.

The variant-key, async, synchronous, feature, and include inventory hashes are
`be2fbe56f4e095c9ebc5ad7a2dc611ec3ca0fcf3878cac552b9b08c3bb0442c7`,
`291bd0ed5b12d2e857bbbfcae3ff967cdb885d1863c10f1a611ac91f68833bf4`,
`160a6566ad05a90da034c1e0be2bafbbc341dd38743dd271592a83443521b81a`,
`ae2f5435de250ebddbc91135bf5847caa09e5150e199aa79061898380c8d180c`,
and
`0df478d04b840824e8f175d0e7fbb2e4a29afecce716f6ca7728163d406b0ea2`.
Both empty Proxy/host inventories and the empty non-pass vector hash to
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
The canonical TSV and JSONL hashes are
`a71f0e04b81bed11d3760296a40753ed18f0572d25145857b5bcee434f6fa2c9`
and
`3c895f2876be7ceabb12e6e85af5f1bc9d9b1eab2f5cb3a884f5f340d871c22a`.

The independent differential locks descriptor/generic behavior, custom
capabilities, fresh element callbacks and shared reject identity, empty and
out-of-order fulfillment, the synchronous-thenable sentinel, first-call
guards, one-time constructor resolve lookup, pinned IteratorClose boundaries,
thenable and identity behavior, forced-GC capture lifetime, and cross-Context
realm routing. Its fixture and pinned transcript hashes are
`e43406b9de7de5a88034ec5321486d7b352f2c6f43986fddba1b36fe79074835`
and
`efb2fd9cfdd1db42291295e0b313dbf271b0007d30f3823e0377cb7196ab6b54`.

R3p also moves the unchanged R3n inventory from its R3o checkpoint of 216/224
to the current 224/224. The empty non-pass, TSV, and JSONL hashes are
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`,
`350e8f80d30a1942e44595c1e771b5e0008fd33aa2f93d6d2345e219d5bb6968`,
and
`4058a876e0f05e0ff0b07d6ae6a5b4886ea9dca3ebbe178c758221aa371df6ca`.
The authenticated 214/224 R3n landing result and 216/224 R3o checkpoint above
remain historical records; their inventory identities did not change.

Reproduce both R3p locks with:

```sh
./scripts/test-r3p-promise-all-oracle.sh --oxide target/debug/qjs
./scripts/test-test262-promise-all.sh
```

Use `--check` on either script to authenticate pinned inputs without comparing
an Oxide oracle transcript or executing the Oxide Test262 cohort.

## R3q Promise.allSettled and Promise.any

R3q freezes both remaining aggregate Promise directories from the pinned
Test262 checkout. `Promise.allSettled` contains 104 paths / 208 sloppy and
strict variants: 57 async paths / 114 variants and 47 synchronous paths / 94
variants. `Promise.any` contains 94 paths / 188 variants: 65 async paths / 130
variants and 29 synchronous paths / 58 variants. Neither cohort contains a
negative, Proxy, or `$262` host test.

Oxide passes the complete cohorts at 208/208 and 188/188, with zero failures,
unsupported results, or skips. Pinned QuickJS 2026-06-04 passes all 104/104
`allSettled` paths and all 94/94 `any` paths. The scoped profiles admit only
the metadata features observed in their respective manifests plus
`[execution] async=true`; the global profile remains byte-identical and
fail-closed for async execution.

For `allSettled`, the manifest, scoped-profile, variant-key, empty non-pass,
TSV, and JSONL hashes are:

```text
5ac6c5f7e21194ee432a6480fc8e8b5ae7fff2c3c859aa61da98f7605261eb52
755439ed09621a0460802bfda11ed27983364d572b13baaf93a2e00c5b481947
9b27ccbbdc3e2d8f3eae0f76b783625cc0aefebc52a2802446e21a6f5dbb083c
e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
69f7dffcd523a759ea7518708d02a74e56349000c86058574c0dc10bc6313b62
d3173fdd5c6d7d2b6b2523c1e9c05b19b3524a6411d383f529c09877a687cc55
```

For `any`, the corresponding hashes are:

```text
331a3d6f0b19a9353904afa5c5d740f844f97c89fcbc99b58cd11275d3b67eaf
8059eea59f179846a4739ddb280b4d16518286261d1cdb361a2d383474f27826
4f2cd9023246ba0631d27846c942f9e227425717208ef0454da1178c105872a5
e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
6b984703c5f155cfd5300314f0f32a98801ad058294aa8b60125f56d478f83a3
856e0679a8425f1a1a403d2577d39547fbeb6053c88dcca4bd9778bf67e6b0f8
```

The combined independent differential fixes QuickJS-specific callback
identity, duplicate-call, property descriptor/order, sentinel, IteratorClose,
forced-GC, and cross-Context realm behavior. Its fixture and pinned transcript
hashes are
`e053bb7944943607b9a29ef15fd34d44a58c44792afaf5193e6b757f4231d8c4`
and
`992d7e26fa681747b67c49a6cfd296c22ae54a558f1d8a86d70ce9eeea3a71e9`.

Reproduce R3q with:

```sh
./scripts/test-r3q-promise-aggregates-oracle.sh --oxide target/debug/qjs
./scripts/test-test262-promise-all-settled.sh
./scripts/test-test262-promise-any.sh
```

Use `--check` on any script to authenticate its frozen inputs without running
the Oxide differential or Test262 cohort.

## R3r generator destructuring return unwind

R3r fixes the only engine faults in the current complete vector. A generator
return injected at a `yield` inside an active array binding or assignment
pattern now follows QuickJS's transient `BlockEnv.has_iterator` path: nested
destructuring iterators close from inner to outer before the return completes,
and an enclosing `finally` runs afterward. The compiler models that region
explicitly; the bytecode verifier remains strict.

QuickJS precompiles a for-of head's assignment fragment before marking the
outer loop iterator in its parser control stack. Its observable behavior is
therefore unusual but pinned: returning from a yield in that fragment closes
an inner destructuring iterator but abandons the outer loop iterator without
calling its `return` method. Oxide reproduces that behavior with the typed
`IteratorDropPreserve` opcode. If the inner close throws, the drop is not
reached and ordinary exception unwinding closes the still-active outer
iterator with the original throw pending, preserving QuickJS's exception
priority.

The independent differential covers binding, assignment, nested patterns,
`finally`, `yield*`, the for-of-head abandon path, and the inner-close throw
path. Its fixture and pinned transcript SHA-256 values are
`05d8e677e984df2a9accb0c56ddb6f2e06ba6d3b2d2d08a51d4ba48811463398`
and
`4e39206df0f8213845227839ad1986759f12566e570a4820265a40e239add715`.

The complete 102,037-key join has no missing or extra key and no previous-pass
regression. Relative to the last checked full baseline it adds 630 passes:
371 `fail-runtime -> pass`, 166 `fail-parse -> pass`, 81
`unsupported-parser -> pass`, and 12 `unsupported-runtime -> pass`; the
remaining changed outcomes are 14 `unsupported-harness-parser ->
fail-runtime`, 11 `fail-parse -> fail-runtime`, and six
`unsupported-parser -> fail-runtime`. Relative to the immediately preceding
R3q implementation rerun, the only changes are both variants of
`staging/sm/expressions/destructuring-array-default-yield.js` moving from
`engine-fault` to `pass`. The complete vector reaches 36,923 passes with no
engine fault; its TSV and JSONL hashes are
`87b1adf3234e6625dd95c96c11357e347447438d412b4007ec2236cb0fd18c7c`
and
`90726c1feee169bf923c857101d73c4f95ffc002de378dfe1f637451ce4fa906`.

R3r also refreshes the flat array-assignment gate's report hashes. All 131
variants still pass; only two successful strict parse-negative detail fields
changed when R3k introduced QuickJS's dedicated `unexpected 'yield' keyword`
diagnostic. A detached R3q checkout produced byte-identical reports to R3r,
proving this bookkeeping refresh is not caused by the unwind fix.

Reproduce R3r with:

```sh
./scripts/test-r3r-generator-destructuring-return-oracle.sh --oxide target/debug/qjs
./scripts/test-test262-array-binding-flat.sh
./scripts/test-test262-array-binding-nested.sh
./scripts/test-test262-array-assignment-flat.sh
./scripts/test-test262-full.sh
```

## R3s complete non-v RegExp built-ins

R3s publishes QuickJS's strict static `RegExp.escape` and completes Annex B
legacy control escapes. The static gate derives all 1,879 pinned
`test/built-ins/RegExp` paths, then excludes a 205-path union: 182
`regexp-v-flag` paths (one also uses `createRealm`), 12 source-audited literal
`/v` paths missing that metadata tag, and 12 `createRealm` paths. The remaining
1,674 paths expand to 3,346 variants; Oxide and pinned QuickJS both pass
3,346/3,346.

The manifest and scoped profile SHA-256 values are
`db6201093f57412de0d0cf16d4ff06f74512af3bc76d6f83c337474c7b982ab3`
and
`0214f6789a3276c4755fadde19477b70620184a6137d29eefef0975cfb379c15`.
The variant-key, TSV, and JSONL hashes are
`98daa9a51c3c4a3067ce293351a4ac9c4cdf0530f67d5bc6ea193c3eb5cbcb26`,
`c2bf334ddcc255048c778095db5bc85e7bacde63ec66049feead47478e66742d`,
and
`9a3ec4c6e5d2c894d22c9e930a74c793dcbf5a691d5e85da34aa024585fac8d0`.
That scoped admission does not widen the global profile.

The complete 102,037-key join has no missing, extra, or duplicate key and no
previous-pass regression. Exactly two `unsupported-parser -> pass` and two
`unsupported-runtime -> pass` transitions raise the total to 36,927. Eight
same-outcome rows proceed past legacy `\c` classification to the existing
ill-formed UTF-16 eval-source frontier. The full TSV/JSONL hashes are
`8f6401e033c8a58d0886ee6453015ca5f289022b90f3f32471e43f7022b2307b`
and
`80055a2278a54aa97f5d0dc8e07bcaefa641cc15ef26ddcc53f35f4095d704e5`.

The independent fixture exercises every `RegExp.escape` classifier category,
strict input behavior, property order and descriptors, lone and paired
surrogates, long ropes, and Annex B `\c` rollback/class behavior. Its fixture
and pinned transcript hashes are
`babb9f0e94a7f4e3cf62ad25faf923dc86adb9248db36f081b4b2e7667c6f784`
and
`c6226637ca00cfcef2c436cb64442d8264ba18553aba31baffe70a34d48f480f`.

Reproduce R3s with:

```sh
./scripts/test-r3s-regexp-escape-control-oracle.sh --oxide target/debug/qjs
./scripts/test-test262-regexp-builtins.sh
./scripts/test-test262-regexp-core.sh
./scripts/test-test262-full.sh
```

## R3t synchronous generators + destructuring binding

R3t derives its boundary from pinned metadata rather than current Oxide
outcomes. A path enters the raw universe when it carries `generators` or
`destructuring-binding` and all of its remaining feature tags belong to the
exact 11-tag scoped profile. That yields 3,418 paths and 6,624 variants.
Removing 25 module paths/variants leaves 3,393 paths and 6,599 variants. Three
source-audited paths contain real async syntax despite omitting the `async`
metadata flag; excluding their six variants freezes the synchronous gate at
3,390 paths and 6,593 variants.

The final inventory contains 3,011 positive paths/5,906 variants and 379
parse-negative paths/687 variants. Its mode split is 3,313 sloppy and 3,280
strict variants. Oxide and pinned QuickJS both pass all 6,593 variants, with an
empty non-pass vector.

The semantic fixes follow the pinned QuickJS implementation:

- mapped `arguments` shares frame VarRefs only through
  `min(actual_count, formal_count)`; extra actual arguments receive detached
  VarRefs (`quickjs.c:16228-16275`);
- generator `.caller` and `.arguments` keep the poison accessors used by
  non-ordinary functions (`quickjs.c:16110-16117`, `17388-17434`, and
  `36513-36516`);
- contextual `yield` is accepted only as the name of a sloppy ordinary
  FunctionExpression (`quickjs.c:36430-36444`);
- scoped generator declarations use the lexical/Annex B duplicate-declaration
  distinction from `quickjs.c:24186-24223` and `36487-36493`; active `yield`
  and the associated `for-in` negatives now produce genuine `SyntaxError`
  results instead of generic unsupported-parser classifications.

The scoped profile, manifest, variant-key, TSV, and JSONL SHA-256 values are
`8057ef347c07ffc80a66c5c83ff73873148a8813af49bcca1ced9863cfb9ac9e`,
`07ad2748c65763366ebdcb8c01893a13aa4fbbcca3e900a31042fc670593f3c5`,
`f5e729f4b439733ee900ce1d7d98163b9969aab6998b4a288cb4a6eea5c35f81`,
`f81c2f7b946360f44c1b2d5bdc40782d2e13f989af372329fb6582cb8ded8978`,
and
`eb1d82ad4d156880bc539d2bfc73e8203cd9dd8f70289e80560388ea07c11083`.
The complete derivation and remaining inventory hashes are frozen in
`tests/test262-generator-destructuring-baseline.txt`.

This is a checksum-bound scoped admission, not a global profile migration.
The global profile remains byte-identical and fail-closed for `generators` and
`destructuring-binding`, so the 6,593 scoped passes are not claimed as a global
uplift. One untagged Annex B generator-declaration test does move from
`fail-runtime` to `pass`. A second untagged staging test moves from
`fail-parse` to the deeper `fail-runtime` expected from the pinned QuickJS
behavior: QuickJS itself rejects that old SpiderMonkey assertion after
accepting contextual `TOK_YIELD`. The exact 102,037-key join therefore has one
new pass, no previous-pass regression, 97 parse failures, and 1,284 runtime
failures. Its score is 36,928/102,037 and its TSV/JSONL hashes are
`6b2fb9219bad5f25bfcebc297ce9373798cd210140ebab0566a18e8dd83d052b`
and
`d2cf352f98f7d12b1ff734d7ff001c443c896be3c8adddd54951dd0a47f78eb2`.
A later, separate admission milestone can classify the three async
adjacencies and refresh every globally profile-bound baseline without mixing
that bookkeeping into the semantic implementation commit.

Reproduce R3t with:

```sh
./scripts/test-test262-generator-destructuring.sh
./scripts/test-test262-full.sh
```

Use `./scripts/test-test262-generator-destructuring.sh --check` to authenticate
the static inventory, scoped profile, and QuickJS oracle without running Oxide.

## Runner contract

`run-test262` provides a conservative, process-isolated progress measurement:

- fresh Rust process, `Runtime`, and `Context` for every runnable variant;
- hard parent-process timeout and crash classification;
- canonical Test262 `raw` behavior (no harness or strict prefix);
- separate harness compilation/evaluation, then test compile and execute;
- exact parse-versus-runtime negative phase and constructor-name checks;
- explicitly typed implementation-frontier errors kept distinct from
  JavaScript `SyntaxError`;
- parse-negative tests execute only after compilation succeeds, so
  `$DONOTEVALUATE` cannot turn a missing parse error into a pass;
- unsupported features and unaudited negative tests fail closed through the
  checksum-pinned quickjs-oxide capability profile;
- metadata and source requirements classify module, async, `CanBlockIsFalse`,
  and the `$262` host hooks used by the pinned suite before execution; async
  execution requires an authenticated scoped-profile opt-in;
- bounded parallel workers with deterministic result ordering and full child
  cleanup after errors;
- deterministic TSV outcome vector plus a JSONL sidecar;
- module variants and non-opted-in async variants reported as unsupported and
  treated as failures unless a caller is explicitly recording a baseline.

The host scan is deliberately conservative and the pinned inventory has no
unknown `$262` hook. Native `$262` objects and an out-of-band host sentinel are
still required before those host-dependent tests can be admitted for execution.

This deliberately fixes three known limitations in the pinned QuickJS
`run-test262.c`: it does not discard negative phase, does not load harness code
for `raw`, and does not let a stable known-error ledger hide the raw failure
count. A future QuickJS-runner compatibility profile may reproduce those quirks
for outcome-vector differential work, but it must remain separate from the
canonical progress report.

## Reproduce

```sh
./scripts/test-test262-smoke.sh
./scripts/test-test262-provenance.sh
./scripts/test-test262-reflect.sh
./scripts/test-test262-date.sh
./scripts/test-test262-string-split.sh
./scripts/test-test262-regexp-core.sh
./scripts/test-test262-regexp-builtins.sh
./scripts/test-r3s-regexp-escape-control-oracle.sh --oxide target/debug/qjs
./scripts/run-test262-regexp-literals.sh
./scripts/run-test262-regexp-search.sh
./scripts/run-test262-regexp-match.sh
./scripts/run-test262-regexp-split.sh
./scripts/run-test262-regexp-compile.sh
./scripts/run-test262-regexp-modifiers.sh
./scripts/run-test262-replace.sh
./scripts/run-test262-regexp-match-all.sh
./scripts/run-test262-regexp-backreferences.sh
./scripts/run-test262-regexp-lookahead.sh
./scripts/run-test262-regexp-lookbehind.sh
./scripts/run-test262-regexp-unicode-properties.sh
./scripts/run-test262-regexp-named-groups.sh
./scripts/run-test262-regexp-duplicate-named-groups.sh
./scripts/run-test262-regexp-match-indices.sh
./scripts/run-test262-regexp-dotall.sh
./scripts/run-test262-unicode-u180e.sh
./scripts/run-test262-eval-intrinsic.sh
./scripts/run-test262-eval-declarations.sh
./scripts/run-test262-nested-direct-eval.sh
./scripts/run-test262-with.sh
./scripts/run-test262-arrow.sh
./scripts/run-test262-object-methods.sh
./scripts/run-test262-object-accessors.sh
./scripts/run-test262-object-super.sh
./scripts/run-test262-object-super-arrow.sh
./scripts/run-test262-object-super-eval.sh
./scripts/test-test262-tagged-template.sh
./scripts/test-test262-json-parse.sh
./scripts/test-test262-json-stringify.sh
./scripts/test-test262-json-raw.sh
./scripts/test-test262-map.sh
./scripts/test-test262-set.sh
./scripts/test-test262-symbol-protocols.sh
./scripts/test-test262-array-binding-flat.sh
./scripts/test-test262-array-binding-nested.sh
./scripts/test-test262-array-assignment-flat.sh
./scripts/test-test262-object-assignment-flat.sh
./scripts/test-test262-object-assignment-nested.sh
./scripts/test-test262-object-assignment-rest.sh
./scripts/test-test262-object-binding.sh
./scripts/test-test262-object-rest-binding.sh
./scripts/test-test262-catch-binding.sh
./scripts/test-test262-identifier-rest.sh
./scripts/test-test262-identifier-defaults.sh
./scripts/test-test262-parameter-binding-patterns.sh
./scripts/test-test262-parameter-expression-binding-patterns.sh
./scripts/test-test262-parameter-direct-eval.sh
./scripts/test-test262-aggregate-error.sh
./scripts/test-test262-argument-spread.sh
./scripts/test-test262-class-base.sh
./scripts/test-test262-class-derived.sh
./scripts/test-test262-class-public-init.sh
./scripts/test-test262-class-private-fields.sh
./scripts/test-test262-class-private-methods.sh
./scripts/test-test262-class-private-accessors.sh
./scripts/test-test262-class-generator-methods.sh
./scripts/test-test262-class-private-generator-methods.sh
./scripts/test-test262-generator-destructuring.sh
./scripts/test-r3r-generator-destructuring-return-oracle.sh --oxide target/debug/qjs
./scripts/test-test262-full.sh
```

The smoke command also exhaustively validates pinned metadata against its
independent fingerprint. The provenance command guards known false-positive
boundaries. The full command uses the release runner, defaults to two workers,
and compares the complete outcome vector and sidecar by SHA-256. Set
`TEST262_WORKERS` to change concurrency without changing the expected bytes.

Math, Reflect, Date, and generic `String.prototype.split` are no longer common
blockers in their reviewed sets.
The Date transition also resolves the four otherwise-ready Reflect variants
which had stopped at `Date.now`; generic split resolves six more linked Reflect
variants. Basic RegExp literal execution, the search/match/split protocols,
legacy compile, scoped modifiers, generic replacement, matchAll, and numeric
backreferences, forward lookahead, lookbehind, Unicode property escapes,
ordinary named captures, duplicate named captures, match indices, and dotAll
and U+180E are now measured separately in
R1b/R1c/R1d/R1e/R1f/R1g/R1h/R1i/R1j/R1k/R1l/R1m/R1n/R1o/R1p/R1q/R1r/R1s/R1t;
R1u separately measures the eval intrinsic shell and its typed String-source
frontier; R1v establishes its syntactic opcode and realm-identity path with a
byte-identical scoreboard; R1w adds the immutable caller-environment table and
live-cell materialization with the same zero-movement result; R1x opens the
bounded independent String-eval root and adds 575 full-vector passes; R1y adds
QuickJS-shaped eval declaration environments and another 768 passes; R1z adds
recursive direct-eval caller-environment relay and another 29 passes; R2a fixes
private FunctionName/eval declaration precedence with a byte-identical full
vector; R2b adds the `with` environment and 198 passes; R2c adds synchronous
simple-parameter ArrowFunctions, declares their shared feature tag, and adds
1,043 passes with zero previous-pass regressions; R2e audits the capability
profile to 53 feature tags and 403 negative paths without changing engine
semantics; R2f adds synchronous simple-parameter ObjectLiteral concise methods,
moves the profile to 413 audited negative paths, and passes its 144-variant
focused gate while adding 492 full-vector passes with zero previous-pass
regressions; R2g adds synchronous simple-parameter ObjectLiteral accessors,
moves the profile to 422 audited negative paths, passes its 128-variant focused
gate, and adds 447 full-vector passes with zero previous-pass regressions; R2h
adds direct ObjectLiteral SuperProperty References, moves the profile to 54
feature tags and 423 audited negative paths, passes its 48-variant focused gate,
and adds 82 full-vector passes with zero previous-pass regressions; R2i relays
ObjectLiteral HomeObject and lexical `this` through synchronous arrows, passes
its eight-variant focused gate, and adds four full-vector passes without
changing the profile or runnable count; R2j authenticates the independent
SuperCall and SuperProperty capabilities through ObjectLiteral direct eval,
passes its 24-variant focused gate, and adds six full-vector passes with no
previous-pass regression or runnable-count change; R2k adds QuickJS-shaped
tagged-template site objects and calls, declares `template`, passes all 83
runnable non-frontier variants in its focused gate, and adds 83 full-vector
passes with zero previous-pass regressions; R2l adds the strict JSON parser,
reviver walk, and exact source contexts, passing 166/168 focused variants with
only the dense-array timeout pair; R2m adds stringify and branded Raw JSON,
passes 160/160 direct stringify variants and 36/42 runnable Raw JSON variants,
declares the two reviewed JSON feature tags, and brings the complete vector to
33,083 passes with zero previous-pass regressions; R2n adds the complete strong
Map surface, passes its 370/370 focused gate, declares only `Map` and
`array-grouping` globally, and adds 314 full-vector passes with zero
previous-pass regression, bringing the complete vector to 33,397 passes.
R2o adds the observable strong Set family and all seven set-composition
methods, passes its 642/642 focused gate, declares only `Set` and
`set-methods`, and adds 644 full-vector passes with zero previous-pass
regression, bringing the complete vector to 34,041 passes.
R2p audits and globally admits the eight remaining well-known Symbol protocol
tags, passes all 806 protocol-ready variants in its frozen 1,010-variant gate,
and adds exactly 806 full-vector passes with zero previous-pass regression,
bringing the complete vector to 34,847 passes.
R2q implements flat array binding declarations, passes all 180 variants in its
90-path scoped gate, and deliberately keeps `destructuring-binding` scoped.
Untagged binding variants nevertheless add 31 full-vector passes with zero
previous-pass regressions, bringing the complete vector to 34,878 passes.
R2r adds recursive nested array declaration patterns across direct
declarations, classic `for`, and synchronous `for-in`/`for-of`, passing all 144
variants in its 72-path scoped gate. The two variants of
`staging/sm/regress/regress-469625-03.js` move to pass with no other
full-vector outcome change, bringing the complete vector to 34,880 passes.
R2s adds fixed and computed recursive object declaration patterns on the same
surfaces, including object/array recursion, observable `with` Reference timing,
and iterator unwind. All 648 variants in its 324-path scoped gate pass. The
full vector gains 36 passes with zero previous-pass regression, reaching
34,916 passes among 38,421 runnable variants; exclusion-aware object rest is
the next binding slice.
R2t adds exclusion-aware object-rest declarations on those direct, loop, and
recursive binding surfaces. All 54 variants in its 27-path scoped gate pass;
the full vector changes only the two modes of one staging path from typed
parser frontier to pass, with zero previous-pass regression, reaching 34,918
passes among 38,421 runnable variants.
R2u adds direct and synchronous for-in/of array assignment patterns, including
member/computed/super References, defaults, rest, recursion, and iterator
unwind. Its direct flat gate passes all 131 variants across 70 paths; the exact
full join adds 15 passes with zero previous-pass regression, reaching 34,933
passes among 38,421 runnable variants. Object assignment is the next
assignment slice.
R2v adds direct and synchronous for-in/of object assignment patterns, including
depth-0-to-3 References, defaults, object/array recursion, exclusion-aware
rest, and iterator unwind. Its three scoped gates pass all 193 variants across
107 paths; the exact full join adds 14 passes with zero previous-pass
regression, reaching 34,947 passes among 38,421 runnable variants.
R2w adds recursive array/object/rest catch BindingPatterns while preserving
catch lexical scope, iterator unwind, direct-eval redeclaration metadata, and
Annex B integration. Its 97-path scoped gate passes all 177 variants; the exact
full join adds 49 passes with zero previous-pass regression, reaching 34,996
passes among 38,421 runnable variants.
R2x adds synchronous identifier-rest parameters to ordinary functions, object
methods, arrows, and the `Function` constructor. Its exact 34-path scoped gate
passes all 65 variants; the full join adds 88 passes with zero previous-pass
regression, reaching 35,084 passes among 38,421 runnable variants. Parameter
Environments, defaults, parameter destructuring, rest BindingPatterns, and
async/generator/class forms remain later FormalParameters milestones.
R2y adds synchronous identifier defaults and a real Parameter Environment to
the same four surfaces. Its exact 76-path scoped gate passes all 143 variants;
the full join adds 60 passes with zero previous-pass regression, reaching
35,144 passes among 38,421 runnable variants.
R2z adds synchronous no-default parameter BindingPatterns across ordinary
functions, object methods, arrows, the `Function` constructor, and setters.
Its exact 149-path scoped gate passes all 298 variants; the full join adds 22
passes with zero previous-pass regression, reaching 35,166 passes among 38,421
runnable variants. BindingPatterns combined with standalone `=` parameter
expressions are the next R3a milestone; async/generator/class forms remain
later callable milestones.
R3a combines standalone `=` parameter expressions with synchronous
BindingPatterns on those surfaces. Its dependency-audited 468-path scoped gate
passes all 936 variants; the full join adds 12 passes with zero previous-pass
regression, reaching 35,178 passes among 38,421 runnable variants.
R3b adds the separate `<var>` / `<arg_var>` direct-eval environment model. Its
71-path scoped gate passes all 71 sloppy variants and its 42-case QuickJS
differential passes all four integration tests. The full join adds 66 passes
with zero previous-pass regression, reaching 35,244 passes among 38,421
runnable variants. Async, generator, and class forms remain later callable
milestones.
R3c adds AggregateError and globally audits Error cause. Its 19-case QuickJS
oracle passes all three integration tests; the complete 56-variant feature
cohort has 50 passes and six exact missing-Proxy dependency results. The full
join adds 52 passes with zero previous-pass regression, reaching 35,296 passes
among 38,483 runnable variants. Proxy, cross-realm host fixtures, class
subclasses, and Promise consumers remain independent milestones.
R3d adds typed ordinary/construct/direct-eval argument spread and the pinned
double-iterator-Get/fast-Array behavior. Its 134-variant focused gate passes
122 with 12 exact adjacent frontiers; the full join adds 124 net passes with no
previous-pass regression, reaching 35,420 among the same 38,483 runnable
variants. The current Symbol protocol and Raw JSON gates pass 892/1,010 and
42/42 runnable variants respectively.
R3e adds synchronous base class declarations/expressions, constructors,
methods/accessors, lexical/TDZ environments, computed names, and HomeObject
semantics. Its 157-path/294-variant focused gate and five-test QuickJS oracle
both pass completely; the full join adds 328 passes with no previous-pass
regression, reaching 35,748 among the same 38,483 runnable variants. Class
heritage/derived constructors, fields/private elements, static blocks, and
generator/async class methods remain separate milestones.
R3f adds synchronous heritage, derived constructors, and `super()` across
direct, arrow, parameter-initializer, and nested direct-eval paths. Its
386-path/767-variant focused gate and dedicated QuickJS differential pass
completely; the exact full join adds 545 passes with no previous-pass
regression, reaching 36,293 among the same 38,483 runnable variants. The global
profile still keeps whole-feature `class` disabled until fields/private
elements, static blocks, and generator/async class methods land.
R3g adds public instance/static fields and static blocks. Its distinct
386-path/767-variant focused gate passes completely against a pinned-QuickJS
oracle. The result remains manifest-scoped; no new whole-suite percentage or
global `class` admission is claimed until the full join and the remaining
private/async/generator class surfaces are complete.
R3h adds private instance/static data fields and private-`in` References. Its
hash-authenticated 630-path/1,260-variant focused gate passes 1,260/1,260;
pinned QuickJS passes 630/630.
R3i adds ordinary synchronous private instance/static methods with independent
class-side brands. Its hash-authenticated 267-path/534-variant focused gate
passes 534/534; pinned QuickJS passes 267/267. R3j adds the disjoint
305-path/610-variant synchronous private-accessor target: Oxide passes 610/610
and pinned QuickJS passes 305/305. R3k adds synchronous generator
declarations/expressions and public object/class generator methods. Its first
authenticated 82-path/160-variant class-generator gate passes 160/160; pinned
QuickJS passes 82/82. Async forms and private generator methods remain later
frontiers at the R3k checkpoint. R3l adds the private instance/static class-
generator slice: its authenticated 82-path/160-variant gate passes 160/160,
and pinned QuickJS passes 82/82. Async forms remain later frontiers, and the
global profile stays closed for `generators`. R3m establishes the Promise
constructor/reaction/job boundary with 112/112 focused variants and QuickJS
57/57 paths. R3n adds `try`, `withResolvers`, and `race`; its complete
112-path/224-variant landing inventory records 214 passes and the ten
explicitly listed `all`/`finally` adjacency failures, while pinned QuickJS
passes 112/112. R3o implements `Promise.prototype.finally`; its complete
29-path/58-variant gate passes 56 variants, with only the two Proxy-dependent
variants failing, while pinned QuickJS passes 29/29. R3p implements
`Promise.all`; its complete 98-path/196-variant gate passes 196/196, and the
unchanged R3n inventory now passes 224/224.
R3q adds `Promise.allSettled` and `Promise.any`; their complete gates pass
208/208 and 188/188. R3r removes the complete vector's final two engine faults
through generator-destructuring iterator unwind. R3s then completes the pinned
non-`v`, non-`createRealm` RegExp built-ins cohort at 3,346/3,346 and raises the
full vector to 36,927 passes without a previous-pass regression.
R3t closes the authenticated synchronous generator/destructuring cohort at
6,593/6,593. Its untagged Annex B fix raises the conservative full vector by
one to 36,928 passes; the scoped cohort itself remains outside that global
score. The global profile remains fail-closed for async and Promise features
and for `generators`/`destructuring-binding`.
The generated Unicode code-point property corpus now passes; properties of
strings remain coupled to `v` mode.
Test262 remains the project scoreboard, while focused QuickJS
differentials decide exact target semantics for each slice. None of these
progress figures is a feature-parity completion claim.
