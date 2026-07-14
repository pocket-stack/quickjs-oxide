# Test262 progress baseline

Test262 is now a pinned progress instrument, not yet a completion claim. The
authoritative compatibility target remains QuickJS 2026-06-04; focused QuickJS
differentials still decide exact behavior inside each implemented slice.

## Pinned inputs

- Test262 commit: `5c8206929d81b2d3d727ca6aac56c18358c8d790`
- QuickJS patch SHA-256: `f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3`
- QuickJS config SHA-256: `79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b`
- 53,125 non-fixture metadata records SHA-256:
  `a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a`

`scripts/prepare-test262.sh` prepares and verifies that exact checkout and the
two harness changes carried by the QuickJS release. No Test262 source is
vendored into the product.

## Current baseline

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

Do not interpret `--all` as an authoritative pass rate yet. Some valid but
unimplemented grammar still reaches a generic `SyntaxError` path instead of
typed `Unsupported`; a parse-negative arrow-function test is a known example
which would otherwise be counted as a pass for the wrong reason. Every passing
parse-negative test in the fixed smoke manifest was audited and does not cross
one of those remaining boundaries. Missing `$262` host capabilities can also
still blend into ordinary runtime failures outside this fixed manifest. The
next milestone closes both provenance gaps before saving the full-suite vector.

## Runner contract

`run-test262` currently supports the trustworthy synchronous-script subset:

- fresh Rust process, `Runtime`, and `Context` for every variant;
- hard parent-process timeout and crash classification;
- canonical Test262 `raw` behavior (no harness or strict prefix);
- separate harness compilation/evaluation, then test compile and execute;
- exact parse-versus-runtime negative phase and constructor-name checks;
- explicitly typed implementation-frontier errors kept distinct from
  JavaScript `SyntaxError`;
- deterministic TSV outcome vector plus a JSONL sidecar;
- module and async variants reported as unsupported and treated as failures
  unless a caller is explicitly recording a baseline.

Complete `$262` host-capability provenance is intentionally not claimed yet;
that classification is part of the next milestone.

This deliberately fixes three known limitations in the pinned QuickJS
`run-test262.c`: it does not discard negative phase, does not load harness code
for `raw`, and does not let a stable known-error ledger hide the raw failure
count. A future QuickJS-runner compatibility profile may reproduce those quirks
for outcome-vector differential work, but it must remain separate from the
canonical progress report.

## Reproduce

```sh
./scripts/test-test262-smoke.sh
```

The command exhaustively validates all pinned metadata against the independent
fingerprint, runs the fixed smoke manifest, and compares the complete result to
the checked-in TSV and JSONL baselines. Any new pass, failure, skip, timeout,
crash, or diagnostic change is visible as baseline drift.

The next Test262 milestone is a complete parser-frontier provenance audit,
followed by a complete classified outcome vector. It must preserve
unsupported/module/async/host buckets instead of inflating the pass rate, and
should add bounded parallel workers before the roughly 100,000 script variants
are used as a routine gate.
