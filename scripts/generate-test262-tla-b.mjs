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
const checkedExemptions = join(
  root,
  "dev-support/test262/negative-diagnostic-exemptions.tsv",
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
const outputModes = [
  "--admissions",
  "--negative-diagnostics",
  "--diagnostic-rules",
  "--focused-roots",
  "--check-current",
].filter((mode) => args.includes(mode));
assert(outputModes.length <= 1, "select at most one output/check mode");
const mode = outputModes[0] ?? "--check-current";
const valueOptions = new Set(["--suite"]);
const flagOptions = new Set(outputModes.length === 0 ? ["--check-current"] : outputModes);
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
const syntaxDirectory = `${cohort}/syntax`;
const syntaxGroup = "tla-syntax-b";
const runtimeGroup = "tla-runtime-b";
const sha256 = (value) => createHash("sha256").update(value).digest("hex");
const bytewise = (left, right) => Buffer.compare(Buffer.from(left), Buffer.from(right));
const sorted = (values) => [...values].sort(bytewise);
const source = (relativePath) => readFileSync(join(suite, relativePath), "utf8");
const pathManifest = (paths) => `${sorted(paths).join("\n")}\n`;
const sourceManifest = (paths) =>
  `${sorted(new Set(paths))
    .map((relativePath) => `${relativePath}\t${sha256(source(relativePath))}`)
    .join("\n")}\n`;
const lines = (header, records) => `${header}\n${records.join("\n")}\n`;

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
    flags: arrayField(text, "flags").sort(bytewise),
    features: arrayField(text, "features"),
    negativePhase: negative?.[1] ?? "",
    negativeType: negative?.[2] ?? "",
  };
}

function assertModuleShape(relativePath, expected = {}) {
  const shape = metadata(relativePath);
  assert(shape.flags.includes("module"), `${relativePath}: Module flag disappeared`);
  assert(!shape.flags.includes("noStrict"), `${relativePath}: noStrict Module shape appeared`);
  assert(!shape.flags.includes("onlyStrict"), `${relativePath}: onlyStrict Module shape appeared`);
  if (expected.includes !== undefined) {
    assert.deepEqual(shape.includes, expected.includes, `${relativePath}: includes changed`);
  }
  if (expected.flags) assert.deepEqual(shape.flags, expected.flags, `${relativePath}: flags changed`);
  if (expected.features) {
    assert.deepEqual(shape.features, expected.features, `${relativePath}: features changed`);
  }
  if (expected.negativePhase !== undefined) {
    assert.equal(
      shape.negativePhase,
      expected.negativePhase,
      `${relativePath}: negative phase changed`,
    );
  }
  if (expected.negativeType !== undefined) {
    assert.equal(
      shape.negativeType,
      expected.negativeType,
      `${relativePath}: negative type changed`,
    );
  }
  return shape;
}

const syntaxFiles = readdirSync(join(suite, syntaxDirectory), { withFileTypes: true })
  .filter((entry) => entry.isFile() && entry.name.endsWith(".js"))
  .map((entry) => `${syntaxDirectory}/${entry.name}`)
  .sort(bytewise);
const admittedGeneratedFamilies = [
  "block-await-expr",
  "export-class-decl-await-expr",
  "export-dft-class-decl-await-expr",
  "export-dflt-assign-expr-await-expr",
  "export-lex-decl-await-expr",
  "export-var-await-expr",
  "if-block-await-expr",
  "if-expr-await-expr",
  "top-level-await-expr",
  "try-await-expr",
  "typeof-await-expr",
  "void-await-expr",
  "while-await-expr",
];
const previouslyAdmittedFamilies = [
  "for-await-await-expr",
  "for-await-expr",
  "for-in-await-expr",
  "for-of-await-expr",
];
const generatedSuffixes = [
  "array-literal",
  "func-expression",
  "identifier",
  "literal-number",
  "literal-string",
  "nested",
  "new-expr",
  "null",
  "obj-literal",
  "regexp",
  "template-literal",
  "this",
];
const generatedPaths = (families) =>
  families.flatMap((family) =>
    generatedSuffixes.map((suffix) => `${syntaxDirectory}/${family}-${suffix}.js`),
  );
