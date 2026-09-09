function fail(label, actual, expected) {
    throw new Error(
        "R3o Promise.finally oracle assertion failed for " + label +
        ": expected " + expected + ", got " + actual
    );
}

function same(actual, expected, label) {
    if (actual !== expected)
        fail(label, String(actual), String(expected));
}

var r3oTranscript = [];
function emit(label, value) {
    r3oTranscript.push(label + "=" + value);
}

function forceOracleGc() {
    if (typeof std === "object" && typeof std.gc === "function")
        std.gc();
}

function checkDescriptorAndGenericReceiver() {
    var method = Promise.prototype.finally;
    var descriptor = Object.getOwnPropertyDescriptor(
        Promise.prototype,
        "finally"
    );
    same(typeof method, "function", "finally type");
    same(method.name, "finally", "finally name");
    same(method.length, 1, "finally length");
    same(descriptor.writable, true, "finally writable");
    same(descriptor.enumerable, false, "finally enumerable");
    same(descriptor.configurable, true, "finally configurable");
    same(
        Object.getOwnPropertyNames(method).join(","),
        "length,name",
        "finally own function keys"
    );
    same(
        Object.prototype.hasOwnProperty.call(method, "prototype"),
        false,
        "finally has no prototype"
    );

    var marker = {};
    var genericOrder = [];
    var receiver = {
        constructor: undefined,
        then: function (onFulfilled, onRejected) {
            genericOrder.push("then");
            same(onFulfilled, marker, "non-callable generic fulfill argument");
            same(onRejected, marker, "non-callable generic reject argument");
            return 42;
        }
    };
    var genericResult = method.call(receiver, marker);
    genericOrder.push("after");
    same(genericResult, 42, "generic finally return value");
    same(genericOrder.join("|"), "then|after", "generic then is synchronous");

    function GenericSpecies(executor) {
        return new Promise(executor);
    }
    var capturedFulfill;
    var capturedReject;
    var onFinallyCalls = 0;
    var callableReceiver = {
        constructor: {
            [Symbol.species]: GenericSpecies
        },
        then: function (onFulfilled, onRejected) {
            capturedFulfill = onFulfilled;
            capturedReject = onRejected;
            return "generic-callable-result";
        }
    };
    var callableResult = method.call(callableReceiver, function () {
        onFinallyCalls++;
    });
    same(callableResult, "generic-callable-result", "callable generic result");
    same(onFinallyCalls, 0, "generic receiver does not invoke handlers itself");
    same(capturedFulfill.length, 1, "finally fulfill wrapper length");
    same(capturedReject.length, 1, "finally reject wrapper length");
    same(capturedFulfill.name, "", "finally fulfill wrapper name");
    same(capturedReject.name, "", "finally reject wrapper name");
    same(
        Object.getPrototypeOf(capturedFulfill),
        Function.prototype,
        "finally wrapper prototype"
    );

    emit("descriptor-generic", [
        method.name + ":" + method.length + ":" +
            descriptor.writable + descriptor.enumerable + descriptor.configurable,
        genericOrder.join("|") + ":" + genericResult,
        typeof capturedFulfill + ":" + capturedFulfill.length + ":" +
            capturedFulfill.name,
        typeof capturedReject + ":" + capturedReject.length + ":" +
            capturedReject.name
    ].join("||"));
}

function checkSpeciesAbruptPrecedesThen() {
    var marker = {};
    var order = [];
    var constructor = {};
    Object.defineProperty(constructor, Symbol.species, {
        get: function () {
            order.push("species-get");
            throw marker;
        }
    });
    var receiver = {};
    Object.defineProperty(receiver, "constructor", {
        get: function () {
            order.push("constructor-get");
            return constructor;
        }
    });
    Object.defineProperty(receiver, "then", {
        get: function () {
            order.push("then-get");
            return function () {};
        }
    });
    var callbackCalls = 0;
    try {
        Promise.prototype.finally.call(receiver, function () {
            callbackCalls++;
        });
        fail("throwing species getter", "returned", "threw");
    } catch (error) {
        same(error, marker, "throwing species getter identity");
    }
    same(callbackCalls, 0, "species abrupt precedes callable handler");
    same(
        order.join("|"),
        "constructor-get|species-get",
        "species abrupt precedes then getter"
    );
    emit("species-abrupt", order.join("|") + "|callback:" + callbackCalls);
}

function checkNonCallablePassThrough() {
    var marker = {};
    var reason = {};

    return Promise.resolve(marker)
        .finally(null)
        .then(function (value) {
            same(value, marker, "non-callable fulfilled pass-through");
            return Promise.reject(reason).finally(0);
        })
        .then(
            function () {
                fail("non-callable rejected pass-through", "fulfilled", "rejected");
            },
            function (actual) {
                same(actual, reason, "non-callable rejection identity");
                emit("non-callable", "fulfilled:true|rejected:true");
            }
        );
}

