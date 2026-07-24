//! Differential coverage for `yield*` inside async generators.
//!
//! The observations in this file were captured from pinned QuickJS 2026-06-04.
//! They cover the two different delegation paths: a manually implemented async
//! iterator passes yielded promises through unchanged, while the hidden
//! Async-from-Sync iterator assimilates synchronous iterator values.

use std::ffi::OsStr;
use std::process::{Command, Output};

use quickjs_oxide::{Context, Runtime, RuntimeError, Value};

struct SuccessCase {
    description: &'static str,
    source: &'static str,
    expected_stdout: &'static str,
}

const SEMANTIC_CASES: &[SuccessCase] = &[
    SuccessCase {
        description: "async iterator selection cached next and raw yielded promise identity",
        source: r#"
var events = [];
var inner = Promise.resolve("inner");
var asyncIterator = {
    label: "async-iterator",
    get next() {
        events.push("get-next:" + this.label);
        var count = 0;
        return function (value) {
            events.push("call-next:" + this.label + ":" + String(value));
            count++;
            if (count === 1) {
                return {
                    get then() {
                        events.push("get-result-then");
                        return function (resolve) {
                            events.push("call-result-then");
                            resolve({
                                get done() {
                                    events.push("get-done-1");
                                    return false;
                                },
                                get value() {
                                    events.push("get-value-1");
                                    return inner;
                                }
                            });
                        };
                    }
                };
            }
            return Promise.resolve({
                get done() {
                    events.push("get-done-2");
                    return true;
                },
                get value() {
                    events.push("get-value-2");
                    return "delegate-complete";
                }
            });
        };
    }
};
var iterable = {
    get [Symbol.asyncIterator]() {
        events.push("get-async");
        return function () {
            events.push("call-async:" + (this === iterable));
            return asyncIterator;
        };
    },
    get [Symbol.iterator]() {
        events.push("get-sync-miss");
        throw new Error("sync iterator must not be read");
    }
};
async function* delegate() {
    events.push("before");
    var value = yield* iterable;
    events.push("after:" + value);
    return "outer:" + value;
}
var iterator = delegate();
iterator.next("ignored").then(function (result) {
    events.push("first:" + (result.value === inner) + ":" + result.done);
    return iterator.next("sent");
}).then(function (result) {
    events.push("second:" + result.value + ":" + result.done);
    print(events.join("|"));
}, function (error) {
    print("reject:" + error.name + ":" + error.message + "|" + events.join("|"));
});
"#,
        expected_stdout: concat!(
            "before|get-async|call-async:true|get-next:async-iterator|",
            "call-next:async-iterator:undefined|get-result-then|call-result-then|",
            "get-done-1|get-value-1|first:true:false|",
            "call-next:async-iterator:sent|get-done-2|get-value-2|",
            "after:delegate-complete|second:outer:delegate-complete:true\n",
        ),
    },
    SuccessCase {
        description: "sync fallback uses Async-from-Sync and assimilates values",
        source: r#"
var events = [];
function thenable(label, value) {
    return {
        get then() {
            events.push("get-then:" + label);
            return function (resolve) {
                events.push("call-then:" + label);
                resolve(value);
            };
        }
    };
}
var syncIterator = {
    label: "sync-iterator",
    get next() {
        events.push("get-next:" + this.label);
        var count = 0;
        return function (value) {
            events.push("call-next:" + this.label + ":" + String(value));
            count++;
            if (count === 1) {
                return {
                    get done() {
                        events.push("get-done-1");
                        return false;
                    },
                    get value() {
                        events.push("get-value-1");
                        return thenable("yield", 42);
                    }
                };
            }
            return {
                get done() {
                    events.push("get-done-2");
                    return true;
                },
                get value() {
                    events.push("get-value-2");
                    return thenable("complete", 7);
                }
            };
        };
    }
};
var iterable = {
    get [Symbol.asyncIterator]() {
        events.push("get-async-null");
        return null;
    },
    get [Symbol.iterator]() {
        events.push("get-sync");
        return function () {
            events.push("call-sync:" + (this === iterable));
            return syncIterator;
        };
    }
};
async function* delegate() {
    var value = yield* iterable;
    events.push("after:" + value);
    return "outer:" + value;
}
var iterator = delegate();
iterator.next("ignored").then(function (result) {
    events.push("first:" + result.value + ":" + result.done);
    return iterator.next("sent");
}).then(function (result) {
    events.push("second:" + result.value + ":" + result.done);
    print(events.join("|"));
}, function (error) {
    print("reject:" + error.name + ":" + error.message + "|" + events.join("|"));
});
"#,
        expected_stdout: concat!(
            "get-async-null|get-sync|call-sync:true|get-next:sync-iterator|",
            "call-next:sync-iterator:undefined|get-done-1|get-value-1|",
            "get-then:yield|call-then:yield|first:42:false|",
            "call-next:sync-iterator:sent|get-done-2|get-value-2|",
            "get-then:complete|call-then:complete|after:7|second:outer:7:true\n",
        ),
    },
    SuccessCase {
        description: "delegated return awaits input and result and preserves FIFO requests",
        source: r#"
var events = [];
var rawYield = Promise.resolve("raw-return-yield");
var input = {
    get then() {
        events.push("get-input-then");
        return function (resolve) {
            events.push("call-input-then");
            resolve("awaited-input");
        };
    }
};
var complete = {
    get then() {
        events.push("get-complete-then");
        return function (resolve) {
            events.push("call-complete-then");
            resolve("return-complete");
        };
    }
};
var delegateIterator = {
    label: "delegate",
    next: function () {
        events.push("call-next");
        return { value: "seed", done: false };
    },
    get return() {
        events.push("get-return");
        var methodCall = ++this.returnGets;
        return function (value) {
            events.push(
                "call-return-" + methodCall + ":" + value + ":" +
                (this === delegateIterator)
            );
            return {
                get then() {
                    events.push("get-result-then-" + methodCall);
                    return function (resolve) {
                        events.push("call-result-then-" + methodCall);
                        resolve(methodCall === 1
                            ? {
                                get done() {
                                    events.push("get-return-done-1");
                                    return false;
                                },
                                get value() {
                                    events.push("get-return-value-1");
                                    return rawYield;
                                }
                            }
                            : {
                                get done() {
                                    events.push("get-return-done-2");
                                    return true;
                                },
                                get value() {
                                    events.push("get-return-value-2");
                                    return complete;
                                }
                            });
                    };
                }
            };
        };
    },
    returnGets: 0
};
var iterable = {
    [Symbol.asyncIterator]: function () {
        return delegateIterator;
    }
};
async function* generator() {
    yield* iterable;
    events.push("after-miss");
}
var iterator = generator();
iterator.next().then(function (first) {
    events.push("first:" + first.value + ":" + first.done);
    var one = iterator.return(input);
    var two = iterator.return("second-input");
    events.push("queued");
    return Promise.all([one, two]);
}).then(function (results) {
    events.push(
        "return-1:" + (results[0].value === rawYield) + ":" + results[0].done
    );
    events.push("return-2:" + results[1].value + ":" + results[1].done);
    print(events.join("|"));
}, function (error) {
    print("reject:" + error.name + ":" + error.message + "|" + events.join("|"));
});
"#,
        expected_stdout: concat!(
            "call-next|first:seed:false|get-input-then|queued|call-input-then|",
            "get-return|call-return-1:awaited-input:true|get-result-then-1|",
            "call-result-then-1|get-return-done-1|get-return-value-1|",
            "get-return|call-return-2:second-input:true|get-result-then-2|",
            "call-result-then-2|get-return-done-2|get-return-value-2|",
            "get-complete-then|call-complete-then|",
            "return-1:true:false|return-2:return-complete:true\n",
        ),
    },
    SuccessCase {
        description: "delegated throw is not input-awaited and preserves FIFO requests",
        source: r#"
var events = [];
var rawYield = Promise.resolve("raw-throw-yield");
var thrown = {
    get then() {
        events.push("unexpected-get-thrown-then");
    }
};
var delegateIterator = {
    next: function () {
        events.push("call-next");
        return { value: "seed", done: false };
    },
    get throw() {
        var methodCall = ++this.throwGets;
        events.push("get-throw-" + methodCall);
        return function (value) {
            events.push(
                "call-throw-" + methodCall + ":" + (value === thrown) + ":" +
                (this === delegateIterator)
            );
            return {
                get then() {
                    events.push("get-result-then-" + methodCall);
                    return function (resolve) {
                        events.push("call-result-then-" + methodCall);
                        resolve(methodCall === 1
                            ? {
                                get done() {
                                    events.push("get-throw-done-1");
                                    return false;
                                },
                                get value() {
                                    events.push("get-throw-value-1");
                                    return rawYield;
                                }
                            }
                            : {
                                get done() {
                                    events.push("get-throw-done-2");
                                    return true;
                                },
                                get value() {
                                    events.push("get-throw-value-2");
                                    return "throw-complete";
                                }
                            });
                    };
                }
            };
        };
    },
    throwGets: 0
};
var iterable = {
    [Symbol.asyncIterator]: function () {
        return delegateIterator;
    }
};
async function* generator() {
    var value = yield* iterable;
    events.push("after:" + value);
    return "outer:" + value;
}
var iterator = generator();
iterator.next().then(function (first) {
    events.push("first:" + first.value + ":" + first.done);
    var one = iterator.throw(thrown);
    var two = iterator.throw("second");
    events.push("queued");
    return Promise.all([one, two]);
}).then(function (results) {
    events.push(
        "throw-1:" + (results[0].value === rawYield) + ":" + results[0].done
    );
    events.push("throw-2:" + results[1].value + ":" + results[1].done);
    print(events.join("|"));
}, function (error) {
    print("reject:" + error.name + ":" + error.message + "|" + events.join("|"));
});
"#,
        expected_stdout: concat!(
            "call-next|first:seed:false|get-throw-1|call-throw-1:true:true|",
            "get-result-then-1|queued|call-result-then-1|get-throw-done-1|",
            "get-throw-value-1|get-throw-2|call-throw-2:false:true|",
            "get-result-then-2|call-result-then-2|get-throw-done-2|",
            "get-throw-value-2|after:throw-complete|",
            "throw-1:true:false|throw-2:outer:throw-complete:true\n",
        ),
    },
    SuccessCase {
        description: "Async-from-Sync forwards present throw and return methods",
        source: r#"
var events = [];
function thenable(label, value) {
    return {
        get then() {
            events.push("get-then:" + label);
            return function (resolve) {
                events.push("call-then:" + label);
                resolve(value);
            };
        }
    };
}
var syncIterator = {
    next: function () {
        events.push("next");
        return { value: thenable("next-value", "seed"), done: false };
    },
    get throw() {
        events.push("get-throw");
        return function (value) {
            events.push("call-throw:" + value + ":" + (this === syncIterator));
            return {
                get done() {
                    events.push("get-throw-done");
                    return false;
                },
                get value() {
                    events.push("get-throw-value");
                    return thenable("throw-value", "throw-yield");
                }
            };
        };
    },
    get return() {
        events.push("get-return");
        return function (value) {
            events.push("call-return:" + value + ":" + (this === syncIterator));
            return {
                get done() {
                    events.push("get-return-done");
                    return true;
                },
                get value() {
                    events.push("get-return-value");
                    return thenable("return-value", "return-complete");
                }
            };
        };
    }
};
var iterable = {
    [Symbol.iterator]: function () {
        return syncIterator;
    }
};
async function* generator() {
    yield* iterable;
    events.push("after-miss");
}
var iterator = generator();
iterator.next().then(function (first) {
    events.push("first:" + first.value + ":" + first.done);
    return iterator.throw("thrown");
}).then(function (thrownResult) {
    events.push("thrown:" + thrownResult.value + ":" + thrownResult.done);
    return iterator.return(thenable("return-input", "awaited-return"));
}).then(function (returnResult) {
    events.push("returned:" + returnResult.value + ":" + returnResult.done);
    print(events.join("|"));
}, function (error) {
    print("reject:" + error.name + ":" + error.message + "|" + events.join("|"));
});
"#,
        expected_stdout: concat!(
            "next|get-then:next-value|call-then:next-value|first:seed:false|",
            "get-throw|call-throw:thrown:true|get-throw-done|get-throw-value|",
            "get-then:throw-value|call-then:throw-value|thrown:throw-yield:false|",
            "get-then:return-input|call-then:return-input|get-return|",
            "call-return:awaited-return:true|get-return-done|get-return-value|",
            "get-then:return-value|call-then:return-value|",
            "returned:return-complete:true\n",
        ),
    },
    SuccessCase {
        description: "missing async return awaits the same non-callable then twice",
        source: r#"
var events = [];
var returned = {
    get then() {
        events.push("get-returned-then");
        return undefined;
    }
};
var delegateIterator = {
    next: function () {
        events.push("next");
        return { value: "seed", done: false };
    }
};
var iterable = {
    [Symbol.asyncIterator]: function () {
        return delegateIterator;
    }
};
async function* generator() {
    yield* iterable;
    events.push("after-miss");
}
var iterator = generator();
iterator.next().then(function (first) {
    events.push("first:" + first.value + ":" + first.done);
    return iterator.return(returned);
}).then(function (result) {
    events.push("returned:" + (result.value === returned) + ":" + result.done);
    print(events.join("|"));
}, function (error) {
    print("reject:" + error.name + ":" + error.message + "|" + events.join("|"));
});
"#,
        expected_stdout: concat!(
            "next|first:seed:false|get-returned-then|get-returned-then|",
            "returned:true:true\n",
        ),
    },
    SuccessCase {
        description: "missing async throw closes without an argument and keeps close errors",
        source: r#"
var events = [];
function make(label, failClose) {
    var delegateIterator = {
        label: label,
        next: function () {
            events.push("next-" + label);
            return { value: label, done: false };
        },
        get return() {
            events.push("get-return-" + label);
            return function () {
                events.push(
                    "call-return-" + label + ":" + arguments.length + ":" +
                    (this === delegateIterator)
                );
                return {
                    get then() {
                        events.push("get-close-then-" + label);
                        return function (resolve, reject) {
                            events.push("call-close-then-" + label);
                            if (failClose) {
                                reject("close-" + label);
                            } else {
                                resolve(1);
                            }
                        };
                    }
                };
            };
        }
    };
    return (async function* () {
        yield* {
            [Symbol.asyncIterator]: function () {
                return delegateIterator;
            }
        };
    })();
}
function outcome(promise) {
    return promise.then(
        function (result) {
            return "ok:" + result.value + ":" + result.done;
        },
        function (error) {
            return "reject:" +
                (error && error.name ? error.name : String(error));
        }
    );
}
var normal = make("normal", false);
var failing = make("failing", true);
Promise.all([normal.next(), failing.next()]).then(function () {
    return Promise.all([
        outcome(normal.throw("original-normal")),
        outcome(failing.throw("original-failing"))
    ]);
}).then(function (results) {
    print(results.join("|") + "|" + events.join("|"));
});
"#,
        expected_stdout: concat!(
            "reject:TypeError|reject:close-failing|next-normal|next-failing|",
            "get-return-normal|call-return-normal:0:true|get-close-then-normal|",
            "get-return-failing|call-return-failing:0:true|",
            "get-close-then-failing|call-close-then-normal|",
            "call-close-then-failing\n",
        ),
    },
    SuccessCase {
        description: "missing sync throw uses IteratorClose and close-error precedence",
        source: r#"
var events = [];
function wrap(label, failClose) {
    var syncIterator = {
        next: function () {
            events.push("next-" + label);
            return { value: label, done: false };
        },
        get return() {
            events.push("get-return-" + label);
            if (failClose) {
                throw "close-" + label;
            }
            return function () {
                events.push(
                    "call-return-" + label + ":" + arguments.length + ":" +
                    (this === syncIterator)
                );
                return {};
            };
        }
    };
    return (async function* () {
        yield* {
            [Symbol.iterator]: function () {
                return syncIterator;
            }
        };
    })();
}
function outcome(promise) {
    return promise.then(
        function (result) {
            return "ok:" + result.value + ":" + result.done;
        },
        function (error) {
            return "reject:" +
                (error && error.name ? error.name : String(error));
        }
    );
}
var normal = wrap("normal", false);
var failing = wrap("failing", true);
Promise.all([normal.next(), failing.next()]).then(function () {
    return Promise.all([
        outcome(normal.throw("original-normal")),
        outcome(failing.throw("original-failing"))
    ]);
}).then(function (results) {
    print(results.join("|") + "|" + events.join("|"));
});
"#,
        expected_stdout: concat!(
            "reject:TypeError|reject:close-failing|next-normal|next-failing|",
            "get-return-normal|call-return-normal:0:true|get-return-failing\n",
        ),
    },
    SuccessCase {
        description: "Async-from-Sync rejection closes next and throw but not return",
        source: r#"
var events = [];
function outcome(promise) {
    return promise.then(
        function (result) {
            return "ok:" + result.value + ":" + result.done;
        },
        function (error) {
            return "reject:" + String(error);
        }
    );
}
function wrap(syncIterator) {
    return (async function* () {
        yield* {
            [Symbol.iterator]: function () {
                return syncIterator;
            }
        };
    })();
}
var nextSync = {
    next: function () {
        events.push("next-reject-next");
        return { done: false, value: Promise.reject("next-reason") };
    },
    get return() {
        events.push("next-reject-get-close");
        throw "next-close-error";
    }
};
var throwSync = {
    next: function () {
        events.push("throw-reject-next");
        return { done: false, value: "seed" };
    },
    throw: function () {
        events.push("throw-reject-throw");
        return { done: false, value: Promise.reject("throw-reason") };
    },
    get return() {
        events.push("throw-reject-get-close");
        throw "throw-close-error";
    }
};
var returnSync = {
    next: function () {
        events.push("return-reject-next");
        return { done: false, value: "seed" };
    },
    get return() {
        events.push("return-reject-get-return");
        return function () {
            events.push("return-reject-call-return");
            return { done: false, value: Promise.reject("return-reason") };
        };
    }
};
var nextIterator = wrap(nextSync);
var throwIterator = wrap(throwSync);
var returnIterator = wrap(returnSync);
Promise.all([
    outcome(nextIterator.next()),
    throwIterator.next(),
    returnIterator.next()
]).then(function (started) {
    return Promise.all([
        Promise.resolve(started[0]),
        outcome(throwIterator.throw("sent")),
        outcome(returnIterator.return("sent"))
    ]);
}).then(function (results) {
    print(results.join("|") + "|" + events.join("|"));
});
"#,
        expected_stdout: concat!(
            "reject:next-reason|reject:throw-reason|reject:return-reason|",
            "next-reject-next|throw-reject-next|return-reject-next|",
            "next-reject-get-close|throw-reject-throw|",
            "throw-reject-get-close|return-reject-get-return|",
            "return-reject-call-return\n",
        ),
    },
    SuccessCase {
        description: "iterator acquisition and awaited result abrupt completions keep order",
        source: r#"
var events = [];
function generator(iterable) {
    return (async function* () {
        yield* iterable;
    })();
}
function outcome(label, promise) {
    return promise.then(
        function (result) {
            return label + ":ok:" + result.value + ":" + result.done;
        },
        function (error) {
            return label + ":reject:" +
                (error && error.name ? error.name : String(error));
        }
    );
}
var asyncGet = {};
Object.defineProperty(asyncGet, Symbol.asyncIterator, {
    get: function () {
        events.push("async-get");
        throw "async-get-error";
    }
});
var asyncPrimitive = {
    [Symbol.asyncIterator]: function () {
        events.push("async-call-primitive");
        return 1;
    }
};
var nextGet = {
    [Symbol.asyncIterator]: function () {
        return {
            get next() {
                events.push("next-get");
                throw "next-get-error";
            }
        };
    }
};
var thenGet = {
    [Symbol.asyncIterator]: function () {
        return {
            next: function () {
                events.push("then-next-call");
                return {
                    get then() {
                        events.push("then-get");
                        throw "then-get-error";
                    }
                };
            }
        };
    }
};
var resultPrimitive = {
    [Symbol.asyncIterator]: function () {
        return {
            next: function () {
                events.push("result-primitive-call");
                return 1;
            }
        };
    }
};
var doneGet = {
    [Symbol.asyncIterator]: function () {
        return {
            next: function () {
                events.push("done-next-call");
                return {
                    get done() {
                        events.push("done-get");
                        throw "done-get-error";
                    },
                    get value() {
                        events.push("unexpected-value-get");
                        return 1;
                    }
                };
            }
        };
    }
};
var syncGet = {};
Object.defineProperty(syncGet, Symbol.asyncIterator, {
    get: function () {
        events.push("async-null");
        return undefined;
    }
});
Object.defineProperty(syncGet, Symbol.iterator, {
    get: function () {
        events.push("sync-get");
        throw "sync-get-error";
    }
});
Promise.all([
    outcome("async-get", generator(asyncGet).next()),
    outcome("async-primitive", generator(asyncPrimitive).next()),
    outcome("next-get", generator(nextGet).next()),
    outcome("then-get", generator(thenGet).next()),
    outcome("result-primitive", generator(resultPrimitive).next()),
    outcome("done-get", generator(doneGet).next()),
    outcome("sync-get", generator(syncGet).next())
]).then(function (results) {
    print(results.join("|") + "|events=" + events.join(","));
});
"#,
        expected_stdout: concat!(
            "async-get:reject:async-get-error|async-primitive:reject:TypeError|",
            "next-get:reject:next-get-error|then-get:reject:then-get-error|",
            "result-primitive:reject:TypeError|done-get:reject:done-get-error|",
            "sync-get:reject:sync-get-error|events=async-get,",
            "async-call-primitive,next-get,then-next-call,then-get,",
            "result-primitive-call,done-next-call,async-null,sync-get,done-get\n",
        ),
    },
];

