#!/usr/bin/env node

import { createHash } from "node:crypto";
import {
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import process from "node:process";
import { spawn } from "node:child_process";
import { setTimeout as wait } from "node:timers/promises";
import { fileURLToPath, pathToFileURL } from "node:url";

const DEFAULT_ATTEMPTS = 15;
const DEFAULT_RETRY_BASE_MS = 2_000;
const DEFAULT_RETRY_MAX_MS = 10_000;
const DEFAULT_REQUEST_TIMEOUT_MS = 15_000;
const DEFAULT_CHILD_TIMEOUT_MS = 15_000;
const CHILD_OUTPUT_LIMIT = 16_384;
const JAVASCRIPT_MIME_TYPES = new Set([
  "application/javascript",
  "text/javascript",
]);
const CONTENT_DIGEST_PATTERN = "([0-9a-f]{64})";
const ASSET_SPECS = [
  {
    hashKey: "indexSha256",
    mimeTypes: new Set(["text/html"]),
    name: "page",
    relativePath: "./",
    repositoryPath: "index.html",
  },
  {
    hashKey: "glueSha256",
    mimeTypes: JAVASCRIPT_MIME_TYPES,
    name: "glue",
    relativePath: "./pkg/quickjs_oxide_web.js",
    repositoryPath: "pkg/quickjs_oxide_web.js",
  },
  {
    hashKey: "wasmSha256",
    mimeTypes: new Set(["application/wasm"]),
    name: "wasm",
    relativePath: "./pkg/quickjs_oxide_web_bg.wasm",
    repositoryPath: "pkg/quickjs_oxide_web_bg.wasm",
  },
];
const PAGE_SPEC = ASSET_SPECS[0];
const CONTENT_REFERENCE_SPECS = {
  app: {
    mimeTypes: JAVASCRIPT_MIME_TYPES,
    name: "app",
    pattern: new RegExp(
      `src="(\\./app\\.${CONTENT_DIGEST_PATTERN}\\.js)"`,
      "gu",
    ),
  },
  glue: {
    mimeTypes: JAVASCRIPT_MIME_TYPES,
    name: "glue",
    pattern: new RegExp(
      `const PACKAGE_SCRIPT = "(\\./pkg/quickjs_oxide_web\\.${CONTENT_DIGEST_PATTERN}\\.js)";`,
      "gu",
    ),
  },
  wasm: {
    mimeTypes: new Set(["application/wasm"]),
    name: "wasm",
    pattern: new RegExp(
      `const PACKAGE_WASM = "(\\./pkg/quickjs_oxide_web_bg\\.${CONTENT_DIGEST_PATTERN}\\.wasm)";`,
      "gu",
    ),
  },
  worker: {
    mimeTypes: JAVASCRIPT_MIME_TYPES,
    name: "worker",
    pattern: new RegExp(
      `new Worker\\("(\\./worker\\.${CONTENT_DIGEST_PATTERN}\\.js)"`,
      "gu",
    ),
  },
};

class AuthenticatedArtifactError extends Error {
  constructor(cause) {
    super("authenticated Pages artifact failed", { cause });
    this.name = "AuthenticatedArtifactError";
  }
}

function requireInteger(name, value, minimum, maximum) {
  const number = Number(value);
  if (!Number.isSafeInteger(number) || number < minimum || number > maximum) {
    throw new TypeError(
      `${name} must be an integer between ${minimum} and ${maximum}`,
    );
  }
  return number;
}

function normalizedBaseUrl(value) {
  const url = new URL(value);
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw new TypeError("Pages URL must use http or https");
  }
  if (!url.pathname.endsWith("/")) {
    url.pathname += "/";
  }
  url.search = "";
  url.hash = "";
  return url;
}

function normalizedMime(response) {
  return (response.headers.get("content-type") || "")
    .split(";", 1)[0]
    .trim()
    .toLowerCase();
}

export function sha256Bytes(value) {
  return createHash("sha256").update(value).digest("hex");
}

export async function hashPagesArtifact(pagesDir) {
  const hashes = {};
  for (const spec of ASSET_SPECS) {
    const bytes = await readFile(path.join(pagesDir, spec.repositoryPath));
    hashes[spec.hashKey] = sha256Bytes(bytes);
  }
  return hashes;
}

