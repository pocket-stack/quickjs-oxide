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
const requiredBrowserResources = new Set([
  `${pagesBasePath}app.js`,
  `${pagesBasePath}examples.js`,
  `${pagesBasePath}style.css`,
  `${pagesBasePath}worker.js`,
  `${pagesBasePath}pkg/quickjs_oxide_web.js`,
  `${pagesBasePath}pkg/quickjs_oxide_web_bg.wasm`,
]);

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

      const metadata = await stat(filePath).catch(() => null);
      if (!metadata?.isFile()) {
        response.writeHead(404).end("Not found\n");
        return;
      }

      const isGlue = filePath === path.join(
        pagesDir,
        "pkg/quickjs_oxide_web.js",
      );
      const body = request.method === "HEAD"
        ? null
        : isGlue && customGlueBody
        ? customGlueBody
        : await readFile(filePath);
      response.writeHead(200, {
        "Cache-Control": "no-store",
        "Content-Length": body?.byteLength ?? metadata.size,
        "Content-Type": isGlue && customGlueMime
          ? customGlueMime
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
    serveGlue(body, mime = null) {
      customGlueBody = body;
      customGlueMime = mime;
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
      /66,552 passes\s*\/\s*66,604 runnable\s*\/\s*102,037 total[\s\S]*pre-parity/,
    );

    assert.ok(
      workerUrls.some(
        (workerUrl) => new URL(workerUrl).pathname === `${pagesBasePath}worker.js`,
      ),
      "the page did not create the expected engine worker",
    );
    for (const resource of requiredBrowserResources) {
      assert.ok(loadedResources.has(resource), `browser did not load ${resource}`);
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
  serveGlue,
  url,
} = await startArtifactServer();
const fixtureDir = await mkdtemp(
  path.join(tmpdir(), "quickjs-oxide-live-pages-test."),
);
const expectedCommit =
  process.env.QUICKJS_OXIDE_COMMIT || process.env.GITHUB_SHA || "local";
const expectedHashes = await hashPagesArtifact(pagesDir);
const originalGlue = await readFile(
  path.join(pagesDir, "pkg/quickjs_oxide_web.js"),
);
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
    /quickjs_oxide_web\.js SHA-256 mismatch/,
  );
  assert.equal(
    await stat(tamperMarker).catch(() => null),
    null,
    "hash-mismatched glue reached the execution child",
  );

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
    /quickjs_oxide_web\.js returned text\/plain/,
  );
  assert.equal(
    await stat(mimeMarker).catch(() => null),
    null,
    "wrong-MIME glue reached the execution child",
  );

  serveGlue(null);
  const timeoutMarker = path.join(fixtureDir, "request-timeout.executed");
  delayVerificationPath(
    `${pagesBasePath}pkg/quickjs_oxide_web_bg.wasm`,
    250,
  );
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
        /quickjs_oxide_web_bg\.wasm fetch failed.*AbortError/s,
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
      attempts: 1,
      baseUrl: url,
      executionMarkerPath: commitMarker,
      expectedCommit: "definitely-wrong-commit",
      expectedHashes,
      label: "Wrong commit fixture",
    }),
    /does not contain build label definitely-wrong-commit/,
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
    serveGlue(returningGlue);
    await verifyPagesDeployment({
      attempts: 1,
      baseUrl: url,
      executionTempRoot: fixtureDir,
      expectedCommit,
      expectedHashes: {
        ...expectedHashes,
        glueSha256: sha256Bytes(returningGlue),
      },
      label: "Returning process-group fixture",
    });
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
  serveGlue(hangingGlue);
  const hangingTimeoutMs = process.platform === "win32" ? 200 : 1_500;
  await assert.rejects(
    verifyPagesDeployment({
      attempts: 1,
      baseUrl: url,
      childTimeoutMs: hangingTimeoutMs,
      executionMarkerPath: hangingMarker,
      executionTempRoot: fixtureDir,
      expectedCommit,
      expectedHashes: {
        ...expectedHashes,
        glueSha256: sha256Bytes(hangingGlue),
      },
      label: "Hanging authenticated glue fixture",
    }),
    new RegExp(
      `authenticated WASM child exceeded ${hangingTimeoutMs} ms and was killed`,
    ),
  );
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
    "Live verifier security fixtures passed: hash/MIME/commit/timeout failures " +
      "never executed untrusted glue; returning and timed-out authenticated " +
      "process groups were killed and cleaned.",
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
