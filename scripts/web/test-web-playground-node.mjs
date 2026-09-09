import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import { pathToFileURL } from "node:url";

const require = createRequire(import.meta.url);
const metricsModulePath = path.resolve(
  process.cwd(),
  "scripts/test262/current-test262-metrics.mjs",
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

const examplesPath = path.resolve(process.cwd(), "apps/web/site/examples.js");
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