function validatedExpectedHashes(value) {
  if (!value || typeof value !== "object") {
    throw new TypeError("expected Pages SHA-256 values are required");
  }
  const hashes = {};
  for (const spec of ASSET_SPECS) {
    const digest = value[spec.hashKey];
    if (typeof digest !== "string" || !/^[0-9a-f]{64}$/u.test(digest)) {
      throw new TypeError(`${spec.hashKey} must be an exact lowercase SHA-256`);
    }
    hashes[spec.hashKey] = digest;
  }
  return hashes;
}

function cacheBustedUrl(baseUrl, relativePath, expectedCommit, attempt) {
  const url = new URL(relativePath, baseUrl);
  url.searchParams.set(
    "quickjs_oxide_verify",
    `${expectedCommit}-${attempt}-${Date.now()}`,
  );
  return url;
}

async function fetchAsset(spec, url, signal) {
  let response;
  try {
    response = await fetch(url, {
      cache: "no-store",
      redirect: "follow",
      signal,
    });
  } catch (error) {
    throw new Error(`${url.pathname} fetch failed`, { cause: error });
  }

  const mime = normalizedMime(response);
  if (!response.ok) {
    throw new Error(`${url.pathname} returned HTTP ${response.status}`);
  }
  if (!spec.mimeTypes.has(mime)) {
    throw new Error(
      `${url.pathname} returned ${mime || "no Content-Type"}; expected ${[
        ...spec.mimeTypes,
      ].join(" or ")}`,
    );
  }
  let body;
  try {
    body = Buffer.from(await response.arrayBuffer());
  } catch (error) {
    throw new Error(`${url.pathname} body read failed`, { cause: error });
  }
  return { body, mime, spec };
}

function authenticateAssets(assets, expectedHashes) {
  for (const asset of Object.values(assets)) {
    const actual = sha256Bytes(asset.body);
    const expected = expectedHashes[asset.spec.hashKey];
    if (actual !== expected) {
      throw new Error(
        `${asset.spec.repositoryPath} SHA-256 mismatch; ` +
          `expected ${expected}, received ${actual}`,
      );
    }
  }
}

