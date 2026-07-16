# Test262 progress baseline

Test262 is now a pinned progress instrument, not yet a completion claim. The
authoritative compatibility target remains QuickJS 2026-06-04; focused QuickJS
differentials still decide exact behavior inside each implemented slice.

## Pinned inputs

- Test262 commit: `5c8206929d81b2d3d727ca6aac56c18358c8d790`
- QuickJS patch SHA-256: `f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3`
- QuickJS config SHA-256: `79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b`
- quickjs-oxide capability profile SHA-256:
  `cc10293aa847f5a449ac2b039709dff98d264b672dddc8828b8e17d8b7e12d9a`
- 53,125 non-fixture metadata records SHA-256:
  `a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a`

`scripts/prepare-test262.sh` prepares and verifies that exact checkout and the
two harness changes carried by the QuickJS release. No Test262 source is
vendored into the product.

## Smoke baseline

`tests/test262-smoke.txt` fixes 100 synchronous script tests. They expand to
193 independent sloppy/strict variants:

- 189 pass;
- 4 are honestly classified as `unsupported-parser`;
- 0 ordinary semantic failures, timeouts, crashes, harness errors, or skips.

The four unsupported variants are parse-negative tests involving `class` or
`super`. Before unsupported compiler provenance was separated from real syntax
errors, those implementation gaps could masquerade as passing `SyntaxError`
tests. They are intentionally not counted as passes now.

This 189/193 result is a runner smoke baseline, not a project-wide 97.9%
estimate. The sample was selected from already implemented synchronous
surfaces. Module, async/jobs, most `$262` host hooks, RegExp's remaining
matchAll/replace well-known-Symbol protocols and advanced pattern grammar,
classes, generators,
TypedArrays and many other broad layers remain absent.

Nineteen additional provenance variants guard the result: four audited negative
variants pass for the intended parse error, while 15 unsupported grammar
variants fail closed instead of passing because they happened to throw a
`SyntaxError`.

## Complete classified vector

The pinned suite expands to 102,037 sloppy/strict variants. The runner emits
every outcome in canonical order, and the checked-in baseline pins the complete
vector hashes and summary:

- 25,119 pass;
- 18,475 are outside the pinned QuickJS target configuration;
- 53,401 are classified as unsupported feature, mode, host capability, parser
  frontier, harness frontier, or unaudited negative-test provenance;
- 985 fail to parse, 3,847 fail at runtime, 206 fail in the harness, and four
  time out; there are no crashes or runner/engine infrastructure faults.

The runner admitted 32,497 variants to execution. That count includes variants
which then report a typed parser or harness frontier rather than an observed
non-unsupported outcome.

Three rates answer different questions:

- raw suite pass rate: 24.62% (`25,119 / 102,037`);
- conservative target-scope lower bound: 30.06%
  (`25,119 / (102,037 - 18,475)`);
- pass rate among variants with a non-unsupported observed outcome: 83.28%
  (`25,119 / 30,161`).

The 30.06% figure is the useful whole-project progress floor, not a claim that
the engine is 30.06% conformant. The 83.28% conditional rate measures quality
only on the currently exposed frontier and must not be read as overall
completion. The capability profile currently admits 18 reviewed Test262
feature tags and 18 reviewed negative-test paths; all other feature-tagged or
negative-provenance cases fail closed. Expanding that profile as implementation
lands can only make the measurement more representative. Focused QuickJS
differential tests remain the semantic judge.

The complete TSV/JSONL reports are generated under `target/` rather than
committed (together they are tens of megabytes). Their complete hashes and
outcome summary are pinned in `tests/test262-full-baseline.txt`. Runner ordering
was cross-checked at five and eight workers through the RegExp split
milestone; the current byte expectations use a fixed
`TZ=America/Los_Angeles`. The hash gate therefore requires a Unix-like zoneinfo
installation; Windows still lacks the corresponding IANA-zone backend.
The R1e TSV and JSONL SHA-256 values are
`5673ac15896bab5b1665bf8930db517447012c3d63d69bfbb1da9b8e7f9574c1`
and
`fe98f9fdb5f4c21c25cd045d8b1824fe34e3481e26c8661376d7afe78596fa64`.

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

