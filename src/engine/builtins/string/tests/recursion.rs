use super::*;

#[test]
fn recursive_string_conversion_family_is_guarded_on_libtest_stack_and_recovers() {
    std::thread::Builder::new()
        .name("string-conversion-stack-proof".into())
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            context
                .eval(
                    r#"function stringSubrangeRecurse(kind,depth){
                var start=Object();
                start[Symbol.toPrimitive]=function(){
                    if(depth!==0)stringSubrangeRecurse((kind+1)%3,depth-1);
                    return 0;
                };
                if(kind===0)return "x".substring(start);
                if(kind===1)return "x".substr(start);
                return "x".slice(start);
            }"#,
                )
                .unwrap();

            for kind in 0..3 {
                assert_eq!(
                    context
                        .eval(&format!("stringSubrangeRecurse({kind},3)"))
                        .unwrap(),
                    Value::String(JsString::from_static("x")),
                    "the proven-safe four-frame subrange chain was rejected for kind {kind}"
                );
                assert_eq!(
                    context
                        .eval(&format!(
                            r#"(function(){{
                        try{{stringSubrangeRecurse({kind},4);return "missing"}}
                        catch(error){{return error.name+":"+error.message}}
                    }})()"#,
                        ))
                        .unwrap(),
                    Value::String(JsString::from_static("InternalError:stack overflow")),
                    "the fifth subrange family frame was not rejected for kind {kind}"
                );
            }

            context
                .eval(
                    r#"function mixedStringSearchRecurse(kind,depth){
                        if(kind===0){
                            var search=Object(),descriptor=Object();
                            descriptor.get=function(){
                                if(depth!==0)mixedStringSearchRecurse(1,depth-1);
                                return false;
                            };
                            Object.defineProperty(search,Symbol.match,descriptor);
                            search[Symbol.toPrimitive]=function(){return "x"};
                            return "x".includes(search);
                        }
                        var start=Object();
                        start[Symbol.toPrimitive]=function(){
                            if(depth!==0)mixedStringSearchRecurse(0,depth-1);
                            return 0;
                        };
                        return "x".slice(start);
                    }"#,
                )
                .unwrap();
            assert_eq!(
                context.eval("mixedStringSearchRecurse(0,3)").unwrap(),
                Value::Bool(true),
                "the proven-safe four-frame includes/subrange chain was rejected"
            );
            assert_eq!(
                context.eval("mixedStringSearchRecurse(1,3)").unwrap(),
                Value::String(JsString::from_static("x")),
                "the reverse four-frame includes/subrange chain was rejected"
            );
            assert_eq!(
                context
                    .eval(
                        r#"(function(){
                            try{mixedStringSearchRecurse(0,4);return "missing"}
                            catch(error){return error.name+":"+error.message}
                        })()"#,
                    )
                    .unwrap(),
                Value::String(JsString::from_static("InternalError:stack overflow")),
                "alternating includes/subrange calls bypassed the shared fifth-frame guard"
            );

            context
                .eval(
                    r#"function mixedStringRepeatRecurse(kind,depth){
                        if(kind===0){
                            var count=Object();
                            count[Symbol.toPrimitive]=function(){
                                if(depth!==0)mixedStringRepeatRecurse(1,depth-1);
                                return 1;
                            };
                            return "x".repeat(count);
                        }
                        var start=Object();
                        start[Symbol.toPrimitive]=function(){
                            if(depth!==0)mixedStringRepeatRecurse(0,depth-1);
                            return 0;
                        };
                        return "x".slice(start);
                    }"#,
                )
                .unwrap();
            for kind in 0..2 {
                assert_eq!(
                    context
                        .eval(&format!("mixedStringRepeatRecurse({kind},3)"))
                        .unwrap(),
                    Value::String(JsString::from_static("x")),
                    "the proven-safe repeat/subrange chain was rejected for kind {kind}"
                );
                assert_eq!(
                    context
                        .eval(&format!(
                            r#"(function(){{
                                try{{mixedStringRepeatRecurse({kind},4);return "missing"}}
                                catch(error){{return error.name+":"+error.message}}
                            }})()"#,
                        ))
                        .unwrap(),
                    Value::String(JsString::from_static("InternalError:stack overflow")),
                    "alternating repeat/subrange calls bypassed the shared fifth-frame guard"
                );
            }

            context
                .eval(
                    r#"function mixedStringPadRecurse(kind,depth){
                        if(kind===0){
                            var target=Object();
                            target[Symbol.toPrimitive]=function(){
                                if(depth!==0)mixedStringPadRecurse(1,depth-1);
                                return 2;
                            };
                            return "x".padEnd(target);
                        }
                        if(kind===1){
                            var count=Object();
                            count[Symbol.toPrimitive]=function(){
                                if(depth!==0)mixedStringPadRecurse(2,depth-1);
                                return 1;
                            };
                            return "x".repeat(count);
                        }
                        if(kind===2){
                            var start=Object();
                            start[Symbol.toPrimitive]=function(){
                                if(depth!==0)mixedStringPadRecurse(3,depth-1);
                                return 0;
                            };
                            return "x".slice(start);
                        }
                        var filler=Object();
                        filler[Symbol.toPrimitive]=function(){
                            if(depth!==0)mixedStringPadRecurse(0,depth-1);
                            return "_";
                        };
                        return "x".padStart(2,filler);
                    }"#,
                )
                .unwrap();
            for (kind, expected) in [(0, "x "), (1, "x"), (2, "x"), (3, "_x")] {
                assert_eq!(
                    context
                        .eval(&format!("mixedStringPadRecurse({kind},3)"))
                        .unwrap(),
                    Value::String(JsString::try_from_utf8(expected).unwrap()),
                    "the proven-safe pad/repeat/slice chain was rejected for kind {kind}"
                );
                assert_eq!(
                    context
                        .eval(&format!(
                            r#"(function(){{
                                try{{mixedStringPadRecurse({kind},4);return "missing"}}
                                catch(error){{return error.name+":"+error.message}}
                            }})()"#,
                        ))
                        .unwrap(),
                    Value::String(JsString::from_static("InternalError:stack overflow")),
                    "alternating pad/repeat/slice calls bypassed the shared fifth-frame guard"
                );
            }

            context
                .eval(
                    r#"function mixedStringTrimRecurse(kind,depth){
                        if(kind===0){
                            var receiver=Object();
                            receiver[Symbol.toPrimitive]=function(){
                                if(depth!==0)mixedStringTrimRecurse(1,depth-1);
                                return " x ";
                            };
                            return String.prototype.trim.call(receiver);
                        }
                        if(kind===1){
                            var target=Object();
                            target[Symbol.toPrimitive]=function(){
                                if(depth!==0)mixedStringTrimRecurse(2,depth-1);
                                return 1;
                            };
                            return "x".padEnd(target);
                        }
                        if(kind===2){
                            var count=Object();
                            count[Symbol.toPrimitive]=function(){
                                if(depth!==0)mixedStringTrimRecurse(3,depth-1);
                                return 1;
                            };
                            return "x".repeat(count);
                        }
                        if(kind===3){
                            var start=Object();
                            start[Symbol.toPrimitive]=function(){
                                if(depth!==0)mixedStringTrimRecurse(4,depth-1);
                                return 0;
                            };
                            return "x".slice(start);
                        }
                        var search=Object(),descriptor=Object();
                        descriptor.get=function(){
                            if(depth!==0)mixedStringTrimRecurse(0,depth-1);
                            return false;
                        };
                        Object.defineProperty(search,Symbol.match,descriptor);
                        search[Symbol.toPrimitive]=function(){return "x"};
                        return "x".includes(search)?"x":"wrong";
                    }"#,
                )
                .unwrap();
            for kind in 0..5 {
                assert_eq!(
                    context
                        .eval(&format!("mixedStringTrimRecurse({kind},3)"))
                        .unwrap(),
                    Value::String(JsString::from_static("x")),
                    "the proven-safe trim/shared-String chain was rejected for kind {kind}",
                );
                assert_eq!(
                    context
                        .eval(&format!(
                            r#"(function(){{
                                try{{mixedStringTrimRecurse({kind},4);return "missing"}}
                                catch(error){{return error.name+":"+error.message}}
                            }})()"#,
                        ))
                        .unwrap(),
                    Value::String(JsString::from_static("InternalError:stack overflow")),
                    "trim alternation bypassed the shared fifth-frame guard for kind {kind}",
                );
            }

            context
                .eval(
                    r#"function mixedStringCreateHtmlRecurse(kind,depth){
                        if(kind===0){
                            var receiver=Object();
                            receiver[Symbol.toPrimitive]=function(){
                                if(depth!==0)mixedStringCreateHtmlRecurse(1,depth-1);
                                return "x";
                            };
                            receiver.anchor=String.prototype.anchor;
                            return receiver.anchor("n");
                        }
                        if(kind===1){
                            var attribute=Object();
                            attribute[Symbol.toPrimitive]=function(){
                                if(depth!==0)mixedStringCreateHtmlRecurse(2,depth-1);
                                return "u";
                            };
                            return "x".link(attribute);
                        }
                        if(kind===2){
                            var receiver=Object();
                            receiver[Symbol.toPrimitive]=function(){
                                if(depth!==0)mixedStringCreateHtmlRecurse(3,depth-1);
                                return "x";
                            };
                            receiver.big=String.prototype.big;
                            return receiver.big();
                        }
                        if(kind===3){
                            var receiver=Object();
                            receiver[Symbol.toPrimitive]=function(){
                                if(depth!==0)mixedStringCreateHtmlRecurse(4,depth-1);
                                return " x ";
                            };
                            receiver.trim=String.prototype.trim;
                            return receiver.trim();
                        }
                        if(kind===4){
                            var target=Object();
                            target[Symbol.toPrimitive]=function(){
                                if(depth!==0)mixedStringCreateHtmlRecurse(5,depth-1);
                                return 1;
                            };
                            return "x".padEnd(target);
                        }
                        if(kind===5){
                            var count=Object();
                            count[Symbol.toPrimitive]=function(){
                                if(depth!==0)mixedStringCreateHtmlRecurse(6,depth-1);
                                return 1;
                            };
                            return "x".repeat(count);
                        }
                        var search=Object(),descriptor=Object();
                        descriptor.get=function(){
                            if(depth!==0)mixedStringCreateHtmlRecurse(0,depth-1);
                            return false;
                        };
                        Object.defineProperty(search,Symbol.match,descriptor);
                        search[Symbol.toPrimitive]=function(){return "x"};
                        return "x".includes(search)?"x":"wrong";
                    }"#,
                )
                .unwrap();
            for (kind, expected) in [
                (0, "<a name=\"n\">x</a>"),
                (1, "<a href=\"u\">x</a>"),
                (2, "<big>x</big>"),
                (3, "x"),
                (4, "x"),
                (5, "x"),
                (6, "x"),
            ] {
                assert_eq!(
                    context
                        .eval(&format!(
                            r#"(function(){{
                                try{{return mixedStringCreateHtmlRecurse({kind},3)}}
                                catch(error){{return "ERROR:"+error.name+":"+error.message}}
                            }})()"#,
                        ))
                        .unwrap(),
                    Value::String(JsString::try_from_utf8(expected).unwrap()),
                    "the proven-safe CreateHTML/shared-String chain was rejected for kind {kind}",
                );
                assert_eq!(
                    context
                        .eval(&format!(
                            r#"(function(){{
                                try{{mixedStringCreateHtmlRecurse({kind},4);return "missing"}}
                                catch(error){{return error.name+":"+error.message}}
                            }})()"#,
                        ))
                        .unwrap(),
                    Value::String(JsString::from_static("InternalError:stack overflow")),
                    "CreateHTML alternation bypassed the shared fifth-frame guard for kind {kind}",
                );
            }

            context
                .eval(
                    r#"function mixedStringCaseRecurse(kind,depth){
                        var receiver=Object();
                        receiver[Symbol.toPrimitive]=function(){
                            if(depth!==0)mixedStringCaseRecurse((kind+1)%2,depth-1);
                            return kind===0?"A":" x ";
                        };
                        if(kind===0){
                            receiver.toLowerCase=String.prototype.toLowerCase;
                            return receiver.toLowerCase();
                        }
                        receiver.trim=String.prototype.trim;
                        return receiver.trim();
                    }"#,
                )
                .unwrap();
            for (kind, expected) in [(0, "a"), (1, "x")] {
                assert_eq!(
                    context
                        .eval(&format!("mixedStringCaseRecurse({kind},3)"))
                        .unwrap(),
                    Value::String(JsString::from_static(expected)),
                    "the proven-safe case/trim chain was rejected for kind {kind}",
                );
                assert_eq!(
                    context
                        .eval(&format!(
                            r#"(function(){{
                                try{{mixedStringCaseRecurse({kind},4);return "missing"}}
                                catch(error){{return error.name+":"+error.message}}
                            }})()"#,
                        ))
                        .unwrap(),
                    Value::String(JsString::from_static("InternalError:stack overflow")),
                    "case/trim alternation bypassed the shared fifth-frame guard for kind {kind}",
                );
            }
            context
                .eval(
                    r#"function mixedStringNormalizeRecurse(kind,depth){
                        if(kind===0){
                            var receiver=Object();
                            receiver[Symbol.toPrimitive]=function(){
                                if(depth!==0)mixedStringNormalizeRecurse(1,depth-1);
                                return "A\u030a";
                            };
                            return String.prototype.normalize.call(receiver);
                        }
                        var form=Object();
                        form[Symbol.toPrimitive]=function(){
                            if(depth!==0)mixedStringNormalizeRecurse(0,depth-1);
                            return "NFC";
                        };
                        return "A\u030a".normalize(form);
                    }"#,
                )
                .unwrap();
            for kind in 0..2 {
                assert_eq!(
                    context
                        .eval(&format!("mixedStringNormalizeRecurse({kind},3)"))
                        .unwrap(),
                    Value::String(JsString::from_static("Å")),
                    "the proven-safe normalize conversion chain was rejected for kind {kind}",
                );
                assert_eq!(
                    context
                        .eval(&format!(
                            r#"(function(){{
                                try{{mixedStringNormalizeRecurse({kind},4);return "missing"}}
                                catch(error){{return error.name+":"+error.message}}
                            }})()"#,
                        ))
                        .unwrap(),
                    Value::String(JsString::from_static("InternalError:stack overflow")),
                    "normalize conversion bypassed the shared fifth-frame guard for kind {kind}",
                );
            }
            assert_eq!(
                context
                    .eval(
                        r#""abc".includes("b")+"|"+"abc".slice(1)+"|"+
                           "ab".repeat(2)+"|"+"a".padEnd(3,"x")+"|"+"a".padStart(3,"x")+"|"+
                           " z ".trim()+"|"+"z".bold()+"|"+"AbΣ".toLocaleLowerCase()+"|"+
                           "A\u030a".normalize()"#,
                    )
                    .unwrap(),
                Value::String(JsString::from_static(
                    "true|bc|abab|axx|xxa|z|<b>z</b>|abς|Å",
                )),
                "the runtime did not recover after mixed String-family overflow"
            );
        })
        .expect("2 MiB String conversion stack-proof thread did not start")
        .join()
        .expect("2 MiB String conversion stack-proof thread panicked");
}

