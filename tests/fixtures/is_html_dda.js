var isHtmlDdaTranscript = [];

function isHtmlDdaEmit(label, values) {
    isHtmlDdaTranscript.push(label + "=" + values.join("|"));
}

function isHtmlDdaThrownName(callback) {
    try {
        callback();
        return "none";
    } catch (error) {
        return error.name;
    }
}

var IsHTMLDDA = $262.IsHTMLDDA;
var isHtmlDdaDescriptor = Object.getOwnPropertyDescriptor($262, "IsHTMLDDA");
isHtmlDdaEmit("host", [
    typeof IsHTMLDDA,
    Boolean(IsHTMLDDA),
    IsHTMLDDA.name,
    IsHTMLDDA.length,
    Object.prototype.hasOwnProperty.call(IsHTMLDDA, "prototype"),
    IsHTMLDDA() === null,
    isHtmlDdaThrownName(function () { new IsHTMLDDA(); }),
    isHtmlDdaDescriptor.writable,
    isHtmlDdaDescriptor.enumerable,
    isHtmlDdaDescriptor.configurable
]);

var isHtmlDdaIfBranch;
if (IsHTMLDDA) {
    isHtmlDdaIfBranch = "truthy";
} else {
    isHtmlDdaIfBranch = "falsy";
}
var isHtmlDdaAndEvaluated = false;
IsHTMLDDA && (isHtmlDdaAndEvaluated = true);
var isHtmlDdaAndAssignment = IsHTMLDDA;
isHtmlDdaAndAssignment &&= 42;
var isHtmlDdaOrAssignment = IsHTMLDDA;
isHtmlDdaOrAssignment ||= 42;
var isHtmlDdaNullishAssignment = IsHTMLDDA;
isHtmlDdaNullishAssignment ??= 42;
isHtmlDdaEmit("boolean", [
    !IsHTMLDDA,
    IsHTMLDDA ? "truthy" : "falsy",
    isHtmlDdaIfBranch,
    isHtmlDdaAndEvaluated,
    IsHTMLDDA || 42,
    isHtmlDdaAndAssignment === IsHTMLDDA,
    isHtmlDdaOrAssignment,
    isHtmlDdaNullishAssignment === IsHTMLDDA
]);

isHtmlDdaEmit("equality", [
    IsHTMLDDA == undefined,
    undefined == IsHTMLDDA,
    IsHTMLDDA == null,
    null == IsHTMLDDA,
    IsHTMLDDA != undefined,
    IsHTMLDDA === IsHTMLDDA,
    IsHTMLDDA === undefined,
    Object.is(IsHTMLDDA, IsHTMLDDA),
    Object.is(IsHTMLDDA, undefined),
    [IsHTMLDDA].includes(IsHTMLDDA),
    [undefined].includes(IsHTMLDDA)
]);

function isHtmlDdaDefault(value = 42) {
    return value;
}
var isHtmlDdaArrayDefault;
[isHtmlDdaArrayDefault = 42] = [IsHTMLDDA];
var isHtmlDdaObjectDefault;
({ value: isHtmlDdaObjectDefault = 42 } = { value: IsHTMLDDA });
isHtmlDdaEmit("nullish", [
    (IsHTMLDDA ?? 42) === IsHTMLDDA,
    isHtmlDdaDefault(IsHTMLDDA) === IsHTMLDDA,
    isHtmlDdaArrayDefault === IsHTMLDDA,
    isHtmlDdaObjectDefault === IsHTMLDDA,
    IsHTMLDDA?.name,
    IsHTMLDDA?.() === null,
    (undefined)?.name === undefined
]);

