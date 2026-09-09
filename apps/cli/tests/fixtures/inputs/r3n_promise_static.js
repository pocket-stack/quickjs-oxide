function fail(label, actual, expected) {
    throw new Error(
        "R3n Promise static oracle assertion failed for " + label +
        ": expected " + expected + ", got " + actual
    );
}

function same(actual, expected, label) {
    if (actual !== expected)
        fail(label, String(actual), String(expected));
}

var r3nTranscript = [];
function emit(label, value) {
    r3nTranscript.push(label + "=" + value);
}

function forceOracleGc() {
    if (typeof std === "object" && typeof std.gc === "function")
        std.gc();
}

function checkDescriptors() {
    function methodFacts(name, expectedLength) {
        var method = Promise[name];
        var descriptor = Object.getOwnPropertyDescriptor(Promise, name);
        same(typeof method, "function", name + " type");
        same(method.name, name, name + " name");
        same(method.length, expectedLength, name + " length");
        same(descriptor.writable, true, name + " writable");
        same(descriptor.enumerable, false, name + " enumerable");
        same(descriptor.configurable, true, name + " configurable");
        same(
            Object.getOwnPropertyNames(method).join(","),
            "length,name",
            name + " own function keys"
        );
        same(
            Object.prototype.hasOwnProperty.call(method, "prototype"),
            false,
            name + " has no prototype"
        );
        return name + ":" + method.length + ":" +
            descriptor.writable + descriptor.enumerable + descriptor.configurable;
    }

    emit("descriptors", [
        methodFacts("try", 1),
        methodFacts("race", 1),
        methodFacts("withResolvers", 0)
    ].join("|"));
}

function checkGenericAndCustomConstructors() {
    var calls = [];

    function Custom(executor) {
        calls.push("construct");
        var result = { state: "pending" };
        executor(
            function (value) {
                calls.push("resolve:" + value);
                result.state = "fulfilled:" + value;
            },
            function (reason) {
                calls.push("reject:" + reason);
                result.state = "rejected:" + reason;
            }
        );
        return result;
    }

    var tried = Promise.try.call(
        Custom,
        function (left, right) {
            "use strict";
            same(this, undefined, "Promise.try callback this");
            calls.push("callback:" + left + ":" + right);
            return left + right;
        },
        20,
        22
    );
    same(tried.state, "fulfilled:42", "custom Promise.try result");

    var capability = Promise.withResolvers.call(Custom);
    same(capability.promise.state, "pending", "custom withResolvers initial state");
    capability.reject("custom");
    same(
        capability.promise.state,
        "rejected:custom",
        "custom withResolvers reject"
    );

    Custom.resolve = function (value) {
        calls.push("static-resolve:" + value);
        return {
            then: function (resolve) {
                calls.push("then:" + value);
                resolve(value);
            }
        };
    };
    var raced = Promise.race.call(Custom, [42]);
    same(raced.state, "fulfilled:42", "custom Promise.race result");

    function genericError(name, invoke) {
        try {
            invoke();
        } catch (error) {
            same(error.name, "TypeError", name + " generic error");
            return error.name;
        }
        fail(name + " generic error", "no throw", "TypeError");
    }

    emit("custom-generic", [
        calls.join("|"),
        genericError("try", function () {
            Promise.try.call({}, function () {});
        }),
        genericError("withResolvers", function () {
            Promise.withResolvers.call({});
        }),
        genericError("race", function () {
            Promise.race.call({}, []);
        })
    ].join("||"));
}

function checkTryReturnArgumentsAndThrow() {
    var order = [];
    var reason = {};

    var returned = Promise.try(
        function (left, right) {
            "use strict";
            same(this, undefined, "Promise.try standard callback this");
            order.push("call:" + left + ":" + right);
            return left + right;
        },
        19,
        23
    );
    order.push("after-return");

    var rejected = Promise.try(function () {
        order.push("throw");
        throw reason;
    });
    order.push("after-throw");

    return returned
        .then(function (value) {
            same(value, 42, "Promise.try fulfillment");
            order.push("fulfilled:" + value);
            return rejected;
        })
        .then(
            function () {
                fail("Promise.try thrown callback", "fulfilled", "rejected");
            },
            function (actual) {
                same(actual, reason, "Promise.try rejection identity");
                order.push("rejected:true");
                emit("try", order.join("|"));
            }
        );
}

function checkWithResolversShapeAndSettlement() {
    var capability = Promise.withResolvers();
    var keys = Object.keys(capability).join(",");
    same(keys, "promise,resolve,reject", "withResolvers key order");
    same(
        Object.getPrototypeOf(capability),
        Object.prototype,
        "withResolvers object prototype"
    );

    var facts = ["promise", "resolve", "reject"].map(function (name) {
        var descriptor = Object.getOwnPropertyDescriptor(capability, name);
        return name + ":" +
            descriptor.writable + descriptor.enumerable + descriptor.configurable;
    });
    same(typeof capability.resolve, "function", "withResolvers resolve type");
    same(typeof capability.reject, "function", "withResolvers reject type");

    var order = [];
    capability.promise.then(
        function (value) {
            order.push("fulfilled:" + value);
        },
        function (reason) {
            order.push("rejected:" + reason);
        }
    );
    capability.resolve(42);
    capability.reject(43);
    order.push("after-settle");

    return capability.promise.then(function (value) {
        same(value, 42, "withResolvers first call wins");
        same(
            order.join("|"),
            "after-settle|fulfilled:42",
            "withResolvers asynchronous settlement"
        );
        emit("withResolvers", [
            keys,
            facts.join("|"),
            order.join("|")
        ].join("||"));
    });
}

