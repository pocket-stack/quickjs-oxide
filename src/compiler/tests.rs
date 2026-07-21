use crate::bigint::JsBigInt;
use crate::bytecode::{
    ArgumentsKind, DefineMethodKind, DynamicEnvironmentSource, EvalVariableSource, Instruction,
};
use crate::debug::DebugInfoMode;
use crate::error::ErrorKind;
use crate::heap::{
    ClosureSource, ClosureVariable, ClosureVariableKind, ClosureVariableName, ConstructorKind,
    EvalBindingSource, EvalCallerProfile, EvalCallerVariableTarget, EvalKind, EvalRootBinding,
    EvalScopeKind, EvalVariableEnvironment, ParameterDefaultSource,
};
use crate::lexer::{LexError, LexErrorKind, Lexer, Position, Span};
use crate::object::{
    AccessorValue, CompleteOrdinaryPropertyDescriptor, DescriptorField, OrdinaryPropertyDescriptor,
    PropertyKey, WellKnownSymbol,
};
use crate::runtime::{Context, Runtime, RuntimeError};
use crate::value::{JsString, Value};
use crate::vm::Vm;

use super::{
    BindingKind, BindingStorage, EVAL_VARIABLE_OBJECT_LOCAL_NAME, EvalCompileContext, FunctionIr,
    FunctionIrOptions, FunctionKind, FunctionSourceInfo, FunctionTree, HOME_OBJECT_LOCAL_NAME,
    InMode, IrScope, MAX_BYTECODE_STACK, MAX_CALL_ARGUMENTS, MAX_LOCAL_VARIABLES, Parser, ScopeId,
    ScopeKind, SourceOffset, SuperCapabilities, THIS_LOCAL_NAME, WITH_OBJECT_LOCAL_NAME,
    build_scope_lifecycles, compile_script, compile_unlinked_eval_with_filename,
    compile_unlinked_script, compile_unlinked_script_with_filename, ensure_closure_variable,
    lex_error, resolve_identifiers, validate_scope_graph,
};

#[test]
fn string_too_long_lex_error_maps_to_js_internal() {
    let position = Position::new(7, 2, 3);
    let error = lex_error(LexError {
        kind: LexErrorKind::StringTooLong,
        span: Span::new(position, position),
        message: "string too long".to_owned(),
    });

    assert_eq!(error.kind(), ErrorKind::JsInternal);
    assert_eq!(error.message(), "string too long");
    assert_eq!(error.span(), None);
}

#[test]
fn parser_records_quickjs_scope_boundaries_and_child_definition_sites() {
    let source = r#"
        { (function blockChild(){ return 1; }); }
        {}
        if ((function ifChild(){ return true; })()) (function ifBody(){});
        for ((function forChild(){ return 0; })(); false;) (function forBody(){});
        switch ((function discriminant(){ return 0; })()) {
            case (function caseChild(){ return 0; })(): (function bodyChild(){});
        }
    "#;
    let tree = Parser::parse(source, JsString::from_static("<scope-test>")).unwrap();
    let root = &tree.functions[0];
    assert_eq!(
        root.scopes
            .iter()
            .map(|scope| scope.kind)
            .collect::<Vec<_>>(),
        vec![
            ScopeKind::FunctionRoot,
            ScopeKind::ProgramBody,
            ScopeKind::Block,
            ScopeKind::If,
            ScopeKind::For,
            ScopeKind::Switch,
        ]
    );

    let parent_scope_kind = |name: &str| {
        let function = tree.functions[1..]
            .iter()
            .find(|function| function.function_name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("missing parsed child {name}"));
        let scope = function
            .parent
            .expect("child definition scope")
            .definition_scope;
        root.scopes[scope.0].kind
    };
    assert_eq!(parent_scope_kind("blockChild"), ScopeKind::Block);
    assert_eq!(parent_scope_kind("ifChild"), ScopeKind::If);
    assert_eq!(parent_scope_kind("ifBody"), ScopeKind::If);
    assert_eq!(parent_scope_kind("forChild"), ScopeKind::For);
    assert_eq!(parent_scope_kind("forBody"), ScopeKind::For);
    assert_eq!(parent_scope_kind("discriminant"), ScopeKind::ProgramBody);
    assert_eq!(parent_scope_kind("caseChild"), ScopeKind::Switch);
    assert_eq!(parent_scope_kind("bodyChild"), ScopeKind::Switch);
}

#[test]
fn var_bindings_keep_root_storage_and_first_declaration_scope() {
    let source = "(function(a,a){{var x=1;}{var x;}(function child(){return a;});return a+x;})";
    let mut tree = Parser::parse(source, JsString::from_static("<scope-test>")).unwrap();
    let function = &tree.functions[1];
    assert_eq!(function.scopes[0].kind, ScopeKind::FunctionRoot);
    assert_eq!(function.scopes[1].kind, ScopeKind::FunctionBody);
    assert_eq!(function.scopes[2].kind, ScopeKind::Block);
    assert_eq!(function.scopes[3].kind, ScopeKind::Block);

    let parameters = function
        .bindings
        .iter()
        .filter(|binding| binding.name == "a")
        .map(|binding| binding.storage)
        .collect::<Vec<_>>();
    assert_eq!(
        parameters,
        vec![BindingStorage::Argument(0), BindingStorage::Argument(1)]
    );
    let x = function
        .bindings
        .iter()
        .find(|binding| binding.name == "x")
        .expect("function-scoped x binding");
    assert_eq!(x.storage_scope.0, 0);
    assert_eq!(x.declaration_scope.0, 2);
    assert_eq!(x.storage, BindingStorage::Local(0));
    assert_eq!(x.kind, BindingKind::Normal);

    resolve_identifiers(&mut tree).unwrap();
    assert!(
        tree.functions[1]
            .ops
            .iter()
            .any(|operation| matches!(operation.op, super::IrOp::Bytecode(Instruction::GetArg(1))))
    );
    assert!(tree.functions[1].ops.iter().any(|operation| matches!(
        operation.op,
        super::IrOp::Bytecode(Instruction::GetLocal(0))
    )));
    assert_eq!(
        tree.functions[2].closure_variables[0].source,
        ClosureSource::ParentArgument(1)
    );
}

#[test]
fn definition_scope_selects_same_named_sibling_bindings() {
    let source = "{(function left(){return shadow;});}{(function right(){return shadow;});}";
    let mut tree = Parser::parse(source, JsString::from_static("<scope-test>")).unwrap();
    let left_scope = tree.functions[1].parent.unwrap().definition_scope;
    let right_scope = tree.functions[2].parent.unwrap().definition_scope;
    assert_ne!(left_scope, right_scope);

    let root = &mut tree.functions[0];
    let left_local = u16::try_from(root.locals.len()).unwrap();
    root.locals.push("shadow".to_owned());
    root.add_binding(
        left_scope,
        left_scope,
        "shadow".to_owned(),
        BindingStorage::Local(left_local),
        BindingKind::Normal,
        None,
    );
    let right_local = u16::try_from(root.locals.len()).unwrap();
    root.locals.push("shadow".to_owned());
    root.add_binding(
        right_scope,
        right_scope,
        "shadow".to_owned(),
        BindingStorage::Local(right_local),
        BindingKind::Normal,
        None,
    );

    resolve_identifiers(&mut tree).unwrap();
    assert_eq!(
        tree.functions[1].closure_variables[0].source,
        ClosureSource::ParentLocal(left_local)
    );
    assert_eq!(
        tree.functions[2].closure_variables[0].source,
        ClosureSource::ParentLocal(right_local)
    );
}

#[test]
fn ancestor_lookup_uses_each_function_definition_scope() {
    let source = "{(function middle(){return (function leaf(){return shadow;});});}";
    let mut tree = Parser::parse(source, JsString::from_static("<scope-test>")).unwrap();
    let middle_definition_scope = tree.functions[1].parent.unwrap().definition_scope;
    let leaf_definition_scope = tree.functions[2].parent.unwrap().definition_scope;
    assert_eq!(
        tree.functions[1].scopes[leaf_definition_scope.0].kind,
        ScopeKind::FunctionBody
    );

    let root = &mut tree.functions[0];
    let local = u16::try_from(root.locals.len()).unwrap();
    root.locals.push("shadow".to_owned());
    root.add_binding(
        middle_definition_scope,
        middle_definition_scope,
        "shadow".to_owned(),
        BindingStorage::Local(local),
        BindingKind::Normal,
        None,
    );

    resolve_identifiers(&mut tree).unwrap();
    assert_eq!(
        tree.functions[1].closure_variables[0].source,
        ClosureSource::ParentLocal(local)
    );
    assert_eq!(
        tree.functions[2].closure_variables[0].source,
        ClosureSource::ParentClosure(0)
    );
}

#[test]
fn identifier_rewrites_preserve_the_original_use_scope() {
    let source = "(function(value){{typeof value;delete value;value=1;value+=2;value||=3;++value;value++;}for(;;value+=1){break;}})";
    let tree = Parser::parse(source, JsString::from_static("<scope-test>")).unwrap();
    let function = &tree.functions[1];
    let scope_kinds = function
        .ops
        .iter()
        .filter_map(|operation| match operation.op {
            super::IrOp::Identifier { scope, .. } => Some(function.scopes[scope.0].kind),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        scope_kinds
            .iter()
            .filter(|kind| **kind == ScopeKind::Block)
            .count(),
        11
    );
    assert_eq!(
        scope_kinds
            .iter()
            .filter(|kind| **kind == ScopeKind::For)
            .count(),
        2
    );
    assert_eq!(scope_kinds.len(), 13);
}

#[test]
fn resolver_uses_source_order_dfs_postorder_for_sibling_relays() {
    let source = "(function outer(a,b){return (function middle(){(function childA(){return a;});(function childB(){return b;});});})";
    let mut tree = Parser::parse(source, JsString::from_static("<scope-test>")).unwrap();
    resolve_identifiers(&mut tree).unwrap();

    let function_id = |name: &str| {
        tree.functions
            .iter()
            .position(|function| function.function_name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("missing parsed function {name}"))
    };
    let middle = function_id("middle");
    let child_a = function_id("childA");
    let child_b = function_id("childB");
    assert_eq!(
        tree.functions[middle]
            .closure_variables
            .iter()
            .map(|binding| binding.source)
            .collect::<Vec<_>>(),
        vec![
            ClosureSource::ParentArgument(0),
            ClosureSource::ParentArgument(1),
        ]
    );
    assert_eq!(
        tree.functions[child_a].closure_variables[0].source,
        ClosureSource::ParentClosure(0)
    );
    assert_eq!(
        tree.functions[child_b].closure_variables[0].source,
        ClosureSource::ParentClosure(1)
    );
}

#[test]
fn closure_slots_deduplicate_by_storage_identity_and_reject_metadata_conflicts() {
    let span = Span::new(Position::new(0, 1, 1), Position::new(0, 1, 1));
    let mut function = FunctionIr::new(
        None,
        FunctionKind::Ordinary,
        FunctionSourceInfo {
            span,
            definition: SourceOffset::try_from_usize(0).unwrap(),
            range: None,
        },
        FunctionIrOptions {
            function_name: None,
            private_name_binding: false,
            parameters: Vec::new(),
            defined_argument_count: 0,
            has_simple_parameter_list: true,
            rest_parameter: None,
            strict: false,
            super_capabilities: SuperCapabilities::NONE,
        },
    )
    .unwrap();
    let local = ClosureVariable {
        source: ClosureSource::ParentLocal(0),
        name: ClosureVariableName::None,
        is_lexical: false,
        is_const: false,
        kind: ClosureVariableKind::Normal,
    };
    assert_eq!(ensure_closure_variable(&mut function, local).unwrap(), 0);
    assert_eq!(ensure_closure_variable(&mut function, local).unwrap(), 0);

    let other_local = ClosureVariable {
        source: ClosureSource::ParentLocal(1),
        ..local
    };
    assert_eq!(
        ensure_closure_variable(&mut function, other_local).unwrap(),
        1
    );

    let conflict = ClosureVariable {
        is_const: true,
        ..local
    };
    assert_eq!(
        ensure_closure_variable(&mut function, conflict)
            .unwrap_err()
            .message(),
        "closure storage source has conflicting binding metadata"
    );

    for name in [0, 1] {
        ensure_closure_variable(
            &mut function,
            ClosureVariable {
                source: ClosureSource::Global,
                name: ClosureVariableName::Constant(name),
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            },
        )
        .unwrap();
    }
    assert_eq!(function.closure_variables.len(), 4);
}

#[test]
fn scope_graph_validation_rejects_invalid_definition_and_binding_identity() {
    let mut bad_parent = Parser::parse(
        "(function child(){})",
        JsString::from_static("<scope-test>"),
    )
    .unwrap();
    bad_parent.functions[1]
        .parent
        .as_mut()
        .unwrap()
        .definition_scope = super::ScopeId(999);
    assert_eq!(
        resolve_identifiers(&mut bad_parent).unwrap_err().message(),
        "child definition scope is out of bounds"
    );

    let mut duplicate = Parser::parse(
        "(function child(value){return value;})",
        JsString::from_static("<scope-test>"),
    )
    .unwrap();
    let binding = duplicate.functions[1].scopes[0].bindings[0];
    duplicate.functions[1].scopes[0].bindings.push(binding);
    assert_eq!(
        resolve_identifiers(&mut duplicate).unwrap_err().message(),
        "binding appears more than once in the scope graph"
    );

    let mut aliased_slot = Parser::parse(
        "(function child(value,value){return value;})",
        JsString::from_static("<scope-test>"),
    )
    .unwrap();
    aliased_slot.functions[1].bindings[1].storage = BindingStorage::Argument(0);
    assert_eq!(
        resolve_identifiers(&mut aliased_slot)
            .unwrap_err()
            .message(),
        "argument slot has more than one binding identity"
    );

    let mut missing_slot = Parser::parse(
        "(function child(value){return value;})",
        JsString::from_static("<scope-test>"),
    )
    .unwrap();
    missing_slot.functions[1].scopes[0].bindings.clear();
    missing_slot.functions[1].bindings.clear();
    assert_eq!(
        resolve_identifiers(&mut missing_slot)
            .unwrap_err()
            .message(),
        "argument slot is missing its binding identity"
    );

    let mut malformed_scope = Parser::parse("0", JsString::from_static("<scope-test>")).unwrap();
    malformed_scope.functions[0].scopes.push(super::IrScope {
        parent: Some(super::ScopeId(0)),
        kind: ScopeKind::ProgramBody,
        bindings: Vec::new(),
    });
    malformed_scope.functions[0].body_scope = super::ScopeId(2);
    malformed_scope.functions[0].current_scope = super::ScopeId(2);
    malformed_scope.functions[0].scopes[1].parent = Some(super::ScopeId(99));
    assert_eq!(
        resolve_identifiers(&mut malformed_scope)
            .unwrap_err()
            .message(),
        "lexical scope parent is malformed"
    );
}

fn evaluate(source: &str) -> Value {
    let bytecode = compile_script(source).unwrap();
    Vm::new().execute(&bytecode).unwrap()
}

fn evaluate_in_context(source: &str) -> Value {
    Runtime::new().new_context().eval(source).unwrap()
}

fn evaluate_error(runtime: &Runtime, context: &mut Context, source: &str) -> (JsString, JsString) {
    assert_eq!(
        context.eval(source),
        Err(RuntimeError::Exception),
        "{source}"
    );
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("source did not throw an Error object: {source}");
    };
    let name = runtime.intern_property_key("name").unwrap();
    let message = runtime.intern_property_key("message").unwrap();
    let Value::String(name) = context.get_property(&error, &name).unwrap() else {
        panic!("Error.name was not a string: {source}");
    };
    let Value::String(message) = context.get_property(&error, &message).unwrap() else {
        panic!("Error.message was not a string: {source}");
    };
    (name, message)
}

fn evaluate_function_name(source: &str) -> (JsString, bool, bool, bool) {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let Value::Object(function) = context.eval(source).unwrap() else {
        panic!("source did not evaluate to a function object");
    };
    let name = runtime.intern_property_key("name").unwrap();
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::String(value),
        writable,
        enumerable,
        configurable,
    } = runtime.get_own_property(&function, &name).unwrap().unwrap()
    else {
        panic!("function name did not have the ordinary data descriptor");
    };
    (value, writable, enumerable, configurable)
}

#[test]
fn ordinary_function_body_lexicals_execute_local_capture_and_constructor_paths() {
    assert_eq!(
        evaluate_in_context(
            "(function(){let x=1,y=x+1,z;return x*10+y+(typeof z==='undefined'?0:100)})()"
        ),
        Value::Int(12)
    );
    assert_eq!(
        evaluate_in_context("(function(){let arguments=3;let eval=4;return arguments+eval})()"),
        Value::Int(7)
    );
    assert_eq!(
        evaluate_in_context("(function(){var read=function(){return x};let x=7;return read()})()"),
        Value::Int(7)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){let x=function(){return x};return x()===x&&x.name==='x'})()"
        ),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){let x=1;var next=function(){x+=1;return x};return next()*10+next()})()"
        ),
        Value::Int(23)
    );
    assert_eq!(
        evaluate_in_context("(function(){const x=4;return function(){return x}()})()"),
        Value::Int(4)
    );
    assert_eq!(
        evaluate_in_context("Function('let x=1;const y=2;return x+y')()"),
        Value::Int(3)
    );
    assert_eq!(
        evaluate_in_context("(function(){var result=delete x;let x=1;return result})()"),
        Value::Bool(false)
    );
    assert_eq!(
        evaluate_in_context("(function(){const x=0;return x&&=missing})()"),
        Value::Int(0)
    );
}

#[test]
fn lexical_lowering_publishes_tdz_vardefs_and_checked_capture_relays() {
    let script =
        compile_unlinked_script("(function(){let x=1;const y=2;return function(){x+=y;return x}})")
            .unwrap();
    let outer = script
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("script lost its outer function");
    assert!(matches!(
        outer.code(),
        [
            Instruction::SetLocalUninitialized(1),
            Instruction::SetLocalUninitialized(0),
            Instruction::PushI32(1),
            Instruction::InitializeLocal(0),
            Instruction::PushI32(2),
            Instruction::InitializeLocal(1),
            ..
        ]
    ));
    assert_eq!(outer.local_definitions().len(), 2);
    assert_eq!(
        outer.local_definitions()[0].name.as_ref(),
        Some(&JsString::from_static("x"))
    );
    assert!(outer.local_definitions()[0].is_lexical);
    assert!(!outer.local_definitions()[0].is_const);
    assert_eq!(
        outer.local_definitions()[1].name.as_ref(),
        Some(&JsString::from_static("y"))
    );
    assert!(outer.local_definitions()[1].is_lexical);
    assert!(outer.local_definitions()[1].is_const);

    let inner = outer
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("outer function lost its captured child");
    assert_eq!(inner.closure_variables().len(), 2);
    for (index, expected_name, is_const) in [(0, "x", false), (1, "y", true)] {
        let descriptor = inner.closure_variables()[index];
        assert_eq!(
            descriptor.source,
            ClosureSource::ParentLocal(u16::try_from(index).unwrap())
        );
        assert!(descriptor.is_lexical);
        assert_eq!(descriptor.is_const, is_const);
        assert_eq!(descriptor.kind, ClosureVariableKind::Normal);
        let ClosureVariableName::Constant(name) = descriptor.name else {
            panic!("lexical descriptor lost its source name");
        };
        assert_eq!(
            inner.constants()[usize::try_from(name).unwrap()].as_primitive(),
            Some(&Value::String(JsString::from_static(expected_name)))
        );
    }
    assert!(inner.code().windows(5).any(|window| matches!(
        window,
        [
            Instruction::GetVarRefCheck(0),
            Instruction::GetVarRefCheck(1),
            Instruction::Add,
            Instruction::Dup,
            Instruction::PutVarRefCheck(0),
        ]
    )));
    assert!(inner.code().windows(2).any(|window| matches!(
        window,
        [Instruction::GetVarRefCheck(0), Instruction::Return]
    )));
}

#[test]
fn captured_with_object_has_close_lifetime_without_lexical_tdz() {
    let make_function = |strict| {
        let span = Span::new(Position::new(0, 1, 1), Position::new(0, 1, 1));
        let mut function = FunctionIr::new(
            None,
            FunctionKind::Ordinary,
            FunctionSourceInfo {
                span,
                definition: SourceOffset::try_from_usize(0).unwrap(),
                range: None,
            },
            FunctionIrOptions {
                function_name: None,
                private_name_binding: false,
                parameters: Vec::new(),
                defined_argument_count: 0,
                has_simple_parameter_list: true,
                rest_parameter: None,
                strict,
                super_capabilities: SuperCapabilities::NONE,
            },
        )
        .unwrap();
        let scope = ScopeId(function.scopes.len());
        function.scopes.push(IrScope {
            parent: Some(function.body_scope),
            kind: ScopeKind::With,
            bindings: Vec::new(),
        });
        function.locals.push(WITH_OBJECT_LOCAL_NAME.to_owned());
        function.add_binding(
            scope,
            scope,
            WITH_OBJECT_LOCAL_NAME.to_owned(),
            BindingStorage::Local(0),
            BindingKind::WithObject,
            None,
        );
        (function, scope)
    };

    let (function, scope) = make_function(false);
    let lifecycles = build_scope_lifecycles(&function, &[true]).unwrap();
    assert!(lifecycles[scope.0].tdz_locals.is_empty());
    assert!(lifecycles[scope.0].function_entries.is_empty());
    assert_eq!(lifecycles[scope.0].close_locals, [0]);

    let (strict, _) = make_function(true);
    let tree = FunctionTree {
        functions: vec![strict],
        source: "".into(),
        filename: JsString::from_static("<strict-with-metadata>"),
    };
    assert!(
        validate_scope_graph(&tree)
            .unwrap_err()
            .message()
            .contains("strict function retained a local with object")
    );
}

#[test]
fn nested_block_and_switch_lexicals_lower_scope_lifetimes() {
    let script = compile_unlinked_script(
        "(function(){var read;{read=function(){return ++value};let value=40;}return read()*100+read();})()",
    )
    .unwrap();
    let outer = script
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("script lost its block function");
    assert!(
        outer
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::SetLocalUninitialized(1)))
    );
    assert!(
        outer
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::CloseLocal(1)))
    );
    let child = outer
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("block function lost its captured child");
    assert_eq!(
        child.closure_variables()[0].source,
        ClosureSource::ParentLocal(1)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){var read;{read=function(){return ++value};let value=40;}return read()*100+read();})()"
        ),
        Value::Int(4142)
    );

    let switch_script = compile_unlinked_script(
        "(function(){var read;switch(0){case 0:let value=40;read=function(){return ++value};break;}return read()*100+read();})()",
    )
    .unwrap();
    let switch_function = switch_script
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("script lost its switch function");
    assert!(switch_function.code().windows(2).any(|window| matches!(
        window,
        [
            Instruction::PushI32(0),
            Instruction::SetLocalUninitialized(1)
        ]
    )));
    assert!(
        switch_function
            .code()
            .windows(2)
            .any(|window| matches!(window, [Instruction::Drop, Instruction::CloseLocal(1)]))
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){var read;switch(0){case 0:let value=40;read=function(){return ++value};break;}return read()*100+read();})()"
        ),
        Value::Int(4142)
    );
    assert_eq!(evaluate_in_context("{let value=42;value;}"), Value::Int(42));
    assert_eq!(
        evaluate_in_context("switch(0){case 0:const value=42;value;}"),
        Value::Int(42)
    );

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert!(matches!(
        context.eval("(function(){{let value=40;throw function(){return ++value};}})()"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(thrown) = context.take_exception().unwrap().unwrap() else {
        panic!("nested lexical throw did not preserve the escaped closure");
    };
    let callable = runtime.as_callable(&thrown).unwrap().unwrap();
    assert_eq!(
        context.call(&callable, Value::Undefined, &[]).unwrap(),
        Value::Int(41)
    );
    assert_eq!(
        context.call(&callable, Value::Undefined, &[]).unwrap(),
        Value::Int(42)
    );
}

#[test]
fn nested_lexical_cleanup_shadowing_and_quickjs_var_quirks_execute() {
    assert_eq!(
        evaluate_in_context(
            "(function(){var first,second,index=0;while(index<2){{let value=index++;if(index===1){first=function(){return ++value};continue;}second=function(){return ++value};}}return first()*100+second()*10+first()+second();})()"
        ),
        Value::Int(125)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){var first,second,index=0;outer:while(index<2){switch(0){case 0:let value=index++;if(index===1){first=function(){return ++value};continue outer;}second=function(){return ++value};break outer;}}return first()*100+second()*10+first()+second();})()"
        ),
        Value::Int(125)
    );
    assert_eq!(
        evaluate_in_context(
            "(function self(parameter){var outer='O',result;{let parameter='P',outer='B',self='S';result=parameter+outer+self;}return result+'|'+parameter+'|'+outer+'|'+typeof self;})('p')"
        ),
        Value::String(JsString::from_static("PBS|p|O|function"))
    );
    for source in [
        "(function(){var value;{var value;let value;}return 1})()",
        "(function(value){{var value;let value;}return 1})(0)",
    ] {
        assert_eq!(evaluate_in_context(source), Value::Int(1), "{source}");
    }
    for (source, message) in [
        (
            "(function(){var value;{let value;var value;}})",
            "invalid redefinition of lexical identifier",
        ),
        (
            "(function(){{var value;}let value;})",
            "invalid redefinition of a variable",
        ),
        (
            "(function(){let value;{var value;}})",
            "invalid redefinition of lexical identifier",
        ),
        (
            "(function(){switch(0){case 0:let value;case 1:const value=1;}})",
            "invalid redefinition of lexical identifier",
        ),
    ] {
        assert_eq!(
            compile_unlinked_script(source).unwrap_err().message(),
            message,
            "{source}"
        );
    }
}

#[test]
fn lexical_tdz_and_readonly_errors_follow_checked_local_and_capture_order() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    for source in [
        "(function(){return x;let x=1})()",
        "(function(){return typeof x;let x=1})()",
        "(function(){x=1;let x})()",
        "(function(){return function(){return x};let x=1})()()",
        "(function(){var set=function(){x=1};set();let x})()",
        "(function(){var add=function(){x+=missing};add();const x=1})()",
    ] {
        assert_eq!(
            evaluate_error(&runtime, &mut context, source),
            (
                JsString::from_static("ReferenceError"),
                JsString::from_static("x is not initialized")
            ),
            "{source}"
        );
    }
    for source in [
        "(function(){const x=1;x=2})()",
        "(function(){const x=1;return function(){x=2}})()()",
        "(function(){var set=function(){x=1};set();const x=2})()",
    ] {
        assert_eq!(
            evaluate_error(&runtime, &mut context, source),
            (
                JsString::from_static("TypeError"),
                JsString::from_static("'x' is read-only")
            ),
            "{source}"
        );
    }
}

#[test]
fn lexical_parser_matches_redefinition_priority_contextual_let_and_boundaries() {
    let syntax_cases = [
        (
            "(function(){\nlet x;\nlet x;\n})",
            "invalid redefinition of lexical identifier",
            3,
            6,
        ),
        (
            "(function(){\nvar x;\nlet x;\n})",
            "invalid redefinition of a variable",
            3,
            6,
        ),
        (
            "(function(){\nlet x;\nvar x;\n})",
            "invalid redefinition of lexical identifier",
            3,
            6,
        ),
        (
            "(function(x){\nlet x;\n})",
            "invalid redefinition of parameter name",
            2,
            6,
        ),
        (
            "(function(){\nconst x;\n})",
            "missing initializer for const variable",
            2,
            8,
        ),
    ];
    for (source, message, line, column) in syntax_cases {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(error.message(), message, "{source}");
        let span = error.span().expect("syntax error lost its source span");
        assert_eq!(
            (span.start.line, span.start.column),
            (line, column),
            "{source}"
        );
    }
    for (source, message) in [
        (
            "(function(){let let=1})",
            "'let' is not a valid lexical identifier",
        ),
        (
            "(function(){'use strict';let eval=1})",
            "invalid variable name in strict mode",
        ),
        (
            "(function(){if(true) let x=1})",
            "lexical declarations can't appear in single-statement context",
        ),
    ] {
        assert_eq!(
            compile_unlinked_script(source).unwrap_err().message(),
            message
        );
    }

    assert_eq!(
        evaluate_in_context("(function(){var let=0;let=2;return let})()"),
        Value::Int(2)
    );
    assert_eq!(
        evaluate_in_context("(function(){var x=0;if(false) let\nx=1;return x})()"),
        Value::Int(1)
    );
    assert_eq!(
        evaluate_in_context("(function named(){let named=3;return named})()"),
        Value::Int(3)
    );

    assert_eq!(
        evaluate_in_context("(function(){let [[item]]=[[1]];return item})()"),
        Value::Int(1)
    );
    assert_eq!(
        evaluate_in_context("(function(){{let [[item]=[2]]=[];return item}})()"),
        Value::Int(2)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){switch(0){case 0:let [...[first,second]]=[3,4];return first+second}})()"
        ),
        Value::Int(7)
    );
    assert_eq!(
        evaluate_in_context("(function(){{let nested=1;return nested}})()"),
        Value::Int(1)
    );
    assert_eq!(
        evaluate_in_context("(function(){switch(0){case 0:let inCase=1;return inCase}})()"),
        Value::Int(1)
    );
}

#[test]
fn program_lexicals_lower_to_source_ordered_global_declarations() {
    let source = "let first=1,second=function(){return later};const later=2;first+second()";
    let script = compile_unlinked_script(source).unwrap();
    assert_eq!(script.local_definitions().len(), 1);
    assert_eq!(script.closure_variables().len(), 3);

    let declaration_names = script
        .closure_variables()
        .iter()
        .map(|descriptor| {
            assert_eq!(descriptor.source, ClosureSource::GlobalDeclaration);
            assert!(descriptor.is_lexical);
            let ClosureVariableName::Constant(index) = descriptor.name else {
                panic!("global declaration lost its semantic name");
            };
            let Value::String(name) = script.constants()[index as usize]
                .as_primitive()
                .expect("global declaration name is not primitive")
            else {
                panic!("global declaration name is not a string");
            };
            name.to_utf8_lossy()
        })
        .collect::<Vec<_>>();
    assert_eq!(declaration_names, ["first", "second", "later"]);
    assert_eq!(
        script
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::PutVarInit(_)))
            .count(),
        3
    );

    let child = script
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("global lexical initializer lost its child function");
    assert_eq!(child.closure_variables().len(), 1);
    assert_eq!(
        child.closure_variables()[0].source,
        ClosureSource::ParentGlobal(2)
    );
    assert!(child.closure_variables()[0].is_lexical);
    assert!(child.closure_variables()[0].is_const);

    let stripped =
        compile_unlinked_script_with_filename(source, "<global-strip>", DebugInfoMode::StripDebug)
            .unwrap();
    assert!(stripped.closure_variables().iter().all(|descriptor| {
        descriptor.source == ClosureSource::GlobalDeclaration
            && matches!(descriptor.name, ClosureVariableName::Constant(_))
    }));
    let stripped_child = stripped
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("stripped global lexical lost its child function");
    assert!(matches!(
        stripped_child.closure_variables()[0].name,
        ClosureVariableName::Constant(_)
    ));
}

#[test]
fn program_vars_keep_every_source_ordered_global_declaration() {
    let source = "var first;{var first;var second=function(){return later}}if(false)var later=3;for(var loop=0;false;){}";
    let script = compile_unlinked_script(source).unwrap();
    assert_eq!(script.local_definitions().len(), 1);

    let declaration_names = script
        .closure_variables()
        .iter()
        .map(|descriptor| {
            assert_eq!(descriptor.source, ClosureSource::GlobalDeclaration);
            assert!(!descriptor.is_lexical);
            assert!(!descriptor.is_const);
            assert_eq!(descriptor.kind, ClosureVariableKind::Normal);
            let ClosureVariableName::Constant(index) = descriptor.name else {
                panic!("global var declaration lost its semantic name");
            };
            let Value::String(name) = script.constants()[index as usize]
                .as_primitive()
                .expect("global var declaration name is not primitive")
            else {
                panic!("global var declaration name is not a string");
            };
            name.to_utf8_lossy()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        declaration_names,
        ["first", "first", "second", "later", "loop"]
    );
    assert_eq!(
        script
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::PutVar(_)))
            .count(),
        3
    );
    assert!(
        !script
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::PutVarInit(_)))
    );

    let child = script
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("global var initializer lost its child function");
    assert_eq!(child.closure_variables().len(), 1);
    assert_eq!(
        child.closure_variables()[0].source,
        ClosureSource::ParentGlobal(3)
    );
    assert!(!child.closure_variables()[0].is_lexical);

    let stripped = compile_unlinked_script_with_filename(
        source,
        "<global-var-strip>",
        DebugInfoMode::StripDebug,
    )
    .unwrap();
    assert!(stripped.closure_variables().iter().all(|descriptor| {
        descriptor.source == ClosureSource::GlobalDeclaration
            && matches!(descriptor.name, ClosureVariableName::Constant(_))
    }));
}

#[test]
fn program_functions_keep_descriptors_but_hoist_into_the_first_name_slot() {
    let script = compile_unlinked_script(
        "let mixed;function mixed(){return mixed}function repeated(){return 1}function repeated(){return 2}repeated",
    )
    .unwrap();
    assert_eq!(script.closure_variables().len(), 4);
    assert!(script.closure_variables()[0].is_lexical);
    assert_eq!(
        script.closure_variables()[1].kind,
        ClosureVariableKind::GlobalFunction
    );
    assert_eq!(
        script.closure_variables()[2].kind,
        ClosureVariableKind::GlobalFunction
    );
    assert_eq!(
        script.closure_variables()[3].kind,
        ClosureVariableKind::GlobalFunction
    );
    assert!(matches!(script.code()[0], Instruction::FClosure(0)));
    assert!(matches!(script.code()[1], Instruction::PutVarInit(0)));
    assert!(matches!(script.code()[2], Instruction::FClosure(1)));
    assert!(matches!(script.code()[3], Instruction::PutVarInit(2)));
    assert!(matches!(script.code()[4], Instruction::FClosure(2)));
    assert!(matches!(script.code()[5], Instruction::PutVarInit(2)));

    let children = script
        .constants()
        .iter()
        .filter_map(|constant| constant.as_child())
        .collect::<Vec<_>>();
    assert_eq!(children.len(), 3);
    assert_eq!(children[0].metadata().function_name_local, None);
    assert_eq!(
        children[0].closure_variables()[0].source,
        ClosureSource::ParentGlobal(0)
    );
    assert!(children[0].closure_variables()[0].is_lexical);
    for child in &children[1..] {
        assert_eq!(child.metadata().function_name_local, None);
    }
}

