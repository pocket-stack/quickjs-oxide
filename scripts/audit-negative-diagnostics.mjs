#!/usr/bin/env node

import { createHash } from "node:crypto";
import { constants as fsConstants } from "node:fs";
import { access, readFile, stat } from "node:fs/promises";
import { availableParallelism } from "node:os";
import path from "node:path";
import { spawn } from "node:child_process";

const CONTRACT_HEADER =
  "path\tvariant\tsource_sha256\tphase\ttype\trule\tmessage\tline\tcolumn\tlocation_policy";
const RULE_HEADER = "rule\tquickjs_anchor\tdescription";
const DEFAULT_CONTRACTS = "dev-support/test262/negative-diagnostics.tsv";
const DEFAULT_RULES = "dev-support/test262/negative-diagnostic-rules.tsv";
const DEFAULT_TIMEOUT_MS = 10_000;

function usage() {
  console.error(
    "usage: audit-negative-diagnostics.mjs [--check] [--contracts FILE] " +
      "[--rules FILE] [--suite DIR --qjs FILE] [--workers N]",
  );
}

function parseArguments(arguments_) {
  const options = {
    check: false,
    contracts: DEFAULT_CONTRACTS,
    qjs: undefined,
    rules: DEFAULT_RULES,
    suite: undefined,
    workers: Math.max(1, Math.min(availableParallelism(), 12)),
  };
  for (let index = 0; index < arguments_.length; index += 1) {
    const argument = arguments_[index];
    const takeValue = () => {
      index += 1;
      if (index >= arguments_.length) throw new Error(`${argument} requires a value`);
      return arguments_[index];
    };
    if (argument === "--check") options.check = true;
    else if (argument === "--contracts") options.contracts = takeValue();
    else if (argument === "--rules") options.rules = takeValue();
    else if (argument === "--suite") options.suite = takeValue();
    else if (argument === "--qjs") options.qjs = takeValue();
    else if (argument === "--workers") {
      const value = takeValue();
      if (!/^[1-9][0-9]*$/.test(value)) throw new Error("--workers must be positive");
      options.workers = Number(value);
    } else if (argument === "--help" || argument === "-h") {
      usage();
      process.exit(0);
    } else {
      throw new Error(`unknown option: ${argument}`);
    }
  }
  if (!options.check && (!options.suite || !options.qjs)) {
    throw new Error("replay requires both --suite and --qjs");
  }
  return options;
}

async function readCanonicalLines(file, expectedHeader, label) {
  const source = await readFile(file, "utf8");
  if (source.includes("\r")) throw new Error(`${label} must use LF line endings`);
  if (!source.endsWith("\n")) throw new Error(`${label} must end with a newline`);
  const lines = source.slice(0, -1).split("\n");
  if (lines.shift() !== expectedHeader) throw new Error(`${label} header drifted`);
  if (lines.length === 0 || lines.some((line) => line.length === 0)) {
    throw new Error(`${label} must contain non-empty rows`);
  }
  return lines;
}

async function loadRules(file) {
  const lines = await readCanonicalLines(file, RULE_HEADER, "diagnostic rule registry");
  const rules = new Map();
  let previous = "";
  for (const [index, line] of lines.entries()) {
    const fields = line.split("\t");
    if (fields.length !== 3 || fields.some((field) => field.trim() !== field)) {
      throw new Error(`diagnostic rule line ${index + 2} is not canonical`);
    }
    const [rule, anchor, description] = fields;
    if (!/^[a-z0-9]+(?:[.-][a-z0-9]+)*$/.test(rule)) {
      throw new Error(`diagnostic rule line ${index + 2} has an invalid rule`);
    }
    if (!/^js_[a-z0-9_]+$/.test(anchor) && anchor !== "get_lvalue") {
      throw new Error(`diagnostic rule ${rule} has an invalid QuickJS anchor`);
    }
    if (!description || previous >= rule || rules.has(rule)) {
      throw new Error(`diagnostic rule registry is duplicate or unsorted at ${rule}`);
    }
    previous = rule;
    rules.set(rule, { anchor, description });
  }
  return rules;
}

