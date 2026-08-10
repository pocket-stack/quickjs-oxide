# quickjs-oxide

An unsafe-free Rust rewrite of QuickJS, targeting semantic Feature Parity with
the official **QuickJS 2026-06-04** release and its ES2025 behavior.

The engine is runnable, but it is not at Feature Parity yet. Implemented slices
include binary data and Atomics, collections and weak references, Promise jobs,
Unicode behavior, and synchronous static-module graphs with default exports,
live namespace objects, and core `import.meta` semantics. Dynamic import,
import attributes, top-level await, and QuickJS host-populated `import.meta`
properties remain parity work.

The authoritative R3ef-B Test262 baseline records **69,763 full-corpus passes
out of 102,037 variants (68.370%)**, with **69,813 eligible variants
(68.419%)**. The 69,763 / 69,813 runnable pass rate (99.928%) is a secondary
quality measure, not the headline compatibility metric.

**[Open the browser playground →](https://pocket-stack.github.io/quickjs-oxide/)**
— it runs this Rust engine's actual WebAssembly build, not host `eval`. The
page reports its exact build commit, QuickJS target, and browser host policy.
Its curated Script-goal examples include a function returning 42 and the
`Atomics.wait` boundary. The playground is a pre-parity milestone, not a
Feature Parity claim.

## Try it

Rust 1.88 or newer is required.

```sh
git clone https://github.com/pocket-stack/quickjs-oxide.git
cd quickjs-oxide
./scripts/demo-42.sh  # 42
cargo run --quiet --bin qjs -- --print-result -e \
  '(function (a) { return a + 1; })(41)'  # 42
```

## Status

- [Current implementation status](docs/status.md)
- [Pinned Test262 baseline and metric definitions](docs/test262.md)
- [Parity acceptance contract](docs/parity.md)
- [Playground build and trust boundary](docs/playground.md)
- [Pinned upstream release](compat/upstream.toml)
- [Test262 gate data and archived history](dev-support/test262/README.md)

## Verify

```sh
cargo test --locked --workspace --all-targets
./scripts/test-test262.sh --check
./scripts/test-test262.sh --focused
TEST262_WORKERS=2 ./scripts/test-test262.sh --full
./scripts/test-web-playground.sh
npm ci && npx playwright install chromium && npm run test:browser
```

## License

[MIT](LICENSE). Third-party notices: [NOTICE](NOTICE), [LICENSES](LICENSES/).
