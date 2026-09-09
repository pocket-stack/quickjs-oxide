use super::*;

#[test]
fn generator_execution_kind_is_orthogonal_to_function_grammar_role() {
    let source = r#"
        function* declaration(value = 1) { yield value; }
        (function* expression() { yield; });
        ({ *fixed() { yield 2; }, *["computed"]() { yield* source; } });
        class C {
            *instance() { yield 3; }
            static *staticMethod() { yield 4; }
            *#privateInstance() { yield 5; }
            static *#privateStatic() { yield 6; }
        }
    "#;
    let tree = Parser::parse(source, JsString::from_static("<generator-kind-test>")).unwrap();
    let generators = tree
        .functions
        .iter()
        .filter(|function| function.execution_kind == BytecodeFunctionKind::Generator)
        .collect::<Vec<_>>();
    assert_eq!(generators.len(), 8);
    assert_eq!(
        generators
            .iter()
            .filter(|function| function.kind == FunctionKind::Ordinary)
            .count(),
        2
    );
    assert_eq!(
        generators
            .iter()
            .filter(|function| function.kind == FunctionKind::Method)
            .count(),
        6
    );
    assert!(generators.iter().all(|function| {
        function
            .ops
            .iter()
            .filter(|operation| {
                matches!(
                    operation.op,
                    super::IrOp::Bytecode(Instruction::InitialYield)
                )
            })
            .count()
            == 1
    }));

    let script = compile_unlinked_script(source).unwrap();
    assert_eq!(
        script
            .local_definitions()
            .iter()
            .filter(|definition| definition.kind == ClosureVariableKind::PrivateMethod)
            .count(),
        2
    );
    assert_eq!(
        script
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::InitializePrivateMethod(_)))
            .count(),
        2
    );
    fn collect_generators<'a>(
        function: &'a crate::engine::code::function::UnlinkedFunction,
        output: &mut Vec<&'a crate::engine::code::function::UnlinkedFunction>,
    ) {
        for constant in function.constants() {
            let Some(child) = constant.as_child() else {
                continue;
            };
            if child.metadata().function_kind == BytecodeFunctionKind::Generator {
                output.push(child);
            }
            collect_generators(child, output);
        }
    }
    let mut published = Vec::new();
    collect_generators(&script, &mut published);
    assert_eq!(published.len(), 8);
    assert!(published.iter().all(|function| {
        function.metadata().has_prototype
            && function.metadata().constructor_kind == ConstructorKind::None
            && function
                .code()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::InitialYield))
    }));
    let delegated = published
        .iter()
        .find(|function| {
            function
                .code()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::YieldStar))
        })
        .expect("delegated generator");
    for expected in [
        Instruction::IteratorStart,
        Instruction::IteratorNext,
        Instruction::IteratorCheckObject,
        Instruction::ThrowIteratorMissingThrow,
    ] {
        assert!(
            delegated
                .code()
                .iter()
                .any(|instruction| std::mem::discriminant(instruction)
                    == std::mem::discriminant(&expected)),
            "delegation lowering omitted {expected:?}"
        );
    }
    assert_eq!(
        delegated
            .code()
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::IteratorCall(kind) => Some(*kind),
                _ => None,
            })
            .collect::<Vec<_>>(),
        [
            IteratorCallKind::ReturnWithValue,
            IteratorCallKind::ThrowWithValue,
            IteratorCallKind::ReturnWithoutValue,
        ]
    );
}

#[test]
fn generator_initial_yield_separates_parameters_from_body_instantiation() {
    let script = compile_unlinked_script(
        "(function* g(value = 1) { function bodyHoist() {} yield value; })",
    )
    .unwrap();
    let generator = script.constants()[0].as_child().unwrap();
    let initial = generator
        .code()
        .iter()
        .position(|instruction| matches!(instruction, Instruction::InitialYield))
        .expect("generator initial yield");
    let parameter_read = generator
        .code()
        .iter()
        .position(|instruction| matches!(instruction, Instruction::GetArg(0)))
        .expect("default parameter read");
    let body_hoist = generator
        .code()
        .iter()
        .position(|instruction| matches!(instruction, Instruction::FClosure(_)))
        .expect("body function hoist");
    let authored_yield = generator
        .code()
        .iter()
        .position(|instruction| matches!(instruction, Instruction::Yield))
        .expect("authored yield");
    assert!(parameter_read < initial);
    assert!(initial < body_hoist);
    assert!(body_hoist < authored_yield);
}

