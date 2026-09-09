var events = [];

function record(value) {
    events.push(value);
}

async function acquisitionAndValueModes() {
    var raw = Promise.resolve(41);
    var count = 0;
    var asyncIterator = {
        label: "async",
        get next() {
            record("acquire:get-next:" + this.label);
            return function () {
                count++;
                record(
                    "acquire:call-next:" +
                    this.label +
                    ":" +
                    arguments.length
                );
                return Promise.resolve({
                    get done() {
                        record("acquire:get-done:" + count);
                        return count > 1;
                    },
                    get value() {
                        record("acquire:get-value:" + count);
                        return count === 1 ? raw : "complete";
                    }
                });
            };
        },
        return: function () {
            record("acquire:return:" + arguments.length);
            return Promise.resolve({});
        }
    };
    var asyncIterable = {
        get [Symbol.asyncIterator]() {
            record("acquire:get-async");
            return function () {
                record("acquire:call-async:" + (this === asyncIterable));
                return asyncIterator;
            };
        },
        get [Symbol.iterator]() {
            record("acquire:sync-miss");
            throw new Error("sync iterator must not be read");
        }
    };
    for await (var value of asyncIterable) {
        record("acquire:raw-value:" + (value === raw));
    }

    var syncCount = 0;
    var syncIterable = {
        get [Symbol.asyncIterator]() {
            record("fallback:get-async-null");
            return null;
        },
        get [Symbol.iterator]() {
            record("fallback:get-sync");
            return function () {
                record("fallback:call-sync:" + (this === syncIterable));
                return {
                    get next() {
                        record("fallback:get-next");
                        return function () {
                            syncCount++;
                            record(
                                "fallback:call-next:" +
                                arguments.length
                            );
                            return {
                                done: syncCount > 1,
                                value: syncCount === 1
                                    ? Promise.resolve(42)
                                    : "complete"
                            };
                        };
                    }
                };
            };
        }
    };
    for await (var fallbackValue of syncIterable) {
        record("fallback:assimilated:" + fallbackValue);
    }
}

async function disabledClosePaths() {
    async function run(label, next) {
        var closeCount = 0;
        var source = {};
        source[Symbol.asyncIterator] = function () {
            return {
                next: next,
                return: function () {
                    closeCount++;
                    record(label + ":return");
                    return {};
                }
            };
        };
        try {
            for await (var value of source) {
                record(label + ":body:" + value);
            }
        } catch (error) {
            record(label + ":catch:" + String(error));
        }
        record(label + ":closed:" + closeCount);
    }

    await run("next-reject", function () {
        record("next-reject:next");
        return Promise.reject("N");
    });
    await run("primitive-result", function () {
        record("primitive-result:next");
        return Promise.resolve(1);
    });
    await run("done-throw", function () {
        record("done-throw:next");
        return Promise.resolve({
            get done() {
                record("done-throw:done");
                throw "D";
            },
            get value() {
                record("done-throw:value-miss");
                return 1;
            }
        });
    });
    await run("value-throw", function () {
        record("value-throw:next");
        return Promise.resolve({
            get done() {
                record("value-throw:done");
                return false;
            },
            get value() {
                record("value-throw:value");
                throw "V";
            }
        });
    });
}

async function loopControlAndClosePrecedence() {
    function make(label, limit, close) {
        var count = 0;
        var iterator = {
            next: function () {
                count++;
                record(label + ":next:" + count);
                return Promise.resolve({
                    done: count > limit,
                    value: count
                });
            }
        };
        Object.defineProperty(iterator, "return", {
            configurable: true,
            get: function () {
                record(label + ":get-return");
                return close;
            }
        });
        var source = {};
        source[Symbol.asyncIterator] = function () {
            return iterator;
        };
        return source;
    }

    var settle;
    var normal = make("normal", 1, function () {
        record("normal:call-return:" + arguments.length);
        return Promise.resolve({}).then(function () {
            record("normal:return-settle");
        });
    });
    for await (var normalValue of normal) {
        record("normal:body:" + normalValue);
    }
    record("normal:after");

    var continuing = make("continue", 2, function () {
        record("continue:call-return");
        return {};
    });
    for await (var continueValue of continuing) {
        record("continue:body:" + continueValue);
        continue;
    }

    var breaking = make("break", 2, function () {
        record("break:call-return:" + arguments.length);
        return new Promise(function (resolve) {
            settle = resolve;
        });
    });
    for await (var breakValue of breaking) {
        record("break:body:" + breakValue);
        break;
    }
    record("break:after");
    Promise.resolve().then(function () {
        record("break:tick");
    });
    await Promise.resolve();
    settle({});
    await Promise.resolve();
    record("break:settled");

    var pendingThrow = make("throw", 1, function () {
        record("throw:call-return");
        throw "close";
    });
    try {
        for await (var throwValue of pendingThrow) {
            record("throw:body:" + throwValue);
            throw "body";
        }
    } catch (error) {
        record("throw:caught:" + error);
    }

    var normalBreak = make("break-error", 1, function () {
        record("break-error:call-return");
        throw "close";
    });
    try {
        for await (var ignored of normalBreak) {
            break;
        }
    } catch (error) {
        record("break-error:caught:" + error);
    }
}

