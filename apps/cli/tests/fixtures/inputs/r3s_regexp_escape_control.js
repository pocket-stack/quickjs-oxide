var r3sTranscript = [];

function hex4(value) {
    var text = value.toString(16);
    while (text.length < 4)
        text = "0" + text;
    return text;
}

function visible(value) {
    var text = String(value);
    var result = "";
    var i;
    var code;
    for (i = 0; i < text.length; i++) {
        code = text.charCodeAt(i);
        if (code === 92)
            result += "\\\\";
        else if (code >= 32 && code <= 126)
            result += String.fromCharCode(code);
        else
            result += "\\u" + hex4(code);
    }
    return result;
}

function errorName(callback) {
    try {
        callback();
        return "none";
    } catch (error) {
        return error.name;
    }
}

function matchText(regexp, input) {
    var match = regexp.exec(input);
    if (match === null)
        return "null";
    return visible(match[0]) + "@" + match.index;
}

var escapeDescriptor = Object.getOwnPropertyDescriptor(RegExp, "escape");
r3sTranscript.push(
    "metadata=" +
    typeof RegExp.escape + ":" +
    RegExp.escape.length + ":" +
    RegExp.escape.name + ":" +
    escapeDescriptor.writable + ":" +
    escapeDescriptor.enumerable + ":" +
    escapeDescriptor.configurable
);
r3sTranscript.push(
    "key-order=" + Object.getOwnPropertyNames(RegExp).join(",") +
    "|" + Object.getOwnPropertySymbols(RegExp).map(function (symbol) {
        return String(symbol);
    }).join(",")
);
r3sTranscript.push(
    "arbitrary-this=" +
    visible(RegExp.escape.call(undefined, "a+b")) + ":" +
    visible(RegExp.escape.call(null, "a+b")) + ":" +
    visible(RegExp.escape.call(42, "a+b")) + ":" +
    visible(RegExp.escape.call({ marker: true }, "a+b"))
);

var coercionCalls = 0;
var coercible = {
    toString: function () {
        coercionCalls++;
        return "a+b";
    }
};
var strictInputs = [
    function () { RegExp.escape(); },
    function () { RegExp.escape(undefined); },
    function () { RegExp.escape(null); },
    function () { RegExp.escape(false); },
    function () { RegExp.escape(42); },
    function () { RegExp.escape(coercible); },
    function () { RegExp.escape(new String("a+b")); },
    function () { RegExp.escape(Symbol("x")); }
];
var strictResults = [];
var strictIndex;
for (strictIndex = 0; strictIndex < strictInputs.length; strictIndex++)
    strictResults.push(errorName(strictInputs[strictIndex]));
r3sTranscript.push(
    "strict-input=" + strictResults.join(",") + ":calls=" + coercionCalls
);
r3sTranscript.push(
    "not-constructor=" +
    errorName(function () { new RegExp.escape("a"); })
);

var c0 = "";
var code;
for (code = 0; code < 32; code++)
    c0 += String.fromCharCode(code);
r3sTranscript.push("c0=" + visible(RegExp.escape(c0)));
r3sTranscript.push("space=" + visible(RegExp.escape(" ")));

var initialAlnum = ["0z", "9z", "Az", "Zz", "az", "zz"];
var initialResults = [];
for (var initialIndex = 0; initialIndex < initialAlnum.length; initialIndex++)
    initialResults.push(visible(RegExp.escape(initialAlnum[initialIndex])));
r3sTranscript.push("initial-alnum=" + initialResults.join(","));
r3sTranscript.push(
    "later-alnum=" + visible(RegExp.escape(".aA0zZ9"))
);
r3sTranscript.push(
    "syntax=" + visible(RegExp.escape("^$\\.*+?()[]{}|/"))
);
r3sTranscript.push(
    "other-punctuator=" +
    visible(RegExp.escape(",-=<>#&!%:;@~'`\""))
);
r3sTranscript.push(
    "underscore=" + visible(RegExp.escape("_a0"))
);
r3sTranscript.push(
    "del=" + visible(RegExp.escape("\u007f"))
);

var latin1 = "";
for (code = 128; code < 256; code++)
    latin1 += String.fromCharCode(code);
r3sTranscript.push("latin1=" + visible(RegExp.escape(latin1)));
r3sTranscript.push(
    "whitespace-boundaries=" +
    visible(RegExp.escape(
        "\u00a0\u1680\u180e\u2000\u200b\u2028\u2029\u202f\u205f\u3000\ufeff"
    ))
);
r3sTranscript.push(
    "surrogates=" +
    visible(RegExp.escape("\ud800X\udc00")) + ":" +
    visible(RegExp.escape("\ud83d\ude00"))
);

var ropeLeft = "a".repeat(9000) + "\ud83d";
var ropeRight = "\ude00" + "b".repeat(9000);
var rope = ropeLeft + ropeRight;
var escapedRope = RegExp.escape(rope);
r3sTranscript.push(
    "rope=" + rope.length + ":" + escapedRope.length + ":" +
    visible(escapedRope.slice(8999, 9009))
);

r3sTranscript.push(
    "control-letter=" +
    matchText(/\cA/, "\u0000\u0001\u0002") + ":" +
    matchText(/\ca/, "\u0000\u0001\u0002") + ":" +
    matchText(/[\cA-\cC]+/, "\u0000\u0001\u0002\u0003\u0004")
);
r3sTranscript.push(
    "control-class-digit=" +
    matchText(/[\c0]/, "\u000f\u0010\u0011") + ":" +
    matchText(/[\c1]/, "\u0010\u0011\u0012") + ":" +
    matchText(/[\c8]/, "\u0017\u0018\u0019") + ":" +
    matchText(/[\c9]/, "\u0018\u0019\u001a") + ":" +
    matchText(/[\c_]/, "\u001e\u001f\u0020")
);
r3sTranscript.push(
    "control-class-consume=" +
    matchText(/[\c00]+/, "\u000f0\u0010\u0011") + ":" +
    matchText(/[\c0-\c2]+/, "\u000f\u0010\u0011\u0012\u0013")
);
r3sTranscript.push(
    "control-outside-rollback=" +
    matchText(/\c0/, "\\c0") + ":" +
    matchText(/\c?/, "\\") + ":" +
    matchText(/\c?/, "\\c") + ":" +
    matchText(/\c?/, "c")
);
r3sTranscript.push(
    "control-class-rollback=" +
    matchText(/[\c?]/, "\\") + ":" +
    matchText(/[\c?]/, "c") + ":" +
    matchText(/[\c?]/, "?") + ":" +
    matchText(/[\c?]/, "x")
);
r3sTranscript.push(
    "control-unicode=" +
    errorName(function () { new RegExp("\\c0", "u"); }) + ":" +
    errorName(function () { new RegExp("\\c_", "u"); }) + ":" +
    errorName(function () { new RegExp("[\\c0]", "u"); }) + ":" +
    errorName(function () { new RegExp("[\\c_]", "u"); }) + ":" +
    matchText(new RegExp("\\cA", "u"), "\u0001")
);

r3sTranscript.push("r3s-regexp-escape-control-oracle=ok");
r3sTranscript.join("\n");
