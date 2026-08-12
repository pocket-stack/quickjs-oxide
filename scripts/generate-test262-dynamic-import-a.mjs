#!/usr/bin/env node

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { join, posix, resolve } from "node:path";

import {
  admissionRecord,
  assertAdmissionGroup,
  renderAdmissionRows,
} from "./test262-admission-data.mjs";

const root = resolve(import.meta.dirname, "..");
const checkedSuite = join(root, "target/oracle/quickjs-2026-06-04/test262");
const checkedAdmissions = join(root, "dev-support/test262/admissions.tsv");
const args = process.argv.slice(2);

function option(name, fallback) {
  const indexes = args.flatMap((value, index) => (value === name ? [index] : []));
  assert(indexes.length <= 1, `duplicate ${name}`);
  if (indexes.length === 0) return fallback;
  const value = args[indexes[0] + 1];
  assert(value && !value.startsWith("--"), `${name} requires a value`);
  return resolve(value);
}

const suite = option("--suite", checkedSuite);
const selectedModes = ["--admissions", "--check-current"].filter((mode) =>
  args.includes(mode),
);
assert(selectedModes.length <= 1, "select at most one output/check mode");
const mode = selectedModes[0] ?? "--check-current";

const valueOptions = new Set(["--suite"]);
const flagOptions = new Set(["--admissions", "--check-current"]);
for (let index = 0; index < args.length; index += 1) {
  const argument = args[index];
  if (valueOptions.has(argument)) {
    index += 1;
  } else {
    assert(flagOptions.has(argument), `unknown option: ${argument}`);
  }
}

assert(existsSync(join(suite, "test")), `missing Test262 suite: ${suite}`);

const cohort = "test/language/expressions/dynamic-import";
const admissionGroup = "dynamic-import-a";
const fixture = `${cohort}/dynamic-import-module_FIXTURE.js`;
const usageFixture = `${cohort}/usage/dynamic-import-module_FIXTURE.js`;
const roots = [
  `${cohort}/always-create-new-promise.js`,
  `${cohort}/assign-expr-get-value-abrupt-throws.js`,
  `${cohort}/reuse-namespace-object.js`,
  `${cohort}/usage/top-level-import-then-returns-thenable.js`,
].sort();
const expectedSourceSha256 = new Map([
  [
    `${cohort}/always-create-new-promise.js`,
    "e11f060801c828fbd99052aeb90eb4fa94420eed9ddf5530983decd121420cc1",
  ],
  [
    `${cohort}/assign-expr-get-value-abrupt-throws.js`,
    "e856975950b76ebdd1447bd273f4a7e07485333a6fe1d82e356582577f732b23",
  ],
  [
    `${cohort}/reuse-namespace-object.js`,
    "aa09ffc882f21eabd879596e11e83655c2146b20f5a9031d5202a2960cdfa68b",
  ],
  [
    `${cohort}/usage/top-level-import-then-returns-thenable.js`,
    "3ffdd7ebe4968cdd6abebafb718b855e818a041735120dec4611c045f98ecf3f",
  ],
  [fixture, "08bd41ffb3fbbfe3a33b50b66d0ca03f6266e5a4468cafc266eb109c15d7a261"],
  [usageFixture, "08bd41ffb3fbbfe3a33b50b66d0ca03f6266e5a4468cafc266eb109c15d7a261"],
]);

const sha256 = (value) => createHash("sha256").update(value).digest("hex");
const source = (relativePath) => readFileSync(join(suite, relativePath), "utf8");
const frontmatter = (text) => text.match(/\/\*---[\s\S]*?---\*\/(?:\r?\n)?/)?.[0] ?? "";
const arrayField = (text, name) => {
  const match = text.match(new RegExp(`^${name}:\\s*\\[([^\\]]*)\\]`, "m"));
  return (
    match?.[1]
      .split(",")
      .map((value) => value.trim())
      .filter(Boolean) ?? []
  );
};

function metadata(relativePath) {
  const text = frontmatter(source(relativePath));
  if (!text) {
    return { includes: [], flags: [], features: [], negativePhase: "", negativeType: "" };
  }
  const negative = text.match(
    /^negative:\s*\n\s*phase:\s*([^\s]+)\s*\n\s*type:\s*([^\s]+)\s*$/m,
  );
  return {
    includes: arrayField(text, "includes"),
    // The Rust metadata parser stores flags in a BTreeSet. Keep the admission
    // contract in the same canonical order rather than frontmatter order.
    flags: arrayField(text, "flags").sort(),
    features: arrayField(text, "features"),
    negativePhase: negative?.[1] ?? "",
    negativeType: negative?.[2] ?? "",
  };
}

