#!/usr/bin/env node

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { basename, dirname, join, posix, relative, resolve, sep } from "node:path";

import {
  admissionHeader,
  admissionRecord,
  assertAdmissionGroup,
  renderAdmissionRows,
} from "./test262-admission-data.mjs";

const root = resolve(import.meta.dirname, "..");
const checkedSource = join(root, "target/oracle/quickjs-2026-06-04");
const checkedSuite = join(checkedSource, "test262");
const checkedRunner = join(checkedSource, "run-test262");
const checkedConfig = join(checkedSource, "test262.conf");
const checkedAdmissions = join(root, "dev-support/test262/admissions.tsv");
const checkedProfile = join(root, "compat/test262-oxide.conf");
const checkedManifest = join(root, "tests/test262-class-private-callables-b.txt");
const checkedDiagnostics = join(root, "dev-support/test262/negative-diagnostics.tsv");
const checkedDiagnosticRules = join(
  root,
  "dev-support/test262/negative-diagnostic-rules.tsv",
);

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
const quickjsRunner = option("--quickjs-runner", checkedRunner);
const quickjsConfig = option("--quickjs-config", checkedConfig);
const output = option("--output", null);
const selectedModes = [
  "--admissions",
  "--diagnostic-candidates",
  "--negative-diagnostics",
  "--check-current",
].filter((mode) => args.includes(mode));
assert(selectedModes.length <= 1, "select at most one output/check mode");
assert(!(output && selectedModes.length), "--output cannot be combined with another mode");

const valueOptions = new Set([
  "--suite",
  "--quickjs-runner",
  "--quickjs-config",
  "--output",
]);
const flagOptions = new Set([
  "--admissions",
  "--diagnostic-candidates",
  "--negative-diagnostics",
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
assert(existsSync(quickjsRunner), `missing pinned QuickJS run-test262: ${quickjsRunner}`);
assert(existsSync(quickjsConfig), `missing pinned QuickJS Test262 config: ${quickjsConfig}`);

const cohort = "test/language/import/import-attributes";
const admissionGroup = "module-json-a";
const expected = {
  roots: 11,
  sources: 20,
  fixtures: 9,
  fileEdges: 11,
  rootedEdges: 11,
  resolutionNegatives: 2,
  normalRoots: 9,
  propertyHelperRoots: 3,
  admissionRows: 42,
  evidenceSha256: {
    "test262-module-json-a.txt":
      "b649c6c9b88cc04e44eac5af955ccd59ee6c95c5362a63b979979c0c1cc4b874",
    "test262-module-json-a-sources.txt":
      "0605201ddecc059245167f90bcd0d5627efe1b9d7d58a0fe20af977d80955c9c",
    "test262-module-json-a-edges.tsv":
      "50aeff1da34833c2906da029f94f1d41130b9a193ccba8309ee9c68adbd746cd",
    "test262-module-json-a-closures.tsv":
      "04f7670da813c6d0f456b31fc16e4aaaca16cd1027d288e848ad78a596d4afc3",
    "test262-module-json-a-ledger.tsv":
      "08f1d3188a51e603e5d5f24e32b5ca01f3fff015358c7b58c4d1b3cd54963f5e",
    "test262-module-json-a-variants.tsv":
      "9a4c9f216ea061a910b06e60a60048707861f9b18ef5be60d37aff28646b2cfa",
    "test262-module-json-a-negatives.txt":
      "6be5f5187fb208cdbf33bf8fbd48e2554251dda9242e13cf557709b2745d4cc2",
    "test262-module-json-a-exclusions.tsv":
      "4c7ae708ff166f22ccdab8862ffa7b9f834f407fd76993efa9af4f05edd9f836",
    "test262-module-json-a-admission-rows.tsv":
      "9ef80b70fe94cfe8c8697cdc4962178cdf51213c334b99d0e5192d7846030b33",
    "test262-module-json-a-negative-diagnostic-candidates.tsv":
      "a7419ac9e00dbd26dfc44b9ed8c0f79d3cfef347d40a2839366d40849955053e",
    "test262-module-json-a-negative-diagnostic-rules.tsv":
      "e7da95d576fa6c5a3d567ca4de0f5efa48e6bfabc8c233f3f7821a8132082581",
    "test262-module-json-a-negative-diagnostics.tsv":
      "2078e259deb6c216b9d70cb375501a8e61c58a4a30f75470db546b570b22493a",
    "test262-module-json-a-quickjs.tsv":
      "5fb067778e71a6264fade18059a9a1ee615c6628979182f39092cc047cd8db14",
  },
};

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
    flags: arrayField(text, "flags"),
    features: arrayField(text, "features"),
    negativePhase: negative?.[1] ?? "",
    negativeType: negative?.[2] ?? "",
  };
}

