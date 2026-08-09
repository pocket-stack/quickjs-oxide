# quickjs-oxide

An unsafe-free Rust rewrite of QuickJS, targeting semantic Feature Parity with
the official **QuickJS 2026-06-04** release and its ES2025 behavior.

The engine is runnable, but it is not at Feature Parity yet. Implemented slices
include binary data and Atomics, collections and weak references, Promise jobs,
Unicode behavior, and synchronous static-module graphs with default exports,
live namespace objects, and core `import.meta` semantics. Dynamic import,
import attributes, top-level await, and QuickJS host-populated `import.meta`
properties remain parity work.

The latest frozen Test262 vector is **68,209 passes / 68,261 runnable /
102,037 total variants**. R3eb-A passes its combined 65-variant default-module
and `import.meta` cohort; 64 are new canonical gains, the other 101,973 rows are
byte-identical to R3dz-A, and there are zero regressions. Detailed receipts and
historical bookkeeping live in the status documents below.

**[Open the browser playground →](https://pocket-stack.github.io/quickjs-oxide/)**
— it runs this Rust engine's actual WebAssembly build, not host `eval`. The
page reports its exact build commit, QuickJS target, and browser host policy.
Its curated Script-goal examples include a function returning 42 and the
`Atomics.wait` boundary. The playground is a pre-parity milestone, not a
Feature Parity claim.

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