#[test]
fn generator_lexical_context_matches_nested_function_and_arrow_boundaries() {
    for source in [
        "function* yield(){ yield 1; }",
        "function* await(){ yield 1; }",
        "(function* await(){ yield 1; })",
        "function* outer(){ function ordinary(){ var yield=1; return yield; } (()=>yield); yield 2; }",
        "function* outer(){ (function yield(){}); yield 1; }",
        "function* outer(){ (function* yield(){}); yield 1; }",
    ] {
        compile_unlinked_script(source).unwrap_or_else(|error| {
            panic!("generator lexical context rejected {source:?}: {error}")
        });
    }

    for source in [
        "function* g(value = yield 1){}",
        "function* g(){ (yield)=>0; }",
        "function* g(){ void yield; }",
        "function* g(){ yield 3 + yield 4; }",
        "function* g(){ function yield(){} }",
        "function* g(){ 'use strict'; (function* yield(){}); }",
        "function* g(){ for ({ yield } in [{}]); }",
        "'use strict'; for ({ yield } in [{}]);",
    ] {
        assert_eq!(
            compile_unlinked_script(source).unwrap_err().kind(),
            ErrorKind::Syntax,
            "{source:?}"
        );
    }

    let strict_declaration = compile_unlinked_script("'use strict'; function* yield(){}")
        .expect_err("strict outer context must reserve a generator declaration named yield");
    assert_eq!(strict_declaration.kind(), ErrorKind::Syntax);
    assert_eq!(strict_declaration.message(), "function name expected");

    for source in ["(function* yield(){})", "(function* \\u0079ield(){})"] {
        let error = compile_unlinked_script(source)
            .expect_err("named generator expression must reserve a yield self binding");
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(
            error.message(),
            "'yield' is a reserved identifier",
            "{source}"
        );
    }
}

#[test]
fn scoped_generator_declarations_do_not_receive_annex_b_duplicate_exceptions() {
    for source in [
        "{ function f(){} function* f(){} }",
        "{ function* f(){} function f(){} }",
        "{ function* f(){} function* f(){} }",
        "switch(0){case 0:function f(){} default:function* f(){}}",
        "switch(0){case 0:function* f(){} default:function f(){}}",
        "switch(0){case 0:function* f(){} default:function* f(){}}",
    ] {
        let error = compile_unlinked_script(source)
            .expect_err("generator declaration duplicate must be an early error");
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(
            error.message(),
            "invalid redefinition of lexical identifier",
            "{source}"
        );
    }

    compile_unlinked_script("{ function f(){} function f(){} }")
        .expect("sloppy ordinary block functions retain the Annex B exception");
    compile_unlinked_script("switch(0){case 0:function f(){} default:function f(){}}")
        .expect("sloppy ordinary switch functions retain the Annex B exception");

    let mut tampered = Parser::parse(
        "{ function* generator(){} }",
        JsString::from_static("<generator-annex-verifier>"),
    )
    .unwrap();
    assert!(
        tampered.functions[0].scoped_functions[0]
            .annex_binding
            .is_none()
    );
    tampered.functions[0].scoped_functions[0].annex_binding = Some(super::IrAnnexBinding::Dynamic);
    assert_eq!(
        validate_scope_graph(&tampered).unwrap_err().message(),
        "scoped generator retained an Annex B outer binding"
    );
}

#[test]
fn generator_yield_in_array_destructuring_unwinds_transient_iterators() {
    let source = r#"
        (function(){
            var log="";
            function make(name,first){
                var step=0;
                return {
                    [Symbol.iterator]:function(){log+="I"+name;return this},
                    next:function(){
                        log+="N"+name;
                        if(step++===0)return{value:first,done:false};
                        return{done:true}
                    },
                    return:function(){log+="R"+name;return{done:true}}
                }
            }

            function* binding(){
                var[a=yield 1]=make("b",undefined);
                return a
            }
            var b=binding(),b0=b.next(),b1=b.return(40);

            function* assignment(){
                var a;
                [a=yield 2]=make("a",undefined);
                return a
            }
            var a=assignment(),a0=a.next(),a1=a.return(41);

            function* nested(){
                var[[x=yield 3]]=make("o",make("n",undefined));
                return x
            }
            var n=nested(),n0=n.next(),n1=n.return(42);

            function* thrown(){
                var[x=yield 4]=make("t",undefined)
            }
            var t=thrown(),t0=t.next(),caught;
            try{t.throw("x")}catch(e){caught=e}

            function* normal(){
                var[x=yield 5]=make("c",undefined);
                return x
            }
            var c=normal(),c0=c.next(),c1=c.next(43);

            return [
                b0.value,b0.done,b1.value,b1.done,
                a0.value,a1.value,n0.value,n1.value,
                t0.value,caught,c0.value,c1.value,c1.done,log
            ].join("|")
        })()
    "#;
    assert_eq!(
        evaluate_in_context(source),
        Value::String(JsString::from_static(
            "1|false|40|true|2|41|3|42|4|x|5|43|true|\
             IbNbRbIaNaRaIoNoInNnRnRoItNtRtIcNcRc"
        )),
    );
}

