#!/usr/bin/env node

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const utf8Decoder = new TextDecoder("utf-8", { fatal: true });

function fail(lineNumber, message) {
  throw new Error(`invalid QJODI1 trace at line ${lineNumber}: ${message}`);
}

function decodeBytes(token, lineNumber, fieldName) {
  if (token === "-") {
    return null;
  }
  const match = /^(0|[1-9][0-9]*):([0-9a-f]*)$/.exec(token);
  if (!match) {
    fail(lineNumber, `${fieldName} is not canonical length-prefixed hex`);
  }
  const byteLength = Number(match[1]);
  if (!Number.isSafeInteger(byteLength) || match[2].length !== byteLength * 2) {
    fail(lineNumber, `${fieldName} byte length does not match its hex payload`);
  }
  const bytes = Buffer.from(match[2], "hex");
  let utf8 = null;
  try {
    utf8 = utf8Decoder.decode(bytes);
  } catch {
    // The hex payload remains lossless for non-UTF-8 host paths.
  }
  return { byteLength, hex: match[2], utf8 };
}

export function parseTrace(input) {
  const bytes = Buffer.isBuffer(input) ? input : Buffer.from(input, "utf8");
  for (const [index, byte] of bytes.entries()) {
    if (byte !== 0x09 && byte !== 0x0a && (byte < 0x20 || byte > 0x7e)) {
      const lineNumber = bytes.subarray(0, index).filter((value) => value === 0x0a)
        .length + 1;
      fail(lineNumber, "record contains a non-ASCII or control byte");
    }
  }
  const text = bytes.toString("ascii");
  if (text.length !== 0 && !text.endsWith("\n")) {
    fail(text.split("\n").length, "final record is not newline terminated");
  }
  const records = [];
  const lines = text.length === 0 ? [] : text.slice(0, -1).split("\n");
  for (const [index, line] of lines.entries()) {
    const lineNumber = index + 1;
    const columns = line.split("\t");
    if (columns[0] !== "QJODI1") {
      fail(lineNumber, "unsupported protocol marker");
    }
    if (columns[1] === "N" && columns.length === 6) {
      records.push({
        kind: "normalize",
        root: decodeBytes(columns[2], lineNumber, "root"),
        base: decodeBytes(columns[3], lineNumber, "base"),
        request: decodeBytes(columns[4], lineNumber, "request"),
        normalized: decodeBytes(columns[5], lineNumber, "normalized"),
      });
    } else if (columns[1] === "L" && columns.length === 7) {
      if (!/^(0|[1-9][0-9]*)$/.test(columns[6])) {
        fail(lineNumber, "loader errno is not a non-negative decimal integer");
      }
      const loadErrno = Number(columns[6]);
      if (!Number.isSafeInteger(loadErrno)) {
        fail(lineNumber, "loader errno exceeds the safe integer range");
      }
      records.push({
        kind: "loader",
        root: decodeBytes(columns[2], lineNumber, "root"),
        request: decodeBytes(columns[3], lineNumber, "request"),
        effectivePath: decodeBytes(columns[4], lineNumber, "effective path"),
        outcome: decodeBytes(columns[5], lineNumber, "outcome"),
        errno: loadErrno,
      });
    } else if (columns[1] === "T" && columns.length === 5) {
      if (columns[4] !== "0" && columns[4] !== "1") {
        fail(lineNumber, "has_tla must be 0 or 1");
      }
      records.push({
        kind: "tla",
        root: decodeBytes(columns[2], lineNumber, "root"),
        module: decodeBytes(columns[3], lineNumber, "module"),
        hasTla: columns[4] === "1",
      });
    } else {
      fail(lineNumber, "unknown record type or wrong column count");
    }
  }
  return records;
}

function main(argv) {
  if (argv.length !== 1) {
    console.error("usage: parse-quickjs-dynamic-import-trace.mjs TRACE_FILE");
    process.exitCode = 2;
    return;
  }
  const records = parseTrace(readFileSync(argv[0]));
  for (const record of records) {
    process.stdout.write(`${JSON.stringify(record)}\n`);
  }
}

if (
  process.argv[1] &&
  fileURLToPath(import.meta.url) === path.resolve(process.argv[1])
) {
  main(process.argv.slice(2));
}