#[test]
fn program_function_then_lexical_remains_a_source_ordered_syntax_error() {
    let error = compile_unlinked_script("function clash(){};let clash").unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Syntax);
    assert_eq!(error.message(), "invalid redefinition of global identifier");
    assert!(compile_unlinked_script("let clash;function clash(){}").is_ok());
}

#[test]
fn program_vars_instantiate_persist_and_preserve_existing_properties() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context
            .eval("var value;var value=2;if(false){var dormant=3}for(var loop=0;loop<2;loop++){};value+'|'+typeof dormant+'|'+loop")
            .unwrap(),
        Value::String(JsString::from_static("2|undefined|2"))
    );
    let global = context.global_object().unwrap();
    for (name, value) in [
        ("value", Value::Int(2)),
        ("dormant", Value::Undefined),
        ("loop", Value::Int(2)),
    ] {
        let key = runtime.intern_property_key(name).unwrap();
        assert_eq!(
            context.get_own_property(&global, &key).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value,
                writable: true,
                enumerable: true,
                configurable: false,
            })
        );
    }
    assert_eq!(context.eval("var value;value").unwrap(), Value::Int(2));
    assert_eq!(
        context
            .eval("var captured=1,read=function(){return captured};captured=4;read()")
            .unwrap(),
        Value::Int(4)
    );
    assert_eq!(context.eval("delete value").unwrap(), Value::Bool(false));

    assert_eq!(context.eval("hostValue=7").unwrap(), Value::Int(7));
    let host = runtime.intern_property_key("hostValue").unwrap();
    assert_eq!(
        context.eval("var hostValue;hostValue").unwrap(),
        Value::Int(7)
    );
    assert_eq!(
        context.get_own_property(&global, &host).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(7),
            writable: true,
            enumerable: true,
            configurable: true,
        })
    );
    assert_eq!(
        context
            .eval("Function.hostRead=function(){return hostValue};var hostValue=8;delete hostValue")
            .unwrap(),
        Value::Bool(true)
    );
    assert!(matches!(
        context.eval("Function.hostRead()"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();
    assert_eq!(
        context.eval("var hostValue;hostValue").unwrap(),
        Value::Undefined
    );
    assert_eq!(
        context.eval("Function.hostRead()").unwrap(),
        Value::Undefined
    );
    assert_eq!(
        context.get_own_property(&global, &host).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Undefined,
            writable: true,
            enumerable: true,
            configurable: false,
        })
    );

    let fixed = runtime.intern_property_key("fixedVar").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &fixed,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Int(1)),
                    writable: DescriptorField::Present(false),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(
        context.eval("var fixedVar;fixedVar").unwrap(),
        Value::Int(1)
    );
    assert_eq!(
        context.eval("var fixedVar=2;fixedVar").unwrap(),
        Value::Int(1)
    );
    assert!(matches!(
        context.eval("'use strict';var fixedVar=2"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();
    assert_eq!(
        context.get_own_property(&global, &fixed).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(1),
            writable: false,
            enumerable: false,
            configurable: true,
        })
    );

    assert_eq!(
        context.eval("Function.varSetterHits=0").unwrap(),
        Value::Int(0)
    );
    let Value::Object(setter) = context
        .eval("(function(value){Function.varSetterHits=value})")
        .unwrap()
    else {
        panic!("global var accessor probe did not create a setter");
    };
    let setter = runtime.as_callable(&setter).unwrap().unwrap();
    let accessor = runtime.intern_property_key("accessorVar").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &accessor,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Undefined),
                    set: DescriptorField::Present(AccessorValue::Callable(setter.clone())),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(
        context
            .eval("var accessorVar;Function.varSetterHits")
            .unwrap(),
        Value::Int(0),
        "a var without initializer must not invoke an existing setter"
    );
    assert_eq!(
        context
            .eval("var accessorVar=9;Function.varSetterHits")
            .unwrap(),
        Value::Int(9)
    );
    assert_eq!(
        context.get_own_property(&global, &accessor).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Accessor {
            get: None,
            set: Some(setter),
            enumerable: false,
            configurable: true,
        })
    );

    let mut inherited = runtime.new_context();
    let inherited_key = runtime.intern_property_key("inheritedVar").unwrap();
    let prototype = inherited.object_prototype().unwrap();
    assert!(
        inherited
            .define_own_property(
                &prototype,
                &inherited_key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Int(5)),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(
        inherited.eval("var inheritedVar;inheritedVar").unwrap(),
        Value::Undefined
    );
    let inherited_global = inherited.global_object().unwrap();
    assert_eq!(
        inherited
            .get_own_property(&inherited_global, &inherited_key)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Undefined,
            writable: true,
            enumerable: true,
            configurable: false,
        })
    );

    let mut auto_init = runtime.new_context();
    assert_eq!(
        auto_init.eval("var Number;typeof Number").unwrap(),
        Value::String(JsString::from_static("function"))
    );
    assert_eq!(
        auto_init.eval("var Number=9;Number").unwrap(),
        Value::Int(9)
    );
    let number = runtime.intern_property_key("Number").unwrap();
    let auto_init_global = auto_init.global_object().unwrap();
    assert_eq!(
        auto_init
            .get_own_property(&auto_init_global, &number)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(9),
            writable: true,
            enumerable: false,
            configurable: true,
        })
    );
}

