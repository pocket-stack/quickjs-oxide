function fail(label, actual, expected) {
    throw new Error(
        "R3i private-method oracle assertion failed for " + label +
        ": expected " + expected + ", got " + actual
    );
}

function same(actual, expected, label) {
    if (actual !== expected)
        fail(label, String(actual), String(expected));
}

function errorText(thunk) {
    try {
        thunk();
        return "none";
    } catch (error) {
        return error.name + ":" + error.message;
    }
}

var r3iTranscript = [];
function emit(label, value) {
    r3iTranscript.push(label + "=" + value);
}

class Methods {
    #method(left, right) { return this.tag + ":" + left + ":" + right; }
    getMethod() { return this.#method; }
    callMethod(left, right) { return this.#method(left, right); }
    hasMethod(receiver) { return #method in receiver; }
    overwrite(receiver) { receiver.#method = 0; }

    static #staticMethod(value) { return this.tag + ":" + value; }
    static getStaticMethod() { return this.#staticMethod; }
    static callStaticMethod(value) { return this.#staticMethod(value); }
    static hasStaticMethod(receiver) { return #staticMethod in receiver; }
}
Methods.tag = "C";
var firstMethodReceiver = new Methods();
firstMethodReceiver.tag = "A";
var secondMethodReceiver = new Methods();
secondMethodReceiver.tag = "B";
var firstMethod = firstMethodReceiver.getMethod();
var methodShape = [
    firstMethodReceiver.callMethod(1, 2),
    firstMethod.name,
    firstMethod.length,
    "prototype" in firstMethod,
    firstMethod === secondMethodReceiver.getMethod(),
    firstMethod.call({ tag: "X" }, 3, 4)
].join("|");
same(methodShape, "A:1:2|#method|2|false|true|X:3:4", "shared method closure and call receiver");
emit("method-shape", methodShape);

var brandSplit = [
    firstMethodReceiver.hasMethod(firstMethodReceiver),
    firstMethodReceiver.hasMethod(Methods),
    Methods.hasStaticMethod(Methods),
    Methods.hasStaticMethod(firstMethodReceiver),
    Methods.callStaticMethod(42),
    Methods.getStaticMethod().name
].join("|");
same(brandSplit, "true|false|true|false|C:42|#staticMethod", "instance and static brands");
emit("brand-split", brandSplit);

class MethodsChild extends Methods {}
var brandErrors = [
    Methods.hasStaticMethod(MethodsChild),
    errorText(function () { MethodsChild.callStaticMethod(1); }),
    errorText(function () { firstMethodReceiver.getMethod.call({}); }),
    errorText(function () { Methods.prototype.getMethod.call(1); }),
    errorText(function () { firstMethodReceiver.overwrite({}); }),
    errorText(function () { firstMethodReceiver.hasMethod(1); })
].join("|");
same(
    brandErrors,
    "false|TypeError:invalid brand on object|TypeError:invalid brand on object|" +
        "TypeError:not an object|TypeError:'#method' is read-only|" +
        "TypeError:invalid 'in' operand",
    "brand errors and readonly precedence"
);
emit("brand-errors", brandErrors);

function makeMethodClass() {
    return class {
        #method() { return 42; }
        getMethod() { return this.#method; }
        hasMethod(receiver) { return #method in receiver; }
    };
}
var FirstMethodClass = makeMethodClass();
var SecondMethodClass = makeMethodClass();
var firstClassReceiver = new FirstMethodClass();
var secondClassReceiver = new SecondMethodClass();
var reevaluation = [
    firstClassReceiver.getMethod() === secondClassReceiver.getMethod(),
    firstClassReceiver.hasMethod(firstClassReceiver),
    firstClassReceiver.hasMethod(secondClassReceiver),
    secondClassReceiver.hasMethod(firstClassReceiver),
    secondClassReceiver.hasMethod(secondClassReceiver)
].join("|");
same(reevaluation, "false|true|false|false|true", "fresh brand and closure per class evaluation");
emit("reevaluation", reevaluation);

class SuperBase {
    value() { return 40; }
    static value() { return 40; }
}
class SuperDerived extends SuperBase {
    #method() { return super.value() + 2; }
    callMethod() { return this.#method(); }
    static #staticMethod() { return super.value() + 2; }
    static callMethod() { return this.#staticMethod(); }
}
var superResult = [new SuperDerived().callMethod(), SuperDerived.callMethod()].join("|");
same(superResult, "42|42", "private method HomeObject super");
emit("super", superResult);

class BaseBrand {
    #method() { return 40; }
    callBase() { return this.#method(); }
    hasBase(receiver) { return #method in receiver; }
}
class DerivedBrand extends BaseBrand {
    #method() { return 42; }
    callDerived() { return this.#method(); }
    hasDerived(receiver) { return #method in receiver; }
}
var derivedReceiver = new DerivedBrand();
var derivedResult = [
    derivedReceiver.callBase(),
    derivedReceiver.callDerived(),
    derivedReceiver.hasBase(derivedReceiver),
    derivedReceiver.hasDerived(derivedReceiver)
].join("|");
same(derivedResult, "40|42|true|true", "base and derived brands coexist");
emit("derived", derivedResult);

class LockedMethodBase {
    constructor() { return Object.preventExtensions({ tag: "locked" }); }
}
class LockedMethodDerived extends LockedMethodBase {
    #method() { return this.tag; }
    read() { return this.#method(); }
    has(receiver) { return #method in receiver; }
}
var lockedMethodReceiver = new LockedMethodDerived();
var lockedMethodPrototype = Object.create(lockedMethodReceiver);
var lockedMethodResult = [
    Object.isExtensible(lockedMethodReceiver),
    Object.getOwnPropertyNames(lockedMethodReceiver).join(","),
    Object.getOwnPropertySymbols(lockedMethodReceiver).length,
    LockedMethodDerived.prototype.has.call(lockedMethodReceiver, lockedMethodReceiver),
    LockedMethodDerived.prototype.has.call(lockedMethodReceiver, lockedMethodPrototype),
    LockedMethodDerived.prototype.read.call(lockedMethodReceiver)
].join("|");
same(lockedMethodResult, "false|tag|0|true|false|locked", "hidden own brand bypasses extensibility");
emit("nonextensible-own-reflection", lockedMethodResult);

class CaptureMethod {
    #method(value) { return value + 1; }
    exercise() {
        var arrow = () => this.#method(39);
        function nested(receiver) { return receiver.#method(40); }
        return [arrow(), nested(this), eval("this.#method(41)")].join("|");
    }
    nestedClass() {
        return class {
            read(receiver) { return receiver.#method(41); }
            has(receiver) { return #method in receiver; }
        };
    }
}
var captureMethodReceiver = new CaptureMethod();
var NestedMethodReader = captureMethodReceiver.nestedClass();
var nestedMethodReader = new NestedMethodReader();
var captureResult = [
    captureMethodReceiver.exercise(),
    nestedMethodReader.read(captureMethodReceiver),
    nestedMethodReader.has(captureMethodReceiver)
].join("|");
same(captureResult, "40|41|42|42|true", "arrow nested eval and nested-class capture");
emit("capture", captureResult);

var forwardInResult;
class ForwardMethod {
    [(forwardInResult = #later in {}, "computed")] = 0;
    #later() { return 42; }
}
same(forwardInResult, false, "forward private method in sees an uninitialized cell");
var forwardGetError = errorText(function () {
    class ForwardMethodGet {
        [({}).#later] = 0;
        #later() { return 42; }
    }
});
same(forwardGetError, "TypeError:not an object", "forward private method get error");
var prebrandError = errorText(function () {
    class BeforeBrand {
        #method() { return 42; }
        [(#method in {}, "computed")] = 0;
    }
});
same(prebrandError, "TypeError:expecting <brand> private field", "method exists before brand publication");
var prebrandPrimitiveGetError = errorText(function () {
    class BeforePrimitiveBrand {
        #method() { return 42; }
        [(1).#method] = 0;
    }
});
same(prebrandPrimitiveGetError, "TypeError:expecting <brand> private field", "brand lookup precedes primitive receiver validation");
emit("forward-order", forwardInResult + "|" + forwardGetError + "|" + prebrandError + "|" + prebrandPrimitiveGetError);

var unsupportedTypeReceiver = {"[unsupported type]": 1};
var uninitializedMethodIn;
class UninitializedMethodIn {
    [(uninitializedMethodIn = #later in unsupportedTypeReceiver, "computed")] = 0;
    #later() { return 42; }
}
var uninitializedFieldIn;
class UninitializedFieldIn {
    [(uninitializedFieldIn = #later in unsupportedTypeReceiver, "computed")] = 0;
    #later = 42;
}
same(uninitializedMethodIn, true, "uninitialized private method in uses QuickJS internal-tag atom");
same(uninitializedFieldIn, true, "uninitialized private field in uses QuickJS internal-tag atom");
emit("uninitialized-private-in", uninitializedMethodIn + "|" + uninitializedFieldIn);

class InstanceOrder {
    answer = this.#method();
    #method() { return 42; }
}
class StaticOrder {
    static answer = this.#method();
    static #method() { return 42; }
}
var initializerOrder = [new InstanceOrder().answer, StaticOrder.answer].join("|");
same(initializerOrder, "42|42", "brand precedes instance and static fields");
emit("initializer-order", initializerOrder);

var sharedReceiver = {};
class SharedReceiverBase {
    constructor() { return sharedReceiver; }
}
class SharedReceiverDerived extends SharedReceiverBase {
    #method() { return 42; }
}
new SharedReceiverDerived();
var duplicateBrandError = errorText(function () { new SharedReceiverDerived(); });
same(duplicateBrandError, "TypeError:private method is already present", "duplicate brand insertion");
emit("duplicate-brand", duplicateBrandError);

var reentryChecks = [];
var reentryReceiver;
function reentryBoom() { throw 0; }
for (var reentryIndex = 0; reentryIndex < 3; reentryIndex++) {
    try {
        class ReenteredMethod {
            #field = 41;
            #method() { return 42; }
            [(
                reentryChecks.push(function (receiver) {
                    return [
                        #field in receiver,
                        receiver.#field,
                        #method in receiver,
                        receiver.#method()
                    ].join(":");
                }),
                reentryIndex < 2 ? reentryBoom() : "ok"
            )]() {}
        }
        reentryReceiver = new ReenteredMethod();
    } catch (error) {}
}
var reentryResult = [
    reentryChecks[0](reentryReceiver),
    reentryChecks[1](reentryReceiver),
    reentryChecks[2](reentryReceiver)
].join("|");
same(
    reentryResult,
    "true:41:true:42|true:41:true:42|true:41:true:42",
    "abrupt class-scope VarRef reentry"
);
emit("abrupt-reentry", reentryResult);

r3iTranscript.push("r3i-class-private-methods-oracle=ok");
r3iTranscript.join("\n");
