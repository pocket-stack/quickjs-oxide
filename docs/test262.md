# Test262 baseline

Test262 is the primary broad conformance signal for quickjs-oxide. The suite is
pinned to commit `5c8206929d81b2d3d727ca6aac56c18358c8d790`, using the authenticated
QuickJS 2026-06-04 patch and configuration recorded in
[`compat/upstream.toml`](../compat/upstream.toml).

## Official metrics

Metrics are reported in this order:

1. **Full pass:** 68,362 / 102,037 (66.997%). Every frozen Test262 variant is in
   the denominator.
2. **Eligible coverage:** 68,414 / 102,037 (67.048%). This measures how much of
   the full vector the current profile admits to execution.
3. **Runnable pass quality:** 68,362 / 68,414 (99.924%). This is useful for
   diagnosing admitted behavior, but it must not replace either coverage
   metric above.

The frozen outcome summary is:

```text
fail-parse=7 fail-runtime=43 pass=68362 skipped-config-exclude=6700 skipped-feature=11775 timeout=2 unsupported-feature=11338 unsupported-module=418 unsupported-negative-provenance=3392
```

## Reproduce

One script consumes one inert data spec:

```sh
./scripts/test-test262.sh --spec dev-support/test262/current.conf --check
./scripts/test-test262.sh --spec dev-support/test262/current.conf --focused
TEST262_WORKERS=2 ./scripts/test-test262.sh \
  --spec dev-support/test262/current.conf --full
```

`--check` authenticates the current upstream pin, profile, negative-diagnostic
contract, focused manifest, and frozen TSV/JSONL receipts. `--focused` replays
the 67-case R3ed-A vector and requires byte-identical output. `--full` runs
every 102,037 variant and checks the complete summary and report hashes.

Negative admissions remain fail-closed: an expected failure counts only when
its exact path is present in the audited-negative data, and execution still
must produce the required phase and error type. Contracted variants additionally
require the exact pinned-QuickJS message and an `exact` or `absent` location
policy. Schema-v4 receipts record the contract hash and expected/actual
diagnostic fields; new negative admissions must extend this data contract.

Historical per-milestone profiles, copied shell gates, and result vectors are
not executable policy. They are preserved in the release archive listed in
[`dev-support/test262/archive/index.tsv`](../dev-support/test262/archive/index.tsv).
