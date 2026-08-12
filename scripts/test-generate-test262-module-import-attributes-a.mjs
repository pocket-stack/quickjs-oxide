#!/usr/bin/env node

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const generator = join(root, "scripts/generate-test262-module-import-attributes-a.mjs");
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
const output = mkdtempSync(join(tmpdir(), "quickjs-oxide-import-attributes-a-"));
try {
  const generated = run(["--output", output]);
  assert.match(generated.stdout, /generated 12 candidate files/u);

  const admissions = run(["--admissions"]).stdout;
  const admissionCandidate = readFileSync(
    join(output, "test262-module-import-attributes-a-admission-rows.tsv"),
    "utf8",
  );
  assert.equal(admissionCandidate.split("\n").slice(1).join("\n"), admissions);
  assert.equal(admissions.trimEnd().split("\n").length, 71);

  const diagnosticCandidates = run(["--diagnostic-candidates"]).stdout;
  assert.equal(
    readFileSync(
      join(output, "test262-module-import-attributes-a-negative-diagnostic-candidates.tsv"),
      "utf8",
    ),
    diagnosticCandidates,
  );
  assert.equal(diagnosticCandidates.trimEnd().split("\n").length, 13);

  const diagnostics = run(["--negative-diagnostics"]).stdout;
  assert.equal(
    readFileSync(
      join(output, "test262-module-import-attributes-a-negative-diagnostics.tsv"),
      "utf8",
    ),
    diagnostics,
  );
  assert.equal(diagnostics.trimEnd().split("\n").length, 13);
  assert.equal(
    diagnostics.split("\n").filter((line) => line.endsWith("\texact")).length,
    3,
  );
  assert.equal(
    diagnostics.split("\n").filter((line) => line.endsWith("\tabsent")).length,
    9,
  );
  for (const line of diagnostics.split("\n").slice(1).filter(Boolean)) {
    const fields = line.split("\t");
    assert.equal(fields.length, 10);
    if (fields[9] === "absent") {
      assert.equal(fields[7], "");
      assert.equal(fields[8], "");
    }
  }

  const unknown = run(["--not-an-option"], 1);
  assert.match(unknown.stderr, /unknown option/u);

  for (const [path, contents] of before) {
    assert.deepEqual(readFileSync(path), contents, `${path} was modified by candidate generation`);
  }
} finally {
  rmSync(output, { recursive: true, force: true });
}

console.log("module-import-attributes-a candidate generator tests passed");
