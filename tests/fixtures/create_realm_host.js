var createRealmTranscript = [];

function createRealmEmit(label, value) {
    createRealmTranscript.push(label + "=" + value);
}

var createRealmParent = $262;
var createRealmChild = createRealmParent.createRealm();
var createRealmGrandchild = createRealmChild.createRealm.call({ ignored: true }, 1, 2);
var createRealmChildGlobal = createRealmChild.global;
var createRealmGrandchildGlobal = createRealmGrandchild.global;
var createRealmProperties = [
    "detachArrayBuffer",
    "evalScript",
    "codePointRange",
    "global",
    "createRealm",
    "gc"
];
var createRealmDescriptors = createRealmProperties.map(function (name) {
    var descriptor = Object.getOwnPropertyDescriptor(createRealmChild, name);
    return descriptor.writable && descriptor.enumerable && descriptor.configurable;
});
var createRealmGlobalDescriptor = Object.getOwnPropertyDescriptor(
    createRealmChildGlobal,
    "$262"
);

createRealmEmit(
    "shape",
    [
        createRealmChild !== createRealmParent,
        createRealmGrandchild !== createRealmChild,
        createRealmChildGlobal !== globalThis,
        createRealmChildGlobal.$262 === createRealmChild,
        createRealmGrandchildGlobal.$262 === createRealmGrandchild,
        createRealmChildGlobal.globalThis === createRealmChildGlobal,
        Object.getPrototypeOf(createRealmChild) === createRealmChildGlobal.Object.prototype,
        createRealmDescriptors.join(""),
        createRealmGlobalDescriptor.writable,
        createRealmGlobalDescriptor.enumerable,
        createRealmGlobalDescriptor.configurable
    ].join("|")
);

createRealmEmit(
    "functions",
    [
        createRealmChild.evalScript.name,
        createRealmChild.evalScript.length,
        Object.prototype.hasOwnProperty.call(createRealmChild.evalScript, "prototype"),
        createRealmChild.createRealm.name,
        createRealmChild.createRealm.length,
        Object.prototype.hasOwnProperty.call(createRealmChild.createRealm, "prototype")
    ].join("|")
);

createRealmEmit(
    "intrinsics",
    [
        createRealmChildGlobal.Object !== Object,
        createRealmChildGlobal.Function !== Function,
        createRealmChildGlobal.Error !== Error,
        createRealmChildGlobal.Symbol.iterator === Symbol.iterator,
        createRealmChildGlobal.Symbol.for("quickjs-oxide") === Symbol.for("quickjs-oxide"),
        createRealmChildGlobal.Symbol("local") !== Symbol("local")
    ].join("|")
);

var createRealmCoercions = [];
var createRealmSource = {
    toString: function () {
        createRealmCoercions.push(this === createRealmSource);
        return "globalThis.realmPersistent = 40; realmPersistent + 2";
    }
};
var createRealmFirstResult = createRealmChild.evalScript.call(
    { ignored: true },
    createRealmSource,
    "ignored"
);
var createRealmSecondResult = createRealmChild.evalScript("realmPersistent += 2");
var createRealmLexicalFirst = createRealmChild.evalScript(
    "let realmLexical = 6; realmLexical"
);
var createRealmLexicalSecond = createRealmChild.evalScript("realmLexical *= 7");
createRealmEmit(
    "eval",
    [
        createRealmFirstResult,
        createRealmSecondResult,
        createRealmChildGlobal.realmPersistent,
        typeof realmPersistent,
        createRealmLexicalFirst,
        createRealmLexicalSecond,
        "realmLexical" in createRealmChildGlobal,
        createRealmCoercions.join(""),
        createRealmChild.evalScript() === undefined
    ].join("|")
);

var createRealmSyntaxError;
try {
    createRealmChild.evalScript(")");
} catch (error) {
    createRealmSyntaxError = error;
}
var createRealmTypeError;
try {
    createRealmChild.evalScript("null.value");
} catch (error) {
    createRealmTypeError = error;
}
var createRealmSymbolError;
try {
    createRealmChild.evalScript(Symbol("source"));
} catch (error) {
    createRealmSymbolError = error;
}
createRealmEmit(
    "errors",
    [
        createRealmSyntaxError instanceof createRealmChildGlobal.SyntaxError,
        createRealmSyntaxError instanceof SyntaxError,
        String(createRealmSyntaxError.stack).indexOf("<evalScript>:1") !== -1,
        createRealmTypeError instanceof createRealmChildGlobal.TypeError,
        createRealmTypeError instanceof TypeError,
        createRealmSymbolError instanceof createRealmChildGlobal.TypeError,
        createRealmSymbolError instanceof TypeError
    ].join("|")
);

var createRealmConstructError;
try {
    new createRealmChild.createRealm();
} catch (error) {
    createRealmConstructError = error;
}
createRealmEmit(
    "construct",
    [
        createRealmConstructError instanceof TypeError,
        createRealmConstructError instanceof createRealmChildGlobal.TypeError
    ].join("|")
);

var createRealmJobInitial = createRealmChild.evalScript(
    "globalThis.realmJobValue = 0;" +
    "Promise.resolve(40).then(function (value) { realmJobValue = value + 2; });" +
    "realmJobValue"
);
createRealmEmit("job-before-drain", createRealmJobInitial);

if (typeof print === "function")
    print(createRealmTranscript.join("\n"));