#[test]
fn recursive_string_constructor_family_is_guarded_and_runtime_recovers() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context
        .eval(
            r#"function stringConstructorRecurse(depth){
                    var value=Object();
                    value[Symbol.toPrimitive]=function(){
                        if(depth!==0)stringConstructorRecurse(depth-1);
                        return "x";
                    };
                    return String(value);
                }
                function stringFromCharCodeRecurse(depth){
                    var value=Object();
                    value[Symbol.toPrimitive]=function(){
                        if(depth!==0)stringFromCharCodeRecurse(depth-1);
                        return 65;
                    };
                    return String.fromCharCode(value);
                }
                function stringFromCodePointRecurse(depth){
                    var value=Object();
                    value[Symbol.toPrimitive]=function(){
                        if(depth!==0)stringFromCodePointRecurse(depth-1);
                        return 65;
                    };
                    return String.fromCodePoint(value);
                }
                function stringRawRecurse(depth){
                    var cooked=Object(),raw=Object();raw.length=1;raw[0]="x";
                    cooked.__defineGetter__("raw",function(){
                        if(depth!==0)stringRawRecurse(depth-1);
                        return raw;
                    });
                    return String.raw(cooked);
                }"#,
        )
        .unwrap();

    for (call, expected) in [
        ("stringConstructorRecurse(8)", "x"),
        ("stringFromCharCodeRecurse(8)", "A"),
        ("stringFromCodePointRecurse(8)", "A"),
        ("stringRawRecurse(8)", "x"),
    ] {
        assert_eq!(
            context.eval(call).unwrap(),
            Value::String(JsString::from_static(expected)),
            "safe String-family recursion drifted for {call}",
        );
    }
    for call in [
        "stringConstructorRecurse(9)",
        "stringFromCharCodeRecurse(9)",
        "stringFromCodePointRecurse(9)",
        "stringRawRecurse(9)",
    ] {
        let value = context
            .eval(&format!(
                r#"(function(){{
                    try{{{call};return "missing"}}
                    catch(error){{return error.name+":"+error.message}}
                }})()"#,
            ))
            .unwrap();
        assert_eq!(
            value,
            Value::String(JsString::from_static("InternalError:stack overflow")),
            "String-family recursion guard drifted for {call}",
        );
    }
    assert_eq!(context.eval("1+1").unwrap(), Value::Int(2));
}

