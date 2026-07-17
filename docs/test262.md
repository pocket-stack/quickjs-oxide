# Test262 progress baseline

Test262 is now a pinned progress instrument, not yet a completion claim. The
authoritative compatibility target remains QuickJS 2026-06-04; focused QuickJS
differentials still decide exact behavior inside each implemented slice.

## Pinned inputs

- Test262 commit: `5c8206929d81b2d3d727ca6aac56c18358c8d790`
- QuickJS patch SHA-256: `f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3`
- QuickJS config SHA-256: `79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b`
- quickjs-oxide capability profile SHA-256:
  `b39bee15a2aaa88e00c8f7ca6cb0736313456d43a77e176a8c5cf7844e9ea718`
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
surfaces. Module, async/jobs, most `$262` host hooks, advanced RegExp pattern
grammar, classes,
generators,
TypedArrays and many other broad layers remain absent.

Nineteen additional provenance variants guard the result: four audited negative
variants pass for the intended parse error, while 15 unsupported grammar
variants fail closed instead of passing because they happened to throw a
`SyntaxError`.

## Complete classified vector

The pinned suite expands to 102,037 sloppy/strict variants. The runner emits
every outcome in canonical order, and the checked-in baseline pins the complete
vector hashes and summary:

- 27,569 pass;
- 18,475 are outside the pinned QuickJS target configuration;
- 51,075 are classified as unsupported feature, mode, host capability, parser
  frontier, harness frontier, or unaudited negative-test provenance;
- 1,005 fail to parse, 3,699 fail at runtime, 210 fail in the harness, and four
  time out; there are no crashes or runner/engine infrastructure faults.

The runner admitted 34,773 variants to execution. That count includes variants
which then report a typed parser or harness frontier rather than an observed
non-unsupported outcome.

Three rates answer different questions:

- raw suite pass rate: 27.02% (`27,569 / 102,037`);
- conservative target-scope lower bound: 32.99%
  (`27,569 / (102,037 - 18,475)`);
- pass rate among variants with a non-unsupported observed outcome: 84.86%
  (`27,569 / 32,487`).

The 32.99% figure is the useful whole-project progress floor, not a claim that
the engine is 32.99% conformant. The 84.86% conditional rate measures quality
only on the currently exposed frontier and must not be read as overall
completion. The capability profile currently admits 28 reviewed Test262
feature tags and 307 reviewed negative-test paths; all other feature-tagged or
negative-provenance cases fail closed. Expanding that profile as implementation
lands can only make the measurement more representative. Focused QuickJS
differential tests remain the semantic judge.

