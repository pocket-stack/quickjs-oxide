#!/usr/bin/env node

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync } from "node:fs";
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
const checkedFocused = join(root, "tests/test262-class-private-callables-b.txt");
const checkedDiagnostics = join(root, "dev-support/test262/negative-diagnostics.tsv");
const checkedRules = join(root, "dev-support/test262/negative-diagnostic-rules.tsv");
const checkedExemptions = join(root, "dev-support/test262/negative-diagnostic-exemptions.tsv");
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
const selectedModes = [
  "--admissions",
  "--diagnostic-candidates",
  "--check-current",
].filter((mode) => args.includes(mode));
assert(selectedModes.length <= 1, "select at most one output/check mode");
const mode = selectedModes[0] ?? "--check-current";

const valueOptions = new Set(["--suite"]);
const flagOptions = new Set([
  "--admissions",
  "--diagnostic-candidates",
  "--check-current",
]);
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
    // The Rust metadata parser stores flags in a BTreeSet. Keep the admission
    // contract in the same canonical order rather than frontmatter order.
    flags: arrayField(text, "flags").sort(),
    features: arrayField(text, "features"),
    negativePhase: negative?.[1] ?? "",
    negativeType: negative?.[2] ?? "",
  };
}

const invalidDirectory = `${cohort}/syntax/invalid`;
const invalidFiles = readdirSync(join(suite, invalidDirectory), { withFileTypes: true })
  .filter((entry) => entry.isFile() && entry.name.endsWith(".js"))
  .map((entry) => `${invalidDirectory}/${entry.name}`)
  .sort(bytewise);
const noNewFiles = invalidFiles.filter((relativePath) =>
  relativePath.endsWith("-no-new-call-expression.js"),
);
const newTargetRoots = noNewFiles.filter((relativePath) =>
  source(relativePath).includes("// - src/dynamic-import/no-new-call-expression.case\n"),
);
const sourcePhaseCanaries = noNewFiles.filter((relativePath) =>
  source(relativePath).includes(
    "// - src/dynamic-import/import-source-no-new-call-expression.case\n",
  ),
);
const importDeferCanaries = noNewFiles.filter((relativePath) =>
  source(relativePath).includes(
    "// - src/dynamic-import/import-defer-no-new-call-expression.case\n",
  ),
);
const pathManifest = (paths) => `${paths.join("\n")}\n`;
assert.equal(noNewFiles.length, 63, "dynamic-import no-new surface matrix changed");
assert.equal(newTargetRoots.length, 21, "plain dynamic-import new-target cohort changed");
assert.equal(sourcePhaseCanaries.length, 21, "import.source new-target canaries changed");
assert.equal(importDeferCanaries.length, 21, "import.defer new-target canaries changed");
const discoveredNoNewFiles = [...newTargetRoots, ...sourcePhaseCanaries, ...importDeferCanaries]
  .sort(bytewise);
assert.equal(
  new Set(discoveredNoNewFiles).size,
  63,
  "dynamic-import no-new provenance cohorts overlap",
);
assert.deepEqual(discoveredNoNewFiles, noNewFiles, "dynamic-import no-new provenance coverage changed");
assert.equal(
  sha256(pathManifest(newTargetRoots)),
  "cded3adc5d0a858964a71bb3cbda146925c5ce221efc99ef4e6fb7cf2815473b",
  "plain dynamic-import new-target path manifest changed",
);
assert.equal(
  sha256(pathManifest(sourcePhaseCanaries)),
  "278258e827ba7b990e0c6981ee9d1cf5b10aa5a05187ac0c0479d009e4675142",
  "import.source new-target path manifest changed",
);
assert.equal(
  sha256(pathManifest(importDeferCanaries)),
  "254e1a06734073963a5c109509f6551aad1dfcd977bc5aad8bb47ec1744a53c6",
  "import.defer new-target path manifest changed",
);
const normalizeNewTargetSurface = (relativePath) =>
  relativePath.replace(/-import-(?:source|defer)-no-new-call-expression\.js$/u, "-no-new-call-expression.js");
assert.deepEqual(
  sourcePhaseCanaries.map(normalizeNewTargetSurface).sort(bytewise),
  newTargetRoots.map(normalizeNewTargetSurface).sort(bytewise),
  "import.source new-target stems changed",
);
assert.deepEqual(
  importDeferCanaries.map(normalizeNewTargetSurface).sort(bytewise),
  newTargetRoots.map(normalizeNewTargetSurface).sort(bytewise),
  "import.defer new-target stems changed",
);

