# Test262 dev support

`current.conf` is the single authoritative Test262 gate specification. It pins
the current profile, focused manifest and receipts, full-corpus metrics, line
counts, summaries, SHA-256 hashes, and the current negative-diagnostic contract.
It also pins the exact admission catalog by repository path, line count, and
SHA-256.
The gate parses it as inert `key=value` data; it is never sourced or evaluated
by a shell.

The focused manifest is a sorted set of source paths. Its receipt remains a
full `(path, variant)` vector, so one path may produce multiple metadata-driven
variants.

`admissions.tsv` is the hash-pinned source of exact Module-goal graphs,
dependency-free module roots, `$262.agent` host paths, and supplemental feature
contracts. Its strict 16-column TSV schema records source hashes, complete
metadata shapes, graph edges and closure sizes, lookup priority, and host
cohort policy. Both coordinator and isolated worker parse and authenticate the
same file; malformed, unsorted, duplicate, open-graph, or checksum-drifted data
fails closed. The four cohort generators emit `--admissions` rows and compare
their owned group against this file in normal check mode.

`negative-diagnostics.tsv` is a strict, source-authenticated overlay for
negative variants whose QuickJS failure reason and location are part of the
gate. Its path/variant rows carry the pinned source hash, phase/type, semantic
rule, exact QuickJS message, and an `exact` or `absent` location policy. The
runner validates the table and source metadata before scheduling tests. A
contracted variant passes only when phase, type, message, and location all
match; Test262 paths and source hashes remain dev-support data and never enter
the production parser.

`negative-diagnostic-rules.tsv` is the complete registry for the semantic
`rule` column and names the corresponding pinned QuickJS parser anchor. The
authenticated audit tool rejects unknown or unused rules. Scheduled
differential CI replays every exact contract through pinned QuickJS using the
same Script/Module goal and strict-prefix policy and compares error type,
message, line, and column.

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
node scripts/audit-negative-diagnostics.mjs --suite /path/to/test262 \
  --qjs /path/to/pinned/qjs
```

For new Script-goal admissions, prepare a bytewise-sorted candidate TSV with
the header `path\tvariant\trule`, then generate source-authenticated rows with:

```sh
node scripts/audit-negative-diagnostics.mjs \
  --generate /path/to/candidates.tsv --output /path/to/contracts.tsv \
  --suite /path/to/test262 --qjs /path/to/pinned/qjs \
  --oxide target/debug/qjs
```

Generation first inserts a leading runtime sentinel and requires both engines
to report the same source diagnostic one line later, proving that parsing fails
before execution. It then succeeds only when the original pinned QuickJS and
Oxide diagnostics match exactly. The candidate file assigns reviewed semantic
rules explicitly; the tool never infers a rule from a fixture path or error
string. Module candidates still use the runner's module path rather than this
Script-only CLI helper. Scheduled differential CI runs the checked-in smoke
cohort plus runtime-deception, metadata, and A/A regression checks.

`--check` authenticates the pinned baseline and prints whether the working tree
is current or stale; staleness is explicit but does not fail fast CI. Focused
byte-for-byte replay refuses a stale source. A full run may produce a new
current-source receipt for promotion, but cannot authenticate against the old
baseline hashes. Promotion updates the pinned source commit and result hashes
only after report metadata matches the recomputed workspace fingerprint.

The repository keeps only the current profile and focused receipt plus the
small semantic ledgers used by runner unit tests. A fast inventory gate requires
every tracked `tests/test262-*` artifact to be referenced by the current spec,
runner, or a generator. Earlier milestone profiles, 168 cohort-specific gates,
result vectors, baselines, 313 superseded manifests, and former long-form status
ledgers are preserved in the public `test262-history-*` release assets. Their
authenticated inventory is in `archive/index.tsv`.

The `$262` realm and agent host is excluded from default library, CLI, and
WASM builds. The central gate explicitly enables the non-default
`test262-host` feature when it builds `run-test262`. Feature-specific checks
can be run with `cargo test --locked --features test262-host --lib --bins`.

Official progress reports lead with full pass and eligible coverage. Runnable
pass rate is secondary. A new admission must be expressed in profile data and
must validate the correct negative phase/type and QuickJS diagnostic rule; the
central gate must not acquire cohort names or fixture-specific branches.
