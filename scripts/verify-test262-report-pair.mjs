#!/usr/bin/env node

import { lstatSync, readFileSync } from "node:fs";

const fields = [
  "path",
  "variant",
  "flags",
  "features",
  "expected_phase",
  "expected_type",
  "outcome",
  "actual_phase",
  "actual_type",
  "detail",
  "expected_rule",
  "expected_message",
  "expected_line",
  "expected_column",
  "location_policy",
  "actual_line",
  "actual_column",
];

function die(message) {
  process.stderr.write(`verify-test262-report-pair: ${message}\n`);
  process.exit(2);
}

if (process.argv.length !== 4) {
  die("usage: verify-test262-report-pair.mjs REPORT.tsv REPORT.jsonl");
}
const [tsvPath, jsonlPath] = process.argv.slice(2);

function readRegular(path) {
  const stat = lstatSync(path, { throwIfNoEntry: false });
  if (stat === undefined || !stat.isFile() || stat.isSymbolicLink()) {
    die(`input is not a regular non-symlink file: ${path}`);
  }
  const text = readFileSync(path, "utf8");
  if (!text.endsWith("\n") || text.includes("\r")) {
    die(`input must use canonical LF-terminated lines: ${path}`);
  }
  return text.slice(0, -1).split("\n");
}

function canonicalKeys(value, expected, label) {
  if (value === null || Array.isArray(value) || typeof value !== "object") {
    die(`${label} is not an object`);
  }
  const actual = Object.keys(value).sort();
  const wanted = [...expected].sort();
  if (actual.length !== wanted.length || actual.some((key, index) => key !== wanted[index])) {
    die(`${label} fields drifted`);
  }
}

function unescapeField(value) {
  let output = "";
  for (let index = 0; index < value.length; index += 1) {
    const character = value[index];
    if (character !== "\\") {
      if (character.codePointAt(0) < 0x20 || character.codePointAt(0) === 0x7f) {
        die("TSV field contains an unescaped control character");
      }
      output += character;
      continue;
    }
    index += 1;
    const escape = value[index];
    if (escape === "\\") output += "\\";
    else if (escape === "t") output += "\t";
    else if (escape === "n") output += "\n";
    else if (escape === "r") output += "\r";
    else if (escape === "u") {
      const digits = value.slice(index + 1, index + 5);
      if (!/^[0-9a-f]{4}$/.test(digits)) die("TSV field has an invalid Unicode escape");
      output += String.fromCodePoint(Number.parseInt(digits, 16));
      index += 4;
    } else {
      die("TSV field has an invalid escape");
    }
  }
  return output;
}

function parseSummary(text, label) {
  if (text.length === 0) die(`${label} summary is empty`);
  const output = Object.create(null);
  let previous = "";
  for (const entry of text.split(" ")) {
    const match = /^([a-z][a-z0-9-]*)=([1-9][0-9]*)$/.exec(entry);
    if (match === null || match[1] <= previous || output[match[1]] !== undefined) {
      die(`${label} summary is not canonical`);
    }
    output[match[1]] = Number(match[2]);
    previous = match[1];
  }
  return output;
}

function equalSummary(left, right) {
  const keys = Object.keys(left);
  const otherKeys = Object.keys(right);
  return (
    keys.length === otherKeys.length &&
    keys.every((key, index) => key === otherKeys[index] && left[key] === right[key])
  );
}

const tsv = readRegular(tsvPath);
const first = /^# quickjs-oxide Test262 outcome vector v5 engine_semantics_sha256=([0-9a-f]{64})$/.exec(
  tsv[0] ?? "",
);
if (first === null) die("TSV vector header drifted");