function authenticatedContentReference(source, spec) {
  const matches = [...source.matchAll(spec.pattern)];
  if (matches.length !== 1) {
    throw new Error(
      `authenticated ${spec.name} parent contains ${matches.length} ` +
        `content-addressed ${spec.name} reference${matches.length === 1 ? "" : "s"}`,
    );
  }
  return {
    digest: matches[0][2],
    mimeTypes: spec.mimeTypes,
    name: spec.name,
    relativePath: matches[0][1],
    repositoryPath: matches[0][1].replace(/^\.\//u, ""),
  };
}

function requireExpectedReference(reference, expectedDigest) {
  if (reference.digest !== expectedDigest) {
    throw new Error(
      `authenticated worker references ${reference.repositoryPath} but ` +
        `the workflow expects SHA-256 ${expectedDigest}`,
    );
  }
}

async function fetchContentAddressedAsset({
  attempt,
  baseUrl,
  expectedCommit,
  reference,
  signal,
}) {
  const asset = await fetchAsset(
    reference,
    cacheBustedUrl(
      baseUrl,
      reference.relativePath,
      expectedCommit,
      attempt,
    ),
    signal,
  );
  const actual = sha256Bytes(asset.body);
  if (actual !== reference.digest) {
    throw new Error(
      `${reference.repositoryPath} content-address mismatch; ` +
        `filename declares ${reference.digest}, received ${actual}`,
    );
  }
  return asset;
}

function validatePageAndBuildLabel(page, wasm, expectedCommit) {
  const requiredPageMarkers = [
    'id="build-commit"',
    'id="frozen-global-vector"',
    'id="parity-contract-link"',
    'id="test262-progress-link"',
    "function that returns 42",
    "real quickjs-oxide Rust interpreter compiled to WebAssembly",
    "68,145 passes / 68,197 runnable / 102,037 total",
    "R3dz-A module namespace admission",
    "+37 namespace passes",
    "8 detail-only rows",
    "pre-parity",
  ];
  const missingPageMarker = requiredPageMarkers.find(
    (marker) => !page.includes(marker),
  );
  if (missingPageMarker) {
    throw new Error(
      `downloaded HTML lacks playground marker ${missingPageMarker}`,
    );
  }
  if (!wasm.includes(Buffer.from(expectedCommit, "utf8"))) {
    throw new Error(
      `downloaded WASM does not contain build label ${expectedCommit}`,
    );
  }
}

function validatedGlueModule(glue) {
  const normalizedGlue = glue.toString("utf8").trim();
  if (!normalizedGlue.startsWith("let wasm_bindgen = (function(exports) {")) {
    throw new Error("authenticated JavaScript is not the no-modules binding");
  }
  if (!normalizedGlue.endsWith("})({ __proto__: null });")) {
    throw new Error("authenticated no-modules binding has an unexpected ending");
  }
  return Buffer.from(`${normalizedGlue}\nexport default wasm_bindgen;\n`, "utf8");
}

function sanitizedChildEnvironment() {
  const environment = { LANG: "C", LC_ALL: "C" };
  for (const name of [
    "PATH",
    "SystemRoot",
    "WINDIR",
    "TMPDIR",
    "TEMP",
    "TMP",
  ]) {
    if (process.env[name]) {
      environment[name] = process.env[name];
    }
  }
  return environment;
}

function appendBoundedOutput(current, chunk) {
  if (current.length >= CHILD_OUTPUT_LIMIT) {
    return current;
  }
  return (current + chunk.toString("utf8")).slice(0, CHILD_OUTPUT_LIMIT);
}

function terminateChildProcessGroup(child) {
  if (!child || !Number.isInteger(child.pid)) {
    return;
  }

  if (process.platform !== "win32") {
    try {
      process.kill(-child.pid, "SIGKILL");
      return;
    } catch (error) {
      if (error?.code !== "ESRCH" && error?.code !== "EPERM") {
        console.warn(`failed to kill authenticated process group: ${error}`);
      }
    }
  }

  if (child.exitCode === null && child.signalCode === null) {
    child.kill("SIGKILL");
  }
}

async function waitForChild(child, timeoutMs) {
  let stderr = "";
  let stdout = "";
  child.stderr.on("data", (chunk) => {
    stderr = appendBoundedOutput(stderr, chunk);
  });
  child.stdout.on("data", (chunk) => {
    stdout = appendBoundedOutput(stdout, chunk);
  });

  await new Promise((resolve, reject) => {
    let complete = false;
    let killFallback = null;
    let timedOut = false;
    const finish = (callback, value) => {
      if (complete) {
        return;
      }
      complete = true;
      clearTimeout(timeout);
      clearTimeout(killFallback);
      callback(value);
    };
    const timeout = setTimeout(() => {
      timedOut = true;
      killFallback = setTimeout(() => {
        terminateChildProcessGroup(child);
        finish(
          reject,
          new Error(
            `authenticated WASM child exceeded ${timeoutMs} ms and did not exit after SIGKILL`,
          ),
        );
      }, 1_000);
      terminateChildProcessGroup(child);
    }, timeoutMs);

    child.once("error", (error) => finish(reject, error));
    child.once("exit", (code, signal) => {
      if (timedOut) {
        finish(
          reject,
          new Error(
            `authenticated WASM child exceeded ${timeoutMs} ms and was killed`,
          ),
        );
        return;
      }
      if (code !== 0) {
        const diagnostic = (stderr || stdout || `signal ${signal || "none"}`).trim();
        finish(
          reject,
          new Error(`authenticated WASM child exited ${code}: ${diagnostic}`),
        );
        return;
      }
      finish(resolve);
    });
  });
}

async function executeAuthenticatedArtifact({
  childTimeoutMs,
  executionMarkerPath,
  executionTempRoot,
  expectedMissingChildEnvName,
  glue,
  wasm,
}) {
  const executionDir = await mkdtemp(
    path.join(
      executionTempRoot || tmpdir(),
      "quickjs-oxide-live-pages.",
    ),
  );
  const bindingPath = path.join(executionDir, "binding.mjs");
  const wasmPath = path.join(executionDir, "engine.wasm");
  const receiptPath = path.join(executionDir, "receipt.json");
  let child = null;

  try {
    await writeFile(bindingPath, validatedGlueModule(glue), {
      flag: "wx",
      mode: 0o600,
    });
    await writeFile(wasmPath, wasm, { flag: "wx", mode: 0o600 });
    if (executionMarkerPath) {
      await writeFile(executionMarkerPath, "authenticated child started\n", {
        flag: "wx",
        mode: 0o600,
      });
    }

    const childArguments = [
      fileURLToPath(import.meta.url),
      "--artifact-child",
      bindingPath,
      wasmPath,
      receiptPath,
    ];
    if (expectedMissingChildEnvName) {
      childArguments.push(expectedMissingChildEnvName);
    }
    child = spawn(
      process.execPath,
      childArguments,
      {
        cwd: executionDir,
        detached: process.platform !== "win32",
        env: sanitizedChildEnvironment(),
        stdio: ["ignore", "pipe", "pipe"],
      },
    );
    await waitForChild(child, childTimeoutMs);

    const receipt = JSON.parse(await readFile(receiptPath, "utf8"));
    if (!receipt || typeof receipt !== "object") {
      throw new TypeError("authenticated WASM child returned no receipt");
    }
    return receipt;
  } finally {
    terminateChildProcessGroup(child);
    await rm(executionDir, { force: true, recursive: true });
  }
}

async function verifyAttempt({
  attempt,
  baseUrl,
  childTimeoutMs,
  executionMarkerPath,
  executionTempRoot,
  expectedCommit,
  expectedHashes,
  expectedMissingChildEnvName,
  requestTimeoutMs,
}) {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), requestTimeoutMs);

  try {
    const page = await fetchAsset(
      PAGE_SPEC,
      cacheBustedUrl(
        baseUrl,
        PAGE_SPEC.relativePath,
        expectedCommit,
        attempt,
      ),
      controller.signal,
    );
    authenticateAssets({ page }, expectedHashes);

    let appReference;
    try {
      appReference = authenticatedContentReference(
        page.body.toString("utf8"),
        CONTENT_REFERENCE_SPECS.app,
      );
    } catch (error) {
      throw new AuthenticatedArtifactError(error);
    }
    const app = await fetchContentAddressedAsset({
      attempt,
      baseUrl,
      expectedCommit,
      reference: appReference,
      signal: controller.signal,
    });

    let workerReference;
    try {
      workerReference = authenticatedContentReference(
        app.body.toString("utf8"),
        CONTENT_REFERENCE_SPECS.worker,
      );
    } catch (error) {
      throw new AuthenticatedArtifactError(error);
    }
    const worker = await fetchContentAddressedAsset({
      attempt,
      baseUrl,
      expectedCommit,
      reference: workerReference,
      signal: controller.signal,
    });

    let glueReference;
    let wasmReference;
    try {
      const workerSource = worker.body.toString("utf8");
      glueReference = authenticatedContentReference(
        workerSource,
        CONTENT_REFERENCE_SPECS.glue,
      );
      wasmReference = authenticatedContentReference(
        workerSource,
        CONTENT_REFERENCE_SPECS.wasm,
      );
      requireExpectedReference(glueReference, expectedHashes.glueSha256);
      requireExpectedReference(wasmReference, expectedHashes.wasmSha256);
    } catch (error) {
      throw new AuthenticatedArtifactError(error);
    }
    const [glue, wasm] = await Promise.all([
      fetchContentAddressedAsset({
        attempt,
        baseUrl,
        expectedCommit,
        reference: glueReference,
        signal: controller.signal,
      }),
      fetchContentAddressedAsset({
        attempt,
        baseUrl,
        expectedCommit,
        reference: wasmReference,
        signal: controller.signal,
      }),
    ]);
    const assets = { app, glue, page, wasm, worker };

    try {
      validatePageAndBuildLabel(
        assets.page.body.toString("utf8"),
        assets.wasm.body,
        expectedCommit,
      );

      const receipt = await executeAuthenticatedArtifact({
        childTimeoutMs,
        executionMarkerPath,
        executionTempRoot,
        expectedMissingChildEnvName,
        glue: assets.glue.body,
        wasm: assets.wasm.body,
      });
      const metadata = receipt.metadata;
      const result = receipt.result;
      if (!metadata || metadata.engine !== "quickjs-oxide") {
        throw new Error("authenticated WASM reported an unexpected engine");
      }
      if (metadata.buildCommit !== expectedCommit) {
        throw new Error(
          `authenticated WASM reports commit ${metadata.buildCommit}; ` +
            `expected ${expectedCommit}`,
        );
      }
      if (
        expectedMissingChildEnvName &&
        (
          receipt.missingEnvironment?.name !== expectedMissingChildEnvName ||
          receipt.missingEnvironment?.absent !== true
        )
      ) {
        throw new Error(
          `authenticated WASM child did not prove ${expectedMissingChildEnvName} absent`,
        );
      }
      if (
        !result ||
        result.ok !== true ||
        result.kind !== "number" ||
        result.text !== "42"
      ) {
        throw new Error(
          `authenticated WASM returned ${JSON.stringify(result)}; ` +
            "expected number 42",
        );
      }

      return {
        commit: metadata.buildCommit,
        glueBytes: assets.glue.body.byteLength,
        pageBytes: assets.page.body.byteLength,
        result,
        wasmBytes: assets.wasm.body.byteLength,
      };
    } catch (error) {
      throw new AuthenticatedArtifactError(error);
    }
  } finally {
    controller.abort();
    clearTimeout(timeout);
  }
}