const assignmentTargetRoots = invalidFiles.filter((relativePath) =>
  /\/invalid-assignmenttargettype-syntax-error-[0-9]+-[^/]+\.js$/u.test(relativePath),
);
assert.equal(
  assignmentTargetRoots.length,
  17,
  "dynamic-import assignment-target negative cohort changed",
);
assert.equal(
  sha256(pathManifest(assignmentTargetRoots)),
  "79db48d835028190823247d0dab8a0587e82da01dd7d20a766c1c04476981803",
  "dynamic-import assignment-target path manifest changed",
);

function parseNegativeShape(relativePath) {
  const shape = metadata(relativePath);
  assert.deepEqual(shape.includes, [], `${relativePath}: includes changed`);
  assert.equal(shape.negativePhase, "parse", `${relativePath}: negative phase changed`);
  assert.equal(shape.negativeType, "SyntaxError", `${relativePath}: negative type changed`);
  return JSON.stringify({ flags: shape.flags, features: shape.features });
}

function assertShapeCounts(paths, expected, label) {
  const actual = new Map();
  for (const relativePath of paths) {
    const shape = parseNegativeShape(relativePath);
    actual.set(shape, (actual.get(shape) ?? 0) + 1);
  }
  assert.deepEqual(
    [...actual].sort(([left], [right]) => bytewise(left, right)),
    [...expected].sort(([left], [right]) => bytewise(left, right)),
    `${label} metadata shapes changed`,
  );
}

const shape = (flags, features) => JSON.stringify({ flags, features });
assertShapeCounts(
  newTargetRoots,
  new Map([
    [shape(["generated"], ["dynamic-import"]), 18],
    [shape(["generated"], ["dynamic-import", "async-iteration"]), 1],
    [shape(["generated", "noStrict"], ["dynamic-import"]), 2],
  ]),
  "plain dynamic-import new-target",
);
assertShapeCounts(
  sourcePhaseCanaries,
  new Map([
    [shape(["generated"], ["source-phase-imports", "source-phase-imports-module-source", "dynamic-import"]), 18],
    [shape(["generated"], ["source-phase-imports", "source-phase-imports-module-source", "dynamic-import", "async-iteration"]), 1],
    [shape(["generated", "noStrict"], ["source-phase-imports", "source-phase-imports-module-source", "dynamic-import"]), 2],
  ]),
  "import.source new-target canary",
);
assertShapeCounts(
  importDeferCanaries,
  new Map([
    [shape(["generated"], ["import-defer", "dynamic-import"]), 18],
    [shape(["generated"], ["import-defer", "dynamic-import", "async-iteration"]), 1],
    [shape(["generated", "noStrict"], ["import-defer", "dynamic-import"]), 2],
  ]),
  "import.defer new-target canary",
);
assertShapeCounts(
  assignmentTargetRoots,
  new Map([
    [shape([], ["dynamic-import"]), 16],
    [shape([], ["dynamic-import", "exponentiation"]), 1],
  ]),
  "dynamic-import assignment-target negative",
);
const variantCount = (paths) =>
  paths.reduce(
    (count, relativePath) => count + (metadata(relativePath).flags.includes("noStrict") ? 1 : 2),
    0,
  );
assert.equal(variantCount(newTargetRoots), 40, "plain new-target variant count changed");
assert.equal(variantCount(sourcePhaseCanaries), 40, "import.source canary variant count changed");
assert.equal(variantCount(importDeferCanaries), 40, "import.defer canary variant count changed");

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

function parseRejectedAdmissionRecords(paths, group) {
  return [
    ...paths.map((relativePath) => {
      const shape = metadata(relativePath);
      return admissionRecord({
        kind: "graph-file",
        group,
        path: relativePath,
        source_sha256: sha256(source(relativePath)),
        includes: shape.includes,
        flags: shape.flags,
        features: shape.features,
        negative_phase: shape.negativePhase,
        negative_type: shape.negativeType,
      });
    }),
    ...paths.map((relativePath) =>
      admissionRecord({
        kind: "dynamic-import-root",
        group,
        path: relativePath,
        closure_file_count: 1,
        priority: 0,
        policy: "parse-rejected",
      }),
    ),
  ];
}

