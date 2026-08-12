const bareBase = "bare";
const computedBase = "./computed-";

Promise.all([
  import(bareBase + ".js"),
  import(computedBase + "block.js"),
  import(`${computedBase}template.js`),
  import(computedBase + "nested.js"),
  import(computedBase + "invalid.js").catch(() => null),
  import(computedBase + "missing.js").catch(() => null),
]);
