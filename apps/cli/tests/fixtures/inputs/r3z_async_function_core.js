var r3zTranscript = [];

function r3zMark(value) {
    r3zTranscript.push(value);
}

var r3zThenable = {};
Object.defineProperty(r3zThenable, "then", {
    get: function () {
        r3zMark("then:get");
        return function (resolve, reject) {
            r3zMark("then:call");
            resolve(20);
            reject("late");
            resolve(99);
        };
    }
});

async function r3zCore(addend) {
    r3zMark("body");
    var value = await r3zThenable;
    r3zMark("await:" + value);
    try {
        await Promise.reject("x");
    } catch (error) {
        r3zMark("catch:" + error);
    } finally {
        r3zMark("finally");
    }
    return await Promise.resolve(eval("value + addend"));
}

var r3zAsyncFunction = Object.getPrototypeOf(r3zCore).constructor;
var r3zPromise = r3zCore(22);

async function r3zStackBoundary() {
    return 1;
}

function r3zDescendToAsync(depth) {
    if (depth === 0) {
        return r3zStackBoundary();
    }
    return r3zDescendToAsync(depth - 1);
}

try {
    var r3zBoundaryPromise = r3zDescendToAsync(30);
    r3zMark("stack-boundary:" + (r3zBoundaryPromise instanceof Promise));
    r3zBoundaryPromise.then(
        function () {},
        function () {}
    );
} catch (error) {
    r3zMark("stack-boundary:sync:" + error.name + ":" + error.message);
}

try {
    new r3zCore();
} catch (error) {
    r3zMark("construct:" + error.name);
}
r3zMark(
    [
        "shape",
        typeof r3zCore,
        r3zCore.length,
        r3zCore.name,
        Object.prototype.hasOwnProperty.call(r3zCore, "prototype"),
        Object.prototype.toString.call(r3zCore),
        r3zAsyncFunction.name,
        r3zAsyncFunction.length,
        "AsyncFunction" in globalThis
    ].join("|")
);
r3zMark("sync");

r3zPromise.then(
    function (value) {
        r3zMark("done:" + value);
        var inner = Promise.resolve(value);
        async function adopt() {
            return inner;
        }
        var outer = adopt();
        r3zMark("outer-independent:" + (outer !== inner));
        outer.then(function (adopted) {
            r3zMark("adopted:" + adopted);
            var dynamic = r3zAsyncFunction("value", "return await value");
            dynamic(adopted).then(function (dynamicValue) {
                r3zMark("dynamic:" + dynamicValue);
                print(r3zTranscript.join("\n"));
            });
        });
    },
    function (error) {
        throw error;
    }
);
