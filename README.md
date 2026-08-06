# quickjs-oxide

An independent Rust rewrite of QuickJS, targeting semantic feature parity with
the official **QuickJS 2026-06-04** release and its ES2025 behavior.

The `unsafe`-free engine is runnable, but it is not at Feature Parity yet. Its
strongest implemented slices cover ArrayBuffer/DataView/TypedArray,
SharedArrayBuffer and Atomics, collections and weak references, Promise jobs,
Unicode normalization, and a growing parser/VM surface.

The latest conformance milestone globally admits eight already-implemented
Test262 leaf features: `Array.prototype.values`, `Object.is`, `caller`,
`for-of`, `json-superset`, `proxy-missing-checks`, `stable-array-sort`, and
`stable-typedarray-sort`. Their complete union is 46 paths / 69 variants:
pinned QuickJS passes 69/69, while Oxide gains 65 global passes and keeps four
`destructuring-assignment` co-feature variants fail-closed. This is a
profile-only admission; the full Feature Parity goal remains unchanged.

The latest audited global vector is 67,878 passes / 67,930 runnable / 102,037
total variants. Detailed hashes, partitions, and historical bookkeeping live
in the status documents below.

**[Open the browser playground →](https://pocket-stack.github.io/quickjs-oxide/)**
— it runs this Rust engine's actual WebAssembly build, not host `eval`. The
page reports its exact build commit, QuickJS target, and non-blocking browser
host policy. Curated examples include a function returning 42 and the
`Atomics.wait` host-policy boundary. The playground is a pre-parity milestone,
not a Feature Parity claim.

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
- [Playground build and trust boundary](docs/playground.md)
- [Pinned upstream release](compat/upstream.toml)

## Verify

```sh
cargo test --locked --workspace --all-targets
./scripts/test-test262-current-global.sh --check  # latest frozen receipt
./scripts/test-test262-full.sh
./scripts/test-web-playground.sh
npm ci && npx playwright install chromium && npm run test:browser
```

Historical focused gates and their checksum-bound receipts are indexed in the
status documents above.

## License

[MIT](LICENSE). Third-party notices: [NOTICE](NOTICE), [LICENSES](LICENSES/).