function checkRaceEmptyAndFifo() {
    var emptyState = "pending";
    Promise.race([]).then(
        function () { emptyState = "fulfilled"; },
        function () { emptyState = "rejected"; }
    );

    var order = [];
    var raced = Promise.race([
        Promise.resolve("first"),
        Promise.resolve("second")
    ]);
    raced.then(function (value) {
        order.push("winner:" + value);
    });

    return Promise.resolve()
        .then(function () {
            same(emptyState, "pending", "empty Promise.race remains pending");
            order.push("checkpoint");
            return raced;
        })
        .then(function (value) {
            same(value, "first", "Promise.race FIFO winner");
            same(
                order.join("|"),
                "checkpoint|winner:first",
                "Promise.race reaction FIFO"
            );
            emit("race-empty-fifo", emptyState + "|" + order.join("|"));
        });
}

function checkResolveGetterOrdering() {
    var log = [];
    var marker = {};

    function Constructor(executor) {
        return new Promise(executor);
    }
    Object.defineProperty(Constructor, "resolve", {
        configurable: true,
        get: function () {
            log.push("resolve-get");
            throw marker;
        }
    });
    var iterable = {};
    iterable[Symbol.iterator] = function () {
        log.push("iterator-get");
        return [][Symbol.iterator]();
    };

    return Promise.race.call(Constructor, iterable).then(
        function () {
            fail("race resolve getter abrupt", "fulfilled", "rejected");
        },
        function (reason) {
            same(reason, marker, "race resolve getter rejection identity");
            same(
                log.join("|"),
                "resolve-get",
                "race resolve getter precedes iterator access"
            );
            emit("race-resolve-getter", log.join("|"));
        }
    );
}

function checkIteratorNextAbruptDoesNotClose() {
    var log = [];
    var marker = {};
    var iterable = {};
    iterable[Symbol.iterator] = function () {
        log.push("iterator");
        return {
            next: function () {
                log.push("next");
                throw marker;
            },
            return: function () {
                log.push("close");
                return {};
            }
        };
    };

    return Promise.race(iterable).then(
        function () {
            fail("race iterator-next abrupt", "fulfilled", "rejected");
        },
        function (reason) {
            same(reason, marker, "race iterator-next rejection identity");
            same(
                log.join("|"),
                "iterator|next",
                "pinned race iterator-next does not close"
            );
            emit("race-next-no-close", log.join("|"));
        }
    );
}

function checkResolveAbruptClosesIterator() {
    var log = [];
    var marker = {};

    function Constructor(executor) {
        return new Promise(executor);
    }
    Constructor.resolve = function () {
        log.push("resolve");
        throw marker;
    };
    var iterable = {};
    iterable[Symbol.iterator] = function () {
        var yielded = false;
        return {
            next: function () {
                log.push("next");
                if (yielded)
                    return { done: true };
                yielded = true;
                return { done: false, value: 1 };
            },
            return: function () {
                log.push("close");
                return {};
            }
        };
    };

    return Promise.race.call(Constructor, iterable).then(
        function () {
            fail("race resolve abrupt", "fulfilled", "rejected");
        },
        function (reason) {
            same(reason, marker, "race resolve abrupt rejection identity");
            same(
                log.join("|"),
                "next|resolve|close",
                "race resolve abrupt closes iterator"
            );
            emit("race-resolve-close", log.join("|"));
        }
    );
}

function checkThenAbruptClosesAndPreservesOriginal() {
    var log = [];
    var marker = {};
    var closeMarker = {};

    function Constructor(executor) {
        return new Promise(executor);
    }
    Constructor.resolve = function () {
        return {
            then: function () {
                log.push("then");
                throw marker;
            }
        };
    };
    var iterable = {};
    iterable[Symbol.iterator] = function () {
        var yielded = false;
        return {
            next: function () {
                log.push("next");
                if (yielded)
                    return { done: true };
                yielded = true;
                return { done: false, value: 1 };
            },
            return: function () {
                log.push("close");
                throw closeMarker;
            }
        };
    };

    return Promise.race.call(Constructor, iterable).then(
        function () {
            fail("race then abrupt", "fulfilled", "rejected");
        },
        function (reason) {
            same(reason, marker, "IteratorClose preserves then abrupt reason");
            same(
                log.join("|"),
                "next|then|close",
                "race then abrupt closes iterator"
            );
            emit("race-then-close", log.join("|") + "|original:true");
        }
    );
}

function checkQueuedRaceGraphSurvivesGc() {
    var gcAnswer = 0;
    var marker = { value: 42 };
    var thenable = {
        payload: marker,
        then: function (resolve) {
            resolve(this.payload);
        }
    };
    var raced = Promise.race([thenable]);
    var done = raced.then(function (value) {
        gcAnswer = value.value;
    });

    marker = null;
    thenable = null;
    raced = null;
    forceOracleGc();

    return done.then(function () {
        same(gcAnswer, 42, "queued Promise.race graph across GC");
        emit("race-gc", gcAnswer);
    });
}

var r3nDone = Promise.resolve()
    .then(checkDescriptors)
    .then(checkGenericAndCustomConstructors)
    .then(checkTryReturnArgumentsAndThrow)
    .then(checkWithResolversShapeAndSettlement)
    .then(checkRaceEmptyAndFifo)
    .then(checkResolveGetterOrdering)
    .then(checkIteratorNextAbruptDoesNotClose)
    .then(checkResolveAbruptClosesIterator)
    .then(checkThenAbruptClosesAndPreservesOriginal)
    .then(checkQueuedRaceGraphSurvivesGc)
    .then(function () {
        emit("r3n-promise-static-oracle", "ok");
    });

forceOracleGc();
r3nDone;
