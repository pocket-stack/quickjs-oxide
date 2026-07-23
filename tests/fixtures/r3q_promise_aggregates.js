function fail(label, actual, expected) {
    throw new Error(
        "R3q Promise aggregate oracle assertion failed for " + label +
        ": expected " + expected + ", got " + actual
    );
}

function same(actual, expected, label) {
    if (actual !== expected)
        fail(label, String(actual), String(expected));
}

var r3qTranscript = [];
function emit(label, value) {
    r3qTranscript.push(label + "=" + value);
}

function forceOracleGc() {
    if (typeof std === "object" && typeof std.gc === "function")
        std.gc();
}

function methodFacts(name) {
    var method = Promise[name];
    var descriptor = Object.getOwnPropertyDescriptor(Promise, name);
    same(typeof method, "function", name + " type");
    same(method.name, name, name + " name");
    same(method.length, 1, name + " length");
    same(descriptor.writable, true, name + " writable");
    same(descriptor.enumerable, false, name + " enumerable");
    same(descriptor.configurable, true, name + " configurable");
    same(
        Reflect.ownKeys(method).map(String).join(","),
        "length,name",
        name + " own keys"
    );
    same(
        Object.prototype.hasOwnProperty.call(method, "prototype"),
        false,
        name + " no prototype"
    );
    return name + ":" + method.length + ":" +
        descriptor.writable + descriptor.enumerable + descriptor.configurable;
}

function checkDescriptors() {
    emit(
        "descriptors",
        methodFacts("allSettled") + "|" + methodFacts("any")
    );
}

function checkAllSettledCustomAndQuickJsOverwrite() {
    var log = [];
    var outerResolve;
    var outerReject;
    var fulfillCallbacks = [];
    var rejectCallbacks = [];

    function Custom(executor) {
        var result = { state: "pending" };
        outerResolve = function (value) {
            result.state = "fulfilled";
            result.value = value;
            log.push("outer-resolve");
        };
        outerReject = function (reason) {
            result.state = "rejected";
            result.reason = reason;
            log.push("outer-reject");
        };
        executor(outerResolve, outerReject);
        return result;
    }
    Custom.resolve = function (value) {
        log.push("resolve:" + value);
        return {
            then: function (onFulfilled, onRejected) {
                fulfillCallbacks.push(onFulfilled);
                rejectCallbacks.push(onRejected);
                log.push("then:" + value);
                onFulfilled("fulfilled-" + value);
                onRejected("rejected-" + value);
            }
        };
    };

    var result = Promise.allSettled.call(Custom, [20, 22]);
    same(result.state, "fulfilled", "allSettled custom state");
    same(result.value.length, 2, "allSettled custom result length");
    same(result.value[0].status, "rejected", "QuickJS overwrite zero");
    same(result.value[0].reason, "rejected-20", "QuickJS overwrite reason zero");
    same(result.value[1].status, "rejected", "QuickJS overwrite one");
    same(result.value[1].reason, "rejected-22", "QuickJS overwrite reason one");
    same(fulfillCallbacks.length, 2, "allSettled fulfill callback count");
    same(rejectCallbacks.length, 2, "allSettled reject callback count");
    same(fulfillCallbacks[0] === fulfillCallbacks[1], false, "fresh fulfill");
    same(rejectCallbacks[0] === rejectCallbacks[1], false, "fresh reject");
    same(fulfillCallbacks[0] === outerResolve, false, "fulfill not outer");
    same(rejectCallbacks[0] === outerReject, false, "reject not outer");
    same(fulfillCallbacks[0].name, "", "allSettled callback name");
    same(fulfillCallbacks[0].length, 1, "allSettled callback length");
    same(
        Reflect.ownKeys(fulfillCallbacks[0]).map(String).join(","),
        "length,name",
        "allSettled callback keys"
    );

    emit("allSettled-custom-overwrite", [
        result.value.map(function (entry) {
            return entry.status + ":" + entry.reason;
        }).join(","),
        fulfillCallbacks[0] === fulfillCallbacks[1],
        rejectCallbacks[0] === rejectCallbacks[1],
        fulfillCallbacks[0].name + ":" + fulfillCallbacks[0].length,
        log.join("|")
    ].join("||"));
}

