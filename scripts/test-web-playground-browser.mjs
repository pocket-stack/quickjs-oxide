import assert from "node:assert/strict";
import { readFile, stat } from "node:fs/promises";
import { createServer } from "node:http";
import path from "node:path";
import process from "node:process";
import { chromium } from "playwright";

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
  const server = createServer(async (request, response) => {
    try {
      const filePath = artifactPath(request.url);
      if (filePath === null) {
        response.writeHead(403).end("Forbidden\n");
        return;
      }

      const metadata = await stat(filePath).catch(() => null);
      if (!metadata?.isFile()) {
        response.writeHead(404).end("Not found\n");
        return;
      }

      const body = request.method === "HEAD" ? null : await readFile(filePath);
      response.writeHead(200, {
        "Cache-Control": "no-store",
        "Content-Length": metadata.size,
        "Content-Type": contentType(filePath),
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
const { server, serverErrors, url } = await startArtifactServer();
try {
  await runAcceptance(url, serverErrors);
} finally {
  await stopServer(server);
}
