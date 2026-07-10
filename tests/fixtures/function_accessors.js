function units(value) {
    var s = String(value), parts = [];
    for (var i = 0; i < s.length; i++)
        parts.push(("0000" + s.charCodeAt(i).toString(16)).slice(-4));
    return s.length + ":" + parts.join(",");
}
function scalar(v) {
    if (v === undefined) return "undefined";
    if (v === null) return "null";
    if (typeof v === "boolean") return "boolean:" + v;
    if (typeof v === "number") {
        if (v !== v) return "number:NaN";
        if (v === 0 && 1 / v === -Infinity) return "number:-0";
        if (v === 0) return "number:+0";
        return "number:" + String(v);
    }
    if (typeof v === "string") return "string:" + units(v);
    return typeof v + ":" + units(String(v));
}
function observe(f) {
    try {
        return "return:" + scalar(f());
    } catch (e) {
        return "throw:" + units(e.name) + ":" + units(e.message);
    }
}
function emit(label, value) { print(label + "=" + value); }
function hasOwn(o, k) {
    return Object.prototype.hasOwnProperty.call(o, k);
}

function makeCounter(start) {
    var value = start;
    return function(step) {
        value += step;
        return value;
    };
}
var counterA = makeCounter(10);
var counterB = makeCounter(100);
emit("closure-independent",
    [counterA(1), counterA(2), counterB(5), counterA(-3), counterB(1)]
        .map(scalar).join("|"));

function makeMethod(prefix) {
    var count = 0;
    return function(delta) {
        "use strict";
        count += delta;
        return prefix + ":" + this.tag + ":" + count;
    };
}
var sharedMethod = makeMethod("prefix");
var methodA = { tag: "A", run: sharedMethod };
var methodB = { tag: "B", run: sharedMethod };
emit("closure-this-a", scalar(methodA.run(1)));
emit("closure-this-b", scalar(methodB.run(2)));

function strictThisTag() {
    "use strict";
    return this === undefined ? "undefined-this" : this.tag;
}
var holder = { tag: "holder", run: strictThisTag };
var detached = holder.run;
emit("this-method", scalar(holder.run()));
emit("this-detached", scalar(detached()));
emit("this-call", scalar(detached.call({ tag: "explicit" })));

var getterPrototype = {};
Object.defineProperty(getterPrototype, "x", {
    configurable: true,
    get: function() { return this.tag; }
});
var getterChild = Object.create(getterPrototype);
getterChild.tag = "child";
emit("getter-prototype-receiver", scalar(getterChild.x));
emit("getter-reflect-receiver",
    scalar(Reflect.get(getterPrototype, "x", { tag: "reflect" })));

var setterPrototype = {};
Object.defineProperty(setterPrototype, "x", {
    configurable: true,
    set: function(value) { this.stored = value; }
});
var setterChild = Object.create(setterPrototype);
setterChild.x = 7;
emit("setter-prototype-state",
    scalar(setterPrototype.stored) + "|" + scalar(setterChild.stored));
var setterReceiver = {};
emit("setter-reflect-result",
    scalar(Reflect.set(setterPrototype, "x", 8, setterReceiver)));
emit("setter-reflect-state",
    scalar(setterPrototype.stored) + "|" + scalar(setterReceiver.stored));

var selfDeletingGetter = {};
var getterCalls = 0;
Object.defineProperty(selfDeletingGetter, "x", {
    configurable: true,
    get: function() {
        getterCalls += 1;
        delete this.x;
        return "call-" + getterCalls + "-own-" + hasOwn(this, "x");
    }
});
emit("getter-self-delete-first", scalar(selfDeletingGetter.x));
emit("getter-self-delete-after",
    scalar(hasOwn(selfDeletingGetter, "x")) + "|" +
    scalar(selfDeletingGetter.x) + "|" + scalar(getterCalls));

var selfDeletingSetter = {};
Object.defineProperty(selfDeletingSetter, "x", {
    configurable: true,
    set: function(value) {
        delete this.x;
        this.seen = value;
    }
});
selfDeletingSetter.x = 9;
emit("setter-self-delete-first",
    scalar(hasOwn(selfDeletingSetter, "x")) + "|" +
    scalar(selfDeletingSetter.seen));
selfDeletingSetter.x = 20;
var d = Object.getOwnPropertyDescriptor(selfDeletingSetter, "x");
emit("setter-self-delete-second",
    scalar(d.value) + "|w=" + Number(d.writable) +
    ",e=" + Number(d.enumerable) + ",c=" + Number(d.configurable));

function makeFragile() {
    var count = 0;
    return function(shouldThrow) {
        count += 1;
        if (shouldThrow) throw new RangeError("boom-" + count);
        return count;
    };
}
var fragile = makeFragile();
emit("closure-exception-before",
    observe(function() { return fragile(false); }));
emit("closure-exception-throw",
    observe(function() { return fragile(true); }));
emit("closure-exception-after",
    observe(function() { return fragile(false); }));

var throwingGetter = {};
Object.defineProperty(throwingGetter, "x", {
    configurable: true,
    get: function() {
        delete this.x;
        throw new TypeError("getter-boom");
    }
});
emit("getter-exception",
    observe(function() { return throwingGetter.x; }));
emit("getter-exception-after",
    scalar(hasOwn(throwingGetter, "x")) + "|" +
    scalar(throwingGetter.x));

var throwingSetter = {};
Object.defineProperty(throwingSetter, "x", {
    configurable: true,
    set: function(value) {
        delete this.x;
        throw new SyntaxError("setter-" + value);
    }
});
emit("setter-exception", observe(function() {
    throwingSetter.x = 4;
    return "unreachable";
}));
emit("setter-exception-after",
    scalar(hasOwn(throwingSetter, "x")) + "|" +
    scalar(throwingSetter.x));

var finallyTrace = "";
function throwingInner() { throw new Error("inner-boom"); }
function throwingOuter() {
    try {
        return throwingInner();
    } finally {
        finallyTrace += "F";
    }
}
emit("nested-call-exception", observe(throwingOuter));
emit("nested-call-finally", scalar(finallyTrace));
