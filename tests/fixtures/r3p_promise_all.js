function fail(label, actual, expected) {
    throw new Error(
        "R3p Promise.all oracle assertion failed for " + label +
        ": expected " + expected + ", got " + actual
    );
}

function same(actual, expected, label) {
    if (actual !== expected)
        fail(label, String(actual), String(expected));
}

var r3pTranscript = [];
function emit(label, value) {
    r3pTranscript.push(label + "=" + value);
}

function forceOracleGc() {
    if (typeof std === "object" && typeof std.gc === "function")
        std.gc();
}

function checkDescriptorAndGenericErrors() {
    var method = Promise.all;
    var descriptor = Object.getOwnPropertyDescriptor(Promise, "all");
    same(typeof method, "function", "all type");
    same(method.name, "all", "all name");
    same(method.length, 1, "all length");
    same(descriptor.writable, true, "all writable");
    same(descriptor.enumerable, false, "all enumerable");
    same(descriptor.configurable, true, "all configurable");
    same(
        Object.getOwnPropertyNames(method).join(","),
        "length,name",
        "all own function keys"
    );
    same(
        Object.prototype.hasOwnProperty.call(method, "prototype"),
        false,
        "all has no prototype"
    );

    function genericError(label, receiver) {
        try {
            method.call(receiver, []);
        } catch (error) {
            same(error.name, "TypeError", label + " TypeError");
            return error.name;
        }
        fail(label, "returned", "threw");
    }

    emit("descriptor-generic", [
        method.name + ":" + method.length + ":" +
            descriptor.writable + descriptor.enumerable + descriptor.configurable,
        genericError("primitive receiver", 1),
        genericError("non-constructor object", {})
    ].join("|"));
}

function checkCustomConstructorAndCallbackShape() {
    var log = [];
    var finalResolve;
    var finalReject;
    var fulfillCallbacks = [];
    var rejectCallbacks = [];

    function Custom(executor) {
        var result = { state: "pending" };
        finalResolve = function (value) {
            result.state = "fulfilled";
            result.value = value;
            log.push("final-resolve");
        };
        finalReject = function (reason) {
            result.state = "rejected";
            result.reason = reason;
            log.push("final-reject");
        };
        executor(finalResolve, finalReject);
        return result;
    }
    Custom.resolve = function (value) {
        log.push("resolve:" + value);
        return {
            then: function (onFulfilled, onRejected) {
                fulfillCallbacks.push(onFulfilled);
                rejectCallbacks.push(onRejected);
                log.push("then:" + value);
                onFulfilled(value);
            }
        };
    };

    var result = Promise.all.call(Custom, [20, 22]);
    same(result.state, "fulfilled", "custom constructor state");
    same(result.value.join(","), "20,22", "custom constructor values");
    same(fulfillCallbacks.length, 2, "custom fulfill callback count");
    same(fulfillCallbacks[0] === fulfillCallbacks[1], false, "fresh callbacks");
    same(fulfillCallbacks[0] === finalResolve, false, "callback is not final resolve");
    same(rejectCallbacks[0], finalReject, "first reject identity");
    same(rejectCallbacks[1], finalReject, "second reject identity");
    same(fulfillCallbacks[0].name, "", "callback name");
    same(fulfillCallbacks[0].length, 1, "callback length");
    same(
        Object.getOwnPropertyNames(fulfillCallbacks[0]).join(","),
        "length,name",
        "callback own keys"
    );
    same(
        Object.prototype.hasOwnProperty.call(fulfillCallbacks[0], "prototype"),
        false,
        "callback has no prototype"
    );

    emit("custom-shape", [
        result.state + ":" + result.value.join(","),
        fulfillCallbacks[0].name + ":" + fulfillCallbacks[0].length,
        fulfillCallbacks[0] === fulfillCallbacks[1],
        rejectCallbacks[0] === finalReject &&
            rejectCallbacks[1] === finalReject,
        log.join("|")
    ].join("||"));
}

function checkEmptyAndOutOfOrder() {
    var emptyOrder = [];
    var empty = Promise.all([]);
    empty.then(function (values) {
        emptyOrder.push("fulfilled:" + values.length);
        same(Object.getPrototypeOf(values), Array.prototype, "empty array prototype");
    });
    emptyOrder.push("after-call");

    var callbacks = [];
    var markers = [{ id: 0 }, { id: 1 }, { id: 2 }];
    var thenables = markers.map(function (_, index) {
        return {
            then: function (resolve) {
                callbacks[index] = resolve;
            }
        };
    });
    var ordered = Promise.all(thenables);

    return Promise.resolve()
        .then(function () {
            same(
                emptyOrder.join("|"),
                "after-call|fulfilled:0",
                "empty ordering"
            );
            same(callbacks.length, 3, "all callbacks captured");
            callbacks[2](markers[2]);
            callbacks[0](markers[0]);
            callbacks[1](markers[1]);
            return ordered;
        })
        .then(function (values) {
            same(values[0], markers[0], "ordered value zero");
            same(values[1], markers[1], "ordered value one");
            same(values[2], markers[2], "ordered value two");
            emit(
                "empty-order",
                emptyOrder.join("|") + "||" +
                    values.map(function (value) { return value.id; }).join(",")
            );
        });
}

