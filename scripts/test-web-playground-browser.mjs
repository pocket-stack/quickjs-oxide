import assert from "node:assert/strict";
import {
  mkdtemp,
  readFile,
  readdir,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import path from "node:path";
import process from "node:process";
import { chromium } from "playwright";
import {
  hashPagesArtifact,
  sha256Bytes,
  verifyPagesDeployment,
} from "./test-live-pages.mjs";

const repoRoot = path.resolve(import.meta.dirname, "..");
const pagesDir = path.resolve(
  process.env.QUICKJS_OXIDE_PAGES_DIR || path.join(repoRoot, "target/pages"),
);
const pagesBasePath = "/quickjs-oxide/";
const upstreamPath = path.join(repoRoot, "compat/upstream.toml");
const requiredArtifactFiles = [
  "index.html",
  "og.png",
  "worker.js",
  "pkg/quickjs_oxide_web.js",
  "pkg/quickjs_oxide_web_bg.wasm",
];
const contentDigestPattern = "[0-9a-f]{64}";
const requiredBrowserResources = [
  [new RegExp(`^${pagesBasePath}app\\.${contentDigestPattern}\\.js$`, "u"), "app"],
  [
    new RegExp(`^${pagesBasePath}examples\\.${contentDigestPattern}\\.js$`, "u"),
    "examples",
  ],
  [
    new RegExp(`^${pagesBasePath}style\\.${contentDigestPattern}\\.css$`, "u"),
    "stylesheet",
  ],
  [
    new RegExp(`^${pagesBasePath}worker\\.${contentDigestPattern}\\.js$`, "u"),
    "worker",
  ],
  [
    new RegExp(
      `^${pagesBasePath}pkg/quickjs_oxide_web\\.${contentDigestPattern}\\.js$`,
      "u",
    ),
    "WASM glue",
  ],
  [
    new RegExp(
      `^${pagesBasePath}pkg/quickjs_oxide_web_bg\\.${contentDigestPattern}\\.wasm$`,
      "u",
    ),
    "WASM binary",
  ],
];

function contentType(filePath) {
  switch (path.extname(filePath)) {
    case ".css":
      return "text/css; charset=utf-8";
    case ".html":
      return "text/html; charset=utf-8";
    case ".js":
      return "text/javascript; charset=utf-8";
    case ".wasm":
      return "application/wasm";
    default:
      return "application/octet-stream";
  }
}

async function assertArtifactExists() {
  for (const relativePath of requiredArtifactFiles) {
    const filePath = path.join(pagesDir, relativePath);
    const metadata = await stat(filePath).catch(() => null);
    assert.ok(
      metadata?.isFile(),
      `missing Pages artifact file ${path.relative(repoRoot, filePath)}`,
    );
  }
}

function artifactPath(requestUrl) {
  const pathname = decodeURIComponent(
    new URL(requestUrl || "/", "http://127.0.0.1").pathname,
  );
  if (!pathname.startsWith(pagesBasePath)) {
    return null;
  }

  const relativePath = pathname === pagesBasePath
    ? "index.html"
    : pathname.slice(pagesBasePath.length);
  const filePath = path.resolve(pagesDir, relativePath);

  if (filePath !== pagesDir && !filePath.startsWith(`${pagesDir}${path.sep}`)) {
    return null;
  }

  return filePath;
}

async function startArtifactServer() {
  const serverErrors = [];
  let customGlueBody = null;
  let customGlueMime = null;
  let customGlueVerificationAttempt = null;
  let customFiles = new Map();
  let delayedVerificationPath = null;
  let delayedVerificationMs = 0;
  let rejectedVerificationAttempt = null;
  const server = createServer(async (request, response) => {
    try {
      const filePath = artifactPath(request.url);
      if (filePath === null) {
        response.writeHead(403).end("Forbidden\n");
        return;
      }

      const parsedRequestUrl = new URL(
        request.url || "/",
        "http://127.0.0.1",
      );
      const verificationToken = parsedRequestUrl.searchParams.get(
        "quickjs_oxide_verify",
      );
      const verificationAttempt = verificationToken?.match(
        /-([0-9]+)-[0-9]+$/u,
      )?.[1];
      if (
        rejectedVerificationAttempt !== null &&
        verificationAttempt === String(rejectedVerificationAttempt)
      ) {
        response.writeHead(503).end("Fixture deployment is still propagating\n");
        return;
      }
      if (
        verificationToken &&
        delayedVerificationPath === parsedRequestUrl.pathname
      ) {
        await new Promise((resolve) => {
          setTimeout(resolve, delayedVerificationMs);
        });
        if (request.destroyed || response.destroyed) {
          return;
        }
      }

      const customFile = customFiles.get(parsedRequestUrl.pathname) || null;
      const metadata = customFile
        ? { isFile: () => true, size: customFile.body.byteLength }
        : await stat(filePath).catch(() => null);
      if (!metadata?.isFile()) {
        response.writeHead(404).end("Not found\n");
        return;
      }

      const isGlue = path.dirname(filePath) === path.join(pagesDir, "pkg") &&
        /^quickjs_oxide_web(?:\.[0-9a-f]{64})?\.js$/u.test(
          path.basename(filePath),
        );
      const useCustomGlue = isGlue && customGlueBody && (
        customGlueVerificationAttempt === null ||
        verificationAttempt === String(customGlueVerificationAttempt)
      );
      const body = request.method === "HEAD"
        ? null
        : useCustomGlue
        ? customGlueBody
        : customFile
        ? customFile.body
        : await readFile(filePath);
      response.writeHead(200, {
        "Cache-Control": "no-store",
        "Content-Length": body?.byteLength ?? metadata.size,
        "Content-Type": useCustomGlue && customGlueMime
          ? customGlueMime
          : customFile?.mime
          ? customFile.mime
          : contentType(filePath),
      });
      response.end(body);
    } catch (error) {
      serverErrors.push(error);
      response.writeHead(500).end("Internal server error\n");
    }
  });

  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });

  const address = server.address();
  assert.ok(address && typeof address === "object");

  return {
    delayVerificationPath(pathname, milliseconds = 0) {
      delayedVerificationPath = pathname;
      delayedVerificationMs = milliseconds;
    },
    rejectVerificationAttempt(attempt) {
      rejectedVerificationAttempt = attempt;
    },
    serveGlue(body, mime = null, verificationAttempt = null) {
      customGlueBody = body;
      customGlueMime = mime;
      customGlueVerificationAttempt = verificationAttempt;
    },
    serveFiles(files = new Map()) {
      customFiles = new Map(files);
    },
    server,
    serverErrors,
    url: `http://127.0.0.1:${address.port}${pagesBasePath}`,
  };
}