const assignmentTargetGroup = "dynamic-import-invalid-assignment-target-a";
const assignmentTargetAdmissionRecords = parseRejectedAdmissionRecords(
  assignmentTargetRoots,
  assignmentTargetGroup,
);
const newTargetGroup = "dynamic-import-new-target-a";
const newTargetAdmissionRecords = parseRejectedAdmissionRecords(
  newTargetRoots,
  newTargetGroup,
);
assert.equal(assignmentTargetAdmissionRecords.length, 34);
assert.equal(newTargetAdmissionRecords.length, 42);

const diagnosticRule = "dynamic-import.invalid-new-target";
const diagnosticRuleLine =
  `${diagnosticRule}\tjs_parse_postfix_expr\tnew cannot directly target ImportCall`;
const diagnosticCandidates = newTargetRoots.flatMap((relativePath) => {
  const variants = metadata(relativePath).flags.includes("noStrict")
    ? ["sloppy"]
    : ["sloppy", "strict"];
  return variants.map((variant) => `${relativePath}\t${variant}\t${diagnosticRule}`);
});
diagnosticCandidates.sort(bytewise);
assert.equal(diagnosticCandidates.length, 40);

function checkedLineSet(path) {
  return new Set(readFileSync(path, "utf8").split("\n"));
}

function profileSection(name) {
  const lines = readFileSync(checkedProfile, "utf8").trimEnd().split("\n");
  const start = lines.indexOf(`[${name}]`);
  assert.notEqual(start, -1, `missing profile section [${name}]`);
  const next = lines.findIndex((line, index) => index > start && /^\[[^\]]+\]$/u.test(line));
  return new Set(lines.slice(start + 1, next === -1 ? lines.length : next).filter(Boolean));
}