#[test]
fn program_var_preflight_conflicts_and_parser_scope_match_quickjs() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context.eval("let existingLexical=1").unwrap(),
        Value::Undefined
    );
    assert!(matches!(
        context.eval("var existingLexical"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("global var/lexical conflict did not throw an Error object");
    };
    let name = runtime.intern_property_key("name").unwrap();
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.get_property(&error, &name).unwrap(),
        Value::String(JsString::from_static("SyntaxError"))
    );
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static("redeclaration of 'existingLexical'"))
    );
    assert_eq!(
        context.eval("globalThis.varMarker=0").unwrap(),
        Value::Int(0)
    );
    assert!(matches!(
        context.eval("varMarker=1;var freshBefore=(varMarker=2),existingLexical=(varMarker=3),freshAfter=(varMarker=4)"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();
    assert_eq!(context.eval("varMarker").unwrap(), Value::Int(0));
    assert_eq!(
        context
            .eval("typeof freshBefore+'|'+typeof freshAfter")
            .unwrap(),
        Value::String(JsString::from_static("undefined|undefined"))
    );

    let mut sealed = runtime.new_context();
    assert_eq!(
        sealed.eval("let sealedLexical=1").unwrap(),
        Value::Undefined
    );
    let sealed_global = sealed.global_object().unwrap();
    runtime.prevent_extensions(&sealed_global).unwrap();
    assert!(matches!(
        sealed.eval("var sealedLexical"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = sealed.take_exception().unwrap().unwrap() else {
        panic!("sealed global var declaration did not throw an Error object");
    };
    assert_eq!(
        sealed.get_property(&error, &name).unwrap(),
        Value::String(JsString::from_static("TypeError"))
    );
    assert_eq!(
        sealed.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static(
            "cannot define variable 'sealedLexical'"
        ))
    );

    let mut atomic = runtime.new_context();
    assert_eq!(
        atomic
            .eval("globalThis.atomicExisting=5;globalThis.atomicMarker=0")
            .unwrap(),
        Value::Int(0)
    );
    let atomic_global = atomic.global_object().unwrap();
    runtime.prevent_extensions(&atomic_global).unwrap();
    assert!(matches!(
        atomic.eval("atomicMarker=1;var atomicExisting=6,atomicMissing=7"),
        Err(RuntimeError::Exception)
    ));
    atomic.take_exception().unwrap().unwrap();
    assert_eq!(atomic.eval("atomicMarker").unwrap(), Value::Int(0));
    assert_eq!(atomic.eval("atomicExisting").unwrap(), Value::Int(5));
    assert_eq!(
        atomic.eval("typeof atomicMissing").unwrap(),
        Value::String(JsString::from_static("undefined"))
    );

    for (source, expected) in [
        (
            "let conflict;var conflict",
            "invalid redefinition of lexical identifier",
        ),
        (
            "var conflict;let conflict",
            "invalid redefinition of global identifier",
        ),
        (
            "{var conflict;var conflict;let conflict}",
            "invalid redefinition of global identifier",
        ),
        (
            "{let conflict;var conflict}",
            "invalid redefinition of lexical identifier",
        ),
    ] {
        assert_eq!(
            compile_unlinked_script(source).unwrap_err().message(),
            expected,
            "{source}"
        );
    }
    for source in [
        "var allowed;{var allowed;let allowed}",
        "{var sibling}{var sibling;let sibling}",
        "{let shadow}var shadow",
    ] {
        compile_unlinked_script(source).unwrap();
    }
}

#[test]
fn program_var_cross_realm_instantiation_and_fallback_match_quickjs() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();

    let fresh = defining.compile("var crossVar=41;crossVar+1").unwrap();
    assert_eq!(caller.execute(&fresh).unwrap(), Value::Int(42));
    assert_eq!(
        defining.eval("typeof crossVar").unwrap(),
        Value::String(JsString::from_static("undefined"))
    );
    assert_eq!(caller.eval("crossVar").unwrap(), Value::Int(41));

    assert_eq!(
        defining.eval("crossData='A'").unwrap(),
        Value::String(JsString::from_static("A"))
    );
    assert_eq!(
        caller.eval("crossData='B'").unwrap(),
        Value::String(JsString::from_static("B"))
    );
    let data = defining
        .compile("var crossData='written';crossData")
        .unwrap();
    assert_eq!(
        caller.execute(&data).unwrap(),
        Value::String(JsString::from_static("written"))
    );
    assert_eq!(
        defining.eval("crossData").unwrap(),
        Value::String(JsString::from_static("A"))
    );
    assert_eq!(
        caller.eval("crossData").unwrap(),
        Value::String(JsString::from_static("written"))
    );

    assert_eq!(
        defining.eval("Function.aSeen='none'").unwrap(),
        Value::String(JsString::from_static("none"))
    );
    assert_eq!(
        caller.eval("Function.bSeen='none'").unwrap(),
        Value::String(JsString::from_static("none"))
    );
    let Value::Object(a_getter) = defining.eval("(function(){return 'Aget'})").unwrap() else {
        panic!("defining realm accessor getter was not callable");
    };
    let Value::Object(a_setter) = defining
        .eval("(function(value){Function.aSeen=value})")
        .unwrap()
    else {
        panic!("defining realm accessor setter was not callable");
    };
    let Value::Object(b_getter) = caller.eval("(function(){return 'Bget'})").unwrap() else {
        panic!("caller realm accessor getter was not callable");
    };
    let Value::Object(b_setter) = caller
        .eval("(function(value){Function.bSeen=value})")
        .unwrap()
    else {
        panic!("caller realm accessor setter was not callable");
    };
    let a_getter = runtime.as_callable(&a_getter).unwrap().unwrap();
    let a_setter = runtime.as_callable(&a_setter).unwrap().unwrap();
    let b_getter = runtime.as_callable(&b_getter).unwrap().unwrap();
    let b_setter = runtime.as_callable(&b_setter).unwrap().unwrap();
    let accessor = runtime.intern_property_key("crossAccessor").unwrap();
    for (context, getter, setter) in [
        (&mut defining, a_getter, a_setter),
        (&mut caller, b_getter, b_setter),
    ] {
        let global = context.global_object().unwrap();
        assert!(
            context
                .define_own_property(
                    &global,
                    &accessor,
                    &OrdinaryPropertyDescriptor {
                        get: DescriptorField::Present(AccessorValue::Callable(getter)),
                        set: DescriptorField::Present(AccessorValue::Callable(setter)),
                        enumerable: DescriptorField::Present(true),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
    }
    let accessor_script = defining
        .compile("var crossAccessor='written';crossAccessor")
        .unwrap();
    assert_eq!(
        caller.execute(&accessor_script).unwrap(),
        Value::String(JsString::from_static("Aget"))
    );
    assert_eq!(
        defining.eval("crossAccessor+'|'+Function.aSeen").unwrap(),
        Value::String(JsString::from_static("Aget|written"))
    );
    assert_eq!(
        caller.eval("crossAccessor+'|'+Function.bSeen").unwrap(),
        Value::String(JsString::from_static("Bget|none"))
    );

    let readonly = runtime.intern_property_key("crossReadonly").unwrap();
    let defining_global = defining.global_object().unwrap();
    assert!(
        defining
            .define_own_property(
                &defining_global,
                &readonly,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Int(1)),
                    writable: DescriptorField::Present(false),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let caller_global = caller.global_object().unwrap();
    assert!(
        caller
            .define_own_property(
                &caller_global,
                &readonly,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Undefined),
                    set: DescriptorField::Present(AccessorValue::Undefined),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let Value::Object(defining_type_prototype) = defining.eval("TypeError.prototype").unwrap()
    else {
        panic!("defining TypeError.prototype was not an object");
    };
    let readonly_script = defining
        .compile("'use strict';var crossReadonly=2")
        .unwrap();
    assert!(matches!(
        caller.execute(&readonly_script),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = caller.take_exception().unwrap().unwrap() else {
        panic!("cross-realm var initializer did not throw an Error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(defining_type_prototype)
    );

    let mut syntax_caller = runtime.new_context();
    syntax_caller.eval("let crossConflict=1").unwrap();
    let Value::Object(syntax_prototype) = syntax_caller.eval("SyntaxError.prototype").unwrap()
    else {
        panic!("caller SyntaxError.prototype was not an object");
    };
    let conflict = defining.compile("var crossConflict").unwrap();
    assert!(matches!(
        syntax_caller.execute(&conflict),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = syntax_caller.take_exception().unwrap().unwrap() else {
        panic!("cross-realm var conflict did not throw an Error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(syntax_prototype)
    );

    let mut type_caller = runtime.new_context();
    let Value::Object(type_prototype) = type_caller.eval("TypeError.prototype").unwrap() else {
        panic!("caller TypeError.prototype was not an object");
    };
    let type_global = type_caller.global_object().unwrap();
    runtime.prevent_extensions(&type_global).unwrap();
    let missing = defining.compile("var missingCrossVar").unwrap();
    assert!(matches!(
        type_caller.execute(&missing),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = type_caller.take_exception().unwrap().unwrap() else {
        panic!("cross-realm non-extensible var did not throw an Error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(type_prototype)
    );
}

#[test]
fn program_var_function_cell_cycle_is_collectable_after_context_drop() {
    let runtime = Runtime::new();
    {
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval("var cycle=function(){return cycle};cycle()===cycle")
                .unwrap(),
            Value::Bool(true)
        );
        let counts = runtime.heap_counts();
        assert_eq!(counts.context_nodes, 1);
        assert!(counts.var_ref_nodes > 0);
        assert!(counts.function_bytecode_nodes > 0);
    }

    assert_eq!(runtime.heap_counts().context_nodes, 1);
    runtime.run_gc().unwrap();
    let counts = runtime.heap_counts();
    assert_eq!(counts.context_nodes, 0);
    assert_eq!(counts.object_nodes, 0);
    assert_eq!(counts.shape_nodes, 0);
    assert_eq!(counts.var_ref_nodes, 0);
    assert_eq!(counts.function_bytecode_nodes, 0);
    assert_eq!(counts.live, 0);
}

#[test]
fn program_function_declaration_cycle_is_collectable_after_context_drop() {
    let runtime = Runtime::new();
    {
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval("function declarationCycle(){return declarationCycle};declarationCycle()===declarationCycle")
                .unwrap(),
            Value::Bool(true)
        );
        let counts = runtime.heap_counts();
        assert_eq!(counts.context_nodes, 1);
        assert!(counts.var_ref_nodes > 0);
        assert!(counts.function_bytecode_nodes > 0);
    }

    assert_eq!(runtime.heap_counts().context_nodes, 1);
    runtime.run_gc().unwrap();
    let counts = runtime.heap_counts();
    assert_eq!(counts.context_nodes, 0);
    assert_eq!(counts.object_nodes, 0);
    assert_eq!(counts.shape_nodes, 0);
    assert_eq!(counts.var_ref_nodes, 0);
    assert_eq!(counts.function_bytecode_nodes, 0);
    assert_eq!(counts.live, 0);
}

#[test]
fn program_global_lexicals_persist_shadow_and_reject_redeclaration() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context
            .eval("let mutable=1,named=function(){};const fixed=3;mutable+'|'+named.name+'|'+fixed")
            .unwrap(),
        Value::String(JsString::from_static("1|named|3"))
    );
    assert_eq!(context.eval("mutable+=2").unwrap(), Value::Int(3));
    assert_eq!(
        context.eval("typeof globalThis.mutable").unwrap(),
        Value::String(JsString::from_static("undefined"))
    );
    assert_eq!(context.eval("delete mutable").unwrap(), Value::Bool(false));

    let global = context.global_object().unwrap();
    let lexical_environment = context.global_var_object().unwrap();
    for (binding_name, expected_value, writable) in [
        ("mutable", Value::Int(3), true),
        ("fixed", Value::Int(3), false),
    ] {
        let key = runtime.intern_property_key(binding_name).unwrap();
        assert!(context.get_own_property(&global, &key).unwrap().is_none());
        assert_eq!(
            context
                .get_own_property(&lexical_environment, &key)
                .unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: expected_value,
                writable,
                enumerable: true,
                configurable: true,
            })
        );
    }

    assert!(matches!(
        context.eval("fixed=4"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("global const write did not throw an Error object");
    };
    let name = runtime.intern_property_key("name").unwrap();
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.get_property(&error, &name).unwrap(),
        Value::String(JsString::from_static("TypeError"))
    );
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static("'fixed' is read-only"))
    );

    assert!(matches!(
        context.eval("let mutable=9"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("global lexical redeclaration did not throw an Error object");
    };
    assert_eq!(
        context.get_property(&error, &name).unwrap(),
        Value::String(JsString::from_static("SyntaxError"))
    );
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static("redeclaration of 'mutable'"))
    );

    assert_eq!(context.eval("shadowedGlobal=1").unwrap(), Value::Int(1));
    assert_eq!(
        context.eval("let shadowedGlobal=2;shadowedGlobal").unwrap(),
        Value::Int(2)
    );
    assert_eq!(
        context.eval("globalThis.shadowedGlobal").unwrap(),
        Value::Int(1)
    );
    assert_eq!(
        context.eval("delete globalThis.shadowedGlobal").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(context.eval("shadowedGlobal").unwrap(), Value::Int(2));

    let mut sealed = runtime.new_context();
    let sealed_global = sealed.global_object().unwrap();
    runtime.prevent_extensions(&sealed_global).unwrap();
    assert_eq!(
        sealed
            .eval("let sealedLexical=6;const sealedConst=7;sealedLexical+sealedConst")
            .unwrap(),
        Value::Int(13)
    );
}

#[test]
fn program_global_lexical_preflight_and_failed_initializers_match_quickjs() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let name = runtime.intern_property_key("name").unwrap();
    let message = runtime.intern_property_key("message").unwrap();

    let delayed = context
        .compile("let delayedGlobal=7;delayedGlobal")
        .unwrap();
    assert_eq!(
        context.eval("typeof delayedGlobal").unwrap(),
        Value::String(JsString::from_static("undefined"))
    );
    assert_eq!(context.execute(&delayed).unwrap(), Value::Int(7));
    assert!(matches!(
        context.execute(&delayed),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();

    let atomic = context
        .compile("let untouched=function(){return Infinity},NaN=1,Infinity=2")
        .unwrap();
    assert!(matches!(
        context.execute(&atomic),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("global declaration preflight did not throw an Error object");
    };
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static("redeclaration of 'NaN'"))
    );
    assert_eq!(
        context.eval("typeof untouched").unwrap(),
        Value::String(JsString::from_static("undefined"))
    );
    assert_eq!(
        context.eval("let untouched=4;untouched").unwrap(),
        Value::Int(4)
    );

    assert!(matches!(
        context.eval(
            "Function.saved=function(){return captured};let captured=(function(){throw 17})()"
        ),
        Err(RuntimeError::Exception)
    ));
    assert_eq!(context.take_exception().unwrap(), Some(Value::Int(17)));
    assert!(matches!(
        context.eval("Function.saved()"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("declaring-script capture did not preserve the global lexical TDZ");
    };
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static("captured is not initialized"))
    );
    assert!(matches!(
        context.eval("captured"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("later eval did not materialize a missing-global ReferenceError");
    };
    assert_eq!(
        context.get_property(&error, &name).unwrap(),
        Value::String(JsString::from_static("ReferenceError"))
    );
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static("'captured' is not defined"))
    );
    assert_eq!(
        context.eval("typeof captured").unwrap(),
        Value::String(JsString::from_static("undefined"))
    );
    assert_eq!(context.eval("delete captured").unwrap(), Value::Bool(false));
    assert!(matches!(
        context.eval("let captured=1"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();

    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let caller_syntax_prototype = caller.eval("SyntaxError.prototype").unwrap();
    let conflict = defining.compile("let NaN=1").unwrap();
    assert!(matches!(
        caller.execute(&conflict),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = caller.take_exception().unwrap().unwrap() else {
        panic!("cross-realm declaration conflict did not throw an Error object");
    };
    let Value::Object(caller_syntax_prototype) = caller_syntax_prototype else {
        panic!("SyntaxError.prototype was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(caller_syntax_prototype)
    );

    let cross_realm = defining
        .compile("let crossRealmBinding=41;crossRealmBinding+1")
        .unwrap();
    assert_eq!(caller.execute(&cross_realm).unwrap(), Value::Int(42));
    assert_eq!(
        defining.eval("typeof crossRealmBinding").unwrap(),
        Value::String(JsString::from_static("undefined"))
    );
    assert_eq!(caller.eval("crossRealmBinding").unwrap(), Value::Int(41));
}

#[test]
fn strip_debug_removes_lexical_tdz_names_but_not_readonly_atoms() {
    let runtime = Runtime::new();
    runtime.set_debug_info_mode(DebugInfoMode::StripDebug);
    let mut context = runtime.new_context();
    for source in [
        "(function(){return localName;let localName=1})()",
        "(function(){return function probe(){return capturedName};let capturedName=1})()()",
        "(function(){var read;outer:{read=function(){return blockName};break outer;let blockName=1;}return read();})()",
        "(function(){var read;switch(1){case 0:let switchName=1;case 1:read=function(){return switchName};break;}return read();})()",
    ] {
        assert_eq!(
            evaluate_error(&runtime, &mut context, source),
            (
                JsString::from_static("ReferenceError"),
                JsString::from_static("lexical variable is not initialized")
            )
        );
    }
    assert_eq!(
        evaluate_error(
            &runtime,
            &mut context,
            "(function(){var write;{const retainedName=1;write=function(){retainedName=2};}write();})()"
        ),
        (
            JsString::from_static("TypeError"),
            JsString::from_static("'retainedName' is read-only")
        )
    );
}

#[test]
fn compiles_precedence_directly_to_stack_bytecode() {
    assert_eq!(evaluate("1 + 2 * 3"), Value::Int(7));
    assert_eq!(evaluate("(1 + 2) * 3"), Value::Int(9));
}

#[test]
fn bitwise_operators_follow_quickjs_precedence_and_numeric_semantics() {
    assert_eq!(evaluate("~0"), Value::Int(-1));
    assert_eq!(evaluate("~4294967296"), Value::Int(-1));
    assert_eq!(evaluate("-1.9 & 3.7"), Value::Int(3));
    assert_eq!(evaluate("'7' ^ true"), Value::Int(6));
    assert_eq!(evaluate("1 | 2 ^ 3 & 4"), Value::Int(3));
    assert_eq!(evaluate("1 | 2 === 3"), Value::Int(1));
    assert_eq!(evaluate("null ?? 1 | 2"), Value::Int(3));
    assert_eq!(evaluate("0 || 1 | 2"), Value::Int(3));

    assert_eq!(evaluate("~0n"), Value::BigInt(JsBigInt::from(-1)));
    assert_eq!(evaluate("-1n ^ 255n"), Value::BigInt(JsBigInt::from(-256)));
    assert_eq!(
        evaluate("123456789012345678901234567890n & -1n"),
        Value::BigInt(JsBigInt::parse_js_string("123456789012345678901234567890").unwrap())
    );
}

#[test]
fn shift_operators_follow_quickjs_precedence_and_numeric_semantics() {
    assert_eq!(evaluate("1 << 3"), Value::Int(8));
    assert_eq!(evaluate("-8 >> 2"), Value::Int(-2));
    assert_eq!(evaluate("-1 >>> 0"), Value::Float(4_294_967_295.0));
    assert_eq!(evaluate("1 << 33"), Value::Int(2));
    assert_eq!(evaluate("1 << -1"), Value::Int(i32::MIN));
    assert_eq!(evaluate("4294967295 >> 0"), Value::Int(-1));
    assert_eq!(evaluate("1 + 2 << 3"), Value::Int(24));
    assert_eq!(evaluate("16 >> 1 + 1"), Value::Int(4));
    assert_eq!(evaluate("1 << 2 < 5"), Value::Bool(true));
    assert_eq!(evaluate("8 >> 1 & 3"), Value::Int(0));
    assert_eq!(evaluate("64 >> 2 >> 1"), Value::Int(8));
    assert_eq!(evaluate("1 ?? 2 << 3"), Value::Int(1));

    assert_eq!(
        evaluate("1n << 65n"),
        Value::BigInt(JsBigInt::parse_js_string("36893488147419103232").unwrap())
    );
    assert_eq!(evaluate("-8n >> 2n"), Value::BigInt(JsBigInt::from(-2)));
    assert_eq!(evaluate("8n << -1n"), Value::BigInt(JsBigInt::from(4)));
    assert_eq!(evaluate("8n >> -2n"), Value::BigInt(JsBigInt::from(32)));
}

#[test]
fn exponentiation_follows_quickjs_precedence_associativity_and_unary_rules() {
    assert_eq!(evaluate("2 ** 3 ** 2"), Value::Int(512));
    assert_eq!(evaluate("2 * 3 ** 2"), Value::Int(18));
    assert_eq!(evaluate("2 ** 3 * 4"), Value::Int(32));
    assert_eq!(evaluate("2 ** -2"), Value::Float(0.25));
    assert_eq!(evaluate("(-2) ** 2"), Value::Int(4));
    assert!(evaluate("(typeof 2) ** 2").as_number().unwrap().is_nan());

    assert_eq!(evaluate("0n ** 0n"), Value::BigInt(JsBigInt::one()));
    assert_eq!(evaluate("(-2n) ** 3n"), Value::BigInt(JsBigInt::from(-8)));
    assert_eq!(
        evaluate("2n ** 100n"),
        Value::BigInt(JsBigInt::parse_js_string("1267650600228229401496703205376").unwrap())
    );

    for source in [
        "-2 ** 2",
        "+2 ** 2",
        "!2 ** 2",
        "~2 ** 2",
        "typeof 2 ** 2",
        "void 2 ** 2",
        "delete Function ** 2",
        "2 ** -2 ** 3",
    ] {
        let error = compile_script(source).unwrap_err();
        assert_eq!(
            error.message(),
            "unparenthesized unary expression can't appear on the left-hand side of '**'",
            "source {source:?}"
        );
    }
}

#[test]
fn update_expressions_follow_quickjs_lvalue_and_power_shapes() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        context
            .eval("(function(){ var x = '01'; var old = x++; return old + '|' + x; })()")
            .unwrap(),
        Value::String(JsString::from_static("1|2"))
    );
    assert_eq!(
        context
            .eval("(function(){ var x = 2; return (++x ** 2) * 100 + (x++ ** 2) * 10 + x; })()")
            .unwrap(),
        Value::Int(994)
    );
    assert_eq!(
        context
            .eval("(function(){ var x = 4n; var old = x--; return old * 10n + --x; })()")
            .unwrap(),
        Value::BigInt(JsBigInt::from(42))
    );
    assert_eq!(
        context
            .eval("(function(){ Function.update = '4'; var old = Function.update++; return old + '|' + ++Function.update; })()")
            .unwrap(),
        Value::String(JsString::from_static("4|6"))
    );
    assert_eq!(
        context
            .eval("(function(){ Function['update'] = 5; var old = Function['update']--; return old * 10 + Function.update; })()")
            .unwrap(),
        Value::Int(54)
    );
    assert_eq!(
        context
            .eval("(function(){ var x = 1, y = 2; x\n++y; return x * 10 + y; })()")
            .unwrap(),
        Value::Int(13)
    );

    let prefix_argument = context
        .compile("(function(value){ return ++value; })")
        .unwrap();
    let prefix_argument = runtime
        .test_child_function_bytecode(&prefix_argument, 0)
        .unwrap();
    let prefix_code = runtime.test_function_code(&prefix_argument).unwrap();
    assert!(prefix_code.windows(3).any(|window| matches!(
        window,
        [
            Instruction::GetArg(0),
            Instruction::Inc,
            Instruction::SetArg(0)
        ]
    )));

    let postfix_argument = context
        .compile("(function(value){ return value++; })")
        .unwrap();
    let postfix_argument = runtime
        .test_child_function_bytecode(&postfix_argument, 0)
        .unwrap();
    let postfix_code = runtime.test_function_code(&postfix_argument).unwrap();
    assert!(postfix_code.windows(3).any(|window| matches!(
        window,
        [
            Instruction::GetArg(0),
            Instruction::PostInc,
            Instruction::PutArg(0)
        ]
    )));

    let fixed = context.compile("Function.update++").unwrap();
    let fixed_code = runtime.test_function_code(&fixed).unwrap();
    assert!(fixed_code.windows(4).any(|window| matches!(
        window,
        [
            Instruction::GetField2(_),
            Instruction::PostInc,
            Instruction::Perm3,
            Instruction::PutField(_)
        ]
    )));

    let computed = context.compile("--Function['update']").unwrap();
    let computed_code = runtime.test_function_code(&computed).unwrap();
    assert!(computed_code.windows(4).any(|window| matches!(
        window,
        [
            Instruction::GetArrayEl3,
            Instruction::Dec,
            Instruction::Insert3,
            Instruction::PutArrayEl
        ]
    )));

    for source in ["++1", "1++", "++(1 + 2)", "(1 + 2)--"] {
        let error = compile_script(source).unwrap_err();
        assert_eq!(error.message(), "invalid increment/decrement operand");
    }
    for source in ["'use strict'; ++eval", "'use strict'; arguments--"] {
        let error = compile_script(source).unwrap_err();
        assert_eq!(error.message(), "invalid lvalue in strict mode");
    }
}

#[test]
fn compiles_primitive_coercion_and_equality() {
    assert_eq!(
        evaluate("'answer: ' + 42"),
        Value::String(JsString::from_static("answer: 42"))
    );
    assert_eq!(evaluate("'42' == 42"), Value::Bool(true));
    assert_eq!(evaluate("'42' === 42"), Value::Bool(false));
}

#[test]
fn compiles_short_circuit_and_conditional_control_flow() {
    assert_eq!(evaluate("false && 42"), Value::Bool(false));
    assert_eq!(
        evaluate("'left' || 'right'"),
        Value::String(JsString::from_static("left"))
    );
    assert_eq!(evaluate("false ? 1 : 2"), Value::Int(2));
    assert!(compile_script("true ? 1, 2 : 3").is_err());
    assert_eq!(evaluate("true ? 1 : 2, 3"), Value::Int(3));
}

#[test]
fn nullish_coalescing_uses_one_quickjs_short_circuit_join() {
    assert_eq!(evaluate("null ?? 42"), Value::Int(42));
    assert_eq!(evaluate("void 0 ?? 7"), Value::Int(7));
    assert_eq!(evaluate("false ?? true"), Value::Bool(false));
    assert_eq!(evaluate("-0 ?? 1"), Value::Float(-0.0));
    assert_eq!(
        evaluate("'' ?? 'fallback'"),
        Value::String(JsString::from_static(""))
    );
    assert_eq!(evaluate("null ?? void 0 ?? 9"), Value::Int(9));
    assert_eq!(evaluate("null ?? 1 + 2 * 3"), Value::Int(7));
    assert_eq!(evaluate("0 ?? 1 ? 2 : 3"), Value::Int(3));

    let chain = compile_script("null ?? void 0 ?? 9").unwrap();
    let targets = chain
        .code
        .iter()
        .filter_map(|instruction| match instruction {
            Instruction::IfFalse(target) => Some(*target),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(targets.len(), 2);
    assert_eq!(targets[0], targets[1]);
    let join = usize::try_from(targets[0]).unwrap();
    assert!(matches!(chain.code[join], Instruction::PutLocal(0)));
    assert!(matches!(chain.code[join + 1], Instruction::GetLocal(0)));
    assert!(matches!(chain.code[join + 2], Instruction::Return));

    for source in ["1 || 2 ?? 3", "1 && 2 ?? 3", "1 ?? 2 || 3", "1 ?? 2 && 3"] {
        assert!(compile_script(source).is_err(), "accepted {source:?}");
    }
    assert_eq!(evaluate("(false || 4) ?? 5"), Value::Int(4));
    assert_eq!(evaluate("null ?? (false || 6)"), Value::Int(6));
    assert_eq!(evaluate("(null ?? 0) || 7"), Value::Int(7));
    assert_eq!(evaluate("false || (null ?? 8)"), Value::Int(8));

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let call = context
        .compile(
            "Function.coalesce = function(){ return this === Function; }; \
             (Function.coalesce ?? Function)()",
        )
        .unwrap();
    let call_code = runtime.test_function_code(&call).unwrap();
    assert!(
        call_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Call(0)))
    );
    assert!(
        !call_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::CallMethod(_)))
    );
    assert_eq!(
        context
            .eval(
                "Function.coalesce = function(){ return this === Function; }; \
                 (Function.coalesce ?? Function)()"
            )
            .unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        context
            .eval("inferred = null ?? function(){}; inferred.name")
            .unwrap(),
        Value::String(JsString::from_static(""))
    );
    assert_eq!(
        context.eval("1 ?? missingNullishRhs").unwrap(),
        Value::Int(1)
    );
    assert_eq!(
        context
            .eval("Function.combo = 0; Function.combo ||= null ?? 4")
            .unwrap(),
        Value::Int(4)
    );
    assert_eq!(
        context
            .eval("Function.combo = null; Function.combo ??= void 0 ?? 5")
            .unwrap(),
        Value::Int(5)
    );
    assert!(
        context
            .compile("(Function.left ?? Function.right) = 1")
            .is_err()
    );
}

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

#[test]
fn relational_membership_uses_runtime_object_protocols() {
    assert_eq!(
        evaluate_in_context("'prototype' in Function"),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_in_context("'missingMembershipKey' in Function"),
        Value::Bool(false)
    );
    assert_eq!(
        evaluate_in_context("'toString' in Function"),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_in_context("Function instanceof Function"),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_in_context("(function(){}) instanceof Function"),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_in_context("1 instanceof Function"),
        Value::Bool(false)
    );
    assert_eq!(
        evaluate_in_context("(function(){}).bind(null) instanceof Function"),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){ var target=function DeepTarget(){}; var bound=target; for(var i=0;i<512;i++) bound=bound.bind(null); return 1 instanceof bound; })()"
        ),
        Value::Bool(false)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){ var result=false; for((result='prototype' in Function);false;); return result; })()"
        ),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){ var result=false; for(result=Function instanceof Function;false;); return result; })()"
        ),
        Value::Bool(true)
    );

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context
        .eval(
            "Function.membershipTrace=''; Function[Symbol.toPrimitive]=function(hint){ Function.membershipTrace+=hint; return 'prototype'; };",
        )
        .unwrap();
    assert!(matches!(
        context.eval("Function in (Function.membershipTrace+='R',1)"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();
    assert_eq!(
        context.eval("Function.membershipTrace").unwrap(),
        Value::String(JsString::from_static("R"))
    );
    assert_eq!(
        context.eval("Function in Function").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        context.eval("Function.membershipTrace").unwrap(),
        Value::String(JsString::from_static("Rstring"))
    );
}

#[test]
fn untagged_templates_follow_quickjs_concat_lowering() {
    assert_eq!(
        evaluate("`plain`"),
        Value::String(JsString::from_static("plain"))
    );
    assert_eq!(
        evaluate_in_context("`a${1 + 2}b${4}c`"),
        Value::String(JsString::from_static("a3b4c"))
    );
    assert_eq!(
        evaluate_in_context("`a${1, 2}b`"),
        Value::String(JsString::from_static("a2b"))
    );
    assert_eq!(
        evaluate_in_context("`a${`b${1}c`}d`"),
        Value::String(JsString::from_static("ab1cd"))
    );
    assert_eq!(evaluate_in_context("`x${8 / 2}y`.length"), Value::Int(3));

    let no_substitution = compile_script("`plain`").unwrap();
    assert!(!no_substitution.code.iter().any(|instruction| matches!(
        instruction,
        Instruction::GetField2(_) | Instruction::CallMethod(_)
    )));

    let interpolated = compile_unlinked_script("`a${1}b${2}c`").unwrap();
    assert!(
        interpolated
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetField2(_)))
    );
    assert!(
        interpolated
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::CallMethod(4)))
    );

    let invalid = compile_script("`\\8`").unwrap_err();
    assert_eq!(
        invalid.message(),
        "malformed escape sequence in string literal"
    );
    assert_eq!(
        compile_script("0`x`").unwrap_err().message(),
        "tagged template objects require runtime publication; use Context::compile or Context::eval"
    );
    assert_eq!(
        compile_script("0\n`x`").unwrap_err().message(),
        "tagged template objects require runtime publication; use Context::compile or Context::eval"
    );

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context
        .compile("(function tag(strings) { return strings[0]; })`x`")
        .expect("runtime publication should materialize tagged-template objects");
}

#[test]
fn detached_vm_rejects_runtime_global_execution_explicitly() {
    let error = compile_script("answer").unwrap_err();
    assert!(error.message().contains("global-environment"));

    let error = compile_script("delete answer").unwrap_err();
    assert!(error.message().contains("global-environment"));
}

#[test]
fn runtime_global_get_and_direct_typeof_use_the_bytecode_realm() {
    let runtime = Runtime::new();
    let mut defining_context = runtime.new_context();
    let mut caller_context = runtime.new_context();
    let answer = runtime.intern_property_key("answer").unwrap();
    let marker = runtime.intern_property_key("marker").unwrap();
    let descriptor = |value| OrdinaryPropertyDescriptor {
        value: DescriptorField::Present(value),
        writable: DescriptorField::Present(true),
        enumerable: DescriptorField::Present(true),
        configurable: DescriptorField::Present(true),
        ..OrdinaryPropertyDescriptor::new()
    };
    assert!(
        defining_context
            .define_own_property(
                &defining_context.global_object().unwrap(),
                &answer,
                &descriptor(Value::Int(1)),
            )
            .unwrap()
    );
    defining_context
        .create_global_lexical_for_test("answer", false, Some(Value::Int(2)))
        .unwrap();
    assert_eq!(defining_context.eval("answer").unwrap(), Value::Int(2));
    assert_eq!(
        defining_context.eval("typeof answer").unwrap(),
        Value::String(JsString::from_static("number"))
    );
    assert_eq!(
        defining_context.eval("typeof missingGlobal").unwrap(),
        Value::String(JsString::from_static("undefined"))
    );
    assert_eq!(
        defining_context.eval("typeof ((missingGlobal))").unwrap(),
        Value::String(JsString::from_static("undefined"))
    );
    assert!(matches!(
        defining_context.eval("typeof (0, missingGlobal)"),
        Err(RuntimeError::Exception)
    ));
    assert!(matches!(
        defining_context.take_exception().unwrap(),
        Some(Value::Object(_))
    ));

    let marker_object = defining_context.new_object().unwrap();
    assert!(
        defining_context
            .define_own_property(
                &defining_context.global_object().unwrap(),
                &marker,
                &descriptor(Value::Object(marker_object.clone())),
            )
            .unwrap()
    );
    let Value::Object(function) = defining_context
        .eval("(0, function(){ return marker; })")
        .unwrap()
    else {
        panic!("global-realm probe did not produce a function");
    };
    let callable = runtime.as_callable(&function).unwrap().unwrap();
    assert_eq!(
        caller_context
            .call(&callable, Value::Undefined, &[])
            .unwrap(),
        Value::Object(marker_object)
    );

    assert!(matches!(
        caller_context.eval("missingGlobal"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = caller_context.take_exception().unwrap().unwrap() else {
        panic!("missing global did not materialize a ReferenceError");
    };
    let message = runtime.intern_property_key("message").unwrap();
    assert!(matches!(
        caller_context.get_property(&exception, &message).unwrap(),
        Value::String(value) if value == JsString::from_static("'missingGlobal' is not defined")
    ));
}

#[test]
fn global_put_matches_strict_sloppy_readonly_and_setter_semantics() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let descriptor = |value, writable| OrdinaryPropertyDescriptor {
        value: DescriptorField::Present(value),
        writable: DescriptorField::Present(writable),
        enumerable: DescriptorField::Present(true),
        configurable: DescriptorField::Present(true),
        ..OrdinaryPropertyDescriptor::new()
    };

    assert_eq!(context.eval("created = 7").unwrap(), Value::Int(7));
    assert_eq!(context.eval("created").unwrap(), Value::Int(7));

    let readonly = runtime.intern_property_key("readonly").unwrap();
    assert!(
        context
            .define_own_property(&global, &readonly, &descriptor(Value::Int(1), false))
            .unwrap()
    );
    assert_eq!(context.eval("readonly = 2").unwrap(), Value::Int(2));
    assert_eq!(context.eval("readonly").unwrap(), Value::Int(1));
    assert!(matches!(
        context.eval("'use strict'; readonly = 2"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
        panic!("strict read-only global assignment did not throw an object");
    };
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.get_property(&exception, &message).unwrap(),
        Value::String(JsString::from_static("'readonly' is read-only"))
    );
    assert_eq!(context.eval("readonly += 2").unwrap(), Value::Int(3));
    assert_eq!(context.eval("readonly").unwrap(), Value::Int(1));
    assert_eq!(
        context.eval("'use strict'; readonly ||= 9").unwrap(),
        Value::Int(1)
    );
    assert!(matches!(
        context.eval("'use strict'; readonly += 2"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();
    assert!(matches!(
        context.eval("'use strict'; readonly &&= 2"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();

    let inherited = runtime.intern_property_key("inheritedReadOnly").unwrap();
    assert!(
        context
            .define_own_property(
                &context.object_prototype().unwrap(),
                &inherited,
                &descriptor(Value::Int(5), false),
            )
            .unwrap()
    );
    assert_eq!(
        context.eval("inheritedReadOnly = 6").unwrap(),
        Value::Int(6)
    );
    assert_eq!(context.eval("inheritedReadOnly").unwrap(), Value::Int(5));
    assert!(matches!(
        context.eval("'use strict'; inheritedReadOnly = 6"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
        panic!("strict inherited read-only assignment did not throw an object");
    };
    assert_eq!(
        context.get_property(&exception, &message).unwrap(),
        Value::String(JsString::from_static("'inheritedReadOnly' is read-only"))
    );

    let no_setter = runtime.intern_property_key("noSetter").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &no_setter,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Undefined),
                    set: DescriptorField::Present(AccessorValue::Undefined),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(context.eval("noSetter = 8").unwrap(), Value::Int(8));
    assert!(matches!(
        context.eval("'use strict'; noSetter = 8"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
        panic!("strict setter-less assignment did not throw an object");
    };
    assert_eq!(
        context.get_property(&exception, &message).unwrap(),
        Value::String(JsString::from_static("no setter for property"))
    );
    assert_eq!(context.eval("noSetter ||= 8").unwrap(), Value::Int(8));
    assert!(matches!(
        context.eval("'use strict'; noSetter ||= 8"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();
    assert_eq!(
        context.eval("'use strict'; noSetter &&= 8").unwrap(),
        Value::Undefined
    );

    assert!(matches!(
        context.eval("'use strict'; trulyMissing = 1"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();
    let truly_missing = runtime.intern_property_key("trulyMissing").unwrap();
    assert!(!runtime.has_own_property(&global, &truly_missing).unwrap());

    let sink = runtime.intern_property_key("sink").unwrap();
    assert!(
        context
            .define_own_property(&global, &sink, &descriptor(Value::Int(0), true))
            .unwrap()
    );
    let Value::Object(setter) = context
        .eval("(function(v) { sink = v; return 99; })")
        .unwrap()
    else {
        panic!("setter source did not produce a function");
    };
    let setter = runtime.as_callable(&setter).unwrap().unwrap();
    let target = runtime.intern_property_key("setterTarget").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &target,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Undefined),
                    set: DescriptorField::Present(AccessorValue::Callable(setter)),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(context.eval("setterTarget = 42").unwrap(), Value::Int(42));
    assert_eq!(context.eval("sink").unwrap(), Value::Int(42));
    assert_eq!(context.eval("setterTarget ||= 17").unwrap(), Value::Int(17));
    assert_eq!(context.eval("sink").unwrap(), Value::Int(17));

    let Value::Object(getter) = context.eval("(function() { return this; })").unwrap() else {
        panic!("getter source did not produce a function");
    };
    let getter = runtime.as_callable(&getter).unwrap().unwrap();
    let getter_target = runtime.intern_property_key("getterTarget").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &getter_target,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(getter)),
                    set: DescriptorField::Present(AccessorValue::Undefined),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(
        context.eval("getterTarget").unwrap(),
        Value::Object(global.clone())
    );
    assert_eq!(
        context.eval("typeof getterTarget").unwrap(),
        Value::String(JsString::from_static("object"))
    );

    let Value::Object(throwing_getter) = context.eval("(function() { throw 17; })").unwrap() else {
        panic!("throwing getter source did not produce a function");
    };
    let throwing_getter = runtime.as_callable(&throwing_getter).unwrap().unwrap();
    let throwing_target = runtime.intern_property_key("throwingGetter").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &throwing_target,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(throwing_getter)),
                    set: DescriptorField::Present(AccessorValue::Undefined),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert!(matches!(
        context.eval("typeof throwingGetter"),
        Err(RuntimeError::Exception)
    ));
    assert_eq!(context.take_exception().unwrap(), Some(Value::Int(17)));

    assert_eq!(context.eval("compoundSide = 0").unwrap(), Value::Int(0));
    assert!(matches!(
        context.eval("missingCompound += (compoundSide = 1)"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();
    assert_eq!(context.eval("compoundSide").unwrap(), Value::Int(0));
    assert!(matches!(
        context.eval("missingLogical ||= (compoundSide = 2)"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();
    assert_eq!(context.eval("compoundSide").unwrap(), Value::Int(0));
}

#[test]
fn global_lexical_tdz_const_shadow_and_initialization_share_the_resolved_cell() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let shadowed = runtime.intern_property_key("shadowed").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &shadowed,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Int(1)),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let Value::Object(reader) = context.eval("(function() { return shadowed; })").unwrap() else {
        panic!("lexical reader source did not produce a function");
    };
    let reader = runtime.as_callable(&reader).unwrap().unwrap();

    context
        .create_global_lexical_for_test("shadowed", true, None)
        .unwrap();
    assert_eq!(context.eval("delete shadowed").unwrap(), Value::Bool(false));
    assert_eq!(
        context.get_property(&global, &shadowed).unwrap(),
        Value::Int(1)
    );
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.call(&reader, Value::Undefined, &[]).unwrap(),
        Value::Int(1),
        "a precompiled ordinary global descriptor falls back to the global object while the later lexical is uninitialized"
    );

    context
        .initialize_global_lexical_for_test("shadowed", Value::Int(2))
        .unwrap();
    assert_eq!(
        context.call(&reader, Value::Undefined, &[]).unwrap(),
        Value::Int(2)
    );
    let Value::Object(writer) = context.eval("(function() { shadowed = 3; })").unwrap() else {
        panic!("lexical writer source did not produce a function");
    };
    let writer = runtime.as_callable(&writer).unwrap().unwrap();
    assert!(matches!(
        context.call(&writer, Value::Undefined, &[]),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
        panic!("const assignment did not throw an object");
    };
    assert_eq!(
        context.get_property(&exception, &message).unwrap(),
        Value::String(JsString::from_static("'shadowed' is read-only"))
    );
    assert_eq!(
        context.get_property(&global, &shadowed).unwrap(),
        Value::Int(1)
    );
    assert_eq!(context.eval("shadowed ||= 9").unwrap(), Value::Int(2));
    assert!(matches!(
        context.eval("shadowed += 3"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
        panic!("const compound assignment did not throw an object");
    };
    assert_eq!(
        context.get_property(&exception, &message).unwrap(),
        Value::String(JsString::from_static("'shadowed' is read-only"))
    );
    assert!(matches!(
        context.eval("shadowed &= 3"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
        panic!("const bitwise compound assignment did not throw an object");
    };
    assert_eq!(
        context.get_property(&exception, &message).unwrap(),
        Value::String(JsString::from_static("'shadowed' is read-only"))
    );
    assert!(matches!(
        context.eval("shadowed <<= 1"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
        panic!("const shift compound assignment did not throw an object");
    };
    assert_eq!(
        context.get_property(&exception, &message).unwrap(),
        Value::String(JsString::from_static("'shadowed' is read-only"))
    );
    assert!(matches!(
        context.eval("shadowed **= 3"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
        panic!("const exponent compound assignment did not throw an object");
    };
    assert_eq!(
        context.get_property(&exception, &message).unwrap(),
        Value::String(JsString::from_static("'shadowed' is read-only"))
    );
    assert!(matches!(
        context.eval("shadowed &&= 3"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();

    context
        .create_global_lexical_for_test("mutableLexical", false, None)
        .unwrap();
    assert_eq!(
        context.eval("typeof mutableLexical").unwrap(),
        Value::String(JsString::from_static("undefined"))
    );
    assert!(matches!(
        context.eval("mutableLexical += 1"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();
    assert!(matches!(
        context.eval("mutableLexical |= 1"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();
    assert!(matches!(
        context.eval("mutableLexical **= 2"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();
    context
        .initialize_global_lexical_for_test("mutableLexical", Value::Int(4))
        .unwrap();
    assert_eq!(context.eval("mutableLexical |= 8").unwrap(), Value::Int(12));
    assert_eq!(context.eval("mutableLexical ^= 3").unwrap(), Value::Int(15));
    assert_eq!(context.eval("mutableLexical &= 7").unwrap(), Value::Int(7));
    assert_eq!(context.eval("mutableLexical += 3").unwrap(), Value::Int(10));
    assert_eq!(context.eval("mutableLexical &&= 5").unwrap(), Value::Int(5));
    assert_eq!(context.eval("mutableLexical ??= 9").unwrap(), Value::Int(5));
    assert_eq!(
        context.eval("mutableLexical **= 2").unwrap(),
        Value::Int(25)
    );
    assert_eq!(context.eval("mutableLexical").unwrap(), Value::Int(25));

    context
        .create_global_lexical_for_test("mutableShift", false, None)
        .unwrap();
    assert!(matches!(
        context.eval("mutableShift >>>= 1"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();
    context
        .initialize_global_lexical_for_test("mutableShift", Value::Int(-8))
        .unwrap();
    assert_eq!(context.eval("mutableShift >>= 1").unwrap(), Value::Int(-4));
    assert_eq!(
        context.eval("mutableShift >>>= 1").unwrap(),
        Value::Int(2_147_483_646)
    );
    assert_eq!(context.eval("mutableShift <<= 1").unwrap(), Value::Int(-4));
}

#[test]
fn unresolved_name_compiles_to_one_global_then_parent_global_relays() {
    let script = compile_unlinked_script(
        "(function() { return function() { return function() { return relayName; }; }; })",
    )
    .unwrap();
    let mut function = &script;
    for depth in 0..4 {
        let descriptor = function
            .closure_variables()
            .first()
            .expect("every function on the unresolved-name path needs a closure slot");
        assert_eq!(
            descriptor.source,
            if depth == 0 {
                ClosureSource::Global
            } else {
                ClosureSource::ParentGlobal(0)
            }
        );
        let ClosureVariableName::Constant(name_index) = descriptor.name else {
            panic!("unlinked global relay did not retain a name constant");
        };
        assert!(matches!(
            function.constants()[name_index as usize].as_primitive(),
            Some(Value::String(name)) if name == &JsString::from_static("relayName")
        ));
        if depth == 3 {
            assert!(matches!(
                function.code(),
                [Instruction::GetVar(0), Instruction::Return, ..]
            ));
            break;
        }
        function = function
            .constants()
            .iter()
            .find_map(|constant| constant.as_child())
            .expect("global relay path lost its nested child");
    }
}

#[test]
fn late_global_property_delete_reconnect_and_cross_realm_use_the_defining_realm() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let Value::Object(reader) = defining
        .eval("(function() { return lateRealmValue; })")
        .unwrap()
    else {
        panic!("late global reader source did not produce a function");
    };
    let reader = runtime.as_callable(&reader).unwrap().unwrap();
    let key = runtime.intern_property_key("lateRealmValue").unwrap();
    let descriptor = |value| OrdinaryPropertyDescriptor {
        value: DescriptorField::Present(Value::Int(value)),
        writable: DescriptorField::Present(true),
        enumerable: DescriptorField::Present(true),
        configurable: DescriptorField::Present(true),
        ..OrdinaryPropertyDescriptor::new()
    };
    assert!(
        caller
            .define_own_property(&caller.global_object().unwrap(), &key, &descriptor(9),)
            .unwrap()
    );
    let defining_global = defining.global_object().unwrap();
    assert!(
        defining
            .define_own_property(&defining_global, &key, &descriptor(1))
            .unwrap()
    );
    runtime.run_gc().unwrap();
    assert_eq!(
        caller.call(&reader, Value::Undefined, &[]).unwrap(),
        Value::Int(1)
    );

    assert!(runtime.delete_property(&defining_global, &key).unwrap());
    runtime.run_gc().unwrap();
    assert!(matches!(
        caller.call(&reader, Value::Undefined, &[]),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = caller.take_exception().unwrap().unwrap() else {
        panic!("missing defining-realm global did not throw an object");
    };
    let reference_error = runtime.intern_property_key("ReferenceError").unwrap();
    let prototype = runtime.intern_property_key("prototype").unwrap();
    let Value::Object(reference_error) = defining
        .get_property(&defining_global, &reference_error)
        .unwrap()
    else {
        panic!("defining realm ReferenceError was not an object");
    };
    let Value::Object(reference_error_prototype) =
        defining.get_property(&reference_error, &prototype).unwrap()
    else {
        panic!("defining realm ReferenceError.prototype was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&exception).unwrap(),
        Some(reference_error_prototype)
    );
    assert!(
        defining
            .define_own_property(&defining_global, &key, &descriptor(2))
            .unwrap()
    );
    runtime.run_gc().unwrap();
    assert_eq!(
        caller.call(&reader, Value::Undefined, &[]).unwrap(),
        Value::Int(2)
    );
}

#[test]
fn runtime_compiler_executes_anonymous_iife_parameters_and_direct_call() {
    let source = "(function(a, b) { return a + b; })(20, 22)";
    assert_eq!(evaluate_in_context(source), Value::Int(42));

    let detached_error = compile_script(source).unwrap_err();
    assert!(
        detached_error
            .message()
            .contains("requires runtime publication")
    );

    let script = compile_unlinked_script("(function(a, b) {})").unwrap();
    let function = script.constants()[0].as_child().unwrap();
    assert_eq!(function.metadata().argument_count, 2);
    assert_eq!(function.metadata().defined_argument_count, 2);
    assert!(function.metadata().has_prototype);
    assert_eq!(function.metadata().constructor_kind, ConstructorKind::Base);

    let runtime = Runtime::new();
    let Value::Object(function) = runtime.new_context().eval("(function() {})").unwrap() else {
        panic!("function expression did not produce an object");
    };
    assert!(runtime.is_constructor(&function).unwrap());
}

#[test]
fn compiler_marks_only_syntactic_eval_identifier_calls() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    for source in ["eval(0)", "(eval)(0)", "((eval))(0)", r"\u0065val(0)"] {
        let root = context.compile(source).unwrap();
        let code = runtime.test_function_code(&root).unwrap();
        assert_eq!(
            code.iter()
                .filter(|instruction| {
                    matches!(
                        instruction,
                        Instruction::Eval {
                            argument_count: 1,
                            ..
                        }
                    )
                })
                .count(),
            1,
            "direct source: {source}"
        );
    }

    let local = context
        .compile("(function(eval){ return eval(0); })")
        .unwrap();
    let local = runtime.test_child_function_bytecode(&local, 0).unwrap();
    assert!(
        runtime
            .test_function_code(&local)
            .unwrap()
            .iter()
            .any(|instruction| {
                matches!(
                    instruction,
                    Instruction::Eval {
                        argument_count: 1,
                        ..
                    }
                )
            })
    );

    for source in [
        "(0, eval)(0)",
        "var alias = eval; alias(0)",
        "(true ? eval : eval)(0)",
        "(eval = Function)(0)",
        "globalThis.eval(0)",
        "eval.call(undefined, 0)",
        "new eval(0)",
    ] {
        let root = context.compile(source).unwrap();
        let code = runtime.test_function_code(&root).unwrap();
        assert!(
            !code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::Eval { .. })),
            "indirect/non-call source: {source}"
        );
    }
}

#[test]
fn compiler_emits_independent_eval_root_and_external_relays() {
    let bindings = vec![
        EvalRootBinding {
            name: JsString::from_static("inner"),
            scope: 0,
            is_lexical: true,
            is_const: false,
            kind: ClosureVariableKind::Normal,
            is_catch_parameter: false,
        },
        EvalRootBinding {
            name: JsString::from_static("outer"),
            scope: 1,
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
            is_catch_parameter: false,
        },
    ];

    for debug_info in [
        DebugInfoMode::Full,
        DebugInfoMode::StripSource,
        DebugInfoMode::StripDebug,
    ] {
        let root = compile_unlinked_eval_with_filename(
            "(function () { return inner + outer; })",
            "<eval>",
            debug_info,
            EvalCompileContext::direct(true, bindings.clone()),
        )
        .unwrap();
        assert_eq!(root.metadata().eval_kind, EvalKind::Direct);
        assert!(root.metadata().strict);
        assert!(!root.metadata().has_prototype);
        assert_eq!(root.metadata().constructor_kind, ConstructorKind::None);
        assert_eq!(root.closure_variables().len(), bindings.len());
        for (index, descriptor) in root.closure_variables().iter().enumerate() {
            assert_eq!(
                descriptor.source,
                ClosureSource::EvalEnvironment(u16::try_from(index).unwrap()),
            );
            let ClosureVariableName::Constant(name) = descriptor.name else {
                panic!("eval root lost its external binding name in {debug_info:?}");
            };
            assert!(matches!(
                root.constants()[name as usize].as_primitive(),
                Some(Value::String(found)) if found == &bindings[index].name
            ));
        }

        let child = root
            .constants()
            .iter()
            .find_map(|constant| constant.as_child())
            .expect("eval function expression did not produce child bytecode");
        assert_eq!(child.metadata().eval_kind, EvalKind::None);
        assert_eq!(child.closure_variables().len(), 2);
        assert_eq!(
            child.closure_variables()[0].source,
            ClosureSource::ParentClosure(0),
        );
        assert_eq!(
            child.closure_variables()[1].source,
            ClosureSource::ParentClosure(1),
        );
        assert!(child.closure_variables()[0].is_lexical);
        assert!(matches!(
            child.closure_variables()[0].name,
            ClosureVariableName::Constant(_)
        ));
    }
}

#[test]
fn direct_eval_super_capability_is_explicit_and_independent_from_imports() {
    let pseudo = |name: &'static str, scope| EvalRootBinding {
        name: JsString::from_static(name),
        scope,
        is_lexical: false,
        is_const: false,
        kind: ClosureVariableKind::Normal,
        is_catch_parameter: false,
    };
    let direct = |bindings: Vec<EvalRootBinding<JsString>>,
                  scope_kinds: Vec<EvalScopeKind>,
                  super_call_allowed,
                  super_allowed| {
        EvalCompileContext::direct_with_profile(
            true,
            bindings,
            EvalCallerProfile {
                scope_kinds: scope_kinds.into_boxed_slice(),
                variable_target: EvalCallerVariableTarget::StrictLocal,
            },
            super_call_allowed,
            super_allowed,
        )
    };
    let method_bindings = || {
        vec![
            pseudo(THIS_LOCAL_NAME, 0),
            pseudo(HOME_OBJECT_LOCAL_NAME, 0),
        ]
    };

    let denied_despite_bindings = compile_unlinked_eval_with_filename(
        "super.value",
        "<eval>",
        DebugInfoMode::StripDebug,
        direct(
            method_bindings(),
            vec![EvalScopeKind::FunctionRoot],
            false,
            false,
        ),
    )
    .expect_err("hidden bindings must not grant parser authority");
    assert_eq!(denied_despite_bindings.kind(), ErrorKind::Syntax);
    assert_eq!(
        denied_despite_bindings.message(),
        "'super' is only valid in a method"
    );

    let allowed = compile_unlinked_eval_with_filename(
        "eval('super.value'); (() => super.value)",
        "<eval>",
        DebugInfoMode::StripDebug,
        direct(
            method_bindings(),
            vec![EvalScopeKind::FunctionRoot],
            false,
            true,
        ),
    )
    .expect("a method-owned direct eval retains SuperProperty capability");
    assert_eq!(allowed.metadata().eval_kind, EvalKind::Direct);
    assert!(allowed.metadata().super_allowed);
    assert!(!allowed.metadata().super_call_allowed);
    assert!(!allowed.metadata().needs_home_object);
    assert!(
        allowed
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Eval { .. }))
    );
    let arrow = allowed
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("direct eval lost its arrow child");
    assert!(
        arrow
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetSuperValue))
    );
    let nested_environment = &allowed.eval_environments()[0];
    assert!(nested_environment.super_allowed);
    assert!(!nested_environment.super_call_allowed);
    let imported_owner = nested_environment
        .scopes
        .iter()
        .rev()
        .find(|scope| scope.kind == EvalScopeKind::FunctionRoot)
        .expect("nested eval lost its imported method owner");
    for expected in [THIS_LOCAL_NAME, HOME_OBJECT_LOCAL_NAME] {
        assert!(
            imported_owner
                .bindings
                .iter()
                .any(|binding| binding.name == JsString::from_static(expected)),
            "nested eval lost {expected}",
        );
    }

    let ordinary_boundary = compile_unlinked_eval_with_filename(
        "super.value",
        "<eval>",
        DebugInfoMode::StripDebug,
        direct(
            vec![
                pseudo(THIS_LOCAL_NAME, 0),
                pseudo(THIS_LOCAL_NAME, 1),
                pseudo(HOME_OBJECT_LOCAL_NAME, 1),
            ],
            vec![EvalScopeKind::FunctionRoot, EvalScopeKind::FunctionRoot],
            false,
            false,
        ),
    )
    .expect_err("an ordinary caller must truncate an outer method capability");
    assert_eq!(ordinary_boundary.kind(), ErrorKind::Syntax);
    assert_eq!(
        ordinary_boundary.message(),
        "'super' is only valid in a method"
    );

    let global = compile_unlinked_eval_with_filename(
        "super.value",
        "<eval>",
        DebugInfoMode::StripDebug,
        direct(
            vec![pseudo(THIS_LOCAL_NAME, 0)],
            vec![EvalScopeKind::FunctionRoot],
            false,
            false,
        ),
    )
    .expect_err("a direct eval without a method owner must reject SuperProperty");
    assert_eq!(global.kind(), ErrorKind::Syntax);
    assert_eq!(global.message(), "'super' is only valid in a method");

    let indirect = compile_unlinked_eval_with_filename(
        "super.value",
        "<eval>",
        DebugInfoMode::StripDebug,
        EvalCompileContext::indirect(),
    )
    .expect_err("indirect eval must reject SuperProperty");
    assert_eq!(indirect.kind(), ErrorKind::Syntax);
    assert_eq!(indirect.message(), "'super' is only valid in a method");

    let super_call = compile_unlinked_eval_with_filename(
        "super()",
        "<eval>",
        DebugInfoMode::StripDebug,
        direct(
            method_bindings(),
            vec![EvalScopeKind::FunctionRoot],
            false,
            true,
        ),
    )
    .expect_err("SuperProperty capability must not enable SuperCall");
    assert_eq!(super_call.kind(), ErrorKind::Syntax);
    assert_eq!(
        super_call.message(),
        "super() is only valid in a derived class constructor"
    );

    let enabled_super_call = compile_unlinked_eval_with_filename(
        "super()",
        "<eval>",
        DebugInfoMode::StripDebug,
        direct(
            method_bindings(),
            vec![EvalScopeKind::FunctionRoot],
            true,
            true,
        ),
    )
    .expect_err("the typed capability reaches the unimplemented SuperCall frontier");
    assert_eq!(enabled_super_call.kind(), ErrorKind::Unsupported);
    assert_eq!(
        enabled_super_call.message(),
        "derived constructor super() is not implemented yet"
    );
}

#[test]
fn compiler_preserves_authenticated_with_environment_relays() {
    let with_object = EvalRootBinding {
        name: JsString::from_static(WITH_OBJECT_LOCAL_NAME),
        scope: 0,
        is_lexical: false,
        is_const: false,
        kind: ClosureVariableKind::WithObject,
        is_catch_parameter: false,
    };
    let root = compile_unlinked_eval_with_filename(
        "(function relay() { eval('value'); })",
        "<eval>",
        DebugInfoMode::StripDebug,
        EvalCompileContext::direct_with_profile(
            false,
            vec![with_object.clone()],
            EvalCallerProfile {
                scope_kinds: vec![
                    EvalScopeKind::With,
                    EvalScopeKind::ProgramBody,
                    EvalScopeKind::FunctionRoot,
                ]
                .into_boxed_slice(),
                variable_target: EvalCallerVariableTarget::Global,
            },
            false,
            false,
        ),
    )
    .unwrap();

    let root_descriptor = root
        .closure_variables()
        .first()
        .expect("eval root lost its with object external");
    assert_eq!(root_descriptor.source, ClosureSource::EvalEnvironment(0));
    assert_eq!(root_descriptor.kind, ClosureVariableKind::WithObject);
    assert!(!root_descriptor.is_lexical);
    assert!(!root_descriptor.is_const);
    let ClosureVariableName::Constant(name) = root_descriptor.name else {
        panic!("eval root lost its with object sentinel name");
    };
    assert!(matches!(
        root.constants()[name as usize].as_primitive(),
        Some(Value::String(name)) if name == &with_object.name
    ));

    let relay = root
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("eval root lost its relay function");
    let relay_descriptor = relay
        .closure_variables()
        .iter()
        .find(|descriptor| descriptor.kind == ClosureVariableKind::WithObject)
        .expect("nested eval did not relay the with object");
    assert_eq!(relay_descriptor.source, ClosureSource::ParentClosure(0));
    let with_scope = relay.eval_environments()[0]
        .scopes
        .iter()
        .find(|scope| scope.kind == EvalScopeKind::With)
        .expect("nested eval profile lost its with scope");
    let [binding] = with_scope.bindings.as_ref() else {
        panic!("with scope did not retain exactly one binding");
    };
    assert_eq!(binding.name, with_object.name);
    assert_eq!(binding.kind, ClosureVariableKind::WithObject);
    assert!(matches!(binding.source, EvalBindingSource::Closure(_)));
    assert!(!binding.is_lexical);
    assert!(!binding.is_const);
    assert!(!binding.is_catch_parameter);
}

#[test]
fn with_statements_execute_ordered_environment_and_reference_paths() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        context
            .eval("var withGet = { value: 42 }; with (withGet) value")
            .unwrap(),
        Value::Int(42),
    );
    assert_eq!(
        context
            .eval(
                "var value = 7; var withHidden = { value: 42, [Symbol.unscopables]: { value: true } }; with (withHidden) value",
            )
            .unwrap(),
        Value::Int(7),
    );
    assert_eq!(
        context
            .eval(
                "var withWrite = { value: 1 }; with (withWrite) { value = 2; value += 3; value++; ++value; value ||= 99; } withWrite.value",
            )
            .unwrap(),
        Value::Int(7),
    );
    assert_eq!(
        context
            .eval(
                "var withCall = { value: 42, method: function(){ return this.value; } }; with (withCall) method()",
            )
            .unwrap(),
        Value::Int(42),
    );
    assert_eq!(
        context
            .eval(
                "var disappearingCall = { method: function(){} }; Object.defineProperty(disappearingCall, Symbol.unscopables, { get: function(){ delete disappearingCall.method; return {}; } }); var strictCaller; with (disappearingCall) strictCaller = function(){ 'use strict'; return method(); }; var disappearingError; try { strictCaller(); } catch (error) { disappearingError = error.name; } disappearingError",
            )
            .unwrap(),
        Value::String(JsString::from_static("TypeError")),
    );
    assert_eq!(
        context
            .eval(
                "var withDelete = { value: 1 }; var deleted; with (withDelete) deleted = delete value; deleted && !('value' in withDelete)",
            )
            .unwrap(),
        Value::Bool(true),
    );
    assert_eq!(
        context
            .eval(
                "var withCapture = { value: 42 }; var captured; with (withCapture) captured = function(){ return value; }; captured()",
            )
            .unwrap(),
        Value::Int(42),
    );

    let strict = compile_unlinked_script("'use strict'; with ({}) 0").unwrap_err();
    assert_eq!(strict.kind(), ErrorKind::Syntax);
    assert_eq!(strict.message(), "invalid keyword: with");
    assert_eq!(context.eval("with (null) 0"), Err(RuntimeError::Exception),);

    assert_eq!(
        context
            .eval(
                "var withSideEffect = 0; const withReadonly = 1; try { with ({}) withReadonly = (withSideEffect = 1); } catch (error) {} withSideEffect",
            )
            .unwrap(),
        Value::Int(0),
    );
    assert_eq!(
        context
            .eval(
                "(function(){ var object = { value: 1 }; var value; with (object) { var value = (delete object.value, 2); } return object.value + '|' + ('value' in object) + '|' + value; })()",
            )
            .unwrap(),
        Value::String(JsString::from_static("2|true|undefined")),
    );
    assert_eq!(
        context
            .eval(
                "if (false) { with ({}) let\n value = 1; } if (false) { with ({}) let\n {} } 'parsed'",
            )
            .unwrap(),
        Value::String(JsString::from_static("parsed")),
    );
}

#[test]
fn eval_compiler_rejects_incoherent_caller_variable_profiles() {
    let strict_global = EvalCompileContext::direct_with_profile(
        true,
        Vec::new(),
        EvalCallerProfile {
            scope_kinds: Box::new([]),
            variable_target: EvalCallerVariableTarget::Global,
        },
        false,
        false,
    );
    assert!(
        compile_unlinked_eval_with_filename(
            "42",
            "<eval>",
            DebugInfoMode::StripDebug,
            strict_global,
        )
        .unwrap_err()
        .message()
        .contains("variable target is not authenticated")
    );

    let variable_object = EvalRootBinding {
        name: JsString::from_static("<var>"),
        scope: 0,
        is_lexical: false,
        is_const: false,
        kind: ClosureVariableKind::EvalVariableObject,
        is_catch_parameter: false,
    };
    let sloppy_global = EvalCompileContext::direct_with_profile(
        false,
        vec![variable_object],
        EvalCallerProfile {
            scope_kinds: vec![EvalScopeKind::FunctionRoot].into_boxed_slice(),
            variable_target: EvalCallerVariableTarget::Global,
        },
        false,
        false,
    );
    assert!(
        compile_unlinked_eval_with_filename(
            "42",
            "<eval>",
            DebugInfoMode::StripDebug,
            sloppy_global,
        )
        .unwrap_err()
        .message()
        .contains("variable target is not authenticated")
    );

    let with_object = EvalRootBinding {
        name: JsString::from_static(WITH_OBJECT_LOCAL_NAME),
        scope: 0,
        is_lexical: false,
        is_const: false,
        kind: ClosureVariableKind::WithObject,
        is_catch_parameter: false,
    };
    let with_as_variable_target = EvalCompileContext::direct_with_profile(
        false,
        vec![with_object],
        EvalCallerProfile {
            scope_kinds: vec![EvalScopeKind::With].into_boxed_slice(),
            variable_target: EvalCallerVariableTarget::ExternalBinding(0),
        },
        false,
        false,
    );
    assert!(
        compile_unlinked_eval_with_filename(
            "42",
            "<eval>",
            DebugInfoMode::StripDebug,
            with_as_variable_target,
        )
        .unwrap_err()
        .message()
        .contains("variable target is not authenticated")
    );
}

#[test]
fn compiler_keeps_eval_lexicals_local_and_compiles_eval_declarations() {
    let root = compile_unlinked_eval_with_filename(
        "let answer = 40; const increment = 2; answer + increment",
        "<eval>",
        DebugInfoMode::StripDebug,
        EvalCompileContext::indirect(),
    )
    .unwrap();
    assert_eq!(root.metadata().eval_kind, EvalKind::Indirect);
    assert!(!root.metadata().strict);
    let definitions = root
        .local_definitions()
        .iter()
        .filter_map(|definition| {
            definition
                .name
                .as_ref()
                .map(|name| (name.to_utf8_lossy(), definition.is_const))
        })
        .collect::<Vec<_>>();
    assert!(definitions.contains(&("answer".to_owned(), false)));
    assert!(definitions.contains(&("increment".to_owned(), true)));

    let declarations = compile_unlinked_eval_with_filename(
        "var answer = 42; function fortyTwo() { return answer; } fortyTwo()",
        "<eval>",
        DebugInfoMode::Full,
        EvalCompileContext::indirect(),
    )
    .unwrap();
    assert!(declarations.closure_variables().iter().any(|descriptor| {
        descriptor.source == ClosureSource::GlobalDeclaration
            && descriptor.kind == ClosureVariableKind::GlobalFunction
    }));

    let strict = compile_unlinked_eval_with_filename(
        "'use strict'; var answer = 42; function fortyTwo() { return answer; }",
        "<eval>",
        DebugInfoMode::Full,
        EvalCompileContext::indirect(),
    )
    .unwrap();
    assert!(strict.metadata().strict);
    assert!(strict.local_definitions().iter().any(|definition| {
        definition
            .name
            .as_ref()
            .is_some_and(|name| name.to_utf8_lossy() == "answer")
    }));

    let nested = compile_unlinked_eval_with_filename(
        "eval('40 + 2')",
        "<eval>",
        DebugInfoMode::Full,
        EvalCompileContext::indirect(),
    )
    .unwrap();
    assert!(
        nested
            .code()
            .iter()
            .any(|instruction| { matches!(instruction, Instruction::Eval { environment: 0, .. }) })
    );
    assert_eq!(nested.eval_environments().len(), 1);
    assert_eq!(
        nested.eval_environments()[0].variable_environment,
        EvalVariableEnvironment::Global
    );
}

#[test]
fn sloppy_eval_function_owns_hidden_variable_object_and_orders_eval_pseudo_bindings() {
    let script =
        compile_unlinked_script("(function named(a) { eval(0); arguments; named; return a; })")
            .unwrap();
    let function = script.constants()[0].as_child().unwrap();
    let object_local = function
        .metadata()
        .eval_variable_object_local
        .expect("sloppy direct eval did not allocate <var>");
    assert!(matches!(
        function.code(),
        [
            Instruction::VariableEnvironment,
            Instruction::PutLocal(local),
            ..
        ] if *local == object_local
    ));
    assert_eq!(
        function.local_definitions()[usize::from(object_local)].kind,
        ClosureVariableKind::EvalVariableObject
    );

    let root_scope = function.eval_environments()[0]
        .scopes
        .iter()
        .find(|scope| scope.kind == EvalScopeKind::FunctionRoot)
        .unwrap();
    let names = root_scope
        .bindings
        .iter()
        .map(|binding| binding.name.to_utf8_lossy())
        .collect::<Vec<_>>();
    let authored = names.iter().position(|name| name == "a").unwrap();
    let object = names
        .iter()
        .position(|name| name == EVAL_VARIABLE_OBJECT_LOCAL_NAME)
        .unwrap();
    let arguments = names.iter().position(|name| name == "arguments").unwrap();
    let private_name = names.iter().position(|name| name == "named").unwrap();
    assert!(authored < object);
    assert!(object < arguments);
    assert!(object < private_name);
    let function_name_local = function
        .metadata()
        .function_name_local
        .expect("direct eval did not materialize the private function name");
    assert!(
        function
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetLocal(local) if *local == function_name_local)),
        "authored code did not read the private function name statically",
    );
    let dynamically_resolves_private_name = function
        .code()
        .iter()
        .filter_map(|instruction| match instruction {
            Instruction::HasEvalVariable { name, .. }
            | Instruction::GetEvalVariable { name, .. }
            | Instruction::PutEvalVariable { name, .. }
            | Instruction::DeleteEvalVariable { name, .. } => Some(*name),
            _ => None,
        })
        .any(|name| {
            matches!(
                function.constants()[usize::try_from(name).unwrap()].as_primitive(),
                Some(Value::String(value)) if value == &JsString::from_static("named")
            )
        });
    assert!(
        !dynamically_resolves_private_name,
        "authored private name was incorrectly wrapped by the eval variable object",
    );
}

#[test]
fn sloppy_direct_eval_keeps_source_ordered_dynamic_declarations() {
    let bindings = vec![EvalRootBinding {
        name: JsString::from_static(EVAL_VARIABLE_OBJECT_LOCAL_NAME),
        scope: 0,
        is_lexical: false,
        is_const: false,
        kind: ClosureVariableKind::EvalVariableObject,
        is_catch_parameter: false,
    }];
    let eval = compile_unlinked_eval_with_filename(
        "var fresh; function f() {} var fresh;",
        "<eval>",
        DebugInfoMode::Full,
        EvalCompileContext::direct(false, bindings),
    )
    .unwrap();
    assert!(matches!(
        eval.code(),
        [
            Instruction::Undefined,
            Instruction::DefineEvalVariable {
                source: EvalVariableSource::Closure(0),
                ..
            },
            Instruction::FClosure(_),
            Instruction::DefineEvalVariable {
                source: EvalVariableSource::Closure(0),
                ..
            },
            Instruction::Undefined,
            Instruction::DefineEvalVariable {
                source: EvalVariableSource::Closure(0),
                ..
            },
            ..
        ]
    ));
}

#[test]
fn eval_redeclaration_throw_precedes_global_and_dynamic_value_writes() {
    let caller_lexical = EvalRootBinding {
        name: JsString::from_static("x"),
        scope: 0,
        is_lexical: true,
        is_const: false,
        kind: ClosureVariableKind::Normal,
        is_catch_parameter: false,
    };
    let global = compile_unlinked_eval_with_filename(
        "function f() {} var y; var x;",
        "<eval>",
        DebugInfoMode::Full,
        EvalCompileContext::direct(false, vec![caller_lexical.clone()]),
    )
    .unwrap();
    assert!(matches!(
        global.code(),
        [
            Instruction::ThrowRedeclaration(_),
            Instruction::FClosure(_),
            ..
        ]
    ));
    assert_eq!(
        global
            .closure_variables()
            .iter()
            .filter(|descriptor| descriptor.source == ClosureSource::GlobalDeclaration)
            .count(),
        3
    );

    let object = EvalRootBinding {
        name: JsString::from_static(EVAL_VARIABLE_OBJECT_LOCAL_NAME),
        scope: 1,
        is_lexical: false,
        is_const: false,
        kind: ClosureVariableKind::EvalVariableObject,
        is_catch_parameter: false,
    };
    let dynamic_conflict = compile_unlinked_eval_with_filename(
        "var y; var x;",
        "<eval>",
        DebugInfoMode::Full,
        EvalCompileContext::direct(false, vec![caller_lexical, object.clone()]),
    )
    .unwrap();
    assert!(matches!(
        dynamic_conflict.code(),
        [
            Instruction::ThrowRedeclaration(_),
            Instruction::Undefined,
            Instruction::DefineEvalVariable { .. },
            ..
        ]
    ));

    let outer_lexical = EvalRootBinding {
        name: JsString::from_static("outer"),
        scope: 2,
        is_lexical: true,
        is_const: false,
        kind: ClosureVariableKind::Normal,
        is_catch_parameter: false,
    };
    let shadows_outer = compile_unlinked_eval_with_filename(
        "var outer;",
        "<eval>",
        DebugInfoMode::Full,
        EvalCompileContext::direct(false, vec![object, outer_lexical]),
    )
    .unwrap();
    assert!(matches!(
        shadows_outer.code(),
        [
            Instruction::Undefined,
            Instruction::DefineEvalVariable { .. },
            ..
        ]
    ));
    assert!(
        !shadows_outer
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::ThrowRedeclaration(_)))
    );
}

#[test]
fn eval_annex_b_functions_target_global_or_dynamic_variable_environments() {
    let indirect = compile_unlinked_eval_with_filename(
        "{ function f() {} }",
        "<eval>",
        DebugInfoMode::Full,
        EvalCompileContext::indirect(),
    )
    .unwrap();
    assert!(indirect.closure_variables().iter().any(|descriptor| {
        descriptor.source == ClosureSource::GlobalDeclaration
            && descriptor.kind == ClosureVariableKind::Normal
    }));

    let object = EvalRootBinding {
        name: JsString::from_static(EVAL_VARIABLE_OBJECT_LOCAL_NAME),
        scope: 0,
        is_lexical: false,
        is_const: false,
        kind: ClosureVariableKind::EvalVariableObject,
        is_catch_parameter: false,
    };
    let direct = compile_unlinked_eval_with_filename(
        "{ function f() {} }",
        "<eval>",
        DebugInfoMode::Full,
        EvalCompileContext::direct(false, vec![object.clone()]),
    )
    .unwrap();
    assert!(
        direct
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::DefineEvalVariable { .. }))
    );
    assert!(direct.code().iter().any(|instruction| matches!(
        instruction,
        Instruction::HasDynamicBinding {
            source: DynamicEnvironmentSource::Eval(_),
            ..
        }
    )));

    let catch_source = "f; try { throw null; } catch (f) {{ function f() {} }} typeof f;";
    let catch_direct = compile_unlinked_eval_with_filename(
        catch_source,
        "<eval>",
        DebugInfoMode::Full,
        EvalCompileContext::direct(false, vec![object]),
    )
    .unwrap();
    assert!(matches!(
        catch_direct.code(),
        [
            Instruction::Undefined,
            Instruction::DefineEvalVariable { .. },
            ..
        ]
    ));

    let catch_indirect = compile_unlinked_eval_with_filename(
        catch_source,
        "<eval>",
        DebugInfoMode::Full,
        EvalCompileContext::indirect(),
    )
    .unwrap();
    assert!(catch_indirect.closure_variables().iter().any(|descriptor| {
        descriptor.source == ClosureSource::GlobalDeclaration
            && descriptor.kind == ClosureVariableKind::Normal
    }));
}

#[test]
fn compiler_links_and_deduplicates_direct_eval_scope_descriptors() {
    let source = r#"
        (function outer(outerArg) {
            let outerLex = 1;
            return function middle() {
                return function inner(local) {
                    outerArg;
                    {
                        let outerLex = 2;
                        let blockOnly = 3;
                        eval(0);
                        eval(1);
                    }
                    eval(2);
                };
            };
        })
    "#;
    let script = compile_unlinked_script(source).unwrap();
    let outer = script.constants()[0].as_child().unwrap();
    let middle = outer.constants()[0].as_child().unwrap();
    let inner = middle.constants()[0].as_child().unwrap();

    let instructions = inner
        .code()
        .iter()
        .filter_map(|instruction| match instruction {
            Instruction::Eval {
                argument_count,
                environment,
            } => Some((*argument_count, *environment)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(instructions, [(1, 0), (1, 0), (1, 1)]);
    assert_eq!(inner.eval_environments().len(), 2);

    let block = &inner.eval_environments()[0];
    assert_eq!(
        block.variable_environment,
        EvalVariableEnvironment::Scope(2)
    );
    assert!(!block.caller_strict);
    assert_eq!(block.scopes[0].kind, EvalScopeKind::Block);
    assert_eq!(block.scopes[1].kind, EvalScopeKind::FunctionBody);
    assert_eq!(block.scopes[2].kind, EvalScopeKind::FunctionRoot);
    assert_eq!(block.scopes[3].kind, EvalScopeKind::FunctionBody);
    assert_eq!(block.scopes[4].kind, EvalScopeKind::FunctionRoot);
    assert_eq!(block.scopes[5].kind, EvalScopeKind::FunctionBody);
    assert_eq!(block.scopes[6].kind, EvalScopeKind::FunctionRoot);
    assert_eq!(block.scopes[7].kind, EvalScopeKind::ProgramBody);
    assert_eq!(block.scopes[8].kind, EvalScopeKind::FunctionRoot);

    let names = |environment: &crate::heap::EvalEnvironment<JsString>| {
        environment
            .scopes
            .iter()
            .flat_map(|scope| scope.bindings.iter())
            .map(|binding| {
                (
                    binding.name.to_utf8_lossy(),
                    binding.source,
                    binding.is_lexical,
                    binding.kind,
                )
            })
            .collect::<Vec<_>>()
    };
    let block_names = names(block);
    assert_eq!(block_names[0].0, "blockOnly");
    assert_eq!(block_names[1].0, "outerLex");
    assert!(matches!(block_names[0].1, EvalBindingSource::Local(_)));
    assert!(matches!(block_names[1].1, EvalBindingSource::Local(_)));
    assert!(block_names[0].2 && block_names[1].2);
    assert!(block_names.iter().any(|binding| {
        binding.0 == "outerLex" && matches!(binding.1, EvalBindingSource::Closure(_))
    }));
    assert!(block_names.iter().any(|binding| {
        binding.0 == "outerArg" && matches!(binding.1, EvalBindingSource::Closure(_))
    }));
    assert!(block_names.iter().any(|binding| {
        binding.0 == "outer"
            && matches!(binding.1, EvalBindingSource::Closure(_))
            && binding.3 == ClosureVariableKind::Normal
    }));
    assert!(block_names.iter().any(|binding| {
        binding.0 == "middle"
            && matches!(binding.1, EvalBindingSource::Closure(_))
            && binding.3 == ClosureVariableKind::Normal
    }));
    assert!(block_names.iter().any(|binding| {
        binding.0 == "inner"
            && matches!(binding.1, EvalBindingSource::Local(_))
            && binding.3 == ClosureVariableKind::FunctionName
    }));
    assert!(block_names.iter().any(|binding| {
        binding.0 == "arguments" && matches!(binding.1, EvalBindingSource::Local(_))
    }));
    assert!(block_names.iter().any(|binding| {
        binding.0 == "local" && matches!(binding.1, EvalBindingSource::Argument(0))
    }));

    let body_names = names(&inner.eval_environments()[1]);
    assert!(!body_names.iter().any(|binding| binding.0 == "blockOnly"));
    let first_outer_lex = body_names
        .iter()
        .find(|binding| binding.0 == "outerLex")
        .unwrap();
    assert!(matches!(first_outer_lex.1, EvalBindingSource::Closure(_)));
    assert_eq!(
        inner.eval_environments()[1].variable_environment,
        EvalVariableEnvironment::Scope(1)
    );

    let block_only = inner
        .local_definitions()
        .iter()
        .position(|definition| {
            definition
                .name
                .as_ref()
                .is_some_and(|name| name.to_utf8_lossy() == "blockOnly")
        })
        .and_then(|index| u16::try_from(index).ok())
        .unwrap();
    assert!(inner.code().iter().any(
        |instruction| matches!(instruction, Instruction::CloseLocal(index) if *index == block_only)
    ));
    assert!(outer.metadata().function_name_local.is_some());
    assert!(middle.metadata().function_name_local.is_some());
    assert!(inner.metadata().function_name_local.is_some());
    let outer_lex_closure = block_names
        .iter()
        .find_map(|binding| match (binding.0.as_str(), binding.1) {
            ("outerLex", EvalBindingSource::Closure(index)) => Some(index),
            _ => None,
        })
        .unwrap();
    let ClosureSource::ParentClosure(relay) =
        inner.closure_variables()[usize::from(outer_lex_closure)].source
    else {
        panic!("outer eval binding did not cross the intermediate function relay");
    };
    assert!(matches!(
        middle.closure_variables()[usize::from(relay)].source,
        ClosureSource::ParentLocal(_)
    ));

    // The eval entry prepass allocates and names this exact relay chain
    // before ordinary identifier resolution; the later reference reuses
    // the first-slot-wins descriptors without creating duplicates.
    let outer_arg_closure = block_names
        .iter()
        .find_map(|binding| match (binding.0.as_str(), binding.1) {
            ("outerArg", EvalBindingSource::Closure(index)) => Some(index),
            _ => None,
        })
        .unwrap();
    let inner_outer_arg = inner.closure_variables()[usize::from(outer_arg_closure)];
    assert!(inner.code().iter().any(
        |instruction| matches!(instruction, Instruction::GetVarRef(index) if *index == outer_arg_closure)
    ));
    let ClosureVariableName::Constant(inner_name) = inner_outer_arg.name else {
        panic!("eval-visible ordinary closure did not retain its name");
    };
    assert!(matches!(
        inner.constants()[usize::try_from(inner_name).unwrap()].as_primitive(),
        Some(Value::String(name)) if name.to_utf8_lossy() == "outerArg"
    ));
    let ClosureSource::ParentClosure(middle_outer_arg) = inner_outer_arg.source else {
        panic!("outer argument did not cross the intermediate relay");
    };
    assert_eq!(
        inner
            .closure_variables()
            .iter()
            .filter(|descriptor| descriptor.source == inner_outer_arg.source)
            .count(),
        1,
        "ordinary resolution must reuse the eval-created inner slot"
    );
    let middle_outer_arg = middle.closure_variables()[usize::from(middle_outer_arg)];
    assert_eq!(
        middle
            .closure_variables()
            .iter()
            .filter(|descriptor| descriptor.source == middle_outer_arg.source)
            .count(),
        1,
        "ordinary resolution must reuse the eval-created relay slot"
    );
    let ClosureVariableName::Constant(middle_name) = middle_outer_arg.name else {
        panic!("intermediate eval relay did not retain its ordinary name");
    };
    assert!(matches!(
        middle.constants()[usize::try_from(middle_name).unwrap()].as_primitive(),
        Some(Value::String(name)) if name.to_utf8_lossy() == "outerArg"
    ));
}

#[test]
fn eval_scope_descriptors_are_semantic_metadata_in_strip_debug_mode() {
    let source = r#"
        (function outer(argument) {
            let captured = 1;
            return function inner() {
                { let local = 2; eval(0); }
                return captured;
            };
        })
    "#;
    let full =
        compile_unlinked_script_with_filename(source, "eval-descriptor.js", DebugInfoMode::Full)
            .unwrap();
    let stripped = compile_unlinked_script_with_filename(
        source,
        "eval-descriptor.js",
        DebugInfoMode::StripDebug,
    )
    .unwrap();
    let full_outer = full.constants()[0].as_child().unwrap();
    let stripped_outer = stripped.constants()[0].as_child().unwrap();
    let full_inner = full_outer.constants()[0].as_child().unwrap();
    let stripped_inner = stripped_outer.constants()[0].as_child().unwrap();

    assert_eq!(
        full_inner.eval_environments(),
        stripped_inner.eval_environments()
    );
    assert_eq!(
        full_inner.closure_variables(),
        stripped_inner.closure_variables()
    );
    assert!(stripped_inner.local_definitions().iter().any(|definition| {
        definition
            .name
            .as_ref()
            .is_some_and(|name| name.to_utf8_lossy() == "local")
    }));

    let runtime = Runtime::new();
    runtime.set_debug_info_mode(DebugInfoMode::StripDebug);
    runtime.new_context().compile(source).unwrap();
}

#[test]
fn direct_function_body_declarations_hoist_into_argument_and_local_bindings() {
    for (source, expected) in [
        (
            "(function(){return before();function before(){return 1}})()",
            Value::Int(1),
        ),
        (
            "(function(parameter){function local(){return later}function parameter(){return 10}function local(){return later+1}let later=2;return parameter()+local()})(0)",
            Value::Int(13),
        ),
        (
            "(function(){function arguments(){return 4}return arguments()})()",
            Value::Int(4),
        ),
        (
            "(function(){var arguments;function arguments(){return 6}return arguments()})()",
            Value::Int(6),
        ),
        (
            "(function(){function arguments(){return 7}var arguments;return arguments()})()",
            Value::Int(7),
        ),
        (
            "(function(){var arguments=function(){return 9};function arguments(){return 8}return arguments()})()",
            Value::Int(9),
        ),
        (
            "(function named(){function named(){return 5}return named()})()",
            Value::Int(5),
        ),
        (
            "(function(){'use strict';function mutable(){mutable=6;return mutable}return mutable()})()",
            Value::Int(6),
        ),
        (
            "(function(){if(false)return 0;function branch(){return 2}var count=0;while(count<1)count++;return branch()+count})()",
            Value::Int(3),
        ),
    ] {
        assert_eq!(evaluate_in_context(source), expected, "{source}");
    }

    let script = compile_unlinked_script(
        "(function(first,second){var local;function local(){}function second(){}function first(){}function local(){return 1}})",
    )
    .unwrap();
    let outer = script.constants()[0].as_child().unwrap();
    assert!(matches!(outer.code()[0], Instruction::FClosure(2)));
    assert!(matches!(outer.code()[1], Instruction::PutArg(0)));
    assert!(matches!(outer.code()[2], Instruction::FClosure(1)));
    assert!(matches!(outer.code()[3], Instruction::PutArg(1)));
    assert!(matches!(outer.code()[4], Instruction::FClosure(3)));
    assert!(matches!(outer.code()[5], Instruction::PutLocal(0)));
    for child in outer
        .constants()
        .iter()
        .filter_map(|value| value.as_child())
    {
        assert_eq!(child.metadata().function_name_local, None);
    }
}

#[test]
fn direct_function_body_declaration_conflicts_match_quickjs_order() {
    for (source, message) in [
        (
            "(function(){function conflict(){};let conflict})",
            "invalid redefinition of a variable",
        ),
        (
            "(function(){let conflict;function conflict(){}})",
            "invalid redefinition of lexical identifier",
        ),
        (
            "(function(conflict){function conflict(){};let conflict})",
            "invalid redefinition of parameter name",
        ),
        (
            "(function(){'use strict';function eval(){}})",
            "invalid function name in strict code",
        ),
    ] {
        assert_eq!(
            compile_unlinked_script(source).unwrap_err().message(),
            message,
            "{source}"
        );
    }
}

#[test]
fn scoped_function_declarations_keep_entry_and_annex_closures_distinct() {
    for (source, expected) in [
        (
            "(function(){var inside;{function f(){return f}inside=f}return (inside!==f)+'|'+(inside()===inside)+'|'+(f()===inside)})()",
            "true|true|true",
        ),
        (
            "(function(){var inside;{function f(){return 1}function f(){return 2}inside=f}return inside()+'|'+f()+'|'+(inside===f)})()",
            "2|1|false",
        ),
        (
            "(function(){'use strict';var inside;{function f(){return 3}inside=f}return typeof f+'|'+inside()})()",
            "undefined|3",
        ),
        (
            "(function(parameter){var inside;{function parameter(){return 4}inside=parameter}return parameter+'|'+inside()})(1)",
            "1|4",
        ),
        (
            "(function(){var g;{f=8;function f(){}g=f}return g+'|'+typeof f+'|'+f.name})()",
            "8|function|f",
        ),
        (
            "(function(){var a,b,i=0;while(i<2){let x=i;function f(){return x}if(i===0)a=f;else b=f;i++}return (a!==b)+'|'+a()+'|'+b()})()",
            "true|0|1",
        ),
        (
            "(function(){var trace=typeof f;switch(0){case (trace+='|'+typeof f,0):function f(){return 5}}return trace+'|'+f()})()",
            "undefined|function|5",
        ),
        (
            "(function(){var original=function self(){{function self(){return 6}}return self};var replacement=original();return (replacement!==original)+'|'+replacement()})()",
            "true|6",
        ),
    ] {
        assert_eq!(
            evaluate_in_context(source),
            Value::String(JsString::from_static(expected)),
            "{source}"
        );
    }

    let script = compile_unlinked_script(
        "(function(){{function duplicate(){return 1}function duplicate(){return 2}}return duplicate()})",
    )
    .unwrap();
    let outer = script.constants()[0].as_child().unwrap();
    let closures = outer
        .code()
        .iter()
        .filter_map(|instruction| match instruction {
            Instruction::FClosure(constant) => Some(*constant),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(closures, [1, 0, 0, 1]);
    assert!(outer.code().windows(4).any(|window| matches!(
        window,
        [
            Instruction::FClosure(0),
            Instruction::Dup,
            Instruction::PutLocal(_),
            Instruction::Drop,
        ]
    )));
    assert!(!outer.code().windows(3).any(|window| matches!(
        window,
        [
            Instruction::FClosure(1),
            Instruction::Dup,
            Instruction::PutLocal(_),
        ]
    )));

    let arguments_script =
        compile_unlinked_script("(function(){{function arguments(){return 3}}return 1})").unwrap();
    let arguments_outer = arguments_script.constants()[0].as_child().unwrap();
    assert_eq!(arguments_outer.local_definitions().len(), 1);
    assert_eq!(
        arguments_outer.local_definitions()[0].name.as_ref(),
        Some(&JsString::from_static("arguments"))
    );
    assert!(arguments_outer.local_definitions()[0].is_lexical);
    assert!(
        !arguments_outer
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Dup)),
        "implicit arguments name incorrectly created an Annex root write"
    );

    let shadow_script = compile_unlinked_script(
        "(function(){let shadow=12;{function shadow(){return 13}}return shadow})",
    )
    .unwrap();
    let shadow_outer = shadow_script.constants()[0].as_child().unwrap();
    assert_eq!(shadow_outer.local_definitions().len(), 2);
    assert!(
        shadow_outer
            .local_definitions()
            .iter()
            .all(|definition| definition.is_lexical)
    );
    assert!(
        !shadow_outer
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Dup)),
        "prior enclosing lexical incorrectly allowed an Annex root write"
    );
}

#[test]
fn scoped_function_conflicts_and_global_annex_order_match_quickjs() {
    for (source, message) in [
        (
            "(function(){{let conflict;function conflict(}})",
            "invalid redefinition of lexical identifier",
        ),
        (
            "(function(){'use strict';{function duplicate(){}function duplicate(}})",
            "invalid redefinition of lexical identifier",
        ),
        (
            "(function(){{var conflict;function conflict(){}}})",
            "invalid redefinition of a variable",
        ),
        (
            "(function(){{function conflict(){}var conflict;}})",
            "invalid redefinition of lexical identifier",
        ),
    ] {
        assert_eq!(
            compile_unlinked_script(source).unwrap_err().message(),
            message,
            "{source}"
        );
    }

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context.eval("{function lateGlobalLexical(){}}let lateGlobalLexical;"),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("Annex B global lexical collision did not throw an Error object");
    };
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static(
            "lateGlobalLexical is not initialized"
        ))
    );

    assert_eq!(
        context
            .eval(
                "let priorGlobalLexical=7;{function priorGlobalLexical(){return 8}}\
                 typeof globalThis.priorGlobalLexical+'|'+priorGlobalLexical"
            )
            .unwrap(),
        Value::String(JsString::from_static("undefined|7"))
    );
}

#[test]
fn annex_b_single_and_labelled_declarations_preserve_scope_shape() {
    let program = compile_unlinked_script(
        "programLabel:function programLabel(){return programLabel};programLabel",
    )
    .unwrap();
    assert_eq!(
        program.local_definitions().len(),
        1,
        "Program labelled functions must not allocate a lexical local"
    );
    assert!(program.code().windows(4).any(|window| matches!(
        window,
        [
            Instruction::FClosure(_),
            Instruction::Dup,
            Instruction::PutVar(_),
            Instruction::PutVar(_),
        ]
    )));

    let body = compile_unlinked_script(
        "(function(){bodyLabel:function bodyLabel(){return 3};return bodyLabel})",
    )
    .unwrap();
    let body = body.constants()[0].as_child().unwrap();
    assert_eq!(body.local_definitions().len(), 2);
    assert!(body.local_definitions()[0].is_lexical);
    assert!(!body.local_definitions()[1].is_lexical);
    assert!(body.code().windows(4).any(|window| matches!(
        window,
        [
            Instruction::FClosure(_),
            Instruction::Dup,
            Instruction::PutLocal(_),
            Instruction::Drop,
        ]
    )));

    let parameter = compile_unlinked_script(
        "(function(parameter){label:function parameter(){return 4};return parameter})",
    )
    .unwrap();
    let parameter = parameter.constants()[0].as_child().unwrap();
    assert_eq!(parameter.local_definitions().len(), 1);
    assert!(parameter.local_definitions()[0].is_lexical);
    assert!(
        !parameter
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Dup)),
        "same-name parameter incorrectly received an Annex root write"
    );

    let duplicate = compile_unlinked_script(
        "(function(){if(true)function duplicate(){return 1}else function duplicate(){return 2};return duplicate})",
    )
    .unwrap();
    let duplicate = duplicate.constants()[0].as_child().unwrap();
    assert_eq!(
        duplicate
            .local_definitions()
            .iter()
            .filter(|definition| definition.is_lexical)
            .count(),
        2
    );
    assert_eq!(
        duplicate
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::Dup))
            .count(),
        1,
        "only the first same-scope if declaration is Annex-eligible"
    );

    for source in [
        "var prior;label:function prior(){}",
        "function prior(){}label:function prior(){}",
        "let prior;label:function prior(){}",
    ] {
        assert_eq!(
            compile_unlinked_script(source).unwrap_err().message(),
            "invalid redefinition of global identifier",
            "{source}"
        );
    }
    compile_unlinked_script("{var nested;}label:function nested(){}")
        .expect("a nested first var must not block the Program label exception");
}

#[test]
fn source_members_preserve_quickjs_reads_keys_references_and_method_receivers() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let data = |value| OrdinaryPropertyDescriptor {
        value: DescriptorField::Present(value),
        writable: DescriptorField::Present(true),
        enumerable: DescriptorField::Present(true),
        configurable: DescriptorField::Present(true),
        ..OrdinaryPropertyDescriptor::new()
    };

    let base = context.new_object().unwrap();
    for (name, value) in [("x", Value::Int(7)), ("default", Value::Int(8))] {
        let key = runtime.intern_property_key(name).unwrap();
        assert!(
            context
                .define_own_property(&base, &key, &data(value))
                .unwrap()
        );
    }
    let base_name = runtime.intern_property_key("base").unwrap();
    assert!(
        context
            .define_own_property(&global, &base_name, &data(Value::Object(base.clone())))
            .unwrap()
    );

    let Value::Object(method) = context
        .eval("(function(){ return this === base; })")
        .unwrap()
    else {
        panic!("method source did not produce a function");
    };
    let method_key = runtime.intern_property_key("m").unwrap();
    assert!(
        context
            .define_own_property(&base, &method_key, &data(Value::Object(method)))
            .unwrap()
    );

    let Value::Object(getter) = context.eval("(function(){ return this; })").unwrap() else {
        panic!("getter source did not produce a function");
    };
    let getter = runtime.as_callable(&getter).unwrap().unwrap();
    let getter_key = runtime.intern_property_key("receiver").unwrap();
    assert!(
        context
            .define_own_property(
                &base,
                &getter_key,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(getter)),
                    set: DescriptorField::Present(AccessorValue::Undefined),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );

    assert_eq!(context.eval("base.x").unwrap(), Value::Int(7));
    assert_eq!(context.eval("base['x']").unwrap(), Value::Int(7));
    assert_eq!(context.eval("base.default").unwrap(), Value::Int(8));
    assert_eq!(context.eval("base\n.x").unwrap(), Value::Int(7));
    assert_eq!(context.eval("base\n['x']").unwrap(), Value::Int(7));
    assert_eq!(
        context.eval("base.receiver === base").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(context.eval("base.m()").unwrap(), Value::Bool(true));
    assert_eq!(context.eval("base['m']()").unwrap(), Value::Bool(true));
    assert_eq!(context.eval("((base.m))()").unwrap(), Value::Bool(true));
    assert_eq!(context.eval("(0, base.m)()").unwrap(), Value::Bool(false));
    assert_eq!(
        context.eval("(true ? base.m : base.m)()").unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        context.eval("(true && base.m)()").unwrap(),
        Value::Bool(false)
    );

    let hint = runtime.intern_property_key("keyHint").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &hint,
                &data(Value::String(JsString::from_static("none"))),
            )
            .unwrap()
    );
    let Value::Object(to_key) = context
        .eval("(function(hint){ keyHint = hint; return 'x'; })")
        .unwrap()
    else {
        panic!("ToPropertyKey source did not produce a function");
    };
    let key_object = context.new_object().unwrap();
    let to_primitive = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive));
    assert!(
        context
            .define_own_property(&key_object, &to_primitive, &data(Value::Object(to_key)))
            .unwrap()
    );
    let key_name = runtime.intern_property_key("keyObject").unwrap();
    assert!(
        context
            .define_own_property(&global, &key_name, &data(Value::Object(key_object)),)
            .unwrap()
    );
    assert_eq!(context.eval("base[keyObject]").unwrap(), Value::Int(7));
    assert_eq!(
        context.eval("keyHint").unwrap(),
        Value::String(JsString::from_static("string"))
    );
    assert_eq!(
        context.eval("keyHint = 'none'").unwrap(),
        Value::String(JsString::from_static("none"))
    );
    assert!(matches!(
        context.eval("null[keyObject]"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();
    assert_eq!(
        context.eval("keyHint").unwrap(),
        Value::String(JsString::from_static("none"))
    );

    assert_eq!(context.eval("'abc'.length").unwrap(), Value::Int(3));
    assert_eq!(
        context.eval("'abc'[1]").unwrap(),
        Value::String(JsString::from_static("b"))
    );
    assert!(matches!(
        context.eval("Function().toString()"),
        Ok(Value::String(_))
    ));
}

#[test]
fn member_assignment_and_delete_lower_through_quickjs_lvalue_shapes() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    let fixed = context.compile("Function.fixed = 1").unwrap();
    let fixed_code = runtime.test_function_code(&fixed).unwrap();
    assert!(
        fixed_code
            .windows(2)
            .any(|window| matches!(window, [Instruction::Insert2, Instruction::PutField(_)]))
    );
    assert!(
        !fixed_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetField(_)))
    );

    let computed = context.compile("Function['computed'] = 2").unwrap();
    let computed_code = runtime.test_function_code(&computed).unwrap();
    assert!(
        computed_code
            .windows(2)
            .any(|window| matches!(window, [Instruction::Insert3, Instruction::PutArrayEl]))
    );
    assert!(
        !computed_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetArrayEl))
    );

    let fixed_compound = context.compile("Function.fixed += 3").unwrap();
    let fixed_compound_code = runtime.test_function_code(&fixed_compound).unwrap();
    assert!(
        fixed_compound_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetField2(_)))
    );
    assert!(fixed_compound_code.windows(3).any(|window| matches!(
        window,
        [
            Instruction::Add,
            Instruction::Insert2,
            Instruction::PutField(_)
        ]
    )));

    let computed_compound = context.compile("Function['computed'] += 4").unwrap();
    let computed_compound_code = runtime.test_function_code(&computed_compound).unwrap();
    assert!(
        computed_compound_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetArrayEl3))
    );
    assert!(computed_compound_code.windows(3).any(|window| matches!(
        window,
        [
            Instruction::Add,
            Instruction::Insert3,
            Instruction::PutArrayEl
        ]
    )));

    let fixed_delete = context.compile("delete Function.fixed").unwrap();
    let fixed_delete_code = runtime.test_function_code(&fixed_delete).unwrap();
    assert!(
        fixed_delete_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Delete))
    );
    assert!(
        !fixed_delete_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetField(_)))
    );

    assert_eq!(
        context
            .eval("Function.paren = 1; (Function.paren) = 2; Function.paren")
            .unwrap(),
        Value::Int(2)
    );
    assert!(context.compile("(0, Function.fixed) = 1").is_err());
    assert!(
        context
            .compile("(true ? Function.fixed : Function.fixed) = 1")
            .is_err()
    );

    assert_eq!(
        context
            .eval("Function.keep = 3; delete (0, Function.keep); Function.keep")
            .unwrap(),
        Value::Int(3)
    );
    assert_eq!(
        context
            .eval("Function.gone = 4; delete (Function.gone); Function.gone")
            .unwrap(),
        Value::Undefined
    );
    assert!(context.compile("'use strict'; delete Function").is_err());
    let direct_delete = context.compile("delete __qjo_delete_global").unwrap();
    assert!(
        runtime
            .test_function_code(&direct_delete)
            .unwrap()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::DeleteVar(0)))
    );
}

#[test]
fn direct_identifier_delete_uses_quickjs_scope_resolution() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        context.eval("delete __qjo_missing_delete").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        context
            .eval("(function(){ var value = 1; return delete value; })()")
            .unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        context
            .eval("(function(value){ return delete value; })(1)")
            .unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        context
            .eval("(function(value){ return (function(){ return delete value; })(); })(1)")
            .unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        context
            .eval("(function named(){ return delete named; })()")
            .unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        context
            .eval("(function(){ return delete arguments; })()")
            .unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        context
            .eval("(function(){ var value = 1; return delete (value); })()")
            .unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        context
            .eval("(function(){ var value = 1; return delete (0, value); })()")
            .unwrap(),
        Value::Bool(true)
    );

    assert_eq!(
        context
            .eval("__qjo_delete_global = 1; delete __qjo_delete_global")
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        context.eval("typeof __qjo_delete_global").unwrap(),
        Value::String(JsString::from_static("undefined"))
    );

    let Value::Object(reader) = context
        .eval("(function(){ return __qjo_delete_reconnect; })")
        .unwrap()
    else {
        panic!("delete/reconnect probe did not produce a function");
    };
    let reader = runtime.as_callable(&reader).unwrap().unwrap();
    assert_eq!(
        context.eval("__qjo_delete_reconnect = 1").unwrap(),
        Value::Int(1)
    );
    assert_eq!(
        context.call(&reader, Value::Undefined, &[]).unwrap(),
        Value::Int(1)
    );
    assert_eq!(
        context.eval("delete __qjo_delete_reconnect").unwrap(),
        Value::Bool(true)
    );
    assert!(matches!(
        context.call(&reader, Value::Undefined, &[]),
        Err(RuntimeError::Exception)
    ));
    assert!(context.take_exception().unwrap().is_some());
    assert_eq!(
        context.eval("__qjo_delete_reconnect = 2").unwrap(),
        Value::Int(2)
    );
    assert_eq!(
        context.call(&reader, Value::Undefined, &[]).unwrap(),
        Value::Int(2)
    );

    for name in ["undefined", "NaN", "Infinity"] {
        assert_eq!(
            context.eval(&format!("delete {name}")).unwrap(),
            Value::Bool(false),
            "global constant {name}"
        );
    }

    let mut caller = runtime.new_context();
    let realm_key = runtime.intern_property_key("__qjo_delete_realm").unwrap();
    let descriptor = OrdinaryPropertyDescriptor {
        value: DescriptorField::Present(Value::Int(1)),
        writable: DescriptorField::Present(true),
        enumerable: DescriptorField::Present(true),
        configurable: DescriptorField::Present(true),
        ..OrdinaryPropertyDescriptor::new()
    };
    assert!(
        context
            .define_own_property(&context.global_object().unwrap(), &realm_key, &descriptor)
            .unwrap()
    );
    assert!(
        caller
            .define_own_property(&caller.global_object().unwrap(), &realm_key, &descriptor)
            .unwrap()
    );
    let Value::Object(deleter) = context
        .eval("(function(){ return delete __qjo_delete_realm; })")
        .unwrap()
    else {
        panic!("cross-realm delete source did not produce a function");
    };
    let deleter = runtime.as_callable(&deleter).unwrap().unwrap();
    assert_eq!(
        caller.call(&deleter, Value::Undefined, &[]).unwrap(),
        Value::Bool(true)
    );
    assert!(
        !runtime
            .has_own_property(&context.global_object().unwrap(), &realm_key)
            .unwrap()
    );
    assert!(
        runtime
            .has_own_property(&caller.global_object().unwrap(), &realm_key)
            .unwrap()
    );

    let global = context.compile("delete __qjo_delete_opcode").unwrap();
    assert!(
        runtime
            .test_function_code(&global)
            .unwrap()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::DeleteVar(0)))
    );
    let local_root = context
        .compile("(function(value){ return delete value; })")
        .unwrap();
    let local = runtime
        .test_child_function_bytecode(&local_root, 0)
        .unwrap();
    assert!(
        runtime
            .test_function_code(&local)
            .unwrap()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::PushFalse))
    );

    for source in [
        "'use strict'; delete direct",
        "'use strict'; delete (direct)",
    ] {
        let error = compile_script(source).unwrap_err();
        assert_eq!(
            error.message(),
            "cannot delete a direct reference in strict mode"
        );
        assert_eq!(
            error.span().unwrap().start.column,
            u32::try_from(source.len() + 1).unwrap()
        );
    }
}

#[test]
fn bitwise_compound_assignment_reuses_quickjs_lvalue_shapes() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    let fixed = context.compile("Function.bits &= 3").unwrap();
    let fixed_code = runtime.test_function_code(&fixed).unwrap();
    assert!(fixed_code.windows(3).any(|window| matches!(
        window,
        [
            Instruction::BitAnd,
            Instruction::Insert2,
            Instruction::PutField(_)
        ]
    )));

    let computed = context.compile("Function['bits'] ^= 4").unwrap();
    let computed_code = runtime.test_function_code(&computed).unwrap();
    assert!(computed_code.windows(3).any(|window| matches!(
        window,
        [
            Instruction::BitXor,
            Instruction::Insert3,
            Instruction::PutArrayEl
        ]
    )));

    let identifier_root = context
        .compile("(function(value){ value |= 8; return value; })")
        .unwrap();
    let identifier = runtime
        .test_child_function_bytecode(&identifier_root, 0)
        .unwrap();
    let identifier_code = runtime.test_function_code(&identifier).unwrap();
    assert!(
        identifier_code
            .windows(2)
            .any(|window| matches!(window, [Instruction::BitOr, Instruction::SetArg(0)]))
    );

    let local_root = context
        .compile("(function(){ var value = 7; value &= 3; return value; })")
        .unwrap();
    let local = runtime
        .test_child_function_bytecode(&local_root, 0)
        .unwrap();
    let local_code = runtime.test_function_code(&local).unwrap();
    assert!(
        local_code
            .windows(2)
            .any(|window| matches!(window, [Instruction::BitAnd, Instruction::SetLocal(0)]))
    );

    let closure_root = context
        .compile("(function(value){ return function(){ value ^= 3; return value; }; })")
        .unwrap();
    let closure_outer = runtime
        .test_child_function_bytecode(&closure_root, 0)
        .unwrap();
    let closure = runtime
        .test_child_function_bytecode(&closure_outer, 0)
        .unwrap();
    let closure_code = runtime.test_function_code(&closure).unwrap();
    assert!(
        closure_code
            .windows(2)
            .any(|window| matches!(window, [Instruction::BitXor, Instruction::SetVarRef(0)]))
    );

    let global = context.compile("__qjo_bit_global |= 8").unwrap();
    let global_code = runtime.test_function_code(&global).unwrap();
    assert!(
        global_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetVar(_)))
    );
    assert!(global_code.windows(3).any(|window| matches!(
        window,
        [Instruction::BitOr, Instruction::Dup, Instruction::PutVar(_)]
    )));

    let sloppy_private_root = context
        .compile("(function named(){ named &= 1; return named; })")
        .unwrap();
    let sloppy_private = runtime
        .test_child_function_bytecode(&sloppy_private_root, 0)
        .unwrap();
    let sloppy_private_code = runtime.test_function_code(&sloppy_private).unwrap();
    assert!(
        sloppy_private_code
            .windows(2)
            .any(|window| matches!(window, [Instruction::BitAnd, Instruction::Nop]))
    );

    let strict_private_root = context
        .compile("(function named(){ 'use strict'; named |= 1; })")
        .unwrap();
    let strict_private = runtime
        .test_child_function_bytecode(&strict_private_root, 0)
        .unwrap();
    let strict_private_code = runtime.test_function_code(&strict_private).unwrap();
    assert!(
        strict_private_code
            .windows(2)
            .any(|window| matches!(window, [Instruction::BitOr, Instruction::ThrowReadOnly(_)]))
    );

    assert_eq!(
        context
            .eval(
                "(function(){ var value = 14; value &= 11; value ^= 3; \
                 value |= 4; return value; })()"
            )
            .unwrap(),
        Value::Int(13)
    );
    assert_eq!(
        context
            .eval("(function(value){ value |= 8; return value; })(1)")
            .unwrap(),
        Value::Int(9)
    );
    assert_eq!(
        context
            .eval(
                "(function(value){ return (function(){ value ^= 3; \
                 return value; })(); })(5)"
            )
            .unwrap(),
        Value::Int(6)
    );
    assert_eq!(
        context
            .eval(
                "Function.bits = 14; Function.bits &= 11; \
                 Function['bits'] ^= 3; Function.bits |= 4"
            )
            .unwrap(),
        Value::Int(13)
    );
    assert_eq!(
        context
            .eval(
                "(function(){ var left = 1, right = 3; \
                 left |= right &= 2; return left * 10 + right; })()"
            )
            .unwrap(),
        Value::Int(32)
    );
    assert_eq!(
        context
            .eval(
                "(function(){ var value = -1n; (value) &= \
                 123456789012345678901234567890n; return value; })()"
            )
            .unwrap(),
        Value::BigInt(JsBigInt::parse_js_string("123456789012345678901234567890").unwrap())
    );
    assert_eq!(
        context
            .eval("(function(value){ (value) |= 2; return value; })(1)")
            .unwrap(),
        Value::Int(3)
    );

    assert!(context.compile("(Function.bits) |= 1").is_ok());
    assert!(context.compile("(bitwiseIdentifier) &= 1").is_ok());
    assert!(context.compile("(0, Function.bits) |= 1").is_err());
    assert!(
        context
            .compile("(true ? Function.bits : Function.bits) |= 1")
            .is_err()
    );
    assert!(context.compile("(Function.bits & 1) |= 1").is_err());
}

