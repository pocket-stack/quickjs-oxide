# quickjs-oxide

An independent Rust rewrite of QuickJS, targeting semantic feature parity with
the official **QuickJS 2026-06-04** release and its ES2025 behavior.

The `unsafe`-free engine is runnable but incomplete. Its strongest covered
slice is the ArrayBuffer/DataView/12-class TypedArray stack: resizable
buffers, fixed and length-tracking views, transfers, iteration, search,
mutation, sorting, species behavior, and the six Uint8Array base64/hex codecs.
QuickJS-shaped `Atomics` load/store/read-modify-write operations now work on
ordinary ArrayBuffer-backed integer TypedArrays. `SharedArrayBuffer` now has
QuickJS-shaped construction, grow, slice, and species behavior, plus DataView
and TypedArray views. A safe, sendable `SharedBufferHandle` can share its
backing across runtimes without exposing heap identities.
`Map`, `Set`, `WeakMap`, `WeakSet`, `WeakRef`, and `FinalizationRegistry` also
have QuickJS-shaped constructors, protocols, weak lifetimes, and ordered
runtime jobs.
`String.prototype.normalize` uses the pinned QuickJS Unicode 17 data for NFC,
NFD, NFKC, and NFKD; `localeCompare` matches QuickJS's non-Intl NFC/code-point
ordering.
The 130-tag global Test262 profile admits `Atomics.pause`,
`Array.prototype.flat`/`flatMap`, `WeakMap`, `WeakSet`, `WeakRef`, and
`FinalizationRegistry` alongside object rest, `DataView`, `Proxy`, optional
chaining, Iterator Helpers, `globalThis`, default parameters, and audited
binary-data and Promise surfaces through checksum-bound gates. Its latest
admission adds `Error.isError`, `RegExp.escape`, and
`TypedArray.prototype.at` without changing runtime semantics.
Its Test262 host provides real reentrant GC, recursive realm creation, and
defining-realm script evaluation. Script/eval parsing implements Annex B HTML
comments and QuickJS's no-op `debugger` semantics. Their negative cohorts and
the 25-path future-reserved negative cohort are globally admitted. Invalid
`enum`, `export`, and `extends`, malformed `import()`, and Script/Eval
`import.meta` are real syntax errors. Valid dynamic import remains typed
Unsupported, deferred through parsing and identifier/private-name resolution
so it cannot hide early errors. String inputs to direct and indirect `eval`
follow QuickJS-compatible WTF-8 semantics, preserving lone UTF-16 surrogates in
strings, templates, RegExp literals, and saved debug source. The last audited
full vector is 65,610 passes / 65,662 runnable / 102,037 total variants.
Modules, shared-memory Atomics, agents/waiters, `Atomics.waitAsync`, and broad
built-in coverage remain incomplete.
Pinned QuickJS is the test oracle, never a product dependency; detailed
bookkeeping lives in the status documents.

**[Open the browser playground →](https://pocket-stack.github.io/quickjs-oxide/)**
— it runs this Rust engine's actual WebAssembly build, not host `eval`. The
`SharedArrayBuffer views` example returns 42 through that build. The
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
- [Playground build and trust boundary](docs/playground.md)
- [Pinned upstream release](compat/upstream.toml)

## Verify

```sh
cargo test --locked --workspace --all-targets
./scripts/test-test262-current-global.sh --check  # latest frozen receipt
./scripts/test-test262-full.sh
./scripts/test-web-playground.sh
```

Historical focused gates and their checksum-bound receipts are indexed in the
status documents above.

## License

[MIT](LICENSE). Third-party notices: [NOTICE](NOTICE), [LICENSES](LICENSES/).