function checkSynchronousThenSentinelAndDuplicate() {
    var log = [];
    var firstCallbacks = [];
    var finalReject;

    function Custom(executor) {
        var result = { state: "pending" };
        executor(
            function (values) {
                log.push("final:" + values.join(","));
                result.state = "fulfilled";
                result.value = values;
            },
            finalReject = function (reason) {
                log.push("reject:" + reason);
                result.state = "rejected";
            }
        );
        return result;
    }
    Custom.resolve = function (value) {
        log.push("resolve:" + value);
        return {
            then: function (onFulfilled, onRejected) {
                firstCallbacks.push(onFulfilled);
                same(onRejected, finalReject, "sync reject identity");
                log.push("then:" + value);
                onFulfilled(value);
                onFulfilled("duplicate-" + value);
            }
        };
    };

    var nextIndex = 0;
    var iterable = {};
    iterable[Symbol.iterator] = function () {
        return {
            next: function () {
                if (nextIndex < 2) {
                    log.push("next:" + nextIndex);
                    return { value: 40 + nextIndex++ * 2, done: false };
                }
                log.push("done");
                return { done: true };
            }
        };
    };

    var result = Promise.all.call(Custom, iterable);
    same(result.state, "fulfilled", "sync custom result state");
    same(result.value.join(","), "40,42", "duplicate callback ignored");
    same(firstCallbacks[0] === firstCallbacks[1], false, "sync callbacks fresh");
    same(
        log.join("|"),
        "next:0|resolve:40|then:40|next:1|resolve:42|then:42|done|final:40,42",
        "remaining sentinel ordering"
    );
    emit("sync-sentinel", log.join("|"));
}

function checkResolveGetterOnce() {
    var log = [];
    function Constructor(executor) {
        return new Promise(executor);
    }
    Object.defineProperty(Constructor, "resolve", {
        configurable: true,
        get: function () {
            log.push("resolve-get");
            return function (value) {
                log.push("resolve-call:" + value);
                return Promise.resolve(value);
            };
        }
    });
    var iterable = {};
    iterable[Symbol.iterator] = function () {
        log.push("iterator");
        var index = 0;
        return {
            next: function () {
                if (index < 2)
                    return { value: ++index, done: false };
                return { done: true };
            }
        };
    };

    return Promise.all.call(Constructor, iterable).then(function (values) {
        same(values.join(","), "1,2", "resolve getter values");
        same(
            log.join("|"),
            "resolve-get|iterator|resolve-call:1|resolve-call:2",
            "resolve getter once and ordering"
        );
        emit("resolve-once", log.join("|"));
    });
}

function closingIterable(log, item) {
    var iterable = {};
    iterable[Symbol.iterator] = function () {
        var yielded = false;
        return {
            next: function () {
                log.push("next");
                if (!yielded) {
                    yielded = true;
                    return { value: item, done: false };
                }
                return { done: true };
            },
            return: function () {
                log.push("close");
                throw { close: true };
            }
        };
    };
    return iterable;
}

function checkIteratorNoCloseMatrix() {
    function run(label, makeIterable, expectedLog) {
        var reason = {};
        var log = [];
        return Promise.all(makeIterable(log, reason)).then(
            function () {
                fail(label, "fulfilled", "rejected");
            },
            function (actual) {
                same(actual, reason, label + " reason identity");
                same(log.join("|"), expectedLog, label + " no-close log");
                return label + ":" + log.join("|");
            }
        );
    }

    return run(
        "next-get",
        function (log, reason) {
            var iterable = {};
            iterable[Symbol.iterator] = function () {
                var iterator = {
                    return: function () {
                        log.push("close");
                    }
                };
                Object.defineProperty(iterator, "next", {
                    get: function () {
                        log.push("next-get");
                        throw reason;
                    }
                });
                return iterator;
            };
            return iterable;
        },
        "next-get"
    ).then(function (first) {
        return run(
            "next-call",
            function (log, reason) {
                var iterable = {};
                iterable[Symbol.iterator] = function () {
                    return {
                        next: function () {
                            log.push("next-call");
                            throw reason;
                        },
                        return: function () {
                            log.push("close");
                        }
                    };
                };
                return iterable;
            },
            "next-call"
        ).then(function (second) {
            emit("iterator-no-close", first + "||" + second);
        });
    });
}

