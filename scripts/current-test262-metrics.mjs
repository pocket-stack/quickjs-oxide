#!/usr/bin/env node

import {
  readFileSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

export const PRIMARY_METRICS_TOKEN = "@@QUICKJS_OXIDE_TEST262_PRIMARY@@";
export const DETAIL_METRICS_TOKEN = "@@QUICKJS_OXIDE_TEST262_DETAIL@@";

const DEFAULT_SPEC_PATH = path.resolve(
  import.meta.dirname,
  "../dev-support/test262/current.conf",
);
const NUMERIC_KEYS = [
  "focused_variants",
  "focused_eligible",
  "focused_runnable",
  "focused_passes",
  "full_variants",
  "full_eligible",
  "full_runnable",
  "full_passes",
];

function parseSpec(source) {
  const values = new Map();
  for (const [index, rawLine] of source.split("\n").entries()) {
    const line = rawLine.endsWith("\r") ? rawLine.slice(0, -1) : rawLine;
    if (line === "" || line.startsWith("#")) {
      continue;
    }
    const match = line.match(/^([a-z][a-z0-9_]*)=(.*)$/u);
    if (!match) {
      throw new TypeError(`malformed Test262 spec line ${index + 1}`);
    }
    const [, key, value] = match;
    if (
      value.length === 0 ||
      value.trim() !== value ||
      !/^[\x20-\x7e]+$/u.test(value)
    ) {
      throw new TypeError(`invalid Test262 spec value for ${key}`);
    }
    if (values.has(key)) {
      throw new TypeError(`duplicate Test262 spec key ${key}`);
    }
    values.set(key, value);
  }
  return values;
}

function required(values, key) {
  const value = values.get(key);
  if (value === undefined) {
    throw new TypeError(`missing Test262 spec key ${key}`);
  }
  return value;
}

function canonicalInteger(values, key) {
  const value = required(values, key);
  if (!/^(0|[1-9][0-9]*)$/u.test(value)) {
    throw new TypeError(`non-canonical Test262 integer ${key}`);
  }
  const number = Number(value);
  if (!Number.isSafeInteger(number)) {
    throw new TypeError(`Test262 integer exceeds the safe range: ${key}`);
  }
  return number;
}

function parseSummary(value, label) {
  const outcomes = new Map();
  let previous = "";
  for (const field of value.split(" ")) {
    const match = field.match(/^([a-z][a-z0-9-]*)=(0|[1-9][0-9]*)$/u);
    if (!match || match[1] <= previous || outcomes.has(match[1])) {
      throw new TypeError(`non-canonical ${label} Test262 summary`);
    }
    const count = Number(match[2]);
    if (!Number.isSafeInteger(count)) {
      throw new TypeError(`${label} Test262 summary exceeds the safe range`);
    }
    outcomes.set(match[1], count);
    previous = match[1];
  }
  return outcomes;
}

function formatInteger(value) {
  return String(value).replace(/\B(?=(?:[0-9]{3})+(?![0-9]))/gu, ",");
}

function displayMilestone(value) {
  if (!/^[a-z0-9]+(?:-[a-z0-9]+)*$/u.test(value)) {
    throw new TypeError("invalid Test262 milestone name");
  }
  return value
    .split("-")
    .map((part, index) =>
      index === 0
        ? `${part[0].toUpperCase()}${part.slice(1)}`
        : part.toUpperCase(),
    )
    .join("-");
}

export function parseCurrentTest262Metrics(source) {
  const values = parseSpec(source);
  if (required(values, "schema") !== "test262-gate-v2") {
    throw new TypeError("unsupported Test262 gate schema");
  }
  const numbers = Object.fromEntries(
    NUMERIC_KEYS.map((key) => [key, canonicalInteger(values, key)]),
  );
  const {
    focused_variants: focusedVariants,
    focused_eligible: focusedEligible,
    focused_runnable: focusedRunnable,
    focused_passes: focusedPasses,
    full_variants: fullVariants,
    full_eligible: fullEligible,
    full_runnable: fullRunnable,
    full_passes: fullPasses,
  } = numbers;
  if (
    focusedPasses > focusedRunnable ||
    focusedRunnable !== focusedEligible ||
    focusedEligible > focusedVariants ||
    fullRunnable === 0 ||
    fullPasses > fullRunnable ||
    fullRunnable !== fullEligible ||
    fullEligible > fullVariants
  ) {
    throw new TypeError("inconsistent Test262 metric ordering");
  }

  const focusedSummary = parseSummary(
    required(values, "focused_summary"),
    "focused",
  );
  const fullSummary = parseSummary(required(values, "full_summary"), "full");
  const focusedTotal = [...focusedSummary.values()].reduce(
    (total, count) => total + count,
    0,
  );
  const fullTotal = [...fullSummary.values()].reduce(
    (total, count) => total + count,
    0,
  );
  const fullIneligible = [...fullSummary]
    .filter(([name]) => /^(?:skipped|unsupported)-/u.test(name))
    .reduce((total, [, count]) => total + count, 0);
  if (
    focusedTotal !== focusedVariants ||
    focusedSummary.get("pass") !== focusedPasses ||
    fullTotal !== fullVariants ||
    fullSummary.get("pass") !== fullPasses ||
    fullVariants - fullIneligible !== fullEligible
  ) {
    throw new TypeError("Test262 summaries disagree with the official metrics");
  }

  const milestone = displayMilestone(required(values, "milestone"));
  const runnableQuality = ((fullPasses / fullRunnable) * 100).toFixed(3);
  return Object.freeze({
    detailText:
      `${milestone} authenticated vector · ` +
      `${formatInteger(focusedPasses)}/${formatInteger(focusedEligible)} focused · ` +
      `runnable quality ${formatInteger(fullPasses)}/${formatInteger(fullRunnable)} ` +
      `(${runnableQuality}%, secondary) · pre-parity`,
    focusedEligible,
    focusedPasses,
    fullEligible,
    fullPasses,
    fullRunnable,
    fullVariants,
    milestone,
    primaryText:
      `${formatInteger(fullPasses)} full passes / ` +
      `${formatInteger(fullEligible)} eligible / ` +
      `${formatInteger(fullVariants)} total`,
    runnableQuality,
  });
}

export function readCurrentTest262Metrics(specPath = DEFAULT_SPEC_PATH) {
  return parseCurrentTest262Metrics(readFileSync(specPath, "utf8"));
}

function htmlText(value) {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function replaceExactlyOnce(source, token, value) {
  const count = source.split(token).length - 1;
  if (count !== 1) {
    throw new TypeError(
      `expected exactly one ${token} placeholder, found ${count}`,
    );
  }
  return source.replace(token, htmlText(value));
}

export function renderCurrentTest262Metrics(
  htmlPath,
  specPath = DEFAULT_SPEC_PATH,
) {
  const metrics = readCurrentTest262Metrics(specPath);
  let html = readFileSync(htmlPath, "utf8");
  html = replaceExactlyOnce(html, PRIMARY_METRICS_TOKEN, metrics.primaryText);
  html = replaceExactlyOnce(html, DETAIL_METRICS_TOKEN, metrics.detailText);
  writeFileSync(htmlPath, html);
  return metrics;
}

const invokedAsScript = process.argv[1]
  ? pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url
  : false;
if (invokedAsScript) {
  if (process.argv.length !== 4 || process.argv[2] !== "--render") {
    console.error(
      "usage: current-test262-metrics.mjs --render PATH_TO_INDEX_HTML",
    );
    process.exitCode = 2;
  } else {
    const metrics = renderCurrentTest262Metrics(path.resolve(process.argv[3]));
    console.log(`Rendered Pages metrics: ${metrics.primaryText}`);
  }
}