function errorDetail(error) {
  const details = [];
  const seen = new Set();
  let current = error;

  while (current !== null && current !== undefined && !seen.has(current)) {
    seen.add(current);
    if (current instanceof Error) {
      const code = typeof current.code === "string" ? ` [${current.code}]` : "";
      details.push(`${current.name}${code}: ${current.message}`);
      current = current.cause;
    } else {
      details.push(String(current));
      break;
    }
  }
  return details.join("; caused by ");
}

export async function verifyPagesDeployment({
  attempts = DEFAULT_ATTEMPTS,
  baseUrl,
  childTimeoutMs = DEFAULT_CHILD_TIMEOUT_MS,
  executionMarkerPath = null,
  executionTempRoot = null,
  expectedCommit,
  expectedHashes,
  expectedMissingChildEnvName = null,
  label = "Live Pages",
  requestTimeoutMs = DEFAULT_REQUEST_TIMEOUT_MS,
  retryBaseMs = DEFAULT_RETRY_BASE_MS,
  retryMaxMs = DEFAULT_RETRY_MAX_MS,
}) {
  const authenticatedHashes = validatedExpectedHashes(expectedHashes);
  if (
    executionTempRoot !== null &&
    (typeof executionTempRoot !== "string" || !path.isAbsolute(executionTempRoot))
  ) {
    throw new TypeError("executionTempRoot must be an absolute path");
  }
  if (
    expectedMissingChildEnvName !== null &&
    (
      typeof expectedMissingChildEnvName !== "string" ||
      !/^[A-Za-z_][A-Za-z0-9_]*$/u.test(expectedMissingChildEnvName)
    )
  ) {
    throw new TypeError("expectedMissingChildEnvName must be an environment name");
  }
  if (!expectedCommit || !/^[0-9A-Za-z._-]{1,64}$/u.test(expectedCommit)) {
    throw new TypeError("expected commit must be a non-empty safe build label");
  }
  const normalizedUrl = normalizedBaseUrl(baseUrl);
  const attemptLimit = requireInteger("attempts", attempts, 1, 60);
  const requestTimeout = requireInteger(
    "requestTimeoutMs",
    requestTimeoutMs,
    100,
    120_000,
  );
  const childTimeout = requireInteger(
    "childTimeoutMs",
    childTimeoutMs,
    100,
    120_000,
  );
  const retryBase = requireInteger("retryBaseMs", retryBaseMs, 0, 60_000);
  const retryMaximum = requireInteger("retryMaxMs", retryMaxMs, 0, 60_000);
  const diagnostics = [];
  let retryDelay = Math.min(retryBase, retryMaximum);

  for (let attempt = 1; attempt <= attemptLimit; attempt += 1) {
    try {
      const receipt = await verifyAttempt({
        attempt,
        baseUrl: normalizedUrl,
        childTimeoutMs: childTimeout,
        executionMarkerPath,
        executionTempRoot,
        expectedCommit,
        expectedHashes: authenticatedHashes,
        expectedMissingChildEnvName,
        requestTimeoutMs: requestTimeout,
      });
      console.log(
        `${label} gate passed: online WASM commit ${receipt.commit.slice(0, 7)} ` +
          `evaluated a JavaScript function to number 42 ` +
          `(${receipt.wasmBytes} WASM bytes).`,
      );
      return receipt;
    } catch (error) {
      const detail = errorDetail(error);
      diagnostics.push(`attempt ${attempt}/${attemptLimit}: ${detail}`);
      if (
        error instanceof AuthenticatedArtifactError ||
        attempt === attemptLimit
      ) {
        break;
      }
      console.warn(`${label} is not current yet; ${diagnostics.at(-1)}`);
      if (retryDelay > 0) {
        await wait(retryDelay);
      }
      retryDelay = Math.min(retryMaximum, Math.max(retryDelay * 2, retryBase));
    }
  }

  throw new Error(
    `${label} did not serve authenticated commit ${expectedCommit} ` +
      `from ${normalizedUrl.href}\n${diagnostics.join("\n")}`,
  );
}

