#!/usr/bin/env node

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import { parseTrace } from "./parse-quickjs-dynamic-import-trace.mjs";

function expectInvalid(text, pattern) {
  assert.throws(() => parseTrace(text), pattern);
}

const escaped = parseTrace("QJODI1\tT\t3:610962\t0:\t0\n");
assert.equal(escaped.length, 1);
assert.equal(escaped[0].root.utf8, "a\tb");
assert.equal(escaped[0].module.utf8, "");
expectInvalid("QJODI1\tT\t0:\t0:\t0", /not newline terminated/);
expectInvalid("QJODI1\tT\t0:\t2:00\t0\n", /byte length/);
expectInvalid("QJODI1\tX\t0:\n", /unknown record type/);
expectInvalid(Buffer.from([0x51, 0x4a, 0x4f, 0x44, 0x49, 0x31, 0x80, 0x0a]), /non-ASCII/);

if (process.argv.length === 2) {
  console.log("QJODI1 parser unit checks passed");
  process.exit(0);
}
if (process.argv.length !== 4) {
  console.error(
    "usage: test-parse-quickjs-dynamic-import-trace.mjs [TRACE ROOT_FILE]",
  );
  process.exit(2);
}

const tracePath = process.argv[2];
const rootFile = path.resolve(process.argv[3]);
const fixtureDir = path.dirname(rootFile);
const records = parseTrace(readFileSync(tracePath));

assert.equal(records.length, 17);
for (const record of records) {
  assert.equal(record.root.utf8, rootFile, `${record.kind} has wrong root identity`);
}

const normalizeRequests = records
  .filter((record) => record.kind === "normalize")
  .map((record) => record.request.utf8);
assert.deepEqual(normalizeRequests, [
  "bare.js",
  "./computed-block.js",
  "./computed-template.js",
  "./computed-nested.js",
  "./computed-invalid.js",
  "./computed-missing.js",
]);

const loaders = records.filter((record) => record.kind === "loader");
assert.equal(loaders.length, 6);
const loaderByPath = new Map(
  loaders.map((record) => [path.basename(record.effectivePath.utf8), record]),
);
for (const basename of [
  "bare.js",
  "computed-block.js",
  "computed-template.js",
  "computed-nested.js",
]) {
  assert.equal(loaderByPath.get(basename)?.outcome.utf8, "ok", basename);
  assert.equal(loaderByPath.get(basename)?.errno, 0, basename);
}
assert.equal(
  loaderByPath.get("computed-invalid.js")?.outcome.utf8,
  "compile_error",
);
assert.equal(loaderByPath.get("computed-invalid.js")?.errno, 0);
assert.equal(
  loaderByPath.get("computed-missing.js")?.outcome.utf8,
  "read_error",
);
assert.ok(loaderByPath.get("computed-missing.js")?.errno > 0);
assert.equal(loaderByPath.get("bare.js")?.request.utf8, "bare.js");
assert.equal(
  loaderByPath.get("bare.js")?.effectivePath.utf8,
  path.join(fixtureDir, "bare.js"),
);

const tlaByPath = new Map(
  records
    .filter((record) => record.kind === "tla")
    .map((record) => [path.basename(record.module.utf8), record.hasTla]),
);
assert.deepEqual(Object.fromEntries(tlaByPath), {
  "root.js": false,
  "bare.js": false,
  "computed-block.js": true,
  "computed-template.js": true,
  "computed-nested.js": false,
});

console.log(
  "QJODI1 trace semantics passed: computed requests and parser-derived TLA are attributable",
);
