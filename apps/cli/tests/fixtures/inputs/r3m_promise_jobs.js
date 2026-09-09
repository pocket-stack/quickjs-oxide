function fail(label, actual, expected) {
    throw new Error(
        "R3m Promise/job oracle assertion failed for " + label +
        ": expected " + expected + ", got " + actual
    );
}

function same(actual, expected, label) {
    if (actual !== expected)
        fail(label, String(actual), String(expected));
}

var r3mTranscript = [];
function emit(label, value) {
    r3mTranscript.push(label + "=" + value);
}

function forceOracleGc() {
    // The pinned qjs invocation exposes std.gc(). A future Oxide comparison can
    // run the same fixture without a script-visible GC hook.
    if (typeof std === "object" && typeof std.gc === "function")
        std.gc();
}

var executorOrder = [];
var executorReactionRan = false;
var executorPromise = new Promise(function (resolve) {
    executorOrder.push("executor");
    resolve(40);
    executorOrder.push("after-resolve");
});
executorOrder.push("after-constructor");
var executorObserved = executorPromise.then(function (value) {
    executorReactionRan = true;
    executorOrder.push("reaction:" + value);
    return value + 1;
});
executorOrder.push("after-then:" + executorReactionRan);
same(
    executorOrder.join("|"),
    "executor|after-resolve|after-constructor|after-then:false",
    "synchronous executor and asynchronous reaction setup"
);
emit("executor-sync", executorOrder.join("|"));

function checkFifoAndNestedTail() {
    var order = [];
    var nestedResolve;
    var nestedDone = new Promise(function (resolve) {
        nestedResolve = resolve;
    });
    var settled = Promise.resolve(0);

    settled.then(function () {
        order.push("A");
        settled.then(function () {
            order.push("nested");
            nestedResolve();
        });
    });
    settled.then(function () {
        order.push("B");
    });

    return nestedDone.then(function () {
        same(order.join("|"), "A|B|nested", "FIFO jobs and nested tail enqueue");
        emit("fifo-nested-tail", order.join("|"));
    });
}

function checkSettledLateThen() {
    var order = [];
    var lateResolve;
    var lateDone = new Promise(function (resolve) {
        lateResolve = resolve;
    });
    var settled = Promise.resolve(42);

    settled.then(function () {
        order.push("first");
        settled.then(function (value) {
            order.push("late:" + value);
            lateResolve();
        });
        order.push("after-register");
    });

    return lateDone.then(function () {
        same(
            order.join("|"),
            "first|after-register|late:42",
            "then registered on an already-settled promise"
        );
        emit("settled-late-then", order.join("|"));
    });
}

function checkPassThroughThrowAndCatch() {
    var marker = {};
    var order = [];

    return Promise.resolve(marker)
        .then()
        .then(function (value) {
            same(value, marker, "missing fulfillment handler pass-through");
            order.push("fulfilled-pass");
            return Promise.reject(marker).then();
        })
        .then(function () {
            fail("missing rejection handler", "fulfilled", "rejected");
        })
        .catch(function (reason) {
            same(reason, marker, "missing rejection handler pass-through");
            order.push("rejected-pass");
            throw new RangeError("chain");
        })
        .catch(function (error) {
            same(error.name, "RangeError", "thrown handler error type");
            same(error.message, "chain", "thrown handler error message");
            order.push(error.name + ":" + error.message);
            return 42;
        })
        .then(function (value) {
            same(value, 42, "catch recovery value");
            order.push(value);
            emit("pass-throw-catch", order.join("|"));
        });
}

function checkThenableTiming() {
    var order = [];
    var thenable = {};
    Object.defineProperty(thenable, "then", {
        configurable: true,
        get: function () {
            order.push("get");
            return function (resolve, reject) {
                order.push("call");
                resolve(42);
                reject(43);
                resolve(44);
            };
        }
    });

    var adopted = Promise.resolve(thenable);
    order.push("after-resolve");
    same(order.join("|"), "get|after-resolve", "then getter versus then call timing");

    var getterReason = {};
    var brokenThenable = {};
    Object.defineProperty(brokenThenable, "then", {
        get: function () {
            order.push("throwing-get");
            throw getterReason;
        }
    });
    var broken = Promise.resolve(brokenThenable);
    order.push("after-throwing-resolve");
    var brokenOutcome = broken.then(
        function () {
            fail("throwing then getter", "fulfilled", "rejected");
        },
        function (reason) {
            same(reason, getterReason, "then getter rejection identity");
            order.push("getter-rejected");
        }
    );

    return adopted
        .then(function (value) {
            same(value, 42, "thenable first call wins");
            order.push("reaction:" + value);
            return brokenOutcome;
        })
        .then(function () {
            same(
                order.join("|"),
                "get|after-resolve|throwing-get|after-throwing-resolve|call|" +
                    "getter-rejected|reaction:42",
                "thenable assimilation ordering"
            );
            emit("thenable-timing", order.join("|"));
        });
}