function checkUndefinedConstructorCallableFinally() {
    var order = [];
    var source = Promise.resolve(42);
    source.constructor = undefined;
    var result = source.finally(function () {
        order.push("onFinally");
        return 7;
    });
    order.push("after-register");

    return result.then(
        function () {
            fail("undefined constructor callable finally", "fulfilled", "rejected");
        },
        function (error) {
            same(error.name, "TypeError", "undefined constructor finally error");
            order.push("rejected:" + error.name);
            same(
                order.join("|"),
                "after-register|onFinally|rejected:TypeError",
                "undefined constructor finally ordering"
            );
            emit("undefined-constructor", order.join("|"));
        }
    );
}

function checkSpeciesAndInternalPromiseResolve() {
    var constructorCalls = 0;
    var resolveGetterCalls = 0;
    var order = [];

    class FinallyPromise extends Promise {
        constructor(executor) {
            constructorCalls++;
            super(executor);
        }

        static get [Symbol.species]() {
            return Promise;
        }
    }
    Object.defineProperty(FinallyPromise, "resolve", {
        configurable: true,
        get: function () {
            resolveGetterCalls++;
            throw new Error("Finally must not read constructor.resolve");
        }
    });

    var source = Promise.resolve(42);
    source.constructor = {
        [Symbol.species]: FinallyPromise
    };
    var result = source.finally(function () {
        order.push("onFinally");
        return 7;
    });
    same(result instanceof FinallyPromise, true, "finally species result");
    same(constructorCalls, 1, "outer species capability construction");
    order.push("after-register");

    return result.then(function (value) {
        same(value, 42, "species finally preserves original value");
        same(constructorCalls, 2, "internal PromiseResolve construction");
        same(resolveGetterCalls, 0, "internal PromiseResolve skips resolve getter");
        same(
            order.join("|"),
            "after-register|onFinally",
            "species finally ordering"
        );
        emit("species-resolve", [
            result instanceof FinallyPromise,
            constructorCalls,
            resolveGetterCalls,
            order.join("|"),
            value
        ].join("|"));
    });
}

function checkOnFinallyReceiverArgumentsAndReturn() {
    var order = [];
    var originalReason = {};

    var fulfilled = Promise.resolve(40).finally(function () {
        "use strict";
        same(this, undefined, "onFinally fulfilled this");
        same(arguments.length, 0, "onFinally fulfilled argc");
        order.push("fulfilled-cleanup");
        return 99;
    });
    var rejected = Promise.reject(originalReason).finally(function () {
        "use strict";
        same(this, undefined, "onFinally rejected this");
        same(arguments.length, 0, "onFinally rejected argc");
        order.push("rejected-cleanup");
        return 100;
    });
    order.push("after-register");

    return fulfilled
        .then(function (value) {
            same(value, 40, "fulfilled cleanup return is ignored");
            order.push("fulfilled:" + value);
            return rejected;
        })
        .then(
            function () {
                fail("rejected cleanup return", "fulfilled", "rejected");
            },
            function (reason) {
                same(reason, originalReason, "rejected cleanup preserves reason");
                order.push("rejected:true");
                emit("receiver-args-return", order.join("|"));
            }
        );
}

function checkThenableTiming() {
    var order = [];
    var thenable = {};
    Object.defineProperty(thenable, "then", {
        configurable: true,
        get: function () {
            order.push("then-get");
            return function (resolve, reject) {
                order.push("then-call");
                resolve("cleanup");
                reject("late");
            };
        }
    });

    var result = Promise.resolve("original").finally(function () {
        order.push("onFinally");
        return thenable;
    });
    order.push("after-register");

    return result.then(function (value) {
        same(value, "original", "fulfilled thenable preserves original value");
        order.push("result:" + value);
        same(
            order.join("|"),
            "after-register|onFinally|then-get|then-call|result:original",
            "finally thenable timing"
        );
        emit("thenable-order", order.join("|"));
    });
}

function checkRejectedReturnAndThrowOverride() {
    var cleanupReason = {};
    var originalReason = {};
    var thrownReason = {};
    var order = [];

    return Promise.resolve(1)
        .finally(function () {
            order.push("return-rejected");
            return Promise.reject(cleanupReason);
        })
        .then(
            function () {
                fail("rejected cleanup promise", "fulfilled", "rejected");
            },
            function (reason) {
                same(reason, cleanupReason, "cleanup rejection overrides fulfillment");
                order.push("cleanup-rejected:true");
                return Promise.reject(originalReason).finally(function () {
                    order.push("throw-cleanup");
                    throw thrownReason;
                });
            }
        )
        .then(
            function () {
                fail("throwing cleanup", "fulfilled", "rejected");
            },
            function (reason) {
                same(reason, thrownReason, "cleanup throw overrides original rejection");
                order.push("throw-overrode:true");
                emit("reject-throw", order.join("|"));
            }
        );
}