function stopServer(server) {
  return new Promise((resolve, reject) => {
    server.close((error) => (error ? reject(error) : resolve()));
  });
}

function processGroupFixtureGlue({
  expectedCommit,
  grandchildScriptPath,
  hang,
  pidMarkerPath,
}) {
  const hangStatement = hang ? "    while (true) {}\n" : "";
  return Buffer.from(
    "let wasm_bindgen = (function(exports) {\n" +
      `    const grandchildScriptPath = ${JSON.stringify(grandchildScriptPath)};\n` +
      `    const pidMarkerPath = ${JSON.stringify(pidMarkerPath)};\n` +
      "    async function waitForGrandchildMarker() {\n" +
      '        const { readFile } = await import("node:fs/promises");\n' +
      "        for (let attempt = 0; attempt < 200; attempt += 1) {\n" +
      "            try {\n" +
      '                await readFile(pidMarkerPath, "utf8");\n' +
      "                return;\n" +
      "            } catch (error) {\n" +
      '                if (error?.code !== "ENOENT") {\n' +
      "                    throw error;\n" +
      "                }\n" +
      "            }\n" +
      "            await new Promise((resolve) => setTimeout(resolve, 5));\n" +
      "        }\n" +
      '        throw new Error("grandchild PID marker did not appear");\n' +
      "    }\n" +
      "    async function initialize() {\n" +
      '        const { spawn } = await import("node:child_process");\n' +
      "        const grandchild = spawn(\n" +
      "            process.execPath,\n" +
      "            [grandchildScriptPath, pidMarkerPath],\n" +
      '            { stdio: "ignore" },\n' +
      "        );\n" +
      "        grandchild.unref();\n" +
      "        await waitForGrandchildMarker();\n" +
      hangStatement +
      "        return exports;\n" +
      "    }\n" +
      "    initialize.engine_metadata = () => ({\n" +
      '        engine: "quickjs-oxide",\n' +
      `        buildCommit: ${JSON.stringify(expectedCommit)},\n` +
      "    });\n" +
      "    initialize.evaluate = () => ({\n" +
      "        ok: true,\n" +
      '        kind: "number",\n' +
      '        text: "42",\n' +
      "    });\n" +
      "    return initialize;\n" +
      "})({ __proto__: null });\n",
    "utf8",
  );
}