#[test]
fn pinned_quickjs_expectations_are_authenticated() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!(
            "SKIP async-generator yield-star expectation authentication: \
             set QJS_ORACLE to pinned upstream qjs"
        );
        return;
    };

    for case in SEMANTIC_CASES {
        let quickjs = run(&oracle, case.source);
        assert_success("pinned QuickJS", case, &quickjs);
    }
}

#[test]
fn async_generator_yield_star_semantics_match_pinned_quickjs() {
    let oracle = std::env::var_os("QJS_ORACLE");
    if oracle.is_none() {
        eprintln!(
            "SKIP async-generator yield-star differential: \
             set QJS_ORACLE to pinned upstream qjs"
        );
    }

    for case in SEMANTIC_CASES {
        let quickjs = if let Some(oracle) = &oracle {
            let quickjs = run(oracle, case.source);
            assert_success("pinned QuickJS", case, &quickjs);
            Some(quickjs)
        } else {
            None
        };

        let oxide = run(env!("CARGO_BIN_EXE_qjs").as_ref(), case.source);
        assert_success("quickjs-oxide", case, &oxide);

        if let Some(quickjs) = quickjs {
            assert_eq!(
                oxide.stdout, quickjs.stdout,
                "async-generator yield-star output differed for {}",
                case.description
            );
        }
    }
}