function checkInnerThenAbrupt() {
    var marker = {};
    var log = [];
    var constructCount = 0;

    function PoisonSpecies(executor) {
        constructCount++;
        var ordinal = constructCount;
        var result = {
            ordinal: ordinal,
            state: "pending"
        };
        log.push("construct:" + ordinal);
        executor(
            function (value) {
                log.push("resolve:" + ordinal + ":" + value);
                result.state = "fulfilled";
                result.value = value;
            },
            function (reason) {
                log.push("reject:" + ordinal + ":" + (reason === marker));
                result.state = "rejected";
                result.reason = reason;
            }
        );
        if (ordinal === 2) {
            Object.defineProperty(result, "then", {
                get: function () {
                    log.push("then-get");
                    throw marker;
                }
            });
        }
        return result;
    }

    var source = Promise.resolve(1);
    source.constructor = {
        [Symbol.species]: PoisonSpecies
    };
    var output = source.finally(function () {
        log.push("onFinally");
        return 7;
    });
    same(output.ordinal, 1, "outer custom species result");
    same(output.state, "pending", "outer custom species starts pending");
    log.push("after-register");

    return Promise.resolve().then(function () {
        same(output.state, "rejected", "inner then abrupt rejects outer capability");
        same(output.reason, marker, "inner then abrupt identity");
        same(
            log.join("|"),
            "construct:1|after-register|onFinally|construct:2|resolve:2:7|" +
                "then-get|reject:1:true",
            "inner dynamic then abrupt ordering"
        );
        emit("then-abrupt", log.join("|"));
    });
}

function checkSymbolThunkAndThrowerSurviveGc() {
    var valueHolder = { symbol: Symbol("finally-value-thunk") };
    var throwHolder = { symbol: Symbol("finally-thrower") };
    var releaseValueCleanup;
    var releaseThrowCleanup;
    var valueCleanup = new Promise(function (resolve) {
        releaseValueCleanup = resolve;
    });
    var throwCleanup = new Promise(function (resolve) {
        releaseThrowCleanup = resolve;
    });
    var valueOutput = Promise.resolve(valueHolder.symbol).finally(function () {
        return valueCleanup;
    });
    var throwOutput = Promise.reject(throwHolder.symbol).finally(function () {
        return throwCleanup;
    });

    // The two source reactions are already ahead of this checkpoint in the
    // FIFO. They create the value thunk and thrower and attach them to the two
    // still-pending cleanup Promises before the forced collection runs.
    return Promise.resolve()
        .then(function () {
            forceOracleGc();
            releaseValueCleanup("value-clean");
            releaseThrowCleanup("throw-clean");
            return valueOutput;
        })
        .then(function (value) {
            same(value, valueHolder.symbol, "value thunk retained Symbol identity");
            return throwOutput.then(
                function () {
                    fail("Symbol thrower", "fulfilled", "rejected");
                },
                function (reason) {
                    same(
                        reason,
                        throwHolder.symbol,
                        "thrower retained Symbol identity"
                    );
                    return true;
                }
            );
        })
        .then(function (throwerMatched) {
            same(throwerMatched, true, "Symbol thrower completion");
            emit("symbol-thunk-thrower-gc", "value:true|thrower:true");
            valueHolder.symbol = null;
            throwHolder.symbol = null;
            valueOutput = null;
            throwOutput = null;
            valueCleanup = null;
            throwCleanup = null;
            forceOracleGc();
        });
}

function checkFinallyGraphSurvivesGc() {
    var gcAnswer = 0;
    var marker = { value: 42 };
    var holder = {
        cleanup: {
            then: function (resolve) {
                resolve("clean");
            }
        }
    };
    var source = Promise.resolve(marker);
    var done = source
        .finally(function () {
            return holder.cleanup;
        })
        .then(function (value) {
            gcAnswer = value.value;
        });

    marker = null;
    source = null;
    forceOracleGc();

    return done.then(function () {
        same(gcAnswer, 42, "finally graph survives GC");
        emit("finally-gc", gcAnswer);
    });
}

var r3oDone = Promise.resolve()
    .then(checkDescriptorAndGenericReceiver)
    .then(checkSpeciesAbruptPrecedesThen)
    .then(checkNonCallablePassThrough)
    .then(checkUndefinedConstructorCallableFinally)
    .then(checkSpeciesAndInternalPromiseResolve)
    .then(checkOnFinallyReceiverArgumentsAndReturn)
    .then(checkThenableTiming)
    .then(checkRejectedReturnAndThrowOverride)
    .then(checkInnerThenAbrupt)
    .then(checkSymbolThunkAndThrowerSurviveGc)
    .then(checkFinallyGraphSurvivesGc)
    .then(function () {
        emit("r3o-promise-finally-oracle", "ok");
    });

forceOracleGc();
r3oDone;
