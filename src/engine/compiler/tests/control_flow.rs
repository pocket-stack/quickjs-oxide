use super::*;

#[test]
fn script_completion_obeys_semicolons_and_asi() {
    assert_eq!(evaluate("1;\n2"), Value::Int(2));
    assert_eq!(evaluate("0\u{2028}1"), Value::Int(1));
    assert_eq!(evaluate("0\u{2029}1"), Value::Int(1));
    assert_eq!(evaluate("0\u{00a0}+1"), Value::Int(1));
    assert!(compile_script("1 2").is_err());
}

#[test]
fn block_and_if_statements_use_the_quickjs_eval_completion_slot() {
    assert_eq!(evaluate(""), Value::Undefined);
    assert_eq!(evaluate("1; {}"), Value::Int(1));
    assert_eq!(evaluate("1; {;;}"), Value::Int(1));
    assert_eq!(evaluate("{ 1; { 2; {} } }"), Value::Int(2));
    assert_eq!(evaluate("1; if (false) 2"), Value::Undefined);
    assert_eq!(evaluate("1; if (true) {}"), Value::Undefined);
    assert_eq!(evaluate("if (true) { 1; 2 } else 3"), Value::Int(2));
    assert_eq!(evaluate("if (false) { 1; 2 } else 3"), Value::Int(3));
    assert_eq!(evaluate("if (true) if (false) 1; else 2"), Value::Int(2));
    assert_eq!(evaluate("{ 'use strict'; } 010"), Value::Int(8));

    assert_eq!(
        evaluate_in_context("(function(x){ if (x) return 1; else return 2; })(0)"),
        Value::Int(2)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){ if (false) { var hidden = 1; } return typeof hidden; })()"
        ),
        Value::String(JsString::from_static("undefined"))
    );
    assert_eq!(
        evaluate_in_context("(function(){ { 'use strict'; } var eval = 7; return eval; })()"),
        Value::Int(7)
    );
    assert_eq!(
        evaluate_in_context(
            "Function.trace = ''; if ((Function.trace += 'c', true)) { Function.trace += 't'; } else { Function.trace += 'f'; } Function.trace"
        ),
        Value::String(JsString::from_static("ct"))
    );

    let bytecode = compile_script("if (true) 1; else 2").unwrap();
    assert!(matches!(bytecode.code.last(), Some(Instruction::Return)));
    assert!(
        bytecode
            .code
            .iter()
            .all(|instruction| !matches!(instruction, Instruction::IfFalse(_)))
    );
    assert!(
        bytecode
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Goto(_)))
    );
    assert!(
        bytecode
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::PutLocal(0)))
    );
    assert!(matches!(
        bytecode.code.get(bytecode.code.len() - 2),
        Some(Instruction::GetLocal(0))
    ));

    for source in [
        "if (false) 1",
        "if (true) 1",
        "if (null) 1",
        "if (void 0) 1",
        "if (0) 1",
        "if (1) 1",
    ] {
        let bytecode = compile_script(source).unwrap();
        assert!(
            bytecode.code.iter().all(|instruction| !matches!(
                instruction,
                Instruction::IfFalse(_) | Instruction::IfTrue(_)
            )),
            "QuickJS constant branch did not fold for {source:?}"
        );
    }
    for source in ["if ('') 1", "if (0.5) 1"] {
        let bytecode = compile_script(source).unwrap();
        assert!(
            bytecode
                .code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::IfFalse(_))),
            "QuickJS intentionally does not fold {source:?}"
        );
    }

    let root = compile_unlinked_script("(function(){ 1; })").unwrap();
    assert_eq!(root.metadata().local_count, 1);
    let ordinary = root.constants()[0].as_child().unwrap();
    assert_eq!(ordinary.metadata().local_count, 0);
}

#[test]
fn while_and_do_while_use_per_function_quickjs_loop_controls() {
    assert_eq!(evaluate("1; while (false) 2"), Value::Undefined);
    assert_eq!(evaluate("while (true) { 3; break; }"), Value::Int(3));
    assert_eq!(evaluate("do 4; while (false)"), Value::Int(4));
    assert_eq!(
        evaluate_in_context("do { break; } while (missing)"),
        Value::Undefined
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){ var i=0; var total=0; while(i<5){ i++; if(i===3) continue; total+=i; } return total; })()"
        ),
        Value::Int(12)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){ var i=0; do { i++; if(i<3) continue; } while(i<3); return i; })()"
        ),
        Value::Int(3)
    );

    // Constant folding turns these into closed backward-edge CFGs. They
    // must compile and verify, but deliberately must not be executed.
    for source in ["while(true);", "while(true) continue;", "do{}while(true)"] {
        let bytecode = compile_script(source).unwrap();
        assert!(bytecode.code.iter().enumerate().any(|(pc, instruction)| {
            matches!(instruction, Instruction::Goto(target) if usize::try_from(*target).is_ok_and(|target| target <= pc))
        }));
        assert!(
            bytecode
                .code
                .iter()
                .all(|instruction| !matches!(instruction, Instruction::Goto(u32::MAX)))
        );
    }

    for source in [
        "(function(){ break; })",
        "(function(){ continue; })",
        "while(false) (function(){ break; })",
        "do (function(){ continue; }); while(false)",
    ] {
        assert!(
            compile_unlinked_script(source).is_err(),
            "nested function saw an enclosing loop for {source:?}"
        );
    }
}

