# Binary-object boundary checker

The command is `./scripts/checks/check-binary-object-boundary.sh`.
Python 3.11 or newer is required; the checker uses only the standard library.

## Responsibilities

- `__main__.py`: candidate-root arguments, diagnostic rendering and exit status.
- `scan.py`: the explicit, ordered rule pipeline.
- `context.py`: observations, diagnostics and caches for one immutable candidate.
- `source.py`: source reading, Rust lexical masking, braced-item extraction and
  normalized evidence helpers. These are checker-specific, not a general Rust parser.
- `rules/`: checks grouped by codec surface, native plans, translation, scalar and
  ordinary publication, runtime protocols, coercion, receipts and shared transport.
- `evidence/`: reviewed expected shapes, fingerprints, operation inventories and
  evidence manifests. Rules copy mutable inventories before using them.
- `canaries/`: reduced-fixture construction, isolated source mutations, receipt
  forgery cases and expected diagnostics. Shell files orchestrate cases; Python
  files perform source rewrites. Pipe-delimited tables preserve the original
  escaped source fragments and their expected diagnostic identifiers.
- `tests/`: regression coverage for source handling and per-scan isolation.

Rules run in the order declared in `scan.py`: later rules consume observations
and authenticated source helpers established by earlier rules. Keep temporary
calculations local to a rule; put only cross-rule state on the context. Do not
reorder phases without checking those dependencies. Caches live on one context
and must never be reused across candidate roots or subsequent scans.

## Verification

```sh
# Production scan only; also useful against an isolated candidate tree.
./scripts/checks/check-binary-object-boundary.sh --scan-only .

# Production scan, clean-fixture checks, all mutations and CI integration checks.
./scripts/checks/check-binary-object-boundary.sh

# The existing focused receipt-forgery suite.
./scripts/checks/check-binary-object-boundary.sh --stage3i-receipt-canaries

PYTHONPATH=scripts/checks python3 -m unittest discover \
  -s scripts/checks/binary_object/tests
```

Independent mutation trees run with four workers by default. Set
`QUICKJS_OXIDE_BOUNDARY_JOBS=1` for serial execution or choose another positive
worker count. Every scheduled result is checked; a failed or escaped mutation
fails the command. The full suite includes all original mutation cases.

When changing a boundary, update its rule, reviewed evidence and corresponding
mutation cases together. Keep diagnostic identifiers stable. Do not update
fingerprints merely to silence a failed scan. The reduced-fixture marker still
requires an out-of-band matching token and cannot bypass checks in a normal root.

## Candidate A migration

The checker reads instructions and drafts from `engine/src/engine/code`, runtime values
from `engine/src/engine/value`, and public Context operations from `engine/src/engine/api`.
Production ownership scans and full mutation fixtures include adapters and
conformance. Mutation targets name the actual owner; the empty-atom case uses
`PrimitiveValue`, and Cargo target mutations operate on `Cargo.toml`.

The migration updates the pinned crate routing and the two publication methods'
crate-only visibility needed by the sibling Context API. The decoder and its
atom-bearing intermediate types remain private. Frozen test snapshots changed
only for equivalent Clippy cleanups (`is_none()` and a single binding in place of
a one-element loop); the raw48 assertions and expected behavior are unchanged.
These source snapshots are separate from historical Test262 receipts, which
retain their original source identity.
