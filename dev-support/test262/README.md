# Test262 dev support

`current.conf` is the single authoritative Test262 gate specification. It pins
the current profile, focused manifest and receipts, full-corpus metrics, line
counts, summaries, and SHA-256 hashes. The gate parses it as inert `key=value`
data; it is never sourced or evaluated by a shell.

The baseline also names the exact source commit and a canonical engine-semantics
fingerprint. The fingerprint hashes sorted repository paths and exact contents
for `Cargo.toml`, `Cargo.lock`, `src/**`, the active profile/upstream pins, and
the central preparation/gate scripts. It excludes generated caches, vendors,
historical vectors, receipts, and `current.conf` itself. The spec remains
separately authenticated, so excluding it avoids a circular checksum.

Use the one parameterized entry point:

```sh
./scripts/test-test262.sh --check
./scripts/test-test262.sh --focused
TEST262_WORKERS=2 ./scripts/test-test262.sh --full
```

`--check` authenticates the pinned baseline and prints whether the working tree
is current or stale; staleness is explicit but does not fail fast CI. Focused
byte-for-byte replay refuses a stale source. A full run may produce a new
current-source receipt for promotion, but cannot authenticate against the old
baseline hashes. Promotion updates the pinned source commit and result hashes
only after report metadata matches the recomputed workspace fingerprint.

The repository keeps only the current profile and focused receipt plus the
small semantic ledgers used by runner unit tests. Earlier milestone profiles,
168 cohort-specific gates, result vectors, baselines, and former long-form
status ledgers are preserved in the public
[`test262-history-r3ed`](https://github.com/pocket-stack/quickjs-oxide/releases/tag/test262-history-r3ed)
release asset. Its authenticated inventory is in `archive/index.tsv`.

The `$262` realm and agent host is excluded from default library, CLI, and
WASM builds. The central gate explicitly enables the non-default
`test262-host` feature when it builds `run-test262`. Feature-specific checks
can be run with `cargo test --locked --features test262-host --lib --bins`.

Official progress reports lead with full pass and eligible coverage. Runnable
pass rate is secondary. A new admission must be expressed in profile data and
must validate the correct negative phase/type and QuickJS diagnostic rule; the
central gate must not acquire cohort names or fixture-specific branches.