#[test]
fn shift_compound_assignment_reuses_quickjs_lvalue_shapes() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    let fixed = context.compile("Function.shift <<= 3").unwrap();
    let fixed_code = runtime.test_function_code(&fixed).unwrap();
    assert!(fixed_code.windows(3).any(|window| matches!(
        window,
        [
            Instruction::Shl,
            Instruction::Insert2,
            Instruction::PutField(_)
        ]
    )));

    let computed = context.compile("Function['shift'] >>= 2").unwrap();
    let computed_code = runtime.test_function_code(&computed).unwrap();
    assert!(computed_code.windows(3).any(|window| matches!(
        window,
        [
            Instruction::Sar,
            Instruction::Insert3,
            Instruction::PutArrayEl
        ]
    )));

    let argument_root = context
        .compile("(function(value){ value >>>= 1; return value; })")
        .unwrap();
    let argument = runtime
        .test_child_function_bytecode(&argument_root, 0)
        .unwrap();
    let argument_code = runtime.test_function_code(&argument).unwrap();
    assert!(
        argument_code
            .windows(2)
            .any(|window| matches!(window, [Instruction::Shr, Instruction::SetArg(0)]))
    );

    let closure_root = context
        .compile("(function(value){ return function(){ value >>= 2; return value; }; })")
        .unwrap();
    let closure_outer = runtime
        .test_child_function_bytecode(&closure_root, 0)
        .unwrap();
    let closure = runtime
        .test_child_function_bytecode(&closure_outer, 0)
        .unwrap();
    let closure_code = runtime.test_function_code(&closure).unwrap();
    assert!(
        closure_code
            .windows(2)
            .any(|window| matches!(window, [Instruction::Sar, Instruction::SetVarRef(0)]))
    );

    let global = context.compile("__qjo_shift_global <<= 1").unwrap();
    let global_code = runtime.test_function_code(&global).unwrap();
    assert!(global_code.windows(3).any(|window| matches!(
        window,
        [Instruction::Shl, Instruction::Dup, Instruction::PutVar(_)]
    )));

    assert_eq!(
        context
            .eval(
                "(function(){ var value = 3; value <<= 2; value >>= 1; \
                 value >>>= 1; return value; })()"
            )
            .unwrap(),
        Value::Int(3)
    );
    assert_eq!(
        context
            .eval(
                "Function.shift = -8; Function.shift >>= 1; \
                 Function['shift'] >>>= 1"
            )
            .unwrap(),
        Value::Int(2_147_483_646)
    );
    assert_eq!(
        context
            .eval(
                "(function(){ var left = 1, right = 3; \
                 left <<= right >>= 1; return left * 10 + right; })()"
            )
            .unwrap(),
        Value::Int(21)
    );
    assert_eq!(
        context
            .eval(
                "(function(value){ return (function(){ value >>= 2; \
                 return value; })(); })(-8)"
            )
            .unwrap(),
        Value::Int(-2)
    );

    assert!(context.compile("(Function.shift) >>>= 1").is_ok());
    assert!(context.compile("(shiftIdentifier) <<= 1").is_ok());
    assert!(context.compile("(0, Function.shift) >>= 1").is_err());
    assert!(context.compile("(Function.shift << 1) >>= 1").is_err());
}