const generatedSyntaxRoots = sorted(generatedPaths(admittedGeneratedFamilies));
const previouslyAdmittedSyntax = sorted(generatedPaths(previouslyAdmittedFamilies));
const syntaxNegativeContracts = [
  {
    path: `${syntaxDirectory}/early-does-not-propagate-to-fn-declaration-body.js`,
    rule: "module.top-level-await.function-context",
    message: "unexpected 'await' keyword",
    line: "38",
    column: "17",
  },
  {
    path: `${syntaxDirectory}/early-does-not-propagate-to-fn-declaration-params.js`,
    rule: "module.top-level-await.function-context",
    message: "unexpected 'await' keyword",
    line: "38",
    column: "17",
  },
  {
    path: `${syntaxDirectory}/early-does-not-propagate-to-fn-expr-body.js`,
    rule: "module.top-level-await.function-context",
    message: "unexpected 'await' keyword",
    line: "29",
    column: "3",
  },
  {
    path: `${syntaxDirectory}/early-does-not-propagate-to-fn-expr-params.js`,
    rule: "module.top-level-await.function-context",
    message: "unexpected 'await' keyword",
    line: "28",
    column: "18",
  },
  {
    path: `${syntaxDirectory}/early-no-escaped-await.js`,
    rule: "module.top-level-await.escaped-keyword",
    message: "'await' is a reserved identifier",
    line: "25",
    column: "1",
  },
];
const syntaxNegativeRoots = syntaxNegativeContracts.map(({ path }) => path);
const syntaxDynamicImportCanaries = [
  `${syntaxDirectory}/await-expr-dyn-import.js`,
  `${syntaxDirectory}/catch-parameter.js`,
];
const syntaxRoots = sorted([...generatedSyntaxRoots, ...syntaxNegativeRoots]);

assert.equal(syntaxFiles.length, 211, "top-level-await syntax inventory changed");
assert.equal(admittedGeneratedFamilies.length, 13);
assert.equal(generatedSuffixes.length, 12);
assert.equal(generatedSyntaxRoots.length, 156);
assert.equal(previouslyAdmittedSyntax.length, 48);
assert.equal(syntaxNegativeRoots.length, 5);
assert.equal(syntaxRoots.length, 161);
assert.equal(new Set(syntaxRoots).size, syntaxRoots.length, "syntax cohort overlaps");
assert.deepEqual(
  sorted([
    ...syntaxRoots,
    ...previouslyAdmittedSyntax,
    ...syntaxDynamicImportCanaries,
  ]),
  syntaxFiles,
  "top-level-await syntax partition changed",
);

for (const relativePath of generatedSyntaxRoots) {
  const shape = assertModuleShape(relativePath, {
    includes: [],
    flags: ["generated", "module"],
    negativePhase: "",
    negativeType: "",
  });
  assert(
    ["top-level-await", "top-level-await,class"].includes(shape.features.join(",")),
    `${relativePath}: generated feature shape changed`,
  );
}
assert.equal(
  generatedSyntaxRoots.filter(
    (relativePath) => metadata(relativePath).features.join(",") === "top-level-await,class",
  ).length,
  36,
  "generated class syntax count changed",
);
for (const relativePath of syntaxNegativeRoots) {
  assertModuleShape(relativePath, {
    includes: [],
    flags: ["module"],
    features: ["top-level-await"],
    negativePhase: "parse",
    negativeType: "SyntaxError",
  });
}
for (const relativePath of syntaxDynamicImportCanaries) {
  assertModuleShape(relativePath, {
    includes: [],
    flags: ["module"],
    features: ["top-level-await", "dynamic-import"],
    negativePhase: "",
    negativeType: "",
  });
}

