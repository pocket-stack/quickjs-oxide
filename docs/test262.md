# Test262 baseline

Test262 is the primary broad conformance signal for quickjs-oxide. The suite is
pinned to commit `5c8206929d81b2d3d727ca6aac56c18358c8d790`, using the authenticated
QuickJS 2026-06-04 patch and configuration recorded in
[`compat/upstream.toml`](../compat/upstream.toml).

## Official metrics

<!-- current-test262-metrics:start -->
Metrics are reported in this order:

1. **Full pass:** 79,064 / 102,037 (77.486%). Every frozen Test262 variant is in
   the denominator.
2. **Eligible coverage:** 79,114 / 102,037 (77.535%). This measures how much of
   the full vector the current profile admits to execution.
3. **Runnable pass quality:** 79,064 / 79,114 (99.937%). This is useful for
   diagnosing admitted behavior, but it must not replace either coverage
   metric above.

The frozen outcome summary is:

```text
fail-parse=7 fail-runtime=43 pass=79064 skipped-config-exclude=6700 skipped-feature=11775 unsupported-feature=1468 unsupported-module=418 unsupported-negative-provenance=2562
```
<!-- current-test262-metrics:end -->

## Reproduce

One script consumes one inert data spec:

```sh
./scripts/test-test262.sh --spec dev-support/test262/current.conf --check
./scripts/test-test262.sh --spec dev-support/test262/current.conf --focused
TEST262_WORKERS=2 ./scripts/test-test262.sh \
  --spec dev-support/test262/current.conf --full
```

`--check` authenticates the current upstream pin, profile, negative-diagnostic
contract and legacy exemption ledger, focused manifest, and frozen TSV/JSONL
receipts. `--focused` replays
the 6,382-variant R3eg-B dependency-closed private-callable vector and requires
byte-identical output. It covers `class-methods-private` and
`class-static-methods-private`, including ordinary methods, accessors,
generators, async methods, and async generators. `--full` runs every 102,037
variant and checks the complete summary and report hashes.

Negative admissions remain fail-closed: an expected failure counts only when
its exact path is present in the audited-negative data. Every admitted
path/variant must then belong to exactly one diagnostic class. The checked-in
exact contracts require the pinned-QuickJS phase, type, message, and `exact` or
`absent` location policy. A frozen ledger identifies the 2,586 legacy variants
that still check phase and type only. Schema-v5 receipts authenticate both data
files and record expected/actual diagnostic fields. New negative admissions
must add exact contracts; they cannot add implicit phase/type-only cases.
The semantic rule registry is separately authenticated, and scheduled QuickJS
differential CI replays every exact variant against QuickJS 2026-06-04 so
the stored message and location cannot drift into an Oxide-only oracle.

Historical per-milestone profiles, copied shell gates, and result vectors are
not executable policy. They are preserved in the release archive listed in
[`dev-support/test262/archive/index.tsv`](../dev-support/test262/archive/index.tsv).
