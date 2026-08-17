#!/usr/bin/env node

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import {
  access,
  chmod,
  copyFile,
  link,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import { constants as fsConstants } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { spawn } from "node:child_process";

const root = path.resolve(import.meta.dirname, "..");
const audit = path.join(root, "scripts/audit-negative-diagnostics.mjs");
const rules = path.join(root, "dev-support/test262/negative-diagnostic-rules.tsv");
const contracts = path.join(root, "dev-support/test262/negative-diagnostics.tsv");
const smoke = path.join(
  root,
  "dev-support/test262/negative-diagnostic-generator-smoke.tsv",
);

function argumentsFrom(commandLine) {
  const values = new Map();
  for (let index = 0; index < commandLine.length; index += 2) {
    const name = commandLine[index];
    const value = commandLine[index + 1];
    if (!/^(?:--suite|--qjs|--oxide)$/.test(name) || !value || values.has(name)) {
      throw new Error(
        "usage: test-negative-diagnostic-generator.mjs " +
          "--suite DIR --qjs FILE --oxide FILE",
      );
    }
    values.set(name, path.resolve(value));
  }
  for (const required of ["--suite", "--qjs", "--oxide"]) {
    if (!values.has(required)) throw new Error(`missing ${required}`);
  }
  return {
    oxide: values.get("--oxide"),
    qjs: values.get("--qjs"),
    suite: values.get("--suite"),
  };
}

function run(arguments_) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [audit, ...arguments_], {
      cwd: root,
      stdio: ["ignore", "pipe", "pipe"],
    });
    const stdout = [];
    const stderr = [];
    child.stdout.on("data", (chunk) => stdout.push(chunk));
    child.stderr.on("data", (chunk) => stderr.push(chunk));
    child.once("error", reject);
    child.once("close", (status, signal) => {
      resolve({
        signal,
        status,
        stderr: Buffer.concat(stderr).toString("utf8"),
        stdout: Buffer.concat(stdout).toString("utf8"),
      });
    });
  });
}

function generationArguments({ candidates, output, oxide, qjs, suite }) {
  return [
    "--generate",
    candidates,
    "--output",
    output,
    "--rules",
    rules,
    "--suite",
    suite,
    "--qjs",
    qjs,
    "--oxide",
    oxide,
    "--workers",
    "2",
  ];
}

async function expectFailure(arguments_, message) {
  const result = await run(arguments_);
  assert.equal(result.signal, null);
  assert.notEqual(result.status, 0, result.stdout);
  assert.match(result.stderr, message);
}

