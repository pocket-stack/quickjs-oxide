# Test262 progress baseline

Test262 is now a pinned progress instrument, not yet a completion claim. The
authoritative compatibility target remains QuickJS 2026-06-04; focused QuickJS
differentials still decide exact behavior inside each implemented slice.

## Pinned inputs

- Test262 commit: `5c8206929d81b2d3d727ca6aac56c18358c8d790`
- QuickJS patch SHA-256: `f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3`
- QuickJS config SHA-256: `79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b`
- quickjs-oxide capability profile SHA-256:
  `f9bf8afb9a1147cac24da1b3cb8b65d473a8470b5f7ef0418ce4e0add8497560`
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

- 17,365 pass;
- 18,475 are outside the pinned QuickJS target configuration;
- 58,322 are classified as unsupported feature, mode, host capability, parser
  frontier, harness frontier, or unaudited negative-test provenance;
- 2,074 fail to parse, 5,591 fail at runtime, 208 fail in the harness, and two
  time out; there are no crashes or runner/engine infrastructure faults.

Three rates answer different questions:

- raw suite pass rate: 17.02% (`17,365 / 102,037`);
- conservative target-scope lower bound: 20.78%
  (`17,365 / (102,037 - 18,475)`);
- pass rate among variants with a non-unsupported observed outcome: 68.80%
  (`17,365 / 25,240`).

The 20.78% figure is the useful whole-project progress floor, not a claim that
the engine is 20.78% conformant. The 68.80% conditional rate measures quality
only on the currently exposed frontier and must not be read as overall
completion. The capability profile currently admits ten reviewed Test262
feature tags and 18 reviewed negative-test paths; all
other feature-tagged or negative-provenance cases fail closed. Expanding that
profile as implementation lands can only make the measurement more
representative. Focused QuickJS differential tests remain the semantic judge.

The complete TSV/JSONL reports are generated under `target/` rather than
committed (together they are tens of megabytes). Their complete hashes and
outcome summary are pinned in `tests/test262-full-baseline.txt`. Runs with five
and eight workers have produced byte-identical vectors; the current baseline
was reproduced with the default eight workers.

## Milestone policy

Test262 is now the project-wide milestone scoreboard, while the pinned QuickJS
source and focused differential probes remain the semantic specification for
each feature slice. A substantial slice lands only after its Rust/unit and
QuickJS differential gates pass; the full Test262 vector then records pass
movement, regressions, newly exposed failures, and unsupported-frontier
movement. Small implementation commits do not need an independent full-suite
run.

This simple-head `for-in` milestone moved 16,675 to 17,365 passes with no
previous-pass regression. Of the 690 added passes, 668 came from the direct
source frontier, eight from adjacent tests that had previously reached a real
runtime `SyntaxError`, and 14 from admitting the `for-in-order` feature tag.
The same expansion honestly exposed 124 direct runtime failures, two direct
parse failures, and four `for-in-order` runtime failures. The target lower
bound moved from 19.96% to 20.78%. Progress estimates should therefore quote
that lower bound together with the unsupported mass and major
parse/runtime/harness frontiers; the conditional 68.80% rate is diagnostic,
not an overall estimate. A keyed old/new audit matched all 102,037 variants;
the 97,403 variants outside the reviewed for-in buckets had no outcome drift.

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
./scripts/test-test262-full.sh
```

The smoke command also exhaustively validates pinned metadata against its
independent fingerprint. The provenance command guards known false-positive
boundaries. The full command uses the release runner, defaults to eight workers,
and compares the complete outcome vector and sidecar by SHA-256. Set
`TEST262_WORKERS` to change concurrency without changing the expected bytes.

All 3,770 variants that previously stopped at `propertyHelper.js`'s `for-in`
frontier now reach its next shared blocker: the implicit ordinary-function
`arguments` binding. Implementing that binding, followed by the missing Math
surface, is the next high-leverage harness path. The direct `for-in` frontier
now exposes 44 later unsupported boundaries: 22 implicit-arguments variants,
ten `with`, ten destructuring, and two object-method variants. The four exposed
`for-in-order` failures require JSON. Normal implementation work should move the
classified vector at feature milestones, while focused QuickJS differentials
protect feature-parity semantics inside each admitted slice.
