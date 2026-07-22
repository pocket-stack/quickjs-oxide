function fail(label, actual, expected) {
    throw new Error(
        "R3j private-accessor oracle assertion failed for " + label +
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

var r3jTranscript = [];
function emit(label, value) {
    r3jTranscript.push(label + "=" + value);
}

class AccessorPair {
    #backing = 40;
    get #value() { return this.#backing + 2; }
    set #value(value) { this.#backing = value; }
    read(receiver) { return receiver.#value; }
    write(receiver, value) { receiver.#value = value; return receiver.#value; }
    has(receiver) { return #value in receiver; }

    static #staticBacking = 40;
    static get #staticValue() { return this.#staticBacking + 2; }
    static set #staticValue(value) { this.#staticBacking = value; }
    static read(receiver) { return receiver.#staticValue; }
    static write(receiver, value) {
        receiver.#staticValue = value;
        return receiver.#staticValue;
    }
    static has(receiver) { return #staticValue in receiver; }
}
var accessorPair = new AccessorPair();
var basicPair = [
    accessorPair.read(accessorPair),
    accessorPair.write(accessorPair, 41),
    accessorPair.has(accessorPair),
    AccessorPair.read(AccessorPair),
    AccessorPair.write(AccessorPair, 41),
    AccessorPair.has(AccessorPair)
].join("|");
same(basicPair, "42|43|true|42|43|true", "instance and static accessor pairs");
emit("pair", basicPair);

class AccessorChild extends AccessorPair {}
var brandErrors = [
    accessorPair.has({}),
    AccessorPair.has(AccessorChild),
    errorText(function () { accessorPair.read({}); }),
    errorText(function () { accessorPair.read(1); }),
    errorText(function () { AccessorPair.read(AccessorChild); }),
    errorText(function () { accessorPair.has(1); })
].join("|");
same(
    brandErrors,
    "false|false|TypeError:invalid brand on object|TypeError:not an object|" +
        "TypeError:invalid brand on object|TypeError:invalid 'in' operand",
    "accessor brand and primitive errors"
);
emit("brand-errors", brandErrors);

class GetterOnly {
    get #value() { return 42; }
    read(receiver) { return receiver.#value; }
    write(receiver) { receiver.#value = 1; }
    has(receiver) { return #value in receiver; }
}
var getterOnly = new GetterOnly();
var getterOnlyResult = [
    getterOnly.read(getterOnly),
    getterOnly.has(getterOnly),
    errorText(function () { getterOnly.write(getterOnly); }),
    errorText(function () { getterOnly.write({}); })
].join("|");
same(
    getterOnlyResult,
    "42|true|TypeError:'#value' is read-only|TypeError:'#value' is read-only",
    "getter-only read-only write precedence"
);
emit("getter-only", getterOnlyResult);

class SetterOnly {
    set #value(value) { this.stored = value; }
    write(receiver, value) { receiver.#value = value; }
    read(receiver) { return receiver.#value; }
    has(receiver) { return #value in receiver; }
}
var setterOnly = new SetterOnly();
var unsupportedTypeReceiver = {"[unsupported type]": 1};
setterOnly.write(setterOnly, 42);
var setterOnlyResult = [
    setterOnly.stored,
    setterOnly.has(setterOnly),
    setterOnly.has(unsupportedTypeReceiver),
    errorText(function () { setterOnly.read(setterOnly); }),
    errorText(function () { setterOnly.write({}, 1); }),
    errorText(function () { setterOnly.has(1); })
].join("|");
same(
    setterOnlyResult,
    "42|false|true|TypeError:'#value' is read-only|" +
        "TypeError:invalid brand on object|TypeError:invalid 'in' operand",
    "setter-only private-in internal-tag quirk"
);
emit("setter-only-private-in", setterOnlyResult);

var getterFirstPartial = [];
class GetterFirstPartial {
    get #value() { return 42; }
    [(
        getterFirstPartial.push(errorText(function () { return ({}).#value; })),
        getterFirstPartial.push(errorText(function () { ({}).#value = 1; })),
        getterFirstPartial.push(errorText(function () {
            return #value in unsupportedTypeReceiver;
        })),
        "computed"
    )]() {}
    set #value(value) {}
}
var setterFirstPartial = [];
class SetterFirstPartial {
    set #value(value) {}
    [(
        setterFirstPartial.push(errorText(function () { return ({}).#value; })),
        setterFirstPartial.push(errorText(function () { ({}).#value = 1; })),
        setterFirstPartial.push(#value in unsupportedTypeReceiver),
        "computed"
    )]() {}
    get #value() { return 42; }
}
var partialResult = getterFirstPartial.join("|") + "||" + setterFirstPartial.join("|");
same(
    partialResult,
    "TypeError:expecting <brand> private field|TypeError:not an object|" +
        "TypeError:expecting <brand> private field||TypeError:not an object|" +
        "TypeError:expecting <brand> private field|true",
    "getter/setter partial initialization"
);
emit("partial-init", partialResult);

class InstanceOrder {
    first = (this.#value = 40);
    answer = this.#value + 2;
    get #value() { return this.backing; }
    set #value(value) { this.backing = value; }
}
class StaticOrder {
    static first = (this.#value = 40);
    static answer = this.#value + 2;
    static get #value() { return this.backing; }
    static set #value(value) { this.backing = value; }
}
var initializerOrder = [
    new InstanceOrder().answer,
    StaticOrder.answer
].join("|");
same(initializerOrder, "42|42", "accessor brand precedes instance and static fields");
emit("initializer-order", initializerOrder);

class BeforeSuperBase {
    constructor() { this.touchBeforeSuperReturn(); }
}
class BeforeSuperGetter extends BeforeSuperBase {
    touchBeforeSuperReturn() { return this.#value; }
    get #value() { return 42; }
}
class BeforeSuperSetter extends BeforeSuperBase {
    touchBeforeSuperReturn() { this.#value = 42; }
    set #value(value) {}
}
var beforeSuperResult = [
    errorText(function () { new BeforeSuperGetter(); }),
    errorText(function () { new BeforeSuperSetter(); })
].join("|");
same(
    beforeSuperResult,
    "TypeError:invalid brand on object|TypeError:invalid brand on object",
    "accessors are installed only after super returns"
);
emit("before-super-return", beforeSuperResult);

class SuperBase {
    get answer() { return this.base; }
    set answer(value) { this.base = value; }
    static get answer() { return this.base; }
    static set answer(value) { this.base = value; }
}
class SuperAccessors extends SuperBase {
    get #value() { return super.answer + 2; }
    set #value(value) { super.answer = value + 2; }
    read() { return this.#value; }
    write(value) { this.#value = value; return this.base; }
    static get #staticValue() { return super.answer + 2; }
    static set #staticValue(value) { super.answer = value + 2; }
    static read() { return this.#staticValue; }
    static write(value) { this.#staticValue = value; return this.base; }
}
var superAccessor = new SuperAccessors();
superAccessor.base = 40;
SuperAccessors.base = 40;
var superResult = [
    superAccessor.read(),
    superAccessor.write(40),
    SuperAccessors.read(),
    SuperAccessors.write(40)
].join("|");
same(superResult, "42|42|42|42", "private accessor HomeObject super");
emit("super", superResult);

class CaptureAccessors {
    #backing = 0;
    get #value() { return this.#backing; }
    set #value(value) { this.#backing = value; }
    exercise() {
        var arrowRead = () => this.#value;
        function nestedWrite(receiver, value) { receiver.#value = value; }
        nestedWrite(this, 40);
        var directEval = eval("this.#value = 41; this.#value");
        return [arrowRead(), directEval].join("|");
    }
    nestedClass() {
        return class {
            read(receiver) { return receiver.#value; }
            write(receiver, value) { receiver.#value = value; }
            has(receiver) { return #value in receiver; }
        };
    }
}
var captureAccessor = new CaptureAccessors();
var NestedAccessor = captureAccessor.nestedClass();
var nestedAccessor = new NestedAccessor();
var captureExercise = captureAccessor.exercise();
nestedAccessor.write(captureAccessor, 42);
var captureResult = [
    captureExercise,
    nestedAccessor.read(captureAccessor),
    nestedAccessor.has(captureAccessor)
].join("|");
same(captureResult, "41|41|42|true", "arrow nested eval and nested-class capture");
emit("capture", captureResult);

function makeAccessorClass() {
    return class {
        get #value() { return 42; }
        has(receiver) { return #value in receiver; }
        read(receiver) { return receiver.#value; }
    };
}
var FirstAccessorClass = makeAccessorClass();
var SecondAccessorClass = makeAccessorClass();
var firstAccessorReceiver = new FirstAccessorClass();
var secondAccessorReceiver = new SecondAccessorClass();
var reevaluation = [
    firstAccessorReceiver.has(firstAccessorReceiver),
    firstAccessorReceiver.has(secondAccessorReceiver),
    secondAccessorReceiver.has(firstAccessorReceiver),
    secondAccessorReceiver.has(secondAccessorReceiver),
    errorText(function () { firstAccessorReceiver.read(secondAccessorReceiver); })
].join("|");
same(
    reevaluation,
    "true|false|false|true|TypeError:invalid brand on object",
    "fresh accessor brand per class evaluation"
);
emit("reevaluation", reevaluation);

var sharedReceiver = {};
class SharedReceiverBase {
    constructor() { return sharedReceiver; }
}
class SharedReceiverDerived extends SharedReceiverBase {
    get #value() { return 42; }
}
new SharedReceiverDerived();
var duplicateBrandError = errorText(function () { new SharedReceiverDerived(); });
same(duplicateBrandError, "TypeError:private method is already present", "duplicate accessor brand insertion");
emit("duplicate-brand", duplicateBrandError);

var reentryChecks = [];
var reentryReceiver;
function reentryBoom() { throw 0; }
for (var reentryIndex = 0; reentryIndex < 3; reentryIndex++) {
    try {
        class ReenteredAccessor {
            #field = 0;
            get #value() { return this.#field + 2; }
            set #value(value) { this.#field = value; }
            [(
                reentryChecks.push(function (receiver) {
                    receiver.#value = 40;
                    return [#value in receiver, receiver.#value].join(":");
                }),
                reentryIndex < 2 ? reentryBoom() : "ok"
            )]() {}
        }
        reentryReceiver = new ReenteredAccessor();
    } catch (error) {}
}
var reentryResult = [
    reentryChecks[0](reentryReceiver),
    reentryChecks[1](reentryReceiver),
    reentryChecks[2](reentryReceiver)
].join("|");
same(
    reentryResult,
    "true:42|true:42|true:42",
    "abrupt accessor VarRef reentry"
);
emit("abrupt-reentry", reentryResult);

var duplicateSyntax = [
    errorText(function () { eval("class C { get #x() {} get #x() {} }"); }),
    errorText(function () { eval("class C { set #x(v) {} set #x(v) {} }"); }),
    errorText(function () { eval("class C { #x; get #x() {} }"); }),
    errorText(function () {
        eval("class C { static get #x() {} set #x(v) {} }");
    }),
    errorText(function () {
        eval("class C { get #x() { return 1; } set #x(v) {} }");
    })
].join("|");
same(
    duplicateSyntax,
    "SyntaxError:private class field is already defined|" +
        "SyntaxError:private class field is already defined|" +
        "SyntaxError:private class field is already defined|" +
        "SyntaxError:private class field is already defined|none",
    "private accessor duplicate parser rules"
);
emit("parser-duplicates", duplicateSyntax);

r3jTranscript.push("r3j-class-private-accessors-oracle=ok");
r3jTranscript.join("\n");
