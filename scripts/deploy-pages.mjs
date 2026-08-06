#!/usr/bin/env node

import process from "node:process";
import { setTimeout as wait } from "node:timers/promises";
import { pathToFileURL } from "node:url";

const DEFAULT_POLL_INTERVAL_MS = 5_000;
const DEFAULT_POLL_TIMEOUT_MS = 1_500_000;
const DEFAULT_REQUEST_TIMEOUT_MS = 30_000;
const MAX_POLL_TIMEOUT_MS = 1_800_000;
const TERMINAL_FAILURES = new Map([
  ["deployment_failed", "deployment failed"],
  ["deployment_content_failed", "deployment content failed validation"],
  ["deployment_cancelled", "deployment was cancelled"],
  ["deployment_lost", "deployment stopped reporting status"],
]);

class HttpResponseError extends Error {
  constructor(label, status, detail = "") {
    super(`${label} returned HTTP ${status}${detail ? `: ${detail}` : ""}`);
    this.name = "HttpResponseError";
    this.status = status;
  }
}

class RequestFailureError extends Error {
  constructor(label) {
    super(`${label} request failed`);
    this.name = "RequestFailureError";
  }
}

function requiredString(environment, name) {
  const value = environment[name];
  if (typeof value !== "string" || value.length === 0) {
    throw new TypeError(`${name} must be a non-empty string`);
  }
  return value;
}

function optionalInteger(environment, name, fallback, minimum, maximum) {
  const value = environment[name];
  if (value === undefined || value === "") {
    return fallback;
  }
  const number = Number(value);
  if (!Number.isSafeInteger(number) || number < minimum || number > maximum) {
    throw new TypeError(
      `${name} must be an integer between ${minimum} and ${maximum}`,
    );
  }
  return number;
}

function normalizedHttpUrl(value, name, { allowQuery = false } = {}) {
  let url;
  try {
    url = new URL(value);
  } catch {
    throw new TypeError(`${name} must be a valid URL`);
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw new TypeError(`${name} must use http or https`);
  }
  if (url.username || url.password || url.hash || (!allowQuery && url.search)) {
    throw new TypeError(`${name} contains unsupported URL components`);
  }
  return url;
}

function repositoryParts(value) {
  const match = value.match(
    /^([A-Za-z0-9](?:[A-Za-z0-9_.-]{0,99}))\/([A-Za-z0-9](?:[A-Za-z0-9_.-]{0,99}))$/u,
  );
  if (!match) {
    throw new TypeError("GITHUB_REPOSITORY must be an owner/name pair");
  }
  return { owner: match[1], repository: match[2] };
}

function redact(value, secrets) {
  let safe = String(value).replace(/[\r\n]+/gu, " ");
  const orderedSecrets = secrets
    .filter(Boolean)
    .sort((left, right) => right.length - left.length);
  for (const secret of orderedSecrets) {
    safe = safe.split(secret).join("[redacted]");
  }
  return safe.slice(0, 512);
}

function responseDetail(body, secrets) {
  try {
    const parsed = JSON.parse(body);
    return typeof parsed?.message === "string"
      ? redact(parsed.message, secrets)
      : "";
  } catch {
    return "";
  }
}

async function requestJson({
  body = undefined,
  fetchImpl,
  headers,
  label,
  method,
  secrets,
  timeoutMs,
  url,
}) {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), timeoutMs);
  let response;
  let responseBody;
  try {
    response = await fetchImpl(url, {
      body,
      headers,
      method,
      redirect: "error",
      signal: controller.signal,
    });
    responseBody = await response.text();
  } catch {
    throw new RequestFailureError(label);
  } finally {
    clearTimeout(timeout);
  }

  if (!response.ok) {
    throw new HttpResponseError(
      label,
      response.status,
      responseDetail(responseBody, secrets),
    );
  }
  const mime = (response.headers.get("content-type") || "")
    .split(";", 1)[0]
    .trim()
    .toLowerCase();
  if (mime !== "application/json" && !mime.endsWith("+json")) {
    throw new TypeError(`${label} returned ${mime || "no Content-Type"}`);
  }
  try {
    const parsed = JSON.parse(responseBody);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      throw new TypeError(`${label} returned a non-object JSON value`);
    }
    return parsed;
  } catch (error) {
    if (error instanceof TypeError) {
      throw error;
    }
    throw new TypeError(`${label} returned invalid JSON`);
  }
}