#[test]
fn classic_for_uses_quickjs_test_update_and_loop_targets() {
    assert_eq!(evaluate("1; for(;false;) 2"), Value::Undefined);
    assert_eq!(evaluate("for(;;){ 3; break; }"), Value::Int(3));
    assert_eq!(
        evaluate_in_context(
            "(function(){ var sum=0; for(var i=0;i<5;i++){ if(i===2) continue; sum+=i; } return sum; })()"
        ),
        Value::Int(8)
    );
    assert_eq!(
        evaluate_in_context("(function(){ var i=0; for(;;i++){ if(i===3) break; } return i; })()"),
        Value::Int(3)
    );
    assert_eq!(
        evaluate_in_context("(function(){ var i=9; for(i=0;i<3;i++); return i; })()"),
        Value::Int(3)
    );
    assert_eq!(
        evaluate_in_context("(function(){ var i=0; for(;i<3;){ i++; } return i; })()"),
        Value::Int(3)
    );

    for source in ["for(;;);", "for(;;) continue;"] {
        let bytecode = compile_script(source).unwrap();
        assert!(bytecode.code.iter().enumerate().any(|(pc, instruction)| {
            matches!(instruction, Instruction::Goto(target) if usize::try_from(*target).is_ok_and(|target| target <= pc))
        }));
        assert!(
            bytecode
                .code
                .iter()
                .all(|instruction| !matches!(instruction, Instruction::Goto(u32::MAX)))
        );
    }

    compile_unlinked_script("for(Function.item in Function);").unwrap();
    for source in [
        "for(Function.item of Function);",
        "for(Function.item of 'a;b');",
        "for(Function.item of `a;b`);",
        "for(Function.item of /a;b/);",
    ] {
        compile_unlinked_script(source).unwrap();
    }
    for source in [
        "for(Function.item in Function;;);",
        "for(Function.item of Function;;);",
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.message(), "expecting ';'");
    }
    for source in [
        "for(Function.flag ? Function.item in Function : false;;);",
        "for((Function.item in Function);;);",
        "for(Function(Function.item in Function);;);",
        "for(;false;); Function.item in Function",
    ] {
        let bytecode = compile_unlinked_script(source).unwrap();
        assert!(
            bytecode
                .code()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::In)),
            "AllowIn boundary lost its in opcode for {source:?}"
        );
    }
    for source in [
        "for(Function.flag ? false : Function.item in Function;;);",
        "for(Function.item = Function.key in Function;;);",
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(
            error.message(),
            "expecting ';'",
            "NoIn boundary drifted for {source:?}"
        );
    }
    let destructuring =
        compile_unlinked_script("(function(){ var let=Function; for(let[0]=1;false;); })")
            .unwrap_err();
    assert_eq!(destructuring.kind(), ErrorKind::Syntax);
    assert_eq!(destructuring.message(), "invalid destructuring target");
    for source in [
        "(function(){ for(let binding=0;false;); })",
        "(function(){ for(let\nbinding=0;false;); })",
        "(function(){ 'use strict'; for(let binding=0;false;); })",
        "(function(){ for(const binding=0;false;); })",
    ] {
        compile_unlinked_script(source)
            .unwrap_or_else(|error| panic!("lexical for head rejected {source:?}: {error}"));
    }
    assert_eq!(
        evaluate_in_context("(function(){var let=0;for(let=0;let<3;let++);return let;})()"),
        Value::Int(3)
    );
    for source in [
        "for(;;) (function(){ break; })",
        "for(;;) (function(){ continue; })",
    ] {
        assert!(
            compile_unlinked_script(source).is_err(),
            "nested function saw an enclosing for loop for {source:?}"
        );
    }
}