const metadataNames = [
  "quickjs",
  "test262",
  "test262_patch_sha256",
  "test262_config_sha256",
  "test262_metadata_sha256",
  "oxide_profile_sha256",
  "negative_diagnostics_sha256",
  "negative_diagnostic_exemptions_sha256",
  "profile",
  "mode",
];
const metadata = Object.create(null);
let cursor = 1;
for (const name of metadataNames) {
  const prefix = `# ${name}=`;
  const line = tsv[cursor];
  if (line === undefined || !line.startsWith(prefix) || line.length === prefix.length) {
    die(`TSV metadata field drifted: ${name}`);
  }
  metadata[name] = line.slice(prefix.length);
  cursor += 1;
}
if (tsv[cursor] !== fields.join("\t")) die("TSV result header drifted");
cursor += 1;
if (tsv.length <= cursor || !tsv.at(-1).startsWith("# summary ")) {
  die("TSV summary is missing");
}

const rows = tsv.slice(cursor, -1).map((line, index) => {
  const values = line.split("\t");
  if (values.length !== fields.length) die(`TSV row ${index + 1} has the wrong field count`);
  return Object.fromEntries(fields.map((field, fieldIndex) => [field, unescapeField(values[fieldIndex])]));
});
if (rows.length === 0) die("TSV result vector is empty");

const computedSummary = Object.create(null);
let previousKey = "";
for (const row of rows) {
  const key = `${row.path}\0${row.variant}`;
  if (key <= previousKey) die("TSV result keys are duplicated or not bytewise sorted");
  previousKey = key;
  computedSummary[row.outcome] = (computedSummary[row.outcome] ?? 0) + 1;
}
const sortedComputedSummary = Object.fromEntries(
  Object.entries(computedSummary).sort(([left], [right]) => Buffer.compare(Buffer.from(left), Buffer.from(right))),
);
const tsvSummary = parseSummary(tsv.at(-1).slice("# summary ".length), "TSV");
if (!equalSummary(tsvSummary, sortedComputedSummary)) die("TSV summary does not match its rows");

const jsonLines = readRegular(jsonlPath);
let records;
try {
  records = jsonLines.map((line) => JSON.parse(line));
} catch (error) {
  die(`JSONL parse failed: ${error.message}`);
}
if (records.length !== rows.length + 2) die("JSONL record count does not match TSV");

const jsonMetadata = records[0];
const jsonMetadataFields = [
  "kind",
  "schema",
  ...metadataNames,
  "engine_semantics_sha256",
];
canonicalKeys(jsonMetadata, jsonMetadataFields, "JSONL metadata");
if (jsonMetadata.kind !== "metadata" || jsonMetadata.schema !== 5) {
  die("JSONL metadata schema drifted");
}
for (const name of metadataNames) {
  if (jsonMetadata[name] !== metadata[name]) die(`TSV/JSONL metadata differs: ${name}`);
}
if (jsonMetadata.engine_semantics_sha256 !== first[1]) {
  die("TSV/JSONL engine semantics fingerprint differs");
}

for (let index = 0; index < rows.length; index += 1) {
  const record = records[index + 1];
  canonicalKeys(record, ["kind", ...fields], `JSONL result ${index + 1}`);
  if (record.kind !== "result") die(`JSONL result ${index + 1} has the wrong kind`);
  for (const field of fields) {
    if (typeof record[field] !== "string" || record[field] !== rows[index][field]) {
      die(`TSV/JSONL result differs at row ${index + 1}, field ${field}`);
    }
  }
}

const jsonSummaryRecord = records.at(-1);
canonicalKeys(jsonSummaryRecord, ["kind", "outcomes"], "JSONL summary");
if (jsonSummaryRecord.kind !== "summary" || typeof jsonSummaryRecord.outcomes !== "object") {
  die("JSONL summary drifted");
}
canonicalKeys(jsonSummaryRecord.outcomes, Object.keys(tsvSummary), "JSONL outcomes");
for (const [outcome, count] of Object.entries(jsonSummaryRecord.outcomes)) {
  if (!Number.isSafeInteger(count) || count <= 0 || tsvSummary[outcome] !== count) {
    die(`TSV/JSONL summary differs: ${outcome}`);
  }
}

process.stdout.write(`Test262 TSV/JSONL pair verified: ${rows.length} rows\n`);
