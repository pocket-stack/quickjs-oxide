function fail(label, actual, expected) {
    throw new Error(
        "R3g class-init oracle assertion failed for " + label +
        ": expected " + expected + ", got " + actual
    );
}

function same(actual, expected, label) {
    if (actual !== expected)
        fail(label, String(actual), String(expected));
}

function errorName(source) {
    try {
        eval(source);
        return "none";
    } catch (error) {
        return error.name;
    }
}

var r3gTranscript = [];
function emit(label, value) {
    r3gTranscript.push(label + "=" + value);
}

var order = [];
var keyCounts = {};
function computedKey(name) {
    keyCounts[name] = (keyCounts[name] || 0) + 1;
    order.push("key:" + name);
    return name;
}

class Ordered {
    [computedKey("i1")] = (order.push("instance:i1"), 11);
    static [computedKey("s1")] = (order.push("static:s1"), 21);
    static { order.push("block:1"); }
    [computedKey("i2")] = (order.push("instance:i2"), 12);
    static [computedKey("s2")] = (order.push("static:s2"), 22);
    static { order.push("block:2"); }
    constructor() { order.push("body"); }
}

var afterClass = order.join(",");
same(
    afterClass,
    "key:i1,key:s1,key:i2,key:s2,static:s1,block:1,static:s2,block:2",
    "all computed keys precede ordered static initialization"
);
var orderedFirst = new Ordered();
var orderedSecond = new Ordered();
same(
    order.join(","),
    afterClass +
        ",instance:i1,instance:i2,body,instance:i1,instance:i2,body",
    "instance fields run for every construction in source order"
);
same(
    [keyCounts.i1, keyCounts.s1, keyCounts.i2, keyCounts.s2].join(","),
    "1,1,1,1",
    "computed keys are evaluated exactly once"
);
same(
    [orderedFirst.i1, orderedFirst.i2, orderedSecond.i1, orderedSecond.i2].join(","),
    "11,12,11,12",
    "computed instance field values"
);
emit("computed-order", order.join(","));
emit("computed-counts", [keyCounts.i1, keyCounts.s1, keyCounts.i2, keyCounts.s2].join(","));

var baseOrder = [];
class BaseTiming {
    field = (baseOrder.push("field"), 42);
    constructor(value = (baseOrder.push("parameter"), 1)) {
        baseOrder.push("body");
        this.value = value;
    }
}
var baseTiming = new BaseTiming();
same(baseOrder.join(","), "field,parameter,body", "base field timing");
same(baseTiming.field, 42, "base field value");
emit("base-order", baseOrder.join(","));

var replacement = { kind: "replacement" };
var derivedOrder = [];
class ReplacementBase {
    constructor() {
        derivedOrder.push("super");
        return replacement;
    }
}
class ReplacementDerived extends ReplacementBase {
    field = (derivedOrder.push("field"), 42);
    constructor() {
        derivedOrder.push("before");
        super();
        derivedOrder.push("after");
    }
}
var replaced = new ReplacementDerived();
same(replaced, replacement, "derived replacement object identity");
same(replaced.field, 42, "derived field is defined on replacement object");
same(derivedOrder.join(","), "before,super,field,after", "derived field timing");
emit("derived-replacement", derivedOrder.join(",") + "|" + replaced.kind + "|" + replaced.field);

var parentCalls = 0;
var fieldCalls = 0;
class TwiceBase {
    constructor() { parentCalls++; }
}
class TwiceDerived extends TwiceBase {
    field = (fieldCalls++, 42);
    constructor() {
        super();
        try {
            super();
        } catch (error) {
            this.secondSuperError = error.name;
        }
    }
}
var twice = new TwiceDerived();
same(parentCalls, 2, "second super invokes the base constructor");
same(fieldCalls, 1, "second super does not repeat derived fields");
same(twice.secondSuperError, "ReferenceError", "second super error kind");
emit("second-super", [parentCalls, fieldCalls, twice.secondSuperError].join("|"));

