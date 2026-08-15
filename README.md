# quickjs-oxide

An unsafe-free Rust rewrite of QuickJS, targeting semantic Feature Parity with
the official **QuickJS 2026-06-04** release and its ES2025 behavior.

The engine is runnable, but it is not at Feature Parity yet. Implemented slices
include binary data and Atomics, collections and weak references, Promise jobs,
Unicode behavior, class elements, and static-module graphs with live bindings,
top-level await, import attributes, JSON modules, `import.meta`, and dynamic
`import()`. The Rust embedding API and file-module CLI also expose QuickJS's
canonical, host-populated `import.meta` object. JavaScript-valued module-host
errors retain their exact values and object/Symbol identity. Context-aware
module-host callbacks can synchronously compile and return same-Context
modules, including nested loader re-entry protected by a finite host-stack
budget. Parse-in-progress module identities and source-order request prefixes
are visible to those callbacks in QuickJS order, with unsafe failed identities
represented deterministically. QuickJS extended-JSON text modules are
supported. Script and ECMAScript module embedding accept byte-exact source
buffers, including module-loader payloads; byte-oriented JSON-module and CLI
loading plus the remaining conformance gaps are still parity work.

<!-- current-test262-metrics:start -->
The authoritative R3ev Test262 baseline records **79,475 full-corpus passes
out of 102,037 variants (77.888%)**, with **79,525 eligible variants
(77.937%)**. The 79,475 / 79,525 runnable pass rate (99.937%) is a secondary
quality measure, not the headline compatibility metric.
<!-- current-test262-metrics:end -->

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
# File modules: -m/--module, --script, .mjs and source detection
cargo run --quiet --bin qjs -- examples/module-42.mjs  # 42
```

The file loader follows QuickJS policy: `.json` is strict JSON,
`type: "json5"` selects QuickJS extended JSON for any suffix, and a `.json5`
file without that attribute remains a JavaScript module.

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