#[test]
fn exponent_compound_assignment_reuses_quickjs_lvalue_shapes() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    let fixed = context.compile("Function.power **= 3").unwrap();
    let fixed_code = runtime.test_function_code(&fixed).unwrap();
    assert!(fixed_code.windows(3).any(|window| matches!(
        window,
        [
            Instruction::Pow,
            Instruction::Insert2,
            Instruction::PutField(_)
        ]
    )));

    let computed = context.compile("Function['power'] **= 2").unwrap();
    let computed_code = runtime.test_function_code(&computed).unwrap();
    assert!(computed_code.windows(3).any(|window| matches!(
        window,
        [
            Instruction::Pow,
            Instruction::Insert3,
            Instruction::PutArrayEl
        ]
    )));

    let argument_root = context
        .compile("(function(value){ value **= 3; return value; })")
        .unwrap();
    let argument = runtime
        .test_child_function_bytecode(&argument_root, 0)
        .unwrap();
    let argument_code = runtime.test_function_code(&argument).unwrap();
    assert!(
        argument_code
            .windows(2)
            .any(|window| matches!(window, [Instruction::Pow, Instruction::SetArg(0)]))
    );

    let closure_root = context
        .compile("(function(value){ return function(){ value **= 2; return value; }; })")
        .unwrap();
    let closure_outer = runtime
        .test_child_function_bytecode(&closure_root, 0)
        .unwrap();
    let closure = runtime
        .test_child_function_bytecode(&closure_outer, 0)
        .unwrap();
    let closure_code = runtime.test_function_code(&closure).unwrap();
    assert!(
        closure_code
            .windows(2)
            .any(|window| matches!(window, [Instruction::Pow, Instruction::SetVarRef(0)]))
    );

    let global = context.compile("__qjo_power_global **= 2").unwrap();
    let global_code = runtime.test_function_code(&global).unwrap();
    assert!(global_code.windows(3).any(|window| matches!(
        window,
        [Instruction::Pow, Instruction::Dup, Instruction::PutVar(_)]
    )));

    let sloppy_private_root = context
        .compile("(function named(){ named **= 1; return named; })")
        .unwrap();
    let sloppy_private = runtime
        .test_child_function_bytecode(&sloppy_private_root, 0)
        .unwrap();
    let sloppy_private_code = runtime.test_function_code(&sloppy_private).unwrap();
    assert!(
        sloppy_private_code
            .windows(2)
            .any(|window| matches!(window, [Instruction::Pow, Instruction::Nop]))
    );

    let strict_private_root = context
        .compile("(function named(){ 'use strict'; named **= 1; })")
        .unwrap();
    let strict_private = runtime
        .test_child_function_bytecode(&strict_private_root, 0)
        .unwrap();
    let strict_private_code = runtime.test_function_code(&strict_private).unwrap();
    assert!(
        strict_private_code
            .windows(2)
            .any(|window| matches!(window, [Instruction::Pow, Instruction::ThrowReadOnly(_)]))
    );

    assert_eq!(
        context
            .eval("(function(){ var value = 2; value **= 3; return value; })()")
            .unwrap(),
        Value::Int(8)
    );
    assert_eq!(
        context
            .eval(
                "(function(value){ return (function(){ value **= 2; \
                 return value; })(); })(3)"
            )
            .unwrap(),
        Value::Int(9)
    );
    assert_eq!(
        context
            .eval(
                "(function(){ var left = 2, right = 3; \
                 left **= right **= 2; return left + right; })()"
            )
            .unwrap(),
        Value::Int(521)
    );
    assert_eq!(
        context
            .eval("(function(){ var value = 2n; (value) **= 100n; return value; })()")
            .unwrap(),
        Value::BigInt(JsBigInt::parse_js_string("1267650600228229401496703205376").unwrap())
    );

    assert!(context.compile("(Function.power) **= 2").is_ok());
    assert!(context.compile("(powerIdentifier) **= 2").is_ok());
    assert!(context.compile("(0, Function.power) **= 2").is_err());
    assert!(context.compile("(Function.power ** 1) **= 2").is_err());
}

#[test]
fn logical_member_assignment_uses_quickjs_branch_cleanup_shapes() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    let fixed = context.compile("Function.fixed &&= 3").unwrap();
    let fixed_code = runtime.test_function_code(&fixed).unwrap();
    assert!(
        fixed_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetField2(_)))
    );
    assert!(
        fixed_code
            .windows(2)
            .any(|window| matches!(window, [Instruction::Insert2, Instruction::PutField(_)]))
    );
    let fixed_branch = fixed_code
        .iter()
        .position(|instruction| matches!(instruction, Instruction::IfFalse(_)))
        .unwrap();
    let Instruction::IfFalse(fixed_short) = fixed_code[fixed_branch] else {
        unreachable!();
    };
    assert!(matches!(
        fixed_code[usize::try_from(fixed_short).unwrap()],
        Instruction::Nip
    ));
    assert_eq!(
        fixed_code
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::Nip))
            .count(),
        1
    );

    let computed = context.compile("Function['computed'] ||= 4").unwrap();
    let computed_code = runtime.test_function_code(&computed).unwrap();
    assert!(
        computed_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetArrayEl3))
    );
    assert!(
        computed_code
            .windows(2)
            .any(|window| matches!(window, [Instruction::Insert3, Instruction::PutArrayEl]))
    );
    let computed_branch = computed_code
        .iter()
        .position(|instruction| matches!(instruction, Instruction::IfTrue(_)))
        .unwrap();
    let Instruction::IfTrue(computed_short) = computed_code[computed_branch] else {
        unreachable!();
    };
    let computed_short = usize::try_from(computed_short).unwrap();
    assert!(matches!(computed_code[computed_short], Instruction::Nip));
    assert!(matches!(
        computed_code[computed_short + 1],
        Instruction::Nip
    ));
    assert_eq!(
        computed_code
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::Nip))
            .count(),
        2
    );
    let computed_goto = computed_code
        .iter()
        .find_map(|instruction| match instruction {
            Instruction::Goto(target) => Some(usize::try_from(*target).unwrap()),
            _ => None,
        })
        .unwrap();
    assert_eq!(computed_goto, computed_short + 2);

    let nullish = context.compile("Function.nullish ??= 5").unwrap();
    let nullish_code = runtime.test_function_code(&nullish).unwrap();
    assert!(nullish_code.windows(3).any(|window| matches!(
        window,
        [
            Instruction::Dup,
            Instruction::IsUndefinedOrNull,
            Instruction::IfFalse(_)
        ]
    )));

    assert_eq!(
        context
            .eval("Function.logic = 0; Function.logic ||= 7")
            .unwrap(),
        Value::Int(7)
    );
    assert_eq!(
        context
            .eval("Function.logic = 0; Function.logic &&= 8")
            .unwrap(),
        Value::Int(0)
    );
    assert_eq!(
        context
            .eval("Function.logic = null; Function.logic ??= 9")
            .unwrap(),
        Value::Int(9)
    );
    assert_eq!(
        context
            .eval(
                "Function.left = 1; Function.right = 0; \
                 Function.left &&= Function.right ||= 9; \
                 Function.left + Function.right"
            )
            .unwrap(),
        Value::Int(18)
    );
    assert_eq!(
        context
            .eval(
                "Function.outer = 1; Function.inner = 0; \
                 Function['outer'] += (Function['inner'] ||= 2); \
                 Function.outer + Function.inner"
            )
            .unwrap(),
        Value::Int(5)
    );

    let logical_call = context
        .compile("(Function.callable ||= function(){ return this === Function; })()")
        .unwrap();
    let logical_call_code = runtime.test_function_code(&logical_call).unwrap();
    assert!(
        logical_call_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Call(0)))
    );
    assert!(
        !logical_call_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::CallMethod(_)))
    );
    assert_eq!(
        context
            .eval("(Function.callable ||= function(){ return this === Function; })()")
            .unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        context
            .eval("delete Function.anon; (Function.anon ??= function(){}).name")
            .unwrap(),
        Value::String(JsString::from_static(""))
    );

    assert!(context.compile("(Function.fixed) &&= 1").is_ok());
    assert!(context.compile("(0, Function.fixed) &&= 1").is_err());
    assert!(
        context
            .compile("(true ? Function.fixed : Function.fixed) &&= 1")
            .is_err()
    );
    assert!(
        context
            .compile("(Function.fixed || Function.computed) &&= 1")
            .is_err()
    );
}

#[test]
fn identifier_compound_assignment_uses_resolved_get_set_paths() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    let argument_root = context
        .compile("(function(value){ value += 2; return value; })")
        .unwrap();
    let argument = runtime
        .test_child_function_bytecode(&argument_root, 0)
        .unwrap();
    let argument_code = runtime.test_function_code(&argument).unwrap();
    assert!(
        argument_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetArg(0)))
    );
    assert!(
        argument_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::SetArg(0)))
    );

    let local_root = context
        .compile("(function(){ var value = 1; value ||= 4; return value; })")
        .unwrap();
    let local = runtime
        .test_child_function_bytecode(&local_root, 0)
        .unwrap();
    let local_code = runtime.test_function_code(&local).unwrap();
    assert!(
        local_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetLocal(0)))
    );
    assert!(
        local_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::SetLocal(0)))
    );
    let branch = local_code
        .iter()
        .find_map(|instruction| match instruction {
            Instruction::IfTrue(target) => Some(usize::try_from(*target).unwrap()),
            _ => None,
        })
        .unwrap();
    let end = local_code
        .iter()
        .find_map(|instruction| match instruction {
            Instruction::Goto(target) => Some(usize::try_from(*target).unwrap()),
            _ => None,
        })
        .unwrap();
    assert_eq!(branch, end);
    assert!(
        !local_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Nip))
    );

    let closure_root = context
        .compile("(function(value){ return function(){ value += 2; return value; }; })")
        .unwrap();
    let closure_outer = runtime
        .test_child_function_bytecode(&closure_root, 0)
        .unwrap();
    let closure_inner = runtime
        .test_child_function_bytecode(&closure_outer, 0)
        .unwrap();
    let closure_code = runtime.test_function_code(&closure_inner).unwrap();
    assert!(
        closure_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetVarRef(0)))
    );
    assert!(
        closure_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::SetVarRef(0)))
    );

    let global = context.compile("identifierCompoundGlobal ||= 2").unwrap();
    let global_code = runtime.test_function_code(&global).unwrap();
    assert!(
        global_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetVar(_)))
    );
    assert!(
        global_code
            .windows(2)
            .any(|window| matches!(window, [Instruction::Dup, Instruction::PutVar(_)]))
    );
    let global_branch = global_code
        .iter()
        .find_map(|instruction| match instruction {
            Instruction::IfTrue(target) => Some(*target),
            _ => None,
        })
        .unwrap();
    let global_end = global_code
        .iter()
        .find_map(|instruction| match instruction {
            Instruction::Goto(target) => Some(*target),
            _ => None,
        })
        .unwrap();
    assert_eq!(global_branch, global_end);

    let sloppy_self_root = context
        .compile("(function named(){ named += ''; return named; })")
        .unwrap();
    let sloppy_self = runtime
        .test_child_function_bytecode(&sloppy_self_root, 0)
        .unwrap();
    let sloppy_self_code = runtime.test_function_code(&sloppy_self).unwrap();
    assert!(
        sloppy_self_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Nop))
    );
    assert!(
        !sloppy_self_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::SetLocal(_)))
    );

    let strict_self_root = context
        .compile("(function named(){ 'use strict'; named &&= 1; })")
        .unwrap();
    let strict_self = runtime
        .test_child_function_bytecode(&strict_self_root, 0)
        .unwrap();
    let strict_self_code = runtime.test_function_code(&strict_self).unwrap();
    assert!(
        strict_self_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::ThrowReadOnly(_)))
    );
    assert!(
        !strict_self_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::SetLocal(_)))
    );

    assert_eq!(
        context
            .eval("(function(value){ value += 2; return value; })(3)")
            .unwrap(),
        Value::Int(5)
    );
    assert_eq!(
        context
            .eval(
                "(function(){ var value = 20; value += 2; value -= 4; \
                 value *= 3; value /= 2; value %= 5; return value; })()"
            )
            .unwrap(),
        Value::Int(2)
    );
    assert_eq!(
        context
            .eval(
                "(function(value){ return (function(){ value += 2; return value; })() \
                 + (function(){ value += 3; return value; })(); })(1)"
            )
            .unwrap(),
        Value::Int(9)
    );
    assert_eq!(
        context
            .eval("identifierCompoundGlobal = 1; identifierCompoundGlobal += 2")
            .unwrap(),
        Value::Int(3)
    );

    for (source, expected) in [
        (
            "(function(value){ value &&= 9; return value; })(2)",
            Value::Int(9),
        ),
        (
            "(function(value){ value &&= 9; return value; })(0)",
            Value::Int(0),
        ),
        (
            "(function(value){ value ||= 9; return value; })(0)",
            Value::Int(9),
        ),
        (
            "(function(value){ value ||= 9; return value; })(2)",
            Value::Int(2),
        ),
        (
            "(function(value){ value ??= 9; return value; })(null)",
            Value::Int(9),
        ),
        (
            "(function(value){ value ??= 9; return value; })(false)",
            Value::Bool(false),
        ),
    ] {
        assert_eq!(context.eval(source).unwrap(), expected, "{source}");
    }

    assert_eq!(
        context
            .eval("(function(){ var named; named ??= function(){}; return named.name; })()")
            .unwrap(),
        Value::String(JsString::from_static("named"))
    );
    assert_eq!(
        context
            .eval("(function(){ var named; (named) ??= function(){}; return named.name; })()")
            .unwrap(),
        Value::String(JsString::from_static(""))
    );
    assert_eq!(
        context
            .eval("(function(){ var named; (named = function(){}); return named.name; })()")
            .unwrap(),
        Value::String(JsString::from_static("named"))
    );
    assert_eq!(
        context
            .eval("(function(){ var named; (named) = function(){}; return named.name; })()")
            .unwrap(),
        Value::String(JsString::from_static(""))
    );

    assert_eq!(
        context
            .eval("(function(value){ (value) += 2; return value; })(3)")
            .unwrap(),
        Value::Int(5)
    );
    assert!(
        context
            .compile("(function(a,b){ (0, a) += 1; return a; })")
            .is_err()
    );
    assert!(
        context
            .compile("(function(a,b){ (true ? a : b) ||= 1; return a; })")
            .is_err()
    );
    assert!(
        context
            .compile("(function(){ 'use strict'; eval += 1; })")
            .is_err()
    );
    assert!(
        context
            .compile("(function(){ 'use strict'; (arguments) ??= 1; })")
            .is_err()
    );

    assert_eq!(
        context
            .eval(
                "(function named(){ var result = named += ''; \
                 return typeof result + '|' + typeof named; })()"
            )
            .unwrap(),
        Value::String(JsString::from_static("string|function"))
    );
    assert_eq!(
        context
            .eval(
                "(function(wrapper){ return wrapper() === wrapper; })(function named(){ \
                 return named ||= 1; })"
            )
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn named_function_expression_has_intrinsic_name_and_private_recursive_binding() {
    assert_eq!(
        evaluate_in_context("(function fact(n) { return n ? n * fact(n - 1) : 1; })(5)"),
        Value::Int(120)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(f) { return f() === f; })(function anonymous() { return anonymous; })"
        ),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_in_context("(function named(named) { return named; })(42)"),
        Value::Int(42)
    );
    assert_eq!(
        evaluate_in_context("(function named() { var named = 42; return named; })()"),
        Value::Int(42)
    );
    assert_eq!(
        evaluate_in_context("(function named() {}), typeof named"),
        Value::String(JsString::from_static("undefined"))
    );

    let (name, writable, enumerable, configurable) =
        evaluate_function_name("(function named() {})");
    assert_eq!(name, JsString::from_static("named"));
    assert!(!writable);
    assert!(!enumerable);
    assert!(configurable);

    let (name, ..) = evaluate_function_name(
        "(function() { var inferred = function intrinsic() {}; return inferred; })()",
    );
    assert_eq!(name, JsString::from_static("intrinsic"));

    let script = compile_unlinked_script("(function unusedName() { return 1; })").unwrap();
    let function = script.constants()[0].as_child().unwrap();
    assert_eq!(function.metadata().function_name_local, None);
    assert_eq!(function.metadata().local_count, 0);

    let script = compile_unlinked_script("(function self() { return self; })").unwrap();
    let function = script.constants()[0].as_child().unwrap();
    assert_eq!(function.metadata().function_name_local, Some(0));
    assert_eq!(function.metadata().local_count, 1);
}

