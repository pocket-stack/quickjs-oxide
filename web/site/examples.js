export const EXAMPLES = [
  {
    id: "return-42",
    label: "Return 42",
    description: "Calls a JavaScript function compiled and run by quickjs-oxide.",
    expected: { kind: "number", text: "42" },
    source: `function answer() {
  return 42;
}

answer();`,
  },
  {
    id: "default-parameters",
    label: "Default parameters",
    description: "Uses parameter destructuring, defaults, and a function call.",
    expected: { kind: "number", text: "42" },
    source: `function answer({ base = 40 } = {}, offset = 2) {
  return base + offset;
}

answer();`,
  },
  {
    id: "typed-array",
    label: "TypedArray.from",
    description: "Builds a typed array from an iterable and reads its elements.",
    expected: { kind: "number", text: "42" },
    source: `var values = Uint8Array.from([10, 20, 12]);

values[0] + values[1] + values[2];`,
  },
  {
    id: "atomics-non-shared",
    label: "Atomics.pause + ArrayBuffer",
    description: "Exercises the pinned QuickJS non-shared Atomics behavior.",
    expected: { kind: "number", text: "42" },
    source: `var counter = new Int32Array(new ArrayBuffer(4));
Atomics.store(counter, 0, 40);
Atomics.add(counter, 0, 2);

Atomics.pause(42) === undefined ? Atomics.load(counter, 0) : 0;`,
  },
  {
    id: "resizable-array-buffer",
    label: "Resizable ArrayBuffer",
    description: "Grows a resizable buffer and observes it through a new view.",
    expected: { kind: "number", text: "42" },
    source: `var buffer = new ArrayBuffer(1, { maxByteLength: 4 });
var bytes = new Uint8Array(buffer);
bytes[0] = 40;
buffer.resize(2);
var grown = new Uint8Array(buffer);
grown[1] = 2;

grown[0] + grown[1];`,
  },
  {
    id: "shared-array-buffer",
    label: "SharedArrayBuffer views",
    description: "Grows, views, and slices a SharedArrayBuffer in one realm.",
    expected: { kind: "number", text: "42" },
    source: `var buffer = new SharedArrayBuffer(2, { maxByteLength: 4 });
var bytes = new Uint8Array(buffer);
bytes[0] = 40;
buffer.grow(4);
new DataView(buffer).setUint8(1, 2);
var copy = buffer.slice(0, 2);

new Uint8Array(copy)[0] + new Uint8Array(copy)[1];`,
  },
  {
    id: "shared-atomics",
    label: "Shared Atomics",
    description: "Performs sequentially consistent operations on shared memory.",
    expected: { kind: "number", text: "42" },
    source: `var counter = new Int32Array(new SharedArrayBuffer(4));
Atomics.store(counter, 0, 40);
Atomics.add(counter, 0, 2);

Atomics.load(counter, 0);`,
  },
  {
    id: "atomics-wait-policy",
    label: "Atomics.wait host policy",
    description: "Confirms that the browser host forbids synchronous blocking.",
    expected: { kind: "number", text: "42" },
    source: `var waiters = new Int32Array(new SharedArrayBuffer(4));
var answer = 0;

try {
  Atomics.wait(waiters, 0, 0, 0);
} catch (error) {
  answer = error instanceof TypeError &&
    error.name === "TypeError" &&
    error.message === "cannot block in this thread"
      ? 42
      : 0;
}

answer;`,
  },
  {
    id: "uint8-codec",
    label: "Uint8Array.fromHex",
    description: "Decodes a hexadecimal byte using the current typed-array API.",
    expected: { kind: "number", text: "42" },
    source: `var bytes = Uint8Array.fromHex("2a");

bytes[0];`,
  },
  {
    id: "unicode-strings",
    label: "Unicode strings",
    description: "Normalizes canonically equivalent UTF-16 string sequences.",
    expected: { kind: "number", text: "42" },
    source: `var composed = "é";
var decomposed = "e\\u0301";

composed.localeCompare(decomposed) === 0 &&
decomposed.normalize("NFC") === composed
  ? 42
  : 0;`,
  },
  {
    id: "class",
    label: "Class instance",
    description: "Constructs a class instance and dispatches an instance method.",
    expected: { kind: "number", text: "42" },
    source: `class Counter {
  constructor(value) {
    this.value = value;
  }

  add(value) {
    this.value += value;
    return this.value;
  }
}

new Counter(40).add(2);`,
  },
  {
    id: "promise",
    label: "Promise creation",
    description: "Creates a fulfilled Promise and checks its runtime identity.",
    expected: { kind: "boolean", text: "true" },
    source: `var promise = Promise.resolve(42);

promise instanceof Promise;`,
  },
  {
    id: "weak-map",
    label: "WeakMap identity",
    description: "Stores and updates a value under an object-identity key.",
    expected: { kind: "number", text: "42" },
    source: `var key = {};
var answers = new WeakMap([[key, 40]]);
answers.set(key, answers.get(key) + 2);

answers.get(key);`,
  },
  {
    id: "weak-ref",
    label: "WeakRef dereference",
    description: "Dereferences a still-live target through a WeakRef.",
    expected: { kind: "number", text: "42" },
    source: `var target = { answer: 42 };
var reference = new WeakRef(target);

reference.deref().answer;`,
  },
  {
    id: "array-pipeline",
    label: "Array pipeline",
    description: "Maps and reduces an array through JavaScript callbacks.",
    expected: { kind: "number", text: "42" },
    source: `[3, 7, 11]
  .map(function (value) {
    return value * 2;
  })
  .reduce(function (total, value) {
    return total + value;
  }, 0);`,
  },
];

export const DEFAULT_EXAMPLE_ID = "return-42";
