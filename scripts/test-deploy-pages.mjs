#!/usr/bin/env node

import assert from "node:assert/strict";
import { createServer } from "node:http";
import {
  deployPages,
  deploymentFailureMessage,
} from "./deploy-pages.mjs";

const BUILD_SHA = "0123456789abcdef0123456789abcdef01234567";
const ARTIFACT_ID = 424242;
const GITHUB_TOKEN = "github-token-must-stay-secret";
const OIDC_REQUEST_TOKEN = "oidc-request-token-must-stay-secret";
const OIDC_TOKEN = `oidc-jwt-${"x".repeat(700)}-must-stay-secret`;

function json(response, status, value) {
  const body = Buffer.from(JSON.stringify(value));
  response.writeHead(status, {
    "content-length": body.byteLength,
    "content-type": "application/json; charset=utf-8",
  });
  response.end(body);
}

async function requestBody(request) {
  const chunks = [];
  for await (const chunk of request) {
    chunks.push(chunk);
  }
  return Buffer.concat(chunks).toString("utf8");
}

async function startMockPages({ deploymentId = "77", statuses }) {
  const requests = [];
  let statusIndex = 0;
  let baseUrl = null;
  const server = createServer(async (request, response) => {
    const url = new URL(request.url || "/", baseUrl);
    const body = await requestBody(request);
    requests.push({
      authorization: request.headers.authorization || null,
      body,
      method: request.method,
      pathname: url.pathname,
    });

    if (request.method === "GET" && url.pathname === "/oidc") {
      json(response, 200, { value: OIDC_TOKEN });
      return;
    }
    if (
      request.method === "POST" &&
      url.pathname === "/repos/pocket-stack/quickjs-oxide/pages/deployments"
    ) {
      json(response, 200, {
        page_url: "https://pocket-stack.github.io/quickjs-oxide/",
        status_url:
          `${baseUrl}/repos/pocket-stack/quickjs-oxide/pages/deployments/` +
          `${deploymentId}/status`,
      });
      return;
    }
    if (
      request.method === "GET" &&
      url.pathname ===
        `/repos/pocket-stack/quickjs-oxide/pages/deployments/${deploymentId}`
    ) {
      const status = statuses[Math.min(statusIndex, statuses.length - 1)];
      statusIndex += 1;
      if (status && typeof status === "object") {
        json(response, status.httpStatus, {
          message: status.message || "mock status request failed",
        });
        return;
      }
      json(response, 200, { status });
      return;
    }
    json(response, 404, { message: "unexpected mock request" });
  });
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address();
  assert.ok(address && typeof address === "object");
  baseUrl = `http://127.0.0.1:${address.port}`;
  return { baseUrl, requests, server };
}

function stopServer(server) {
  return new Promise((resolve, reject) => {
    server.close((error) => (error ? reject(error) : resolve()));
  });
}

function scenarioEnvironment(baseUrl) {
  return {
    ACTIONS_ID_TOKEN_REQUEST_TOKEN: OIDC_REQUEST_TOKEN,
    ACTIONS_ID_TOKEN_REQUEST_URL: `${baseUrl}/oidc?api-version=2.0`,
    GITHUB_API_URL: baseUrl,
    GITHUB_REPOSITORY: "pocket-stack/quickjs-oxide",
    GITHUB_SHA: BUILD_SHA,
    GITHUB_TOKEN,
    QUICKJS_OXIDE_PAGES_ARTIFACT_ID: String(ARTIFACT_ID),
    QUICKJS_OXIDE_PAGES_POLL_INTERVAL_MS: "5",
    QUICKJS_OXIDE_PAGES_POLL_TIMEOUT_MS: "20",
    QUICKJS_OXIDE_PAGES_REQUEST_TIMEOUT_MS: "1000",
  };
}

const allLogs = [];

function fixtureLogger() {
  const entries = [];
  return {
    entries,
    info(message) {
      entries.push(`info:${message}`);
      allLogs.push(`info:${message}`);
    },
    warning(message) {
      entries.push(`warning:${message}`);
      allLogs.push(`warning:${message}`);
    },
  };
}

async function runScenario(statuses) {
  const mock = await startMockPages({ statuses });
  const logger = fixtureLogger();
  let clock = 0;
  try {
    const operation = deployPages({
      environment: scenarioEnvironment(mock.baseUrl),
      logger,
      now: () => clock,
      sleep: async (milliseconds) => {
        clock += milliseconds;
      },
    });
    return {
      logger,
      mock,
      operation,
      stop: async () => stopServer(mock.server),
    };
  } catch (error) {
    await stopServer(mock.server);
    throw error;
  }
}

