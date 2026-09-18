# Verify — B9: 超长原型链的 GET / `in` / instanceof 不再 abort

Task 928 (S3 P2/P3) originally used baseline `main@56faa471`, release build
(`cargo build --release --bin qjs`). Oracle:
`$QJS_ORACLE_CACHE/quickjs-2026-06-04/qjs` (QuickJS 2026-06-04).
Task 1030 rebased that result onto `main@49d1a299`; the rebase validation is
recorded below.

## Method

Build a plain chain `o = Object.create(o)` n times, then perform one
operation at the tip, per-process (exactly Scout P2/P3 repros).

## Baseline reproduction (pre-fix)

| operation | case | oxide exit | oxide result | pinned exit | pinned result |
|---|---|---:|---|---:|---|
| GET missing | n=20000 (`/tmp/b9/chain_get.js`) | **134 (SIGABRT)** | `thread 'main' has overflowed its stack` | 0 | `undefined` |
| `in` | n=20000 (`/tmp/b9/chain_in.js`) | **134 (SIGABRT)** | stack overflow abort | 0 | `false` |
| `instanceof F` | n=20000 (`/tmp/b9/chain_instanceof.js`) | 0 | `false` | 0 | `false` |
| GET inherited accessor | n=20000 (`/tmp/b9/chain_getter.js`) | **134 (SIGABRT)** | stack overflow abort | 0 | `got` |
| SET new property (ordinary chain) | n=20000/50000/100000 | 0 | `ok` | 0 | `ok` |
| SET new property, chain **with Proxy** forcing slow path | n=50000 | **134 (SIGABRT)** | stack overflow abort | 0 | `ok` |
| `for-in` enumerate | n=20000 | 0 | `0` | 0 | `0` |
| GET/`in` via 20000 empty-handler Proxies | n=20000 | 1 (catchable) | `InternalError: stack overflow` | 1 | `InternalError: stack overflow` |

Abort thresholds bisected in this worktree: GET last OK n=18000 / first
abort n=19000 (Scout: 18625→18750); `in` last OK 13000 / first abort 14000
(Scout: 13375→13562). Confirms the Scout numbers.

## Anchor analysis

- GET: `Runtime::internal_get` walks the prototype chain by tail self-call
  (`src/runtime/internal_methods.rs:821`, self-call at `:866`). Each link is
  a real Rust frame (own-property lookup, `internal_get_prototype_of`,
  descriptor match) → host-stack exhaustion.
- `in`: `Runtime::internal_has_property`
  (`src/runtime/internal_methods.rs:751`, self-call at `:780`), same shape.
- Global-binding/missing sentinel: `internal_get_or_missing`
  (`src/runtime/internal_methods.rs:878`, self-call at `:929`), same shape.
- `instanceof`: ordinary candidate-prototype walk is **already iterative**
  (`src/runtime.rs:7726-7736`) and bound-function chains trampoline in a
  `loop` (`src/runtime.rs:7652`); measured no abort at n=20000.
- SET: ordinary chains take an already-iterative fast path
  (`ordinary_set_fast_path_available` scans the chain in a `while`), but when
  the fast path is unavailable (a Proxy or typed array anywhere below the
  tip), `internal_set` reached the prototype tail by tail self-call
  (`src/runtime/internal_methods.rs`, pre-fix `:1110`) and aborted on long
  chains. `for-in` uses an explicit iterator.
- Deep Proxy forwarding is protected by the proxy-method stack guard and
  throws the same catchable InternalError as pinned — parity already holds.

## Upstream behavior (the parity target)

Pinned qjs returns normally at **n=1,000,000** for GET (`undefined`), `in`
(`false`), and instanceof (`false`). Upstream `JS_GetPropertyInternal` /
`JS_HasProperty` / ordinary `JS_IsInstanceOf` walk prototypes in a C `for`
loop — there is **no depth limit**; the only overflow upstream can hit is its
physical C-stack guard deep in unrelated recursion, which a chain walk never
consumes one frame per link of. Therefore the correct fix is **recursion →
iteration**, not a depth cap: a cap would reject inputs the pinned engine
accepts (uncatalogued deviation). An accessor found at the chain end must
still be invoked with the original receiver (`o.x` getter case).

## Plan

1. Convert `internal_get`, `internal_has_property`, and
   `internal_get_or_missing` prototype walks to `loop` while preserving:
   Proxy boundary (re-enters proxy intrinsic per hop), typed-array numeric
   index handling, accessor call with original receiver, Throw propagation.
2. Add Rust unit tests (long-chain GET/`in`/instanceof, inherited data +
   accessor, receiver identity, proxy-in-chain re-entry, throw propagation).
3. Mutate once to prove tests have teeth.
4. Re-diff against pinned qjs; full gate; do not touch receipts/hashes.

## Post-fix results

**Abort eliminated (red→green), byte-identical with pinned qjs:**

| case | n | oxide before | oxide after | pinned |
|---|---:|---|---|---|
| GET missing | 20,000 | exit 134 SIGABRT | exit 0 `undefined` | exit 0 `undefined` |
| GET missing | 1,000,000 | — | exit 0 `undefined` | exit 0 `undefined` |
| `in` | 20,000 | exit 134 SIGABRT | exit 0 `false` | exit 0 `false` |
| `in` | 1,000,000 | — | exit 0 `false` | exit 0 `false` |
| `instanceof F` | 20,000 / 1,000,000 | exit 0 `false` | exit 0 `false` | exit 0 `false` |
| inherited accessor GET (receiver `this` check) | 20,000 / 1,000,000 | exit 134 | exit 0 `got` | exit 0 `got` |
| edge bundle (null-terminal, mid-chain Proxy trap, inherited setter receiver) | 50,000 | — | exit 0, output `cmp`-identical | same |
| SET slow path (Proxy below tip; new prop, inherited setter, read-only reject, strict TypeError, trap count) | 50,000 | exit 134 SIGABRT | exit 0, output `cmp`-identical (`true:9|3|true|TypeError|1|7|42`) | same |
| SET slow path timing | 1,000,000 | — | exit 0, 255 ms (linear) | exit 0, 27 ms |
| `with(deepChain) missingVar` (get_or_missing → ReferenceError) | 50,000 | — | stderr `cmp`-identical | same |
| 20,000 nested empty-handler Proxies | 20,000 | exit 1 catchable InternalError | unchanged: exit 1 InternalError | exit 1 InternalError |