#[test]
fn named_function_self_binding_captures_through_relays_and_is_per_instance() {
    let source = "(function(f) { return f()()() === f; })(function named() { return function() { return function() { return named; }; }; })";
    assert_eq!(evaluate_in_context(source), Value::Bool(true));
    assert_eq!(
        evaluate_in_context(
            "(function() { var make = function() { return function named() { return named; }; }; var a = make(), b = make(); return a() === a && b() === b && a !== b; })()"
        ),
        Value::Bool(true)
    );

    let script = compile_unlinked_script(
        "(function named() { return function() { return function() { return named; }; }; })",
    )
    .unwrap();
    let named = script.constants()[0].as_child().unwrap();
    let relay = named.constants()[0].as_child().unwrap();
    let inner = relay.constants()[0].as_child().unwrap();
    assert_eq!(named.metadata().function_name_local, Some(0));
    assert_eq!(named.metadata().local_count, 1);
    assert_eq!(relay.closure_variables().len(), 1);
    assert_eq!(
        relay.closure_variables()[0].source,
        ClosureSource::ParentLocal(0)
    );
    assert_eq!(
        relay.closure_variables()[0].kind,
        ClosureVariableKind::FunctionName
    );
    let ClosureVariableName::Constant(relay_name) = relay.closure_variables()[0].name else {
        panic!("function-name relay did not retain its source name");
    };
    assert_eq!(
        relay.constants()[usize::try_from(relay_name).unwrap()].as_primitive(),
        Some(&Value::String(JsString::from_static("named")))
    );
    assert!(!relay.closure_variables()[0].is_const);
    assert_eq!(
        inner.closure_variables()[0].source,
        ClosureSource::ParentClosure(0)
    );
    assert_eq!(
        inner.closure_variables()[0].kind,
        ClosureVariableKind::FunctionName
    );
    let ClosureVariableName::Constant(inner_name) = inner.closure_variables()[0].name else {
        panic!("transitive function-name relay did not retain its source name");
    };
    assert_eq!(
        inner.constants()[usize::try_from(inner_name).unwrap()].as_primitive(),
        Some(&Value::String(JsString::from_static("named")))
    );
}

#[test]
fn named_function_self_assignment_matches_quickjs_strict_and_sloppy_rules() {
    assert_eq!(
        evaluate_in_context("(function named() { return named = 1; })()"),
        Value::Int(1)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(f) { return f() === f; })(function named() { named = 1; return named; })"
        ),
        Value::Bool(true)
    );
    // QuickJS carries JS_VAR_FUNCTION_NAME semantics from the defining
    // function through closure relays. A nested strict directive does not
    // turn a sloppy outer function-name binding into a throwing write.
    assert_eq!(
        evaluate_in_context(
            "(function(f) { return f()() === f; })(function named() { return function() { 'use strict'; named = 1; return named; }; })"
        ),
        Value::Bool(true)
    );

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context.eval("(function named() { 'use strict'; named = 1; })()"),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("strict function-name assignment did not materialize TypeError");
    };
    let name = runtime.intern_property_key("name").unwrap();
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.get_property(&error, &name).unwrap(),
        Value::String(JsString::from_static("TypeError"))
    );
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static("'named' is read-only"))
    );

    assert_eq!(
        context.eval("(function named() { 'use strict'; return function() { named = 1; }; })()()"),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("captured strict function-name assignment did not materialize TypeError");
    };
    assert_eq!(
        context.get_property(&error, &name).unwrap(),
        Value::String(JsString::from_static("TypeError"))
    );
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static("'named' is read-only"))
    );

    let strict = compile_unlinked_script(
        "(function named() { 'use strict'; return function() { return named; }; })",
    )
    .unwrap();
    let named = strict.constants()[0].as_child().unwrap();
    let child = named.constants()[1].as_child().unwrap();
    assert_eq!(
        child.closure_variables()[0].kind,
        ClosureVariableKind::FunctionName
    );
    assert!(child.closure_variables()[0].is_const);
    assert!(!child.closure_variables()[0].is_lexical);
}

#[test]
fn nested_capture_installs_parent_closure_relay_and_executes() {
    let source =
        "(function(a) { return function() { return function(b) { return a + b; }; }; })(20)()(22)";
    assert_eq!(evaluate_in_context(source), Value::Int(42));

    let script = compile_unlinked_script(source).unwrap();
    let outer = script.constants()[0].as_child().unwrap();
    let relay = outer.constants()[0].as_child().unwrap();
    let inner = relay.constants()[0].as_child().unwrap();

    assert!(outer.closure_variables().is_empty());
    assert_eq!(relay.closure_variables().len(), 1);
    assert_eq!(
        relay.closure_variables()[0].source,
        ClosureSource::ParentArgument(0)
    );
    assert_eq!(inner.closure_variables().len(), 1);
    assert_eq!(
        inner.closure_variables()[0].source,
        ClosureSource::ParentClosure(0)
    );
}

#[test]
fn function_local_var_capture_uses_parent_local_then_parent_closure() {
    let source = "(function() { var a = 20; return function() { return function(b) { return a + b; }; }; })()()(22)";
    assert_eq!(evaluate_in_context(source), Value::Int(42));

    let script = compile_unlinked_script(source).unwrap();
    let outer = script.constants()[0].as_child().unwrap();
    let relay = outer.constants()[0].as_child().unwrap();
    let inner = relay.constants()[0].as_child().unwrap();
    assert_eq!(outer.metadata().local_count, 1);
    assert_eq!(
        relay.closure_variables()[0].source,
        ClosureSource::ParentLocal(0)
    );
    assert_eq!(
        inner.closure_variables()[0].source,
        ClosureSource::ParentClosure(0)
    );
}

#[test]
fn ordinary_function_fallthrough_returns_undefined() {
    assert_eq!(
        evaluate_in_context("(function(a) { a; })(42)"),
        Value::Undefined
    );
}

#[test]
fn strict_and_escaped_reserved_binding_names_are_rejected_late() {
    for source in [
        "(function(implements) { 'use strict'; return implements; })(1)",
        "'use strict'; (function(let) { return let; })(1)",
        "(function() { 'use strict'; var eval = 1; return eval; })()",
        "(function() { 'use strict'; return impl\\u0065ments; })()",
        "(function(\\u0069f) { return \\u0069f; })(1)",
    ] {
        assert!(
            compile_unlinked_script(source).is_err(),
            "accepted {source:?}"
        );
    }
    assert_eq!(
        evaluate_in_context("(function(implements) { return implements; })(1)"),
        Value::Int(1)
    );
    assert_eq!(
        evaluate_in_context("(function(impl\\u0065ments) { return impl\\u0065ments; })(1)"),
        Value::Int(1)
    );
}

#[test]
fn parser_driven_lexing_preserves_quickjs_error_priority_and_locations() {
    let cases = [
        (
            r"(function(){ var \u0069f\u{}=14; })()",
            "'if' is a reserved identifier",
            1,
            18,
        ),
        (
            r"(function(){ var if\u{}=14; })()",
            "'if' is a reserved identifier",
            1,
            18,
        ),
        (
            r"(function(){ var if\x61=1; })()",
            "variable name expected",
            1,
            18,
        ),
        (
            r"(function(){ var \u{}=1; })()",
            "variable name expected",
            1,
            18,
        ),
        (r"(function(){ var a\u{}=1; })()", "expecting ';'", 1, 19),
        (
            "(function(){ var 'unterminated })()",
            "unexpected end of string",
            1,
            18,
        ),
        (
            "(function(a 'unterminated){})",
            "unexpected end of string",
            1,
            13,
        ),
        (
            "(function(){ return (1 'unterminated); })()",
            "unexpected end of string",
            1,
            24,
        ),
        (
            "(function(eval){ \"use strict\"; \"x\"; \"unterminated })()",
            "unexpected end of string",
            1,
            37,
        ),
        (
            "(function(){ \"use strict\"; (function(eval){ \"x\"; \"unterminated })() })()",
            "unexpected end of string",
            1,
            50,
        ),
    ];

    for (source, message, line, column) in cases {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(error.message(), message, "{source}");
        let span = error
            .span()
            .unwrap_or_else(|| panic!("missing span for {source}"));
        assert_eq!(
            (span.start.line, span.start.column),
            (line, column),
            "{source}"
        );
    }

    let reached_lex_error =
        compile_unlinked_script("(function(){ throw\n'unterminated })()").unwrap_err();
    assert_eq!(reached_lex_error.message(), "unexpected end of string");
    let reached_span = reached_lex_error.span().unwrap();
    assert_eq!((reached_span.start.line, reached_span.start.column), (2, 1));

    let raw_token_error = compile_unlinked_script("(function(){ throw\n\\u{}; })()").unwrap_err();
    assert_eq!(
        raw_token_error.message(),
        "line terminator not allowed after throw"
    );
    let raw_span = raw_token_error.span().unwrap();
    assert_eq!((raw_span.start.line, raw_span.start.column), (2, 1));
}

#[test]
fn primary_expression_slashes_are_rescanned_as_complete_regexp_tokens() {
    // Pinned QuickJS makes this decision in its primary-expression parser:
    // `/` and `/=` are ordinary punctuators until the grammar requires an
    // operand, at which point it rewinds and scans the complete literal.
    for source in [
        "/start/g;",
        "/=prefix/m;",
        "Function.value = /rhs/gi;",
        "(function(){ return /ret/m; })",
        "Function(/argument/s);",
        "true ? /consequent/u : 0;",
        "false ? 0 : /alternate/y;",
        "false || /logical/d;",
        "1 / /denominator/u;",
    ] {
        compile_unlinked_script(source)
            .unwrap_or_else(|error| panic!("RegExp literal {source:?} failed: {error}"));
    }
    let script = compile_unlinked_script("/start/g;").unwrap();
    assert!(matches!(script.code()[0], Instruction::RegExp(0)));

    let invalid_pattern = compile_unlinked_script("/(/").unwrap_err();
    assert_eq!(invalid_pattern.kind(), ErrorKind::Syntax);
    assert_eq!(invalid_pattern.message(), "expecting ')'");
    let span = invalid_pattern.span().expect("literal SyntaxError span");
    assert_eq!((span.start.line, span.start.column), (1, 1));
    assert_eq!((span.start.byte_offset, span.end.byte_offset), (0, 3));

    let invalid_flags = compile_unlinked_script("/a/gg").unwrap_err();
    assert_eq!(invalid_flags.kind(), ErrorKind::Syntax);
    assert_eq!(invalid_flags.message(), "invalid regular expression flags");

    for (source, expected) in [
        ("/a", "unexpected end of regexp"),
        ("/a\n/", "unexpected line terminator in regexp"),
        ("/a\\\n/", "unexpected line terminator in regexp"),
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source:?}");
        assert_eq!(error.message(), expected, "{source:?}");
    }

    compile_unlinked_script("/(?=a)/").expect("forward lookahead literal should compile");
    compile_unlinked_script("/(?<=a)/").expect("backward lookaround literal should compile");
    let unsupported_v = compile_unlinked_script("1 / /denominator/v;").unwrap_err();
    assert_eq!(unsupported_v.kind(), ErrorKind::Unsupported);
    assert!(unsupported_v.message().contains("UnicodeSetOperation"));

    // The same slash tokens remain operators when the expression parser
    // has already produced their left operand.
    assert_eq!(evaluate("84 / 2"), Value::Int(42));
    assert_eq!(
        evaluate_in_context("(function(){ var value=84; value /= 2; return value; })()"),
        Value::Int(42)
    );
}

#[test]
fn implicit_arguments_binding_is_lazy_and_precedes_body_hoists() {
    for source in [
        "(function() { return 1; })",
        "(function() { return delete arguments; })",
        "(function(arguments) { return arguments; })",
    ] {
        let script = compile_unlinked_script(source).unwrap();
        let function = script.constants()[0].as_child().unwrap();
        assert!(
            !function
                .code()
                .iter()
                .any(|op| matches!(op, Instruction::Arguments(_))),
            "unexpected arguments object for {source}"
        );
    }

    for (source, kind) in [
        ("(function() { return arguments; })", ArgumentsKind::Mapped),
        (
            "(function() { 'use strict'; return arguments; })",
            ArgumentsKind::Unmapped,
        ),
        (
            "(function() { var arguments; return typeof arguments; })",
            ArgumentsKind::Mapped,
        ),
    ] {
        let script = compile_unlinked_script(source).unwrap();
        let function = script.constants()[0].as_child().unwrap();
        assert!(matches!(
            function.code(),
            [Instruction::Arguments(actual), Instruction::PutLocal(0), ..]
                if *actual == kind
        ));
        assert_eq!(function.local_definitions().len(), 1, "{source}");
    }

    let script = compile_unlinked_script(
        "(function arguments() { function arguments() {} return arguments; })",
    )
    .unwrap();
    let function = script.constants()[0].as_child().unwrap();
    assert_eq!(function.metadata().function_name_local, None);
    assert!(matches!(
        function.code(),
        [
            Instruction::Arguments(ArgumentsKind::Mapped),
            Instruction::PutLocal(0),
            Instruction::FClosure(0),
            Instruction::PutLocal(0),
            ..
        ]
    ));

    let script = compile_unlinked_script(
        "(function outer() { return function inner() { return arguments; }; })",
    )
    .unwrap();
    let outer = script.constants()[0].as_child().unwrap();
    let inner = outer.constants()[0].as_child().unwrap();
    assert!(
        !outer
            .code()
            .iter()
            .any(|op| matches!(op, Instruction::Arguments(_)))
    );
    assert!(matches!(
        inner.code(),
        [
            Instruction::Arguments(ArgumentsKind::Mapped),
            Instruction::PutLocal(0),
            ..
        ]
    ));

    assert_eq!(
        evaluate_in_context("(function(arguments) { var arguments; return arguments; })(7)"),
        Value::Int(7)
    );
}

#[test]
fn parameter_binding_patterns_publish_quickjs_anonymous_argument_abi() {
    let script =
        compile_unlinked_script("(function(left,[a,b],{c},...[rest]){return left+a+b+c+rest})")
            .unwrap();
    let function = script.constants()[0].as_child().unwrap();
    assert_eq!(function.metadata().argument_count, 3);
    assert_eq!(function.metadata().defined_argument_count, 4);
    assert_eq!(function.metadata().rest_parameter, None);
    assert_eq!(function.metadata().rest_pattern_start, Some(3));
    assert_eq!(function.metadata().parameter_environment_local_count, 0);
    assert_eq!(function.metadata().pattern_argument_count, 2);
    let marker = usize::try_from(
        function
            .metadata()
            .parameter_pattern_end
            .expect("pattern initialization marker"),
    )
    .unwrap();
    assert!(matches!(
        function.code().get(marker),
        Some(Instruction::Nop)
    ));
    assert_eq!(
        function
            .argument_definitions()
            .iter()
            .map(|definition| definition.name.is_some())
            .collect::<Vec<_>>(),
        vec![true, false, false]
    );
    assert_eq!(
        function
            .code()
            .iter()
            .enumerate()
            .filter_map(|(pc, instruction)| match instruction {
                Instruction::GetArg(argument) if pc < marker => Some(*argument),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert!(
        function.code()[..marker]
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Rest(3)))
    );
    assert!(!function.code()[marker + 1..].iter().any(|instruction| {
        matches!(
            instruction,
            Instruction::GetArg(1)
                | Instruction::PutArg(1)
                | Instruction::SetArg(1)
                | Instruction::GetArg(2)
                | Instruction::PutArg(2)
                | Instruction::SetArg(2)
        )
    }));

    let rest_only = compile_unlinked_script("(function(...[a,b]){})").unwrap();
    let rest_only = rest_only.constants()[0].as_child().unwrap();
    assert_eq!(rest_only.metadata().argument_count, 0);
    assert_eq!(rest_only.metadata().defined_argument_count, 1);
    assert_eq!(rest_only.metadata().rest_pattern_start, Some(0));
    assert_eq!(rest_only.metadata().pattern_argument_count, 0);
    assert_eq!(rest_only.argument_definitions(), []);
}

#[test]
fn parameter_binding_patterns_execute_across_sync_function_forms() {
    assert_eq!(
        evaluate_in_context(
            r#"(function(){
                var out=[];
                function declaration([a],{b}){return a+b}
                out.push(declaration([40],{b:2}));
                out.push((function([a]){return a+2})([40]));
                out.push((([a])=>a+2)([40]));
                out.push(({base:40,method([a]){return this.base+a}}).method([2]));
                out.push(Function('[a]','return a+2')([40]));
                var assigned;
                var setter={set value([a]){assigned=a}};
                setter.value=[42];
                out.push(assigned);
                return out.join('|');
            })()"#,
        ),
        Value::String(JsString::from_static("42|42|42|42|42|42"))
    );

    assert_eq!(
        evaluate_in_context(
            r#"(function([a,,[b,...tail]],{x:y,[String('z')]:z,...rest}){
                return a+'|'+b+'|'+tail.join(',')+'|'+y+'|'+z+'|'+rest.extra;
            })([1,0,[2,3,4]],{x:5,z:6,extra:7})"#,
        ),
        Value::String(JsString::from_static("1|2|3,4|5|6|7"))
    );

    assert_eq!(
        evaluate_in_context(
            r#"(function(...[a,b]){
                return a+b+'|'+arguments.length+'|'+(function(...[]){}).length+'|'+
                    (function(...{}){}).length+'|'+(function(...[[]]){}).length+'|'+
                    (function(...[,]){}).length+'|'+(function(...[x]){}).length+'|'+
                    (function(...{x}){}).length+'|'+(function(...[]){var x}).length+'|'+
                    (function(...[]){return arguments}).length+'|'+
                    (function(...[]){return this}).length+'|'+
                    (function(...[]){return new.target}).length+'|'+
                    (function(...[]){return function(){return this}}).length+'|'+
                    (function(...[]){return ()=>this}).length+'|'+
                    (function named(...[]){return named}).length+'|'+
                    (function(x,...[]){}).length+'|'+(function(x,...[y]){}).length+'|'+
                    (function mixed([x],...rest){return x+rest[0]+'|'+mixed.length})([40],2);
            })(40,2)"#,
        ),
        Value::String(JsString::from_static(
            "42|2|0|0|0|0|1|1|1|1|1|1|0|1|1|2|2|42|1"
        ))
    );
}

#[test]
fn parameter_binding_patterns_follow_quickjs_scope_arguments_and_hoist_order() {
    assert_eq!(
        evaluate_in_context(
            r#"(function(){
                var results=[];
                results.push((function([a]){
                    arguments[0]=[9];
                    a=7;
                    return a+'|'+arguments[0][0];
                })([1]));
                results.push((function([a]){var a;return a})([42]));
                results.push((function([a]){
                    function a(){return 42}
                    return typeof a+'|'+a();
                })([1]));
                var key='outer';
                results.push((function({[String(key)]:value}){
                    var key='body';
                    return value;
                })({undefined:42,outer:1}));
                var lexical='outer';
                results.push((function({[lexical]:value}){
                    let lexical='body';
                    return value;
                })({outer:42}));
                results.push((function([a]){return eval('a')})([42]));
                results.push((function({[eval('"key"')]:value}){
                    return value;
                })({key:42}));
                results.push((function({[arguments]:arguments}){
                    return arguments;
                })({undefined:1,"[object Arguments]":42}));
                results.push((function({[(()=>eval('typeof key'))()]:value}){
                    var key='body';
                    return value;
                })({undefined:42}));
                results.push((function(){
                    return (([a])=>a+arguments[1])([40]);
                })(0,2));
                return results.join(';');
            })()"#,
        ),
        Value::String(JsString::from_static(
            "7|9;42;function|42;42;42;42;42;42;42;42"
        ))
    );

    assert_eq!(
        evaluate_in_context(
            r#"(function(){
                var saved;
                function capture(fn){saved=fn;return 'undefined'}
                return (function({[capture(()=>key)]:value}){
                    var key='body';
                    return value+'|'+saved();
                })({undefined:42});
            })()"#,
        ),
        Value::String(JsString::from_static("42|body"))
    );
}

#[test]
fn parameter_binding_pattern_assignment_prescan_matches_quickjs_token_rule() {
    assert_eq!(
        evaluate_in_context(
            "(function(){var key=0;return (function({[(key+=1)]:value}){return value})({1:42})})()",
        ),
        Value::Int(42)
    );

    for source in [
        "(function([a=1]){})",
        "(function([a],b=1){})",
        "(function(a=1,[b]){})",
        "(function({[(key=1)]:value}){})",
        "(function(...[a=1]){})",
        "(([a=1])=>a)",
        "({method({value=1}){}})",
    ] {
        let script = compile_unlinked_script(source)
            .unwrap_or_else(|error| panic!("parameter expression {source:?}: {error}"));
        let function = script.constants()[0].as_child().unwrap();
        assert!(
            function.parameter_environment().is_some(),
            "standalone '=' did not publish a Parameter Environment for {source}"
        );
    }
}

#[test]
fn parameter_assignment_prescan_retains_quickjs_bits_at_the_depth_bound() {
    let source = format!("(a=1,{}0{})", "(".repeat(260), ")".repeat(260));
    let mut lexer = Lexer::new(&source);
    let first = lexer.next_token().unwrap();
    let source_span = first.span;
    let parser = Parser {
        lexer,
        tokens: vec![first],
        cursor: 0,
        current_function: 0,
        in_mode: InMode::Allow,
        functions: vec![
            FunctionIr::new(
                None,
                FunctionKind::Script,
                FunctionSourceInfo {
                    span: source_span,
                    definition: SourceOffset::try_from_usize(0).unwrap(),
                    range: None,
                },
                FunctionIrOptions {
                    function_name: Some("<scan-test>".to_owned()),
                    private_name_binding: false,
                    parameters: Vec::new(),
                    defined_argument_count: 0,
                    has_simple_parameter_list: true,
                    rest_parameter: None,
                    strict: false,
                    super_capabilities: SuperCapabilities::NONE,
                },
            )
            .unwrap(),
        ],
        anonymous_function_definition: None,
    };

    assert_eq!(parser.parenthesized_parameter_has_assignment(), Some(true));
}

#[test]
fn parameter_expression_binding_patterns_publish_the_quickjs_argument_scope_abi() {
    let script = compile_unlinked_script(
        "(function(left,[a,b=left],{c},right=a+b+c){return left+a+b+c+right})",
    )
    .unwrap();
    let function = script.constants()[0].as_child().unwrap();
    let layout = function
        .parameter_environment()
        .expect("standalone '=' creates the parentless argument scope");

    assert_eq!(function.metadata().argument_count, 4);
    assert_eq!(function.metadata().defined_argument_count, 3);
    assert_eq!(function.metadata().parameter_environment_local_count, 5);
    assert_eq!(function.metadata().pattern_argument_count, 2);
    assert_eq!(
        function
            .argument_definitions()
            .iter()
            .map(|definition| definition.name.is_some())
            .collect::<Vec<_>>(),
        vec![true, false, false, true]
    );
    assert_eq!(
        layout
            .argument_cells
            .iter()
            .map(|cell| (cell.argument, cell.parameter_local))
            .collect::<Vec<_>>(),
        vec![(0, 0), (3, 4)]
    );
    assert_eq!(
        layout
            .pattern_copies
            .iter()
            .map(|copy| (copy.parameter_local, copy.body_local))
            .collect::<Vec<_>>(),
        vec![(1, 5), (2, 6), (3, 7)]
    );
    assert_eq!(
        layout.default_sources.as_ref(),
        [ParameterDefaultSource::Argument(3)]
    );
    assert_eq!(
        function.metadata().parameter_pattern_end,
        Some(layout.initialization_end)
    );
}

#[test]
fn parameter_expression_binding_patterns_match_quickjs_scope_copy_and_length_quirks() {
    assert_eq!(
        evaluate_in_context(
            "(function(){var key='outer';return (function({[String(key)]:value},[x=1]){var key='body';return value})({outer:42,undefined:1},[])})()",
        ),
        Value::Int(42)
    );
    assert_eq!(
        evaluate_in_context("(function([a],b=(a=5)){return a})([1])"),
        Value::Int(5)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){var saved;var body=(function([a],f=(saved=()=>a)){var read=()=>a;a=2;return read})([1]);return body()+'|'+saved()})()",
        ),
        Value::String(JsString::from_static("2|1"))
    );
    assert_eq!(
        evaluate_in_context(
            "[(function([a=1],b){}).length,(function([a]=[1],b){}).length,(function(a,[b=1],c){}).length,(function(a,[b]=[1],c){}).length,(function(a,...[b=1]){}).length,(function(a=1,...[b]){}).length].join('|')",
        ),
        Value::String(JsString::from_static("2|0|3|1|2|0"))
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){var source={};return (function({}=source){var source=null;return 42})()})()",
        ),
        Value::Int(42)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){return (function(...[a]=[99]){return String(a)} )()+'|'+(function(...[a]=[99]){return a})(42)+'|'+(function(...[a]=[99]){}).length})()",
        ),
        Value::String(JsString::from_static("undefined|42|0"))
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){var out;({set value([a]=[42]){out=a}}).value=undefined;return out})()",
        ),
        Value::Int(42)
    );
}

#[test]
fn parameter_expression_binding_patterns_compose_identifier_rest_across_surfaces() {
    assert_eq!(
        evaluate_in_context(
            r#"(function(){
                function ordinary([a=1],...rest){return a+'|'+rest.join(',')}
                var arrow=([a=1],...rest)=>a+'|'+rest.join(',');
                var object={method([a=1],...rest){return a+'|'+rest.join(',')}};
                return ordinary([],2,3)+';'+arrow([],2,3)+';'+object.method([],2,3);
            })()"#,
        ),
        Value::String(JsString::from_static("1|2,3;1|2,3;1|2,3"))
    );
    assert_eq!(
        evaluate_in_context(
            r#"(function(){
                function ordinary([a]=[1],...rest){return a+'|'+rest.join(',')}
                var arrow=([a]=[1],...rest)=>a+'|'+rest.join(',');
                var object={method([a]=[1],...rest){return a+'|'+rest.join(',')}};
                return ordinary(undefined,2,3)+';'+arrow(undefined,2,3)+';'+
                    object.method(undefined,2,3);
            })()"#,
        ),
        Value::String(JsString::from_static("1|2,3;1|2,3;1|2,3"))
    );
    assert_eq!(
        evaluate_in_context(
            r#"(function(){
                function later([a],b=1,...rest){return a+'|'+b+'|'+rest.join(',')}
                function earlier(a=0,[b],...rest){return a+'|'+b+'|'+rest.join(',')}
                function empty({},b=1,...rest){return b+'|'+rest.join(',')}
                return later([40],undefined,2,3)+';'+earlier(undefined,[40],2,3)+';'+
                    empty({},undefined,2,3);
            })()"#,
        ),
        Value::String(JsString::from_static("40|1|2,3;0|40|2,3;1|2,3"))
    );
}

#[test]
fn parameter_expression_binding_arguments_and_duplicate_order_match_quickjs() {
    let script =
        compile_unlinked_script("(function({arguments=1}){var arguments;return arguments})")
            .unwrap();
    let function = script.constants()[0].as_child().unwrap();
    assert!(function.parameter_environment().is_some());
    assert!(
        function
            .code()
            .iter()
            .all(|instruction| !matches!(instruction, Instruction::Arguments(_)))
    );
    assert_eq!(
        evaluate_in_context("(function({arguments=1}){var arguments;return arguments})({})"),
        Value::Int(1)
    );

    for source in [
        "function f(a,a=1){'use strict'}",
        "({m(a,a=1){'use strict'}})",
        "(a,a=1)=>{'use strict'}",
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(
            error.message(),
            "duplicate parameter names not allowed in this context",
            "{source}"
        );
    }
}

#[test]
fn parameter_expression_binding_cells_and_body_copies_survive_gc_independently() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context
        .eval(
            "var initializerRead,bodyRead;bodyRead=(function([a],f=(initializerRead=()=>a)){var read=()=>a;a=2;return read})([1])",
        )
        .unwrap();
    runtime.run_gc().unwrap();
    assert_eq!(
        context.eval("bodyRead()+'|'+initializerRead()").unwrap(),
        Value::String(JsString::from_static("2|1"))
    );
}

#[test]
fn identifier_rest_parameters_publish_quickjs_length_and_entry_order() {
    let script = compile_unlinked_script(
        "(function(left,...rest){ function rest(){ return 42; } return arguments; })",
    )
    .unwrap();
    let function = script.constants()[0].as_child().unwrap();
    assert_eq!(function.metadata().argument_count, 2);
    assert_eq!(function.metadata().defined_argument_count, 1);
    assert!(matches!(
        function.code(),
        [
            Instruction::Arguments(ArgumentsKind::Unmapped),
            Instruction::PutLocal(0),
            Instruction::Rest(1),
            Instruction::PutArg(1),
            Instruction::FClosure(0),
            Instruction::PutArg(1),
            ..
        ]
    ));

    let script = compile_unlinked_script("({method(left,...rest){return rest}})").unwrap();
    let method = script.constants()[0].as_child().unwrap();
    assert_eq!(method.metadata().argument_count, 2);
    assert_eq!(method.metadata().defined_argument_count, 1);
    assert!(matches!(
        method.code(),
        [Instruction::Rest(1), Instruction::PutArg(1), ..]
    ));

    let script = compile_unlinked_script("(left,...rest)=>rest").unwrap();
    let arrow = script.constants()[0].as_child().unwrap();
    assert_eq!(arrow.metadata().argument_count, 2);
    assert_eq!(arrow.metadata().defined_argument_count, 1);
    assert!(matches!(
        arrow.code(),
        [Instruction::Rest(1), Instruction::PutArg(1), ..]
    ));
}

#[test]
fn identifier_rest_parameters_execute_across_sync_function_forms() {
    assert_eq!(
        evaluate_in_context(
            r#"(function(){
                function ordinary(left,...rest){
                    return left+'|'+rest.length+'|'+rest[0]+'|'+rest[1]+'|'+Array.isArray(rest);
                }
                var arrow=(left,...rest)=>left+'|'+rest.length+'|'+rest[0]+'|'+rest[1];
                var object={base:40,method(left,...rest){
                    return this.base+'|'+left+'|'+rest.length+'|'+rest[0]+'|'+rest[1];
                }};
                var dynamic=Function('left','...rest','return left+rest[0]+rest[1]');
                return ordinary(40,1,2)+';'+arrow(40,1,2)+';'+object.method(1,1,2)+';'+
                    dynamic(40,1,1)+';'+ordinary.length+'|'+arrow.length+'|'+
                    object.method.length+'|'+dynamic.length;
            })()"#,
        ),
        Value::String(JsString::from_static(
            "40|2|1|2|true;40|2|1|2;40|1|2|1|2;42;1|1|1|1"
        ))
    );

    assert_eq!(
        evaluate_in_context(
            r#"(function(left,...rest){
                arguments[0]=7;
                arguments[1]=8;
                var first=left+'|'+rest[0];
                left=9;
                rest[0]=10;
                return first+'|'+left+'|'+rest[0]+'|'+arguments[0]+'|'+arguments[1];
            })(1,2)"#,
        ),
        Value::String(JsString::from_static("1|2|9|10|7|8"))
    );

    assert_eq!(
        evaluate_in_context(
            r#"(function(){
                var retained=(function(...rest){var rest;return Array.isArray(rest)+'|'+rest.length})();
                var hoisted=(function(...rest){function rest(){return 42}return typeof rest+'|'+rest()})();
                var capture=(function(...rest){return function(){return rest[0]+rest[1]}})(40,2);
                return retained+';'+hoisted+';'+capture();
            })()"#,
        ),
        Value::String(JsString::from_static("true|0;function|42;42"))
    );
}

