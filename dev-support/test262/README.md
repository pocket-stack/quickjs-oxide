# Test262 dev support

`current.conf` is the single authoritative Test262 gate specification. It pins
the current profile, focused manifest and receipts, full-corpus metrics, line
counts, summaries, and SHA-256 hashes. The gate parses it as inert `key=value`
data; it is never sourced or evaluated by a shell.

Use the one parameterized entry point:

```sh
./scripts/test-test262.sh --check
./scripts/test-test262.sh --focused
TEST262_WORKERS=2 ./scripts/test-test262.sh --full
```

The repository keeps only the current profile and focused receipt plus the
small semantic ledgers used by runner unit tests. Earlier milestone profiles,
168 cohort-specific gates, result vectors, baselines, and former long-form
status ledgers are preserved in the public
[`test262-history-r3ed`](https://github.com/pocket-stack/quickjs-oxide/releases/tag/test262-history-r3ed)
release asset. Its authenticated inventory is in `archive/index.tsv`.

Official progress reports lead with full pass and eligible coverage. Runnable
pass rate is secondary. A new admission must be expressed in profile data and
must validate the correct negative phase/type and QuickJS diagnostic rule; the
central gate must not acquire cohort names or fixture-specific branches.
