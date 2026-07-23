function fail(label, actual, expected) {
    throw new Error(
        "R3l private-generator oracle assertion failed for " + label +
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

function descriptorText(descriptor) {
    return [
        descriptor.writable,
        descriptor.enumerable,
        descriptor.configurable
    ].join(":");
}

var r3lTranscript = [];
function emit(label, value) {
    r3lTranscript.push(label + "=" + value);
}

var parameterEffects = 0;
class PrivateGenerators {
    #value = 40;

    *#instance(first, second = (++parameterEffects, 2)) {
        yield this.#value;
        yield second;
        return this.#value + first + second;
    }
    getInstance() { return this.#instance; }
    start(first) { return this.#instance(first); }
    hasInstance(receiver) { return #instance in receiver; }
    overwrite(receiver) { receiver.#instance = 0; }

    static *#staticGenerator(value) {
        yield this.tag;
        return value;
    }
    static getStatic() { return this.#staticGenerator; }
    static startStatic(value) { return this.#staticGenerator(value); }
    static hasStatic(receiver) { return #staticGenerator in receiver; }
}
PrivateGenerators.tag = "C";

var receiver = new PrivateGenerators();
var secondReceiver = new PrivateGenerators();
var instanceGenerator = receiver.getInstance();
var instanceIterator = receiver.start(0);
var lazyResult = [
    parameterEffects,
    instanceIterator.next().value,
    parameterEffects,
    instanceIterator.next().value,
    instanceIterator.next().value,
    instanceIterator.next().done
].join("|");
same(lazyResult, "1|40|1|2|42|true", "parameters and resumable instance body");
emit("instance-resume", lazyResult);

var staticIterator = PrivateGenerators.startStatic(42);
var staticResult = [
    staticIterator.next().value,
    staticIterator.next().value,
    staticIterator.next().done,
    PrivateGenerators.hasStatic(PrivateGenerators),
    receiver.hasInstance(receiver),
    receiver.hasInstance(PrivateGenerators),
    PrivateGenerators.hasStatic(receiver)
].join("|");
same(staticResult, "C|42|true|true|true|false|false", "static generator and split brands");
emit("static-brand", staticResult);

var extractedStaticGenerator = PrivateGenerators.getStatic();
var extractedStaticIterator = extractedStaticGenerator.call({ tag: "X" }, 42);
var extractedStaticFirst = extractedStaticIterator.next();
var extractedStaticReturn = extractedStaticIterator.next();
var extractedStaticResult = [
    extractedStaticFirst.value,
    extractedStaticFirst.done,
    extractedStaticReturn.value,
    extractedStaticReturn.done
].join("|");
same(
    extractedStaticResult,
    "X|false|42|true",
    "extracted static private generator does not repeat the brand check"
);
emit("static-extracted-call", extractedStaticResult);

var prototypeDescriptor = Object.getOwnPropertyDescriptor(instanceGenerator, "prototype");
var extractedIterator = instanceGenerator.call(secondReceiver, 0, 2);
var reflectionResult = [
    instanceGenerator.name,
    instanceGenerator.length,
    Object.prototype.hasOwnProperty.call(instanceGenerator, "prototype"),
    descriptorText(prototypeDescriptor),
    Object.getPrototypeOf(instanceGenerator) === Object.getPrototypeOf(function* () {}),
    Object.getPrototypeOf(instanceGenerator.call(receiver, 0, 2)) ===
        instanceGenerator.prototype,
    instanceGenerator === secondReceiver.getInstance(),
    extractedIterator.next().value,
    Function.prototype.toString.call(instanceGenerator).indexOf("*#instance(") === 0,
    errorText(function () { new instanceGenerator(); })
].join("|");
same(
    reflectionResult,
    "#instance|1|true|true:false:false|true|true|true|40|true|" +
        "TypeError:#instance is not a constructor",
    "private generator reflection"
);
emit("reflection", reflectionResult);

var wrongReceiverEffects = 0;
class WrongReceiver {
    *#method(value) { yield value; }
    start(receiver) {
        return receiver.#method((wrongReceiverEffects++, 42));
    }
}
var wrongReceiver = new WrongReceiver();
var brandErrors = [
    errorText(function () { wrongReceiver.start({}); }),
    wrongReceiverEffects,
    errorText(function () { receiver.overwrite({}); }),
    errorText(function () { receiver.hasInstance(1); })
].join("|");
same(
    brandErrors,
    "TypeError:invalid brand on object|0|TypeError:'#instance' is read-only|" +
        "TypeError:invalid 'in' operand",
    "brand-before-argument and read-only errors"
);
emit("brand-errors", brandErrors);

class SuperBase {
    value() { return 40; }
    static value() { return 41; }
}
class SuperDerived extends SuperBase {
    *#instance() {
        yield super.value();
        return super.value() + 2;
    }
    start() { return this.#instance(); }

    static *#staticGenerator() {
        yield super.value();
        return super.value() + 1;
    }
    static start() { return this.#staticGenerator(); }
}
var superInstance = new SuperDerived().start();
var superStatic = SuperDerived.start();
var firstSuperValue = superInstance.next().value;
SuperBase.prototype.value = function () { return 50; };
var superResult = [
    firstSuperValue,
    superInstance.next().value,
    superStatic.next().value,
    superStatic.next().value
].join("|");
same(superResult, "40|52|41|42", "dynamic super survives private generator suspension");
emit("super", superResult);

class CaptureGenerator {
    #value = 40;
    *#method() {
        yield eval("this.#value");
        yield (() => this.#value + 1)();
        return this.#value + 2;
    }
    start() { return this.#method(); }
}
var capture = new CaptureGenerator().start();
var captureResult = [
    capture.next().value,
    capture.next().value,
    capture.next().value,
    capture.next().done
].join("|");
same(captureResult, "40|41|42|true", "eval arrow and private capture across suspension");
emit("capture", captureResult);

class DelegatingGenerator {
    *#method() {
        return yield* [40, 41];
    }
    start() { return this.#method(); }
}
var delegated = new DelegatingGenerator().start();
var delegationResult = [
    delegated.next().value,
    delegated.next().value,
    delegated.next().value,
    delegated.next().done
].join("|");
same(delegationResult, "40|41||true", "private yield-star lifecycle");
emit("yield-star", delegationResult);

function makeGeneratorClass() {
    return class {
        *#method() { yield 42; }
        getMethod() { return this.#method; }
        hasMethod(receiver) { return #method in receiver; }
    };
}
var FirstGeneratorClass = makeGeneratorClass();
var SecondGeneratorClass = makeGeneratorClass();
var firstGeneratorReceiver = new FirstGeneratorClass();
var secondGeneratorReceiver = new SecondGeneratorClass();
var reevaluationResult = [
    firstGeneratorReceiver.getMethod() === secondGeneratorReceiver.getMethod(),
    firstGeneratorReceiver.hasMethod(firstGeneratorReceiver),
    firstGeneratorReceiver.hasMethod(secondGeneratorReceiver),
    secondGeneratorReceiver.hasMethod(firstGeneratorReceiver),
    secondGeneratorReceiver.hasMethod(secondGeneratorReceiver)
].join("|");
same(reevaluationResult, "false|true|false|false|true", "fresh callable and brand per class");
emit("reevaluation", reevaluationResult);

class StaticGeneratorChild extends PrivateGenerators {}
var subclassResult = [
    PrivateGenerators.hasStatic(StaticGeneratorChild),
    errorText(function () { StaticGeneratorChild.startStatic(1); })
].join("|");
same(
    subclassResult,
    "false|TypeError:invalid brand on object",
    "static private generator rejects subclass receiver"
);
emit("static-subclass", subclassResult);

function* outerClassEvaluation() {
    var key = yield "key";
    class ResumedClass {
        *#method() { yield 42; }
        [key]() { return this.#method(); }
    }
    return new ResumedClass()[key]().next().value;
}
var outer = outerClassEvaluation();
var outerFirst = outer.next();
var outerSecond = outer.next("start");
var outerResult = [
    outerFirst.value,
    outerFirst.done,
    outerSecond.value,
    outerSecond.done
].join("|");
same(outerResult, "key|false|42|true", "class evaluation resumes into private generator publication");
emit("outer-resume", outerResult);

r3lTranscript.push("r3l-class-private-generators-oracle=ok");
r3lTranscript.join("\n");
