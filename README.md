# quickjs-oxide

An independent Rust rewrite of QuickJS, targeting semantic feature parity with
the official **QuickJS 2026-06-04** release and its ES2025 behavior.

The `unsafe`-free engine is runnable, but it is not at Feature Parity yet. Its
strongest implemented slices cover ArrayBuffer/DataView/TypedArray,
SharedArrayBuffer and Atomics, collections and weak references, Promise jobs,
Unicode normalization, and a growing parser/VM surface.

Building on 16 previously admitted agent paths, the latest conformance
milestone adds 22 source-audited bounded `Atomics.wait` / `notify` paths: all
44 variants pass. In total, 38/59 paths are admitted; the other 21 paths / 42
variants remain fail-closed globally, although scoped gates now pass the 17
wake/count/location paths and four FIFO paths awaiting global promotion.
Modules, broad built-ins, and the complete embedding/tooling surface remain
unfinished. Pinned QuickJS has no `Atomics.waitAsync`, so it is outside this
rewrite's parity target.

The latest audited global vector is 66,552 passes / 66,604 runnable / 102,037
total variants, with zero regressions from the prior milestone. Detailed
implementation and Test262 bookkeeping live in the status documents below.

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