async function bindingAndSourceShapes() {
    var closures = [];
    var target = { value: 0 };
    var key = "value";
    for await (
        let {
            nested: [value = 0],
            ...rest
        } of [
            Promise.resolve({ nested: [19], extra: 1 }),
            Promise.resolve({ nested: [23], extra: 2 })
        ]
    ) {
        closures.push(function () {
            return value + ":" + rest.extra;
        });
        target[key] += value;
    }
    record(
        "binding:" +
        closures[0]() +
        ":" +
        closures[1]() +
        ":" +
        target.value
    );

    var rhs = [1];
    try {
        for await (let rhs of rhs) {
            record("tdz:miss");
        }
    } catch (error) {
        record("tdz:" + error.name);
    }

    var objectMethod = {
        async run(source) {
            var total = 0;
            for await (var value of source) {
                total += value;
            }
            return total;
        }
    };
    class Public {
        static async run(source) {
            var total = 0;
            for await (var value of source) {
                total += value;
            }
            return total;
        }
    }
    record(
        "shapes:" +
        await objectMethod.run([19, 23]) +
        ":" +
        await Public.run([20, 22])
    );
}

async function asyncGeneratorReturnClose() {
    var resolveClose;
    var source = {};
    source[Symbol.asyncIterator] = function () {
        return {
            next: function () {
                record("generator:next");
                return Promise.resolve({ value: 1, done: false });
            },
            return: function () {
                record("generator:return");
                return new Promise(function (resolve) {
                    resolveClose = resolve;
                });
            }
        };
    };

    async function* sample() {
        try {
            for await (var value of source) {
                yield value;
            }
        } finally {
            record("generator:finally");
        }
    }

    var iterator = sample();
    var first = await iterator.next();
    record("generator:first:" + first.value + ":" + first.done);
    var returned = iterator.return(7);
    record("generator:queued");
    Promise.resolve().then(function () {
        record("generator:tick");
    });
    await Promise.resolve();
    record("generator:before-close-resolve");
    resolveClose(1);
    var result = await returned;
    record("generator:result:" + result.value + ":" + result.done);
}

async function asyncGeneratorQueuedReturnDuringNext() {
    var resolveNext;
    var resolveClose;
    var source = {};
    source[Symbol.asyncIterator] = function () {
        return {
            next: function () {
                record("pending-generator:next");
                return new Promise(function (resolve) {
                    resolveNext = resolve;
                });
            },
            return: function () {
                record("pending-generator:return");
                return new Promise(function (resolve) {
                    resolveClose = resolve;
                });
            }
        };
    };
    async function* sample() {
        for await (var value of source) {
            yield value;
        }
    }

    var iterator = sample();
    var firstPromise = iterator.next();
    record("pending-generator:next-request");
    var returnPromise = iterator.return(8);
    record("pending-generator:return-request");
    await Promise.resolve();
    record("pending-generator:before-next-resolve");
    resolveNext({ value: 3, done: false });
    var first = await firstPromise;
    record("pending-generator:first:" + first.value + ":" + first.done);
    await Promise.resolve();
    record("pending-generator:before-close-resolve");
    resolveClose({});
    var returned = await returnPromise;
    record(
        "pending-generator:result:" +
        returned.value +
        ":" +
        returned.done
    );
}

async function asyncFunctionReturnClose() {
    var resolveClose;
    var source = {};
    source[Symbol.asyncIterator] = function () {
        return {
            next: function () {
                return Promise.resolve({ value: 1, done: false });
            },
            return: function () {
                record("function-return:call-close");
                return new Promise(function (resolve) {
                    resolveClose = resolve;
                });
            }
        };
    };
    async function sample() {
        for await (var value of source) {
            return 9;
        }
    }
    var promise = sample();
    var result = await promise;
    record("function-return:result:" + result);
    resolveClose({});
    await Promise.resolve();
    record("function-return:close-settled");
}

async function main() {
    await acquisitionAndValueModes();
    await disabledClosePaths();
    await loopControlAndClosePrecedence();
    await bindingAndSourceShapes();
    await asyncGeneratorReturnClose();
    await asyncGeneratorQueuedReturnDuringNext();
    await asyncFunctionReturnClose();
    print(events.join("\n"));
}

main().catch(function (error) {
    print(
        "UNEXPECTED:" +
        error.name +
        ":" +
        error.message +
        "\n" +
        events.join("\n")
    );
});
