#!/usr/bin/env node

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
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
const suiteIndex = args.indexOf("--suite");
const outputIndex = args.indexOf("--output");
const suite = resolve(suiteIndex === -1 ? checkedSuite : args[suiteIndex + 1]);
const mode = args.includes("--admissions")
  ? "admissions"
  : outputIndex === -1
    ? "check"
    : "output";
const output = outputIndex === -1 ? null : resolve(args[outputIndex + 1]);

assert(existsSync(join(suite, "test")), `missing Test262 suite: ${suite}`);

const cohort = "test/language/module-code";
const expected = {
  roots: 38,
  sources: 58,
  fixtures: 20,
  rootedEdges: 45,
  selfEdges: 21,
  maxClosure: 10,
  rootSha256: "c38a9ef682aadeb60b135f03b2292ed3f6db60268d20c1fe674aa935bc8a93d6",
  sourcesSha256: "a124cac7c65ed20e26907f1cbfec566edbdf6176c699d3617ce8e16c9f41c9bb",
  edgesSha256: "8d89e0b7eab9a65c56eb7b683ba71f99e21a95f5e7fab36264678e500d049069",
  closuresSha256: "245ca0d78acd6511c643e3969330300932e4fbe0e1dc31628eb5a0ec44d90e16",
};

const sha256 = (value) => createHash("sha256").update(value).digest("hex");
const source = (relative) => readFileSync(join(suite, relative), "utf8");
const frontmatter = (text) => text.match(/\/\*---[\s\S]*?---\*\/(?:\r?\n)?/)?.[0] ?? "";
const arrayField = (text, name) => {
  const match = text.match(new RegExp(`^${name}:\\s*\\[([^\\]]*)\\]`, "m"));
  return match?.[1]
    .split(",")
    .map((value) => value.trim())
    .filter(Boolean) ?? [];
};

function metadata(relative) {
  const text = frontmatter(source(relative));
  if (!text) {
    return { includes: [], flags: [], features: [], negativePhase: "", negativeType: "" };
  }
  const negative = text.match(
    /^negative:\s*\n\s*phase:\s*([^\s]+)\s*\n\s*type:\s*([^\s]+)\s*$/m,
  );
  return {
    includes: arrayField(text, "includes"),
    flags: arrayField(text, "flags"),
    features: arrayField(text, "features"),
    negativePhase: negative?.[1] ?? "",
    negativeType: negative?.[2] ?? "",
  };
}