async function runArtifactChild(
  bindingPath,
  wasmPath,
  receiptPath,
  expectedMissingEnvironmentName = null,
) {
  const missingEnvironment = expectedMissingEnvironmentName
    ? {
        absent: process.env[expectedMissingEnvironmentName] === undefined,
        name: expectedMissingEnvironmentName,
      }
    : null;
  if (missingEnvironment && !missingEnvironment.absent) {
    throw new Error(
      `parent environment ${expectedMissingEnvironmentName} reached the child`,
    );
  }

  const wasm = new WebAssembly.Module(await readFile(wasmPath));
  const exports = new Map(
    WebAssembly.Module.exports(wasm).map((entry) => [entry.name, entry.kind]),
  );
  if (
    exports.get("engine_metadata") !== "function" ||
    exports.get("evaluate") !== "function"
  ) {
    throw new TypeError("authenticated WASM lacks the engine function exports");
  }

  const bindingUrl = pathToFileURL(bindingPath);
  bindingUrl.searchParams.set("child", Date.now().toString());
  const { default: wasmBindings } = await import(bindingUrl.href);
  if (typeof wasmBindings !== "function") {
    throw new TypeError("authenticated binding has no initialization function");
  }

  await wasmBindings({ module_or_path: wasm });
  if (
    typeof wasmBindings.engine_metadata !== "function" ||
    typeof wasmBindings.evaluate !== "function"
  ) {
    throw new TypeError("authenticated binding lacks the engine API");
  }
  const metadata = wasmBindings.engine_metadata();
  const result = wasmBindings.evaluate("(function () { return 42; })()");
  await writeFile(
    receiptPath,
    `${JSON.stringify({ metadata, missingEnvironment, result })}\n`,
    { flag: "wx", mode: 0o600 },
  );
}

