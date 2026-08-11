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
export const DOC_METRICS_START = "<!-- current-test262-metrics:start -->";
export const DOC_METRICS_END = "<!-- current-test262-metrics:end -->";

const DEFAULT_REPO_ROOT = path.resolve(import.meta.dirname, "..");
const DEFAULT_SPEC_PATH = path.resolve(
  DEFAULT_REPO_ROOT,
  "dev-support/test262/current.conf",
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

function formatPercent(numerator, denominator) {
  return ((numerator / denominator) * 100).toFixed(3);
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
  const fullPassPercent = formatPercent(fullPasses, fullVariants);
  const eligiblePercent = formatPercent(fullEligible, fullVariants);
  const runnableQuality = formatPercent(fullPasses, fullRunnable);
  return Object.freeze({
    detailText:
      `${milestone} authenticated vector · ` +
      `${formatInteger(focusedPasses)}/${formatInteger(focusedEligible)} focused · ` +
      `runnable quality ${formatInteger(fullPasses)}/${formatInteger(fullRunnable)} ` +
      `(${runnableQuality}%, secondary) · pre-parity`,
    eligibleFailures: fullRunnable - fullPasses,
    eligiblePercent,
    eligibleTimeouts: fullSummary.get("timeout") ?? 0,
    focusedEligible,
    focusedPasses,
    fullEligible,
    fullPassPercent,
    fullPasses,
    fullRunnable,
    fullSummaryText: required(values, "full_summary"),
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

function markedDocument(source, body, filePath) {
  const startCount = source.split(DOC_METRICS_START).length - 1;
  const endCount = source.split(DOC_METRICS_END).length - 1;
  if (startCount !== 1 || endCount !== 1) {
    throw new TypeError(
      `${filePath} must contain exactly one Test262 metrics marker pair`,
    );
  }
  const start = source.indexOf(DOC_METRICS_START);
  const end = source.indexOf(DOC_METRICS_END, start);
  if (end < start) {
    throw new TypeError(`${filePath} has reversed Test262 metrics markers`);
  }
  const before = source.slice(0, start);
  const after = source.slice(end + DOC_METRICS_END.length);
  return `${before}${DOC_METRICS_START}\n${body}\n${DOC_METRICS_END}${after}`;
}

function currentTest262DocumentBlocks(metrics) {
  const fullPasses = formatInteger(metrics.fullPasses);
  const fullEligible = formatInteger(metrics.fullEligible);
  const fullVariants = formatInteger(metrics.fullVariants);
  const fullRunnable = formatInteger(metrics.fullRunnable);
  const timeoutText = metrics.eligibleTimeouts === 0
    ? "no timeouts"
    : `${formatInteger(metrics.eligibleTimeouts)} timeouts`;
  return new Map([
    [
      "README.md",
      `The authoritative ${metrics.milestone} Test262 baseline records **${fullPasses} full-corpus passes\n` +
        `out of ${fullVariants} variants (${metrics.fullPassPercent}%)**, with **${fullEligible} eligible variants\n` +
        `(${metrics.eligiblePercent}%)**. The ${fullPasses} / ${fullRunnable} runnable pass rate (${metrics.runnableQuality}%) is a secondary\n` +
        "quality measure, not the headline compatibility metric.",
    ],
    [
      "docs/status.md",
      `The authoritative ${metrics.milestone} Test262 vector has:\n\n` +
        `- ${fullPasses} full-corpus passes out of ${fullVariants} variants (${metrics.fullPassPercent}%)\n` +
        `- ${fullEligible} eligible variants out of ${fullVariants} (${metrics.eligiblePercent}%)\n` +
        `- ${fullPasses} passes out of ${fullRunnable} runnable variants (${metrics.runnableQuality}%, secondary quality\n` +
        "  metric)\n" +
        `- ${formatInteger(metrics.eligibleFailures)} classified failures and ${timeoutText} among eligible variants`,
    ],
    [
      "docs/test262.md",
      "Metrics are reported in this order:\n\n" +
        `1. **Full pass:** ${fullPasses} / ${fullVariants} (${metrics.fullPassPercent}%). Every frozen Test262 variant is in\n` +
        "   the denominator.\n" +
        `2. **Eligible coverage:** ${fullEligible} / ${fullVariants} (${metrics.eligiblePercent}%). This measures how much of\n` +
        "   the full vector the current profile admits to execution.\n" +
        `3. **Runnable pass quality:** ${fullPasses} / ${fullRunnable} (${metrics.runnableQuality}%). This is useful for\n` +
        "   diagnosing admitted behavior, but it must not replace either coverage\n" +
        "   metric above.\n\n" +
        "The frozen outcome summary is:\n\n" +
        "```text\n" +
        `${metrics.fullSummaryText}\n` +
        "```",
    ],
  ]);
}

export function syncCurrentTest262Docs({
  check = false,
  repoRoot = DEFAULT_REPO_ROOT,
  specPath = DEFAULT_SPEC_PATH,
} = {}) {
  const metrics = readCurrentTest262Metrics(specPath);
  for (const [relativePath, body] of currentTest262DocumentBlocks(metrics)) {
    const filePath = path.resolve(repoRoot, relativePath);
    const source = readFileSync(filePath, "utf8");
    const rendered = markedDocument(source, body, relativePath);
    if (check) {
      if (rendered !== source) {
        throw new TypeError(
          `${relativePath} Test262 metrics drifted; run ` +
            "node scripts/current-test262-metrics.mjs --write-docs",
        );
      }
    } else {
      writeFileSync(filePath, rendered);
    }
  }
  return metrics;
}

const invokedAsScript = process.argv[1]
  ? pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url
  : false;
if (invokedAsScript) {
  if (process.argv.length === 3 && process.argv[2] === "--check-docs") {
    const metrics = syncCurrentTest262Docs({ check: true });
    console.log(`Test262 documentation metrics passed: ${metrics.primaryText}`);
  } else if (process.argv.length === 3 && process.argv[2] === "--write-docs") {
    const metrics = syncCurrentTest262Docs();
    console.log(`Rendered Test262 documentation metrics: ${metrics.primaryText}`);
  } else if (process.argv.length === 4 && process.argv[2] === "--render") {
    const metrics = renderCurrentTest262Metrics(path.resolve(process.argv[3]));
    console.log(`Rendered Pages metrics: ${metrics.primaryText}`);
  } else {
    console.error(
      "usage: current-test262-metrics.mjs " +
        "--check-docs | --write-docs | --render PATH_TO_INDEX_HTML",
    );
    process.exitCode = 2;
  }
}
