#!/usr/bin/env node

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const generator = join(root, "scripts/generate-test262-tla-b.mjs");
const suiteArguments = process.argv.slice(2);
const protectedFiles = [
  "dev-support/test262/current.conf",
  "dev-support/test262/admissions.tsv",
  "dev-support/test262/negative-diagnostics.tsv",
  "dev-support/test262/negative-diagnostic-rules.tsv",
  "dev-support/test262/negative-diagnostic-exemptions.tsv",
  "compat/test262-oxide.conf",
  "tests/test262-class-private-callables-b.txt",
].map((relativePath) => join(root, relativePath));

function run(arguments_, expectedStatus = 0) {
  const result = spawnSync(process.execPath, [generator, ...suiteArguments, ...arguments_], {
    cwd: root,
    encoding: "utf8",
  });
  assert.equal(result.signal, null);
  assert.equal(result.status, expectedStatus, result.stderr);
  return result;
}

const before = new Map(protectedFiles.map((path) => [path, readFileSync(path)]));

const admissions = run(["--admissions"]).stdout.trimEnd().split("\n");
assert.equal(admissions.length, 223);
const admissionKinds = new Map();
for (const line of admissions) {
  const fields = line.split("\t");
  assert.equal(fields.length, 16);
  admissionKinds.set(fields[0], (admissionKinds.get(fields[0]) ?? 0) + 1);
}
assert.deepEqual(admissionKinds, new Map([
  ["graph-file", 21],
  ["graph-request", 18],
  ["graph-root", 8],
  ["module", 176],
]));

const focused = run(["--focused-roots"]).stdout.trimEnd().split("\n");
assert.equal(focused.length, 184);
assert.equal(new Set(focused).size, 184);

const diagnostics = run(["--negative-diagnostics"]).stdout.trimEnd().split("\n");
assert.equal(diagnostics.length, 8);
assert.equal(diagnostics.filter((line) => line.endsWith("\texact")).length, 7);
assert.equal(diagnostics.filter((line) => line.endsWith("\tabsent")).length, 0);
assert.equal(diagnostics.filter((line) => line.includes("\truntime\tTypeError\t")).length, 2);

const rules = run(["--diagnostic-rules"]).stdout.trimEnd().split("\n");
assert.equal(rules.length, 4);

const excluded = [
  "test/language/module-code/top-level-await/fulfillment-order.js",
  "test/language/module-code/top-level-await/module-graphs-does-not-hang.js",
  "test/language/module-code/top-level-await/module-import-rejection-tick.js",
  "test/language/module-code/top-level-await/rejection-order.js",
  "test/language/module-code/top-level-await/syntax/await-expr-dyn-import.js",
  "test/language/module-code/top-level-await/syntax/catch-parameter.js",
];
const generated = [admissions, focused, diagnostics].flat().join("\n");
for (const relativePath of excluded) {
  assert(!generated.includes(relativePath), `${relativePath}: excluded canary escaped`);
}

run(["--check-current"]);
const unknown = run(["--not-an-option"], 1);
assert.match(unknown.stderr, /unknown option/u);

for (const [path, contents] of before) {
  assert.deepEqual(readFileSync(path), contents, `${path} was modified by generator self-test`);
}

console.log("tla-b candidate generator tests passed");