async function main() {
  const expectedCommit =
    process.env.QUICKJS_OXIDE_COMMIT || process.env.GITHUB_SHA;
  const baseUrl = process.argv[2] || process.env.QUICKJS_OXIDE_PAGES_URL;
  if (!baseUrl) {
    throw new Error(
      "set QUICKJS_OXIDE_PAGES_URL or pass the deployed Pages URL as argv[2]",
    );
  }
  if (!expectedCommit) {
    throw new Error("set QUICKJS_OXIDE_COMMIT or GITHUB_SHA");
  }
  if (
    process.env.GITHUB_ACTIONS === "true" &&
    !/^[0-9a-f]{40}$/u.test(expectedCommit)
  ) {
    throw new Error("GitHub deployment verification requires an exact commit");
  }

  await verifyPagesDeployment({
    attempts: process.env.QUICKJS_OXIDE_LIVE_ATTEMPTS || DEFAULT_ATTEMPTS,
    baseUrl,
    childTimeoutMs:
      process.env.QUICKJS_OXIDE_LIVE_CHILD_TIMEOUT_MS ||
      DEFAULT_CHILD_TIMEOUT_MS,
    expectedCommit,
    expectedHashes: {
      glueSha256: process.env.QUICKJS_OXIDE_GLUE_SHA256,
      indexSha256: process.env.QUICKJS_OXIDE_INDEX_SHA256,
      wasmSha256: process.env.QUICKJS_OXIDE_WASM_SHA256,
    },
    requestTimeoutMs:
      process.env.QUICKJS_OXIDE_LIVE_TIMEOUT_MS || DEFAULT_REQUEST_TIMEOUT_MS,
    retryBaseMs:
      process.env.QUICKJS_OXIDE_LIVE_RETRY_MS || DEFAULT_RETRY_BASE_MS,
    retryMaxMs:
      process.env.QUICKJS_OXIDE_LIVE_RETRY_MAX_MS || DEFAULT_RETRY_MAX_MS,
  });
}

const invokedPath = process.argv[1] ? pathToFileURL(process.argv[1]).href : null;
if (invokedPath === import.meta.url) {
  const operation = process.argv[2];
  const entry = operation === "--artifact-child"
    ? runArtifactChild(
        process.argv[3],
        process.argv[4],
        process.argv[5],
        process.argv[6],
      )
    : main();
  entry.catch((error) => {
    console.error(error instanceof Error ? error.stack : error);
    process.exitCode = 1;
  });
}