#[test]
fn recursive_string_includes_family_match_getter_is_guarded_and_runtime_recovers() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context
        .eval(
            r#"function stringIncludesFamilyRecurse(kind,depth){
                var search=Object(),descriptor=Object();
                descriptor.get=function(){
                    if(depth!==0)stringIncludesFamilyRecurse(kind,depth-1);
                    return false;
                };
                Object.defineProperty(search,Symbol.match,descriptor);
                search.toString=function(){return "x"};
                if(kind===0)return "x".includes(search);
                if(kind===1)return "x".endsWith(search);
                return "x".startsWith(search);
            }"#,
        )
        .unwrap();

    for (method, kind) in [("includes", 0), ("endsWith", 1), ("startsWith", 2)] {
        assert_eq!(
            context
                .eval(&format!("stringIncludesFamilyRecurse({kind},3)"))
                .unwrap(),
            Value::Bool(true),
            "the proven-safe four-frame {method} chain was rejected",
        );
        assert_eq!(
            context
                .eval(&format!(
                    r#"(function(){{
                        try{{stringIncludesFamilyRecurse({kind},4);return "missing"}}
                        catch(error){{return error.name+":"+error.message}}
                    }})()"#,
                ))
                .unwrap(),
            Value::String(JsString::from_static("InternalError:stack overflow")),
        );
    }
    assert_eq!(context.eval("1+1").unwrap(), Value::Int(2));
}

