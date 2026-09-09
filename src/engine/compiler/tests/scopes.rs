use super::*;

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
            class_constructor: false,
            derived_class_constructor: false,
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
        is_parameter_initializer: false,
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
