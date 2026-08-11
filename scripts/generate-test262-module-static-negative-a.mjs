#!/usr/bin/env node

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join, relative, resolve, sep } from "node:path";

import {
  admissionRecord,
  assertAdmissionGroup,
  renderAdmissionRows,
} from "./test262-admission-data.mjs";

const root = resolve(import.meta.dirname, "..");
const checkedSuite = join(root, "target/oracle/quickjs-2026-06-04/test262");
const checkedProfile = join(root, "compat/test262-oxide.conf");
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
assert(existsSync(checkedProfile), `missing Oxide profile: ${checkedProfile}`);

const expected = {
  roots: 67,
  emptyFeatures: 57,
  exportStarNamespace: 4,
  generators: 3,
  let: 1,
  letConst: 1,
  newTarget: 1,
  requests: 13,
  blockListFlags: 7,
  canaries: 25,
  manifestSha256: "dd8e65fab5447123ad48aa383a835893b72a5e899d34d2dce3a81660bdacc145",
  evidenceSha256: {
    "tests/test262-module-static-negative-a.txt":
      "dd8e65fab5447123ad48aa383a835893b72a5e899d34d2dce3a81660bdacc145",
    "tests/test262-module-static-negative-a-ledger.tsv":
      "e58797b86f3fa3d22f439e2d0c5e575030db0fbc9fc948bcfd9d8e2ff589765c",
    "tests/test262-module-static-negative-a-requests.tsv":
      "fcfc7d66f73137a7959b2c249b8c0ed877fdca5069366c782b29cef1e1180ba5",
    "tests/test262-module-static-negative-a-variants.tsv":
      "96241ec0e0a5fd07a2e0fbbf8d3b624960367678ec2bb1ce47a094d45c58b271",
    "tests/test262-module-static-negative-a-negatives.txt":
      "dd8e65fab5447123ad48aa383a835893b72a5e899d34d2dce3a81660bdacc145",
    "tests/test262-module-static-negative-a-exclusions.tsv":
      "b9ff3c813883844d745ce959ee79a0b61316cb94244157657a4c5daf0d76af82",
    "tests/test262-module-static-negative-a-provenance.tsv":
      "9b422029fcc07575e66f43f8f0b6c913622b1bd5c9c9c7c274793af9b10a28b8",
  },
};

const sha256 = (value) => createHash("sha256").update(value).digest("hex");
const source = (relativePath) => readFileSync(join(suite, relativePath), "utf8");
const frontmatter = (text) => text.match(/\/\*---[\s\S]*?---\*\/(?:\r?\n)?/)?.[0] ?? "";
const bytewise = (left, right) => Buffer.compare(Buffer.from(left), Buffer.from(right));

