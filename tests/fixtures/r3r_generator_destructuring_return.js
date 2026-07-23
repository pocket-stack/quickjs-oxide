var r3rTranscript = [];
var r3rLog = [];

function resultText(result) {
    return String(result.value) + ":" + result.done;
}

function errorText(error) {
    return error.name + ":" + error.message;
}

function tracedIterable(label, values, closeError) {
    return {
        [Symbol.iterator]: function () {
            var index = 0;
            r3rLog.push(label + ".iterator");
            return {
                next: function () {
                    r3rLog.push(label + ".next");
                    if (index < values.length)
                        return { value: values[index++], done: false };
                    return { value: undefined, done: true };
                },
                return: function (value) {
                    r3rLog.push(
                        label + ".return(" +
                        (arguments.length === 0 ? "-" : String(value)) +
                        ")"
                    );
                    if (closeError !== undefined)
                        throw new Error(closeError);
                    return { value: value, done: true };
                }
            };
        }
    };
}

function record(label, callback) {
    r3rLog = [];
    callback();
    r3rTranscript.push(label + "=" + r3rLog.join(","));
}

record("var", function () {
    function* generator() {
        var [value = yield "pause"] = tracedIterable("var", [undefined]);
    }
    var iterator = generator();
    r3rLog.push("next=" + resultText(iterator.next()));
    r3rLog.push("return=" + resultText(iterator.return(42)));
});

record("assignment", function () {
    function* generator() {
        var value;
        [value = yield "pause"] = tracedIterable("assignment", [undefined]);
    }
    var iterator = generator();
    r3rLog.push("next=" + resultText(iterator.next()));
    r3rLog.push("return=" + resultText(iterator.return(42)));
});

record("nested", function () {
    function* generator() {
        var [[value = yield "pause"]] = tracedIterable(
            "outer",
            [tracedIterable("inner", [undefined])]
        );
    }
    var iterator = generator();
    r3rLog.push("next=" + resultText(iterator.next()));
    r3rLog.push("return=" + resultText(iterator.return(42)));
});

record("finally", function () {
    function* generator() {
        try {
            var [value = yield "pause"] =
                tracedIterable("finally-iterator", [undefined]);
        } finally {
            r3rLog.push("finally");
        }
    }
    var iterator = generator();
    r3rLog.push("next=" + resultText(iterator.next()));
    r3rLog.push("return=" + resultText(iterator.return(42)));
});

record("yield-star", function () {
    function* generator() {
        var [value = yield* tracedIterable("delegate", ["pause"])] =
            tracedIterable("host", [undefined]);
    }
    var iterator = generator();
    r3rLog.push("next=" + resultText(iterator.next()));
    r3rLog.push("return=" + resultText(iterator.return(42)));
});

record("for-of-head", function () {
    function* generator() {
        for (var [value = yield "pause"] of tracedIterable(
            "loop",
            [tracedIterable("head", [undefined])]
        )) {
            r3rLog.push("body");
        }
    }
    var iterator = generator();
    r3rLog.push("next=" + resultText(iterator.next()));
    r3rLog.push("return=" + resultText(iterator.return(42)));
});

record("throwing-inner-close", function () {
    function* generator() {
        for (var [value = yield "pause"] of tracedIterable(
            "throw-outer",
            [tracedIterable("throw-inner", [undefined], "inner-close")],
            "outer-close"
        )) {
            r3rLog.push("body");
        }
    }
    var iterator = generator();
    r3rLog.push("next=" + resultText(iterator.next()));
    try {
        iterator.return(42);
        r3rLog.push("return=none");
    } catch (error) {
        r3rLog.push("return=" + errorText(error));
    }
});

r3rTranscript.push("r3r-generator-destructuring-return-oracle=ok");
r3rTranscript.join("\n");
