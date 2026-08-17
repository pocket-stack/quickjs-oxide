#!/usr/bin/env node

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { basename, dirname, join, relative, resolve, sep } from "node:path";

import {
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
const checkedFocused = join(root, "tests/test262-class-private-callables-b.txt");
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
const selectedModes = ["--admissions", "--check-current"].filter((mode) =>
  args.includes(mode),
);
assert(selectedModes.length <= 1, "select at most one output/check mode");
const mode = selectedModes[0] ?? "--check-current";

const valueOptions = new Set(["--suite", "--quickjs-runner", "--quickjs-config"]);
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
assert(existsSync(quickjsRunner), `missing pinned QuickJS run-test262: ${quickjsRunner}`);
assert(existsSync(quickjsConfig), `missing pinned QuickJS Test262 config: ${quickjsConfig}`);

const cohort = "test/language/module-code";
const admissionGroup = "module-local-binding-a";
const legacyAdmissionGroup = "dependency-free";
const expected = {
  allRoots: 15,
  legacyRoots: 4,
  newRoots: 11,
  allPathSha256: "751191a66b3067f726546a6536726b315f0bd290b57b1acf9f81971a15c4227c",
  legacyPathSha256:
    "88ed3898675a84f6dc7418db9bfc038aed3b20e40aa2f70a17b3c9a499965ab4",
  newPathSha256: "64f8fa725f7b369c18c616c9a57ea5c5e3db18a9e2546f8f3f27db9435a1d7a8",
  allPathSourceSha256:
    "376070ccf13bbf5d070a06fbcc934e23809e6e114455b3fad435c8120750dfed",
  legacyPathSourceSha256:
    "d037f8f933ca22037018e0ff02b382d4f00f30241427cf519387c305eae75ce9",
  newPathSourceSha256:
    "8a0ee25f611031461243fe4615e21911889a8d9a96bc3afb8d35322e02944e82",
};

const bytewise = (left, right) => Buffer.compare(Buffer.from(left), Buffer.from(right));
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

const pathManifest = (paths) => `${paths.join("\n")}\n`;
const allRoots = readdirSync(join(suite, cohort), { withFileTypes: true })
  .filter(
    (entry) => entry.isFile() && /^instn-local-bndng-.+\.js$/u.test(entry.name),
  )
  .map((entry) => `${cohort}/${entry.name}`)
  .sort(bytewise);
const legacyNames = new Set([
  "instn-local-bndng-const.js",
  "instn-local-bndng-fun.js",
  "instn-local-bndng-let.js",
  "instn-local-bndng-var.js",
]);
const legacyRoots = allRoots.filter((relativePath) =>
  legacyNames.has(relativePath.slice(cohort.length + 1)),
);
const newRoots = allRoots.filter((relativePath) => !legacyRoots.includes(relativePath));
const pathSourceManifest = (paths) =>
  pathManifest(paths.map((relativePath) => `${relativePath}\t${sha256(source(relativePath))}`));

assert.equal(allRoots.length, expected.allRoots, "local-binding family count changed");
assert.equal(legacyRoots.length, expected.legacyRoots, "legacy local-binding count changed");
assert.equal(newRoots.length, expected.newRoots, "new local-binding count changed");
assert.equal(new Set(allRoots).size, allRoots.length, "local-binding family contains duplicates");
assert.equal(
  sha256(pathManifest(allRoots)),
  expected.allPathSha256,
  "local-binding family path manifest changed",
);
assert.equal(
  sha256(pathManifest(legacyRoots)),
  expected.legacyPathSha256,
  "legacy local-binding path manifest changed",
);
assert.equal(
  sha256(pathManifest(newRoots)),
  expected.newPathSha256,
  "new local-binding path manifest changed",
);
assert.equal(
  sha256(pathSourceManifest(allRoots)),
  expected.allPathSourceSha256,
  "local-binding family path/source manifest changed",
);
assert.equal(
  sha256(pathSourceManifest(legacyRoots)),
  expected.legacyPathSourceSha256,
  "legacy local-binding path/source manifest changed",
);
assert.equal(
  sha256(pathSourceManifest(newRoots)),
  expected.newPathSourceSha256,
  "new local-binding path/source manifest changed",
);

const noHarnessInclude = new Set([
  `${cohort}/instn-local-bndng-for-dup.js`,
  `${cohort}/instn-local-bndng-var-dup.js`,
]);
for (const relativePath of allRoots) {
  const text = source(relativePath);
  const body = text.replace(frontmatter(text), "");
  assert.deepEqual(metadata(relativePath), {
    includes: noHarnessInclude.has(relativePath) ? [] : ["fnGlobalObject.js"],
    flags: ["module"],
    features: [],
    negativePhase: "",
    negativeType: "",
  });
  assert(
    !/\b(?:import|export)\b[^;]*\bfrom\s*["']/su.test(body),
    `${relativePath}: static module request added`,
  );
  assert(!/^\s*import\s*["']/mu.test(body), `${relativePath}: bare import request added`);
  assert(!/\bimport\s*\(/u.test(body), `${relativePath}: dynamic import added`);
}
assert.equal(
  allRoots.filter((relativePath) => metadata(relativePath).includes.length === 1).length,
  13,
);
assert.equal(
  allRoots.filter((relativePath) => metadata(relativePath).includes.length === 0).length,
  2,
);

const admissionRecords = newRoots.map((relativePath) => {
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
assert.equal(admissionRecords.length, expected.newRoots);

if (mode === "--admissions") {
  process.stdout.write(renderAdmissionRows(admissionRecords));
  process.exit(0);
}

function configSection(text, name) {
  const lines = text.split(/\r?\n/);
  const start = lines.indexOf(`[${name}]`);
  assert.notEqual(start, -1, `${quickjsConfig}: missing [${name}]`);
  const end = lines.findIndex((line, index) => index > start && /^\[.+\]$/u.test(line));
  return lines
    .slice(start + 1, end === -1 ? undefined : end)
    .map((line) => line.replace(/#.*$/u, "").trim())
    .filter(Boolean);
}

assertAdmissionGroup(checkedAdmissions, admissionGroup, admissionRecords);
const checkedAdmissionRows = readFileSync(checkedAdmissions, "utf8")
  .trimEnd()
  .split("\n")
  .slice(1)
  .map((line) => line.split("\t"));
for (const relativePath of allRoots) {
  const owners = checkedAdmissionRows
    .filter((fields) => fields[2] === relativePath)
    .map((fields) => `${fields[0]}\t${fields[1]}`);
  assert.deepEqual(
    owners,
    [`module\t${legacyRoots.includes(relativePath) ? legacyAdmissionGroup : admissionGroup}`],
    `${relativePath}: admission ownership changed`,
  );
}

const quickjsConfigText = readFileSync(quickjsConfig, "utf8");
const configEntries = new Map(
  configSection(quickjsConfigText, "config").map((line) => {
    const separator = line.indexOf("=");
    assert.notEqual(separator, -1, `${quickjsConfig}: malformed config line ${line}`);
    return [line.slice(0, separator).trim(), line.slice(separator + 1).trim()];
  }),
);
assert.equal(configEntries.get("module"), "yes", "pinned QuickJS config skips modules");

const quickjsRoot = dirname(quickjsConfig);
const suiteArguments = newRoots.map((relativePath) => {
  const argument = relative(quickjsRoot, join(suite, relativePath)).split(sep).join("/");
  assert(
    argument && !argument.startsWith("../"),
    `suite must be below the QuickJS Test262 source root: ${suite}`,
  );
  return argument;
});
const errorFileName = configEntries.get("errorfile");
assert(errorFileName, `${quickjsConfig}: missing errorfile`);
const errorFile = join(quickjsRoot, errorFileName);
assert(existsSync(errorFile), `missing pinned QuickJS error file: ${errorFile}`);
const knownErrorPaths = new Set(
  readFileSync(errorFile, "utf8")
    .split(/\r?\n/)
    .flatMap((line) => line.match(/^(.*?):\d+: /u)?.[1] ?? []),
);
for (const argument of suiteArguments) {
  assert(!knownErrorPaths.has(argument), `${argument}: QuickJS pass masked by known-error ledger`);
}

const result = spawnSync(
  quickjsRunner,
  ["-v", "-T", "1", "-c", basename(quickjsConfig), "-f", ...suiteArguments],
  { cwd: quickjsRoot, encoding: "utf8" },
);
assert.equal(result.error, undefined, `QuickJS local-binding oracle failed to start: ${result.error}`);
assert.equal(result.signal, null, "QuickJS local-binding oracle terminated by signal");
assert.equal(
  result.status,
  0,
  `QuickJS local-binding oracle failed:\n${result.stdout}${result.stderr}`,
);
const transcript = `${result.stdout}${result.stderr}`.replaceAll("\r\n", "\n");
assert(!/\bSKIPPED\b/u.test(transcript), `QuickJS skipped an audited root:\n${transcript}`);
assert(!/\bFAILED\b/u.test(transcript), `QuickJS failed an audited root:\n${transcript}`);
assert(
  !/^(?:.*Error|FAIL):/mu.test(transcript),
  `QuickJS reported an unexpected diagnostic:\n${transcript}`,
);

const focused = new Set(readFileSync(checkedFocused, "utf8").trimEnd().split("\n"));
for (const relativePath of newRoots) {
  assert(focused.has(relativePath), `${relativePath}: focused path not promoted`);
}

console.log(
  "module-local-binding-a current baseline authenticated: " +
    "family=15 legacy=4 roots=11 variants=11 dependencies=0 quickjs=11/11",
);