function assertPromoted() {
  const profile = profileSection("audited-negative-tests");
  const focused = checkedLineSet(checkedFocused);
  const canaryPaths = new Set([...sourcePhaseCanaries, ...importDeferCanaries]);
  for (const relativePath of assignmentTargetRoots) {
    assert(profile.has(relativePath), `${relativePath}: assignment profile path not promoted`);
  }
  for (const relativePath of newTargetRoots) {
    assert(profile.has(relativePath), `${relativePath}: profile path not promoted`);
    assert(focused.has(relativePath), `${relativePath}: focused path not promoted`);
  }
  for (const relativePath of [...sourcePhaseCanaries, ...importDeferCanaries]) {
    assert(!profile.has(relativePath), `${relativePath}: canary escaped into the profile`);
    assert(!focused.has(relativePath), `${relativePath}: canary escaped into the focused manifest`);
  }
  const admittedPaths = readFileSync(checkedAdmissions, "utf8")
    .trimEnd()
    .split("\n")
    .slice(1)
    .map((line) => line.split("\t")[2]);
  assert(
    admittedPaths.every((relativePath) => !canaryPaths.has(relativePath)),
    "new-target surface canary escaped into admissions",
  );
  const rules = checkedLineSet(checkedRules);
  assert(rules.has(diagnosticRuleLine), `${diagnosticRule}: rule not promoted`);

  const contractLines = readFileSync(checkedDiagnostics, "utf8")
    .trimEnd()
    .split("\n")
    .slice(1);
  const assignmentPaths = new Set(assignmentTargetRoots);
  const assignmentContracts = contractLines
    .map((line) => line.split("\t"))
    .filter((fields) => assignmentPaths.has(fields[0]));
  assert.equal(
    assignmentContracts.length,
    34,
    "assignment-target diagnostic contract count drifted",
  );
  const expectedAssignmentKeys = assignmentTargetRoots
    .flatMap((relativePath) =>
      (metadata(relativePath).flags.includes("noStrict") ? ["sloppy"] : ["sloppy", "strict"])
        .map((variant) => `${relativePath}\t${variant}`),
    )
    .sort(bytewise);
  const actualAssignmentKeys = assignmentContracts
    .map((fields) => `${fields[0]}\t${fields[1]}`)
    .sort(bytewise);
  assert.deepEqual(
    actualAssignmentKeys,
    expectedAssignmentKeys,
    "assignment-target diagnostic variants drifted",
  );
  for (const fields of assignmentContracts) {
    assert.equal(fields.length, 10, `${fields[0]} ${fields[1]}: assignment diagnostic schema drifted`);
    assert.equal(
      fields[2],
      sha256(source(fields[0])),
      `${fields[0]}: assignment diagnostic source hash drifted`,
    );
    assert.equal(fields[3], "parse", `${fields[0]}: assignment diagnostic phase drifted`);
    assert.equal(fields[4], "SyntaxError", `${fields[0]}: assignment diagnostic type drifted`);
    assert(fields[5], `${fields[0]}: assignment diagnostic rule is empty`);
    assert(fields[6], `${fields[0]}: assignment diagnostic message is empty`);
    assert.match(fields[7], /^[1-9][0-9]*$/u, `${fields[0]}: assignment diagnostic line drifted`);
    assert.match(fields[8], /^[1-9][0-9]*$/u, `${fields[0]}: assignment diagnostic column drifted`);
    assert.equal(fields[9], "exact", `${fields[0]}: assignment location policy drifted`);
  }
  const ownedPaths = new Set(newTargetRoots);
  const ownedContracts = contractLines
    .map((line) => line.split("\t"))
    .filter((fields) => ownedPaths.has(fields[0]));
  assert(
    contractLines.every((line) => !canaryPaths.has(line.split("\t")[0])),
    "new-target surface canary escaped into diagnostics",
  );
  assert.equal(ownedContracts.length, 40, "new-target diagnostic contract count drifted");
  for (const fields of ownedContracts) {
    assert.equal(fields.length, 10, `${fields[0]} ${fields[1]}: diagnostic schema drifted`);
    assert.equal(fields[2], sha256(source(fields[0])), `${fields[0]}: diagnostic source hash drifted`);
    assert.equal(fields[3], "parse", `${fields[0]}: diagnostic phase drifted`);
    assert.equal(fields[4], "SyntaxError", `${fields[0]}: diagnostic type drifted`);
    assert.equal(fields[5], diagnosticRule, `${fields[0]}: diagnostic rule drifted`);
    assert.equal(fields[6], "invalid use of 'import()'", `${fields[0]}: diagnostic message drifted`);
    assert.match(fields[7], /^[1-9][0-9]*$/u, `${fields[0]}: diagnostic line drifted`);
    assert.match(fields[8], /^[1-9][0-9]*$/u, `${fields[0]}: diagnostic column drifted`);
    assert.equal(fields[9], "exact", `${fields[0]}: diagnostic location policy drifted`);
  }
  const actualCandidates = ownedContracts
    .map((fields) => `${fields[0]}\t${fields[1]}\t${fields[5]}`)
    .sort(bytewise);
  assert.deepEqual(actualCandidates, diagnosticCandidates, "new-target diagnostic contracts drifted");

  const exemptions = readFileSync(checkedExemptions, "utf8")
    .trimEnd()
    .split("\n")
    .slice(1)
    .map((line) => line.split("\t")[0]);
  assert(
    exemptions.every((relativePath) => !ownedPaths.has(relativePath)),
    "new-target diagnostics must not use exemptions",
  );
  assert(
    exemptions.every((relativePath) => !assignmentPaths.has(relativePath)),
    "assignment-target diagnostics must not use exemptions",
  );
  assert(
    exemptions.every((relativePath) => !canaryPaths.has(relativePath)),
    "new-target surface canary escaped into exemptions",
  );
}

if (mode === "--admissions") {
  process.stdout.write(
    renderAdmissionRows([
      ...admissionRecords,
      ...assignmentTargetAdmissionRecords,
      ...newTargetAdmissionRecords,
    ]),
  );
} else if (mode === "--diagnostic-candidates") {
  process.stdout.write(`path\tvariant\trule\n${diagnosticCandidates.join("\n")}\n`);
} else {
  assertAdmissionGroup(checkedAdmissions, admissionGroup, admissionRecords);
  assertAdmissionGroup(
    checkedAdmissions,
    assignmentTargetGroup,
    assignmentTargetAdmissionRecords,
  );
  assertAdmissionGroup(checkedAdmissions, newTargetGroup, newTargetAdmissionRecords);
  assertPromoted();
  console.log(
    "dynamic-import admissions authenticated: runtime roots=4/variants=8; " +
      "assignment negatives=17/34; new-target negatives=21/40; sources=44; edges=3",
  );
}
