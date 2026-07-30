export const EXAMPLES = [
  {
    id: "return-42",
    label: "Return 42",
    source: `function answer() {
  return 42;
}

answer();`,
  },
  {
    id: "typed-array",
    label: "TypedArray.from",
    source: `var values = Uint8Array.from([10, 20, 12]);

values[0] + values[1] + values[2];`,
  },
  {
    id: "class",
    label: "Class instance",
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
    source: `var promise = Promise.resolve(42);

promise instanceof Promise;`,
  },
  {
    id: "array-pipeline",
    label: "Array pipeline",
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
