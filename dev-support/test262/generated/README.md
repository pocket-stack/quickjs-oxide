# Generated conformance metadata

This directory holds the active generated Test262 manifests, source inventories,
closure/edge tables, and diagnostic ledgers formerly stored at `tests/test262-*`.
The relocation preserves their bytes, source hashes, and pinned corpus revision.
They describe conformance inputs; they are not ordinary Cargo test targets.

Their producer scripts are `scripts/generate-test262-*.mjs`. Match a file's
`test262-<topic>` prefix to its producer (for example `module-default-a`,
`module-static-negative-a`, or `import-meta-a`). Existing producer `--check` modes
verify the frozen output without rewriting it. Use the pinned corpus prepared by
the conformance tooling, and inspect generator usage before regenerating files.
New outputs from these producers use this directory too.

`scripts/checks/check-test262-artifact-inventory.mjs` checks that tracked artifacts have
consumers. `scripts/test262/test-test262.sh --spec dev-support/test262/current.conf --check`
authenticates the current conformance spec and receipts. A structural change can
make a historical engine fingerprint stale without invalidating those receipts;
only a new conformance run establishes results for the new source revision.
