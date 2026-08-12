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
const checkedProfile = join(root, "compat/test262-oxide.conf");
const checkedManifest = join(root, "tests/test262-class-private-callables-b.txt");
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

const cohort = "test/language/module-code/top-level-await";
const admissionGroup = "tla-core-a";
const roots = [
  `${cohort}/async-module-does-not-block-sibling-modules.js`,
  `${cohort}/await-expr-resolution.js`,
  `${cohort}/module-import-resolution.js`,
  `${cohort}/new-await-script-code.js`,
];
const moduleRoots = roots.filter((relativePath) =>
  relativePath !== `${cohort}/new-await-script-code.js`,
);
const sources = [
  `${cohort}/async-module-does-not-block-sibling-modules.js`,
  `${cohort}/async-module-sync_FIXTURE.js`,
  `${cohort}/async-module-tla_FIXTURE.js`,
  `${cohort}/await-expr-resolution.js`,
  `${cohort}/module-import-resolution.js`,
  `${cohort}/module-import-resolution_FIXTURE.js`,
  `${cohort}/new-await-script-code.js`,
];
const moduleSources = sources.filter(
  (relativePath) => relativePath !== `${cohort}/new-await-script-code.js`,
);
const expectedSourceSha256 = new Map([
  [
    `${cohort}/async-module-does-not-block-sibling-modules.js`,
    "d68087ed6cc6b70767803a6711706666dd8cb83007d63351e428cfa817697e9a",
  ],
  [
    `${cohort}/async-module-sync_FIXTURE.js`,
    "4712c3bf30078873947b4cf7c258a3b678cc4f9f6bfe1a432ef09d69ba4c2a32",
  ],
  [
    `${cohort}/async-module-tla_FIXTURE.js`,
    "8c60e1afd07b1ee862c4c983fdc1bac6e8a3647e45fc987aeafedc7fa01af76e",
  ],
  [
    `${cohort}/await-expr-resolution.js`,
    "8d174bfc4457c1f0c28503dcf71a7839fe0253fdd9d5136da4b75b1c2ddf61b7",
  ],
  [
    `${cohort}/module-import-resolution.js`,
    "5b09874f3479970d85876293e69e425adede72cca6a0eed1f559f02911917570",
  ],
  [
    `${cohort}/module-import-resolution_FIXTURE.js`,
    "6caafde4555e5d2b80417b2986b81d8af6780534a645d875cdb56ee51efc9f1b",
  ],
  [
    `${cohort}/new-await-script-code.js`,
    "5fa53f54d7e723c4a81e068c69f013b145d08ac73a5dff84f349fa0bd4ac8d17",
  ],
]);
const expectedRequests = new Map([
  [
    `${cohort}/async-module-does-not-block-sibling-modules.js`,
    ["./async-module-tla_FIXTURE.js", "./async-module-sync_FIXTURE.js"],
  ],
  [`${cohort}/async-module-sync_FIXTURE.js`, []],
  [`${cohort}/async-module-tla_FIXTURE.js`, []],
  [`${cohort}/await-expr-resolution.js`, []],
  [`${cohort}/module-import-resolution.js`, ["./module-import-resolution_FIXTURE.js"]],
  [`${cohort}/module-import-resolution_FIXTURE.js`, []],
]);

const sha256 = (value) => createHash("sha256").update(value).digest("hex");
const bytewise = (left, right) => Buffer.compare(Buffer.from(left), Buffer.from(right));
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
    // The runner stores flags in a BTreeSet, so admissions use that order.
    flags: arrayField(text, "flags").sort(bytewise),
    features: arrayField(text, "features"),
    negativePhase: negative?.[1] ?? "",
    negativeType: negative?.[2] ?? "",
  };
}