var inheritedSetterCalls = 0;
class DescriptorBase {}
Object.defineProperty(DescriptorBase.prototype, "value", {
    configurable: true,
    set: function () { inheritedSetterCalls++; }
});
Object.defineProperty(DescriptorBase, "staticValue", {
    configurable: true,
    set: function () { inheritedSetterCalls++; }
});
class DescriptorDerived extends DescriptorBase {
    value = 41;
    static staticValue = 42;
}
var descriptorInstance = new DescriptorDerived();
var instanceDescriptor = Object.getOwnPropertyDescriptor(descriptorInstance, "value");
var staticDescriptor = Object.getOwnPropertyDescriptor(DescriptorDerived, "staticValue");
same(inheritedSetterCalls, 0, "fields bypass inherited setters");
same(
    [instanceDescriptor.value, instanceDescriptor.writable,
        instanceDescriptor.enumerable, instanceDescriptor.configurable].join(","),
    "41,true,true,true",
    "instance field descriptor"
);
same(
    [staticDescriptor.value, staticDescriptor.writable,
        staticDescriptor.enumerable, staticDescriptor.configurable].join(","),
    "42,true,true,true",
    "static field descriptor"
);
emit(
    "define-field",
    inheritedSetterCalls + "|" +
        [instanceDescriptor.value, instanceDescriptor.writable,
            instanceDescriptor.enumerable, instanceDescriptor.configurable].join(",") + "|" +
        [staticDescriptor.value, staticDescriptor.writable,
            staticDescriptor.enumerable, staticDescriptor.configurable].join(",")
);

var inferredSymbol = Symbol("field-symbol");
class InferredNames {
    fn = function () {};
    klass = class {};
    [inferredSymbol] = () => 1;
    static sfn = function () {};
    static ["sklass"] = class {};
    static [inferredSymbol] = function () {};
}
var inferred = new InferredNames();
var inferredNames = [
    inferred.fn.name,
    inferred.klass.name,
    inferred[inferredSymbol].name,
    InferredNames.sfn.name,
    InferredNames.sklass.name,
    InferredNames[inferredSymbol].name
].join("|");
same(
    inferredNames,
    "fn|klass|[field-symbol]|sfn|sklass|[field-symbol]",
    "anonymous function and class field name inference"
);
emit("inferred-names", inferredNames);

class InitialHome {}
Object.defineProperty(InitialHome.prototype, "answer", {
    configurable: true,
    get: function () { return this.seed + 1; }
});
class AlternateHome {}
Object.defineProperty(AlternateHome.prototype, "answer", {
    configurable: true,
    get: function () { return this.seed + 2; }
});
class InstanceHome extends InitialHome {
    seed = 40;
    viaSuper = super.answer;
}
Object.setPrototypeOf(InstanceHome.prototype, AlternateHome.prototype);
var instanceHome = new InstanceHome();

class InitialStaticHome {}
Object.defineProperty(InitialStaticHome, "answer", {
    configurable: true,
    get: function () { return this.seed + 1; }
});
class AlternateStaticHome {}
Object.defineProperty(AlternateStaticHome, "answer", {
    configurable: true,
    get: function () { return this.seed + 2; }
});
class StaticHome extends InitialStaticHome {
    static seed = 40;
    static redirect = (Object.setPrototypeOf(StaticHome, AlternateStaticHome), 0);
    static viaSuper = super.answer;
    static { this.blockSuper = super.answer + 1; }
}
same(instanceHome.viaSuper, 42, "instance field live home object and receiver");
same(StaticHome.viaSuper, 42, "static field live home object and receiver");
same(StaticHome.blockSuper, 43, "static block live home object and receiver");
emit("home-object", [instanceHome.viaSuper, StaticHome.viaSuper, StaticHome.blockSuper].join("|"));