function requestSpecifiers(relative) {
  const body = source(relative).replace(/\/\*---[\s\S]*?---\*\//, "");
  const requests = [];
  for (const line of body.split(/\r?\n/)) {
    const match =
      line.match(/\sfrom\s*['"]([^'"]+)['"]/) ??
      line.match(/^\s*import\s*['"]([^'"]+)['"]/);
    if (match && !requests.includes(match[1])) requests.push(match[1]);
  }
  for (const request of requests) {
    assert(request.startsWith("./"), `${relative}: non-child request ${request}`);
    assert(!request.includes("/../"), `${relative}: escaping request ${request}`);
  }
  return requests;
}

const normalize = (base, request) => posix.join(posix.dirname(base), request);

function isDefaultGraphRoot(name) {
  return (
    (name.startsWith("eval-export-dflt-") &&
      !name.startsWith("eval-export-dflt-expr-err-")) ||
    name.startsWith("eval-gtbndng-indirect-") ||
    /^eval-rqstd-(once|order)\.js$/.test(name) ||
    name === "eval-self-once.js" ||
    name === "export-star-as-dflt.js" ||
    (name.startsWith("instn-") &&
      name.includes("dflt") &&
      !/^instn-star-(as-)?props-dflt/.test(name))
  );
}

const roots = readdirSync(join(suite, cohort), { withFileTypes: true })
  .filter(
    (entry) =>
      entry.isFile() &&
      entry.name.endsWith(".js") &&
      !entry.name.endsWith("_FIXTURE.js") &&
      isDefaultGraphRoot(entry.name),
  )
  .map((entry) => `${cohort}/${entry.name}`)
  .sort();

function closure(rootPath) {
  const reached = new Set();
  const pending = [rootPath];
  while (pending.length > 0) {
    const base = pending.pop();
    if (reached.has(base)) continue;
    reached.add(base);
    for (const request of requestSpecifiers(base)) pending.push(normalize(base, request));
  }
  return [...reached].sort();
}

const sources = [...new Set(roots.flatMap(closure))].sort();
const rootedEdges = roots.flatMap((rootPath) =>
  closure(rootPath).flatMap((base) =>
    requestSpecifiers(base).map((specifier, requestIndex) => ({
      rootPath,
      base,
      requestIndex,
      specifier,
      normalized: normalize(base, specifier),
    })),
  ),
);
const fileEdges = new Map(
  sources.map((base) => [
    base,
    requestSpecifiers(base).map((specifier) => ({
      specifier,
      normalized: normalize(base, specifier),
    })),
  ]),
);

const negativeRoots = roots.filter((relative) => metadata(relative).negativePhase);
const exclusionCanaries = [
  ["dynamic-import", "test/language/expressions/dynamic-import/always-create-new-promise.js"],
  ["top-level-await", "test/language/module-code/top-level-await/await-expr-resolution.js"],
  ["import-attributes", "test/language/module-code/import-attributes/import-attribute-empty.js"],
  ["import.meta", "test/language/expressions/import.meta/same-object-returned.js"],
  ["source-phase-import", "test/language/module-code/source-phase-import/import-source.js"],
];

const lines = (...values) => `${values.join("\n")}\n`;
const manifest = lines(...roots);
const sourceManifest = lines(...sources);
const edges = lines(
  "root_path\tbase_path\trequest_index\tspecifier\tnormalized_path",
  ...rootedEdges.map(({ rootPath, base, requestIndex, specifier, normalized }) =>
    [rootPath, base, requestIndex, specifier, normalized].join("\t"),
  ),
);
const closures = lines(
  "root_path\tclosure_files\trequest_edges",
  ...roots.map((rootPath) => {
    const files = closure(rootPath);
    return [
      rootPath,
      files.length,
      files.reduce((count, relative) => count + fileEdges.get(relative).length, 0),
    ].join("\t");
  }),
);
const ledger = lines(
  "path\trole\tincludes\tflags\tfeatures\tnegative_phase\tnegative_type\tsource_sha256\tfrontmatter_sha256",
  ...sources.map((relative) => {
    const text = source(relative);
    const shape = metadata(relative);
    return [
      relative,
      roots.includes(relative) ? "root" : "fixture",
      shape.includes.join(","),
      shape.flags.join(","),
      shape.features.join(","),
      shape.negativePhase,
      shape.negativeType,
      sha256(text),
      sha256(frontmatter(text)),
    ].join("\t");
  }),
);
const negatives = lines(...negativeRoots);
const exclusions = lines(
  "surface\tcanary_path",
  ...exclusionCanaries.map((record) => record.join("\t")),
);

assert.equal(roots.length, expected.roots);
assert.equal(sources.length, expected.sources);
assert.equal(sources.length - roots.length, expected.fixtures);
assert.equal(rootedEdges.length, expected.rootedEdges);
assert.equal(rootedEdges.filter((edge) => edge.base === edge.normalized).length, expected.selfEdges);
assert.equal(Math.max(...roots.map((rootPath) => closure(rootPath).length)), expected.maxClosure);
assert.equal(negativeRoots.length, 5);
assert.equal(sha256(manifest), expected.rootSha256);
assert.equal(sha256(sourceManifest), expected.sourcesSha256);
assert.equal(sha256(edges), expected.edgesSha256);
assert.equal(sha256(closures), expected.closuresSha256);
for (const [, relative] of exclusionCanaries) {
  assert(existsSync(join(suite, relative)), `missing exclusion canary: ${relative}`);
  assert(!roots.includes(relative), `excluded surface entered cohort: ${relative}`);
}

const evidence = new Map([
  ["tests/test262-module-default-a.txt", manifest],
  ["tests/test262-module-default-a-sources.txt", sourceManifest],
  ["tests/test262-module-default-a-edges.tsv", edges],
  ["tests/test262-module-default-a-closures.tsv", closures],
  ["tests/test262-module-default-a-ledger.tsv", ledger],
  ["tests/test262-module-default-a-negatives.txt", negatives],
  ["tests/test262-module-default-a-exclusions.tsv", exclusions],
]);

const admissionGroup = "module-default-a";
const admissionRecords = [
  ...sources.map((relative) => {
    const shape = metadata(relative);
    return admissionRecord({
      kind: "graph-file",
      group: admissionGroup,
      path: relative,
      source_sha256: sha256(source(relative)),
      includes: shape.includes,
      flags: shape.flags,
      features: shape.features,
      negative_phase: shape.negativePhase,
      negative_type: shape.negativeType,
    });
  }),
  ...sources.flatMap((relative) =>
    fileEdges.get(relative).map((request, requestIndex) =>
      admissionRecord({
        kind: "graph-request",
        group: admissionGroup,
        path: relative,
        request_index: requestIndex,
        specifier: request.specifier,
        normalized_path: request.normalized,
      }),
    ),
  ),
  ...roots.map((rootPath) =>
    admissionRecord({
      kind: "graph-root",
      group: admissionGroup,
      path: rootPath,
      closure_file_count: closure(rootPath).length,
      priority: 0,
    }),
  ),
];

if (mode === "admissions") {
  process.stdout.write(renderAdmissionRows(admissionRecords));
} else if (mode === "output") {
  assert(output, "--output requires a directory");
  for (const [relative, contents] of evidence) {
    const destination = join(output, relative.split("/").at(-1));
    writeFileSync(destination, contents);
  }
  console.log(`generated ${evidence.size} authenticated evidence files in ${output}`);
} else {
  assertAdmissionGroup(checkedAdmissions, admissionGroup, admissionRecords);
  for (const [relative, contents] of evidence) {
    if (
      ![
        "tests/test262-module-default-a-edges.tsv",
        "tests/test262-module-default-a-closures.tsv",
        "tests/test262-module-default-a-ledger.tsv",
      ].includes(relative)
    ) {
      continue;
    }
    assert.equal(readFileSync(join(root, relative), "utf8"), contents, `${relative} drifted`);
  }
  console.log(
    `module-default-a: roots=${roots.length} sources=${sources.length} rooted_edges=${rootedEdges.length} negatives=${negativeRoots.length}`,
  );
}
