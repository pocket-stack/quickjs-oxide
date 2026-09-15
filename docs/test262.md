# Test262 baseline

Test262 is the primary broad conformance signal for quickjs-oxide. The suite is
pinned to commit `5c8206929d81b2d3d727ca6aac56c18358c8d790`, using the authenticated
QuickJS 2026-06-04 patch and configuration recorded in
[`compat/upstream.toml`](../compat/upstream.toml).

## Official metrics

<!-- current-test262-metrics:start -->
Metrics are reported in this order:

1. **Full pass:** 79,982 / 102,037 (78.385%). Every frozen Test262 variant is in
   the denominator.
2. **Eligible coverage:** 80,032 / 102,037 (78.434%). This measures how much of
   the full vector the current profile admits to execution.
3. **Runnable pass quality:** 79,982 / 80,032 (99.938%). This is useful for
   diagnosing admitted behavior, but it must not replace either coverage
   metric above.

The frozen outcome summary is:

```text
fail-parse=7 fail-runtime=43 pass=79982 skipped-config-exclude=6700 skipped-feature=11775 unsupported-feature=847 unsupported-module=121 unsupported-negative-provenance=2562
```
<!-- current-test262-metrics:end -->

## Reproduce

One script consumes one inert data spec:

```sh
./scripts/test262/test-test262.sh --spec dev-support/test262/current.conf --check
./scripts/test262/test-test262.sh --spec dev-support/test262/current.conf --runner-provenance
./scripts/test262/test-test262.sh --spec dev-support/test262/current.conf --focused
TEST262_WORKERS=2 ./scripts/test262/test-test262.sh \
  --spec dev-support/test262/current.conf --full
```

`--check` authenticates the current upstream pin, profile, negative-diagnostic
contract and legacy exemption ledger, focused manifest, and frozen TSV/JSONL
receipts. `--runner-provenance` builds the Rust runner with the current source
fingerprint, verifies the embedded binding, and executes no Test262 cases.
`--focused` replays the current 6,844-variant focused vector and
requires byte-identical output. It retains the dependency-closed
private-callable, static import-attributes, and static JSON module coverage and
adds source-authenticated dynamic-import and top-level-await syntax, runtime,
graph, and rejection cohorts plus the exact dependency-free module
local-binding family. `--full` runs every 102,037 variant and checks the complete
summary and report hashes.

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

## 栈 VM 的显式测试配置

`./scripts/test262/test-test262.sh --stack-vm --full` 使用同一冻结 Test262 向量验收
非默认 VM；省略 `--stack-vm` 验收默认配置。两者不修改 admission、skip 或期望值。
构建读取 Cargo compiler-artifact 的实际 features，并将配置、源码指纹和二进制哈希
写入 `target/test262-runner-{default|stack-vm}.json`。栈 VM 的完整结果另存为
`target/test262-stack-vm-full.{tsv,jsonl}`，避免覆盖默认结果。
完整执行先认证当前 runner 与报告的源码指纹，再对冻结 receipt 校验结果字节。
若源码指纹不同，仅在临时副本中还原首行的来源字段，然后比较原冻结 SHA-256；
其他 metadata 和逐用例结果必须完全一致，保存的原报告仍保留当前源码指纹。
