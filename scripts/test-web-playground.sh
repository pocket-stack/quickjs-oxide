#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
wasm_target="wasm32-unknown-unknown"
wasm_stem="quickjs_oxide_web"
cargo_target_dir="${CARGO_TARGET_DIR:-${repo_root}/target}"
if [[ "${cargo_target_dir}" != /* ]]; then
  cargo_target_dir="${repo_root}/${cargo_target_dir}"
fi
wasm_file="${cargo_target_dir}/${wasm_target}/web/${wasm_stem}.wasm"
node_dir="${repo_root}/target/web-playground-node"

cd "${repo_root}"

if ! command -v rg >/dev/null 2>&1; then
  echo "ripgrep (rg) is required for the playground anti-delegation gate" >&2
  exit 1
fi

if ! grep -Fqx \
  'quickjs-oxide = { path = "../..", default-features = false }' \
  web/wasm/Cargo.toml; then
  echo "web wrapper must path-depend on quickjs-oxide without dev-support features" >&2
  exit 1
fi

if ! grep -Fqx 'wasm-bindgen = "=0.2.126"' web/wasm/Cargo.toml \
  || ! grep -Fqx \
    'js-sys = { version = "=0.3.103", default-features = false }' \
    web/wasm/Cargo.toml; then
  echo "web bindings must stay exactly pinned with js-sys unsafe-eval disabled" >&2
  exit 1
fi

rust_host_pattern='js_sys::(eval|Function)|Function::new'
browser_host_pattern='(^|[^.$[:alnum:]_])eval[[:space:]]*[(]|(globalThis|window|self)[.]eval[[:space:]]*[(]|new[[:space:]]+Function[[:space:]]*[(]|(^|[^.$[:alnum:]_])Function[[:space:]]*[(]'
if rg -n "${rust_host_pattern}" web/wasm \
  || rg -n --glob '!pkg/**' "${browser_host_pattern}" web/site; then
  echo "playground source must not delegate evaluation to the browser host" >&2
  exit 1
fi

./scripts/build-web-playground.sh

rm -rf "${node_dir}"
mkdir -p "${node_dir}"
wasm-bindgen \
  "${wasm_file}" \
  --out-dir "${node_dir}" \
  --out-name "${wasm_stem}" \
  --target nodejs \
  --no-typescript

node --input-type=module <<'NODE'
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import { pathToFileURL } from "node:url";

const require = createRequire(import.meta.url);
const metricsModulePath = path.resolve(
  process.cwd(),
  "scripts/current-test262-metrics.mjs",
);
const { parseCurrentTest262Metrics } = await import(
  pathToFileURL(metricsModulePath)
);
const currentSpec = await readFile(
  path.resolve(process.cwd(), "dev-support/test262/current.conf"),
  "utf8",
);
const currentMetrics = parseCurrentTest262Metrics(currentSpec);
const renderedIndex = await readFile(
  path.resolve(process.cwd(), "target/pages/index.html"),
  "utf8",
);
assert.ok(renderedIndex.includes(currentMetrics.primaryText));
assert.ok(renderedIndex.includes(currentMetrics.detailText));
assert.throws(
  () => parseCurrentTest262Metrics(`${currentSpec}\nfull_passes=1\n`),
  /duplicate Test262 spec key full_passes/u,
);
assert.throws(
  () => parseCurrentTest262Metrics(
    currentSpec.replace(
      `pass=${currentMetrics.fullPasses}`,
      `pass=${currentMetrics.fullPasses - 1}`,
    ),
  ),
  /summaries disagree with the official metrics/u,
);

const wrapperPath = path.resolve(
  process.cwd(),
  "target/web-playground-node/quickjs_oxide_web.js",
);
const { engine_metadata: engineMetadata, evaluate } = require(wrapperPath);

const examplesPath = path.resolve(process.cwd(), "web/site/examples.js");
const examplesSource = await readFile(examplesPath, "utf8");
const examplesModule = await import(
  `data:text/javascript;base64,${Buffer.from(examplesSource).toString("base64")}`
);
const expected = new Map([
  ["return-42", { kind: "number", text: "42" }],
  ["default-parameters", { kind: "number", text: "42" }],
  ["typed-array", { kind: "number", text: "42" }],
  ["atomics-non-shared", { kind: "number", text: "42" }],
  ["resizable-array-buffer", { kind: "number", text: "42" }],
  ["shared-array-buffer", { kind: "number", text: "42" }],
  ["shared-atomics", { kind: "number", text: "42" }],
  ["atomics-wait-policy", { kind: "number", text: "42" }],
  ["uint8-codec", { kind: "number", text: "42" }],
  ["unicode-strings", { kind: "number", text: "42" }],
  ["class", { kind: "number", text: "42" }],
  ["promise", { kind: "boolean", text: "true" }],
  ["weak-map", { kind: "number", text: "42" }],
  ["weak-ref", { kind: "number", text: "42" }],
  ["array-pipeline", { kind: "number", text: "42" }],
]);

assert.equal(examplesModule.EXAMPLES.length, expected.size);
for (const example of examplesModule.EXAMPLES) {
  assert.ok(
    expected.has(example.id),
    `missing expectation for playground example ${example.id}`,
  );
  assert.deepEqual(
    example.expected,
    expected.get(example.id),
    `displayed expectation drifted for playground example ${example.id}`,
  );
  const result = evaluate(example.source);
  assert.deepEqual(
    { ok: result.ok, kind: result.kind, text: result.text },
    { ok: true, ...expected.get(example.id) },
    `unexpected result for playground example ${example.id}`,
  );
}

const metadata = engineMetadata();
assert.deepEqual(
  {
    engine: metadata.engine,
    crateVersion: metadata.crateVersion,
    quickjsTarget: metadata.quickjsTarget,
    buildCommit: metadata.buildCommit,
    canBlock: metadata.canBlock,
  },
  {
    engine: "quickjs-oxide",
    crateVersion: "0.0.1",
    quickjsTarget: "QuickJS 2026-06-04",
    buildCommit:
      process.env.QUICKJS_OXIDE_COMMIT || process.env.GITHUB_SHA || "local",
    canBlock: false,
  },
);

const qjsHostIsolation = evaluate(
  '(typeof print) + "|" + (typeof console)',
);
assert.deepEqual(
  {
    ok: qjsHostIsolation.ok,
    kind: qjsHostIsolation.kind,
    text: qjsHostIsolation.text,
  },
  { ok: true, kind: "string", text: "undefined|undefined" },
);

const evalVarDestructuring = evaluate(`
  (function () {
    eval("var { answer = function () { return 42; } } = {};");
    return answer.name === "answer" ? answer() : 0;
  })()
`);
assert.deepEqual(
  {
    ok: evalVarDestructuring.ok,
    kind: evalVarDestructuring.kind,
    text: evalVarDestructuring.text,
  },
  { ok: true, kind: "number", text: "42" },
);

const deepYieldStar = evaluate(`
  (function () {
    function* chain(depth) {
      return yield* (depth ? chain(depth - 1) : [42]);
    }
    return chain(20).next().value;
  })()
`);
assert.deepEqual(
  { ok: deepYieldStar.ok, kind: deepYieldStar.kind, text: deepYieldStar.text },
  { ok: true, kind: "number", text: "42" },
);

const caughtYieldStarOverflow = evaluate(`
  (function () {
    function* chain(depth) {
      return yield* (depth ? chain(depth - 1) : [42]);
    }
    var observed;
    try {
      chain(1000).next();
      observed = "missing";
    } catch (error) {
      observed = error.name + ":" + error.message;
    }
    return observed + "|" + 6 * 7;
  })()
`);
assert.deepEqual(
  {
    ok: caughtYieldStarOverflow.ok,
    kind: caughtYieldStarOverflow.kind,
    text: caughtYieldStarOverflow.text,
  },
  { ok: true, kind: "string", text: "InternalError:stack overflow|42" },
);

const syntaxError = evaluate("function () {");
assert.equal(syntaxError.ok, false);
assert.equal(syntaxError.kind, "exception");
assert.match(syntaxError.text, /^SyntaxError:/);

console.log(
  `Node/WASM smoke: ${examplesModule.EXAMPLES.length} playground examples, current Test262 metrics, and build metadata passed; direct eval and quickjs-oxide returned 42; deep yield-star overflow stayed catchable`,
);
NODE