function checkAllSettledResultObjects() {
    var object = { answer: 42 };
    var reason = { reason: true };
    return Promise.allSettled([
        object,
        Promise.reject(reason)
    ]).then(function (results) {
        var fulfilled = results[0];
        var rejected = results[1];
        same(Object.getPrototypeOf(results), Array.prototype, "results Array realm");
        same(Object.getPrototypeOf(fulfilled), Object.prototype, "fulfilled realm");
        same(Object.getPrototypeOf(rejected), Object.prototype, "rejected realm");
        same(Reflect.ownKeys(fulfilled).map(String).join(","), "status,value",
            "fulfilled own-key order");
        same(Reflect.ownKeys(rejected).map(String).join(","), "status,reason",
            "rejected own-key order");
        same(fulfilled.value, object, "fulfilled object identity");
        same(rejected.reason, reason, "rejected object identity");
        var statusDescriptor = Object.getOwnPropertyDescriptor(fulfilled, "status");
        var valueDescriptor = Object.getOwnPropertyDescriptor(fulfilled, "value");
        same(
            [
                statusDescriptor.writable,
                statusDescriptor.enumerable,
                statusDescriptor.configurable,
                valueDescriptor.writable,
                valueDescriptor.enumerable,
                valueDescriptor.configurable
            ].join(","),
            "true,true,true,true,true,true",
            "result property descriptors"
        );
        emit(
            "allSettled-results",
            Reflect.ownKeys(fulfilled).map(String).join(",") + "|" +
                Reflect.ownKeys(rejected).map(String).join(",") + "|" +
                "object:true|reason:true|descriptors:true"
        );
    });
}

function checkAnyCustomIdentityAndInputOrder() {
    var outerResolve;
    var outerReject;
    var fulfillCallbacks = [];
    var rejectCallbacks = [];
    var reasons = [{ index: 0 }, { index: 1 }, { index: 2 }];

    function Custom(executor) {
        var result = { state: "pending" };
        outerResolve = function (value) {
            result.state = "fulfilled";
            result.value = value;
        };
        outerReject = function (reason) {
            result.state = "rejected";
            result.reason = reason;
        };
        executor(outerResolve, outerReject);
        return result;
    }
    Custom.resolve = function () {
        return {
            then: function (onFulfilled, onRejected) {
                fulfillCallbacks.push(onFulfilled);
                rejectCallbacks.push(onRejected);
            }
        };
    };

    var result = Promise.any.call(Custom, [0, 1, 2]);
    same(result.state, "pending", "any custom initially pending");
    same(fulfillCallbacks[0], outerResolve, "any first fulfill is outer resolve");
    same(fulfillCallbacks[1], outerResolve, "any second fulfill is outer resolve");
    same(fulfillCallbacks[2], outerResolve, "any third fulfill is outer resolve");
    same(rejectCallbacks[0] === rejectCallbacks[1], false, "any rejects fresh");
    same(rejectCallbacks[1] === rejectCallbacks[2], false, "any rejects fresh two");
    same(rejectCallbacks[0].name, "", "any reject callback name");
    same(rejectCallbacks[0].length, 1, "any reject callback length");

    rejectCallbacks[2](reasons[2]);
    rejectCallbacks[2]({ duplicate: true });
    rejectCallbacks[0](reasons[0]);
    same(result.state, "pending", "any waits for every rejection");
    rejectCallbacks[1](reasons[1]);
    same(result.state, "rejected", "any custom rejected");
    same(result.reason instanceof AggregateError, true, "any AggregateError brand");
    same(result.reason.errors[0], reasons[0], "any input order zero");
    same(result.reason.errors[1], reasons[1], "any input order one");
    same(result.reason.errors[2], reasons[2], "any input order two");
    same(result.reason.errors.length, 3, "any errors prefilled length");
    same(
        Reflect.ownKeys(result.reason).map(String).join(","),
        "errors",
        "internal AggregateError own keys"
    );
    same(result.reason.errors, result.reason.errors, "errors stable identity");
    same(
        Object.getPrototypeOf(result.reason.errors),
        Array.prototype,
        "errors Array realm"
    );

    emit("any-custom-order", [
        fulfillCallbacks[0] === outerResolve &&
            fulfillCallbacks[1] === outerResolve &&
            fulfillCallbacks[2] === outerResolve,
        rejectCallbacks[0] === rejectCallbacks[1],
        rejectCallbacks[0].name + ":" + rejectCallbacks[0].length,
        result.reason.errors.map(function (reason) {
            return reason.index;
        }).join(","),
        Reflect.ownKeys(result.reason).map(String).join(","),
        Reflect.ownKeys(result.reason.errors).map(String).join(",")
    ].join("||"));
}

