import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

export const admissionColumns = [
  "kind",
  "group",
  "path",
  "source_sha256",
  "includes",
  "flags",
  "features",
  "negative_phase",
  "negative_type",
  "closure_file_count",
  "priority",
  "request_index",
  "specifier",
  "normalized_path",
  "policy",
  "cohort",
];

export const admissionHeader = admissionColumns.join("\t");
const emptyField = "-";

const bytewise = (left, right) => Buffer.compare(Buffer.from(left), Buffer.from(right));

function semanticFieldValue(value) {
  if (value === undefined || value === null) return "";
  if (Array.isArray(value)) return value.join(",");
  return String(value);
}

export function admissionRecord(fields) {
  for (const name of Object.keys(fields)) {
    assert(admissionColumns.includes(name), `unknown admission field: ${name}`);
  }
  const record = Object.fromEntries(
    admissionColumns.map((name) => [name, semanticFieldValue(fields[name])]),
  );
  for (const [name, value] of Object.entries(record)) {
    assert(!/[\t\r\n]/.test(value), `${name} contains a TSV control character`);
    assert.notEqual(value, emptyField, `${name} uses the reserved empty-field sentinel`);
  }
  assert(record.kind, "admission kind must not be empty");
  assert(record.group, "admission group must not be empty");
  assert(record.path, "admission path must not be empty");
  return record;
}

export function admissionLine(record) {
  return admissionColumns
    .map((name) => semanticFieldValue(record[name]) || emptyField)
    .join("\t");
}

export function renderAdmissionRows(records) {
  const rows = records.map(admissionLine).sort(bytewise);
  assert.equal(new Set(rows).size, rows.length, "generated duplicate admission rows");
  return rows.length === 0 ? "" : `${rows.join("\n")}\n`;
}

function readAdmissionRows(path) {
  const contents = readFileSync(path, "utf8");
  assert(!contents.includes("\r"), `${path}: admissions must use LF line endings`);
  assert(contents.endsWith("\n"), `${path}: admissions must end with a newline`);
  const [header, ...rows] = contents.slice(0, -1).split("\n");
  assert.equal(header, admissionHeader, `${path}: admission header drifted`);

  let previous = null;
  return rows.map((line, index) => {
    const values = line.split("\t");
    assert.equal(
      values.length,
      admissionColumns.length,
      `${path}:${index + 2}: expected ${admissionColumns.length} columns`,
    );
    assert(
      values.every((value) => value.length > 0),
      `${path}:${index + 2}: empty fields must use ${emptyField}`,
    );
    if (previous !== null) {
      assert(
        bytewise(previous, line) < 0,
        `${path}:${index + 2}: admission rows are not strictly bytewise sorted`,
      );
    }
    previous = line;
    return {
      line,
      record: Object.fromEntries(
        admissionColumns.map((name, field) => [
          name,
          values[field] === emptyField ? "" : values[field],
        ]),
      ),
    };
  });
}

export function assertAdmissionGroup(path, group, generatedRecords) {
  assert(
    generatedRecords.every((record) => record.group === group),
    `${group}: generated record escaped its group`,
  );
  const expected = renderAdmissionRows(generatedRecords);
  const actualRows = readAdmissionRows(path)
    .filter(({ record }) => record.group === group)
    .map(({ line }) => line);
  const actual = actualRows.length === 0 ? "" : `${actualRows.join("\n")}\n`;
  assert.equal(actual, expected, `${group}: ${path} admissions drifted`);
  return actualRows.length;
}