class Contexts {
    instanceThis = this;
    instanceTarget = new.target;
    static staticThis = this;
    static staticTarget = new.target;
    static {
        this.blockThis = this;
        this.blockTarget = new.target;
        this.nestedArguments = (function () { return arguments.length; })(1, 2);
    }
}
var contexts = new Contexts();
same(contexts.instanceThis, contexts, "instance initializer this");
same(contexts.instanceTarget, undefined, "instance initializer new.target");
same(Contexts.staticThis, Contexts, "static field this");
same(Contexts.staticTarget, undefined, "static field new.target");
same(Contexts.blockThis, Contexts, "static block this");
same(Contexts.blockTarget, undefined, "static block new.target");
same(Contexts.nestedArguments, 2, "nested ordinary function arguments in static block");
var instanceArgumentsError = errorName("(class { field = arguments; })");
var staticArgumentsError = errorName("(class { static field = arguments; })");
var blockArgumentsError = errorName("(class { static { arguments; } })");
same(instanceArgumentsError, "SyntaxError", "instance field arguments early error");
same(staticArgumentsError, "SyntaxError", "static field arguments early error");
same(blockArgumentsError, "SyntaxError", "static block arguments early error");
emit(
    "contexts",
    [contexts.instanceThis === contexts, String(contexts.instanceTarget),
        Contexts.staticThis === Contexts, String(Contexts.staticTarget),
        Contexts.blockThis === Contexts, String(Contexts.blockTarget),
        Contexts.nestedArguments, instanceArgumentsError,
        staticArgumentsError, blockArgumentsError].join("|")
);

var scopeClosure;
var secondBlockScope;
class StaticScope {
    static {
        var blockVar = "var";
        let blockLet = "let";
        scopeClosure = function () { return blockVar + ":" + blockLet; };
    }
    static {
        secondBlockScope = typeof blockVar + ":" + typeof blockLet;
    }
}
same(scopeClosure(), "var:let", "static block closure captures block scope");
same(secondBlockScope, "undefined:undefined", "static block scopes are independent");
same(typeof blockVar, "undefined", "static block var does not leak");
same(typeof blockLet, "undefined", "static block lexical does not leak");
emit("static-scope", scopeClosure() + "|" + secondBlockScope);

var abruptMarker = { marker: 42 };
var computedAbruptLog = [];
var computedCaught;
try {
    (class {
        static early = (computedAbruptLog.push("static-early"), 1);
        [(computedAbruptLog.push("key-throw"), (function () { throw abruptMarker; })())] = 2;
    });
} catch (error) {
    computedCaught = error;
}
same(computedCaught, abruptMarker, "computed key abrupt value");
same(computedAbruptLog.join(","), "key-throw", "computed key abrupt precedes static init");

var staticAbruptLog = [];
var staticCaught;
try {
    (class {
        static first = (staticAbruptLog.push("field"), 1);
        static {
            staticAbruptLog.push("block");
            throw abruptMarker;
        }
        static later = (staticAbruptLog.push("later-field"), 2);
        static { staticAbruptLog.push("later-block"); }
    });
} catch (error) {
    staticCaught = error;
}
same(staticCaught, abruptMarker, "static block abrupt value");
same(staticAbruptLog.join(","), "field,block", "static block abrupt stops later elements");

var instanceAbruptLog = [];
class InstanceAbrupt {
    first = (instanceAbruptLog.push("first"), 1);
    second = (instanceAbruptLog.push("second"), (function () { throw abruptMarker; })());
    later = (instanceAbruptLog.push("later"), 3);
    constructor() { instanceAbruptLog.push("body"); }
}
var instanceCaught;
try {
    new InstanceAbrupt();
} catch (error) {
    instanceCaught = error;
}
same(instanceCaught, abruptMarker, "instance initializer abrupt value");
same(instanceAbruptLog.join(","), "first,second", "instance abrupt stops fields and body");
emit(
    "abrupt",
    computedAbruptLog.join(",") + "|" +
        staticAbruptLog.join(",") + "|" + instanceAbruptLog.join(",")
);

var prototypeFieldError = errorName("class PrototypeField { prototype; }");
same(prototypeFieldError, "SyntaxError", "pinned QuickJS instance prototype field quirk");
emit("prototype-field-quirk", prototypeFieldError);

r3gTranscript.push("r3g-class-public-init-oracle=ok");
r3gTranscript.join("\n");