function checkEmptyAggregates() {
    var order = [];
    var settled = Promise.allSettled([]).then(function (values) {
        same(values.length, 0, "empty allSettled length");
        same(Object.getPrototypeOf(values), Array.prototype, "empty allSettled realm");
        order.push("allSettled:" + values.length);
    });
    var any = Promise.any([]).then(
        function () {
            fail("empty any", "fulfilled", "rejected");
        },
        function (error) {
            same(error instanceof AggregateError, true, "empty any brand");
            same(error.message, "", "empty any message");
            same(error.errors.length, 0, "empty any errors");
            same(Reflect.ownKeys(error).map(String).join(","), "errors",
                "empty any own keys");
            order.push("any:" + error.errors.length);
        }
    );
    order.push("after-call");
    return Promise.all([settled, any]).then(function () {
        same(
            order.join("|"),
            "after-call|allSettled:0|any:0",
            "empty aggregate ordering"
        );
        emit("empty", order.join("|"));
    });
}

function checkResolveGettersOnce() {
    function run(name) {
        var log = [];
        function Constructor(executor) {
            return new Promise(executor);
        }
        Object.defineProperty(Constructor, "resolve", {
            configurable: true,
            get: function () {
                log.push("resolve-get");
                return function (value) {
                    log.push("resolve:" + value);
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
        return Promise[name].call(Constructor, iterable).then(function (value) {
            same(
                log.join("|"),
                "resolve-get|iterator|resolve:1|resolve:2",
                name + " resolve getter once"
            );
            return name + ":" + log.join("|") + ":" +
                (name === "any" ? value : value.length);
        });
    }

    return run("allSettled").then(function (first) {
        return run("any").then(function (second) {
            emit("resolve-getters", first + "||" + second);
        });
    });
}

function checkIteratorNoClose() {
    function run(name, kind) {
        var reason = {};
        var log = [];
        var iterable = {};
        iterable[Symbol.iterator] = function () {
            if (kind === "get") {
                var iterator = {
                    return: function () { log.push("close"); }
                };
                Object.defineProperty(iterator, "next", {
                    get: function () {
                        log.push("next-get");
                        throw reason;
                    }
                });
                return iterator;
            }
            return {
                next: function () {
                    log.push("next-call");
                    throw reason;
                },
                return: function () { log.push("close"); }
            };
        };
        return Promise[name](iterable).then(
            function () {
                fail(name + " no-close " + kind, "fulfilled", "rejected");
            },
            function (actual) {
                same(actual, reason, name + " no-close reason");
                same(
                    log.join("|"),
                    kind === "get" ? "next-get" : "next-call",
                    name + " no-close log"
                );
                return name + ":" + kind + ":" + log.join("|");
            }
        );
    }

    return run("allSettled", "get").then(function (a) {
        return run("allSettled", "call").then(function (b) {
            return run("any", "get").then(function (c) {
                return run("any", "call").then(function (d) {
                    emit("iterator-no-close", [a, b, c, d].join("||"));
                });
            });
        });
    });
}

function closingIterable(log) {
    var iterable = {};
    iterable[Symbol.iterator] = function () {
        var yielded = false;
        return {
            next: function () {
                log.push("next");
                if (!yielded) {
                    yielded = true;
                    return { value: 42, done: false };
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

function checkIteratorClose() {
    function run(name, kind) {
        var reason = {};
        var log = [];
        function Constructor(executor) {
            return new Promise(executor);
        }
        if (kind === "resolve") {
            Constructor.resolve = function () {
                log.push("resolve");
                throw reason;
            };
        } else {
            Constructor.resolve = function () {
                log.push("resolve");
                var value = {};
                Object.defineProperty(value, "then", {
                    get: function () {
                        log.push("then-get");
                        throw reason;
                    }
                });
                return value;
            };
        }
        return Promise[name].call(Constructor, closingIterable(log)).then(
            function () {
                fail(name + " close " + kind, "fulfilled", "rejected");
            },
            function (actual) {
                same(actual, reason, name + " close preserves original");
                same(
                    log.join("|"),
                    kind === "resolve"
                        ? "next|resolve|close"
                        : "next|resolve|then-get|close",
                    name + " close log"
                );
                return name + ":" + kind + ":" + log.join("|");
            }
        );
    }

    return run("allSettled", "resolve").then(function (a) {
        return run("allSettled", "then").then(function (b) {
            return run("any", "resolve").then(function (c) {
                return run("any", "then").then(function (d) {
                    emit("iterator-close", [a, b, c, d].join("||"));
                });
            });
        });
    });
}

function checkSynchronousSettlementContinuesIteration() {
    function run(name) {
        var log = [];
        var index = 0;
        function Constructor(executor) {
            var result = { state: "pending" };
            executor(
                function (value) {
                    result.state = "fulfilled";
                    result.value = value;
                    log.push("outer-resolve");
                },
                function (reason) {
                    result.state = "rejected";
                    result.reason = reason;
                    log.push("outer-reject");
                }
            );
            return result;
        }
        Constructor.resolve = function (value) {
            log.push("resolve:" + value);
            return {
                then: function (onFulfilled, onRejected) {
                    log.push("then:" + value);
                    if (name === "any")
                        onFulfilled(value);
                    else
                        onRejected(value);
                    if (name === "allSettled")
                        onRejected("duplicate-" + value);
                }
            };
        };
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
        var result = Promise[name].call(Constructor, iterable);
        same(result.state, "fulfilled", name + " sync state");
        same(
            log.join("|"),
            name === "any"
                ? "next:0|resolve:0|then:0|outer-resolve|" +
                    "next:1|resolve:1|then:1|outer-resolve|" +
                    "next:2|resolve:2|then:2|outer-resolve|done"
                : "next:0|resolve:0|then:0|next:1|resolve:1|then:1|" +
                    "next:2|resolve:2|then:2|done|outer-resolve",
            name + " continues iteration"
        );
        if (name === "allSettled") {
            same(result.value[0].reason, 0, "same callback duplicate ignored zero");
            same(result.value[1].reason, 1, "same callback duplicate ignored one");
            same(result.value[2].reason, 2, "same callback duplicate ignored two");
        }
        return name + ":" + log.join("|");
    }

    emit("sync-continues", run("allSettled") + "||" + run("any"));
}

function checkFrozenAllSettledValuesContinueSilently() {
    var log = [];
    var index = 0;

    function Constructor(executor) {
        var result = {};
        executor(
            function (values) {
                log.push("resolve:" + Reflect.ownKeys(values).map(String).join(","));
                Object.freeze(values);
                result.values = values;
            },
            function (reason) {
                log.push("reject:" + reason.name);
                result.reason = reason;
            }
        );
        return result;
    }
    Constructor.resolve = function (value) {
        return {
            then: function (onFulfilled, onRejected) {
                log.push("then:" + value);
                onFulfilled(value);
                onRejected(value);
            }
        };
    };

    var iterable = {};
    iterable[Symbol.iterator] = function () {
        return {
            next: function () {
                log.push("next:" + index);
                if (index < 2)
                    return { value: index++, done: false };
                return { done: true };
            },
            return: function () {
                log.push("close");
                return { done: true };
            }
        };
    };

    var result = Promise.allSettled.call(Constructor, iterable);
    same(
        log.join("|"),
        "next:0|then:0|resolve:0,length|" +
            "next:1|then:1|resolve:0,length|next:2",
        "frozen values silently continue"
    );
    same(result.reason, undefined, "frozen values do not call outer reject");
    same(result.values.length, 1, "frozen values retain the first record only");
    same(result.values[0].status, "rejected", "frozen first record overwrite");
    same(
        Reflect.ownKeys(result.values).map(String).join(","),
        "0,length",
        "frozen values own keys"
    );
    emit("allSettled-frozen-values", log.join("|") + "||reject:false");
}

function checkThenablesSymbolsAndObjects() {
    var object = { answer: 42 };
    var reason = { rejected: true };
    var symbol = Symbol("r3q");
    var fulfilledThenable = {
        then: function (resolve) {
            resolve(object);
        }
    };
    var rejectedThenable = {
        then: function (_, reject) {
            reject(reason);
        }
    };

    return Promise.allSettled([
        fulfilledThenable,
        rejectedThenable,
        symbol
    ]).then(function (results) {
        same(results[0].value, object, "thenable object identity");
        same(results[1].reason, reason, "thenable reason identity");
        same(results[2].value, symbol, "allSettled Symbol identity");
        return Promise.any([rejectedThenable, fulfilledThenable, symbol]);
    }).then(function (value) {
        same(value, symbol, "any job-order Symbol identity");
        emit(
            "thenable-identities",
            "object:true|reason:true|symbol:true|any-symbol:true"
        );
    });
}

function checkCallbackGraphsSurviveGc() {
    var allHeld = [];
    var anyHeld = [];
    var object = { answer: 42 };
    var symbol = Symbol("r3q-gc");

    function AllConstructor(executor) {
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
    AllConstructor.resolve = function (value) {
        return {
            then: function (onFulfilled, onRejected) {
                allHeld.push({
                    fulfill: onFulfilled,
                    reject: onRejected,
                    value: value
                });
            }
        };
    };

    function AnyConstructor(executor) {
        var result = { state: "pending" };
        executor(
            function (value) {
                result.state = "fulfilled";
                result.value = value;
            },
            function (reason) {
                result.state = "rejected";
                result.reason = reason;
            }
        );
        return result;
    }
    AnyConstructor.resolve = function (value) {
        return {
            then: function (onFulfilled, onRejected) {
                anyHeld.push({
                    fulfill: onFulfilled,
                    reject: onRejected,
                    value: value
                });
            }
        };
    };

    var allResult = Promise.allSettled.call(AllConstructor, [object, symbol]);
    var anyResult = Promise.any.call(AnyConstructor, [object, symbol]);
    forceOracleGc();
    allHeld[1].reject(symbol);
    forceOracleGc();
    allHeld[0].fulfill(object);
    anyHeld[0].reject(object);
    forceOracleGc();
    anyHeld[1].fulfill(symbol);
    forceOracleGc();

    same(allResult.state, "fulfilled", "GC allSettled state");
    same(allResult.values[0].value, object, "GC allSettled object");
    same(allResult.values[1].reason, symbol, "GC allSettled Symbol");
    same(anyResult.state, "fulfilled", "GC any state");
    same(anyResult.value, symbol, "GC any Symbol");
    emit("gc", "allSettled:true|any:true|object:true|symbol:true");
}

checkDescriptors();
checkAllSettledCustomAndQuickJsOverwrite();
checkAnyCustomIdentityAndInputOrder();
checkSynchronousSettlementContinuesIteration();
checkFrozenAllSettledValuesContinueSilently();
checkCallbackGraphsSurviveGc();

var r3qDone = checkAllSettledResultObjects()
    .then(checkEmptyAggregates)
    .then(checkResolveGettersOnce)
    .then(checkIteratorNoClose)
    .then(checkIteratorClose)
    .then(checkThenablesSymbolsAndObjects);

r3qDone;
