# Deviation ledger

This ledger is subordinate to [`parity.md`](parity.md). There are currently no
approved observable differences from QuickJS 2026-06-04. An unsupported feature
or an unresolved mismatch blocks the relevant parity claim; it is not silently
accepted as a deviation.

## Resolved findings

### FORIN-FAST-ARRAY-001

- Status: resolved on 2026-07-15; no deviation approval requested.
- Surface: representation-sensitive Array mutation during `for-in`.
- Upstream anchor: `quickjs.c` 16282-16509.
- Compatibility impact while open: a deleted dense own index could incorrectly
  hide an inherited key, and a newly added own key could fail to hide one.

Minimal deletion probe:

```js
(function () {
  var p = [];
  p[1] = "proto";
  var a = [0, 1], out = "";
  Object.setPrototypeOf(a, p);
  for (var key in a) {
    out += key + ",";
    if (key === "0") delete a[1];
  }
  return out;
})()
```

Pinned QuickJS and the current Rust engine both return `0,1,`. A second
differential forces the same current key set through `Object.defineProperty` or
sparse growth first; both engines then retain the slow representation and
return `0,`.

The fix records QuickJS's irreversible fast/slow Array state in the heap
payload. A count-only fast iterator refreshes the source's current own names
when prototype enumeration becomes necessary; a slow iterator retains its
initial snapshot. Both paths are pinned in `tests/oracle_for_in.rs`.

## Open implementation frontiers

- `Promise.all` matches pinned QuickJS on ordinary JavaScript-observable paths,
  but internal allocation failure is not yet routed identically. Failure to
  allocate the values Array currently returns a host runtime error instead of
  rejecting the new capability; failure to allocate an element callback also
  omits QuickJS's close-then-reject path. The checked `u32` element counter has
  a theoretical multi-billion-element RangeError boundary instead of
  QuickJS's C `int` environment behavior. These are unresolved hardening
  frontiers, not approved deviations.
- `for-in` over Proxy is not implemented because Proxy itself is absent. The VM
  host outcome already carries arbitrary JavaScript throws; before Proxy is
  admitted, differential tests must lock the two prototype passes, `ownKeys`,
  descriptor, live-presence, and `getPrototypeOf` trap order. This is an
  unsupported frontier, not an approved deviation.
