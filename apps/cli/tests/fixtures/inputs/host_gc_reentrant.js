function hostGcFail(label, actual, expected) {
    throw new Error(
        "host GC oracle assertion failed for " + label +
        ": expected " + expected + ", got " + actual
    );
}

function hostGcSame(actual, expected, label) {
    if (actual !== expected)
        hostGcFail(label, String(actual), String(expected));
}

var hostGc =
    typeof $262 === "object" && typeof $262.gc === "function"
        ? $262.gc
        : typeof std === "object" && typeof std.gc === "function"
          ? std.gc
          : undefined;
hostGcSame(typeof hostGc, "function", "host callback availability");

var hostGcTranscript = [];
function hostGcEmit(label, value) {
    hostGcTranscript.push(label + "=" + value);
}

var hostGcConstructRejected = false;
try {
    new hostGc();
} catch (error) {
    hostGcConstructRejected = error instanceof TypeError;
}
hostGcEmit(
    "host-shape",
    [
        typeof hostGc,
        hostGc.name,
        hostGc.length,
        Object.prototype.hasOwnProperty.call(hostGc, "prototype"),
        hostGcConstructRejected,
        hostGc.call({ ignored: true }, 1, 2) === undefined
    ].join("|")
);

var hostGcActiveAnswer = (function (argument) {
    var local = { value: 20 };
    var captured = { value: 2 };
    function closure() {
        return captured.value;
    }
    var receiver = this;
    hostGcSame(hostGc(), undefined, "active-frame GC return value");
    return receiver.value + argument.value + local.value + closure();
}).call({ value: 10 }, { value: 10 });
hostGcSame(hostGcActiveAnswer, 42, "active receiver, argument, local, and closure roots");
hostGcEmit("active-frame", hostGcActiveAnswer);

var hostGcGenerator = (function* () {
    var local = { value: 20 };
    var captured = { value: 2 };
    function closure() {
        return captured.value;
    }
    var argument = yield "ready";
    hostGcSame(hostGc(), undefined, "generator-frame GC return value");
    return argument.value + local.value + closure();
})();
var hostGcGeneratorFirst = hostGcGenerator.next();
hostGcSame(hostGcGeneratorFirst.value, "ready", "generator suspension value");
hostGcSame(hostGcGeneratorFirst.done, false, "generator suspension state");
var hostGcGeneratorLast = hostGcGenerator.next({ value: 20 });
hostGcSame(hostGcGeneratorLast.value, 42, "generator active-frame roots");
hostGcSame(hostGcGeneratorLast.done, true, "generator completion state");
hostGcEmit("generator-frame", hostGcGeneratorLast.value);

var hostGcImmediateWeakRef = (function () {
    var target = { marker: "immediate" };
    var reference = new WeakRef(target);
    target = null;
    return reference;
})();
var hostGcImmediateDead = hostGcImmediateWeakRef.deref() === undefined;

var hostGcCycleWeakRef;
(function () {
    var target = { marker: "cycle" };
    target.self = target;
    hostGcCycleWeakRef = new WeakRef(target);
    target = null;
})();
var hostGcCycleAliveBefore = hostGcCycleWeakRef.deref() !== undefined;
hostGc();
var hostGcCycleDeadAfterFirst = hostGcCycleWeakRef.deref() === undefined;
hostGc();
var hostGcCycleDeadAfterSecond = hostGcCycleWeakRef.deref() === undefined;
hostGcSame(hostGcImmediateDead, true, "zero-refcount WeakRef target before explicit GC");
hostGcSame(hostGcCycleAliveBefore, true, "cycle WeakRef target before explicit GC");
hostGcSame(hostGcCycleDeadAfterFirst, true, "cycle WeakRef target after first explicit GC");
hostGcSame(hostGcCycleDeadAfterSecond, true, "cycle WeakRef target after weak-list cleanup");
hostGcEmit(
    "weakref-death",
    [
        hostGcImmediateDead,
        hostGcCycleAliveBefore,
        hostGcCycleDeadAfterFirst,
        hostGcCycleDeadAfterSecond
    ].join("|")
);