function replaceExactlyOnce(source, reference, replacement, label) {
  assert.equal(
    source.split(reference).length,
    2,
    `${label} fixture reference is not unique`,
  );
  return source.replace(reference, replacement);
}

function contentAddressedExecutionFixture({
  appSource,
  expectedHashes,
  glue,
  indexSource,
  workerSource,
}) {
  const originalGlueReference = workerSource.match(
    /const PACKAGE_SCRIPT = "(\.\/pkg\/quickjs_oxide_web\.[0-9a-f]{64}\.js)";/u,
  )?.[1];
  const originalWorkerReference = appSource.match(
    /new Worker\("(\.\/worker\.[0-9a-f]{64}\.js)"/u,
  )?.[1];
  const originalAppReference = indexSource.match(
    /src="(\.\/app\.[0-9a-f]{64}\.js)"/u,
  )?.[1];
  assert.ok(originalGlueReference, "worker fixture has no hashed glue reference");
  assert.ok(originalWorkerReference, "app fixture has no hashed worker reference");
  assert.ok(originalAppReference, "index fixture has no hashed app reference");

  const glueDigest = sha256Bytes(glue);
  const glueReference = `./pkg/quickjs_oxide_web.${glueDigest}.js`;
  const worker = Buffer.from(
    replaceExactlyOnce(
      workerSource,
      originalGlueReference,
      glueReference,
      "worker glue",
    ),
    "utf8",
  );
  const workerDigest = sha256Bytes(worker);
  const workerReference = `./worker.${workerDigest}.js`;
  const app = Buffer.from(
    replaceExactlyOnce(
      appSource,
      originalWorkerReference,
      workerReference,
      "app worker",
    ),
    "utf8",
  );
  const appDigest = sha256Bytes(app);
  const appReference = `./app.${appDigest}.js`;
  const index = Buffer.from(
    replaceExactlyOnce(
      indexSource,
      originalAppReference,
      appReference,
      "index app",
    ),
    "utf8",
  );

  return {
    expectedHashes: {
      ...expectedHashes,
      glueSha256: glueDigest,
      indexSha256: sha256Bytes(index),
    },
    files: new Map([
      [
        pagesBasePath,
        { body: index, mime: "text/html; charset=utf-8" },
      ],
      [
        `${pagesBasePath}${appReference.slice(2)}`,
        { body: app, mime: "text/javascript; charset=utf-8" },
      ],
      [
        `${pagesBasePath}${workerReference.slice(2)}`,
        { body: worker, mime: "text/javascript; charset=utf-8" },
      ],
      [
        `${pagesBasePath}${glueReference.slice(2)}`,
        { body: glue, mime: "text/javascript; charset=utf-8" },
      ],
    ]),
  };
}

async function readFixturePid(markerPath) {
  const value = (await readFile(markerPath, "utf8")).trim();
  assert.match(value, /^[1-9][0-9]*$/u, "fixture wrote an invalid PID marker");
  return Number(value);
}

async function assertProcessExited(pid, label) {
  const deadline = Date.now() + 3_000;
  while (Date.now() < deadline) {
    try {
      process.kill(pid, 0);
    } catch (error) {
      if (error?.code === "ESRCH") {
        return;
      }
      throw error;
    }
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  assert.fail(`${label} process ${pid} survived process-group cleanup`);
}

async function expectedQuickJsVersion() {
  const upstream = await readFile(upstreamPath, "utf8");
  const quickJsSection = upstream.match(
    /^\[quickjs\]\s*$([\s\S]*?)(?=^\[[^\]]+\]\s*$|(?![\s\S]))/m,
  );
  assert.ok(quickJsSection, "compat/upstream.toml has no [quickjs] section");

  const version = quickJsSection[1].match(/^version\s*=\s*"([^"]+)"\s*$/m);
  assert.ok(version, "compat/upstream.toml has no pinned QuickJS version");
  return version[1];
}

