# quickjs-oxide

An independent Rust rewrite of QuickJS, targeting semantic feature parity with
the official **QuickJS 2026-06-04** release and its ES2025 behavior.

The `unsafe`-free engine is runnable but incomplete. Its strongest covered
slice is the shared ArrayBuffer/DataView/12-class TypedArray stack: resizable
buffers, fixed and length-tracking views, transfers, iteration, search,
mutation, sorting, species behavior, and the six Uint8Array base64/hex codecs.
The 91-tag global Test262 profile also admits `Proxy`, optional chaining,
Iterator Helpers, `globalThis`, and the implemented Promise surface through
checksum-bound audits. The complete conservative vector is 59,507/102,037
with 60,026 runnable variants. Modules, SharedArrayBuffer/Atomics, and broad
built-in coverage remain incomplete.
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
./scripts/test-test262-computed-property-names.sh
./scripts/test-test262-computed-property-names-global.sh --full
./scripts/test-test262-full.sh
./scripts/test-web-playground.sh
```

## License

[MIT](LICENSE). Third-party notices are retained in [NOTICE](NOTICE) and
[LICENSES](LICENSES/).