function cleanScalar(value) {
  return value.trim().replace(/^(['"])(.*)\1$/, "$2");
}

function listField(text, name) {
  const lines = text.split(/\r?\n|\r/);
  const index = lines.findIndex((line) => {
    if (/^\s/.test(line)) return false;
    const colon = line.indexOf(":");
    return colon !== -1 && line.slice(0, colon).trim() === name;
  });
  if (index === -1) return [];

  const raw = lines[index].slice(lines[index].indexOf(":") + 1).trim();
  if (raw.startsWith("[")) {
    let joined = raw;
    for (let next = index + 1; !joined.includes("]") && next < lines.length; next += 1) {
      joined += ` ${lines[next].trim()}`;
    }
    const end = joined.indexOf("]");
    assert.notEqual(end, -1, `unterminated ${name} list`);
    return joined
      .slice(1, end)
      .split(",")
      .flatMap((value) => value.trim().split(/\s+/))
      .map(cleanScalar)
      .filter(Boolean);
  }
  if (raw) return [cleanScalar(raw)];

  const values = [];
  for (const line of lines.slice(index + 1)) {
    if (!/^\s/.test(line)) break;
    const nested = line.trim();
    if (nested.startsWith("-")) values.push(cleanScalar(nested.slice(1)));
  }
  return values;
}

function usesBlockListField(text, name) {
  const lines = text.split(/\r?\n|\r/);
  const index = lines.findIndex((line) => line === `${name}:`);
  if (index === -1) return false;
  for (const line of lines.slice(index + 1)) {
    if (!/^\s/.test(line)) break;
    if (line.trim().startsWith("-")) return true;
  }
  return false;
}

function nestedScalar(text, parent, name) {
  const lines = text.split(/\r?\n|\r/);
  const index = lines.findIndex((line) => line === `${parent}:`);
  if (index === -1) return "";
  for (const line of lines.slice(index + 1)) {
    if (!/^\s/.test(line)) break;
    const match = line.trim().match(/^([^:]+):\s*(.*?)\s*$/);
    if (match?.[1].trim() === name) return cleanScalar(match[2]);
  }
  return "";
}

function metadata(relativePath) {
  const text = frontmatter(source(relativePath));
  return {
    includes: listField(text, "includes"),
    flags: listField(text, "flags"),
    features: listField(text, "features"),
    negativePhase: nestedScalar(text, "negative", "phase"),
    negativeType: nestedScalar(text, "negative", "type"),
  };
}

function collectJavaScriptFiles(dir, paths = []) {
  const entries = readdirSync(dir, { withFileTypes: true }).sort((left, right) =>
    bytewise(left.name, right.name),
  );
  for (const entry of entries) {
    const absolute = join(dir, entry.name);
    if (entry.isDirectory()) {
      collectJavaScriptFiles(absolute, paths);
    } else if (entry.isFile() && entry.name.endsWith(".js") && !entry.name.endsWith("_FIXTURE.js")) {
      paths.push(relative(suite, absolute).split(sep).join("/"));
    }
  }
  return paths;
}

function profileSection(text, name) {
  const lines = text.split("\n");
  const start = lines.indexOf(`[${name}]`);
  assert.notEqual(start, -1, `missing [${name}] section`);
  const values = [];
  for (const line of lines.slice(start + 1)) {
    if (line.startsWith("[")) break;
    if (line) values.push(line);
  }
  return values;
}

function withoutOwnedProfileLines(text, owned) {
  return text
    .split("\n")
    .filter((line) => !owned.has(line))
    .join("\n");
}

function staticRequests(relativePath) {
  const body = source(relativePath).replace(/\/\*---[\s\S]*?---\*\//, "");
  const requests = [];
  for (const line of body.split(/\r?\n|\r/)) {
    const trimmed = line.trimStart();
    if (!trimmed.startsWith("import") && !trimmed.startsWith("export")) continue;
    let request = null;
    const from = trimmed.indexOf(" from ");
    if (from !== -1) {
      request = trimmed.slice(from + " from ".length).trimStart();
    } else if (trimmed.startsWith("import")) {
      request = trimmed.slice("import".length).trimStart();
      if (!request.startsWith("'") && !request.startsWith('"')) request = null;
    }
    if (!request || (!request.startsWith("'") && !request.startsWith('"'))) continue;
    const quote = request[0];
    const end = request.indexOf(quote, 1);
    assert.notEqual(end, -1, `${relativePath}: unterminated static module request`);
    requests.push(request.slice(1, end));
  }
  return requests;
}

const allowedFeatures = new Set([
  "",
  "export-star-as-namespace-from-module",
  "generators",
  "let",
  "let,const",
  "new.target",
]);
const manifestPath = join(root, "tests/test262-module-static-negative-a.txt");
const owned = new Set(
  existsSync(manifestPath)
    ? readFileSync(manifestPath, "utf8").split("\n").filter(Boolean)
    : [],
);
const profile = readFileSync(checkedProfile, "utf8");
const audited = new Set(profileSection(profile, "audited-negative-tests"));
const previouslyAudited = new Set([...audited].filter((relativePath) => !owned.has(relativePath)));
const previousProfile = withoutOwnedProfileLines(profile, owned);

function selectorReason(relativePath) {
  if (relativePath.endsWith("_FIXTURE.js")) return "fixture";
  const shape = metadata(relativePath);
  if (shape.includes.length !== 0) return "includes-not-empty";
  if (shape.flags.join(",") !== "module") return "flags-not-exact-module";
  if (shape.negativePhase !== "parse" || shape.negativeType !== "SyntaxError") {
    return "not-parse-syntaxerror";
  }
  if (!allowedFeatures.has(shape.features.join(","))) return "features-not-allowed";
  if (previouslyAudited.has(relativePath)) return "previously-audited-negative";
  return "selected";
}

const roots = collectJavaScriptFiles(join(suite, "test"))
  .filter((relativePath) => selectorReason(relativePath) === "selected")
  .sort(bytewise);

const exclusionCanaries = [
  ["hidden-dynamic-import", "test/language/expressions/assignmenttargettype/direct-importcall.js"],
  ["hidden-dynamic-import", "test/language/expressions/assignmenttargettype/parenthesized-importcall.js"],
  ["dynamic-import", "test/language/expressions/dynamic-import/escape-sequence-import.js"],
  ["class-private", "test/language/module-code/invalid-private-names-call-expression-bad-reference.js"],
  ["class-private", "test/language/module-code/private-identifiers-not-empty.js"],
  ["class-private", "test/language/module-code/privatename-not-valid-earlyerr-module-1.js"],
  ["import-attributes", "test/language/module-code/import-attributes/early-dup-attribute-key-export.js"],
  ["import-attributes", "test/language/module-code/import-attributes/early-dup-attribute-key-import-nobinding.js"],
  ["import-attributes", "test/language/module-code/import-attributes/early-dup-attribute-key-import-withbinding.js"],
  ["top-level-await", "test/language/module-code/top-level-await/early-errors-await-not-simple-assignment-target.js"],
  ["top-level-await", "test/language/module-code/top-level-await/new-await.js"],
  ["top-level-await", "test/language/module-code/top-level-await/no-operand.js"],
  ["top-level-await", "test/language/module-code/top-level-await/syntax/early-does-not-propagate-to-fn-declaration-body.js"],
  ["top-level-await", "test/language/module-code/top-level-await/syntax/early-does-not-propagate-to-fn-declaration-params.js"],
  ["top-level-await", "test/language/module-code/top-level-await/syntax/early-does-not-propagate-to-fn-expr-body.js"],
  ["top-level-await", "test/language/module-code/top-level-await/syntax/early-does-not-propagate-to-fn-expr-params.js"],
  ["top-level-await", "test/language/module-code/top-level-await/syntax/early-no-escaped-await.js"],
  ["source-phase-import", "test/language/expressions/assignmenttargettype/direct-importcall-source.js"],
  ["source-phase-import", "test/language/expressions/assignmenttargettype/parenthesized-importcall-source.js"],
  ["import-defer", "test/language/expressions/assignmenttargettype/direct-importcall-defer.js"],
  ["import-defer", "test/language/expressions/assignmenttargettype/parenthesized-importcall-defer.js"],
  ["adjacent-syntax", "test/language/expressions/import.meta/syntax/escape-sequence-import.js"],
  ["adjacent-syntax", "test/language/import/import-attributes/json-invalid.js"],
  ["adjacent-syntax", "test/language/module-code/early-export-ill-formed-string.js"],
  ["adjacent-syntax", "test/language/statements/for-of/head-await-using-bound-names-in-stmt.js"],
];

const lines = (...values) => `${values.join("\n")}\n`;
const manifest = lines(...roots);
const requestRows = roots.flatMap((relativePath) =>
  staticRequests(relativePath).map((specifier, requestIndex) =>
    [relativePath, requestIndex, specifier].join("\t"),
  ),
);
const requests = lines(
  "path\trequest_index\tspecifier",
  ...requestRows,
);
const ledger = lines(
  "path\tincludes\tflags\tfeatures\tnegative_phase\tnegative_type\trequest_count\tsource_sha256\tfrontmatter_sha256",
  ...roots.map((relativePath) => {
    const text = source(relativePath);
    const shape = metadata(relativePath);
    return [
      relativePath,
      shape.includes.join(","),
      shape.flags.join(","),
      shape.features.join(","),
      shape.negativePhase,
      shape.negativeType,
      staticRequests(relativePath).length,
      sha256(text),
      sha256(frontmatter(text)),
    ].join("\t");
  }),
);
const variants = lines(
  "path\tvariant\tflags\tfeatures\tnegative_phase\tnegative_type\tsource_sha256",
  ...roots.map((relativePath) => {
    const shape = metadata(relativePath);
    return [
      relativePath,
      "sloppy",
      shape.flags.join(","),
      shape.features.join(","),
      shape.negativePhase,
      shape.negativeType,
      sha256(source(relativePath)),
    ].join("\t");
  }),
);
const exclusions = lines(
  "surface\tcanary_path\tselector_reason\tincludes\tflags\tfeatures\tnegative_phase\tnegative_type\tsource_sha256\tfrontmatter_sha256",
  ...exclusionCanaries.map(([surface, relativePath]) => {
    const text = source(relativePath);
    const shape = metadata(relativePath);
    return [
      surface,
      relativePath,
      selectorReason(relativePath),
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
const provenance = lines(
  "metric\tvalue",
  `selector\tincludes=[];flags=[module];negative=parse/SyntaxError;features in {[],[export-star-as-namespace-from-module],[generators],[let],[let,const],[new.target]};subtract prior audited negatives`,
  `parent_profile_sha256\t${sha256(previousProfile)}`,
  `parent_audited_negatives\t${previouslyAudited.size}`,
  `selected_roots\t${roots.length}`,
  `manifest_sha256\t${sha256(manifest)}`,
);

const evidence = new Map([
  ["tests/test262-module-static-negative-a.txt", manifest],
  ["tests/test262-module-static-negative-a-ledger.tsv", ledger],
  ["tests/test262-module-static-negative-a-requests.tsv", requests],
  ["tests/test262-module-static-negative-a-variants.tsv", variants],
  ["tests/test262-module-static-negative-a-negatives.txt", manifest],
  ["tests/test262-module-static-negative-a-exclusions.tsv", exclusions],
  ["tests/test262-module-static-negative-a-provenance.tsv", provenance],
]);

const featureCount = (features) =>
  roots.filter((relativePath) => metadata(relativePath).features.join(",") === features).length;
assert.equal(roots.length, expected.roots);
assert.equal(featureCount(""), expected.emptyFeatures);
assert.equal(featureCount("export-star-as-namespace-from-module"), expected.exportStarNamespace);
assert.equal(featureCount("generators"), expected.generators);
assert.equal(featureCount("let"), expected.let);
assert.equal(featureCount("let,const"), expected.letConst);
assert.equal(featureCount("new.target"), expected.newTarget);
assert.equal(requestRows.length, expected.requests);
assert.equal(
  roots.filter((relativePath) => usesBlockListField(frontmatter(source(relativePath)), "flags"))
    .length,
  expected.blockListFlags,
);
assert.equal(exclusionCanaries.length, expected.canaries);
assert.equal(sha256(manifest), expected.manifestSha256);
assert(roots.every((relativePath) => !previouslyAudited.has(relativePath)));
assert.equal(new Set(roots).size, roots.length);
assert(roots.every((relativePath) => !relativePath.endsWith("_FIXTURE.js")));
for (const [surface, relativePath] of exclusionCanaries) {
  assert(existsSync(join(suite, relativePath)), `missing ${surface} canary: ${relativePath}`);
  assert(!roots.includes(relativePath), `${surface} canary entered cohort: ${relativePath}`);
  assert.notEqual(selectorReason(relativePath), "selected", `${surface} canary became eligible`);
}

const admissionGroup = "module-static-negative-a";
const admissionRecords = roots.map((relativePath) => {
  const shape = metadata(relativePath);
  return admissionRecord({
    kind: "module",
    group: admissionGroup,
    path: relativePath,
    source_sha256: sha256(source(relativePath)),
    includes: shape.includes,
    flags: shape.flags,
    features: shape.features,
    negative_phase: shape.negativePhase,
    negative_type: shape.negativeType,
  });
});

if (mode === "admissions") {
  process.stdout.write(renderAdmissionRows(admissionRecords));
} else if (mode === "output") {
  assert(output, "--output requires a directory");
  for (const [relativePath, contents] of evidence) {
    writeFileSync(join(output, relativePath.split("/").at(-1)), contents);
  }
  console.log(`generated ${evidence.size} authenticated evidence files in ${output}`);
} else {
  assertAdmissionGroup(checkedAdmissions, admissionGroup, admissionRecords);
  assert(roots.every((relativePath) => audited.has(relativePath)), "profile is missing an admission");
  for (const [relativePath, contents] of evidence) {
    if (relativePath === "tests/test262-module-static-negative-a-provenance.tsv") {
      // This receipt records the profile as it stood when the cohort was
      // promoted. Later milestones legitimately extend that live profile, so
      // authenticate the historical receipt instead of regenerating it from
      // today's parent profile.
      assert.equal(
        sha256(readFileSync(join(root, relativePath), "utf8")),
        expected.evidenceSha256[relativePath],
        `${relativePath} historical receipt changed`,
      );
      continue;
    }
    if (expected.evidenceSha256[relativePath] !== "PENDING") {
      assert.equal(
        sha256(contents),
        expected.evidenceSha256[relativePath],
        `${relativePath} changed`,
      );
    }
    if (
      [
        "tests/test262-module-static-negative-a.txt",
        "tests/test262-module-static-negative-a-ledger.tsv",
        "tests/test262-module-static-negative-a-requests.tsv",
        "tests/test262-module-static-negative-a-exclusions.tsv",
        "tests/test262-module-static-negative-a-provenance.tsv",
      ].includes(relativePath)
    ) {
      assert.equal(
        readFileSync(join(root, relativePath), "utf8"),
        contents,
        `${relativePath} drifted`,
      );
    }
  }
  console.log(
    `module-static-negative-a: roots=${roots.length} empty=${featureCount("")} export-star=${featureCount("export-star-as-namespace-from-module")} generators=${featureCount("generators")} let=${featureCount("let")} let-const=${featureCount("let,const")} new-target=${featureCount("new.target")} requests=${requestRows.length} block-list-flags=${expected.blockListFlags} canaries=${exclusionCanaries.length}`,
  );
}