async function assertResult(page, expectedText) {
  await page.locator("#result-state").filter({ hasText: "Complete" }).waitFor({
    state: "visible",
    timeout: 10_000,
  });
  await page.locator("#result-content").waitFor({ state: "visible" });
  assert.equal((await page.locator("#result-value").textContent())?.trim(), expectedText);
  assert.equal((await page.locator("#result-type").textContent())?.trim(), "number");
}

async function runAcceptance(url, serverErrors) {
  const browser = await chromium.launch({ headless: true });
  const context = await browser.newContext();
  const consoleErrors = [];
  const pageErrors = [];
  const requestFailures = [];
  const badResponses = [];
  const loadedResources = new Set();
  const workerUrls = [];

  context.on("console", (message) => {
    if (message.type() !== "error") {
      return;
    }

    const location = message.location();
    const source = message.worker() ? "worker" : "page";
    consoleErrors.push(`${source} console: ${message.text()} (${location.url})`);
  });
  context.on("weberror", (webError) => {
    const error = webError.error();
    const location = webError.location();
    pageErrors.push(
      `page error: ${error.stack || error} (${location.url}:${location.line + 1})`,
    );
  });

  const page = await context.newPage();
  page.setDefaultTimeout(10_000);
  page.on("requestfailed", (request) => {
    requestFailures.push(
      `${request.method()} ${request.url()}: ${request.failure()?.errorText || "failed"}`,
    );
  });
  page.on("response", (response) => {
    const resourceUrl = new URL(response.url());
    if (resourceUrl.origin === new URL(url).origin) {
      loadedResources.add(resourceUrl.pathname);
    }
    if (response.status() >= 400) {
      badResponses.push(`${response.status()} ${response.url()}`);
    }
  });
  page.on("worker", (worker) => workerUrls.push(worker.url()));

  try {
    await page.goto(url, { waitUntil: "domcontentloaded" });
    await page.locator("#engine-status").filter({ hasText: "Engine ready" }).waitFor({
      state: "visible",
      timeout: 30_000,
    });

    assert.equal(await page.locator("#example-select").inputValue(), "return-42");
    assert.equal(
      (await page.locator("#example-description").textContent())?.trim(),
      "Calls a JavaScript function compiled and run by quickjs-oxide.",
    );
    assert.equal(
      (await page.locator("#example-expected").textContent())?.trim(),
      "42 · number",
    );
    assert.equal(await page.locator("#run-button").isEnabled(), true);
    await page.locator("#run-button").click();
    await assertResult(page, "42");

    const atomicsWaitOption = page.locator(
      '#example-select option[value="atomics-wait-policy"]',
    );
    assert.equal(
      await atomicsWaitOption.count(),
      1,
      "the Pages artifact does not expose the Atomics.wait host-policy example",
    );
    const selected = await page.locator("#example-select").selectOption(
      "atomics-wait-policy",
    );
    assert.deepEqual(selected, ["atomics-wait-policy"]);
    assert.equal(
      (await page.locator("#example-description").textContent())?.trim(),
      "Confirms that the browser host forbids synchronous blocking.",
    );
    assert.equal(
      (await page.locator("#example-expected").textContent())?.trim(),
      "42 · number",
    );
    await page.locator("#run-button").click();
    await assertResult(page, "42");

    const expectedCommit =
      process.env.QUICKJS_OXIDE_COMMIT || process.env.GITHUB_SHA || "local";
    const buildCommit = page.locator("#build-commit");
    assert.equal((await buildCommit.textContent())?.trim(), expectedCommit);
    assert.equal(await buildCommit.getAttribute("data-commit"), expectedCommit);
    const expectedCommitUrl = /^[0-9a-f]{7,64}$/iu.test(expectedCommit)
      ? `https://github.com/pocket-stack/quickjs-oxide/commit/${expectedCommit}`
      : "https://github.com/pocket-stack/quickjs-oxide";
    assert.equal(await buildCommit.getAttribute("href"), expectedCommitUrl);

    const quickJsVersion = await expectedQuickJsVersion();
    assert.equal(
      (await page.locator("#engine-version").textContent())?.trim(),
      "quickjs-oxide v0.0.1",
    );
    assert.equal(
      (await page.locator("#quickjs-target").textContent())?.trim(),
      `QuickJS ${quickJsVersion}`,
    );
    assert.equal(
      (await page.locator("#host-policy").textContent())?.trim(),
      "canBlock = false · blocking disabled",
    );
    assert.equal(
      await page.locator('meta[property="og:image"]').getAttribute("content"),
      "https://pocket-stack.github.io/quickjs-oxide/og.png",
    );

    const expectedDocumentationRef = /^[0-9a-f]{40}$/iu.test(expectedCommit)
      ? expectedCommit
      : "main";
    const expectedDocumentationBase =
      `https://github.com/pocket-stack/quickjs-oxide/blob/${expectedDocumentationRef}`;
    const parityContract = page.locator("#parity-contract-link");
    const test262Progress = page.locator("#test262-progress-link");
    assert.equal(
      await parityContract.getAttribute("href"),
      `${expectedDocumentationBase}/docs/parity.md`,
    );
    assert.equal(
      await parityContract.getAttribute("data-repository-ref"),
      expectedDocumentationRef,
    );
    assert.equal(
      await test262Progress.getAttribute("href"),
      `${expectedDocumentationBase}/docs/test262.md`,
    );
    assert.equal(
      await test262Progress.getAttribute("data-repository-ref"),
      expectedDocumentationRef,
    );
    assert.match(
      (await page.locator("#frozen-global-vector").textContent()) || "",
      /67,490 passes\s*\/\s*67,542 runnable\s*\/\s*102,037 total[\s\S]*base class admission \+816 global passes[\s\S]*finer class features\s*remain gated[\s\S]*pre-parity/,
    );

    assert.ok(
      workerUrls.some(
        (workerUrl) =>
          new RegExp(
            `^${pagesBasePath}worker\\.${contentDigestPattern}\\.js$`,
            "u",
          ).test(new URL(workerUrl).pathname),
      ),
      "the page did not create the content-addressed engine worker",
    );
    for (const [pattern, label] of requiredBrowserResources) {
      assert.ok(
        [...loadedResources].some((resource) => pattern.test(resource)),
        `browser did not load the content-addressed ${label}`,
      );
    }

    await page.waitForTimeout(100);
    assert.deepEqual(
      serverErrors.map(String),
      [],
      "artifact server failed while serving a request",
    );
    assert.deepEqual(requestFailures, [], "browser requests failed");
    assert.deepEqual(badResponses, [], "browser received error responses");
    assert.deepEqual(pageErrors, [], "page raised unhandled errors");
    assert.deepEqual(consoleErrors, [], "page or worker wrote console errors");

    console.log(
      `Browser/WASM acceptance passed: default and Atomics.wait policy examples returned 42; ` +
        `commit ${expectedCommit.slice(0, 7)} targets QuickJS ${quickJsVersion}.`,
    );
  } catch (error) {
    const engineState = await page.locator("#engine-status").textContent({ timeout: 500 })
      .catch(() => null);
    const resultState = await page.locator("#result-raw").textContent({ timeout: 500 })
      .catch(() => null);
    const diagnostics = [
      engineState ? `engine status: ${engineState.trim()}` : null,
      resultState ? `engine response: ${resultState.trim()}` : null,
      ...serverErrors.map((entry) => `server error: ${entry.stack || entry}`),
      ...requestFailures,
      ...badResponses,
      ...pageErrors,
      ...consoleErrors,
    ].filter(Boolean);
    if (diagnostics.length > 0) {
      console.error(`Browser diagnostics:\n${diagnostics.join("\n")}`);
    }
    throw error;
  } finally {
    await context.close();
    await browser.close();
  }
}

