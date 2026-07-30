# quickjs-oxide

An independent Rust rewrite of QuickJS, targeting semantic feature parity with
the official **QuickJS 2026-06-04** release and its ES2025 behavior.

The `unsafe`-free engine is runnable but incomplete. The current TypedArray
stack includes the shared 12-class kernel, in-place mutation, indexed search,
QuickJS-shaped `find`/`findIndex`/`findLast`/`findLastIndex`,
`every`/`some`, `forEach`, `reduce`/`reduceRight`, species-aware
`map`/`filter` and `slice`/`subarray`, plus non-species `with` and
`toReversed`, dedicated `join`/`toLocaleString`, inherited `toString`, and
QuickJS-shaped `sort`/`toSorted`, certified `entries`/`keys` iterators, and
pinned-QuickJS-certified static `of` and `from`. The global `TypedArray`,
`Proxy`, and `optional-chaining` feature tags are now admitted through
checksum-bound audits. The complete conservative vector is 56,526/102,037
with 57,045 runnable variants. Uint8Array codecs, modules,
SharedArrayBuffer/Atomics, and broad built-in coverage remain incomplete.
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
./scripts/test-test262-array-buffer.sh
./scripts/test-test262-data-view.sh
./scripts/test-test262-typed-array-core.sh
./scripts/test-test262-proxy.sh
./scripts/test-test262-optional-chaining.sh
./scripts/test-test262-full.sh
```

## License

[MIT](LICENSE). Third-party notices are retained in [NOTICE](NOTICE) and
[LICENSES](LICENSES/).
