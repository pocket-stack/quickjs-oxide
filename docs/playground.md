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

The WASM package also reports provenance for the exact loaded artifact:

```text
{ engine, crateVersion, quickjsTarget, buildCommit, canBlock }
```

The page renders that record rather than duplicating deployment state in its
JavaScript. The Pages workflow injects `github.sha` as
`QUICKJS_OXIDE_COMMIT`; local builds without that explicit variable are
labelled `local` so a dirty checkout is never presented as a clean commit. A
caller-supplied local identity is only a build label; the trusted public
identity boundary is the repository-owned Pages workflow.

## Build and test locally

Install the WebAssembly target and the exact CLI release used by the wrapper:

```bash
rustup target add --toolchain stable wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.126 --locked
```

Then build the deployable tree and run the Node/WebAssembly smoke:

```bash
./scripts/test-web-playground.sh
python3 -m http.server 4173 --directory target/pages
```

Open <http://localhost:4173/>. The build script writes only generated files
under `target/pages`; `web/site` remains the reviewable static source.
If a local shared Cargo target is busy, set `CARGO_TARGET_DIR` to a separate
build cache; the deployable tree still lands in `target/pages`.

The smoke executes all 15 curated examples in the real WebAssembly engine and
checks that each displayed expectation matches an independent expected-value
map. It also checks the engine version, compatibility target, build identity,
and host policy exported by the WASM package.
The set retains a no-Atomics `SharedArrayBuffer` views example and a `Shared
Atomics` example that stores 40, atomically adds 2, and loads 42 from the shared
backing. The engine implements synchronous `Atomics.wait`, but the browser
runtime intentionally keeps QuickJS's default `can_block=false`. The
`Atomics.wait host policy` example returns 42 only after observing the exact
TypeError boundary, without blocking the worker event loop. The Test262 agent
host remains outside R3dj, and pinned QuickJS has no `waitAsync` parity target.

For the same acceptance used by Pages, install the pinned browser dependency
and run Chromium against the final `target/pages` artifact:

```bash
npm ci
npx playwright install chromium
npm run test:browser
```

The browser gate waits for `Engine ready`, runs both the default function and
the `Atomics.wait` policy example, checks the displayed metadata, proves the
dedicated worker and WASM resources loaded, and rejects console, page, worker,
request, or HTTP errors. Set `QUICKJS_OXIDE_COMMIT` when testing an artifact
built with an explicit commit identity.

The page footer keeps the frozen global Test262 vector visibly marked
pre-parity and links the parity contract and Test262 ledger to the exact commit
reported by the loaded WASM package. A local build labelled `local` falls back
to the repository's `main` documentation links.

## Deployment

`.github/workflows/pages.yml` rebuilds the same artifact on `main`, runs the
Node anti-delegation/smoke gate and real Chromium acceptance, then uploads
`target/pages` through GitHub's official artifact action. A small
repository-owned client obtains the Actions OIDC token, creates the deployment
with the uploaded artifact ID and exact commit, and polls the Pages REST API.
Unlike the general deploy action, exhausting this bounded poll never cancels a
deployment that the Pages backend is still processing. Explicit terminal Pages
statuses still fail the workflow. Mock-server fixtures cover the create
payload, success, terminal failure, deferred polling, credential handling, and
the absence of cancellation requests. The crate and CLI `wasm-bindgen`
versions and Playwright dependency are pinned so the browser proof cannot
silently drift. The site also ships a project-owned social preview image; it
contains no runtime claim beyond the visible pre-parity target.

The deployable index references a fully content-addressed asset chain:
index to app, app to worker, and worker to the generated glue and WASM. CSS and
example data use the same scheme. This prevents a Pages CDN edge from combining
new HTML or JavaScript with a stale fixed-path WASM response while a deployment
is propagating.

After deployment, a separate read-only job runs `scripts/test-live-pages.mjs`
without Pages-write or OIDC permissions, and it passes no token to the
verifier. It polls the cache-busted public index with bounded backoff. Only
after the index matches the build job's SHA-256 does it follow the
content-addressed app, worker, glue, and `application/wasm` references; every
downloaded filename must match its bytes, and the glue/WASM digests must also
match the build outputs.
No downloaded code runs before that complete chain is authenticated. The
binding and WASM then run in a credential-scrubbed isolated process group with
a hard timeout and temporary-file cleanup. If the non-cancelling deployment
client exhausts its polling window while the Pages backend is still publishing,
this read-only verifier continues waiting for the exact artifact instead of
accepting an inconsistent or older deployment. The gate accepts only when the
package reports the workflow's exact commit and evaluates a JavaScript function
to the number 42; it never sends that example to host `eval` or `Function`. The
Chromium gate exercises the same verifier against a local artifact server,
including hash tamper rejection, bounded process-tree termination,
credential-scrubbing, cleanup, and stale-deployment retry fixtures.

The playground is a milestone view of an incomplete engine, not a claim of
complete ECMAScript support. The Test262 scoreboard remains the compatibility
authority.