#[test]
fn generator_destructuring_return_orders_iterator_finally_and_yield_star() {
    let source = r#"
        (function(){
            var log="";
            function source(name){
                var step=0;
                return {
                    [Symbol.iterator]:function(){log+="I"+name+"|";return this},
                    next:function(){
                        log+="N"+name+"|";
                        if(step++===0)return{value:undefined,done:false};
                        return{done:true}
                    },
                    return:function(){log+="R"+name+"|";return{done:true}}
                }
            }

            function* withFinally(){
                try{
                    var[x=yield 1]=source("outer")
                }finally{
                    log+="F|"
                }
            }
            var a=withFinally(),a0=a.next(),a1=a.return(40);

            var delegate={
                [Symbol.iterator]:function(){log+="Id|";return this},
                next:function(){log+="Nd|";return{value:2,done:false}},
                return:function(value){
                    log+="Rd:"+value+"|";
                    return{value:value,done:true}
                }
            };
            function* delegated(){
                var[x=yield* delegate]=source("host")
            }
            var b=delegated(),b0=b.next(),b1=b.return(41);

            return [
                a0.value,a1.value,a1.done,
                b0.value,b1.value,b1.done,log
            ].join("|")
        })()
    "#;
    assert_eq!(
        evaluate_in_context(source),
        Value::String(JsString::from_static(
            "1|40|true|2|41|true|Iouter|Nouter|Router|F|\
             Ihost|Nhost|Id|Nd|Rd:41|Rhost|"
        )),
    );
}

#[test]
fn generator_for_of_head_return_drops_outer_iterator_like_quickjs() {
    let source = r#"
        (function(){
            var log="",out=[],holder={};
            function make(name,values){
                var index=0;
                return {
                    [Symbol.iterator]:function(){log+="I"+name;return this},
                    next:function(){
                        log+="N"+name;
                        return index<values.length
                            ? {value:values[index++],done:false}
                            : {done:true}
                    },
                    return:function(){log+="R"+name;return{done:true}}
                }
            }
            function take(name,run){
                log="";
                out.push(name+":"+run()+":"+log)
            }

            take("binding",function(){
                function* g(){
                    for(var[x=yield 1]of make("o",[make("i",[undefined])])){}
                }
                var iterator=g(),first=iterator.next(),last=iterator.return(40);
                return[first.value,first.done,last.value,last.done].join(",")
            });
            take("assignment",function(){
                var x;
                function* g(){
                    for([x=yield 2]of make("o",[make("i",[undefined])])){}
                }
                var iterator=g(),first=iterator.next(),last=iterator.return(41);
                return[first.value,last.value,last.done].join(",")
            });
            take("computed-return",function(){
                function* g(){for(holder[yield 3]of make("o",[9])){}}
                var iterator=g(),first=iterator.next(),last=iterator.return(42);
                return[first.value,last.value,last.done].join(",")
            });
            take("computed-next",function(){
                function* g(){
                    for(holder[yield 4]of make("o",[9])){log+="B"+holder.k}
                }
                var iterator=g(),first=iterator.next(),last=iterator.next("k");
                return[first.value,last.value,last.done,holder.k].join(",")
            });
            take("nested",function(){
                function* g(){
                    for(var[[x=yield 5]]of
                        make("o",[make("m",[make("i",[undefined])])])){}
                }
                var iterator=g(),first=iterator.next(),last=iterator.return(43);
                return[first.value,last.value,last.done].join(",")
            });
            take("yield-star",function(){
                var delegate={
                    [Symbol.iterator]:function(){log+="Id";return this},
                    next:function(){log+="Nd";return{value:6,done:false}},
                    return:function(value){
                        log+="Rd"+value;
                        return{value:value,done:true}
                    }
                };
                function* g(){
                    for(var[x=yield* delegate]of
                        make("o",[make("i",[undefined])])){}
                }
                var iterator=g(),first=iterator.next(),last=iterator.return(44);
                return[first.value,last.value,last.done].join(",")
            });
            take("body",function(){
                function* g(){for(var x of make("o",[9]))yield 7}
                var iterator=g(),first=iterator.next(),last=iterator.return(45);
                return[first.value,last.value,last.done].join(",")
            });
            take("rhs",function(){
                function* g(){
                    for(var x of(log+="Q",yield 8,make("o",[9]))){}
                }
                var iterator=g(),first=iterator.next(),last=iterator.return(46);
                return[first.value,last.value,last.done].join(",")
            });
            take("for-in-assignment",function(){
                function* g(){for(holder[yield 9]in{a:1}){}}
                var iterator=g(),first=iterator.next(),last=iterator.return(47);
                return[first.value,last.value,last.done].join(",")
            });
            take("for-in-binding",function(){
                function* g(){for(var{[yield 10]:x}in{a:1}){}}
                var iterator=g(),first=iterator.next(),last=iterator.return(48);
                return[first.value,last.value,last.done].join(",")
            });
            return out.join("|")
        })()
    "#;
    assert_eq!(
        evaluate_in_context(source),
        Value::String(JsString::from_static(
            "binding:1,false,40,true:IoNoIiNiRi|\
             assignment:2,41,true:IoNoIiNiRi|\
             computed-return:3,42,true:IoNo|\
             computed-next:4,,true,9:IoNoB9No|\
             nested:5,43,true:IoNoImNmIiNiRiRm|\
             yield-star:6,44,true:IoNoIiNiIdNdRd44Ri|\
             body:7,45,true:IoNoRo|\
             rhs:8,46,true:Q|\
             for-in-assignment:9,47,true:|\
             for-in-binding:10,48,true:"
        )),
    );
}