#[test]
fn classic_for_lexicals_close_captured_cells_at_quickjs_boundaries() {
    let script = compile_unlinked_script(
        "(function(){var read;for(let value=0;value<1;value++){read=function(){return value}}return read;})",
    )
    .unwrap();
    let outer = script
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("script lost its lexical-for function");
    assert_eq!(
        outer
            .code()
            .iter()
            .filter(|instruction| { matches!(instruction, Instruction::SetLocalUninitialized(1)) })
            .count(),
        1
    );
    assert_eq!(
        outer
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::CloseLocal(1)))
            .count(),
        3,
        "initializer, normal body fallthrough, and loop exit each need a close site"
    );
    let reader = outer
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("lexical-for function lost its captured reader");
    assert_eq!(
        reader.closure_variables()[0].source,
        ClosureSource::ParentLocal(1)
    );

    let two_binding_script = compile_unlinked_script(
        "(function(){var readLeft,readRight;for(let left=0,right=2;left<1;(left++,right+=left===1?2:1)){readLeft=function(){return left};readRight=function(){return right};}return readLeft()*10+readRight();})()",
    )
    .unwrap();
    let two_binding = two_binding_script
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("script lost its two-binding lexical-for function");
    for index in [2, 3] {
        assert_eq!(
            two_binding
                .code()
                .iter()
                .filter(|instruction| matches!(instruction, Instruction::CloseLocal(found) if *found == index))
                .count(),
            3,
            "captured head local {index} lost one static close site"
        );
    }
    assert!(two_binding.code().iter().all(|instruction| !matches!(
        instruction,
        Instruction::Goto(u32::MAX)
            | Instruction::IfFalse(u32::MAX)
            | Instruction::IfTrue(u32::MAX)
    )));
    assert_eq!(
        evaluate_in_context(
            "(function(){var readLeft,readRight;for(let left=0,right=2;left<1;(left++,right+=left===1?2:1)){readLeft=function(){return left};readRight=function(){return right};}return readLeft()*10+readRight();})()"
        ),
        Value::Int(2)
    );

    assert_eq!(
        evaluate_in_context(
            "(function(){var first,second,third;for(let value=0;value<3;value++){if(value===0)first=function(){return value};else if(value===1)second=function(){return value};else third=function(){return value};}return first()*100+second()*10+third();})()"
        ),
        Value::Int(12)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){var first,second,third;for(let value=0;value<3;value++){if(value===0)first=function(){return value};else if(value===1)second=function(){return value};else third=function(){return value};continue;}return first()*100+second()*10+third();})()"
        ),
        Value::Int(333)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){var first,second,third;for(let value=0;value<3;value++){if(value===0)first=function(){return value};else if(value===1)second=function(){return value};else third=function(){return value};if(value===0)continue;}return first()*100+second()*10+third();})()"
        ),
        Value::Int(112)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){var initial,body;for(let value=(initial=function(){return value},0);value<1;value++){body=function(){return value};value=5;}return initial()*10+body();})()"
        ),
        Value::Int(5)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){var body0,update0,body1;for(let value=0;value<2;(update0=update0||function(){return value},value++)){if(value===0)body0=function(){return value};else body1=function(){return value};}return body0()*100+update0()*10+body1();})()"
        ),
        Value::Int(11)
    );
    assert_eq!(
        evaluate_in_context(
            "Function.saved=undefined;for(let value=0;value<1;value++){Function.saved=function(){return value};}Function.saved()*10+(typeof value==='undefined')"
        ),
        Value::Int(1)
    );
}

#[test]
fn classic_for_lexicals_match_tdz_const_shadow_and_conflict_rules() {
    assert_eq!(
        evaluate_in_context(
            "(function(){let value=9,result;for(let value=0;value<1;value++)result=value;return value*10+result;})()"
        ),
        Value::Int(90)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){var total=0;for(let left=0,right=3;left<right;left++,right--)total+=left+right;return total;})()"
        ),
        Value::Int(6)
    );
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    for (source, name) in [
        ("(function(){for(let value=value;false;);})()", "value"),
        (
            "(function(){for(let first=second,second=1;false;);})()",
            "second",
        ),
    ] {
        assert_eq!(
            evaluate_error(&runtime, &mut context, source),
            (
                JsString::from_static("ReferenceError"),
                JsString::try_from_utf8(&format!("{name} is not initialized")).unwrap()
            ),
            "{source}"
        );
    }
    for (source, message) in [
        (
            "(function(){for(let value=0;value<1;value++){var value;}})",
            "invalid redefinition of lexical identifier",
        ),
        (
            "(function(){for(let value=0,value=1;false;);})",
            "invalid redefinition of lexical identifier",
        ),
        (
            "(function(){for(const value;false;);})",
            "missing initializer for const variable",
        ),
    ] {
        assert_eq!(
            compile_unlinked_script(source).unwrap_err().message(),
            message,
            "{source}"
        );
    }
    for source in [
        "(function(value){for(let value=0;false;);return value;})(7)",
        "(function(){var value=7;for(let value=0;false;);return value;})()",
        "(function(){for(let value=0;false;);var value=7;return value;})()",
    ] {
        assert_eq!(evaluate_in_context(source), Value::Int(7), "{source}");
    }
}