#[test]
fn suspended_delegation_retains_async_and_sync_iterators_across_gc() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    eval(
        &mut context,
        r#"
var asyncOutcome = "pending";
var syncOutcome = "pending";
var releaseAsync;
var releaseSync;
var asyncGate = new Promise(function (resolve) {
    releaseAsync = resolve;
});
var syncGate = new Promise(function (resolve) {
    releaseSync = resolve;
});
var asyncIterable = {
    [Symbol.asyncIterator]: function () {
        return {
            token: 40,
            next: function () {
                var self = this;
                return asyncGate.then(function () {
                    return { value: self.token + 2, done: false };
                });
            }
        };
    }
};
var syncIterable = {
    [Symbol.iterator]: function () {
        return {
            token: 40,
            next: function () {
                var self = this;
                return {
                    value: syncGate.then(function () {
                        return self.token + 2;
                    }),
                    done: false
                };
            }
        };
    }
};
var asyncIterator = (async function* () {
    yield* asyncIterable;
})();
var syncIterator = (async function* () {
    yield* syncIterable;
})();
asyncIterator.next().then(function (result) {
    asyncOutcome = result.value + ":" + result.done;
});
syncIterator.next().then(function (result) {
    syncOutcome = result.value + ":" + result.done;
});
asyncIterable = null;
syncIterable = null;
asyncIterator = null;
syncIterator = null;
"#,
    );

    runtime.run_gc().unwrap();
    eval(&mut context, "releaseAsync(); releaseSync();");
    while runtime.is_job_pending() {
        runtime.execute_pending_job().unwrap();
        runtime.run_gc().unwrap();
    }

    assert_eq!(
        text(eval(&mut context, "asyncOutcome + '|' + syncOutcome")),
        "42:false|42:false"
    );
}

fn assert_success(engine: &str, case: &SuccessCase, output: &Output) {
    assert!(
        output.status.success(),
        "{engine} rejected {}: {}\nsource:\n{}",
        case.description,
        String::from_utf8_lossy(&output.stderr),
        case.source
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        case.expected_stdout,
        "{engine} output drifted for {}",
        case.description
    );
}

fn run(executable: &OsStr, source: &str) -> Output {
    Command::new(executable)
        .args(["-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run {executable:?}: {error}"))
}

fn eval(context: &mut Context, source: &str) -> Value {
    context.eval(source).unwrap_or_else(|error| {
        if error == RuntimeError::Exception {
            panic!(
                "unexpected JavaScript exception: {:?}",
                context.take_exception()
            );
        }
        panic!("unexpected engine error: {error}");
    })
}

fn text(value: Value) -> String {
    let Value::String(value) = value else {
        panic!("expected a string");
    };
    value.to_utf8_lossy()
}