async function loadContracts(file, rules) {
  const lines = await readCanonicalLines(file, CONTRACT_HEADER, "negative diagnostics");
  const contracts = [];
  const usedRules = new Set();
  let previous = "";
  for (const [index, line] of lines.entries()) {
    const fields = line.split("\t");
    if (fields.length !== 10 || fields.some((field) => field.trim() !== field)) {
      throw new Error(`negative diagnostic line ${index + 2} is not canonical`);
    }
    const [
      relative,
      variant,
      sourceSha256,
      phase,
      errorType,
      rule,
      message,
      lineText,
      columnText,
      locationPolicy,
    ] = fields;
    const key = `${relative}\t${variant}`;
    if (!relative.startsWith("test/") || relative.includes("\\") || previous >= key) {
      throw new Error(`negative diagnostics are duplicate or unsorted at ${key}`);
    }
    if (!/^(sloppy|strict)$/.test(variant) || !/^[0-9a-f]{64}$/.test(sourceSha256)) {
      throw new Error(`negative diagnostic ${key} has invalid identity data`);
    }
    if (phase !== "parse" || errorType !== "SyntaxError") {
      throw new Error(`QuickJS audit does not yet support ${phase}/${errorType}: ${key}`);
    }
    if (!rules.has(rule)) throw new Error(`negative diagnostic ${key} uses unknown rule ${rule}`);
    if (!message) throw new Error(`negative diagnostic ${key} has an empty message`);
    const exact = locationPolicy === "exact";
    const absent = locationPolicy === "absent";
    if ((!exact && !absent) || (exact && (!/^[1-9][0-9]*$/.test(lineText) || !/^[1-9][0-9]*$/.test(columnText))) || (absent && (lineText || columnText))) {
      throw new Error(`negative diagnostic ${key} has an invalid location policy`);
    }
    previous = key;
    usedRules.add(rule);
    contracts.push({
      column: exact ? Number(columnText) : undefined,
      errorType,
      line: exact ? Number(lineText) : undefined,
      locationPolicy,
      message,
      phase,
      relative,
      rule,
      sourceSha256,
      variant,
    });
  }
  const unused = [...rules.keys()].filter((rule) => !usedRules.has(rule));
  if (unused.length) throw new Error(`diagnostic rule registry contains unused rules: ${unused.join(", ")}`);
  return contracts;
}

function isModuleSource(source) {
  const start = source.indexOf("/*---");
  const end = start < 0 ? -1 : source.indexOf("---*/", start + 5);
  if (end < 0) return false;
  const frontmatter = source.slice(start + 5, end);
  return /(?:^|\n)flags:\s*\[[^\]]*\bmodule\b[^\]]*\]/m.test(frontmatter);
}

function runQuickJs(qjs, arguments_, timeoutMs) {
  return new Promise((resolve, reject) => {
    const child = spawn(qjs, arguments_, { stdio: ["ignore", "pipe", "pipe"] });
    const stdout = [];
    const stderr = [];
    let stdoutBytes = 0;
    let stderrBytes = 0;
    let timedOut = false;
    const timer = setTimeout(() => {
      timedOut = true;
      child.kill("SIGKILL");
    }, timeoutMs);
    child.stdout.on("data", (chunk) => {
      stdoutBytes += chunk.length;
      if (stdoutBytes <= 1_048_576) stdout.push(chunk);
    });
    child.stderr.on("data", (chunk) => {
      stderrBytes += chunk.length;
      if (stderrBytes <= 1_048_576) stderr.push(chunk);
    });
    child.on("error", (error) => {
      clearTimeout(timer);
      reject(error);
    });
    child.on("close", (status, signal) => {
      clearTimeout(timer);
      if (timedOut) return reject(new Error(`QuickJS timed out after ${timeoutMs}ms`));
      if (stdoutBytes > 1_048_576 || stderrBytes > 1_048_576) {
        return reject(new Error("QuickJS diagnostic output exceeded 1 MiB"));
      }
      resolve({
        signal,
        status,
        stderr: Buffer.concat(stderr).toString("utf8"),
        stdout: Buffer.concat(stdout).toString("utf8"),
      });
    });
  });
}

