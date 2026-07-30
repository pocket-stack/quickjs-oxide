import { DEFAULT_EXAMPLE_ID, EXAMPLES } from "./examples.js";

const EXECUTION_TIMEOUT_MS = 2_000;
const LOAD_TIMEOUT_MS = 15_000;

const elements = {
  editor: document.querySelector("#source-editor"),
  exampleSelect: document.querySelector("#example-select"),
  resetButton: document.querySelector("#reset-button"),
  runButton: document.querySelector("#run-button"),
  runButtonLabel: document.querySelector("#run-button-label"),
  engineStatus: document.querySelector("#engine-status"),
  statusLight: document.querySelector("#status-light"),
  sourceStat: document.querySelector("#source-stat"),
  offlineNotice: document.querySelector("#offline-notice"),
  resultState: document.querySelector("#result-state"),
  resultEmpty: document.querySelector("#result-empty"),
  resultContent: document.querySelector("#result-content"),
  resultLabel: document.querySelector("#result-label"),
  resultValue: document.querySelector("#result-value"),
  resultType: document.querySelector("#result-type"),
  resultDuration: document.querySelector("#result-duration"),
  resultDetails: document.querySelector("#result-details"),
  resultRaw: document.querySelector("#result-raw"),
};

let worker = null;
let workerGeneration = 0;
let workerReady = false;
let workerFailed = false;
let currentRequest = null;
let nextRequestId = 1;
let executionTimer = null;
let loadTimer = null;

function setEngineState(state, label) {
  elements.engineStatus.textContent = label;
  elements.statusLight.className = `status-light is-${state}`;

  const loading = state === "loading";
  const running = state === "running";
  const error = state === "error";

  elements.runButton.disabled = loading || running;
  elements.runButtonLabel.textContent = error
    ? "Retry load"
    : running
    ? "Running…"
    : loading
    ? "Loading…"
    : "Run";
}

function setResultState(state, label) {
  elements.resultState.className = `result-state is-${state}`;
  elements.resultState.textContent = label;
}

function showEmptyResult(
  message = "Choose an example or write a script, then run it.",
) {
  elements.resultEmpty.querySelector("p").textContent = message;
  elements.resultEmpty.hidden = false;
  elements.resultContent.hidden = true;
  elements.resultContent.classList.remove("is-error");
  elements.resultDetails.open = false;
  setResultState("idle", "Waiting");
}

