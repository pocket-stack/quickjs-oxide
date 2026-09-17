#!/usr/bin/env node

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "../..");
const generator = join(root, "scripts/test262/generate-test262-module-depfree-bindings-a.mjs");
const oracleArguments = process.argv.slice(2);
const protectedFiles = [
  "dev-support/test262/current.conf",
  "dev-support/test262/admissions.tsv",
  "dev-support/test262/negative-diagnostics.tsv",
  "dev-support/test262/negative-diagnostic-rules.tsv",
].map((relativePath) => join(root, relativePath));

function run(arguments_, expectedStatus = 0) {
  const result = spawnSync(process.execPath, [generator, ...oracleArguments, ...arguments_], {
    cwd: root,
    encoding: "utf8",
  });
  assert.equal(result.signal, null);
  assert.equal(result.status, expectedStatus, result.stderr);
  return result;
}

const before = new Map(protectedFiles.map((path) => [path, readFileSync(path)]));
const output = mkdtempSync(join(tmpdir(), "quickjs-oxide-depfree-bindings-a-"));
try {
  const generated = run(["--output", output]);
  assert.match(generated.stdout, /generated 11 authenticated evidence files/u);

  const manifest = readFileSync(
    join(output, "test262-module-depfree-bindings-a.txt"),
    "utf8",
  );
  assert.equal(manifest.trimEnd().split("\n").length, 57);

  const admissions = run(["--admissions"]).stdout;
  const admissionCandidate = readFileSync(
    join(output, "test262-module-depfree-bindings-a-admission-rows.tsv"),
    "utf8",
  );
  assert.equal(admissionCandidate.split("\n").slice(1).join("\n"), admissions);
  assert.equal(admissions.trimEnd().split("\n").length, 237);
  const admissionKinds = new Set();
  for (const line of admissions.trimEnd().split("\n")) {
    admissionKinds.add(line.split("\t")[0]);
  }
  assert.deepEqual([...admissionKinds].sort(), [
    "graph-file",
    "graph-request",
    "graph-root",
    "module",
  ]);

  const diagnosticCandidates = run(["--diagnostic-candidates"]).stdout;
  assert.equal(
    readFileSync(
      join(output, "test262-module-depfree-bindings-a-negative-diagnostic-candidates.tsv"),
      "utf8",
    ),
    diagnosticCandidates,
  );
  assert.equal(diagnosticCandidates.trimEnd().split("\n").length, 13);

  const diagnostics = run(["--negative-diagnostics"]).stdout;
  assert.equal(
    readFileSync(
      join(output, "test262-module-depfree-bindings-a-negative-diagnostics.tsv"),
      "utf8",
    ),
    diagnostics,
  );
  assert.equal(diagnostics.trimEnd().split("\n").length, 13);
  // All twelve contracts are resolution-phase SyntaxErrors; the five
  // dependency-syntax cases carry exact locations, the seven missing/circular
  // export cases carry absent locations.
  const rows = diagnostics.split("\n").slice(1).filter(Boolean);
  assert.equal(rows.length, 12);
  for (const line of rows) {
    const fields = line.split("\t");
    assert.equal(fields.length, 10);
    assert.equal(fields[3], "resolution");
    assert.equal(fields[4], "SyntaxError");
  }
  assert.equal(rows.filter((line) => line.endsWith("\texact")).length, 5);
  assert.equal(rows.filter((line) => line.endsWith("\tabsent")).length, 7);

  const rules = run(["--diagnostic-rules"]).stdout;
  assert.equal(rules.trimEnd().split("\n").length, 4);
  const ruleNames = rules
    .trimEnd()
    .split("\n")
    .slice(1)
    .map((line) => line.split("\t")[0])
    .sort();
  assert.deepEqual(ruleNames, [
    "module.circular-export",
    "module.dependency-parse-break",
    "module.dependency-parse-lvalue",
  ]);

  // The two root-throw runtime siblings must stay out of the cohort.
  assert(!manifest.includes("eval-export-dflt-expr-err-eval.js"));
  assert(!manifest.includes("eval-export-dflt-expr-err-get-value.js"));

  const unknown = run(["--not-an-option"], 1);
  assert.match(unknown.stderr, /unknown option/u);

  for (const [path, contents] of before) {
    assert.deepEqual(readFileSync(path), contents, `${path} was modified by candidate generation`);
  }
} finally {
  rmSync(output, { recursive: true, force: true });
}

console.log("module-depfree-bindings-a candidate generator tests passed");