#[test]
fn mixed_string_and_regexp_search_recursion_is_guarded_and_runtime_recovers() {
    std::thread::Builder::new()
        .name("string-regexp-search-stack-proof".into())
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            context
                .eval(
                    r#"function mixedSearchRecurse(kind,depth){
                        if(kind===0){
                            var pattern=Object();
                            pattern[Symbol.search]=function(){
                                if(depth!==0)return mixedSearchRecurse(1,depth-1);
                                return 0;
                            };
                            return "x".search(pattern);
                        }
                        var regexp=Object();
                        regexp.lastIndex=0;
                        regexp.exec=function(){
                            if(depth!==0)mixedSearchRecurse(0,depth-1);
                            return {index:0};
                        };
                        return RegExp.prototype[Symbol.search].call(regexp,"x");
                    }"#,
                )
                .unwrap();

            for (entry, kind) in [("String.prototype.search", 0), ("RegExp @@search", 1)] {
                assert_eq!(
                    context
                        .eval(&format!("mixedSearchRecurse({kind},3)"))
                        .unwrap(),
                    Value::Int(0),
                    "the proven-safe four-frame {entry} chain was rejected",
                );
                assert_eq!(
                    context
                        .eval(&format!(
                            r#"(function(){{
                                try{{mixedSearchRecurse({kind},4);return "missing"}}
                                catch(error){{return error.name+":"+error.message}}
                            }})()"#,
                        ))
                        .unwrap(),
                    Value::String(JsString::from_static("InternalError:stack overflow")),
                    "the fifth mixed search frame was not rejected from {entry}",
                );
            }
            assert_eq!(
                context
                    .eval(
                        r#""abc".search("b")+"|"+
                           RegExp.prototype[Symbol.search].call({
                               lastIndex:0,exec:function(){return null}
                           },"x")"#,
                    )
                    .unwrap(),
                Value::String(JsString::from_static("1|-1")),
                "the runtime did not recover after mixed search overflow",
            );
        })
        .expect("2 MiB String/RegExp search stack-proof thread did not start")
        .join()
        .expect("2 MiB String/RegExp search stack-proof thread panicked");
}