#[test]
fn identifier_default_parameters_execute_across_sync_function_forms() {
    assert_eq!(
        evaluate_in_context(
            r#"(function(){
                function ordinary(a=40,b=a+2){return b}
                var arrow=(a=40,b=a+2)=>b;
                var object={base:40,method(a=this.base,b=a+2){return b}};
                var dynamic=Function('a=40','b=a+2','return b');
                return ordinary()+'|'+arrow()+'|'+object.method()+'|'+dynamic()+'|'+
                    ordinary.length+'|'+arrow.length+'|'+object.method.length+'|'+dynamic.length;
            })()"#,
        ),
        Value::String(JsString::from_static("42|42|42|42|0|0|0|0"))
    );
}

#[test]
fn identifier_default_before_rest_uses_the_parameter_environment_abi() {
    let script = compile_unlinked_script("function f(a=40,...rest){return a+rest.length}").unwrap();
    let function = script.constants()[0].as_child().unwrap();
    assert_eq!(function.metadata().argument_count, 2);
    assert_eq!(function.metadata().defined_argument_count, 0);
    assert_eq!(function.metadata().rest_parameter, Some(1));
    assert_eq!(function.metadata().parameter_environment_local_count, 2);
    assert!(matches!(
        function.code(),
        [
            Instruction::SetLocalUninitialized(1),
            Instruction::SetLocalUninitialized(0),
            ..
        ]
    ));
    assert!(function.code().windows(4).any(|window| matches!(
        window,
        [
            Instruction::Rest(1),
            Instruction::Dup,
            Instruction::PutArg(1),
            Instruction::InitializeLocal(1),
        ]
    )));

    assert_eq!(
        evaluate_in_context("(function(a=40,...rest){return a+rest.length})(undefined,1,1)"),
        Value::Int(42)
    );
}

#[test]
fn identifier_default_parameter_environment_matches_quickjs_tdz_and_body_split() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    for source in [
        "(function(a=b,b=2){return a})()",
        "(function(a=a){return a})()",
        "((a=b,b=2)=>a)()",
        "({method(a=a){return a}}).method()",
    ] {
        let (name, _) = evaluate_error(&runtime, &mut context, source);
        assert_eq!(name, JsString::from_static("ReferenceError"), "{source}");
    }

    assert_eq!(
        evaluate_in_context(
            r#"(function(){
                var read;
                function f(a=1,b=(read=function(){return a},0)){
                    a=2;
                    return a+'|'+read();
                }
                return f();
            })()"#,
        ),
        // Pinned QuickJS keeps body reads on the raw argument slot while the
        // initializer closure retains the lexical parameter cell.
        Value::String(JsString::from_static("2|1"))
    );
}

#[test]
fn identifier_default_parameter_uses_private_function_name_before_body_shadowing() {
    assert_eq!(
        evaluate_in_context(
            r#"(function(){
                var read=(function f(a=f){var f;return typeof a+'|'+(a===undefined)+'|'+typeof f})();
                var closure=(function f(a=()=>f){var f=1;return typeof a()+'|'+(a()===f)})();
                var write=(function f(a=(f=1)){var f;return typeof f+'|'+(f===1)})();
                return read+';'+closure+';'+write;
            })()"#,
        ),
        Value::String(JsString::from_static(
            "function|false|undefined;function|false;undefined|false"
        ))
    );

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let (name, message) = evaluate_error(
        &runtime,
        &mut context,
        "(function(){'use strict';return (function f(a=(f=1)){var f;return typeof f})()})()",
    );
    assert_eq!(name, JsString::from_static("TypeError"));
    assert_eq!(message, JsString::from_static("'f' is read-only"));
}

#[test]
fn identifier_default_parameters_run_before_body_hoists_and_use_unmapped_arguments() {
    assert_eq!(
        evaluate_in_context(
            r#"(function(){
                var outer=40;
                function f(a=outer,b=arguments[0]){
                    var outer=1;
                    function a(){return 42}
                    arguments[0]=7;
                    return a()+'|'+b+'|'+arguments.length;
                }
                return f(undefined);
            })()"#,
        ),
        Value::String(JsString::from_static("42|undefined|1"))
    );
}

#[test]
fn identifier_rest_array_is_allocated_in_the_callee_realm() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let Value::Object(function) = defining
        .eval("(function(...rest){return Object.getPrototypeOf(rest)===Array.prototype})")
        .unwrap()
    else {
        panic!("rest source did not produce a function");
    };
    let function = runtime.as_callable(&function).unwrap().unwrap();
    assert_eq!(
        caller
            .call(
                &function,
                Value::Undefined,
                &[Value::Int(40), Value::Int(2)]
            )
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn identifier_rest_parameter_early_errors_match_quickjs_policy() {
    for source in [
        "function f(...rest,next){}",
        "function f(...rest,){}",
        "function f(...rest=[]){}",
        "function f(value,...value,){}",
        "(...rest,next)=>0",
        "(...rest,)=>0",
        "(...rest=[])=>0",
        "(value,...value,)=>0",
        "({method(value,...value,){}})",
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(error.message(), "expecting ')'", "{source}");
    }

    for source in [
        "function f(value,value,...rest){}",
        "(value,value,...rest)=>0",
        "({method(value,value,...rest){}})",
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(
            error.message(),
            "duplicate argument names not allowed in this context",
            "{source}"
        );
    }

    for source in [
        "function f(...rest){'use strict';}",
        "function f(value,...value){'use strict';}",
        "(...rest)=>{'use strict';}",
        "(value,...value)=>{'use strict';}",
        "({method(value,...value){'use strict';}})",
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(
            error.message(),
            "\"use strict\" not allowed in function with default or destructuring parameter",
            "{source}"
        );
    }

    for source in ["({get value(...rest){}})", "({set value(...rest){}})"] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(
            error.message(),
            "invalid number of arguments for getter or setter",
            "{source}"
        );
    }

    compile_unlinked_script("function f(value,value){}")
        .expect("a sloppy ordinary simple parameter list may contain duplicates");
    compile_unlinked_script("'use strict';function f(...rest){}")
        .expect("inherited strictness does not make a rest parameter directive invalid");
}

#[test]
fn var_initializer_named_evaluation_follows_quickjs_set_name_marker() {
    for source in [
        "(function() { var f = function() {}; return f; })()",
        "(function() { var f = (((function() {}))); return f; })()",
        "(function() { var \\u0066 = function() {}; return f; })()",
        "(function() { var f; f = function() {}; return f; })()",
    ] {
        assert_eq!(
            evaluate_function_name(source),
            (JsString::from_static("f"), false, false, true),
            "direct anonymous initializer should inherit the binding name: {source}"
        );
    }

    for source in [
        "(function() { return function() {}; })()",
        "(function() { var f = (0, function() {}); return f; })()",
        "(function() { var f = true ? function() {} : function() {}; return f; })()",
        "(function() { var f = 0 || function() {}; return f; })()",
    ] {
        assert_eq!(
            evaluate_function_name(source),
            (JsString::from_static(""), false, false, true),
            "non-AnonymousFunctionDefinition expression must keep an empty name: {source}"
        );
    }
}

#[test]
fn new_and_new_target_follow_quickjs_base_constructor_semantics() {
    assert_eq!(
        evaluate_in_context(
            "(function(){ var F = function(){ return this; }; return typeof new F(); })()"
        ),
        Value::String(JsString::from_static("object"))
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){ var F = function(){ return new.target; }; return new F() === F; })()"
        ),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){ var marker = function(){}; var F = function(a){ return a; }; return new F(marker) === marker; })()"
        ),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){ var F = function(){ return 1; }; return typeof new F; })()"
        ),
        Value::String(JsString::from_static("object"))
    );
    assert_eq!(
        evaluate_in_context("(function(){ return new.target; })()"),
        Value::Undefined
    );

    let error = compile_unlinked_script("new.target").unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Syntax);
    assert_eq!(error.message(), "new.target only allowed within functions");

    for source in [
        r"(function(){ return new.\u0074arget; })()",
        r"(function(){ return new.t\u0061rget; })()",
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax);
        assert_eq!(error.message(), "expecting target");
    }
}

#[test]
fn quickjs_argument_slot_limit_uses_catchable_internal_error() {
    let parameters = std::iter::repeat_n("a", MAX_LOCAL_VARIABLES + 1)
        .collect::<Vec<_>>()
        .join(",");
    let source = format!("(function({parameters}) {{}})");
    let error = compile_unlinked_script(&source).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::JsInternal);
    assert_eq!(error.message(), "too many arguments");

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(context.compile(&source), Err(RuntimeError::Exception));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("argument overflow must materialize InternalError");
    };
    let name = runtime.intern_property_key("name").unwrap();
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.get_property(&error, &name).unwrap(),
        Value::String(JsString::from_static("InternalError"))
    );
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static("too many arguments"))
    );
}

#[test]
fn quickjs_call_argument_boundary_materializes_stack_overflow() {
    let arguments = std::iter::repeat_n("0", MAX_CALL_ARGUMENTS)
        .collect::<Vec<_>>()
        .join(",");
    let source = format!("(function() {{}})({arguments})");
    let error = compile_unlinked_script(&source).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::JsInternal);
    assert_eq!(error.message(), "stack overflow");

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(context.compile(&source), Err(RuntimeError::Exception));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("bytecode stack overflow must materialize InternalError");
    };
    let name = runtime.intern_property_key("name").unwrap();
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.get_property(&error, &name).unwrap(),
        Value::String(JsString::from_static("InternalError"))
    );
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static("stack overflow"))
    );

    let mut too_many = source.clone();
    let closing_parenthesis = too_many
        .rfind(')')
        .expect("generated call expression has a closing parenthesis");
    too_many.insert_str(closing_parenthesis, ",0");
    let error = compile_unlinked_script(&too_many).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Syntax);
    assert_eq!(error.message(), "Too many call arguments");

    let unreachable = format!("(function(){{ return 1; {source}; }})");
    compile_unlinked_script(&unreachable).unwrap();
}

#[test]
fn quickjs_template_stack_overflow_is_deferred_until_after_parsing() {
    // Each substitution is one concat argument; the kept receiver and
    // method let 65,532 arguments exactly reach JS_STACK_SIZE_MAX.
    let largest_valid = "${0}".repeat(MAX_BYTECODE_STACK - 2);
    let largest_valid = compile_unlinked_script(&format!("`{largest_valid}`")).unwrap();
    assert_eq!(
        largest_valid.metadata().max_stack,
        MAX_BYTECODE_STACK as u16
    );

    // One more argument exceeds the limit without passing through the
    // ordinary call parser's argument guard.
    let substitutions = "${0}".repeat(MAX_BYTECODE_STACK - 1);
    let source = format!("`{substitutions}`");
    let error = compile_unlinked_script(&source).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::JsInternal);
    assert_eq!(error.message(), "stack overflow");

    // QuickJS computes the bytecode stack only after parsing the whole
    // function, so a later reached lexical error has priority.
    let later_lexical_error = format!("{source}; \"unterminated");
    let error = compile_unlinked_script(&later_lexical_error).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Syntax);
    assert_eq!(error.message(), "unexpected end of string");

    let later_parser_error = format!("{source} 0");
    let error = compile_unlinked_script(&later_parser_error).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Syntax);
    assert_eq!(error.message(), "expecting ';'");

    // QuickJS computes stack depth over reachable bytecode PCs. The same
    // oversized call after a terminal return is encoded but ignored by
    // the control-flow walk.
    let unreachable = format!("(function(){{ return 1; {source}; }})");
    compile_unlinked_script(&unreachable).unwrap();

    // Once argc no longer fits u16, QuickJS encodes its low bits. A live
    // path has already crossed the stack cap before that call, while dead
    // bytecode remains valid and must not be diagnosed from the truncated
    // operand's residual stack effect.
    let wrapped_substitutions = "${0}".repeat(usize::from(u16::MAX) + 1);
    let wrapped = format!("`{wrapped_substitutions}`");
    let error = compile_unlinked_script(&wrapped).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::JsInternal);
    assert_eq!(error.message(), "stack overflow");
    let unreachable = format!("(function(){{ return 1; {wrapped}; }})");
    compile_unlinked_script(&unreachable).unwrap();
}

#[test]
fn quickjs_closure_slot_limit_is_65534_and_uses_internal_error() {
    let span = Span::new(Position::new(0, 1, 1), Position::new(0, 1, 1));
    let mut function = FunctionIr::new(
        None,
        FunctionKind::Ordinary,
        FunctionSourceInfo {
            span,
            definition: SourceOffset::try_from_usize(0).unwrap(),
            range: None,
        },
        FunctionIrOptions {
            function_name: None,
            private_name_binding: false,
            parameters: Vec::new(),
            defined_argument_count: 0,
            has_simple_parameter_list: true,
            rest_parameter: None,
            strict: false,
            super_capabilities: SuperCapabilities::NONE,
        },
    )
    .unwrap();
    function.closure_variables = (0..MAX_LOCAL_VARIABLES - 1)
        .map(|index| ClosureVariable {
            source: ClosureSource::ParentLocal(
                u16::try_from(index).expect("test index is below the QuickJS slot limit"),
            ),
            name: ClosureVariableName::None,
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        })
        .collect();

    assert_eq!(
        ensure_closure_variable(
            &mut function,
            ClosureVariable {
                source: ClosureSource::ParentArgument(0),
                name: ClosureVariableName::None,
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            },
        )
        .unwrap(),
        65_533
    );
    let error = ensure_closure_variable(
        &mut function,
        ClosureVariable {
            source: ClosureSource::ParentArgument(1),
            name: ClosureVariableName::None,
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::JsInternal);
    assert_eq!(error.message(), "too many closure variables");
}

#[test]
fn compiles_throw_as_a_terminal_completion_and_enforces_no_line_terminator() {
    let bytecode = compile_script("throw 9").unwrap();
    assert!(
        bytecode
            .code
            .iter()
            .any(|instruction| matches!(instruction, crate::bytecode::Instruction::Throw))
    );
    assert!(compile_script("throw\n9").is_err());
}

#[test]
fn try_catch_lowering_keeps_parameter_and_body_scopes_distinct() {
    let source = r#"
        try { throw 1; }
        catch (e) { let x = e; function f(){ return e + x; } }
    "#;
    let tree = Parser::parse(source, JsString::from_static("<try-scope-test>")).unwrap();
    let root = &tree.functions[0];
    let catch_scope = root
        .scopes
        .iter()
        .position(|scope| scope.kind == ScopeKind::Catch)
        .map(super::ScopeId)
        .unwrap();
    let catch_body_scope = root
        .scopes
        .iter()
        .enumerate()
        .find(|(_, scope)| scope.kind == ScopeKind::Block && scope.parent == Some(catch_scope))
        .map(|(index, _)| super::ScopeId(index))
        .unwrap();
    let catch_binding = root.binding_in_scope(catch_scope, "e").unwrap();
    let BindingStorage::Local(catch_local) = catch_binding.storage else {
        panic!("catch parameter did not use local storage");
    };
    assert!(catch_binding.is_catch_parameter);
    assert_eq!(catch_binding.kind, BindingKind::Lexical { is_const: false });
    assert!(root.binding_in_scope(catch_body_scope, "x").is_some());
    assert!(root.binding_in_scope(catch_body_scope, "f").is_some());
    assert_eq!(
        tree.functions
            .iter()
            .find(|function| function.function_name.as_deref() == Some("f"))
            .and_then(|function| function.parent)
            .map(|parent| parent.definition_scope),
        Some(catch_body_scope)
    );

    let bytecode = compile_unlinked_script(source).unwrap();
    assert_eq!(
        bytecode
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::Catch(_)))
            .count(),
        2
    );
    assert!(bytecode.code().iter().any(
        |instruction| matches!(instruction, Instruction::SetLocalUninitialized(index) if *index == catch_local)
    ));
    assert!(bytecode.code().iter().any(
        |instruction| matches!(instruction, Instruction::CloseLocal(index) if *index == catch_local)
    ));
    assert!(
        bytecode
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Ret))
    );
}

#[test]
fn catch_binding_conflicts_and_var_initializer_follow_quickjs() {
    for (source, keyword) in [("catch (e) {}", "catch"), ("finally {}", "finally")] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(
            error.message(),
            format!("unexpected token in expression: '{keyword}'"),
            "{source}"
        );
    }
    let extra_catch = compile_unlinked_script("try {} finally {} catch (e) {}").unwrap_err();
    assert_eq!(
        extra_catch.message(),
        "unexpected token in expression: 'catch'"
    );

    for source in [
        "try {} catch (e) { let e; }",
        "try {} catch (e) { function e(){} }",
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(
            error.message(),
            "invalid redefinition of lexical identifier",
            "{source}"
        );
    }
    compile_unlinked_script("try {} catch (e) { { let e = 1; e; } }").unwrap();

    let source = "try { throw 1; } catch (e) { var e = e + 1; e; }";
    let tree = Parser::parse(source, JsString::from_static("<catch-var-test>")).unwrap();
    let catch_local = tree.functions[0]
        .bindings
        .iter()
        .find(|binding| binding.is_catch_parameter)
        .and_then(|binding| match binding.storage {
            BindingStorage::Local(index) => Some(index),
            _ => None,
        })
        .unwrap();
    let bytecode = compile_unlinked_script(source).unwrap();
    assert!(bytecode.code().iter().any(
        |instruction| matches!(instruction, Instruction::GetLocalCheck(index) if *index == catch_local)
    ));
    assert!(bytecode.code().iter().any(
        |instruction| matches!(instruction, Instruction::PutLocalCheck(index) if *index == catch_local)
    ));
    assert_eq!(evaluate_in_context(source), Value::Int(2));

    let strict_source = "\"use strict\"; try {} catch (eval) {}";
    let strict_error = compile_unlinked_script(strict_source).unwrap_err();
    assert_eq!(
        strict_error.message(),
        "invalid variable name in strict mode"
    );
    assert_eq!(
        strict_error.span().unwrap().start.column,
        u32::try_from(strict_source.find(')').unwrap() + 1).unwrap()
    );
}

#[test]
fn catch_binding_patterns_compile_with_ordinary_lexical_provenance() {
    for source in [
        "try { throw [1, 2]; } catch ([first, second]) { first + second; }",
        "try { throw { value: 42 }; } catch ({value}) { value; }",
        "try { throw { nested: [42] }; } catch ({nested: [value]}) { value; }",
        "try { throw [1, 2, 3]; } catch ([head, ...tail]) { head + tail.length; }",
        "try { throw { value: 1, extra: 2 }; } catch ({value, ...rest}) { value + rest.extra; }",
        "try { throw []; } catch ([value = function () {}]) { value.name; }",
    ] {
        compile_unlinked_script(source).unwrap_or_else(|error| {
            panic!("catch binding pattern did not compile: {source}: {error}")
        });
    }

    let source = concat!(
        "try { throw { value: 1, nested: [], extra: 2 }; } ",
        "catch ({value, nested: [fallback = 40], ...rest}) { ",
        "value + fallback + rest.extra; }",
    );
    let tree = Parser::parse(source, JsString::from_static("<catch-pattern-scope-test>"))
        .expect("nested catch binding pattern should parse");
    let root = &tree.functions[0];
    let catch_scope = root
        .scopes
        .iter()
        .position(|scope| scope.kind == ScopeKind::Catch)
        .map(ScopeId)
        .expect("catch pattern lost its scope");

    for name in ["value", "fallback", "rest"] {
        let binding = root
            .binding_in_scope(catch_scope, name)
            .unwrap_or_else(|| panic!("catch pattern lost binding {name}"));
        assert_eq!(binding.storage_scope, catch_scope, "{name}");
        assert_eq!(binding.declaration_scope, catch_scope, "{name}");
        assert_eq!(
            binding.kind,
            BindingKind::Lexical { is_const: false },
            "{name}"
        );
        assert!(
            !binding.is_catch_parameter,
            "pattern leaf {name} incorrectly received the simple-catch marker"
        );
    }

    let simple = Parser::parse(
        "try { throw 1; } catch (value) { value; }",
        JsString::from_static("<simple-catch-scope-test>"),
    )
    .expect("simple catch binding should parse");
    let simple_root = &simple.functions[0];
    let simple_scope = simple_root
        .scopes
        .iter()
        .position(|scope| scope.kind == ScopeKind::Catch)
        .map(ScopeId)
        .expect("simple catch binding lost its scope");
    assert!(
        simple_root
            .binding_in_scope(simple_scope, "value")
            .expect("simple catch binding was not registered")
            .is_catch_parameter,
        "only a simple catch binding receives the Annex-B marker"
    );

    let catch_scope_entry = root
        .ops
        .iter()
        .position(|operation| {
            matches!(operation.op, super::IrOp::EnterScope(scope) if scope == catch_scope)
        })
        .expect("catch scope has no EnterScope marker");
    let handler_target = root
        .ops
        .iter()
        .find_map(|operation| match &operation.op {
            super::IrOp::Bytecode(Instruction::Catch(target)) => {
                Some(usize::try_from(*target).expect("catch target fits usize"))
            }
            _ => None,
        })
        .expect("try statement has no Catch handler");
    assert!(
        catch_scope_entry < handler_target,
        "the exceptional handler must skip QuickJS's pre-label Catch EnterScope"
    );
    assert!(
        matches!(
            root.ops.get(handler_target).map(|operation| &operation.op),
            Some(super::IrOp::PrepareCatchScope(scope)) if *scope == catch_scope
        ),
        "the Catch target must land on its default-undefined preparation"
    );
}

#[test]
fn catch_binding_pattern_diagnostics_follow_quickjs() {
    for source in [
        "try {} catch ([value, value]) {}",
        "try {} catch ({first: value, second: value}) {}",
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(
            error.message(),
            "invalid redefinition of lexical identifier",
            "{source}"
        );
    }

    for source in [
        "\"use strict\"; try {} catch ({eval}) {}",
        "\"use strict\"; try {} catch ([arguments]) {}",
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(error.message(), "invalid destructuring target", "{source}");
    }
}

#[test]
fn direct_eval_profile_distinguishes_pattern_and_simple_catch_bindings() {
    let direct = |is_catch_parameter| {
        EvalCompileContext::direct_with_profile(
            false,
            vec![EvalRootBinding {
                name: JsString::from_static("value"),
                scope: 0,
                is_lexical: true,
                is_const: false,
                kind: ClosureVariableKind::Normal,
                is_catch_parameter,
            }],
            EvalCallerProfile {
                scope_kinds: vec![EvalScopeKind::Catch].into_boxed_slice(),
                variable_target: EvalCallerVariableTarget::Global,
            },
            false,
            false,
        )
    };

    let read = compile_unlinked_eval_with_filename(
        "value",
        "<catch-pattern-eval>",
        DebugInfoMode::StripDebug,
        direct(false),
    )
    .expect("direct eval should read an imported catch-pattern lexical");
    let [descriptor] = read.closure_variables() else {
        panic!("catch-pattern eval did not retain exactly one imported binding");
    };
    assert_eq!(descriptor.source, ClosureSource::EvalEnvironment(0));
    assert!(descriptor.is_lexical);
    assert!(!descriptor.is_const);
    assert!(
        read.code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetVarRefCheck(0))),
        "catch-pattern eval read lost its lexical TDZ check"
    );

    let pattern_var = compile_unlinked_eval_with_filename(
        "var value;",
        "<catch-pattern-eval>",
        DebugInfoMode::StripDebug,
        direct(false),
    )
    .expect("the caller lexical conflict is represented by entry bytecode");
    assert!(
        matches!(pattern_var.code(), [Instruction::ThrowRedeclaration(_), ..]),
        "a pattern catch lexical must reject sloppy eval var redeclaration"
    );

    let simple_var = compile_unlinked_eval_with_filename(
        "var value;",
        "<simple-catch-eval>",
        DebugInfoMode::StripDebug,
        direct(true),
    )
    .expect("a simple catch binding should retain QuickJS's var exception");
    assert!(
        !simple_var
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::ThrowRedeclaration(_))),
        "the simple-catch marker did not preserve the sloppy eval var exception"
    );
}

#[test]
fn for_in_of_array_bindings_use_nested_iterator_records() {
    let for_of = compile_unlinked_script("for(const [a,,b,] of [[1,2,3]])a+b").unwrap();
    assert_eq!(
        for_of
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::ForOfStart))
            .count(),
        2
    );
    assert_eq!(
        for_of
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::ForOfNext(0)))
            .count(),
        4
    );
    assert_eq!(
        for_of
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::IteratorClose))
            .count(),
        2
    );
    assert_eq!(
        for_of
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::InitializeLocal(_)))
            .count(),
        2
    );

    let for_in = compile_unlinked_script("for(var [a,b] in {ab:1})a+b").unwrap();
    assert_eq!(
        for_in
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::ForInStart))
            .count(),
        1
    );
    assert_eq!(
        for_in
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::ForOfStart))
            .count(),
        1
    );
    assert_eq!(
        for_in
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::ForOfNext(1)))
            .count(),
        2
    );

    for source in [
        "for(var [a]=[1] in {a:1})a",
        "for(let [a]=[1] in {a:1})a",
        "for(const [a]=[1] of [[1]])a",
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(
            error.message(),
            format!(
                "a declaration in the head of a for-{} loop can't have an initializer",
                if source.contains(" of ") { "of" } else { "in" }
            ),
            "{source}"
        );
    }
}

#[test]
fn object_binding_declarations_cover_direct_and_loop_surfaces() {
    for (source, expected) in [
        (
            "(function(){var {fixed}={fixed:'v'};let {['computed']:computed}={computed:'l'};const {nested:{value}}={nested:{value:'c'}};return fixed+computed+value})()",
            "vlc",
        ),
        (
            "(function(){var result='';for(var {fixed}={fixed:'v'};;){result+=fixed;break}for(let {['computed']:computed}={computed:'l'};;){result+=computed;break}for(const {nested:{value}}={nested:{value:'c'}};;){result+=value;break}return result})()",
            "vlc",
        ),
        (
            "(function(){var result='';for(var {0:first} in {ab:1})result+=first;for(let {[1]:second} in {ab:1})result+=second;for(const {constructor:{name}} in {ab:1})result+=name;return result})()",
            "abString",
        ),
        (
            "(function(){var result='';for(var {fixed} of [{fixed:'v'}])result+=fixed;for(let {['computed']:computed} of [{computed:'l'}])result+=computed;for(const {nested:{value}} of [{nested:{value:'c'}}])result+=value;return result})()",
            "vlc",
        ),
    ] {
        assert_eq!(
            evaluate_in_context(source),
            Value::String(JsString::from_static(expected)),
            "{source}"
        );
    }
}

#[test]
fn nested_finally_abrupt_edges_use_typed_cleanup_and_shared_subroutines() {
    let source = r#"
        (function f(){
            outer: while (1) {
                try {
                    try { return 1; }
                    finally { break outer; }
                } finally { return 3; }
            }
            return 4;
        })()
    "#;
    let bytecode = compile_unlinked_script(source).unwrap();
    let function = bytecode
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .unwrap();
    assert_eq!(
        function
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::Catch(_)))
            .count(),
        2
    );
    assert_eq!(
        function
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::Ret))
            .count(),
        2
    );
    assert!(
        function
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::NipCatch))
            .count()
            >= 2
    );
    assert!(
        function
            .code()
            .windows(2)
            .any(|window| matches!(window, [Instruction::DropGosub, Instruction::Drop]))
    );
    assert_eq!(evaluate_in_context(source), Value::Int(3));
}

#[test]
fn script_finally_saves_and_normally_restores_eval_completion() {
    let source = "1; try { 2; } finally { 3; }";
    let bytecode = compile_unlinked_script(source).unwrap();
    assert_eq!(bytecode.local_definitions().len(), 2);
    assert!(
        bytecode
            .local_definitions()
            .iter()
            .all(|definition| definition.name.is_none() && !definition.is_lexical)
    );
    assert!(bytecode.code().windows(4).any(|window| matches!(
        window,
        [
            Instruction::GetLocal(0),
            Instruction::PutLocal(1),
            Instruction::Undefined,
            Instruction::PutLocal(0)
        ]
    )));
    assert!(bytecode.code().windows(3).any(|window| matches!(
        window,
        [
            Instruction::GetLocal(1),
            Instruction::PutLocal(0),
            Instruction::Ret
        ]
    )));
    assert_eq!(evaluate_in_context(source), Value::Int(2));
    assert_eq!(
        evaluate_in_context("try { throw 1; } catch { 4; } finally { 5; }"),
        Value::Int(4)
    );
}

#[test]
fn compiles_bigint_literals_without_a_fixed_width_limit() {
    let expected = JsBigInt::parse_radix("10000000000000000000000000000000000000000", 16).unwrap();
    assert_eq!(
        evaluate("0x10000000000000000000000000000000000000000n"),
        Value::BigInt(expected)
    );
}

#[test]
fn use_strict_directive_rejects_legacy_literals() {
    assert!(compile_script("'use strict'; 010").is_err());
    assert!(compile_script("'use strict'; '\\1'").is_err());
    assert!(compile_script("'\\1'; 'use strict'; 0").is_err());
    assert!(compile_script("'\\8'; 'use strict'; 0").is_err());
    assert!(compile_script("; 'use strict'; 010").is_ok());
    assert!(compile_script("'use\\x20strict'; 010").is_ok());
    assert!(compile_script("'not strict'\n'use strict'\n010").is_err());
    assert!(compile_script("'not strict'\n+ 'use strict'; 010").is_ok());
    assert!(compile_script("'use strict' + ''; 010").is_ok());
    assert!(compile_script("'use strict'\n!0; 010").is_ok());
    assert!(compile_script("'use strict'\nvoid 0; 010").is_ok());
}

#[test]
fn unlinked_script_preserves_strict_mode_metadata() {
    let strict = compile_unlinked_script("'use strict'; 0").unwrap();
    let sloppy = compile_unlinked_script("'use\\x20strict'; 0").unwrap();

    assert!(strict.metadata().strict);
    assert!(!sloppy.metadata().strict);
}

#[test]
fn unlinked_script_preserves_verified_maximum_stack() {
    let source = "'left' + (0.5 * 2.5)";
    let bytecode = compile_script(source).unwrap();
    let verified = bytecode.verify().unwrap();
    let unlinked = compile_unlinked_script(source).unwrap();

    assert_eq!(bytecode.max_stack, 3);
    assert_eq!(unlinked.metadata().max_stack, bytecode.max_stack);
    assert_eq!(unlinked.metadata().max_stack, verified.max_stack);
}

#[test]
fn unlinked_script_converts_every_compiled_constant_to_a_primitive() {
    let source = "'\\ud800x'; 3.5; 0x100000000000000000000000000000000n";
    let bytecode = compile_script(source).unwrap();
    let unlinked = compile_unlinked_script(source).unwrap();

    assert_eq!(bytecode.constants.len(), 3);
    assert_eq!(unlinked.constants().len(), bytecode.constants.len());
    for (constant, expected) in unlinked.constants().iter().zip(&bytecode.constants) {
        assert_eq!(constant.as_primitive(), Some(expected));
        assert!(constant.as_child().is_none());
    }
}

#[test]
fn detached_string_literals_follow_quickjs_atom_identity_boundaries() {
    let atomized = compile_script("['same','same']").unwrap();
    let Value::String(first) = &atomized.constants[0] else {
        panic!("first atomized literal was not a String");
    };
    let Value::String(second) = &atomized.constants[1] else {
        panic!("second atomized literal was not a String");
    };
    assert!(first.same_representation(second));

    let immediate = compile_script("['2147483647','2147483647']").unwrap();
    let Value::String(first) = &immediate.constants[0] else {
        panic!("first immediate-atom literal was not a String");
    };
    let Value::String(second) = &immediate.constants[1] else {
        panic!("second immediate-atom literal was not a String");
    };
    assert!(!first.same_representation(second));

    let table_backed = compile_script("['2147483648','2147483648']").unwrap();
    let Value::String(first) = &table_backed.constants[0] else {
        panic!("first table-backed numeric literal was not a String");
    };
    let Value::String(second) = &table_backed.constants[1] else {
        panic!("second table-backed numeric literal was not a String");
    };
    assert!(first.same_representation(second));
}

#[test]
fn object_literals_lower_quickjs_data_proto_computed_and_spread_paths() {
    let fixed = compile_unlinked_script("({a:1,if:2,'x':3,0x10:4,a:5})").unwrap();
    assert!(
        fixed
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Object))
    );
    assert_eq!(
        fixed
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::DefineField(_)))
            .count(),
        5
    );

    let shorthand = compile_unlinked_script("var value=1;({value})").unwrap();
    assert!(
        shorthand
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::DefineField(_)))
    );

    let computed = compile_unlinked_script("({[1]:function(){}})").unwrap();
    assert!(computed.code().windows(4).any(|window| matches!(
        window,
        [
            Instruction::FClosure(_),
            Instruction::SetNameComputed,
            Instruction::DefineArrayEl,
            Instruction::Drop
        ]
    )));
    assert!(
        computed
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::ToPropKey))
    );

    let proto = compile_unlinked_script("({__proto__:null})").unwrap();
    assert!(
        proto
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::SetProto))
    );
    assert!(
        !proto
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::DefineField(_)))
    );

    let spread = compile_unlinked_script("({a:1,...value,b:2})").unwrap();
    assert!(
        spread
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::CopyDataProperties))
    );
}

