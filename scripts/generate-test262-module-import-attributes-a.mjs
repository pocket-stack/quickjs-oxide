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

const cohort = "test/language/module-code/import-attributes";
const admissionGroup = "module-import-attributes-a";
const expected = {
  roots: 13,
  sources: 17,
  fixtures: 4,
  fileEdges: 41,
  rootedEdges: 49,
  parseNegatives: 3,
  resolutionNegatives: 9,
  normalRoots: 1,
  globalThisRoots: 9,
  rawRoots: 1,
  admissionRows: 71,
  evidenceSha256: {
    "test262-module-import-attributes-a.txt":
      "6498ab7ceb520dd1a2f5be7c29a5443b0d8c32a1c0d0080aaba34dda4823e7f6",
    "test262-module-import-attributes-a-sources.txt":
      "e051078d11fcea7319b1ca4d229ff8c9a92e023a43de79baad951a002f38c421",
    "test262-module-import-attributes-a-edges.tsv":
      "dc9edcedad6359b9cd3d24e7f8b792031bbda26cd3564f63c8b02e0a08c1aaff",
    "test262-module-import-attributes-a-closures.tsv":
      "d2b528aa127bd176ddf2d9cc05537cb77893411f139b90035fb3ccd1b602eccb",
    "test262-module-import-attributes-a-ledger.tsv":
      "5bea1281c25ecec81cc4bce2008cfde1cc26533ef274dce6d5a010bf0de95841",
    "test262-module-import-attributes-a-variants.tsv":
      "49e32c96a09c0acd4aca4624ed90efa9a8a4219bd1aff43c4efdf6852388e4ea",
    "test262-module-import-attributes-a-negatives.txt":
      "742c1a273c78783fcd2c5f03e95ca29a5fc8749719c7078814b21bba9525d464",
    "test262-module-import-attributes-a-exclusions.tsv":
      "79b6b2da32e37fb51c2a4caeed678307d5225be93ab10a9eac757ad4830a76a0",
    "test262-module-import-attributes-a-admission-rows.tsv":
      "3b7788ce064643355773bd08f806ecc7081e3d51e1afc477cf83fbb854e9bb88",
    "test262-module-import-attributes-a-negative-diagnostic-candidates.tsv":
      "42ded132c14ed2f20996ac299b7ff13464897baab5ae2e45cc8a5ceb255b26ec",
    "test262-module-import-attributes-a-negative-diagnostic-rules.tsv":
      "f74e59f9f9d4b96d164edab43c4ac4835ee646146888682de10ef1ca95ffa8b1",
    "test262-module-import-attributes-a-negative-diagnostics.tsv":
      "2237755c0fe61810a051e1836c4bb51e036e74882f93594bb68917c98d140de5",
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
const roots = readdirSync(join(suite, cohort), { withFileTypes: true })
  .filter(
    (entry) =>
      entry.isFile() && entry.name.endsWith(".js") && !entry.name.endsWith("_FIXTURE.js"),
  )
  .map((entry) => `${cohort}/${entry.name}`)
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
const parseNegatives = negativeRoots.filter(
  (relativePath) => metadata(relativePath).negativePhase === "parse",
);
const resolutionNegatives = negativeRoots.filter(
  (relativePath) => metadata(relativePath).negativePhase === "resolution",
);
const normalRoots = roots.filter((relativePath) => !metadata(relativePath).negativePhase);

const exclusionCanaries = [
  [
    "dynamic-import-attributes",
    "test/language/expressions/dynamic-import/import-attributes/2nd-param-await-expr.js",
  ],
  [
    "json-modules",
    "test/language/import/import-attributes/json-value-object.js",
  ],
  [
    "source-phase-import",
    "test/language/module-code/source-phase-import/import-source.js",
  ],
];

assert.equal(roots.length, expected.roots);
assert.equal(sources.length, expected.sources);
assert.equal(sources.length - roots.length, expected.fixtures);
assert.equal(fileEdges.length, expected.fileEdges);
assert.equal(rootedEdges.length, expected.rootedEdges);
assert.equal(parseNegatives.length, expected.parseNegatives);
assert.equal(resolutionNegatives.length, expected.resolutionNegatives);
assert.equal(normalRoots.length, expected.normalRoots);
assert.equal(
  roots.filter((relativePath) => metadata(relativePath).features.includes("globalThis")).length,
  expected.globalThisRoots,
);
assert.equal(
  roots.filter((relativePath) => metadata(relativePath).flags.includes("raw")).length,
  expected.rawRoots,
);
assert(
  roots.every((relativePath) => {
    const shape = metadata(relativePath);
    return (
      shape.includes.length === 0 &&
      shape.flags.includes("module") &&
      shape.features.includes("import-attributes") &&
      (!shape.negativePhase || shape.negativeType === "SyntaxError")
    );
  }),
  "import-attributes root metadata shape drifted",
);
assert(
  sources
    .filter((relativePath) => relativePath.endsWith("_FIXTURE.js"))
    .every((relativePath) => frontmatter(source(relativePath)) === ""),
  "import-attributes fixtures unexpectedly gained Test262 metadata",
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
  return metadata(relativePath).negativePhase === "parse"
    ? "module.import-attributes.duplicate-key"
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
  "module.import-attributes.duplicate-key\tjs_parse_with_clause\tstatic import attributes reject decoded duplicate keys",
  "module.missing-export\tjs_resolve_export_throw_error\tmodule resolution reports a requested binding absent from the dependency graph",
);

function quickjsDiagnostic(relativePath) {
  const quickjsRoot = dirname(quickjsConfig);
  const suiteArgument = relative(quickjsRoot, join(suite, relativePath)).split(sep).join("/");
  assert(
    suiteArgument && !suiteArgument.startsWith("../"),
    `suite must be below the QuickJS Test262 source root: ${suite}`,
  );
  const result = spawnSync(
    quickjsRunner,
    ["-v", "-c", basename(quickjsConfig), "-f", suiteArgument],
    { cwd: quickjsRoot, encoding: "utf8" },
  );
  assert.equal(result.signal, null, `${relativePath}: QuickJS runner terminated by signal`);
  assert.equal(result.status, 0, `${relativePath}: QuickJS runner failed:\n${result.stderr}`);
  const transcript = `${result.stdout}${result.stderr}`;
  const error = transcript.match(/^SyntaxError: (.+)$/m);
  assert(error, `${relativePath}: QuickJS runner emitted no SyntaxError:\n${transcript}`);
  const suitePrefix = `${relative(quickjsRoot, suite).split(sep).join("/")}/`;
  const message = error[1].split(suitePrefix).join("");
  const escaped = relativePath.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const location = transcript.match(new RegExp(`(?:${suitePrefix})?${escaped}:(\\d+):(\\d+)`));
  return {
    message,
    line: location?.[1] ?? "",
    column: location?.[2] ?? "",
    locationPolicy: location ? "exact" : "absent",
  };
}

const negativeDiagnosticRecords = negativeRoots.map((relativePath) => {
  const shape = metadata(relativePath);
  const actual = quickjsDiagnostic(relativePath);
  assert.equal(shape.negativeType, "SyntaxError");
  if (shape.negativePhase === "parse") {
    assert.equal(actual.message, "duplicate with key");
    assert.equal(actual.locationPolicy, "exact");
  } else {
    assert.equal(shape.negativePhase, "resolution");
    assert.equal(actual.locationPolicy, "absent");
    assert.equal(
      actual.message,
      "Could not find export 'nonExistent' in module " +
        "'test/language/module-code/import-attributes/ensure-linking-error_FIXTURE.js'",
    );
  }
  return [
    relativePath,
    "sloppy",
    sha256(source(relativePath)),
    shape.negativePhase,
    shape.negativeType,
    diagnosticRule(relativePath),
    actual.message,
    actual.line,
    actual.column,
    actual.locationPolicy,
  ].join("\t");
});
const negativeDiagnostics = lines(
  "path\tvariant\tsource_sha256\tphase\ttype\trule\tmessage\tline\tcolumn\tlocation_policy",
  ...negativeDiagnosticRecords,
);

const evidence = new Map([
  ["test262-module-import-attributes-a.txt", manifest],
  ["test262-module-import-attributes-a-sources.txt", sourceManifest],
  ["test262-module-import-attributes-a-edges.tsv", edges],
  ["test262-module-import-attributes-a-closures.tsv", closures],
  ["test262-module-import-attributes-a-ledger.tsv", ledger],
  ["test262-module-import-attributes-a-variants.tsv", variants],
  ["test262-module-import-attributes-a-negatives.txt", negatives],
  ["test262-module-import-attributes-a-exclusions.tsv", exclusions],
  ["test262-module-import-attributes-a-admission-rows.tsv", admissionCandidate],
  ["test262-module-import-attributes-a-negative-diagnostic-candidates.tsv", diagnosticCandidates],
  ["test262-module-import-attributes-a-negative-diagnostic-rules.tsv", diagnosticRules],
  ["test262-module-import-attributes-a-negative-diagnostics.tsv", negativeDiagnostics],
]);
for (const [name, contents] of evidence) {
  assert.equal(sha256(contents), expected.evidenceSha256[name], `${name} changed`);
}

const mode = output ? "output" : selectedModes[0] ?? "check";
if (mode === "--admissions") {
  process.stdout.write(admissionRows);
} else if (mode === "--diagnostic-candidates") {
  process.stdout.write(diagnosticCandidates);
} else if (mode === "--negative-diagnostics") {
  process.stdout.write(negativeDiagnostics);
} else if (mode === "--check-current") {
  assertAdmissionGroup(checkedAdmissions, admissionGroup, admissionRecords);
  const checked = readFileSync(checkedDiagnostics, "utf8");
  for (const record of negativeDiagnosticRecords) {
    assert(checked.includes(`\n${record}\n`), `${record.split("\t")[0]} diagnostic not promoted`);
  }
  const checkedRules = readFileSync(checkedDiagnosticRules, "utf8");
  for (const rule of diagnosticRules.trimEnd().split("\n").slice(1)) {
    assert(checkedRules.includes(`\n${rule}\n`), `${rule.split("\t")[0]} rule not promoted`);
  }
  console.log(`module-import-attributes-a current baseline is authenticated`);
} else if (mode === "output") {
  assert(output, "--output requires a directory");
  for (const [name, contents] of evidence) writeFileSync(join(output, name), contents);
  console.log(`generated ${evidence.size} candidate files in ${output}`);
} else {
  console.log(
    `module-import-attributes-a candidate: roots=${roots.length} sources=${sources.length} ` +
      `file_edges=${fileEdges.length} rooted_edges=${rootedEdges.length} ` +
      `negatives=${negativeRoots.length} admission_rows=${admissionRecords.length}`,
  );
}