No deviations ledger entry needed: behavior is now byte-identical to the
pinned engine, which itself imposes **no** chain-depth limit.

**Mutation checks:** (1) restored the recursive tail self-call in
`internal_has_property` → `in_operator_on_long_chain_returns_false` aborted
the libtest process with SIGABRT; (2) restored the recursive
`internal_set` prototype tail → `set_walks_long_chain_iteratively_when_slow_path_forced`
aborted with SIGABRT (`has overflowed its stack`). Both prove the tests
catch the regressions; both mutations were reverted before the gate run.

**Test262 subset (no contract data changed):** `language/expressions/in`,
`language/expressions/instanceof`, `built-ins/Object/create`,
`built-ins/Proxy/{has,get,set}` = 471 files / **929 variants**.
Baseline (sha `f61afc…0333`) and final fixed (sha `03169d9e…73fc`) reports
are row-identical: **928 pass, 1 `unsupported-negative-provenance`
(`in/rhs-yield-absent-strict.js`, frozen bucket), 0 fail both before and
after** — zero regressions. The aborts require chain lengths (>14k) no
test262 case constructs, so the new coverage is the 9 Rust unit tests plus
the pinned-qjs CLI differentials above.

## Original task 928 files changed

- `src/runtime/internal_methods.rs` — `internal_get` (loop, receiver
  threaded across hops), `internal_has_property` (loop),
  `internal_get_or_missing` (loop), `internal_set` (loop over the prototype
  tail with a single tip-level fast-path verdict); 9 new unit tests.
- `findings/verify-B9-proto-chain.md` — this report.

## Task 1030 rebase onto `49d1a299`

The new baseline moved and redesigned the implementation rather than retaining
`src/runtime/internal_methods.rs`: ordinary GET is iterative in
`src/engine/object/ordinary.rs::prepare_ordinary_read_selected`, `in` is
iterative in `src/engine/object/internal_methods.rs::prepare_has_property`, SET
is iterative in `src/engine/object/ordinary/set.rs::State::select_walk_probe`,
and `instanceof` is driven by the loop in
`src/engine/builtins/function/instance.rs::finish`. The rebase therefore keeps
those production paths and ports the nine B9 regression tests to
`src/engine/object/internal_methods.rs`; it does not restore the deleted legacy
runtime module. The rebased implementation commit is `f0a1a922`.

Validation on 2026-09-17:

- `cargo test --locked --workspace --all-targets`: exit 0. The principal suites
  reported 2,287 library tests, 32 CLI tests, 907 passed plus one explicitly
  ignored oracle stress test, and 122 Test262-runner tests, all with zero
  failures.
- `cargo test --locked -p quickjs-oxide long_prototype_chain_tests --lib --
  --nocapture`: 9 passed, 0 failed.
- `cargo fmt --all -- --check`: exit 0.
- `rustup run 1.88.0 cargo clippy --locked --workspace --lib --bins -- -D
  warnings`: exit 0 (the CI-pinned Rust version).
- `./scripts/checks/check-rust-only.sh`: `rust-only gate passed: product paths
  and resolved Cargo dependencies contain no external QuickJS engine`.
- `python3 scripts/checks/check-source-layout.py`: `Source layout passed: 693
  reachable Rust files.` The old binary-object boundary checker named in the
  task context was intentionally removed by target-base commit `49d1a299`; the
  current CI runs this source-layout gate instead.
- `./scripts/test262/test-test262.sh --spec
  dev-support/test262/current.conf --check`: the spec and frozen receipts
  authenticated. As documented for this project state, the receipt source is
  stale (`baseline=f61afc...0333`, `current=990ee3...1762`), so it was not
  promoted or edited.
- Direct Test262 runner replay of the 471-file B9 manifest produced 929
  variants: 928 pass, one frozen `unsupported-negative-provenance`, zero true
  failures. After normalizing only the required engine fingerprint metadata,
  its row vector is byte-identical to task 928's fixed vector; both normalized
  TSV files have SHA-256
  `968206db6251b485798724c928ae9b187e569cc6fa3f07ccb1b7e4746d19a7d4`.
- Release CLI differential against the pinned QuickJS 2026-06-04 binary ran
  GET missing, `in`, `instanceof`, and inherited getter at 1,000,000 links,
  plus Proxy SET, Proxy GET/has, receiver identity, read-only assignment, and
  missing-binding cases at 50,000 links. All nine probes matched byte-for-byte
  in stdout and stderr and matched exit status; no probe aborted.

Rebased diff against `49d1a299`: 173 test lines in
`src/engine/object/internal_methods.rs` plus this standalone verification
report. No Test262 receipt, profile, admission, hash, or deviation ledger was
changed.

Delivery: branch `lfkdsk:fix/b9-long-prototype-chain`, PR
https://github.com/pocket-stack/quickjs-oxide/pull/28 targeting `main`. GitHub
reported the two-commit PR as open and mergeable after creation.