With the subsequent Date, generic-split, and RegExp R1a/R1b slices linked, the
focused Reflect vector has 331 passes, 22 runtime failures, eight parse
failures, six harness failures, six explicit parser frontiers, and 54 variants
still gated by honest adjacent feature requirements. The two R1b transitions
are the sloppy and strict variants of
`Object/getOwnPropertyDescriptors/order-after-define-property.js`, whose RegExp
literal now reaches and passes its property-order assertions. All 153
`built-ins/Reflect` files are
represented: 252 variants pass and 54 stay gated by Proxy,
arrow/computed-property grammar or `Symbol.toStringTag`; every admitted
built-ins/Reflect variant now passes. The other non-pass results expose
ArrayBuffer, JSON, TypedArray, parser, or harness frontiers rather than
being hidden from the scoreboard.

The observable Date milestone moves the complete vector from 21,740 to 23,016
passes without changing its 32,227 admitted jobs. An exact keyed join across
all 102,037 variants records exactly 1,276 `fail-runtime -> pass` transitions,
no previous-pass regression, and no outcome change outside the reviewed Date
manifest. Five- and eight-worker full reports are byte-identical.

The Date-focused review corpus contains 799 paths and 1,598 sloppy/strict
variants. Its Date-owned subset contains all 646 paths and 1,292 variants from
`built-ins/Date`, `annexB/built-ins/Date`, and `staging/sm/Date`; 153 adjacent
paths expose Date through globals, reflection, constructors, or indirect
dependencies. The currently linked focused outcome vector has 1,298 passes, 22
parse failures, four runtime failures, 34 configured/feature skips, and 240
explicitly unsupported outcomes. The runner admits 1,394 jobs; 70 of those
terminate at the typed parser frontier, leaving 1,324 non-unsupported observed
outcomes and a 98.04% pass rate on that frontier (81.23% of the complete
focused vector).

