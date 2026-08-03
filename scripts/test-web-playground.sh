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

if ! grep -Fqx 'quickjs-oxide = { path = "../.." }' web/wasm/Cargo.toml; then
  echo "web wrapper must path-depend on the repository's quickjs-oxide crate" >&2
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

const require = createRequire(import.meta.url);
const wrapperPath = path.resolve(
  process.cwd(),
  "target/web-playground-node/quickjs_oxide_web.js",
);
const { evaluate } = require(wrapperPath);

const examplesPath = path.resolve(process.cwd(), "web/site/examples.js");
const examplesSource = await readFile(examplesPath, "utf8");
const examplesModule = await import(
  `data:text/javascript;base64,${Buffer.from(examplesSource).toString("base64")}`
);
const expected = new Map([
  ["return-42", { kind: "number", text: "42" }],
  ["default-parameters", { kind: "number", text: "42" }],
  ["typed-array", { kind: "number", text: "42" }],
  ["resizable-array-buffer", { kind: "number", text: "42" }],
  ["uint8-codec", { kind: "number", text: "42" }],
  ["class", { kind: "number", text: "42" }],
  ["promise", { kind: "boolean", text: "true" }],
  ["weak-map", { kind: "number", text: "42" }],
  ["array-pipeline", { kind: "number", text: "42" }],
]);

assert.equal(examplesModule.EXAMPLES.length, expected.size);
for (const example of examplesModule.EXAMPLES) {
  assert.ok(
    expected.has(example.id),
    `missing expectation for playground example ${example.id}`,
  );
  const result = evaluate(example.source);
  assert.deepEqual(
    { ok: result.ok, kind: result.kind, text: result.text },
    { ok: true, ...expected.get(example.id) },
    `unexpected result for playground example ${example.id}`,
  );
}

const syntaxError = evaluate("function () {");
assert.equal(syntaxError.ok, false);
assert.equal(syntaxError.kind, "exception");
assert.match(syntaxError.text, /^SyntaxError:/);

console.log(
  `Node/WASM smoke: ${examplesModule.EXAMPLES.length} playground examples passed; quickjs-oxide returned 42`,
);
NODE