function checkFirstCallAndSelfResolution() {
    var resolveFirst;
    var rejectFirst;
    var first = new Promise(function (resolve, reject) {
        resolveFirst = resolve;
        rejectFirst = reject;
    });
    var firstOutcome = first.then(
        function (value) { return value; },
        function () { fail("resolve/reject pair", "rejected", "fulfilled"); }
    );
    resolveFirst(42);
    rejectFirst(43);
    resolveFirst(44);

    var resolveSelf;
    var self = new Promise(function (resolve) {
        resolveSelf = resolve;
    });
    var selfOutcome = self.then(
        function () { fail("self resolution", "fulfilled", "rejected"); },
        function (error) {
            same(error.name, "TypeError", "self-resolution error type");
            return error.name;
        }
    );
    resolveSelf(self);

    var order = [];
    return firstOutcome
        .then(function (value) {
            same(value, 42, "first resolve call wins");
            order.push("first:" + value);
            return selfOutcome;
        })
        .then(function (errorName) {
            order.push("self:" + errorName);
            emit("first-call-self", order.join("|"));
        });
}

function checkConstructorIdentityAndSpecies() {
    class DerivedPromise extends Promise {}
    class SpeciesPromise extends Promise {
        static get [Symbol.species]() { return Promise; }
    }

    var base = Promise.resolve(40);
    var derived = new DerivedPromise(function (resolve) { resolve(41); });
    var wrapped = Promise.resolve(derived);
    var viaDerived = Promise.resolve.call(DerivedPromise, base);
    var speciesSource = new SpeciesPromise(function (resolve) { resolve(40); });
    var speciesResult = speciesSource.then(function (value) { return value + 2; });
    var reason = {};
    var rejected = Promise.reject(reason);
    var rejectedAgain = Promise.reject(reason);
    var rejectedOutcome = rejected.catch(function (actual) {
        return actual === reason;
    });
    var rejectedAgainOutcome = rejectedAgain.catch(function (actual) {
        return actual === reason;
    });

    var facts = [
        Promise.resolve(base) === base,
        DerivedPromise.resolve(derived) === derived,
        wrapped !== derived,
        wrapped instanceof Promise,
        wrapped instanceof DerivedPromise,
        viaDerived !== base,
        viaDerived instanceof DerivedPromise,
        rejected !== rejectedAgain,
        speciesResult instanceof Promise,
        speciesResult instanceof SpeciesPromise
    ];

    // qjs has no script-level createRealm hook. Constructor and @@species
    // selection are the allocation-boundary semantics observable in this fixture;
    // true cross-context realm ownership belongs in the Rust runtime tests.
    return wrapped
        .then(function (value) {
            facts.push(value);
            return viaDerived;
        })
        .then(function (value) {
            facts.push(value);
            return rejectedOutcome;
        })
        .then(function (sameReason) {
            facts.push(sameReason);
            return rejectedAgainOutcome;
        })
        .then(function (sameReason) {
            facts.push(sameReason);
            return speciesResult;
        })
        .then(function (value) {
            facts.push(value);
            same(
                facts.join("|"),
                "true|true|true|true|false|true|true|true|true|false|" +
                    "41|40|true|true|42",
                "Promise constructor identity reject identity and species"
            );
            emit("constructor-species", facts.join("|"));
        });
}

function checkQueuedJobRetention() {
    var payload = { value: 42 };
    var source = Promise.resolve(payload);
    var observed = source.then(function (value) {
        return value.value;
    });

    payload = null;
    source = null;
    forceOracleGc();

    return observed.then(function (value) {
        same(value, 42, "queued job retains its value graph");
        forceOracleGc();
        emit("queued-job-retention", value);
    });
}

var r3mDone = executorObserved
    .then(function (value) {
        same(value, 41, "executor reaction result");
        same(executorReactionRan, true, "reaction eventually ran");
        emit("executor-reaction", executorOrder.join("|") + "|result:" + value);
        return checkFifoAndNestedTail();
    })
    .then(checkSettledLateThen)
    .then(checkPassThroughThrowAndCatch)
    .then(checkThenableTiming)
    .then(checkFirstCallAndSelfResolution)
    .then(checkConstructorIdentityAndSpecies)
    .then(checkQueuedJobRetention)
    .then(function () {
        r3mTranscript.push("r3m-promise-jobs-oracle=ok");
        return r3mTranscript.join("\n");
    });

r3mDone;