The 26 current parse/runtime non-passes are explained adjacent frontiers rather
than Date algorithm drift. RegExp R1b moves ten formerly literal-blocked
variants forward: eight pass, while the two variants of
`staging/sm/Date/toString-generic.js` continue to their still-unsupported arrow
syntax. Arrow syntax therefore accounts for all 22 parse failures. Generic
split and RegExp R1a resolve fourteen other formerly blocked variants; the four
remaining runtime failures require `eval` or older complete-global inventory.
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
`RegExp.prototype[Symbol.split]`. The same frozen vector now admits 244
variants and records 234 passes, four runtime failures at the independent
Annex-B legacy-`compile` frontier, eight adjacent feature outcomes, two
IsHTMLDDA host outcomes, and six typed parser frontiers. Its TSV and JSONL
SHA-256 values are
`ad66315d9b6d285240d9f0628a899ab71b64496ea451f153bcf4916d7ffeccdb`
and
`c0182c6f56c9df1cb4b1e991f60aa94aa5c8173e01f7882e7fa4031e966eaebc`.
The capability profile remains unchanged at 18 reviewed tags.

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
grammar explicitly. The same 450-variant core vector now records 430 passes,
ten `fail-runtime` outcomes caused only by the separate missing-`eval`
frontier, and ten `unsupported-runtime` outcomes for backreferences,
lookaround, Unicode properties, or legacy octal/control escapes. The full
vector moves from 23,190 to 23,859 passes, reduces `fail-runtime` from 4,540 to 3,861,
and adds ten typed `unsupported-runtime` outcomes. RegExp literals, legacy
`compile`, `RegExp.escape`, and Symbol protocols are not claimed by this slice.
An exact join matches all 102,037 `(path, variant)` keys with no duplicates or
missing rows and zero previous-pass regressions. Its only 679 transitions are
669 `fail-runtime -> pass` and ten
`fail-runtime -> unsupported-runtime`. The new passes span 462 RegExp, 132
Object, 42 Array, 12 String, nine language-expression and 12 adjacent global,
literal or staging variants; those collateral groups construct or consume
regular expressions rather than representing unrelated feature work.
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
`String.prototype.match` call; R1d later makes both pass, moving the current
focused literal vector from 88 to 90 passes. The complete R1b vector moves
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
`scripts/run-test262-regexp-search.sh` reproduces the gate. It admits 124
variants and records 112 passes, 12 typed `unsupported-parser` outcomes from
six object-literal method/accessor paths in both modes, and eight outcomes still
gated by adjacent feature requirements. At R1b the same keys were 2 passes, 60
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
reproduces the gate. It admits 198 variants and records 180 passes, two
`fail-runtime` outcomes at the independent missing-`eval` frontier, 16 typed
`unsupported-parser` outcomes from eight object-literal method/accessor paths,
and ten outcomes still gated by five adjacent feature declarations. At R1c the
same keys were two passes, 76 runtime failures and 130 feature-gated outcomes.
The focused TSV and JSONL SHA-256 values are
`7db1917f2f5e2f0ed2a9a5bfb01a3bda94c498a92bfaf38f8519e642127fac84`
and
`1450d3d8445e86ab30b3b6fc80386a18358a8b36811c4150afc6073207302707`.

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
gate. It admits 48 variants and records 40 passes, four runtime failures at the
independent Annex-B legacy-`compile` frontier, 40 core variants conservatively
gated by the undeclared `Symbol.species` profile tag, two variants gated by
`arrow-function`, two create-realm host outcomes, and four typed parser
frontiers. The QuickJS differential suite separately locks SpeciesConstructor
semantics without widening the full-suite capability profile. Its TSV and
JSONL SHA-256 values are
`f8e4f9fa74f3f2843e9d3c101438a26ada2dde7d69d9a387114d834cf3447566`
and
`5e860397dcda991548347474ca13106b4289db5149f215227f7c0fb40fa349b7`.
The independent 127-path String split gate now records the 234 passes and
hashes above.

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
  and the `$262` host hooks used by the pinned suite before execution;
- bounded parallel workers with deterministic result ordering and full child
  cleanup after errors;
- deterministic TSV outcome vector plus a JSONL sidecar;
- module and async variants reported as unsupported and treated as failures
  unless a caller is explicitly recording a baseline.

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
./scripts/run-test262-regexp-literals.sh
./scripts/run-test262-regexp-search.sh
./scripts/run-test262-regexp-match.sh
./scripts/run-test262-regexp-split.sh
./scripts/test-test262-full.sh
```

The smoke command also exhaustively validates pinned metadata against its
independent fingerprint. The provenance command guards known false-positive
boundaries. The full command uses the release runner, defaults to eight workers,
and compares the complete outcome vector and sidecar by SHA-256. Set
`TEST262_WORKERS` to change concurrency without changing the expected bytes.

Math, Reflect, Date, and generic `String.prototype.split` are no longer common
blockers in their reviewed sets.
The Date transition also resolves the four otherwise-ready Reflect variants
which had stopped at `Date.now`; generic split resolves six more linked Reflect
variants. Basic RegExp literal execution and the search/match/split protocols
are now measured separately in R1b/R1c/R1d/R1e. MatchAll/replace protocol work
and advanced RegExp grammar are the next conservative candidates; the complete
classified report will decide their ordering. The remaining RegExp-backed
String methods stay named frontiers. Test262 remains the project scoreboard,
while focused QuickJS differentials decide exact target semantics for each
slice. None of these progress figures is a feature-parity completion claim.