The complete TSV/JSONL reports are generated under `target/` rather than
committed (together they are tens of megabytes). Their complete hashes and
outcome summary are pinned in `tests/test262-full-baseline.txt`. Runner ordering
was cross-checked at five and eight workers through the scoped RegExp modifier
milestone; the current byte expectations use a fixed
`TZ=America/Los_Angeles`. The hash gate therefore requires a Unix-like zoneinfo
installation; Windows still lacks the corresponding IANA-zone backend.
The current TSV and JSONL SHA-256 values are
`e09478accaf05c27e39555c5a4c1889617c97ce5c1454ddf945c7f675ea3d2ef`
and
`95ea74491558035ac02af4f60c3a2d202120798fc2ab08c41c7050a6031e950b`.

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
`separator-regexp.js` variants. The current gate has 236 passes and four typed
parser frontiers; current TSV/JSONL hashes are
`d8befca75b131842c564099071f3558aa08150102a87679b20f0ca2f83c1a1fd`
and
`7039dfbd827a0db6465c8837372bf4b0419aa9f73119b2828aaced4cce749ae4`.

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
Unicode bare-`\k` diagnostic resolves two more. The current core vector has 438
passes, ten missing-`eval` runtime failures, and two typed legacy-control
frontiers. Its TSV/JSONL hashes are
`795528ba6f14fa0955f632db1c8883e2a3e54db70f811e6cb3b9485b7de81fd6`
and
`b37b1c7d255ceac85d1ed930f68a9beadd01dbf03fd485574702f846417ba8bc`.
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
reproduces the gate. It admits 200 variants and records 182 passes, two
`fail-runtime` outcomes at the independent missing-`eval` frontier, 16 typed
`unsupported-parser` outcomes from eight object-literal method/accessor paths,
and eight outcomes still gated by four adjacent-feature paths. At R1c the
same keys were two passes, 76 runtime failures and 130 feature-gated outcomes.
The focused TSV and JSONL SHA-256 values are
`13811543bada9e1f91e69f9b6aee968812d9dcb67e7aa549fabeb20c8b3c10e6`
and
`cfeef60ff5c832df3591e88548042ff3881f771743fd10b15a324a95b0aee5a9`.

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
gate. It admits 48 variants and records 44 passes after R1f links both Annex-B
recompilation paths. Forty core variants remain conservatively gated by the
undeclared `Symbol.species` profile tag, two variants by `arrow-function`, two
require the create-realm host hook, and four retain typed parser frontiers. The
QuickJS differential suite separately locks SpeciesConstructor semantics
without widening the full-suite capability profile. Its current TSV and JSONL
SHA-256 values are
`ce13c1409af6af84140fb79457c2e98e3d9264a054444dcd5359803c4545ab48`
and
`e9c24f1210c1e1b27225e94bdd0cb56d3daa8f5c7b09873492c314a51c8cc490`.
The independent 127-path String split gate now records the 236 passes and
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
R1i's direct standard-RegExp route preserved that outcome vector. R1p now
admits 348 variants and records 300 passes, two arrow-function plus four
rest-parameter/array-spread parse failures, 42 typed parser frontiers, 22
adjacent feature outcomes, and six host outcomes. Current focused TSV/JSONL
hashes are
`06c12e5aa4b874c56c6a28262fa6dde9f6efec3f3dedd803d213f294ab7c7749`
and
`0f5d159db6972e241e43f67363f8ca84c3575215005d20032610a41091318256`.

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
variants, and R1p admits the final six named-group variants. The current gate
therefore runs all 98 variants: 94 pass and four reach typed
lexical-destructuring parser frontiers. Current focused TSV/JSONL hashes are
`fb9f4501ef7a267b6a6e10275ce88c4c914dbb782f8b56806341927b60c7309c`
and
`288906a0a7edd0229807931614a6b60aac551bfd1619c9fe5a07dca3687a126f`.

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
`f161d1c666e327b16e5ab7b57d7d77371a1d96d65076c7fda7b96656c8b534de`
and
`8215004edf9d2cca29cdf403cd52cf2fa24cc043f83efc0adbab1f0e16a9bc4d`.

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

The frozen focused gate contains 31 paths and 62 variants. It admits 50 and
passes 38; two variants fail at the existing arrow-function parser frontier,
four stop in the existing `deepEqual.js` harness frontier, and six reach the
typed object-setter parser frontier. Ten remain behind the independently gated
`regexp-dotall` feature, while two retain the missing `$262.createRealm` host
requirement. Focused TSV/JSONL hashes are
`b626f453c4a22402c9bf35f0b6a95ad3cf54cb2095ff21c023a150ec6904a230`
and
`edc7cb06eb9d18596202ae4d6f9faa4e56c1e2c4a6a81b51a54a26b0b34cd31f`.

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
variants. Basic RegExp literal execution, the search/match/split protocols,
legacy compile, scoped modifiers, generic replacement, matchAll, and numeric
backreferences, forward lookahead, lookbehind, Unicode property escapes,
ordinary named captures, duplicate named captures, and match indices are now
measured separately in
R1b/R1c/R1d/R1e/R1f/R1g/R1h/R1j/R1k/R1l/R1m/R1n/R1o/R1p/R1q/R1r; R1i
completes the direct standard-RegExp replacement route without changing that
scoreboard.
The generated Unicode code-point property corpus now passes; properties of
strings remain coupled to `v` mode.
Test262 remains the project scoreboard, while focused QuickJS
differentials decide exact target semantics for each slice. None of these
progress figures is a feature-parity completion claim.