#[test]
fn generator_for_of_head_inner_close_throw_pending_closes_outer() {
    let source = r#"
        (function(){
            var log="",out=[];
            function make(name,values,throwValue){
                var index=0;
                return {
                    [Symbol.iterator]:function(){log+="I"+name;return this},
                    next:function(){
                        log+="N"+name;
                        return index<values.length
                            ? {value:values[index++],done:false}
                            : {done:true}
                    },
                    return:function(){
                        log+="R"+name;
                        if(throwValue!==undefined)throw throwValue;
                        return{done:true}
                    }
                }
            }
            function run(name,outerThrow,throwResume){
                log="";
                function* g(){
                    for(var[x=yield 1]of
                        make("o",[make("i",[undefined],"inner")],outerThrow)){}
                }
                var iterator=g(),caught;
                iterator.next();
                try{
                    if(throwResume)iterator.throw("pending");
                    else iterator.return(42)
                }catch(error){
                    caught=error
                }
                out.push(name+":"+caught+":"+log)
            }
            run("outer-normal",undefined,false);
            run("outer-throw","outer",false);
            run("resume-throw","outer",true);
            return out.join("|")
        })()
    "#;
    assert_eq!(
        evaluate_in_context(source),
        Value::String(JsString::from_static(
            "outer-normal:inner:IoNoIiNiRiRo|\
             outer-throw:inner:IoNoIiNiRiRo|\
             resume-throw:pending:IoNoIiNiRiRo"
        )),
    );
}

#[test]
fn generator_method_frontiers_keep_async_delegation_explicit() {
    for source in [
        "class C { *constructor(){} }",
        "class C { static *prototype(){} }",
    ] {
        assert_eq!(
            compile_unlinked_script(source).unwrap_err().kind(),
            ErrorKind::Syntax,
            "{source:?}"
        );
    }
    let generator_named_async = compile_unlinked_script("class C { *async foo(){} }")
        .expect_err("generator property name must not trigger the contextual async-method path");
    assert_eq!(generator_named_async.kind(), ErrorKind::Syntax);
    assert_eq!(generator_named_async.message(), "invalid property name");
    assert_eq!(
        compile_unlinked_script("({ *async foo(){} })")
            .unwrap_err()
            .message(),
        "invalid property name"
    );
    let yield_reference = compile_unlinked_script("class C { *#gen() { void yield; } }")
        .expect_err("yield must not parse as an IdentifierReference in a generator method");
    assert_eq!(yield_reference.kind(), ErrorKind::Syntax);
    assert_eq!(
        yield_reference.message(),
        "unexpected token in expression: 'yield'"
    );
    assert_eq!(yield_reference.span().unwrap().start.column, 26);

    compile_unlinked_script("class C { async *method(){ yield 1; } }")
        .expect("public class async-generator method should use the independent async driver");
    compile_unlinked_script("class C { async *#method(){ yield 1; } }")
        .expect("private class async-generator method should use the independent async driver");
    compile_unlinked_script("({ async *method(){ yield 1; } })")
        .expect("object async-generator method should use the independent async driver");
    for source in [
        "({ async *method(){ yield* source; } })",
        "class C { async *method(){ yield* source; } }",
    ] {
        compile_unlinked_script(source)
            .unwrap_or_else(|error| panic!("async delegation source rejected {source:?}: {error}"));
    }
    for source in [
        "({ async *method(){ for await (var value of values) yield value; } })",
        "({ async *method(values){ for (var value of values) yield value; } })",
        "class C { async *method(){ for await (var value of values) yield value; } }",
        "class C { async *method(values){ for (var value of values) yield value; } }",
    ] {
        compile_unlinked_script(source)
            .unwrap_or_else(|error| panic!("async iteration source rejected {source:?}: {error}"));
    }
}