function configuration(environment) {
  const githubToken = requiredString(environment, "GITHUB_TOKEN");
  const oidcRequestToken = requiredString(
    environment,
    "ACTIONS_ID_TOKEN_REQUEST_TOKEN",
  );
  const oidcRequestUrl = normalizedHttpUrl(
    requiredString(environment, "ACTIONS_ID_TOKEN_REQUEST_URL"),
    "ACTIONS_ID_TOKEN_REQUEST_URL",
    { allowQuery: true },
  );
  const apiUrl = normalizedHttpUrl(
    environment.GITHUB_API_URL || "https://api.github.com",
    "GITHUB_API_URL",
  );
  const { owner, repository } = repositoryParts(
    requiredString(environment, "GITHUB_REPOSITORY"),
  );
  const buildVersion = requiredString(environment, "GITHUB_SHA");
  if (!/^[0-9a-f]{40}$/u.test(buildVersion)) {
    throw new TypeError("GITHUB_SHA must be an exact lowercase commit SHA");
  }
  const artifactId = Number(
    requiredString(environment, "QUICKJS_OXIDE_PAGES_ARTIFACT_ID"),
  );
  if (!Number.isSafeInteger(artifactId) || artifactId <= 0) {
    throw new TypeError(
      "QUICKJS_OXIDE_PAGES_ARTIFACT_ID must be a positive safe integer",
    );
  }

  return {
    apiUrl,
    artifactId,
    buildVersion,
    githubToken,
    oidcRequestToken,
    oidcRequestUrl,
    owner,
    pollIntervalMs: optionalInteger(
      environment,
      "QUICKJS_OXIDE_PAGES_POLL_INTERVAL_MS",
      DEFAULT_POLL_INTERVAL_MS,
      1,
      60_000,
    ),
    pollTimeoutMs: optionalInteger(
      environment,
      "QUICKJS_OXIDE_PAGES_POLL_TIMEOUT_MS",
      DEFAULT_POLL_TIMEOUT_MS,
      1,
      MAX_POLL_TIMEOUT_MS,
    ),
    repository,
    requestTimeoutMs: optionalInteger(
      environment,
      "QUICKJS_OXIDE_PAGES_REQUEST_TIMEOUT_MS",
      DEFAULT_REQUEST_TIMEOUT_MS,
      100,
      120_000,
    ),
  };
}

function apiEndpoint(config, suffix = "") {
  const base = config.apiUrl.href.replace(/\/$/u, "");
  const repository =
    `${encodeURIComponent(config.owner)}/` +
    encodeURIComponent(config.repository);
  return `${base}/repos/${repository}/pages/deployments${suffix}`;
}

async function requestOidcToken(config, fetchImpl) {
  const response = await requestJson({
    fetchImpl,
    headers: {
      accept: "application/json",
      authorization: `Bearer ${config.oidcRequestToken}`,
      "user-agent": "quickjs-oxide-pages-deployer",
    },
    label: "Actions OIDC endpoint",
    method: "GET",
    secrets: [config.oidcRequestToken, config.githubToken],
    timeoutMs: config.requestTimeoutMs,
    url: config.oidcRequestUrl,
  });
  if (typeof response.value !== "string" || response.value.length === 0) {
    throw new TypeError("Actions OIDC endpoint returned no ID token");
  }
  return response.value;
}

function githubHeaders(config) {
  const authorizationScheme = config.githubToken.split(".").length === 3
    ? "bearer"
    : "token";
  return {
    accept: "application/vnd.github+json",
    authorization: `${authorizationScheme} ${config.githubToken}`,
    "content-type": "application/json",
    "user-agent": "quickjs-oxide-pages-deployer",
    "x-github-api-version": "2022-11-28",
  };
}

function deploymentIdentifier(response, config) {
  if (
    (
      typeof response.id === "number" &&
      Number.isSafeInteger(response.id) &&
      response.id > 0
    ) ||
    (
      typeof response.id === "string" &&
      /^[0-9A-Za-z][0-9A-Za-z._-]{0,127}$/u.test(response.id)
    )
  ) {
    return String(response.id);
  }
  if (typeof response.status_url !== "string") {
    throw new TypeError("Pages create response has no deployment identifier");
  }
  const statusUrl = normalizedHttpUrl(response.status_url, "Pages status_url");
  const prefix = `${apiEndpoint(config)}/`;
  if (!statusUrl.href.startsWith(prefix)) {
    throw new TypeError(
      "Pages status_url is outside the repository deployment API",
    );
  }
  let identifierPath = statusUrl.href.slice(prefix.length);
  if (identifierPath.endsWith("/status")) {
    identifierPath = identifierPath.slice(0, -"/status".length);
  }
  const identifier = decodeURIComponent(identifierPath);
  if (!/^[0-9A-Za-z][0-9A-Za-z._-]{0,127}$/u.test(identifier)) {
    throw new TypeError("Pages status_url has an invalid deployment identifier");
  }
  return identifier;
}

