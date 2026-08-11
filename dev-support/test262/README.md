# Test262 dev support

`current.conf` is the single authoritative Test262 gate specification. It pins
the current profile, focused manifest and receipts, full-corpus metrics, line
counts, summaries, SHA-256 hashes, and the current negative-diagnostic contract.
The gate parses it as inert `key=value` data; it is never sourced or evaluated
by a shell.

The focused manifest is a sorted set of source paths. Its receipt remains a
full `(path, variant)` vector, so one path may produce multiple metadata-driven
variants.

`negative-diagnostics.tsv` is a strict, source-authenticated overlay for
negative variants whose QuickJS failure reason and location are part of the
gate. Its path/variant rows carry the pinned source hash, phase/type, semantic
rule, exact QuickJS message, and an `exact` or `absent` location policy. The
runner validates the table and source metadata before scheduling tests. A
contracted variant passes only when phase, type, message, and location all
match; Test262 paths and source hashes remain dev-support data and never enter
the production parser.

`negative-diagnostic-exemptions.tsv` freezes the legacy variants admitted
before exact diagnostic contracts became mandatory. Every audited negative
variant must belong to exactly one of the two files: an exact contract or this
legacy phase/type-only ledger. Overlap and missing rows are fatal. New
admissions therefore extend the exact contract and cannot silently inherit the
old phase/type-only behavior; removing an exemption requires replacing it with
an exact contract.

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
status ledgers are preserved in the public `test262-history-*` release assets.
Their authenticated inventory is in `archive/index.tsv`.

The `$262` realm and agent host is excluded from default library, CLI, and
WASM builds. The central gate explicitly enables the non-default
`test262-host` feature when it builds `run-test262`. Feature-specific checks
can be run with `cargo test --locked --features test262-host --lib --bins`.

Official progress reports lead with full pass and eligible coverage. Runnable
pass rate is secondary. A new admission must be expressed in profile data and
must validate the correct negative phase/type and QuickJS diagnostic rule; the
central gate must not acquire cohort names or fixture-specific branches.
