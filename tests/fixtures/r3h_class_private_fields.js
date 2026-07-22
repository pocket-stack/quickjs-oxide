function fail(label, actual, expected) {
    throw new Error(
        "R3h private-field oracle assertion failed for " + label +
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

var r3hTranscript = [];
function emit(label, value) {
    r3hTranscript.push(label + "=" + value);
}

class FirstIdentity {
    #same = 41;
    read() { return this.#same; }
    static has(receiver) { return #same in receiver; }
}
class SecondIdentity {
    #same = 42;
    read() { return this.#same; }
    static has(receiver) { return #same in receiver; }
}
var firstIdentity = new FirstIdentity();
var secondIdentity = new SecondIdentity();
var identityResult = [
    firstIdentity.read(),
    secondIdentity.read(),
    FirstIdentity.has(firstIdentity),
    FirstIdentity.has(secondIdentity),
    SecondIdentity.has(firstIdentity),
    SecondIdentity.has(secondIdentity)
].join("|");
same(identityResult, "41|42|true|false|false|true", "fresh identity per declaration");
emit("identity", identityResult);

class Operations {
    #value = 1;
    read() { return this.#value; }
    write(value) { return this.#value = value; }
    add(value) { return this.#value += value; }
    postIncrement() { return this.#value++; }
    preIncrement() { return ++this.#value; }
    has(receiver) { return #value in receiver; }
}
var operations = new Operations();
var operationsResult = [
    operations.read(),
    operations.write(40),
    operations.add(2),
    operations.postIncrement(),
    operations.preIncrement(),
    operations.read(),
    operations.has(operations),
    operations.has({})
].join("|");
same(operationsResult, "1|40|42|42|44|44|true|false", "private reference operations");
emit("operations", operationsResult);

var fieldOrder = [];
class OrderedFields {
    static #staticFirst = (fieldOrder.push("static:first"), 1);
    #instanceFirst = (fieldOrder.push("instance:first"), 2);
    static #staticSecond = (fieldOrder.push("static:second"), 3);
    #instanceSecond = (fieldOrder.push("instance:second"), 4);
    values() {
        return this.#instanceFirst + ":" + this.#instanceSecond;
    }
    static values() {
        return this.#staticFirst + ":" + this.#staticSecond;
    }
}
same(fieldOrder.join(","), "static:first,static:second", "static private field order");
var orderedFields = new OrderedFields();
same(
    fieldOrder.join(","),
    "static:first,static:second,instance:first,instance:second",
    "instance private field order"
);
same(orderedFields.values(), "2:4", "ordered instance values");
same(OrderedFields.values(), "1:3", "ordered static values");
emit("field-order", fieldOrder.join(","));

class LockedBase {
    constructor() {
        return Object.preventExtensions({ marker: "replacement" });
    }
}
class LockedDerived extends LockedBase {
    #answer = 42;
    static read(receiver) { return receiver.#answer; }
    static has(receiver) { return #answer in receiver; }
}
var locked = new LockedDerived();
var lockedPrototype = Object.create(locked);
var lockedReflection = [
    Object.isExtensible(locked),
    Object.getOwnPropertyNames(locked).join(","),
    Object.getOwnPropertySymbols(locked).length,
    "#answer" in locked,
    LockedDerived.has(locked),
    LockedDerived.has(lockedPrototype),
    LockedDerived.read(locked)
].join("|");
same(
    lockedReflection,
    "false|marker|0|false|true|false|42",
    "private fields bypass extensibility and remain hidden and own-only"
);
var prototypeReadError = errorText(function () { LockedDerived.read(lockedPrototype); });
same(
    prototypeReadError,
    "TypeError:private class field '#answer' does not exist",
    "private lookup does not traverse prototypes"
);
emit("nonextensible-own-reflection", lockedReflection + "|" + prototypeReadError);

class StaticFields {
    static #value = 40;
    static add(value) { return this.#value += value; }
    static read() { return this.#value; }
    static has(receiver) { return #value in receiver; }
}
class StaticFieldsChild extends StaticFields {}
var staticResult = [
    StaticFields.add(2),
    StaticFields.read(),
    StaticFields.has(StaticFields),
    StaticFields.has(StaticFieldsChild)
].join("|");
same(staticResult, "42|42|true|false", "static private field identity");
var inheritedStaticError = errorText(function () { StaticFieldsChild.read(); });
same(
    inheritedStaticError,
    "TypeError:private class field '#value' does not exist",
    "static private field is not inherited"
);
emit("static", staticResult + "|" + inheritedStaticError);

class CapturedFields {
    #value = 40;
    exercise() {
        var arrow = () => this.#value + 1;
        function nested(receiver) { return receiver.#value + 2; }
        var directEval = eval("this.#value += 2; this.#value");
        return [arrow(), nested(this), directEval, this.#value].join("|");
    }
    nestedClass() {
        return class NestedReader {
            read(receiver) { return receiver.#value; }
            has(receiver) { return #value in receiver; }
        };
    }
}
var capturedFields = new CapturedFields();
var captureResult = capturedFields.exercise();
same(captureResult, "43|44|42|42", "arrow nested function and direct eval capture");
var NestedReader = capturedFields.nestedClass();
var nestedReader = new NestedReader();
var nestedClassResult = [nestedReader.read(capturedFields), nestedReader.has(capturedFields)].join("|");
same(nestedClassResult, "42|true", "nested class captures enclosing private name");
emit("capture", captureResult + "|" + nestedClassResult);

class ErrorFields {
    #value = 0;
    static read(receiver) { return receiver.#value; }
    static write(receiver) { return receiver.#value = 1; }
    static has(receiver) { return #value in receiver; }
}
var errorResult = [
    errorText(function () { ErrorFields.read({}); }),
    errorText(function () { ErrorFields.write({}); }),
    errorText(function () { ErrorFields.read(1); }),
    errorText(function () { ErrorFields.write(1); }),
    errorText(function () { ErrorFields.has(1); })
].join("|");
same(
    errorResult,
    "TypeError:private class field '#value' does not exist|" +
        "TypeError:private class field '#value' does not exist|" +
        "TypeError:not an object|TypeError:not an object|" +
        "TypeError:invalid 'in' operand",
    "private field error text"
);
emit("errors", errorResult);

var forwardOrder = [];
var forwardInResult;
class ForwardIn {
    [(
        forwardOrder.push("computed-before"),
        forwardInResult = #later in {},
        "computed"
    )] = (forwardOrder.push("public-instance"), 0);
    #later = (forwardOrder.push("private-instance"), 42);
    static after = (forwardOrder.push("static-after"), 0);
}
same(forwardInResult, false, "forward private-in sees an uninitialized name");
same(
    forwardOrder.join(","),
    "computed-before,static-after",
    "computed key precedes static initialization"
);
new ForwardIn();
same(
    forwardOrder.join(","),
    "computed-before,static-after,public-instance,private-instance",
    "instance fields retain source order"
);
var forwardGetError = errorText(function () {
    class ForwardGet {
        [({}).#later] = 0;
        #later = 42;
    }
});
same(forwardGetError, "TypeError:not a symbol", "forward private get error");
emit("forward-order", forwardInResult + "|" + forwardOrder.join(",") + "|" + forwardGetError);

r3hTranscript.push("r3h-class-private-fields-oracle=ok");
r3hTranscript.join("\n");