async function createDeployment(config, oidcToken, fetchImpl) {
  const payload = JSON.stringify({
    artifact_id: config.artifactId,
    pages_build_version: config.buildVersion,
    oidc_token: oidcToken,
  });
  const response = await requestJson({
    body: payload,
    fetchImpl,
    headers: githubHeaders(config),
    label: "Pages create deployment",
    method: "POST",
    secrets: [config.githubToken, config.oidcRequestToken, oidcToken],
    timeoutMs: config.requestTimeoutMs,
    url: apiEndpoint(config),
  });
  return {
    deploymentId: deploymentIdentifier(response, config),
  };
}

function deploymentStatus(response) {
  if (
    typeof response.status !== "string" ||
    !/^[a-z][a-z0-9_]{0,63}$/u.test(response.status)
  ) {
    throw new TypeError("Pages status response has an invalid status");
  }
  return response.status;
}

function retryableStatusCode(status) {
  return status === 404 || status === 408 || status === 429 || status >= 500;
}

function defaultLogger() {
  return {
    info(message) {
      console.log(message);
    },
    warning(message) {
      const escaped = String(message)
        .replace(/%/gu, "%25")
        .replace(/\r/gu, "%0D")
        .replace(/\n/gu, "%0A");
      console.warn(`::warning::${escaped}`);
    },
  };
}

export async function deployPages({
  environment = process.env,
  fetchImpl = globalThis.fetch,
  logger = defaultLogger(),
  now = Date.now,
  sleep = wait,
} = {}) {
  if (typeof fetchImpl !== "function") {
    throw new TypeError("fetch implementation is required");
  }
  const config = configuration(environment);
  const oidcToken = await requestOidcToken(config, fetchImpl);
  const { deploymentId } = await createDeployment(
    config,
    oidcToken,
    fetchImpl,
  );
  logger.info(
    `Created Pages deployment ${deploymentId} for ` +
      `${config.buildVersion.slice(0, 7)} from artifact ${config.artifactId}.`,
  );

  const startedAt = now();
  let lastStatus = "created";
  while (now() - startedAt <= config.pollTimeoutMs) {
    try {
      const response = await requestJson({
        fetchImpl,
        headers: githubHeaders(config),
        label: "Pages deployment status",
        method: "GET",
        secrets: [config.githubToken, config.oidcRequestToken, oidcToken],
        timeoutMs: config.requestTimeoutMs,
        url: apiEndpoint(config, `/${encodeURIComponent(deploymentId)}`),
      });
      lastStatus = deploymentStatus(response);
      if (lastStatus === "succeed") {
        logger.info(`Pages deployment ${deploymentId} reported success.`);
        return { deploymentId, outcome: "succeed", status: lastStatus };
      }
      const failure = TERMINAL_FAILURES.get(lastStatus);
      if (failure) {
        throw new Error(`Pages deployment ${deploymentId} ${failure}.`);
      }
      logger.info(`Pages deployment ${deploymentId} status: ${lastStatus}.`);
    } catch (error) {
      if (
        error instanceof RequestFailureError ||
        (
          error instanceof HttpResponseError &&
          retryableStatusCode(error.status)
        )
      ) {
        const detail = error instanceof HttpResponseError
          ? `HTTP ${error.status}`
          : "network error";
        logger.warning(
          `Pages deployment ${deploymentId} status check is temporarily unavailable ` +
            `(${detail}); polling will continue.`,
        );
      } else {
        throw error;
      }
    }

    const elapsed = now() - startedAt;
    if (elapsed >= config.pollTimeoutMs) {
      break;
    }
    await sleep(Math.min(config.pollIntervalMs, config.pollTimeoutMs - elapsed));
  }

  logger.warning(
    `Pages deployment ${deploymentId} remains ${lastStatus} after ` +
      `${config.pollTimeoutMs} ms; it was left active and the independent live ` +
      "verifier will decide this workflow's outcome.",
  );
  return { deploymentId, outcome: "deferred", status: lastStatus };
}

async function main() {
  await deployPages();
}

export function deploymentFailureMessage(error) {
  return `Pages deployment failed: ${
    error instanceof Error ? error.message : String(error)
  }`;
}

const invokedPath = process.argv[1] ? pathToFileURL(process.argv[1]).href : null;
if (invokedPath === import.meta.url) {
  main().catch((error) => {
    console.error(deploymentFailureMessage(error));
    process.exitCode = 1;
  });
}
