for (var value of [1]) {}
export { value };

for (let value of [42]) {
  print(`nested:${value}`);
  {
    let value = 43;
    print(`nested-deep:${value}`);
  }
  print(`nested-after:${value}`);
}

for (let value of [44]) {
  print(`sibling:${value}`);
}

print(`outer-runtime:${value}`);
