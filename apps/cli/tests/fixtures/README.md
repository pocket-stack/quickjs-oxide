# Fixture provenance

`inputs/` contains authored JS, module, and C probes. `expected/` contains frozen
observations produced with the pinned QuickJS release identified in each file
name (or its parent suite). These files are shared by Rust integration tests,
reference-engine scripts, and binary-object contract checks, so they remain in
the shared fixture tree rather than belonging to the oracle Rust target alone.

The move to separate directories preserves every input and output byte. Paths in
registries and callers change; the fixture checksums remain the same.

- `dev-support/quickjs-fixture-gates.tsv` records JS probe inputs, expected outputs,
  checksums, and driver modes. Run `scripts/quickjs/test-quickjs-fixtures.sh --validate`
  to authenticate the registry, or `--all` to execute it with the pinned engine.
- `dev-support/quickjs-c-oracles.tsv` records C probes and expected transcripts.
  Run `scripts/quickjs/test-quickjs-c-oracles.sh --validate` to authenticate them, or
  `--check` to build and compare them with the reference engine.
- Dedicated host and dynamic-import probes retain their `scripts/test-*.sh`
  drivers. Unicode C probes support the existing Unicode generation tools.

To update a reference observation, use its driver with the pinned engine, inspect
the behavior change, then update the expected output and its registry checksum
in the same change. Never silently record new expected output during a test run.
Generated Test262 metadata belongs in `dev-support/test262/generated/`, not here.
