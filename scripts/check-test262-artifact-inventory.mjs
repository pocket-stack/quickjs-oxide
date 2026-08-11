#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";

function trackedFiles(pattern) {
  const output = execFileSync("git", ["ls-files", "-z", "--", pattern]);
  return output
    .toString("utf8")
    .split("\0")
    .filter(Boolean);
}

const artifacts = trackedFiles("tests/test262-*").filter((relative) =>
  existsSync(relative),
);
if (artifacts.length === 0) {
  throw new Error("tracked Test262 artifact inventory is unexpectedly empty");
}
const artifactSet = new Set(artifacts);
const referenceSources = trackedFiles(":(top)**").filter(
  (relative) => !artifactSet.has(relative),
);
const references = referenceSources
  .map((relative) => {
    try {
      return readFileSync(relative, "utf8");
    } catch {
      return "";
    }
  })
  .join("\n");

const unreferenced = artifacts.filter((relative) => {
  const basename = path.posix.basename(relative);
  return !references.includes(relative) && !references.includes(basename);
});
if (unreferenced.length !== 0) {
  console.error(unreferenced.join("\n"));
  throw new Error(
    "tracked Test262 artifacts must be referenced by the current spec, runner, or generator",
  );
}

console.log(
  `Test262 artifact inventory passed: ${artifacts.length} current referenced files.`,
);
