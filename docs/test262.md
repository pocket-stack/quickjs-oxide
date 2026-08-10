# Test262 baseline

Test262 is the primary broad conformance signal for quickjs-oxide. The suite is
pinned to commit `5c8206929d81b2d3d727ca6aac56c18358c8d790`, using the authenticated
QuickJS 2026-06-04 patch and configuration recorded in
[`compat/upstream.toml`](../compat/upstream.toml).

## Official metrics

Metrics are reported in this order:

1. **Full pass:** 78,234 / 102,037 (76.672%). Every frozen Test262 variant is in
   the denominator.
2. **Eligible coverage:** 78,284 / 102,037 (76.721%). This measures how much of
   the full vector the current profile admits to execution.
3. **Runnable pass quality:** 78,234 / 78,284 (99.936%). This is useful for
   diagnosing admitted behavior, but it must not replace either coverage
   metric above.

The frozen outcome summary is:

```text
fail-parse=7 fail-runtime=43 pass=78234 skipped-config-exclude=6700 skipped-feature=11775 unsupported-feature=1468 unsupported-module=418 unsupported-negative-provenance=3392
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
the 6,382-variant R3eg-B dependency-closed private-callable vector and requires
byte-identical output. It covers `class-methods-private` and
`class-static-methods-private`, including ordinary methods, accessors,
generators, async methods, and async generators. `--full` runs every 102,037
variant and checks the complete summary and report hashes.

Negative admissions remain fail-closed: an expected failure counts only when
its exact path is present in the audited-negative data, and execution still
must produce the required phase and error type. Contracted variants additionally
require the exact pinned-QuickJS message and an `exact` or `absent` location
policy. Schema-v4 receipts record the contract hash and expected/actual
diagnostic fields; new negative admissions must extend this data contract.

Historical per-milestone profiles, copied shell gates, and result vectors are
not executable policy. They are preserved in the release archive listed in
[`dev-support/test262/archive/index.tsv`](../dev-support/test262/archive/index.tsv).