function checkIteratorCloseMatrix() {
    function run(label, configure, expectedLog) {
        var reason = {};
        var log = [];
        function Constructor(executor) {
            return new Promise(executor);
        }
        configure(Constructor, log, reason);
        return Promise.all.call(
            Constructor,
            closingIterable(log, label)
        ).then(
            function () {
                fail(label, "fulfilled", "rejected");
            },
            function (actual) {
                same(actual, reason, label + " original reason identity");
                same(log.join("|"), expectedLog, label + " close log");
                return label + ":" + log.join("|") + ":original";
            }
        );
    }

    return run(
        "resolve-throw",
        function (Constructor, log, reason) {
            Constructor.resolve = function () {
                log.push("resolve");
                throw reason;
            };
        },
        "next|resolve|close"
    ).then(function (first) {
        return run(
            "then-get-throw",
            function (Constructor, log, reason) {
                Constructor.resolve = function () {
                    log.push("resolve");
                    var poisoned = {};
                    Object.defineProperty(poisoned, "then", {
                        get: function () {
                            log.push("then-get");
                            throw reason;
                        }
                    });
                    return poisoned;
                };
            },
            "next|resolve|then-get|close"
        ).then(function (second) {
            emit("iterator-close", first + "||" + second);
        });
    });
}

function checkRejectContinuesIteration() {
    var reason = {};
    var log = [];
    function Constructor(executor) {
        return new Promise(executor);
    }
    Constructor.resolve = function (value) {
        log.push("resolve:" + value);
        return {
            then: function (onFulfilled, onRejected) {
                log.push("then:" + value);
                if (value === 0)
                    onRejected(reason);
                else
                    onFulfilled(value);
            }
        };
    };
    var index = 0;
    var iterable = {};
    iterable[Symbol.iterator] = function () {
        return {
            next: function () {
                if (index < 3) {
                    log.push("next:" + index);
                    return { value: index++, done: false };
                }
                log.push("done");
                return { done: true };
            }
        };
    };

    return Promise.all.call(Constructor, iterable).then(
        function () {
            fail("reject continues", "fulfilled", "rejected");
        },
        function (actual) {
            same(actual, reason, "reject continues reason");
            same(
                log.join("|"),
                "next:0|resolve:0|then:0|next:1|resolve:1|then:1|" +
                    "next:2|resolve:2|then:2|done",
                "reject continues full iteration"
            );
            emit("reject-continues", log.join("|"));
        }
    );
}

function checkThenablePoisonAndIdentities() {
    var object = { answer: 42 };
    var symbol = Symbol("r3p");
    var poisonedReason = {};
    var thenable = {
        then: function (resolve) {
            resolve(object);
        }
    };
    var poisoned = {};
    Object.defineProperty(poisoned, "then", {
        get: function () {
            throw poisonedReason;
        }
    });

    return Promise.all([thenable, symbol])
        .then(function (values) {
            same(values[0], object, "thenable object identity");
            same(values[1], symbol, "Symbol identity");
            return Promise.all([poisoned]);
        })
        .then(
            function () {
                fail("poisoned thenable", "fulfilled", "rejected");
            },
            function (reason) {
                same(reason, poisonedReason, "poisoned then reason identity");
                emit("thenable-identities", "object:true|symbol:true|poison:true");
            }
        );
}

function checkCallbackGraphSurvivesGc() {
    var held = [];
    var object = { answer: 42 };
    var symbol = Symbol("r3p-gc");
    var objectIdentity = object;
    var symbolIdentity = symbol;

    function Constructor(executor) {
        var result = { state: "pending" };
        executor(
            function (values) {
                result.state = "fulfilled";
                result.values = values;
            },
            function (reason) {
                result.state = "rejected";
                result.reason = reason;
            }
        );
        return result;
    }
    Constructor.resolve = function (value) {
        return {
            then: function (onFulfilled) {
                held.push({
                    callback: onFulfilled,
                    value: value
                });
            }
        };
    };

    var result = Promise.all.call(Constructor, [object, symbol]);
    same(result.state, "pending", "GC graph initial state");
    object = null;
    symbol = null;
    forceOracleGc();

    held[1].callback(held[1].value);
    held[1] = null;
    forceOracleGc();
    held[0].callback(held[0].value);
    held[0] = null;
    forceOracleGc();

    same(result.state, "fulfilled", "GC graph final state");
    same(result.values[0], objectIdentity, "GC graph object identity");
    same(result.values[1], symbolIdentity, "GC graph Symbol identity");
    emit("callback-gc", "state:" + result.state + "|object:true|symbol:true");
}

var r3pDone = Promise.resolve()
    .then(checkDescriptorAndGenericErrors)
    .then(checkCustomConstructorAndCallbackShape)
    .then(checkEmptyAndOutOfOrder)
    .then(checkSynchronousThenSentinelAndDuplicate)
    .then(checkResolveGetterOnce)
    .then(checkIteratorNoCloseMatrix)
    .then(checkIteratorCloseMatrix)
    .then(checkRejectContinuesIteration)
    .then(checkThenablePoisonAndIdentities)
    .then(checkCallbackGraphSurvivesGc)
    .then(function () {
        emit("r3p-promise-all-oracle", "ok");
    });

forceOracleGc();
r3pDone;