function parseQuickJsDiagnostic(result) {
  if (result.status === 0 || result.signal) {
    throw new Error(`QuickJS did not return a normal failing status: ${JSON.stringify(result)}`);
  }
  const lines = result.stderr.replaceAll("\r\n", "\n").split("\n");
  const errorLine = lines.find((line) => /^[A-Za-z_$][A-Za-z0-9_$]*Error: /.test(line));
  if (!errorLine) throw new Error(`QuickJS emitted no native error: ${result.stderr}`);
  const separator = errorLine.indexOf(": ");
  const errorType = errorLine.slice(0, separator);
  const message = errorLine.slice(separator + 2);
  const locationLine = lines.find((line) => /^\s*at <cmdline>:[1-9][0-9]*:[1-9][0-9]*\s*$/.test(line));
  const location = locationLine?.match(/<cmdline>:([1-9][0-9]*):([1-9][0-9]*)/);
  return {
    column: location ? Number(location[2]) : undefined,
    errorType,
    line: location ? Number(location[1]) : undefined,
    message,
  };
}

async function replayContract(options, contract) {
  const sourcePath = path.join(options.suite, contract.relative);
  const sourceBytes = await readFile(sourcePath);
  const actualSha256 = createHash("sha256").update(sourceBytes).digest("hex");
  if (actualSha256 !== contract.sourceSha256) {
    throw new Error(`${contract.relative} source hash drifted`);
  }
  const source = sourceBytes.toString("utf8");
  const module = isModuleSource(source);
  const authored = contract.variant === "strict" && !module ? `"use strict";\n${source}` : source;
  const result = await runQuickJs(
    options.qjs,
    [module ? "--module" : "--script", "-e", authored],
    DEFAULT_TIMEOUT_MS,
  );
  const actual = parseQuickJsDiagnostic(result);
  const expectedLocation = contract.locationPolicy === "exact";
  if (
    actual.errorType !== contract.errorType ||
    actual.message !== contract.message ||
    (expectedLocation && (actual.line !== contract.line || actual.column !== contract.column)) ||
    (!expectedLocation && (actual.line !== undefined || actual.column !== undefined))
  ) {
    throw new Error(
      `${contract.relative} ${contract.variant} ${contract.rule} mismatch:\n` +
        `expected ${JSON.stringify({ type: contract.errorType, message: contract.message, line: contract.line, column: contract.column })}\n` +
        `actual   ${JSON.stringify(actual)}\n${result.stderr}`,
    );
  }
}

async function replayAll(options, contracts) {
  const suite = await stat(options.suite);
  const qjs = await stat(options.qjs);
  if (!suite.isDirectory() || !qjs.isFile()) throw new Error("suite or qjs path has the wrong type");
  await access(options.qjs, fsConstants.X_OK);
  let next = 0;
  const failures = [];
  const worker = async () => {
    while (failures.length === 0) {
      const index = next;
      next += 1;
      if (index >= contracts.length) return;
      try {
        await replayContract(options, contracts[index]);
      } catch (error) {
        failures.push(error);
      }
    }
  };
  await Promise.all(Array.from({ length: Math.min(options.workers, contracts.length) }, worker));
  if (failures.length) throw failures[0];
}

async function main() {
  const options = parseArguments(process.argv.slice(2));
  const rules = await loadRules(options.rules);
  const contracts = await loadContracts(options.contracts, rules);
  if (options.check) {
    console.log(
      `Negative diagnostic registry passed: ${contracts.length} exact contracts / ${rules.size} rules.`,
    );
    return;
  }
  await replayAll(options, contracts);
  console.log(
    `QuickJS negative diagnostic audit passed: ${contracts.length} exact contracts / ${rules.size} rules.`,
  );
}

main().catch((error) => {
  console.error(`audit-negative-diagnostics: ${error.stack || error}`);
  process.exitCode = 1;
});