var hostGcEphemeronMap = new WeakMap();
var hostGcEphemeronKey = { marker: "key" };
var hostGcEphemeronValue = { answer: 42 };
var hostGcEphemeronValueRef = new WeakRef(hostGcEphemeronValue);
hostGcEphemeronMap.set(hostGcEphemeronKey, hostGcEphemeronValue);
hostGcEphemeronKey = null;
hostGcEphemeronValue = null;
var hostGcEphemeronAliveBefore = hostGcEphemeronValueRef.deref().answer === 42;
hostGc();
var hostGcEphemeronDeadAfter = hostGcEphemeronValueRef.deref() === undefined;
hostGcSame(hostGcEphemeronAliveBefore, true, "WeakMap value before dead-key collection");
hostGcSame(hostGcEphemeronDeadAfter, true, "WeakMap value after dead-key collection");
hostGcEmit(
    "weakmap-ephemeron",
    hostGcEphemeronAliveBefore + "|" + hostGcEphemeronDeadAfter
);

var hostGcJobOrder = [];
var hostGcDoneResolve;
var hostGcDone = new Promise(function (resolve) {
    hostGcDoneResolve = resolve;
});
var hostGcRegistry = new FinalizationRegistry(function (heldValue) {
    hostGcJobOrder.push("finalizer:" + heldValue);
    if (heldValue === "pending")
        hostGcDoneResolve();
});

var hostGcPendingWeakRef;
(function () {
    var captured = { answer: 42 };
    hostGcPendingWeakRef = new WeakRef(captured);
    hostGcRegistry.register(captured, "pending");
    Promise.resolve().then(function () {
        hostGcSame(
            hostGcPendingWeakRef.deref(),
            captured,
            "pending reaction closure before reentrant GC"
        );
        hostGc();
        hostGcSame(
            hostGcPendingWeakRef.deref(),
            captured,
            "pending reaction closure during reentrant GC"
        );
        hostGcJobOrder.push("pending-active:" + captured.answer);
        Promise.resolve().then(function () {
            var deadBefore = hostGcPendingWeakRef.deref() === undefined;
            hostGc();
            var deadAfter = hostGcPendingWeakRef.deref() === undefined;
            hostGcJobOrder.push("pending-released:" + deadBefore + "|" + deadAfter);
        });
    });
})();

Promise.resolve().then(function () {
    hostGcJobOrder.push("promise-before-native");
});

function hostGcRegisterTransient(heldValue) {
    var target = { heldValue: heldValue };
    hostGcRegistry.register(target, heldValue);
    return target;
}
hostGc.call(
    hostGcRegisterTransient("native-this"),
    hostGcRegisterTransient("native-argument")
);
Promise.resolve().then(function () {
    hostGcJobOrder.push("promise-after-native");
});
hostGc();

var hostGcCycleTarget = { marker: "finalization-cycle" };
hostGcCycleTarget.self = hostGcCycleTarget;
hostGcRegistry.register(hostGcCycleTarget, "cycle");
hostGcCycleTarget = null;
Promise.resolve().then(function () {
    hostGcJobOrder.push("promise-before-cycle");
});
hostGc();
Promise.resolve().then(function () {
    hostGcJobOrder.push("promise-between-cycle-gcs");
});
hostGc();
Promise.resolve().then(function () {
    hostGcJobOrder.push("promise-after-cycle");
});

hostGcSame(hostGcJobOrder.length, 0, "host GC must not execute pending jobs");
hostGcEmit("gc-sync", hostGcJobOrder.length);

hostGcDone.then(function () {
    hostGcEmit("job-fifo", hostGcJobOrder.join("|"));
    if (typeof print === "function")
        print(hostGcTranscript.join("\n"));
});