#[test]
fn labels_use_per_function_quickjs_break_control_search() {
    assert_eq!(evaluate("plain: 6;"), Value::Int(6));
    assert_eq!(evaluate("7; empty: ;"), Value::Int(7));
    assert_eq!(evaluate("outer: { 2; break outer; 3; }"), Value::Int(2));
    assert_eq!(
        evaluate_in_context(
            "(function(){ var i=0; var x=0; outer: for(;i<3;i++){ while(true){ x++; continue outer; } } return i+'|'+x; })()"
        ),
        Value::String(JsString::from_static("3|3"))
    );
    assert_eq!(
        evaluate("first: { 1; break first; } first: 2;"),
        Value::Int(2)
    );

    for source in [
        "duplicate: { duplicate: 1; }",
        "regular: { continue regular; }",
        "outer: inner: while(true){ continue outer; }",
        "outer: while(false) (function(){ break outer; })",
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert!(
            matches!(error.kind(), ErrorKind::Syntax),
            "label error was not a SyntaxError for {source:?}: {error}"
        );
    }
    let duplicate = compile_unlinked_script("duplicate: { duplicate: 1; }").unwrap_err();
    assert_eq!(duplicate.message(), "duplicate label name");
    let multiple =
        compile_unlinked_script("outer: inner: while(true){ continue outer; }").unwrap_err();
    assert_eq!(multiple.message(), "break/continue label not found");

    let bytecode = compile_script("outer: while(true){ break outer; }").unwrap();
    assert!(
        bytecode
            .code
            .iter()
            .all(|instruction| !matches!(instruction, Instruction::Goto(u32::MAX)))
    );
}

#[test]
fn switch_uses_quickjs_case_fallthrough_and_abrupt_cleanup() {
    assert_eq!(evaluate("1; switch(0){}"), Value::Undefined);
    assert_eq!(
        evaluate("switch(2){case 1: 1; case 2: 2; case 3: 3;}"),
        Value::Int(3)
    );
    assert_eq!(
        evaluate("switch(9){case 1: 1; default: 4; case 2: 2;}"),
        Value::Int(2)
    );
    assert_eq!(
        evaluate("switch(1){case 1: 1; default: 4; case 2: 2;}"),
        Value::Int(2)
    );
    assert_eq!(
        evaluate("switch(2){case 1: 1; default: 4; case 2: 2;}"),
        Value::Int(2)
    );
    assert_eq!(
        evaluate("switch('1'){case 1: 1; default: 2;}"),
        Value::Int(2)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){var log='';switch((log+='s',2)){case (log+='a',1):log+='A';break;case (log+='b',2):log+='B';break;case (log+='c',3):log+='C';}return log})()"
        ),
        Value::String(JsString::from_static("sabB"))
    );
    assert_eq!(
        evaluate("outer: while(true){switch(1){case 1: break outer;}} 7"),
        Value::Int(7)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){var i=0;outer:while(i++<2){switch(i){case 1:continue outer;default:break;}}return i})()"
        ),
        Value::Int(3)
    );
    assert_eq!(
        evaluate_in_context("(function(){switch(1){case 1:return 4;default:return 5;}})()"),
        Value::Int(4)
    );

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert!(matches!(
        context.eval("switch(1){case 1:throw 4;}"),
        Err(RuntimeError::Exception)
    ));
    assert_eq!(context.take_exception().unwrap(), Some(Value::Int(4)));

    let bytecode =
        compile_unlinked_script("switch(Function){case Function:1;break;default:2;}").unwrap();
    assert!(
        bytecode
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::StrictEq))
    );
    assert!(
        bytecode
            .code()
            .iter()
            .all(|instruction| !matches!(instruction, Instruction::Goto(u32::MAX)))
    );

    for (source, message) in [
        ("switch(0){ 1; }", "invalid switch statement"),
        ("switch(0){default:1;default:2;}", "duplicate default"),
        ("switch(0){case 0 1;}", "expecting ':'"),
        (
            "switch(0){case 0:continue;}",
            "continue must be inside loop",
        ),
    ] {
        assert_eq!(
            compile_unlinked_script(source).unwrap_err().message(),
            message
        );
    }
}
