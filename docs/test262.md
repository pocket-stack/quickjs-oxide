# Test262 progress baseline

Test262 is now a pinned progress instrument, not yet a completion claim. The
authoritative compatibility target remains QuickJS 2026-06-04; focused QuickJS
differentials still decide exact behavior inside each implemented slice.

## Pinned inputs

- Test262 commit: `5c8206929d81b2d3d727ca6aac56c18358c8d790`
- QuickJS patch SHA-256: `f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3`
- QuickJS config SHA-256: `79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b`
- quickjs-oxide capability profile SHA-256:
  `24e10f7a2010452799c1a9ba99b6da74d5c45ece098cfe3ef4ddbb13105c35f7`
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
surfaces. Module, async/jobs, most `$262` host hooks, RegExp, classes,
generators, TypedArrays and many other broad layers remain absent.

Nineteen additional provenance variants guard the result: four audited negative
variants pass for the intended parse error, while 15 unsupported grammar
variants fail closed instead of passing because they happened to throw a
`SyntaxError`.

## Complete classified vector

The pinned suite expands to 102,037 sloppy/strict variants. The runner emits
every outcome in canonical order, and the checked-in baseline pins the complete
vector hashes and summary:

- 21,740 pass;
- 18,475 are outside the pinned QuickJS target configuration;
- 53,510 are classified as unsupported feature, mode, host capability, parser
  frontier, harness frontier, or unaudited negative-test provenance;
- 2,140 fail to parse, 5,962 fail at runtime, 206 fail in the harness, and four
  time out; there are no crashes or runner/engine infrastructure faults.

The runner admitted 32,227 variants to execution. That count includes variants
which then report a typed parser or harness frontier rather than an observed
non-unsupported outcome.

Three rates answer different questions:

- raw suite pass rate: 21.31% (`21,740 / 102,037`);
- conservative target-scope lower bound: 26.02%
  (`21,740 / (102,037 - 18,475)`);
- pass rate among variants with a non-unsupported observed outcome: 72.34%
  (`21,740 / 30,052`).

The 26.02% figure is the useful whole-project progress floor, not a claim that
the engine is 26.02% conformant. The 72.34% conditional rate measures quality
only on the currently exposed frontier and must not be read as overall
completion. The capability profile currently admits 15 reviewed Test262
feature tags and 18 reviewed negative-test paths; all
other feature-tagged or negative-provenance cases fail closed. Expanding that
profile as implementation lands can only make the measurement more
representative. Focused QuickJS differential tests remain the semantic judge.

The complete TSV/JSONL reports are generated under `target/` rather than
committed (together they are tens of megabytes). Their complete hashes and
outcome summary are pinned in `tests/test262-full-baseline.txt`. Runner ordering
has previously been cross-checked at five and eight workers; the current byte
expectations use the default eight workers.

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

The focused Reflect vector now has 311 passes, 40 runtime failures, ten parse
failures, six harness failures, six explicit parser frontiers, and 54 variants
still gated by honest adjacent feature requirements. All 153
`built-ins/Reflect` files are represented: 248 variants pass, 54 stay gated by
Proxy, arrow/computed-property grammar or `Symbol.toStringTag`, and the
remaining four reach the still-missing `Date.now`. The non-pass results thus
record adjacent implementation frontiers rather than being hidden from the
scoreboard.

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
./scripts/test-test262-full.sh
```

The smoke command also exhaustively validates pinned metadata against its
independent fingerprint. The provenance command guards known false-positive
boundaries. The full command uses the release runner, defaults to eight workers,
and compares the complete outcome vector and sidecar by SHA-256. Set
`TEST262_WORKERS` to change concurrency without changing the expected bytes.

Math and Reflect are no longer common blockers in their reviewed sets. Within
the earlier 4,589-variant Math reviewed set, the leading named dependencies are
Date (308 variants), DataView (86), Map (72), RegExp (70), eval (69), Promise
(60), and Set (58); the 308 count is not the size of the full Date suite. The
pinned `built-ins/Date` tree itself contains 594 files and 1,188 sloppy/strict
variants, while four otherwise-ready Reflect variants also stop at `Date.now`.
Date is therefore the next selected intrinsic milestone. Test262 remains the
project scoreboard, while focused QuickJS differentials decide exact target
semantics for each slice.
