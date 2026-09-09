# Development tools

Implementations have a single owner:

| Directory | Responsibility |
| --- | --- |
| `checks/` | Architecture and provenance checks, BC5 pin checks, oracle inventory, aggregate parity gate |
| `test262/` | Suite preparation, admission generators, diagnostics, metrics and receipt verification |
| `quickjs/` | Pinned reference builds, cache tests, differential fixtures and dynamic-import traces |
| `unicode/` | Unicode table generation and source-fingerprint verification |
| `web/` | Playground build, Node/browser checks and Pages deployment tooling |

Commands, CI and generated-file headers use these paths directly. There are no
compatibility entries at the old paths. `demo-42.sh` remains a small
repository-level example. New tools belong in their owner directory.

Shell scripts coordinate commands and temporary files. Python, Node and AWK
analysis lives in separate source files, including Unicode extraction, Test262
report validation, host metadata checks and playground assertions. Helpers stay
with their owner. The BC5 gates share `checks/lib/bc5-gate-primitives.mjs`; admission
generators share `test262/test262-admission-data.mjs`. Avoid a generic shared
utilities directory until unrelated owners actually need the same contract.

Handwritten Unicode algorithms live in `src/unicode*.rs`; generated production
tables live in `src/source/unicode/generated/unicode/`. Product builds consume those checked-in
tables without running generators or compiling the QuickJS reference. Test262
generated evidence remains in `dev-support/test262/generated/`.

Tool regression tests live beside the implementation they exercise. Engine unit
and integration tests retain the Cargo layout documented in `tests/README.md`.
See [the binary-object checker](checks/binary_object/README.md) for its rule,
evidence and mutation-test layout.

Historical Test262 receipts retain their original source commit, input list and
fingerprint. Current tool paths point to the owned, byte-identical fingerprint
and diagnostic-audit implementations. Current-run fingerprints cover
the relocated preparation and runner scripts plus the extracted report validator.
Historical receipt input paths remain historical data, not executable entry points.