#[test]
fn object_literal_methods_lower_fixed_and_computed_non_constructors() {
    let script = compile_unlinked_script("({a(value,){return value},['b'](){return this}})")
        .expect("concise methods compile");
    assert!(script.code().iter().any(|instruction| matches!(
        instruction,
        Instruction::DefineMethod {
            kind: DefineMethodKind::Method,
            enumerable: true,
            ..
        }
    )));
    assert!(script.code().iter().any(|instruction| matches!(
        instruction,
        Instruction::DefineMethodComputed {
            kind: DefineMethodKind::Method,
            enumerable: true,
        }
    )));

    let methods = script
        .constants()
        .iter()
        .filter_map(|constant| constant.as_child())
        .collect::<Vec<_>>();
    assert_eq!(methods.len(), 2);
    assert_eq!(methods[0].metadata().argument_count, 1);
    for method in methods {
        assert!(!method.metadata().has_prototype);
        assert_eq!(method.metadata().constructor_kind, ConstructorKind::None);
    }
}

#[test]
fn object_literal_accessors_lower_fixed_and_computed_zero_one_arity_functions() {
    let script = compile_unlinked_script(
        "({get fixed(){return this},set fixed(value,){this.value=value},get ['computed'](){return arguments.length},set [1](value){return typeof new.target}})",
    )
    .expect("object literal accessors compile");

    let mut fixed_kinds = Vec::new();
    let mut computed_kinds = Vec::new();
    for instruction in script.code() {
        match instruction {
            Instruction::DefineMethod {
                kind, enumerable, ..
            } => {
                assert!(*enumerable);
                fixed_kinds.push(*kind);
            }
            Instruction::DefineMethodComputed { kind, enumerable } => {
                assert!(*enumerable);
                computed_kinds.push(*kind);
            }
            _ => {}
        }
    }
    assert_eq!(
        fixed_kinds,
        [DefineMethodKind::Getter, DefineMethodKind::Setter]
    );
    assert_eq!(
        computed_kinds,
        [DefineMethodKind::Getter, DefineMethodKind::Setter]
    );
    assert_eq!(
        script
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::ToPropKey))
            .count(),
        2
    );

    let accessors = script
        .constants()
        .iter()
        .filter_map(|constant| constant.as_child())
        .collect::<Vec<_>>();
    assert_eq!(accessors.len(), 4);
    assert_eq!(
        accessors
            .iter()
            .map(|accessor| accessor.metadata().argument_count)
            .collect::<Vec<_>>(),
        [0, 1, 0, 1]
    );
    for accessor in accessors {
        assert!(!accessor.metadata().has_prototype);
        assert_eq!(accessor.metadata().constructor_kind, ConstructorKind::None);
    }
}

#[test]
fn object_literal_super_references_authenticate_home_object_and_quickjs_stack_forms() {
    let script = compile_unlinked_script(
        "({get read(){return super.value},set write(value){super.value=value},call(key){return super[key]()},update(){return super.value++},remove(){return delete super.value},iterate(values){for(super.value of values){}}})",
    )
    .expect("direct object-method super properties compile");
    let methods = script
        .constants()
        .iter()
        .filter_map(|constant| constant.as_child())
        .collect::<Vec<_>>();
    assert_eq!(methods.len(), 6);
    assert!(
        methods
            .iter()
            .all(|method| method.metadata().needs_home_object)
    );
    assert!(methods.iter().all(|method| {
        method.code().windows(4).any(|window| {
            matches!(
                window,
                [
                    Instruction::PushHomeObject,
                    Instruction::PutLocal(_),
                    Instruction::PushThis,
                    Instruction::PutLocal(_),
                ]
            )
        })
    }));
    assert!(methods.iter().all(|method| {
        method.code().windows(3).any(|window| {
            matches!(
                window,
                [
                    Instruction::GetLocal(_),
                    Instruction::GetLocal(_),
                    Instruction::GetSuper
                ]
            )
        })
    }));
    assert!(
        methods[0]
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetSuperValue))
    );
    assert!(
        methods[1]
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::PutSuperValue))
    );
    assert!(
        methods[2]
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetSuperValueForCall))
    );
    assert!(
        methods[3]
            .code()
            .windows(2)
            .any(|window| matches!(window, [Instruction::Perm5, Instruction::PutSuperValue]))
    );
    assert!(
        methods[4]
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::ThrowDeleteSuper))
    );
    assert!(
        methods[5]
            .code()
            .windows(2)
            .any(|window| matches!(window, [Instruction::Rot4Left, Instruction::PutSuperValue]))
    );
}

#[test]
fn object_literal_arrow_super_relays_lexical_this_and_home_object() {
    let script = compile_unlinked_script(
        "({method(){return()=>()=>{super['value'];super.call();super.value=1;super.value+=1;super.value||=2;++super.value;super.value++;delete super.value}}})",
    )
    .expect("nested arrows inherit ObjectLiteral super properties");
    let method = script
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("script lost its object method");
    let relay = method
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("method lost its first arrow");
    let inner = relay
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("first arrow lost its nested arrow");

    assert!(method.metadata().needs_home_object);
    assert!(method.metadata().super_allowed);
    assert!(!method.metadata().super_call_allowed);
    assert!(!relay.metadata().needs_home_object);
    assert!(relay.metadata().super_allowed);
    assert!(!relay.metadata().super_call_allowed);
    assert!(!inner.metadata().needs_home_object);
    assert!(inner.metadata().super_allowed);
    assert!(!inner.metadata().super_call_allowed);
    assert_eq!(method.metadata().local_count, 2);
    assert_eq!(relay.metadata().local_count, 0);
    assert_eq!(inner.metadata().local_count, 0);

    let [
        Instruction::PushHomeObject,
        Instruction::PutLocal(home_object),
        Instruction::PushThis,
        Instruction::PutLocal(this_value),
        ..,
    ] = method.code()
    else {
        panic!("method pseudo-binding prologue did not match QuickJS order");
    };
    assert_ne!(home_object, this_value);
    assert_eq!(relay.closure_variables().len(), 2);
    assert!(relay.closure_variables().iter().any(|descriptor| {
        descriptor.source == ClosureSource::ParentLocal(*this_value)
            && descriptor.kind == ClosureVariableKind::Normal
            && descriptor.name == ClosureVariableName::None
    }));
    assert!(relay.closure_variables().iter().any(|descriptor| {
        descriptor.source == ClosureSource::ParentLocal(*home_object)
            && descriptor.kind == ClosureVariableKind::Normal
            && descriptor.name == ClosureVariableName::None
    }));
    assert_eq!(inner.closure_variables().len(), 2);
    assert!(inner.closure_variables().iter().any(|descriptor| {
        descriptor.source == ClosureSource::ParentClosure(0)
            && descriptor.kind == ClosureVariableKind::Normal
            && descriptor.name == ClosureVariableName::None
    }));
    assert!(inner.closure_variables().iter().any(|descriptor| {
        descriptor.source == ClosureSource::ParentClosure(1)
            && descriptor.kind == ClosureVariableKind::Normal
            && descriptor.name == ClosureVariableName::None
    }));
    assert!(
        inner
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetSuperValueForCall))
    );
    assert!(
        inner
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::PutSuperValue))
    );
    assert!(
        inner
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::ThrowDeleteSuper))
    );

    assert_eq!(
        evaluate_in_context(
            r#"
                (function () {
                    var log = "";
                    var proto = {
                        method(addend) { log += "c"; return this.count + addend; },
                        get value() { log += "g"; return this.count; },
                        set value(input) { log += "s"; this.count = input; }
                    };
                    var home = {
                        __proto__: proto,
                        count: 38,
                        make(key) {
                            return () => () => {
                                var read = super[key];
                                var call = super.method(2);
                                var assigned = super.value = 39;
                                var compound = super.value += 1;
                                var logical = super.value ||= 99;
                                var post = super.value++;
                                var pre = ++super.value;
                                var deleted;
                                try { delete super.value; }
                                catch (error) { deleted = error.name; }
                                return [
                                    read, call, assigned, compound, logical,
                                    post, pre, this.count, deleted, log
                                ].join("|");
                            };
                        }
                    };
                    var relay = home.make("value").call({ count: 100 });
                    return relay.call({ count: 200 });
                })()
            "#,
        ),
        Value::String(JsString::from_static(
            "38|40|39|40|40|40|42|42|ReferenceError|gcsgsggsgs"
        ))
    );

    let error =
        compile_unlinked_script("({method(){return()=>function ordinary(){return super.value}}})")
            .expect_err("ordinary functions must truncate inherited super capability");
    assert_eq!(error.kind(), ErrorKind::Syntax);
    assert_eq!(error.message(), "'super' is only valid in a method");
}

#[test]
fn object_literal_direct_eval_super_relays_authenticated_home_object() {
    let script = compile_unlinked_script(
        "({method(){return eval('super.value')},get read(){return eval('super.value')},set write(value){eval('super.value=value')},arrow(){return()=>eval('super.value')}})",
    )
    .expect("ObjectLiteral methods with direct eval SuperProperty compile");
    let methods = script
        .constants()
        .iter()
        .filter_map(|constant| constant.as_child())
        .collect::<Vec<_>>();
    assert_eq!(methods.len(), 4);
    assert!(!script.metadata().super_allowed);
    assert!(!script.metadata().super_call_allowed);
    assert!(methods.iter().all(|method| method.metadata().super_allowed));
    assert!(
        methods
            .iter()
            .all(|method| !method.metadata().super_call_allowed)
    );
    assert!(
        methods
            .iter()
            .all(|method| method.metadata().needs_home_object)
    );
    for method in &methods {
        let home_entries = method
            .code()
            .iter()
            .enumerate()
            .filter_map(|(index, instruction)| {
                matches!(instruction, Instruction::PushHomeObject).then_some(index)
            })
            .collect::<Vec<_>>();
        let this_entries = method
            .code()
            .iter()
            .enumerate()
            .filter_map(|(index, instruction)| {
                matches!(instruction, Instruction::PushThis).then_some(index)
            })
            .collect::<Vec<_>>();
        let ([home_entry], [this_entry]) = (home_entries.as_slice(), this_entries.as_slice())
        else {
            panic!("method did not have unique HomeObject/this entry operations");
        };
        assert!(home_entry < this_entry);
        let Instruction::PutLocal(home_local) = method.code()[home_entry + 1] else {
            panic!("HomeObject entry did not initialize its local");
        };
        let Instruction::PutLocal(this_local) = method.code()[this_entry + 1] else {
            panic!("this entry did not initialize its local");
        };
        assert_ne!(home_local, this_local);
    }

    for direct_eval_owner in &methods[..3] {
        let environment = &direct_eval_owner.eval_environments()[0];
        assert!(environment.super_allowed);
        assert!(!environment.super_call_allowed);
        let function_root = environment
            .scopes
            .iter()
            .find(|scope| scope.kind == EvalScopeKind::FunctionRoot)
            .expect("method direct eval lost its function root");
        for expected in [THIS_LOCAL_NAME, HOME_OBJECT_LOCAL_NAME] {
            assert!(
                function_root
                    .bindings
                    .iter()
                    .any(|binding| binding.name == JsString::from_static(expected)),
                "method direct eval lost {expected}",
            );
        }
    }
    let arrow = methods[3]
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("method lost its eval-owning arrow");
    assert!(arrow.metadata().super_allowed);
    assert!(!arrow.metadata().super_call_allowed);
    let named_parent_local = |expected: &'static str| {
        arrow
            .closure_variables()
            .iter()
            .find(|descriptor| {
                let ClosureVariableName::Constant(name) = descriptor.name else {
                    return false;
                };
                matches!(descriptor.source, ClosureSource::ParentLocal(_))
                    && matches!(
                        arrow.constants()[name as usize].as_primitive(),
                        Some(Value::String(name)) if name == &JsString::from_static(expected)
                    )
            })
            .expect("eval-visible arrow lost its named pseudo-binding relay")
    };
    let this_relay = named_parent_local(THIS_LOCAL_NAME);
    let home_relay = named_parent_local(HOME_OBJECT_LOCAL_NAME);
    assert_ne!(this_relay.source, home_relay.source);
    let arrow_environment = &arrow.eval_environments()[0];
    assert!(arrow_environment.super_allowed);
    assert!(!arrow_environment.super_call_allowed);
    let arrow_owner = arrow_environment
        .scopes
        .iter()
        .find(|scope| {
            scope.kind == EvalScopeKind::FunctionRoot
                && scope
                    .bindings
                    .iter()
                    .any(|binding| binding.name == JsString::from_static(HOME_OBJECT_LOCAL_NAME))
        })
        .expect("arrow direct eval lost its method owner");
    for expected in [THIS_LOCAL_NAME, HOME_OBJECT_LOCAL_NAME] {
        assert!(
            arrow_owner
                .bindings
                .iter()
                .any(|binding| binding.name == JsString::from_static(expected)),
            "arrow direct eval lost {expected}",
        );
    }

    let cutoff_script = compile_unlinked_script(
        "({method(){super.value;return function ordinary(){return eval('super.value')}}})",
    )
    .expect("ordinary direct-eval cutoff compiles before runtime String parsing");
    let cutoff_method = cutoff_script
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("cutoff script lost its method");
    let cutoff_ordinary = cutoff_method
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("cutoff method lost its ordinary function");
    assert!(cutoff_method.metadata().super_allowed);
    assert!(!cutoff_ordinary.metadata().super_allowed);
    assert!(!cutoff_ordinary.metadata().super_call_allowed);
    assert!(!cutoff_ordinary.eval_environments()[0].super_allowed);
    assert!(!cutoff_ordinary.eval_environments()[0].super_call_allowed);

    assert_eq!(
        evaluate_in_context(
            r#"
                (function () {
                    var proto = {
                        get value() { return this.base + 2; },
                        set value(input) { this.base = input - 2; },
                        call() { return this.base + 2; }
                    };
                    var home = {
                        __proto__: proto,
                        base: 40,
                        method() { return eval("super.value"); },
                        get read() { return eval("super.value"); },
                        set write(value) { eval("super.value = value"); },
                        arrow() { return (() => eval("super.call()"))(); },
                        escaped() { return eval("() => super.value"); },
                        nested() { return eval("eval('super.value')"); },
                        cutoff() {
                            super.value;
                            return function ordinary() { return eval("super.value"); };
                        },
                        indirect() { return (0, eval)("super.value"); },
                        superCall() { return eval("super(sideEffect = true)"); }
                    };
                    var initial = home.method();
                    var getter = home.read;
                    home.write = 44;
                    var setter = home.base;
                    var arrow = home.arrow();
                    var receiver = { base: 40 };
                    var escaped = home.escaped.call(receiver)();
                    var nested = home.nested.call(receiver);
                    var cutoff;
                    try { home.cutoff()(); } catch (error) { cutoff = error.name; }
                    var globalEval;
                    try { eval("super.value"); } catch (error) { globalEval = error.name; }
                    var indirect;
                    try { home.indirect(); } catch (error) { indirect = error.name; }
                    var sideEffect = false;
                    var superCall;
                    try { home.superCall(); } catch (error) { superCall = error.name; }
                    return [
                        initial, getter, setter, arrow, escaped, nested,
                        cutoff, globalEval, indirect, superCall, sideEffect
                    ].join("|");
                })()
            "#,
        ),
        Value::String(JsString::from_static(
            "42|42|42|44|42|42|SyntaxError|SyntaxError|SyntaxError|SyntaxError|false"
        ))
    );
}

#[test]
fn object_literal_grammar_is_fail_closed_at_remaining_method_frontiers() {
    for source in [
        "({})",
        "({a:1,})",
        "({a})",
        "({let})",
        "({['x']:2})",
        "({...null})",
        "({__proto__:null,a:1})",
        "({a(){}})",
        "({a(...rest){return rest}})",
        "({a(value=1){}})",
        "({a({value}){}})",
        "({get(){}})",
        "({set(value){}})",
        "({[1](){}})",
        "({get a(){}})",
        "({set a(value){}})",
        "({set a(value=1){}})",
        "({set a({value}){}})",
        "({get ['a'](){}})",
        "({set [1](value,){}})",
        "({get get(){}})",
        "({set set(value){}})",
        "({get\nlineBreak(){},set\nlineBreak(value){}})",
        "({g\\u0065t(){},s\\u0065t(value){}})",
    ] {
        compile_unlinked_script(source)
            .unwrap_or_else(|error| panic!("valid Object literal {source:?}: {error}"));
    }

    for source in ["({*a(){}})", "({async a(){}})"] {
        assert!(
            compile_unlinked_script(source)
                .unwrap_err()
                .message()
                .contains("not implemented yet"),
            "method frontier was not explicit for {source}"
        );
    }
    for source in [
        "({get a(value){}})",
        "({get a(value=1){}})",
        "({get a({value}){}})",
        "({get a(...rest){}})",
        "({set a(){}})",
        "({set a(...rest){}})",
        "({set a(left,right){}})",
        "({set a(left=1,right){}})",
        "({set a({left},right){}})",
        "({set a([left],right){}})",
        "({set a(value,value){}})",
    ] {
        assert_eq!(
            compile_unlinked_script(source).unwrap_err().message(),
            "invalid number of arguments for getter or setter",
            "accessor arity did not match QuickJS for {source}"
        );
    }
    for source in ["({get a(1){}})", "({set a(1,2){}})"] {
        assert_eq!(
            compile_unlinked_script(source).unwrap_err().message(),
            "missing formal parameter",
            "malformed parameters must fail before the accessor arity check for {source}"
        );
    }
    for source in [
        "({get a(){'use strict';public=42}})",
        "'use strict';({set a(value){public=42}})",
    ] {
        assert_eq!(
            compile_unlinked_script(source).unwrap_err().message(),
            "unexpected token in expression: 'public'",
            "strict future-reserved references must remain SyntaxError for {source}"
        );
    }
    assert_eq!(
        compile_unlinked_script("({a(value,value){}})")
            .unwrap_err()
            .message(),
        "duplicate argument names not allowed in this context"
    );
    assert_eq!(
        compile_unlinked_script("({__proto__:null,__proto__:{}})")
            .unwrap_err()
            .message(),
        "duplicate __proto__ property name"
    );
    assert_eq!(
        compile_unlinked_script("({get=1})").unwrap_err().message(),
        "expecting '}'"
    );
    assert_eq!(
        compile_unlinked_script("({#private:1})")
            .unwrap_err()
            .message(),
        "private identifiers are not valid in object literals"
    );
}

#[test]
fn object_literal_runtime_preserves_descriptors_proto_names_and_pinned_spread() {
    assert_eq!(
        evaluate_in_context(
            "(function(){var x=3;var o={2:'two',a:1,x,a:4};var d=Object.getOwnPropertyDescriptor(o,'a');return o[2]+'|'+o.x+'|'+o.a+'|'+d.writable+'|'+d.enumerable+'|'+d.configurable})()"
        ),
        Value::String(JsString::from_static("two|3|4|true|true|true"))
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){var p={marker:7};var a={__proto__:p};var b={__proto__:1};var c={['__proto__']:9};return a.marker+'|'+Object.hasOwn(a,'__proto__')+'|'+(Object.getPrototypeOf(b)===Object.prototype)+'|'+c.__proto__})()"
        ),
        Value::String(JsString::from_static("7|false|true|9"))
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){var s=Symbol('key');var a={[s]:function(){},plain:function(){}};return a[s].name+'|'+a.plain.name+'|'+Object.hasOwn({...\"ab\"},'0')})()"
        ),
        Value::String(JsString::from_static("[key]|plain|false"))
    );
}

#[test]
fn array_literals_lower_dense_fixed_hole_and_spread_phases() {
    let dense = compile_unlinked_script("[1,2,3]").unwrap();
    assert!(
        dense
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::ArrayFrom(3)))
    );
    assert!(!dense.code().iter().any(|instruction| matches!(
        instruction,
        Instruction::DefineField(_) | Instruction::DefineArrayEl | Instruction::Append
    )));

    let large_source = format!(
        "[{}]",
        (0..33)
            .map(|value| value.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    let large = compile_unlinked_script(&large_source).unwrap();
    assert!(
        large
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::ArrayFrom(32)))
    );
    let fixed_key = large
        .code()
        .iter()
        .find_map(|instruction| match instruction {
            Instruction::DefineField(index) => Some(*index),
            _ => None,
        })
        .expect("33rd Array element must use DefineField");
    assert!(matches!(
        large.constants()[usize::try_from(fixed_key).unwrap()].as_primitive(),
        Some(Value::String(value)) if value == &JsString::from_static("32")
    ));

    let holes = compile_unlinked_script("[,1,,]").unwrap();
    let hole_code = holes.code();
    assert!(
        hole_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::ArrayFrom(0)))
    );
    assert!(
        hole_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::DefineField(_)))
    );
    assert!(hole_code.windows(3).any(|window| matches!(
        window,
        [
            Instruction::Dup,
            Instruction::PushI32(3),
            Instruction::PutField(_)
        ]
    )));

    let spread = compile_unlinked_script("[1,...'ab',,4]").unwrap();
    assert!(
        spread
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Append))
    );
    assert!(spread.code().windows(3).any(|window| matches!(
        window,
        [
            Instruction::DefineArrayEl,
            Instruction::Inc,
            Instruction::Drop
        ]
    )));
}

#[test]
fn array_literal_grammar_keeps_quickjs_boundaries_and_reference_state() {
    for source in [
        "[]",
        "[,]",
        "[,,]",
        "[1,]",
        "[...'',]",
        "for([1 in Function];false;);",
    ] {
        compile_unlinked_script(source)
            .unwrap_or_else(|error| panic!("valid Array literal {source:?}: {error}"));
    }
    assert_eq!(
        compile_unlinked_script("[1 2]").unwrap_err().message(),
        "expecting ']'"
    );
    compile_unlinked_script("[/a/]").unwrap();
    for source in ["[... ]", "[1,, 2 3]"] {
        assert!(
            compile_unlinked_script(source).is_err(),
            "invalid Array literal unexpectedly compiled: {source}"
        );
    }

    let named = compile_unlinked_script("var named=[function(){}]").unwrap();
    assert!(
        !named
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::SetName(_)))
    );
    let child = named
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("Array element function child");
    assert_eq!(child.func_name(), None);
}

#[test]
fn debug_metadata_tracks_operator_tail_call_and_root_call_sites() {
    let source = "(function outer(){ return (function inner(){ return 1n + 1; })(); })()";
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let root = context.compile_with_filename(source, "<cmdline>").unwrap();
    let outer = runtime.test_child_function_bytecode(&root, 0).unwrap();
    let inner = runtime.test_child_function_bytecode(&outer, 0).unwrap();

    let root_code = runtime.test_function_code(&root).unwrap();
    let outer_code = runtime.test_function_code(&outer).unwrap();
    let inner_code = runtime.test_function_code(&inner).unwrap();
    let root_call = root_code
        .iter()
        .rposition(|instruction| matches!(instruction, Instruction::Call(0)))
        .unwrap();
    let outer_call = outer_code
        .iter()
        .rposition(|instruction| matches!(instruction, Instruction::Call(0)))
        .unwrap();
    let inner_add = inner_code
        .iter()
        .position(|instruction| matches!(instruction, Instruction::Add))
        .unwrap();

    assert_eq!(
        runtime
            .test_function_debug_location(&inner, Some(inner_add))
            .unwrap(),
        Some((
            JsString::from_static("<cmdline>"),
            crate::LineColumn::new(0, 55)
        ))
    );
    assert_eq!(
        runtime
            .test_function_debug_location(&outer, Some(outer_call))
            .unwrap(),
        Some((
            JsString::from_static("<cmdline>"),
            crate::LineColumn::new(0, 19)
        ))
    );
    assert_eq!(
        runtime
            .test_function_debug_location(&root, Some(root_call))
            .unwrap(),
        Some((
            JsString::from_static("<cmdline>"),
            crate::LineColumn::new(0, 68)
        ))
    );
    assert_eq!(runtime.test_function_debug_source(&root).unwrap(), None);
    assert_eq!(
        runtime.test_function_debug_source(&outer).unwrap(),
        Some(b"function outer(){ return (function inner(){ return 1n + 1; })(); }".to_vec())
    );
    assert_eq!(
        runtime.test_function_debug_source(&inner).unwrap(),
        Some(b"function inner(){ return 1n + 1; }".to_vec())
    );
}

#[test]
fn ordinary_assignment_inherits_last_rhs_marker_and_var_initializer_marks_equal() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let assignment_source = "\"use strict\"; missing = 1";
    let root = context
        .compile_with_filename(assignment_source, "globals.js")
        .unwrap();
    let code = runtime.test_function_code(&root).unwrap();
    let dup = code
        .iter()
        .position(|instruction| matches!(instruction, Instruction::Dup))
        .unwrap();
    assert!(matches!(code.get(dup + 1), Some(Instruction::PutVar(_))));
    let lhs = u32::try_from(assignment_source.find("missing").unwrap()).unwrap();
    let expected = Some((
        JsString::from_static("globals.js"),
        crate::LineColumn::new(0, lhs),
    ));
    assert_eq!(
        runtime
            .test_function_debug_location(&root, Some(dup))
            .unwrap(),
        expected
    );
    assert_eq!(
        runtime
            .test_function_debug_location(&root, Some(dup + 1))
            .unwrap(),
        expected
    );

    let identifier_rhs_source = "(function(){ \"use strict\"; var y=1; missing = y; })";
    let identifier_rhs_root = context
        .compile_with_filename(identifier_rhs_source, "globals.js")
        .unwrap();
    let identifier_rhs = runtime
        .test_child_function_bytecode(&identifier_rhs_root, 0)
        .unwrap();
    let identifier_rhs_code = runtime.test_function_code(&identifier_rhs).unwrap();
    let identifier_put = identifier_rhs_code
        .iter()
        .position(|instruction| matches!(instruction, Instruction::PutVar(_)))
        .unwrap();
    let rhs_identifier = u32::try_from(identifier_rhs_source.rfind("y;").unwrap()).unwrap();
    assert_eq!(
        runtime
            .test_function_debug_location(&identifier_rhs, Some(identifier_put))
            .unwrap(),
        Some((
            JsString::from_static("globals.js"),
            crate::LineColumn::new(0, rhs_identifier)
        ))
    );

    let operator_rhs_source = "\"use strict\"; missing = 1 + 2";
    let operator_rhs = context
        .compile_with_filename(operator_rhs_source, "globals.js")
        .unwrap();
    let operator_rhs_code = runtime.test_function_code(&operator_rhs).unwrap();
    let operator_put = operator_rhs_code
        .iter()
        .position(|instruction| matches!(instruction, Instruction::PutVar(_)))
        .unwrap();
    let plus = u32::try_from(operator_rhs_source.find('+').unwrap()).unwrap();
    assert_eq!(
        runtime
            .test_function_debug_location(&operator_rhs, Some(operator_put))
            .unwrap(),
        Some((
            JsString::from_static("globals.js"),
            crate::LineColumn::new(0, plus)
        ))
    );

    let declaration_source = "(function(){ var x = 1; return x; })";
    let declaration_root = context
        .compile_with_filename(declaration_source, "globals.js")
        .unwrap();
    let declaration = runtime
        .test_child_function_bytecode(&declaration_root, 0)
        .unwrap();
    let declaration_code = runtime.test_function_code(&declaration).unwrap();
    let put_local = declaration_code
        .iter()
        .position(|instruction| matches!(instruction, Instruction::PutLocal(_)))
        .unwrap();
    let equal = u32::try_from(declaration_source.find("= 1").unwrap()).unwrap();
    assert_eq!(
        runtime
            .test_function_debug_location(&declaration, Some(put_local))
            .unwrap(),
        Some((
            JsString::from_static("globals.js"),
            crate::LineColumn::new(0, equal)
        ))
    );
}

#[test]
fn call_and_construct_debug_sites_follow_quickjs_tokens() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    let call_source = "Error()";
    let call_root = context
        .compile_with_filename(call_source, "calls.js")
        .unwrap();
    let call_code = runtime.test_function_code(&call_root).unwrap();
    let call_pc = call_code
        .iter()
        .position(|instruction| matches!(instruction, Instruction::Call(0)))
        .unwrap();
    assert_eq!(
        runtime
            .test_function_debug_location(&call_root, Some(call_pc))
            .unwrap(),
        Some((
            JsString::from_static("calls.js"),
            crate::LineColumn::new(0, 5)
        ))
    );

    let construct_source = "(function f(){ return new Error('x'); })";
    let construct_root = context
        .compile_with_filename(construct_source, "construct.js")
        .unwrap();
    let constructor = runtime
        .test_child_function_bytecode(&construct_root, 0)
        .unwrap();
    let constructor_code = runtime.test_function_code(&constructor).unwrap();
    let construct_pc = constructor_code
        .iter()
        .position(|instruction| matches!(instruction, Instruction::Construct(1)))
        .unwrap();
    let left_paren = construct_source.find("Error(").unwrap() + "Error".len();
    assert_eq!(
        runtime
            .test_function_debug_location(&constructor, Some(construct_pc))
            .unwrap(),
        Some((
            JsString::from_static("construct.js"),
            crate::LineColumn::new(0, u32::try_from(left_paren).unwrap())
        ))
    );

    let no_parens_source = "(function f(){ return new Error; })";
    let no_parens_root = context
        .compile_with_filename(no_parens_source, "construct.js")
        .unwrap();
    let no_parens_constructor = runtime
        .test_child_function_bytecode(&no_parens_root, 0)
        .unwrap();
    let no_parens_code = runtime.test_function_code(&no_parens_constructor).unwrap();
    let no_parens_pc = no_parens_code
        .iter()
        .position(|instruction| matches!(instruction, Instruction::Construct(0)))
        .unwrap();
    let semicolon = no_parens_source.find("Error;").unwrap() + "Error".len();
    assert_eq!(
        runtime
            .test_function_debug_location(&no_parens_constructor, Some(no_parens_pc))
            .unwrap(),
        Some((
            JsString::from_static("construct.js"),
            crate::LineColumn::new(0, u32::try_from(semicolon).unwrap())
        ))
    );
}

#[test]
fn primitive_and_function_primaries_do_not_emit_source_markers() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    for source in [
        "(1)",
        "('x')",
        "(null)",
        "(true)",
        "(this)",
        "(function(){})",
        "(!1)",
        "(void 1)",
        "(typeof 1)",
        "(true && false)",
        "(1 ? 2 : 3)",
        "(1, 2)",
    ] {
        let root = context.compile_with_filename(source, "primary.js").unwrap();
        for pc in 0..runtime.test_function_code(&root).unwrap().len() {
            assert_eq!(
                runtime
                    .test_function_debug_location(&root, Some(pc))
                    .unwrap(),
                Some((
                    JsString::from_static("primary.js"),
                    crate::LineColumn::new(0, 0)
                )),
                "source: {source}, pc: {pc}"
            );
        }
    }
}

#[test]
fn root_and_ordinary_function_names_stay_distinct() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let root = context.compile("(function(){ return 1; })").unwrap();
    let child = runtime.test_child_function_bytecode(&root, 0).unwrap();

    assert_eq!(
        runtime.test_function_name(&root).unwrap(),
        Some(JsString::from_static("<eval>"))
    );
    assert_eq!(runtime.test_function_name(&child).unwrap(), None);
    assert_eq!(
        runtime.test_function_debug_location(&root, None).unwrap(),
        Some((
            JsString::from_static(super::DEFAULT_EVAL_FILENAME),
            crate::LineColumn::new(0, 0)
        ))
    );
}

#[test]
fn filename_atom_ownership_counts_every_function_and_same_atom_use() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline_atoms = runtime.test_atom_count();
    let root = context
        .compile_with_filename("(function(){ return same; })", "same")
        .unwrap();
    let child = runtime.test_child_function_bytecode(&root, 0).unwrap();

    assert_eq!(
        runtime.test_debug_filename_atom_ownership(&root).unwrap(),
        Some((2, Some(4)))
    );
    assert_eq!(
        runtime.test_debug_filename_atom_ownership(&child).unwrap(),
        Some((2, Some(4)))
    );
    drop(root);
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 1);
    assert_eq!(
        runtime.test_function_debug_source(&child).unwrap(),
        Some(b"function(){ return same; }".to_vec())
    );
    drop(child);
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
    assert_eq!(runtime.test_atom_count(), baseline_atoms);
}

#[test]
fn runtime_strip_mode_controls_debug_payload_and_filename_atom_ownership() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline_atoms = runtime.test_atom_count();

    runtime.set_debug_info_mode(DebugInfoMode::StripSource);
    let root = context
        .compile_with_filename("(function(){})", "strip-source-unique.js")
        .unwrap();
    let child = runtime.test_child_function_bytecode(&root, 0).unwrap();
    assert_eq!(
        runtime.test_function_debug_location(&child, None).unwrap(),
        Some((
            JsString::from_static("strip-source-unique.js"),
            crate::LineColumn::new(0, 1),
        ))
    );
    assert_eq!(runtime.test_function_debug_source(&child).unwrap(), None);
    assert!(
        runtime
            .test_debug_filename_atom_ownership(&child)
            .unwrap()
            .is_some()
    );
    drop(root);
    drop(child);
    assert_eq!(runtime.test_atom_count(), baseline_atoms);

    runtime.set_debug_info_mode(DebugInfoMode::StripDebug);
    let root = context
        .compile_with_filename("(function(){})", "strip-debug-unique.js")
        .unwrap();
    let child = runtime.test_child_function_bytecode(&root, 0).unwrap();
    assert_eq!(
        runtime.test_function_debug_location(&child, None).unwrap(),
        None
    );
    assert_eq!(runtime.test_function_debug_source(&child).unwrap(), None);
    assert_eq!(
        runtime.test_debug_filename_atom_ownership(&child).unwrap(),
        None
    );
    assert_eq!(runtime.test_atom_count(), baseline_atoms);
    drop(root);
    drop(child);
    assert_eq!(runtime.test_atom_count(), baseline_atoms);
}