await assertArtifactExists();
const {
  delayVerificationPath,
  rejectVerificationAttempt,
  server,
  serverErrors,
  serveFiles,
  serveGlue,
  url,
} = await startArtifactServer();
const fixtureDir = await mkdtemp(
  path.join(tmpdir(), "quickjs-oxide-live-pages-test."),
);
const expectedCommit =
  process.env.QUICKJS_OXIDE_COMMIT || process.env.GITHUB_SHA || "local";
const expectedHashes = await hashPagesArtifact(pagesDir);
const originalApp = await readFile(path.join(pagesDir, "app.js"), "utf8");
const originalGlue = await readFile(
  path.join(pagesDir, "pkg/quickjs_oxide_web.js"),
);
const originalIndex = await readFile(
  path.join(pagesDir, "index.html"),
  "utf8",
);
const originalWorker = await readFile(
  path.join(pagesDir, "worker.js"),
  "utf8",
);
const appReference = originalIndex.match(
  /src="\.\/(app\.[0-9a-f]{64}\.js)"/u,
);
const workerReference = originalApp.match(
  /new Worker\("\.\/(worker\.[0-9a-f]{64}\.js)"/u,
);
const wasmReference = originalWorker.match(
  /const PACKAGE_WASM = "\.\/(pkg\/quickjs_oxide_web_bg\.[0-9a-f]{64}\.wasm)";/u,
);
assert.ok(appReference, "built index has no content-addressed app reference");
assert.ok(workerReference, "built app has no content-addressed worker reference");
assert.ok(wasmReference, "built worker has no content-addressed WASM reference");
const contentAddressedAppPath = `${pagesBasePath}${appReference[1]}`;
const contentAddressedWorkerPath = `${pagesBasePath}${workerReference[1]}`;
const contentAddressedWasmPath = `${pagesBasePath}${wasmReference[1]}`;
const grandchildScriptPath = path.join(fixtureDir, "grandchild.mjs");
const grandchildPidMarkers = [];
try {
  await writeFile(
    grandchildScriptPath,
    'import { writeFile } from "node:fs/promises";\n' +
      'import process from "node:process";\n' +
      "await writeFile(process.argv[2], `${process.pid}\\n`, " +
      '{ flag: "wx", mode: 0o600 });\n' +
      "setInterval(() => {}, 60_000);\n",
    { flag: "wx", mode: 0o600 },
  );
  await runAcceptance(url, serverErrors);

  const tamperedApp = Buffer.from(originalApp);
  tamperedApp[0] ^= 1;
  const appTamperMarker = path.join(fixtureDir, "tampered-app.executed");
  serveFiles(new Map([
    [
      contentAddressedAppPath,
      { body: tamperedApp, mime: "text/javascript; charset=utf-8" },
    ],
  ]));
  try {
    await assert.rejects(
      verifyPagesDeployment({
        attempts: 1,
        baseUrl: url,
        executionMarkerPath: appTamperMarker,
        expectedCommit,
        expectedHashes,
        label: "Tampered app fixture",
      }),
      /app\.[0-9a-f]{64}\.js content-address mismatch/,
    );
  } finally {
    serveFiles();
  }
  assert.equal(
    await stat(appTamperMarker).catch(() => null),
    null,
    "hash-mismatched app reached the execution child",
  );

  const tamperedWorker = Buffer.from(originalWorker);
  tamperedWorker[0] ^= 1;
  const workerTamperMarker = path.join(fixtureDir, "tampered-worker.executed");
  serveFiles(new Map([
    [
      contentAddressedWorkerPath,
      { body: tamperedWorker, mime: "text/javascript; charset=utf-8" },
    ],
  ]));
  try {
    await assert.rejects(
      verifyPagesDeployment({
        attempts: 1,
        baseUrl: url,
        executionMarkerPath: workerTamperMarker,
        expectedCommit,
        expectedHashes,
        label: "Tampered worker fixture",
      }),
      /worker\.[0-9a-f]{64}\.js content-address mismatch/,
    );
  } finally {
    serveFiles();
  }
  assert.equal(
    await stat(workerTamperMarker).catch(() => null),
    null,
    "hash-mismatched worker reached the execution child",
  );

  const tamperedGlue = Buffer.from(originalGlue);
  tamperedGlue[0] ^= 1;
  const tamperMarker = path.join(fixtureDir, "tampered-glue.executed");
  serveGlue(tamperedGlue);
  await assert.rejects(
    verifyPagesDeployment({
      attempts: 1,
      baseUrl: url,
      executionMarkerPath: tamperMarker,
      expectedCommit,
      expectedHashes,
      label: "Tampered glue fixture",
    }),
    /quickjs_oxide_web\.[0-9a-f]{64}\.js content-address mismatch/,
  );
  assert.equal(
    await stat(tamperMarker).catch(() => null),
    null,
    "hash-mismatched glue reached the execution child",
  );

  const propagationMarker = path.join(fixtureDir, "propagating-glue.executed");
  serveGlue(tamperedGlue, null, 1);
  await verifyPagesDeployment({
    attempts: 2,
    baseUrl: url,
    executionMarkerPath: propagationMarker,
    expectedCommit,
    expectedHashes,
    label: "Propagating glue fixture",
    retryBaseMs: 1,
    retryMaxMs: 1,
  });
  assert.equal(
    (await stat(propagationMarker)).isFile(),
    true,
    "hash-mismatched glue was not retried before authenticated execution",
  );
  await rm(propagationMarker, { force: true });

  const mimeMarker = path.join(fixtureDir, "wrong-mime.executed");
  serveGlue(originalGlue, "text/plain; charset=utf-8");
  await assert.rejects(
    verifyPagesDeployment({
      attempts: 1,
      baseUrl: url,
      executionMarkerPath: mimeMarker,
      expectedCommit,
      expectedHashes,
      label: "Wrong MIME fixture",
    }),
    /quickjs_oxide_web\.[0-9a-f]{64}\.js returned text\/plain/,
  );
  assert.equal(
    await stat(mimeMarker).catch(() => null),
    null,
    "wrong-MIME glue reached the execution child",
  );

  serveGlue(null);
  const timeoutMarker = path.join(fixtureDir, "request-timeout.executed");
  delayVerificationPath(contentAddressedWasmPath, 250);
  await assert.rejects(
    verifyPagesDeployment({
      attempts: 1,
      baseUrl: url,
      executionMarkerPath: timeoutMarker,
      expectedCommit,
      expectedHashes,
      label: "Request timeout fixture",
      requestTimeoutMs: 100,
    }),
    (error) => {
      assert.match(error.message, /attempt 1\/1/);
      assert.match(
        error.message,
        /quickjs_oxide_web_bg\.[0-9a-f]{64}\.wasm fetch failed.*AbortError/s,
      );
      return true;
    },
  );
  assert.equal(
    await stat(timeoutMarker).catch(() => null),
    null,
    "request-timeout assets reached the execution child",
  );
  delayVerificationPath(null);
  await new Promise((resolve) => {
    setTimeout(resolve, 300);
  });

  const commitMarker = path.join(fixtureDir, "wrong-commit.executed");
  await assert.rejects(
    verifyPagesDeployment({
      attempts: 2,
      baseUrl: url,
      executionMarkerPath: commitMarker,
      expectedCommit: "definitely-wrong-commit",
      expectedHashes,
      label: "Wrong commit fixture",
      retryBaseMs: 1,
      retryMaxMs: 1,
    }),
    (error) => {
      assert.match(error.message, /attempt 1\/2/);
      assert.doesNotMatch(error.message, /attempt 2\/2/);
      assert.match(
        error.message,
        /does not contain build label definitely-wrong-commit/,
      );
      return true;
    },
  );
  assert.equal(
    await stat(commitMarker).catch(() => null),
    null,
    "wrong-commit WASM reached the execution child",
  );

  if (process.platform !== "win32") {
    const returningPidMarker = path.join(
      fixtureDir,
      "returning-grandchild.pid",
    );
    grandchildPidMarkers.push(returningPidMarker);
    const returningGlue = processGroupFixtureGlue({
      expectedCommit,
      grandchildScriptPath,
      hang: false,
      pidMarkerPath: returningPidMarker,
    });
    const returningGraph = contentAddressedExecutionFixture({
      appSource: originalApp,
      expectedHashes,
      glue: returningGlue,
      indexSource: originalIndex,
      workerSource: originalWorker,
    });
    serveFiles(returningGraph.files);
    try {
      await verifyPagesDeployment({
        attempts: 1,
        baseUrl: url,
        executionTempRoot: fixtureDir,
        expectedCommit,
        expectedHashes: returningGraph.expectedHashes,
        label: "Returning process-group fixture",
      });
    } finally {
      serveFiles();
    }
    await assertProcessExited(
      await readFixturePid(returningPidMarker),
      "normally returning authenticated grandchild",
    );
    await rm(returningPidMarker, { force: true });
  }

  const hangingPidMarker = path.join(fixtureDir, "hanging-grandchild.pid");
  grandchildPidMarkers.push(hangingPidMarker);
  const hangingGlue = process.platform === "win32"
    ? Buffer.from(
        "let wasm_bindgen = (function(exports) {\n" +
          "    while (true) {}\n" +
          "    return exports;\n" +
          "})({ __proto__: null });\n",
        "utf8",
      )
    : processGroupFixtureGlue({
        expectedCommit,
        grandchildScriptPath,
        hang: true,
        pidMarkerPath: hangingPidMarker,
      });
  const hangingMarker = path.join(fixtureDir, "hanging-glue.executed");
  const hangingGraph = contentAddressedExecutionFixture({
    appSource: originalApp,
    expectedHashes,
    glue: hangingGlue,
    indexSource: originalIndex,
    workerSource: originalWorker,
  });
  serveFiles(hangingGraph.files);
  const hangingTimeoutMs = process.platform === "win32" ? 200 : 1_500;
  try {
    await assert.rejects(
      verifyPagesDeployment({
        attempts: 1,
        baseUrl: url,
        childTimeoutMs: hangingTimeoutMs,
        executionMarkerPath: hangingMarker,
        executionTempRoot: fixtureDir,
        expectedCommit,
        expectedHashes: hangingGraph.expectedHashes,
        label: "Hanging authenticated glue fixture",
      }),
      new RegExp(
        `authenticated WASM child exceeded ${hangingTimeoutMs} ms and was killed`,
      ),
    );
  } finally {
    serveFiles();
  }
  assert.equal(
    (await stat(hangingMarker)).isFile(),
    true,
    "authenticated hanging glue never reached the bounded child",
  );
  if (process.platform !== "win32") {
    await assertProcessExited(
      await readFixturePid(hangingPidMarker),
      "timed-out authenticated grandchild",
    );
    await rm(hangingPidMarker, { force: true });
  }
  assert.deepEqual(
    (await readdir(fixtureDir)).filter((name) =>
      name.startsWith("quickjs-oxide-live-pages."),
    ),
    [],
    "killed authenticated child left an execution directory behind",
  );
  console.log(
    "Live verifier security fixtures passed: stale hashes were retried without " +
      "execution; app/worker/glue hash and MIME/commit/timeout failures never " +
      "executed untrusted code; returning and timed-out authenticated process " +
      "groups were killed and cleaned.",
  );

  serveGlue(null);
  rejectVerificationAttempt(1);
  const sentinelEnvironmentName = "QUICKJS_OXIDE_TEST_PARENT_SECRET";
  const previousSentinelValue = process.env[sentinelEnvironmentName];
  process.env[sentinelEnvironmentName] = "must-not-reach-child";
  try {
    await verifyPagesDeployment({
      attempts: 2,
      baseUrl: url,
      expectedCommit,
      expectedHashes,
      expectedMissingChildEnvName: sentinelEnvironmentName,
      label: "Local Pages fixture",
      retryBaseMs: 1,
      retryMaxMs: 1,
    });
  } finally {
    if (previousSentinelValue === undefined) {
      delete process.env[sentinelEnvironmentName];
    } else {
      process.env[sentinelEnvironmentName] = previousSentinelValue;
    }
  }
  console.log(
    "Live verifier isolation fixture passed: the child receipt proved the " +
      "parent sentinel environment variable was absent.",
  );
  assert.deepEqual(
    serverErrors.map(String),
    [],
    "local live verifier artifact server failed",
  );
} finally {
  try {
    await stopServer(server);
  } finally {
    try {
      for (const markerPath of grandchildPidMarkers) {
        const value = await readFile(markerPath, "utf8").catch(() => null);
        const pid = Number(value?.trim());
        if (Number.isSafeInteger(pid) && pid > 0) {
          try {
            process.kill(pid, "SIGKILL");
          } catch (error) {
            if (error?.code !== "ESRCH") {
              throw error;
            }
          }
        }
      }
    } finally {
      await rm(fixtureDir, { force: true, recursive: true });
    }
  }
}