const runtimeRootNames = [
  "await-awaits-thenable-not-callable.js",
  "await-awaits-thenables-that-throw.js",
  "await-awaits-thenables.js",
  "await-expr-func-expression.js",
  "await-expr-new-expr-reject.js",
  "await-expr-new-expr.js",
  "await-expr-regexp.js",
  "await-expr-reject-throws.js",
  "await-void-expr.js",
  "if-await-expr.js",
  "new-await-parens.js",
  "top-level-ticks-2.js",
  "top-level-ticks.js",
  "void-await-expr.js",
  "while-dynamic-evaluation.js",
];
const runtimeRoots = runtimeRootNames.map((name) => `${cohort}/${name}`);
assert.deepEqual(runtimeRoots, sorted(runtimeRoots), "runtime roots must stay bytewise sorted");
assert.equal(runtimeRoots.length, 15);
for (const relativePath of runtimeRoots) {
  assertModuleShape(relativePath, {
    includes: relativePath.includes("/top-level-ticks") ? ["compareArray.js"] : [],
    flags: ["async", "module"],
    features: ["top-level-await"],
    negativePhase: "",
    negativeType: "",
  });
}

function requestSpecifiers(relativePath) {
  const body = source(relativePath).replace(/\/\*---[\s\S]*?---\*\//u, "");
  const requests = [];
  for (const line of body.split(/\r?\n|\r/u)) {
    const match =
      line.match(/\sfrom\s*["']([^"']+)["']/u) ??
      line.match(/^\s*import\s*["']([^"']+)["']/u);
    if (!match || requests.includes(match[1])) continue;
    assert(match[1].startsWith("./"), `${relativePath}: non-child request ${match[1]}`);
    assert(!match[1].includes("/../"), `${relativePath}: escaping request ${match[1]}`);
    requests.push(match[1]);
  }
  return requests;
}
const normalize = (base, request) => posix.join(posix.dirname(base), request);

function graphClosure(rootPath) {
  const reached = new Set();
  const pending = [rootPath];
  const edges = new Map();
  while (pending.length > 0) {
    const relativePath = pending.pop();
    if (reached.has(relativePath)) continue;
    assert(relativePath.startsWith(`${cohort}/`), `${rootPath}: graph escaped TLA cohort`);
    assert(existsSync(join(suite, relativePath)), `${rootPath}: missing ${relativePath}`);
    reached.add(relativePath);
    const requests = requestSpecifiers(relativePath).map((specifier) => ({
      specifier,
      normalized: normalize(relativePath, specifier),
    }));
    edges.set(relativePath, requests);
    for (const { normalized } of requests) pending.push(normalized);
  }
  return { files: sorted(reached), edges };
}

const graphSpecs = [
  {
    group: "tla-graph-b-dfs-invariant",
    root: `${cohort}/dfs-invariant.js`,
    fileCount: 5,
    edgeCount: 6,
  },
  {
    group: "tla-graph-b-async-resolution",
    root: `${cohort}/module-async-import-async-resolution-ticks.js`,
    fileCount: 2,
    edgeCount: 1,
  },
  {
    group: "tla-graph-b-import-unwrapped",
    root: `${cohort}/module-import-unwrapped.js`,
    fileCount: 2,
    edgeCount: 1,
  },
  {
    group: "tla-graph-b-self-resolution",
    root: `${cohort}/module-self-import-async-resolution-ticks.js`,
    fileCount: 1,
    edgeCount: 1,
  },
  {
    group: "tla-graph-b-sync-resolution",
    root: `${cohort}/module-sync-import-async-resolution-ticks.js`,
    fileCount: 2,
    edgeCount: 1,
  },
  {
    group: "tla-graph-b-pending-cycle",
    root: `${cohort}/pending-async-dep-from-cycle.js`,
    fileCount: 5,
    edgeCount: 6,
  },
];
const rejectionContracts = [
  {
    group: "tla-rejection-b-body",
    root: `${cohort}/module-import-rejection-body.js`,
    rule: "module.tla-dependency-rejection",
    message: "I reject this!",
    line: "8",
    column: "45",
  },
  {
    group: "tla-rejection-b-default",
    root: `${cohort}/module-import-rejection.js`,
    rule: "module.tla-dependency-rejection",
    message: "error in the default export line",
    line: "7",
    column: "50",
  },
];
const rejectionSpecs = rejectionContracts.map(({ group, root }) => ({
  group,
  root,
  fileCount: 2,
  edgeCount: 1,
}));
const allGraphSpecs = [...graphSpecs, ...rejectionSpecs].map((spec) => ({
  ...spec,
  ...graphClosure(spec.root),
}));
for (const spec of allGraphSpecs) {
  assert.equal(spec.files.length, spec.fileCount, `${spec.root}: closure size changed`);
  assert.equal(
    [...spec.edges.values()].reduce((count, requests) => count + requests.length, 0),
    spec.edgeCount,
    `${spec.root}: request edge count changed`,
  );
}
for (const relativePath of graphSpecs.map(({ root: rootPath }) => rootPath)) {
  const shape = assertModuleShape(relativePath, {
    negativePhase: "",
    negativeType: "",
  });
  assert(shape.features.includes("top-level-await"), `${relativePath}: TLA feature disappeared`);
}
for (const { root: rootPath } of rejectionContracts) {
  assertModuleShape(rootPath, {
    includes: [],
    flags: ["module"],
    features: ["top-level-await"],
    negativePhase: "runtime",
    negativeType: "TypeError",
  });
}
for (const relativePath of runtimeRoots) {
  assert.deepEqual(requestSpecifiers(relativePath), [], `${relativePath}: gained a module request`);
}

const rejectionTickCanary = `${cohort}/module-import-rejection-tick.js`;
const hiddenDynamicImportCanaries = [
  `${cohort}/fulfillment-order.js`,
  `${cohort}/module-graphs-does-not-hang.js`,
  `${cohort}/rejection-order.js`,
];
const excludedCanaries = sorted([
  ...syntaxDynamicImportCanaries,
  rejectionTickCanary,
  ...hiddenDynamicImportCanaries,
]);
assertModuleShape(rejectionTickCanary, {
  includes: [],
  flags: ["module"],
  features: ["top-level-await"],
  negativePhase: "runtime",
  negativeType: "RangeError",
});
for (const relativePath of hiddenDynamicImportCanaries) {
  assertModuleShape(relativePath);
  assert.match(source(relativePath), /\bimport\s*\(/u, `${relativePath}: hidden import() disappeared`);
}

const graphEdgeManifest = `${allGraphSpecs
  .flatMap((spec) =>
    spec.files.flatMap((relativePath) =>
      spec.edges.get(relativePath).map(
        ({ specifier, normalized }, requestIndex) =>
          `${spec.group}\t${relativePath}\t${requestIndex}\t${specifier}\t${normalized}`,
      ),
    ),
  )
  .sort(bytewise)
  .join("\n")}\n`;
const candidateSources = sorted(
  new Set([
    ...syntaxRoots,
    ...runtimeRoots,
    ...allGraphSpecs.flatMap(({ files }) => files),
  ]),
);
const focusedRoots = sorted([
  ...syntaxRoots,
  ...runtimeRoots,
  ...graphSpecs.map(({ root: rootPath }) => rootPath),
  ...rejectionContracts.map(({ root: rootPath }) => rootPath),
]);
assert.equal(focusedRoots.length, 184);
assert.equal(new Set(focusedRoots).size, focusedRoots.length, "focused TLA B roots overlap");
assert(
  excludedCanaries.every((relativePath) => !focusedRoots.includes(relativePath)),
  "excluded TLA canary entered focused roots",
);

const pinned = {
  syntaxInventoryPaths: "8a77ed13fbbfb7464c10f862a3f88e96e5f90c8ef68d160240b43aeda52099ba",
  syntaxCandidatePaths: "40c252335cab240fd960c260c5a43c5ba6ad6361f0a65e8f20491755b9314555",
  candidateSources: "df8aaf6a1747a48ebe1ba44f8858c97351df51f31b639beba46ca90c282ebb72",
  graphEdges: "a10e284c701dce09569a488d9b9e511123c3f63f41b7cc462d4b1251295d05ca",
  excludedCanarySources: "6181b399186d349bfb828a7216847d2b7d3ba8861746c93c383ce2b1b9168718",
};
assert.equal(sha256(pathManifest(syntaxFiles)), pinned.syntaxInventoryPaths);
assert.equal(sha256(pathManifest(syntaxRoots)), pinned.syntaxCandidatePaths);
assert.equal(sha256(sourceManifest(candidateSources)), pinned.candidateSources);
assert.equal(sha256(graphEdgeManifest), pinned.graphEdges);
assert.equal(sha256(sourceManifest(excludedCanaries)), pinned.excludedCanarySources);

function moduleAdmission(relativePath, group) {
  const shape = metadata(relativePath);
  return admissionRecord({
    kind: "module",
    group,
    path: relativePath,
    source_sha256: sha256(source(relativePath)),
    includes: shape.includes,
    flags: shape.flags,
    features: shape.features,
    negative_phase: shape.negativePhase,
    negative_type: shape.negativeType,
  });
}

function graphAdmissionRecords(spec) {
  return [
    ...spec.files.map((relativePath) => {
      const shape = metadata(relativePath);
      return admissionRecord({
        kind: "graph-file",
        group: spec.group,
        path: relativePath,
        source_sha256: sha256(source(relativePath)),
        includes: shape.includes,
        flags: shape.flags,
        features: shape.features,
        negative_phase: shape.negativePhase,
        negative_type: shape.negativeType,
      });
    }),
    ...spec.files.flatMap((relativePath) =>
      spec.edges.get(relativePath).map((request, requestIndex) =>
        admissionRecord({
          kind: "graph-request",
          group: spec.group,
          path: relativePath,
          request_index: requestIndex,
          specifier: request.specifier,
          normalized_path: request.normalized,
        }),
      ),
    ),
    admissionRecord({
      kind: "graph-root",
      group: spec.group,
      path: spec.root,
      closure_file_count: spec.files.length,
      priority: 4,
    }),
  ];
}

const admissionGroups = new Map([
  [syntaxGroup, syntaxRoots.map((relativePath) => moduleAdmission(relativePath, syntaxGroup))],
  [runtimeGroup, runtimeRoots.map((relativePath) => moduleAdmission(relativePath, runtimeGroup))],
  ...allGraphSpecs.map((spec) => [spec.group, graphAdmissionRecords(spec)]),
]);
const admissionRecords = [...admissionGroups.values()].flat();
assert.equal(admissionGroups.size, 10);
assert.equal(admissionRecords.length, 223);
assert.equal(admissionRecords.filter(({ kind }) => kind === "module").length, 176);
assert.equal(admissionRecords.filter(({ kind }) => kind === "graph-file").length, 21);
assert.equal(admissionRecords.filter(({ kind }) => kind === "graph-request").length, 18);
assert.equal(admissionRecords.filter(({ kind }) => kind === "graph-root").length, 8);

const diagnosticRules = [
  "module.tla-dependency-rejection\tjs_async_module_execution_rejected\tasync module evaluation propagates the dependency rejection unchanged",
  "module.top-level-await.escaped-keyword\tjs_parse_error_reserved_identifier\tthe await terminal cannot contain Unicode escapes",
  "module.top-level-await.function-context\tjs_parse_unary\tthe Await grammar parameter does not propagate into nested function syntax",
].sort(bytewise);
const negativeDiagnosticRecords = [
  ...syntaxNegativeContracts.map(({ path, rule, message, line, column }) =>
    [
      path,
      "sloppy",
      sha256(source(path)),
      "parse",
      "SyntaxError",
      rule,
      message,
      line,
      column,
      "exact",
    ].join("\t"),
  ),
  ...rejectionContracts.map(({ root: rootPath, rule, message, line, column }) =>
    [
      rootPath,
      "sloppy",
      sha256(source(rootPath)),
      "runtime",
      "TypeError",
      rule,
      message,
      line,
      column,
      "exact",
    ].join("\t"),
  ),
].sort(bytewise);
assert.equal(negativeDiagnosticRecords.length, 7);
assert.equal(diagnosticRules.length, 3);
const negativeDiagnostics = lines(
  "path\tvariant\tsource_sha256\tphase\ttype\trule\tmessage\tline\tcolumn\tlocation_policy",
  negativeDiagnosticRecords,
);
const diagnosticRuleData = lines(
  "rule\tquickjs_anchor\tdescription",
  diagnosticRules,
);

function lineSet(path) {
  return new Set(readFileSync(path, "utf8").split("\n").filter(Boolean));
}

function profileSection(name) {
  const lines_ = readFileSync(checkedProfile, "utf8").trimEnd().split("\n");
  const start = lines_.indexOf(`[${name}]`);
  assert.notEqual(start, -1, `missing profile section [${name}]`);
  const next = lines_.findIndex((line, index) => index > start && /^\[[^\]]+\]$/u.test(line));
  return new Set(lines_.slice(start + 1, next === -1 ? lines_.length : next).filter(Boolean));
}

function assertCurrent() {
  for (const [group, records] of admissionGroups) {
    assertAdmissionGroup(checkedAdmissions, group, records);
  }
  const focused = lineSet(checkedFocused);
  for (const relativePath of focusedRoots) {
    assert(focused.has(relativePath), `${relativePath}: focused root not promoted`);
  }
  const auditedNegatives = profileSection("audited-negative-tests");
  for (const { path } of syntaxNegativeContracts) {
    assert(auditedNegatives.has(path), `${path}: syntax negative not promoted`);
  }
  for (const { root: rootPath } of rejectionContracts) {
    assert(auditedNegatives.has(rootPath), `${rootPath}: runtime negative not promoted`);
  }
  const diagnostics = lineSet(checkedDiagnostics);
  for (const record of negativeDiagnosticRecords) {
    assert(diagnostics.has(record), `${record.split("\t")[0]}: diagnostic not promoted exactly`);
  }
  const rules = lineSet(checkedRules);
  for (const record of diagnosticRules) {
    assert(rules.has(record), `${record.split("\t")[0]}: rule not promoted exactly`);
  }
  const admissions = readFileSync(checkedAdmissions, "utf8");
  const exemptions = readFileSync(checkedExemptions, "utf8");
  for (const relativePath of excludedCanaries) {
    assert(!admissions.includes(`\t${relativePath}\t`), `${relativePath}: canary admitted`);
    assert(!focused.has(relativePath), `${relativePath}: canary focused`);
    assert(!auditedNegatives.has(relativePath), `${relativePath}: canary entered profile`);
    assert(
      ![...diagnostics].some((line) => line.startsWith(`${relativePath}\t`)),
      `${relativePath}: canary gained a diagnostic contract`,
    );
    assert(
      !exemptions.includes(`\n${relativePath}\t`),
      `${relativePath}: canary gained a diagnostic exemption`,
    );
  }
}

if (mode === "--admissions") {
  process.stdout.write(renderAdmissionRows(admissionRecords));
} else if (mode === "--negative-diagnostics") {
  process.stdout.write(negativeDiagnostics);
} else if (mode === "--diagnostic-rules") {
  process.stdout.write(diagnosticRuleData);
} else if (mode === "--focused-roots") {
  process.stdout.write(pathManifest(focusedRoots));
} else {
  assertCurrent();
  console.log(
    "tla-b current baseline authenticated: roots=184 variants=184 " +
      "admission_records=223 diagnostics=7 rules=3 excluded_canaries=6",
  );
}