var isHtmlDdaPropertyTarget = {};
Object.defineProperty(isHtmlDdaPropertyTarget, "value", {
    value: 1,
    writable: IsHTMLDDA,
    enumerable: IsHTMLDDA,
    configurable: IsHTMLDDA
});
var isHtmlDdaPropertyDescriptor = Object.getOwnPropertyDescriptor(
    isHtmlDdaPropertyTarget,
    "value"
);
var isHtmlDdaProxyValue = IsHTMLDDA;
var isHtmlDdaDefineProxy = new Proxy({}, {
    defineProperty: function () { return isHtmlDdaProxyValue; }
});
var isHtmlDdaHasProxy = new Proxy({}, {
    has: function () { return isHtmlDdaProxyValue; }
});
var isHtmlDdaSetProxy = new Proxy({}, {
    set: function () { return isHtmlDdaProxyValue; }
});
isHtmlDdaEmit("descriptors-proxy", [
    isHtmlDdaPropertyDescriptor.writable,
    isHtmlDdaPropertyDescriptor.enumerable,
    isHtmlDdaPropertyDescriptor.configurable,
    Reflect.defineProperty(isHtmlDdaDefineProxy, "value", { value: 1 }),
    "value" in isHtmlDdaHasProxy,
    Reflect.set(isHtmlDdaSetProxy, "value", 1)
]);

var isHtmlDdaSpreadable = [1, 2];
isHtmlDdaSpreadable[Symbol.isConcatSpreadable] = IsHTMLDDA;
var isHtmlDdaConcat = [0].concat(isHtmlDdaSpreadable);
isHtmlDdaEmit("array", [
    [1].every(function () { return IsHTMLDDA; }),
    [1].some(function () { return IsHTMLDDA; }),
    [1].filter(function () { return IsHTMLDDA; }).length,
    [1].find(function () { return IsHTMLDDA; }) === undefined,
    isHtmlDdaConcat.length,
    isHtmlDdaConcat[1] === isHtmlDdaSpreadable
]);

var isHtmlDdaTypedArray = new Uint8Array([1]);
isHtmlDdaEmit("typed-array", [
    isHtmlDdaTypedArray.every(function () { return IsHTMLDDA; }),
    isHtmlDdaTypedArray.some(function () { return IsHTMLDDA; }),
    isHtmlDdaTypedArray.filter(function () { return IsHTMLDDA; }).length,
    isHtmlDdaTypedArray.find(function () { return IsHTMLDDA; }) === undefined
]);

isHtmlDdaEmit("iterator-helper", [
    Iterator.from([1]).every(function () { return IsHTMLDDA; }),
    Iterator.from([1]).some(function () { return IsHTMLDDA; }),
    Iterator.from([1]).find(function () { return IsHTMLDDA; }) === undefined,
    Iterator.from([1]).filter(function () { return IsHTMLDDA; }).next().done
]);

var isHtmlDdaIteratorStep = 0;
var isHtmlDdaIterable = {};
isHtmlDdaIterable[Symbol.iterator] = function () {
    return {
        next: function () {
            if (isHtmlDdaIteratorStep++ === 0) {
                return { value: 7, done: IsHTMLDDA };
            }
            return { done: true };
        }
    };
};
var isHtmlDdaIteratorValues = Array.from(isHtmlDdaIterable);
var isHtmlDdaCallableIterator = {};
isHtmlDdaCallableIterator[Symbol.iterator] = IsHTMLDDA;
var isHtmlDdaMatcher = {};
isHtmlDdaMatcher[Symbol.match] = IsHTMLDDA;
isHtmlDdaEmit("get-method", [
    isHtmlDdaIteratorValues.length,
    isHtmlDdaIteratorValues[0],
    isHtmlDdaThrownName(function () { Array.from(isHtmlDdaCallableIterator); }),
    "abc".match(isHtmlDdaMatcher) === null
]);

var isHtmlDdaBound = IsHTMLDDA.bind(null);
var isHtmlDdaProxy = new Proxy(IsHTMLDDA, {});
function isHtmlDdaOrdinaryFunction() {
    return null;
}
isHtmlDdaEmit("non-propagation", [
    Boolean(isHtmlDdaBound),
    typeof isHtmlDdaBound,
    isHtmlDdaBound == undefined,
    isHtmlDdaBound() === null,
    Boolean(isHtmlDdaProxy),
    typeof isHtmlDdaProxy,
    isHtmlDdaProxy == null,
    isHtmlDdaProxy() === null,
    Boolean(isHtmlDdaOrdinaryFunction),
    typeof isHtmlDdaOrdinaryFunction
]);

if (typeof print === "function") {
    print(isHtmlDdaTranscript.join("\n"));
}
