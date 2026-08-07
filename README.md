# quickjs-oxide

An independent Rust rewrite of QuickJS, targeting semantic feature parity with
the official **QuickJS 2026-06-04** release and its ES2025 behavior.

The `unsafe`-free engine is runnable, but it is not at Feature Parity yet. Its
strongest implemented slices cover ArrayBuffer/DataView/TypedArray,
SharedArrayBuffer and Atomics, collections and weak references, Promise jobs,
Unicode normalization, a synchronous static-module graph slice, and a growing
parser/VM surface.

The admitted R3dy-A milestone adds a Context-local source loader and cache,
iterative SCC linking/evaluation, side-effect and direct named imports, and
read-only live import bindings. Its source-authenticated dependency-graph gate
covers four roots and nine files; pinned QuickJS and Oxide both pass 4/4.
Default and namespace imports, complete indirect/star export resolution,
module namespace exotic objects, `import.meta`, attributes, and top-level
await remain fail-closed. The full Feature Parity goal is unchanged.

The latest admitted canonical vector is 68,108 passes / 68,160 runnable /
102,037 total variants. R3dy-A adds exactly four module-graph passes over
R3dx, leaves the other 102,033 variants unchanged, and has two byte-identical
full replays. Detailed hashes and historical bookkeeping live in the status
documents below.

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
./scripts/test-test262-module-loader-linker-a.sh --check
./scripts/test-test262-full.sh
./scripts/test-web-playground.sh
npm ci && npx playwright install chromium && npm run test:browser
```

Historical focused gates and their checksum-bound receipts are indexed in the
status documents above.

## License

[MIT](LICENSE). Third-party notices: [NOTICE](NOTICE), [LICENSES](LICENSES/).
