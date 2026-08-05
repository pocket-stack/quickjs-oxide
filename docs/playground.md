# Browser playground

The public playground at
<https://pocket-stack.github.io/quickjs-oxide/> runs quickjs-oxide itself. The
Rust crate is compiled to WebAssembly, loaded in the browser, and given each
editor submission through the public `Runtime` and `Context` APIs. It does not
delegate JavaScript execution to the browser's `eval` or `Function`
implementations.

Each run gets a fresh runtime and realm. The WebAssembly boundary returns a
small record:

```text
{ ok: boolean, kind: string, text: string }
```

`kind` identifies the JavaScript completion type on success, or
`exception`/`engine-error` on failure. Pending jobs created by the script are
drained before the result is returned.

## Build and test locally

Install the WebAssembly target and the exact CLI release used by the wrapper:

```bash
rustup target add --toolchain stable wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.126 --locked
```

Then build the deployable tree and run the Node/WebAssembly `return 42` smoke
test:

```bash
./scripts/test-web-playground.sh
python3 -m http.server 4173 --directory target/pages
```

Open <http://localhost:4173/>. The build script writes only generated files
under `target/pages`; `web/site` remains the reviewable static source.
If a local shared Cargo target is busy, set `CARGO_TARGET_DIR` to a separate
build cache; the deployable tree still lands in `target/pages`.

The smoke executes all 14 curated examples in the real WebAssembly engine.
The set retains a no-Atomics `SharedArrayBuffer` views example and a `Shared
Atomics` example that stores 40, atomically adds 2, and loads 42 from the shared
backing. The engine implements synchronous `Atomics.wait`, but the browser
runtime intentionally keeps QuickJS's default `can_block=false`; a playground
worker therefore reports the matching TypeError instead of blocking its event
loop. The Test262 agent host remains outside R3dj, and pinned QuickJS has no
`waitAsync` parity target.

## Deployment

`.github/workflows/pages.yml` rebuilds the same artifact on `main`, runs the
smoke and anti-delegation gates, uploads `target/pages`, and deploys it through
GitHub's official Pages actions. The crate and CLI `wasm-bindgen` versions are
pinned together so generated glue cannot silently drift.

The playground is a milestone view of an incomplete engine, not a claim of
complete ECMAScript support. The Test262 scoreboard remains the compatibility
authority.