function requestSpecifiers(relativePath) {
  if (!relativePath.endsWith(".js")) return [];
  const body = source(relativePath).replace(/\/\*---[\s\S]*?---\*\//, "");
  const requests = [];
  for (const line of body.split(/\r?\n|\r/)) {
    const match =
      line.match(/\sfrom\s*['"]([^'"]+)['"]/) ??
      line.match(/^\s*import\s*['"]([^'"]+)['"]/);
    if (!match) continue;
    const request = match[1];
    assert(request.startsWith("./"), `${relativePath}: non-child request ${request}`);
    assert(!request.includes("/../"), `${relativePath}: escaping request ${request}`);
    requests.push(request);
  }
  return requests;
}

const normalize = (base, request) => posix.join(posix.dirname(base), request);
const jsonCandidates = readdirSync(join(suite, cohort), { withFileTypes: true })
  .filter(
    (entry) =>
      entry.isFile() &&
      entry.name.startsWith("json-") &&
      entry.name.endsWith(".js") &&
      !entry.name.endsWith("_FIXTURE.js"),
  )
  .map((entry) => `${cohort}/${entry.name}`)
  .sort(bytewise);
const roots = jsonCandidates
  .filter((relativePath) => !metadata(relativePath).features.includes("dynamic-import"))
  .sort(bytewise);

function closure(rootPath) {
  const reached = new Set();
  const pending = [rootPath];
  while (pending.length > 0) {
    const base = pending.pop();
    if (reached.has(base)) continue;
    reached.add(base);
    for (const request of requestSpecifiers(base)) {
      const normalized = normalize(base, request);
      assert(existsSync(join(suite, normalized)), `${base}: missing request ${request}`);
      pending.push(normalized);
    }
  }
  return [...reached].sort(bytewise);
}

const sources = [...new Set(roots.flatMap(closure))].sort(bytewise);
const fileEdges = sources.flatMap((basePath) =>
  requestSpecifiers(basePath).map((specifier, requestIndex) => ({
    basePath,
    requestIndex,
    specifier,
    normalizedPath: normalize(basePath, specifier),
  })),
);
const edgesBySource = new Map(
  sources.map((relativePath) => [
    relativePath,
    fileEdges.filter((edge) => edge.basePath === relativePath),
  ]),
);
const rootedEdges = roots.flatMap((rootPath) =>
  closure(rootPath).flatMap((basePath) =>
    edgesBySource.get(basePath).map((edge) => ({ rootPath, ...edge })),
  ),
);
const negativeRoots = roots.filter((relativePath) => metadata(relativePath).negativePhase);
const resolutionNegatives = negativeRoots.filter(
  (relativePath) => metadata(relativePath).negativePhase === "resolution",
);
const normalRoots = roots.filter((relativePath) => !metadata(relativePath).negativePhase);
const excludedJsonRoots = jsonCandidates.filter((relativePath) => !roots.includes(relativePath));

const exclusionCanaries = [
  ["dynamic-json-module", `${cohort}/json-idempotency.js`],
  [
    "dynamic-import-json-attributes",
    "test/language/expressions/dynamic-import/import-attributes/2nd-param-with-enumeration-enumerable.js",
  ],
  ["text-module", `${cohort}/text-javascript.js`],
];

assert.equal(roots.length, expected.roots);
assert.equal(sources.length, expected.sources);
assert.equal(sources.length - roots.length, expected.fixtures);
assert.equal(fileEdges.length, expected.fileEdges);
assert.equal(rootedEdges.length, expected.rootedEdges);
assert.equal(resolutionNegatives.length, expected.resolutionNegatives);
assert.equal(negativeRoots.length, expected.resolutionNegatives);
assert.equal(normalRoots.length, expected.normalRoots);
assert.equal(
  roots.filter((relativePath) => metadata(relativePath).includes.includes("propertyHelper.js"))
    .length,
  expected.propertyHelperRoots,
);
assert.deepEqual(excludedJsonRoots, [`${cohort}/json-idempotency.js`]);
assert(
  roots.every((relativePath) => {
    const shape = metadata(relativePath);
    return (
      shape.flags.join(",") === "module" &&
      shape.features.join(",") === "import-attributes,json-modules" &&
      shape.includes.every((include) => include === "propertyHelper.js") &&
      (!shape.negativePhase ||
        (shape.negativePhase === "resolution" && shape.negativeType === "SyntaxError"))
    );
  }),
  "JSON module root metadata shape drifted",
);
assert(
  sources
    .filter((relativePath) => relativePath.endsWith(".json"))
    .every((relativePath) => frontmatter(source(relativePath)) === ""),
  "JSON fixtures unexpectedly gained Test262 metadata",
);
assert(
  fileEdges.every(
    ({ specifier, normalizedPath }) =>
      specifier.endsWith("_FIXTURE.json") && normalizedPath.endsWith("_FIXTURE.json"),
  ),
  "static JSON graph gained a non-JSON request",
);
for (const [surface, relativePath] of exclusionCanaries) {
  assert(existsSync(join(suite, relativePath)), `missing ${surface} canary: ${relativePath}`);
  assert(!roots.includes(relativePath), `${surface} canary entered the static cohort`);
  assert(!sources.includes(relativePath), `${surface} canary entered the static graph`);
}

const lines = (...values) => `${values.join("\n")}\n`;
const manifest = lines(...roots);
const sourceManifest = lines(...sources);
const edges = lines(
  "base_path\trequest_index\tspecifier\tnormalized_path",
  ...fileEdges.map(({ basePath, requestIndex, specifier, normalizedPath }) =>
    [basePath, requestIndex, specifier, normalizedPath].join("\t"),
  ),
);
const closures = lines(
  "root_path\tclosure_files\trequest_edges",
  ...roots.map((rootPath) => {
    const files = closure(rootPath);
    return [
      rootPath,
      files.length,
      files.reduce((count, relativePath) => count + edgesBySource.get(relativePath).length, 0),
    ].join("\t");
  }),
);
const ledger = lines(
  "path\trole\tincludes\tflags\tfeatures\tnegative_phase\tnegative_type\tsource_sha256\tfrontmatter_sha256",
  ...sources.map((relativePath) => {
    const shape = metadata(relativePath);
    return [
      relativePath,
      roots.includes(relativePath) ? "root" : "fixture",
      shape.includes.join(","),
      shape.flags.join(","),
      shape.features.join(","),
      shape.negativePhase,
      shape.negativeType,
      sha256(source(relativePath)),
      sha256(frontmatter(source(relativePath))),
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
const negatives = lines(...negativeRoots);
const exclusions = lines(
  "surface\tcanary_path\tincludes\tflags\tfeatures\tnegative_phase\tnegative_type\tsource_sha256\tfrontmatter_sha256",
  ...exclusionCanaries.map(([surface, relativePath]) => {
    const shape = metadata(relativePath);
    return [
      surface,
      relativePath,
      shape.includes.join(","),
      shape.flags.join(","),
      shape.features.join(","),
      shape.negativePhase,
      shape.negativeType,
      sha256(source(relativePath)),
      sha256(frontmatter(source(relativePath))),
    ].join("\t");
  }),
);

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
  ...fileEdges.map(({ basePath, requestIndex, specifier, normalizedPath }) =>
    admissionRecord({
      kind: "graph-request",
      group: admissionGroup,
      path: basePath,
      request_index: requestIndex,
      specifier,
      normalized_path: normalizedPath,
    }),
  ),
  ...roots.map((rootPath) =>
    admissionRecord({
      kind: "graph-root",
      group: admissionGroup,
      path: rootPath,
      closure_file_count: closure(rootPath).length,
      priority: 2,
    }),
  ),
];
assert.equal(admissionRecords.length, expected.admissionRows);
const admissionRows = renderAdmissionRows(admissionRecords);
const admissionCandidate = `${admissionHeader}\n${admissionRows}`;

function diagnosticRule(relativePath) {
  return relativePath.endsWith("/json-invalid.js")
    ? "module.json-parse"
    : "module.missing-export";
}

const diagnosticCandidates = lines(
  "path\tvariant\trule",
  ...negativeRoots.map((relativePath) =>
    [relativePath, "sloppy", diagnosticRule(relativePath)].join("\t"),
  ),
);
const diagnosticRules = lines(
  "rule\tquickjs_anchor\tdescription",
  "module.json-parse\tjson_parse_value\tJSON module resolution reports strict JSON grammar errors from the authenticated fixture",
  "module.missing-export\tjs_resolve_export_throw_error\tmodule resolution reports a requested binding absent from the dependency graph",
);

const mode = output ? "output" : selectedModes[0] ?? "check";
if (mode === "--admissions") {
  process.stdout.write(admissionRows);
  process.exit(0);
}
if (mode === "--diagnostic-candidates") {
  process.stdout.write(diagnosticCandidates);
  process.exit(0);
}

function quickjsOracle() {
  const quickjsRoot = dirname(quickjsConfig);
  const suiteArguments = roots.map((relativePath) => {
    const argument = relative(quickjsRoot, join(suite, relativePath)).split(sep).join("/");
    assert(
      argument && !argument.startsWith("../"),
      `suite must be below the QuickJS Test262 source root: ${suite}`,
    );
    return argument;
  });
  const result = spawnSync(
    quickjsRunner,
    ["-v", "-T", "1", "-c", basename(quickjsConfig), "-f", ...suiteArguments],
    { cwd: quickjsRoot, encoding: "utf8" },
  );
  assert.equal(result.signal, null, "QuickJS JSON module oracle terminated by signal");
  assert.equal(result.status, 0, `QuickJS JSON module oracle failed:\n${result.stderr}`);
  const transcript = `${result.stdout}${result.stderr}`.replaceAll("\r\n", "\n");
  const suitePrefix = `${relative(quickjsRoot, suite).split(sep).join("/")}/`;
  const normalizedTranscript = transcript.split(suitePrefix).join("");
  const errors = [...normalizedTranscript.matchAll(/^SyntaxError: (.+)$/gm)].map(
    (match) => match[1],
  );
  assert.deepEqual(errors, [
    "expecting property name",
    "Could not find export 'name' in module " +
      "'test/language/import/import-attributes/json-named-bindings_FIXTURE.json'",
  ]);
  const invalidLocation = normalizedTranscript.match(
    /test\/language\/import\/import-attributes\/json-invalid_FIXTURE\.json:(\d+):(\d+)/,
  );
  assert(
    invalidLocation,
    `QuickJS JSON parse diagnostic has no fixture location:\n${transcript}`,
  );
  assert.equal(invalidLocation[1], "2");
  assert.equal(invalidLocation[2], "3");
  assert(
    !/json-named-bindings_FIXTURE\.json:\d+:\d+/.test(normalizedTranscript),
    "QuickJS missing-export diagnostic unexpectedly gained a location",
  );
  return { invalidLine: invalidLocation[1], invalidColumn: invalidLocation[2] };
}

const oracle = quickjsOracle();
const negativeDiagnosticRecords = negativeRoots.map((relativePath) => {
  const shape = metadata(relativePath);
  assert.equal(shape.negativePhase, "resolution");
  assert.equal(shape.negativeType, "SyntaxError");
  const invalid = relativePath.endsWith("/json-invalid.js");
  const message = invalid
    ? "expecting property name"
    : "Could not find export 'name' in module " +
      "'test/language/import/import-attributes/json-named-bindings_FIXTURE.json'";
  return [
    relativePath,
    "sloppy",
    sha256(source(relativePath)),
    shape.negativePhase,
    shape.negativeType,
    diagnosticRule(relativePath),
    message,
    invalid ? oracle.invalidLine : "",
    invalid ? oracle.invalidColumn : "",
    invalid ? "exact" : "absent",
  ].join("\t");
});
const negativeDiagnostics = lines(
  "path\tvariant\tsource_sha256\tphase\ttype\trule\tmessage\tline\tcolumn\tlocation_policy",
  ...negativeDiagnosticRecords,
);
const quickjsEvidence = lines(
  "path\tvariant\texpected_phase\texpected_type\toracle_outcome\tmessage\tline\tcolumn\tlocation_policy",
  ...roots.map((relativePath) => {
    const shape = metadata(relativePath);
    const invalid = relativePath.endsWith("/json-invalid.js");
    const named = relativePath.endsWith("/json-named-bindings.js");
    const message = invalid
      ? "expecting property name"
      : named
        ? "Could not find export 'name' in module " +
          "'test/language/import/import-attributes/json-named-bindings_FIXTURE.json'"
        : "";
    return [
      relativePath,
      "sloppy",
      shape.negativePhase || "normal",
      shape.negativeType,
      shape.negativePhase ? "expected-error" : "pass",
      message,
      invalid ? oracle.invalidLine : "",
      invalid ? oracle.invalidColumn : "",
      invalid ? "exact" : named ? "absent" : "",
    ].join("\t");
  }),
);

if (mode === "--negative-diagnostics") {
  process.stdout.write(negativeDiagnostics);
  process.exit(0);
}

const evidence = new Map([
  ["test262-module-json-a.txt", manifest],
  ["test262-module-json-a-sources.txt", sourceManifest],
  ["test262-module-json-a-edges.tsv", edges],
  ["test262-module-json-a-closures.tsv", closures],
  ["test262-module-json-a-ledger.tsv", ledger],
  ["test262-module-json-a-variants.tsv", variants],
  ["test262-module-json-a-negatives.txt", negatives],
  ["test262-module-json-a-exclusions.tsv", exclusions],
  ["test262-module-json-a-admission-rows.tsv", admissionCandidate],
  ["test262-module-json-a-negative-diagnostic-candidates.tsv", diagnosticCandidates],
  ["test262-module-json-a-negative-diagnostic-rules.tsv", diagnosticRules],
  ["test262-module-json-a-negative-diagnostics.tsv", negativeDiagnostics],
  ["test262-module-json-a-quickjs.tsv", quickjsEvidence],
]);
for (const [name, contents] of evidence) {
  assert.equal(sha256(contents), expected.evidenceSha256[name], `${name} changed`);
}

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

if (mode === "--check-current") {
  assertAdmissionGroup(checkedAdmissions, admissionGroup, admissionRecords);
  const checked = readFileSync(checkedDiagnostics, "utf8");
  for (const record of negativeDiagnosticRecords) {
    assert(checked.includes(`\n${record}\n`), `${record.split("\t")[0]} diagnostic not promoted`);
  }
  const checkedRules = readFileSync(checkedDiagnosticRules, "utf8");
  for (const rule of diagnosticRules.trimEnd().split("\n").slice(1)) {
    assert(checkedRules.includes(`\n${rule}\n`), `${rule.split("\t")[0]} rule not promoted`);
  }
  const features = profileSection(checkedProfile, "features");
  assert(features.has("json-modules"), "json-modules feature not promoted");
  const auditedNegatives = profileSection(checkedProfile, "audited-negative-tests");
  for (const relativePath of negativeRoots) {
    assert(auditedNegatives.has(relativePath), `${relativePath} negative not promoted`);
  }
  const focused = new Set(
    readFileSync(checkedManifest, "utf8").split("\n").filter(Boolean),
  );
  for (const relativePath of roots) {
    assert(focused.has(relativePath), `${relativePath} focused root not promoted`);
  }
  console.log("module-json-a current baseline is authenticated");
} else if (mode === "output") {
  assert(output, "--output requires a directory");
  for (const [name, contents] of evidence) writeFileSync(join(output, name), contents);
  console.log(`generated ${evidence.size} candidate files in ${output}`);
} else {
  console.log(
    `module-json-a candidate: roots=${roots.length} sources=${sources.length} ` +
      `file_edges=${fileEdges.length} rooted_edges=${rootedEdges.length} ` +
      `negatives=${negativeRoots.length} admission_rows=${admissionRecords.length}`,
  );
}