function requestSpecifiers(relativePath) {
  const body = source(relativePath).replace(/\/\*---[\s\S]*?---\*\//, "");
  const requests = [];
  const importCall = /\bimport\s*\(\s*(["'])([^"']+)\1/g;
  for (const match of body.matchAll(importCall)) {
    if (!requests.includes(match[2])) requests.push(match[2]);
  }
  for (const request of requests) {
    assert(request.startsWith("./"), `${relativePath}: non-child request ${request}`);
    assert(!request.includes("/../"), `${relativePath}: escaping request ${request}`);
  }
  return requests;
}

const normalize = (base, request) => posix.join(posix.dirname(base), request);
const sources = [...roots, fixture, usageFixture].sort();
const fileEdges = new Map(
  sources.map((base) => [
    base,
    requestSpecifiers(base).map((specifier) => ({
      specifier,
      normalized: normalize(base, specifier),
    })),
  ]),
);

for (const relativePath of sources) {
  assert.equal(
    sha256(source(relativePath)),
    expectedSourceSha256.get(relativePath),
    `${relativePath}: pinned source changed`,
  );
}
for (const rootPath of roots) {
  const shape = metadata(rootPath);
  assert.deepEqual(shape.features, ["dynamic-import"], `${rootPath}: feature shape changed`);
  assert(!shape.flags.includes("module"), `${rootPath}: Script goal changed`);
  assert.equal(shape.negativePhase, "", `${rootPath}: negative phase changed`);
  assert.equal(shape.negativeType, "", `${rootPath}: negative type changed`);
}
assert.deepEqual(metadata(fixture), {
  includes: [],
  flags: [],
  features: [],
  negativePhase: "",
  negativeType: "",
});
assert.deepEqual(fileEdges.get(`${cohort}/assign-expr-get-value-abrupt-throws.js`), []);
for (const rootPath of roots.filter((path) => !path.endsWith("assign-expr-get-value-abrupt-throws.js"))) {
  const normalized = rootPath.includes("/usage/") ? usageFixture : fixture;
  assert.deepEqual(fileEdges.get(rootPath), [
    { specifier: "./dynamic-import-module_FIXTURE.js", normalized },
  ]);
}
assert.deepEqual(fileEdges.get(fixture), []);
assert.deepEqual(fileEdges.get(usageFixture), []);

const closureSize = (rootPath) => (fileEdges.get(rootPath).length === 0 ? 1 : 2);
const admissionRecords = [
  ...sources.map((relativePath) => {
    const shape = metadata(relativePath);
    return admissionRecord({
      kind: "graph-file",
      group: admissionGroup,
      path: relativePath,
      source_sha256: sha256(source(relativePath)),
      includes: shape.includes,
      flags: shape.flags,
      features: shape.features,
      negative_phase: shape.negativePhase,
      negative_type: shape.negativeType,
    });
  }),
  ...sources.flatMap((relativePath) =>
    fileEdges.get(relativePath).map((request, requestIndex) =>
      admissionRecord({
        kind: "graph-request",
        group: admissionGroup,
        path: relativePath,
        request_index: requestIndex,
        specifier: request.specifier,
        normalized_path: request.normalized,
      }),
    ),
  ),
  ...roots.map((rootPath) =>
    admissionRecord({
      kind: "dynamic-import-root",
      group: admissionGroup,
      path: rootPath,
      closure_file_count: closureSize(rootPath),
      priority: 0,
      policy: "initial-import-tree",
    }),
  ),
];

assert.equal(roots.length, 4);
assert.equal(sources.length, 6);
assert.equal(
  [...fileEdges.values()].reduce((count, requests) => count + requests.length, 0),
  3,
);
assert.equal(admissionRecords.length, 13);

if (mode === "--admissions") {
  process.stdout.write(renderAdmissionRows(admissionRecords));
} else {
  assertAdmissionGroup(checkedAdmissions, admissionGroup, admissionRecords);
  console.log(
    "dynamic-import-a admissions authenticated: roots=4 variants=8 sources=6 edges=3",
  );
}
