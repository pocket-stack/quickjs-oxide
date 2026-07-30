/* global wasm_bindgen */

"use strict";

const PACKAGE_SCRIPT = "./pkg/quickjs_oxide_web.js";
const PACKAGE_WASM = "./pkg/quickjs_oxide_web_bg.wasm";

let initialized = false;

function serializeError(error, fallbackName = "Error") {
  if (error && typeof error === "object") {
    return {
      name: typeof error.name === "string" && error.name.length > 0
        ? error.name
        : fallbackName,
      message: typeof error.message === "string" && error.message.length > 0
        ? error.message
        : String(error),
      stack: typeof error.stack === "string" ? error.stack : null,
    };
  }

  return {
    name: fallbackName,
    message: String(error),
    stack: null,
  };
}

async function initialize() {
  try {
    importScripts(PACKAGE_SCRIPT);

    if (typeof wasm_bindgen !== "function") {
      throw new TypeError(
        "The wasm-bindgen loader did not expose the expected global function.",
      );
    }

    await wasm_bindgen(PACKAGE_WASM);

    if (typeof wasm_bindgen.evaluate !== "function") {
      throw new TypeError(
        "The WASM package did not expose the expected evaluate function.",
      );
    }

    initialized = true;
    self.postMessage({ type: "ready" });
  } catch (error) {
    self.postMessage({
      type: "load-error",
      error: serializeError(error, "EngineLoadError"),
      online: self.navigator ? self.navigator.onLine : null,
    });
  }
}

self.addEventListener("message", (event) => {
  const message = event.data;

  if (
    !message ||
    message.type !== "run" ||
    typeof message.id !== "number" ||
    typeof message.source !== "string"
  ) {
    return;
  }

  if (!initialized) {
    self.postMessage({
      type: "result",
      id: message.id,
      durationMs: 0,
      result: {
        ok: false,
        error: {
          name: "EngineNotReadyError",
          message: "The quickjs-oxide engine is not ready.",
          stack: null,
        },
      },
    });
    return;
  }

  const startedAt = performance.now();

  try {
    const result = wasm_bindgen.evaluate(message.source);
    self.postMessage({
      type: "result",
      id: message.id,
      durationMs: performance.now() - startedAt,
      result,
    });
  } catch (error) {
    self.postMessage({
      type: "result",
      id: message.id,
      durationMs: performance.now() - startedAt,
      result: {
        ok: false,
        error: serializeError(error, "EvaluationError"),
      },
    });
  }
});

void initialize();