function stableStringify(value) {
  if (value === undefined) {
    return "undefined";
  }

  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function displayValue(value, type) {
  if (value === undefined || type === "undefined") {
    return "undefined";
  }

  if (typeof value === "string") {
    return value;
  }

  return stableStringify(value);
}

function normalizeResult(rawResult) {
  if (rawResult && typeof rawResult === "object" && "ok" in rawResult) {
    return rawResult;
  }

  return {
    ok: true,
    type: rawResult === null ? "null" : typeof rawResult,
    value: rawResult,
  };
}

function renderResult(rawResult, durationMs) {
  const result = normalizeResult(rawResult);
  const failed = result.ok === false;
  const error = failed && result.error && typeof result.error === "object"
    ? result.error
    : null;
  const resultType = failed
    ? error && error.name ? error.name : result.kind || "Error"
    : result.type ||
      result.kind ||
      (result.value === null ? "null" : typeof result.value);
  const resultText = failed
    ? error && error.message
      ? error.message
      : result.text || result.message || "Evaluation failed."
    : result.display !== undefined
    ? String(result.display)
    : result.text !== undefined
    ? String(result.text)
    : displayValue(result.value, result.type);

  elements.resultEmpty.hidden = true;
  elements.resultContent.hidden = false;
  elements.resultContent.classList.toggle("is-error", failed);
  elements.resultLabel.textContent = failed
    ? "Evaluation error"
    : "Evaluation result";
  elements.resultValue.textContent = resultText;
  elements.resultType.textContent = resultType;
  elements.resultDuration.textContent = formatDuration(durationMs);
  elements.resultRaw.textContent = stableStringify(rawResult);
  elements.resultDetails.open = false;

  setResultState(failed ? "error" : "success", failed ? "Failed" : "Complete");
}

function renderInfrastructureError(name, message, detail, durationMs = null) {
  const raw = {
    ok: false,
    error: {
      name,
      message,
      detail,
    },
  };

  renderResult(raw, durationMs);
  elements.resultLabel.textContent = name === "TimeoutError"
    ? "Time limit reached"
    : "Engine error";
  elements.resultRaw.textContent = stableStringify(raw);
}

function formatDuration(value) {
  if (value === null || value === undefined || !Number.isFinite(value)) {
    return "—";
  }

  if (value < 1) {
    return "< 1 ms";
  }

  return `${value.toFixed(value < 10 ? 2 : 1)} ms`;
}

function clearTimers() {
  globalThis.clearTimeout(executionTimer);
  globalThis.clearTimeout(loadTimer);
  executionTimer = null;
  loadTimer = null;
}

function terminateWorker() {
  clearTimers();
  currentRequest = null;
  workerReady = false;

  if (worker) {
    worker.terminate();
    worker = null;
  }
}

function loadErrorMessage(error, online) {
  if (online === false || !navigator.onLine) {
    return "The engine could not load while this browser is offline. Reconnect, then retry.";
  }

  if (error && error.message) {
    return error.message;
  }

  return "The compiled quickjs-oxide WASM package could not be loaded.";
}

function bootWorker(
  { preserveResult = false, reason = "Loading engine…" } = {},
) {
  terminateWorker();
  workerFailed = false;
  workerGeneration += 1;
  const generation = workerGeneration;

  setEngineState("loading", reason);
  if (!preserveResult) {
    showEmptyResult();
  }

  try {
    worker = new Worker("./worker.js", {
      name: "quickjs-oxide-engine",
    });
  } catch (error) {
    handleLoadFailure(error, navigator.onLine);
    return;
  }

  worker.addEventListener("message", (event) => {
    if (generation !== workerGeneration) {
      return;
    }

    handleWorkerMessage(event.data);
  });

  worker.addEventListener("error", (event) => {
    if (generation !== workerGeneration) {
      return;
    }

    event.preventDefault();
    handleLoadFailure(
      {
        name: "WorkerError",
        message: event.message || "The engine worker stopped unexpectedly.",
      },
      navigator.onLine,
    );
  });

  loadTimer = globalThis.setTimeout(() => {
    if (generation !== workerGeneration || workerReady) {
      return;
    }

    handleLoadFailure(
      {
        name: "EngineLoadTimeout",
        message: "The WASM engine did not finish loading within 15 seconds.",
      },
      navigator.onLine,
    );
  }, LOAD_TIMEOUT_MS);
}

function handleLoadFailure(error, online) {
  terminateWorker();
  workerFailed = true;

  const message = loadErrorMessage(error, online);
  setEngineState("error", "Engine unavailable");
  renderInfrastructureError(
    error && error.name ? error.name : "EngineLoadError",
    message,
    "Expected ./pkg/quickjs_oxide_web.js and ./pkg/quickjs_oxide_web_bg.wasm.",
  );
}

function handleWorkerMessage(message) {
  if (!message || typeof message.type !== "string") {
    return;
  }

  if (message.type === "ready") {
    globalThis.clearTimeout(loadTimer);
    loadTimer = null;
    workerReady = true;
    workerFailed = false;
    setEngineState("ready", "Engine ready");
    return;
  }

  if (message.type === "load-error") {
    handleLoadFailure(message.error, message.online);
    return;
  }

  if (
    message.type !== "result" ||
    !currentRequest ||
    message.id !== currentRequest.id
  ) {
    return;
  }

  globalThis.clearTimeout(executionTimer);
  executionTimer = null;
  currentRequest = null;
  renderResult(message.result, message.durationMs);
  setEngineState("ready", "Engine ready");
}

function runSource() {
  if (workerFailed) {
    bootWorker({ preserveResult: true, reason: "Retrying engine…" });
    return;
  }

  if (!worker || !workerReady || currentRequest) {
    return;
  }

  const id = nextRequestId;
  nextRequestId += 1;
  currentRequest = {
    id,
    startedAt: performance.now(),
  };

  setEngineState("running", "Evaluating in worker…");
  setResultState("running", "Running");
  elements.resultEmpty.hidden = false;
  elements.resultEmpty.querySelector("p").textContent =
    "The Rust engine is evaluating your script…";
  elements.resultContent.hidden = true;

  worker.postMessage({
    type: "run",
    id,
    source: elements.editor.value,
  });

  executionTimer = globalThis.setTimeout(() => {
    if (!currentRequest || currentRequest.id !== id) {
      return;
    }

    const elapsed = performance.now() - currentRequest.startedAt;
    terminateWorker();
    renderInfrastructureError(
      "TimeoutError",
      "Execution exceeded the playground’s 2-second time budget.",
      "The worker was terminated and a fresh engine is loading.",
      elapsed,
    );
    bootWorker({
      preserveResult: true,
      reason: "Restarting after timeout…",
    });
  }, EXECUTION_TIMEOUT_MS);
}

function selectedExample() {
  return (
    EXAMPLES.find((example) => example.id === elements.exampleSelect.value) ||
    EXAMPLES[0]
  );
}

function loadSelectedExample({ focus = true } = {}) {
  elements.editor.value = selectedExample().source;
  updateSourceStat();
  showEmptyResult("Example loaded. Run it when you are ready.");

  if (focus) {
    elements.editor.focus();
  }
}

function resetPlayground() {
  loadSelectedExample();
  bootWorker({ reason: "Starting a fresh engine…" });
}

function updateSourceStat() {
  const source = elements.editor.value;
  const lines = source.length === 0 ? 0 : source.split("\n").length;
  const bytes = new TextEncoder().encode(source).byteLength;
  elements.sourceStat.textContent = `${lines} ${
    lines === 1 ? "line" : "lines"
  } · ${bytes} bytes`;
}

function insertIndent(event) {
  if (event.key !== "Tab" || event.metaKey || event.ctrlKey || event.altKey) {
    return;
  }

  event.preventDefault();
  const start = elements.editor.selectionStart;
  const end = elements.editor.selectionEnd;
  const source = elements.editor.value;
  elements.editor.value = `${source.slice(0, start)}  ${source.slice(end)}`;
  elements.editor.selectionStart = start + 2;
  elements.editor.selectionEnd = start + 2;
  updateSourceStat();
}

function initializeExamples() {
  for (const example of EXAMPLES) {
    const option = document.createElement("option");
    option.value = example.id;
    option.textContent = example.label;
    elements.exampleSelect.append(option);
  }

  elements.exampleSelect.value = DEFAULT_EXAMPLE_ID;
  loadSelectedExample({ focus: false });
}

elements.runButton.addEventListener("click", runSource);
elements.resetButton.addEventListener("click", resetPlayground);
elements.exampleSelect.addEventListener("change", () => {
  loadSelectedExample();
});
elements.editor.addEventListener("input", updateSourceStat);
elements.editor.addEventListener("keydown", (event) => {
  insertIndent(event);

  if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
    event.preventDefault();
    runSource();
  }
});

globalThis.addEventListener("offline", () => {
  elements.offlineNotice.hidden = false;
});

globalThis.addEventListener("online", () => {
  elements.offlineNotice.hidden = true;

  if (workerFailed) {
    bootWorker({
      preserveResult: true,
      reason: "Connection restored. Retrying…",
    });
  }
});

elements.offlineNotice.hidden = navigator.onLine;
initializeExamples();
bootWorker();
