# quickjs-oxide

An independent Rust rewrite of QuickJS, targeting semantic feature parity with
the official **QuickJS 2026-06-04** release and its ES2025 behavior.

The `unsafe`-free engine is runnable but incomplete. Its strongest covered
slice is the shared ArrayBuffer/DataView/12-class TypedArray stack: resizable
buffers, fixed and length-tracking views, transfers, iteration, search,
mutation, sorting, species behavior, and the six Uint8Array base64/hex codecs.
`Map`, `Set`, `WeakMap`, `WeakSet`, `WeakRef`, and `FinalizationRegistry` also
have QuickJS-shaped constructors, protocols, weak lifetimes, and ordered
runtime jobs.
`String.prototype.normalize` uses the pinned QuickJS Unicode 17 data for NFC,
NFD, NFKC, and NFKD; `localeCompare` matches QuickJS's non-Intl NFC/code-point
ordering.
The 122-tag global Test262 profile admits `WeakMap`, `WeakSet`, `WeakRef`, and
`FinalizationRegistry` alongside object rest, `DataView`, `Proxy`, optional
chaining, Iterator Helpers, `globalThis`, default parameters, and the
implemented binary-data and Promise surfaces through checksum-bound audits.
Its Test262 host provides real reentrant GC, recursive realm creation, and
defining-realm script evaluation. The last audited full vector is
65,280/102,037 with 65,406 runnable variants.
Modules, SharedArrayBuffer/Atomics, and broad built-in coverage remain
incomplete.
Pinned QuickJS is the test oracle, never a product dependency; detailed
bookkeeping lives in the status documents.

**[Open the browser playground →](https://pocket-stack.github.io/quickjs-oxide/)**
— it runs this Rust engine's actual WebAssembly build, not host `eval`. The
playground is a pre-parity milestone, not a Feature Parity claim.

## Try it

Rust 1.85 or newer is required.

```sh
git clone https://github.com/pocket-stack/quickjs-oxide.git
cd quickjs-oxide
./scripts/demo-42.sh  # 42
cargo run --quiet --bin qjs -- --print-result -e \
  '(function (a) { return a + 1; })(41)'  # 42
```

## Status

- [Implementation status and milestone ledger](docs/status.md)
- [Pinned Test262 progress baseline](docs/test262.md)
- [Parity acceptance contract](docs/parity.md)
- [Pinned upstream release](compat/upstream.toml)

## Verify

```sh
cargo test --locked --workspace --all-targets
./scripts/test-test262-weak-collections.sh
./scripts/test-test262-weak-collections-global.sh
./scripts/test-test262-weak-ref-finalization.sh
./scripts/test-test262-weak-ref-finalization-global.sh
./scripts/test-host-gc-reentrant-oracle.sh --oxide
./scripts/test-test262-host-gc.sh
./scripts/test-test262-host-gc-global.sh
./scripts/test-test262-create-realm.sh
./scripts/test-test262-eval-script.sh
./scripts/test-test262-realm-hosts-global.sh
./scripts/test-test262-string-locale-compare.sh
./scripts/test-test262-current-global.sh
./scripts/test-test262-full.sh
./scripts/test-web-playground.sh
```

## License

[MIT](LICENSE). Third-party notices are retained in [NOTICE](NOTICE) and
[LICENSES](LICENSES/).