function assertCreateContract(requests) {
  assert.equal(requests[0].method, "GET");
  assert.equal(requests[0].pathname, "/oidc");
  assert.equal(requests[0].authorization, `Bearer ${OIDC_REQUEST_TOKEN}`);
  assert.equal(requests[1].method, "POST");
  assert.equal(
    requests[1].pathname,
    "/repos/pocket-stack/quickjs-oxide/pages/deployments",
  );
  assert.equal(requests[1].authorization, `token ${GITHUB_TOKEN}`);
  assert.deepEqual(JSON.parse(requests[1].body), {
    artifact_id: ARTIFACT_ID,
    oidc_token: OIDC_TOKEN,
    pages_build_version: BUILD_SHA,
  });
  for (const request of requests.slice(2)) {
    assert.equal(request.method, "GET");
    assert.equal(request.authorization, `token ${GITHUB_TOKEN}`);
    assert.match(
      request.pathname,
      /^\/repos\/pocket-stack\/quickjs-oxide\/pages\/deployments\/[0-9A-Za-z._-]+$/u,
    );
  }
}

function assertNoCancel(requests) {
  assert.equal(
    requests.some((request) => request.pathname.endsWith("/cancel")),
    false,
    "deployer sent a Pages cancellation request",
  );
}

{
  const scenario = await runScenario(["building", "succeed"]);
  try {
    const result = await scenario.operation;
    assert.equal(result.outcome, "succeed");
    assert.equal(result.status, "succeed");
    assertCreateContract(scenario.mock.requests);
    assertNoCancel(scenario.mock.requests);
  } finally {
    await scenario.stop();
  }
}

for (const status of [404, 408, 429, 500]) {
  const scenario = await runScenario([{ httpStatus: status }, "succeed"]);
  try {
    const result = await scenario.operation;
    assert.equal(result.outcome, "succeed");
    assert.match(
      scenario.logger.entries.join("\n"),
      new RegExp(`HTTP ${status}`),
    );
    assertCreateContract(scenario.mock.requests);
    assertNoCancel(scenario.mock.requests);
  } finally {
    await scenario.stop();
  }
}

{
  const scenario = await runScenario(["deployment_content_failed"]);
  try {
    await assert.rejects(
      scenario.operation,
      /deployment content failed validation/,
    );
    assertCreateContract(scenario.mock.requests);
    assertNoCancel(scenario.mock.requests);
  } finally {
    await scenario.stop();
  }
}

{
  const scenario = await runScenario(["building"]);
  try {
    const result = await scenario.operation;
    assert.equal(result.outcome, "deferred");
    assert.equal(result.status, "building");
    assert.match(
      scenario.logger.entries.join("\n"),
      /left active.*independent live verifier/s,
    );
    assertCreateContract(scenario.mock.requests);
    assertNoCancel(scenario.mock.requests);
  } finally {
    await scenario.stop();
  }
}

for (const status of [400, 401, 403]) {
  const scenario = await runScenario([
    {
      httpStatus: status,
      message: `${GITHUB_TOKEN} ${OIDC_REQUEST_TOKEN} ${OIDC_TOKEN}`,
    },
  ]);
  try {
    await assert.rejects(scenario.operation, (error) => {
      const message = deploymentFailureMessage(error);
      assert.match(message, new RegExp(`HTTP ${status}`));
      assert.match(message, /\[redacted\]/);
      for (const secret of [GITHUB_TOKEN, OIDC_REQUEST_TOKEN, OIDC_TOKEN]) {
        assert.equal(message.includes(secret), false);
        assert.equal(message.includes(secret.slice(0, 24)), false);
      }
      return true;
    });
    assertCreateContract(scenario.mock.requests);
    assertNoCancel(scenario.mock.requests);
  } finally {
    await scenario.stop();
  }
}

{
  const mock = await startMockPages({ statuses: ["succeed"] });
  try {
    const environment = scenarioEnvironment(mock.baseUrl);
    environment.QUICKJS_OXIDE_PAGES_ARTIFACT_ID = "not-an-artifact";
    await assert.rejects(
      deployPages({ environment }),
      /PAGES_ARTIFACT_ID must be a positive safe integer/,
    );
    assert.deepEqual(mock.requests, []);
  } finally {
    await stopServer(mock.server);
  }
}

for (const secret of [GITHUB_TOKEN, OIDC_REQUEST_TOKEN, OIDC_TOKEN]) {
  assert.equal(
    allLogs.join("\n").includes(secret),
    false,
    "deployment logs exposed a credential",
  );
  assert.equal(
    allLogs.join("\n").includes(secret.slice(0, 24)),
    false,
    "deployment logs exposed a credential prefix",
  );
}

console.log(
  "Pages deploy client fixtures passed: create payload, success, retryable " +
    "HTTP errors, fail-fast 4xx redaction, terminal failure, timeout deferral, " +
    "environment validation, and no cancellation.",
);