async function main() {
  const engines = argumentsFrom(process.argv.slice(2));
  await Promise.all([
    access(engines.suite, fsConstants.R_OK),
    access(engines.qjs, fsConstants.X_OK),
    access(engines.oxide, fsConstants.X_OK),
  ]);
  const temporary = await mkdtemp(path.join(tmpdir(), "quickjs-oxide-negative-generator-"));
  try {
    const output = path.join(temporary, "smoke.tsv");
    const success = await run(
      generationArguments({ candidates: smoke, output, ...engines }),
    );
    assert.equal(success.signal, null);
    assert.equal(success.status, 0, success.stderr);

    const candidateKeys = new Set(
      (await readFile(smoke, "utf8"))
        .trimEnd()
        .split("\n")
        .slice(1)
        .map((line) => line.split("\t").slice(0, 2).join("\t")),
    );
    const canonicalLines = (await readFile(contracts, "utf8")).trimEnd().split("\n");
    const expected = `${[
      canonicalLines[0],
      ...canonicalLines.slice(1).filter((line) => {
        const key = line.split("\t").slice(0, 2).join("\t");
        return candidateKeys.has(key);
      }),
    ].join("\n")}\n`;
    assert.equal(await readFile(output, "utf8"), expected);

    const resolutionRow = canonicalLines.slice(1).find((line) => {
      const fields = line.split("\t");
      return fields[3] === "resolution";
    });
    assert(resolutionRow, "missing a resolution diagnostic contract");
    const resolutionFields = resolutionRow.split("\t");
    const resolutionRuleName = resolutionFields[5];
    const resolutionRule = (await readFile(rules, "utf8"))
      .trimEnd()
      .split("\n")
      .find((line) => line.startsWith(`${resolutionRuleName}\t`));
    assert(resolutionRule, `missing ${resolutionRuleName} diagnostic rule`);
    const resolutionContracts = path.join(temporary, "resolution-contracts.tsv");
    const resolutionRules = path.join(temporary, "resolution-rules.tsv");
    await writeFile(
      resolutionContracts,
      `${canonicalLines[0]}\n${resolutionRow}\n`,
    );
    await writeFile(
      resolutionRules,
      `rule\tquickjs_anchor\tdescription\n${resolutionRule}\n`,
    );
    const resolutionReplay = await run([
      "--contracts",
      resolutionContracts,
      "--rules",
      resolutionRules,
      "--suite",
      engines.suite,
      "--qjs",
      engines.qjs,
      "--workers",
      "1",
    ]);
    assert.equal(resolutionReplay.signal, null);
    assert.equal(resolutionReplay.status, 0, resolutionReplay.stderr);
    assert.match(resolutionReplay.stdout, /1 exact contracts \/ 1 rules/u);

    const tamperedFields = resolutionRow.split("\t");
    tamperedFields[6] += " (tampered)";
    const tamperedContracts = path.join(temporary, "resolution-tampered.tsv");
    await writeFile(
      tamperedContracts,
      `${canonicalLines[0]}\n${tamperedFields.join("\t")}\n`,
    );
    await expectFailure(
      [
        "--contracts",
        tamperedContracts,
        "--rules",
        resolutionRules,
        "--suite",
        engines.suite,
        "--qjs",
        engines.qjs,
        "--workers",
        "1",
      ],
      new RegExp(`${resolutionRuleName.replaceAll(".", "\\.")} mismatch`, "u"),
    );

    const bomCandidates = path.join(temporary, "bom-candidates.tsv");
    await writeFile(
      bomCandidates,
      Buffer.concat([Buffer.from([0xef, 0xbb, 0xbf]), await readFile(smoke)]),
    );
    await expectFailure(
      generationArguments({
        candidates: bomCandidates,
        output: path.join(temporary, "bom.tsv"),
        ...engines,
      }),
      /diagnostic candidates header drifted/u,
    );

    const syntheticSuite = path.join(temporary, "suite");
    await mkdir(path.join(syntheticSuite, "test"), { recursive: true });

    const runtimeModulePath = "test/runtime-module.js";
    const runtimeModuleSource =
      "/*---\nflags: [module]\nnegative:\n  phase: runtime\n  type: TypeError\n---*/\n" +
      'import "./runtime-module_FIXTURE.js";\n' +
      'throw new RangeError("runtime body must be unreachable");\n';
    const runtimeFixture = path.join(
      syntheticSuite,
      "test/runtime-module_FIXTURE.js",
    );
    await writeFile(
      path.join(syntheticSuite, runtimeModulePath),
      runtimeModuleSource,
    );
    await writeFile(
      runtimeFixture,
      'await Promise.reject(new TypeError("runtime oracle"));\n',
    );
    const runtimeRuleName = "runtime.module-rejection";
    const runtimeRules = path.join(temporary, "runtime-rules.tsv");
    await writeFile(
      runtimeRules,
      "rule\tquickjs_anchor\tdescription\n" +
        `${runtimeRuleName}\tjs_async_module_execution_rejected\t` +
        "Runtime module dependency rejection\n",
    );
    const runtimeFields = [
      runtimeModulePath,
      "sloppy",
      createHash("sha256").update(runtimeModuleSource).digest("hex"),
      "runtime",
      "TypeError",
      runtimeRuleName,
      "runtime oracle",
      "1",
      "35",
      "exact",
    ];
    const runtimeContracts = path.join(temporary, "runtime-contracts.tsv");
    const runtimeArguments = (contractFile) => [
      "--contracts",
      contractFile,
      "--rules",
      runtimeRules,
      "--suite",
      syntheticSuite,
      "--qjs",
      engines.qjs,
      "--workers",
      "1",
    ];
    const writeRuntimeContract = async (file, fields) =>
      writeFile(file, `${canonicalLines[0]}\n${fields.join("\t")}\n`);
    await writeRuntimeContract(runtimeContracts, runtimeFields);
    const runtimeReplay = await run(runtimeArguments(runtimeContracts));
    assert.equal(runtimeReplay.signal, null);
    assert.equal(runtimeReplay.status, 0, runtimeReplay.stderr);
    assert.match(runtimeReplay.stdout, /1 exact contracts \/ 1 rules/u);

    const directRuntimePath = "test/runtime-direct-deception.js";
    const directRuntimeSource =
      "/*---\nflags: [module]\nnegative:\n  phase: runtime\n  type: TypeError\n---*/\n" +
      'throw new TypeError("runtime oracle");\n';
    await writeFile(
      path.join(syntheticSuite, directRuntimePath),
      directRuntimeSource,
    );
    const directRuntimeFields = [...runtimeFields];
    directRuntimeFields[0] = directRuntimePath;
    directRuntimeFields[2] = createHash("sha256")
      .update(directRuntimeSource)
      .digest("hex");
    directRuntimeFields[7] = "7";
    directRuntimeFields[8] = "20";
    const directRuntimeContracts = path.join(
      temporary,
      "runtime-direct-deception.tsv",
    );
    await writeRuntimeContract(directRuntimeContracts, directRuntimeFields);
    await expectFailure(
      runtimeArguments(directRuntimeContracts),
      /runtime dependency rejection provenance mismatch/u,
    );

    const wrongPhaseFields = [...runtimeFields];
    wrongPhaseFields[3] = "parse";
    wrongPhaseFields[4] = "SyntaxError";
    const wrongPhaseContracts = path.join(temporary, "runtime-wrong-phase.tsv");
    await writeRuntimeContract(wrongPhaseContracts, wrongPhaseFields);
    await expectFailure(
      runtimeArguments(wrongPhaseContracts),
      /diagnostic metadata does not match the contract/u,
    );

    await writeFile(
      runtimeFixture,
      'await Promise.reject(new RangeError("wrong runtime type"));\n',
    );
    await expectFailure(
      runtimeArguments(runtimeContracts),
      /runtime\.module-rejection mismatch/u,
    );
    await writeFile(
      runtimeFixture,
      "await Promise.reject(new TypeError());\n",
    );
    await expectFailure(
      runtimeArguments(runtimeContracts),
      /must emit exactly one native error/u,
    );
    await writeFile(
      runtimeFixture,
      'await Promise.reject(new TypeError("runtime oracle"));\n',
    );

    for (const [name, body] of [
      ["parse", "0 = 1;\n"],
      ["resolution", 'import "./missing-runtime_FIXTURE.js";\n'],
    ]) {
      const deceptivePath = `test/runtime-${name}-deception.js`;
      const deceptiveSource =
        "/*---\nflags: [module]\nnegative:\n  phase: runtime\n  type: TypeError\n---*/\n" +
        body;
      await writeFile(
        path.join(syntheticSuite, deceptivePath),
        deceptiveSource,
      );
      const deceptiveFields = [...runtimeFields];
      deceptiveFields[0] = deceptivePath;
      deceptiveFields[2] = createHash("sha256")
        .update(deceptiveSource)
        .digest("hex");
      const deceptiveContracts = path.join(
        temporary,
        `runtime-${name}-deception.tsv`,
      );
      await writeRuntimeContract(deceptiveContracts, deceptiveFields);
      await expectFailure(
        runtimeArguments(deceptiveContracts),
        /runtime\.module-rejection mismatch/u,
      );
    }

    const wrongMessageFields = [...runtimeFields];
    wrongMessageFields[6] = "fabricated runtime oracle";
    const wrongMessageContracts = path.join(temporary, "runtime-wrong-message.tsv");
    await writeRuntimeContract(wrongMessageContracts, wrongMessageFields);
    await expectFailure(
      runtimeArguments(wrongMessageContracts),
      /runtime\.module-rejection mismatch/u,
    );

    const wrongLocationFields = [...runtimeFields];
    wrongLocationFields[7] = "1";
    wrongLocationFields[8] = "1";
    const wrongLocationContracts = path.join(temporary, "runtime-wrong-location.tsv");
    await writeRuntimeContract(wrongLocationContracts, wrongLocationFields);
    await expectFailure(
      runtimeArguments(wrongLocationContracts),
      /runtime\.module-rejection mismatch/u,
    );

    const absentLocationFields = [...runtimeFields];
    absentLocationFields[7] = "";
    absentLocationFields[8] = "";
    absentLocationFields[9] = "absent";
    const absentLocationContracts = path.join(temporary, "runtime-absent-location.tsv");
    await writeRuntimeContract(absentLocationContracts, absentLocationFields);
    await expectFailure(
      runtimeArguments(absentLocationContracts),
      /runtime TypeError audit requires an exact location/u,
    );

    const wrongRuntimeRules = path.join(temporary, "runtime-wrong-anchor-rules.tsv");
    await writeFile(
      wrongRuntimeRules,
      "rule\tquickjs_anchor\tdescription\n" +
        `${runtimeRuleName}\tjs_parse_unary\tWrong runtime anchor\n`,
    );
    const wrongAnchorArguments = runtimeArguments(runtimeContracts);
    wrongAnchorArguments[wrongAnchorArguments.indexOf(runtimeRules)] = wrongRuntimeRules;
    await expectFailure(
      wrongAnchorArguments,
      /runtime TypeError audit requires js_async_module_execution_rejected/u,
    );

    const unsupportedRuntimeFields = [...runtimeFields];
    unsupportedRuntimeFields[4] = "ReferenceError";
    const unsupportedRuntimeContracts = path.join(
      temporary,
      "runtime-unsupported-kind.tsv",
    );
    await writeRuntimeContract(unsupportedRuntimeContracts, unsupportedRuntimeFields);
    await expectFailure(
      runtimeArguments(unsupportedRuntimeContracts),
      /QuickJS audit does not yet support runtime\/ReferenceError/u,
    );

    const emptyMessageFields = [...runtimeFields];
    emptyMessageFields[6] = "";
    const emptyMessageContracts = path.join(temporary, "runtime-empty-message.tsv");
    await writeRuntimeContract(emptyMessageContracts, emptyMessageFields);
    await expectFailure(
      runtimeArguments(emptyMessageContracts),
      /has an empty message/u,
    );

    const runtimeScriptPath = "test/runtime-script.js";
    const runtimeScriptSource =
      "/*---\nnegative:\n  phase: runtime\n  type: TypeError\n---*/\n" +
      'throw new TypeError("runtime oracle");\n';
    await writeFile(
      path.join(syntheticSuite, runtimeScriptPath),
      runtimeScriptSource,
    );
    const runtimeScriptFields = [...runtimeFields];
    runtimeScriptFields[0] = runtimeScriptPath;
    runtimeScriptFields[2] = createHash("sha256")
      .update(runtimeScriptSource)
      .digest("hex");
    const runtimeScriptContracts = path.join(temporary, "runtime-script.tsv");
    await writeRuntimeContract(runtimeScriptContracts, runtimeScriptFields);
    await expectFailure(
      runtimeArguments(runtimeScriptContracts),
      /runtime contract is not a Module test/u,
    );

    await writeFile(
      path.join(syntheticSuite, "test/runtime.js"),
      "/*---\nnegative:\n  type: SyntaxError\n  phase: parse\n---*/\n" +
        'throw new SyntaxError("runtime-deception");\n',
    );
    const runtimeCandidates = path.join(temporary, "runtime-candidates.tsv");
    await writeFile(
      runtimeCandidates,
      "path\tvariant\trule\n" +
        "test/runtime.js\tsloppy\tassignment-target.non-simple\n",
    );
    const deceptiveOutput = path.join(temporary, "runtime.tsv");
    await expectFailure(
      generationArguments({
        candidates: runtimeCandidates,
        output: deceptiveOutput,
        suite: syntheticSuite,
        qjs: engines.qjs,
        oxide: engines.oxide,
      }),
      /parse probe emitted no native error/u,
    );
    await assert.rejects(access(deceptiveOutput), { code: "ENOENT" });

    await expectFailure(
      generationArguments({
        candidates: smoke,
        output: path.join(temporary, "a-a.tsv"),
        suite: engines.suite,
        qjs: engines.qjs,
        oxide: engines.qjs,
      }),
      /QuickJS and Oxide must be distinct executables/u,
    );

    const hardlinkOutput = path.join(temporary, "candidate-hardlink.tsv");
    await link(smoke, hardlinkOutput);
    await expectFailure(
      generationArguments({
        candidates: smoke,
        output: hardlinkOutput,
        ...engines,
      }),
      /generated output must not alias an input/u,
    );
    const symlinkOutput = path.join(temporary, "candidate-symlink.tsv");
    await symlink(smoke, symlinkOutput);
    await expectFailure(
      generationArguments({
        candidates: smoke,
        output: symlinkOutput,
        ...engines,
      }),
      /generated output must be a regular non-symlink file/u,
    );

    const copiedQuickJs = path.join(temporary, "copied-qjs");
    await copyFile(engines.qjs, copiedQuickJs);
    await chmod(copiedQuickJs, 0o700);
    await expectFailure(
      generationArguments({
        candidates: smoke,
        output: path.join(temporary, "copied-a-a.tsv"),
        suite: engines.suite,
        qjs: engines.qjs,
        oxide: copiedQuickJs,
      }),
      /QuickJS and Oxide must be distinct executables/u,
    );

    await writeFile(
      path.join(syntheticSuite, "test/module.js"),
      "/*---\nflags:\n  - module\nnegative:\n  type: SyntaxError\n  phase: parse\n---*/\n" +
        "import.meta = 1;\n",
    );
    const moduleCandidates = path.join(temporary, "module-candidates.tsv");
    await writeFile(
      moduleCandidates,
      "path\tvariant\trule\n" +
        "test/module.js\tsloppy\tassignment-target.import-meta-script-goal\n",
    );
    await expectFailure(
      generationArguments({
        candidates: moduleCandidates,
        output: path.join(temporary, "module.tsv"),
        suite: syntheticSuite,
        qjs: engines.qjs,
        oxide: engines.oxide,
      }),
      /module; Oxide CLI generation is script-only/u,
    );

    await writeFile(
      path.join(syntheticSuite, "test/raw.js"),
      "/*---\nflags: [raw]\nnegative:\n  type: SyntaxError\n  phase: parse\n---*/\n" +
        "0 = 1;\n",
    );
    const rawCandidates = path.join(temporary, "raw-candidates.tsv");
    await writeFile(
      rawCandidates,
      "path\tvariant\trule\n" +
        "test/raw.js\tstrict\tassignment-target.non-simple\n",
    );
    await expectFailure(
      generationArguments({
        candidates: rawCandidates,
        output: path.join(temporary, "raw.tsv"),
        suite: syntheticSuite,
        qjs: engines.qjs,
        oxide: engines.oxide,
      }),
      /strict is not selected by Test262 metadata/u,
    );

    await symlink("runtime.js", path.join(syntheticSuite, "test/source-link.js"));
    const symlinkCandidates = path.join(temporary, "symlink-candidates.tsv");
    await writeFile(
      symlinkCandidates,
      "path\tvariant\trule\n" +
        "test/source-link.js\tsloppy\tassignment-target.non-simple\n",
    );
    await expectFailure(
      generationArguments({
        candidates: symlinkCandidates,
        output: path.join(temporary, "source-link.tsv"),
        suite: syntheticSuite,
        qjs: engines.qjs,
        oxide: engines.oxide,
      }),
      /not a regular non-symlink suite source/u,
    );
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
  console.log("Negative diagnostic generator regression checks passed.");
}

main().catch((error) => {
  console.error(`test-negative-diagnostic-generator: ${error.stack || error}`);
  process.exitCode = 1;
});