function requestSpecifiers(relativePath) {
  const body = source(relativePath).replace(/\/\*---[\s\S]*?---\*\//, "");
  const requests = [];
  for (const line of body.split(/\r?\n|\r/)) {
    const match =
      line.match(/\sfrom\s*["']([^"']+)["']/) ??
      line.match(/^\s*import\s*["']([^"']+)["']/);
    if (!match || requests.includes(match[1])) continue;
    assert(match[1].startsWith("./"), `${relativePath}: non-child request ${match[1]}`);
    assert(!match[1].includes("/../"), `${relativePath}: escaping request ${match[1]}`);
    requests.push(match[1]);
  }
  return requests;
}

const normalize = (base, request) => posix.join(posix.dirname(base), request);
const fileEdges = new Map(
  moduleSources.map((basePath) => [
    basePath,
    requestSpecifiers(basePath).map((specifier) => ({
      specifier,
      normalized: normalize(basePath, specifier),
    })),
  ]),
);

function closure(rootPath) {
  const reached = new Set();
  const pending = [rootPath];
  while (pending.length > 0) {
    const basePath = pending.pop();
    if (reached.has(basePath)) continue;
    reached.add(basePath);
    for (const { normalized } of fileEdges.get(basePath)) {
      assert(moduleSources.includes(normalized), `${basePath}: unpinned request ${normalized}`);
      pending.push(normalized);
    }
  }
  return [...reached].sort(bytewise);
}

assert.deepEqual(roots, [...roots].sort(bytewise), "roots must remain bytewise sorted");
assert.deepEqual(sources, [...sources].sort(bytewise), "sources must remain bytewise sorted");
assert.equal(roots.length, 4);
assert.equal(moduleRoots.length, 3);
assert.equal(sources.length, 7);
assert.equal(moduleSources.length, 6);
assert.deepEqual([...expectedSourceSha256.keys()], sources);
assert.deepEqual([...expectedRequests.keys()], moduleSources);

for (const relativePath of sources) {
  assert.equal(
    sha256(source(relativePath)),
    expectedSourceSha256.get(relativePath),
    `${relativePath}: pinned source changed`,
  );
}
for (const relativePath of moduleSources) {
  assert.deepEqual(
    fileEdges.get(relativePath).map(({ specifier }) => specifier),
    expectedRequests.get(relativePath),
    `${relativePath}: module requests changed`,
  );
}
for (const rootPath of moduleRoots) {
  assert.deepEqual(metadata(rootPath), {
    includes: [],
    flags: ["async", "module"],
    features: ["top-level-await"],
    negativePhase: "",
    negativeType: "",
  });
}
const fixtures = moduleSources.filter((relativePath) => relativePath.endsWith("_FIXTURE.js"));
for (const fixture of fixtures) {
  assert.deepEqual(metadata(fixture), {
    includes: [],
    flags: [],
    features: [],
    negativePhase: "",
    negativeType: "",
  });
}
assert.deepEqual(metadata(`${cohort}/new-await-script-code.js`), {
  includes: [],
  flags: [],
  features: ["top-level-await"],
  negativePhase: "",
  negativeType: "",
});

const variantCount = roots.reduce(
  (count, relativePath) => count + (metadata(relativePath).flags.includes("module") ? 1 : 2),
  0,
);
const edgeCount = [...fileEdges.values()].reduce((count, requests) => count + requests.length, 0);
assert.equal(variantCount, 5);
assert.equal(edgeCount, 3);
assert.deepEqual(moduleRoots.map((rootPath) => closure(rootPath).length), [3, 1, 2]);

const admissionRecords = [
  ...moduleSources.map((relativePath) => {
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
  ...moduleSources.flatMap((relativePath) =>
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
  ...moduleRoots.map((rootPath) =>
    admissionRecord({
      kind: "graph-root",
      group: admissionGroup,
      path: rootPath,
      closure_file_count: closure(rootPath).length,
      priority: 4,
    }),
  ),
];
assert.equal(admissionRecords.length, 12);

function profileSection(path, section) {
  const lines = readFileSync(path, "utf8").split("\n");
  const start = lines.indexOf(`[${section}]`);
  assert.notEqual(start, -1, `${path}: missing [${section}]`);
  const end = lines.findIndex((line, index) => index > start && /^\[.+\]$/.test(line));
  return new Set(
    lines
      .slice(start + 1, end === -1 ? undefined : end)
      .map((line) => line.trim())
      .filter((line) => line && !line.startsWith("#")),
  );
}

if (mode === "--admissions") {
  process.stdout.write(renderAdmissionRows(admissionRecords));
} else {
  assertAdmissionGroup(checkedAdmissions, admissionGroup, admissionRecords);
  const features = profileSection(checkedProfile, "features");
  assert(features.has("top-level-await"), "top-level-await feature not promoted");
  const focused = new Set(
    readFileSync(checkedManifest, "utf8").split("\n").filter(Boolean),
  );
  for (const relativePath of roots) {
    assert(focused.has(relativePath), `${relativePath} focused root not promoted`);
  }
  console.log(
    "tla-core-a current baseline authenticated: " +
      "roots=4 variants=5 sources=7 graph_files=6 edges=3 records=12",
  );
}
