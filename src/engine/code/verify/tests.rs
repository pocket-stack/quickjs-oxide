use super::flow::explicit_control_flow_target;
use crate::engine::code::bytecode::verify_parts;
use crate::engine::code::bytecode::{DynamicEnvironmentSource, WithObjectSource};
use crate::engine::code::function::metadata::ClassInitializerKind;
use crate::engine::code::function::metadata::ConstructorKind;
use crate::engine::code::function::metadata::FunctionKind;
use crate::engine::code::function::metadata::{FunctionMetadata, ParameterEnvironmentLayout};
use crate::engine::value::Value;

use super::*;
use crate::engine::code::bytecode::Instruction;
use crate::engine::code::debug::DebugInfoMode;

use crate::engine::code::function::metadata::{
    EvalBinding, EvalBindingSource, EvalEnvironment, EvalScope, EvalScopeKind,
    EvalVariableEnvironment, ParameterArgumentCell, ParameterBodyStorage, ParameterDefaultSource,
    ParameterPatternCopy,
};
use crate::engine::code::function::{
    UnlinkedFunctionDebug, UnlinkedFunctionParts, UnlinkedVariableDefinition,
};
use crate::engine::code::module::{
    ModuleLinkInitializer, ModuleLinkInitializerValue, UnlinkedModuleTables,
};
use crate::engine::compiler::compile_unlinked_module_with_filename;

fn trusted_ordinary_leaf(metadata: FunctionMetadata) -> UnlinkedFunction {
    UnlinkedFunction::fixture(
        vec![Instruction::GetArg(0), Instruction::Return],
        Vec::new(),
        metadata,
    )
}

#[test]
fn trusted_ordinary_leaf_has_a_distinct_root_publication_role() {
    let metadata = FunctionMetadata {
        argument_count: 1,
        defined_argument_count: 1,
        max_stack: 1,
        strip_variable_debug: true,
        function_kind: FunctionKind::Normal,
        has_prototype: true,
        constructor_kind: ConstructorKind::Base,
        ..FunctionMetadata::default()
    };
    verify_unlinked_ordinary_leaf(&trusted_ordinary_leaf(metadata)).unwrap();

    for forged in [
        FunctionMetadata {
            super_allowed: true,
            ..metadata
        },
        FunctionMetadata {
            arguments_forbidden: true,
            ..metadata
        },
        FunctionMetadata {
            has_prototype: false,
            ..metadata
        },
        FunctionMetadata {
            constructor_kind: ConstructorKind::None,
            ..metadata
        },
        FunctionMetadata {
            strip_variable_debug: false,
            ..metadata
        },
    ] {
        assert!(
            verify_unlinked_ordinary_leaf(&trusted_ordinary_leaf(forged))
                .unwrap_err()
                .to_string()
                .contains(
                    "trusted ordinary leaf metadata disagrees with its publication entry point"
                )
        );
    }

    let child = trusted_ordinary_leaf(metadata);
    let with_child = UnlinkedFunction::fixture(
        vec![Instruction::GetArg(0), Instruction::Return],
        vec![UnlinkedConstant::child(child)],
        metadata,
    );
    assert!(verify_unlinked_ordinary_leaf(&with_child).is_err());

    let with_atom_string = UnlinkedFunction::fixture(
        vec![Instruction::PushConst(0), Instruction::Return],
        vec![UnlinkedConstant::atom_string(JsString::from_static("atom"))],
        metadata,
    );
    assert!(verify_unlinked_ordinary_leaf(&with_atom_string).is_err());

    let with_empty_atom_string = UnlinkedFunction::fixture(
        vec![Instruction::PushConst(0), Instruction::Return],
        vec![UnlinkedConstant::atom_string(JsString::from_static(""))],
        metadata,
    );
    verify_unlinked_ordinary_leaf(&with_empty_atom_string).unwrap();

    let named_argument = trusted_ordinary_leaf(metadata).with_fixture_definitions(
        vec![UnlinkedVariableDefinition::ordinary(Some(
            JsString::from_static("argument"),
        ))],
        Vec::new(),
    );
    assert!(verify_unlinked_ordinary_leaf(&named_argument).is_err());

    let lexical_argument = trusted_ordinary_leaf(metadata).with_fixture_definitions(
        vec![UnlinkedVariableDefinition::lexical(None, false)],
        Vec::new(),
    );
    assert!(verify_unlinked_ordinary_leaf(&lexical_argument).is_err());

    let parameter_environment = trusted_ordinary_leaf(metadata).with_parameter_environment(Some(
        ParameterEnvironmentLayout {
            initialization_end: 0,
            argument_cells: Box::new([]),
            pattern_copies: Box::new([]),
            default_sources: Box::new([]),
            synthetic_arguments_local: None,
            arg_eval_variable_object_local: None,
        },
    ));
    assert!(verify_unlinked_ordinary_leaf(&parameter_environment).is_err());

    let with_debug = trusted_ordinary_leaf(metadata).with_debug(UnlinkedFunctionDebug {
        filename: JsString::from_static("ordinary-leaf.js"),
        pc2line: None,
        source: None,
    });
    assert!(verify_unlinked_ordinary_leaf(&with_debug).is_err());
}

fn module_with_link_initializers(
    module: UnlinkedModule,
    link_initializers: Vec<ModuleLinkInitializer>,
) -> UnlinkedModule {
    let parts = module.into_parts();
    UnlinkedModule::new(
        parts.name,
        parts.function,
        parts.has_top_level_await,
        UnlinkedModuleTables {
            declaration_order: parts.declaration_order.into_vec(),
            link_initializers,
            import_collisions: parts.import_collisions.into_vec(),
            requested_modules: parts.requested_modules.into_vec(),
            imports: parts.imports.into_vec(),
            exports: parts.exports.into_vec(),
            star_exports: parts.star_exports.into_vec(),
        },
    )
}

fn module_with_import_collisions(
    module: UnlinkedModule,
    import_collisions: Vec<crate::engine::code::module::ModuleImportCollision>,
) -> UnlinkedModule {
    let parts = module.into_parts();
    UnlinkedModule::new(
        parts.name,
        parts.function,
        parts.has_top_level_await,
        UnlinkedModuleTables {
            declaration_order: parts.declaration_order.into_vec(),
            link_initializers: parts.link_initializers.into_vec(),
            import_collisions,
            requested_modules: parts.requested_modules.into_vec(),
            imports: parts.imports.into_vec(),
            exports: parts.exports.into_vec(),
            star_exports: parts.star_exports.into_vec(),
        },
    )
}

fn module_with_code(
    module: UnlinkedModule,
    code: Vec<Instruction>,
    max_stack: u16,
) -> UnlinkedModule {
    let parts = module.into_parts();
    let mut function_parts = parts.function.into_parts();
    function_parts.code = code;
    function_parts.metadata.max_stack = max_stack;
    UnlinkedModule::new(
        parts.name,
        function_from_parts(function_parts),
        parts.has_top_level_await,
        UnlinkedModuleTables {
            declaration_order: parts.declaration_order.into_vec(),
            link_initializers: parts.link_initializers.into_vec(),
            import_collisions: parts.import_collisions.into_vec(),
            requested_modules: parts.requested_modules.into_vec(),
            imports: parts.imports.into_vec(),
            exports: parts.exports.into_vec(),
            star_exports: parts.star_exports.into_vec(),
        },
    )
}

fn module_with_mutated_function(
    module: UnlinkedModule,
    mutate: impl FnOnce(&mut UnlinkedFunctionParts),
) -> UnlinkedModule {
    let parts = module.into_parts();
    let mut function_parts = parts.function.into_parts();
    mutate(&mut function_parts);
    UnlinkedModule::new(
        parts.name,
        function_from_parts(function_parts),
        parts.has_top_level_await,
        UnlinkedModuleTables {
            declaration_order: parts.declaration_order.into_vec(),
            link_initializers: parts.link_initializers.into_vec(),
            import_collisions: parts.import_collisions.into_vec(),
            requested_modules: parts.requested_modules.into_vec(),
            imports: parts.imports.into_vec(),
            exports: parts.exports.into_vec(),
            star_exports: parts.star_exports.into_vec(),
        },
    )
}

fn module_with_top_level_await_flag(
    module: UnlinkedModule,
    has_top_level_await: bool,
) -> UnlinkedModule {
    let parts = module.into_parts();
    UnlinkedModule::new(
        parts.name,
        parts.function,
        has_top_level_await,
        UnlinkedModuleTables {
            declaration_order: parts.declaration_order.into_vec(),
            link_initializers: parts.link_initializers.into_vec(),
            import_collisions: parts.import_collisions.into_vec(),
            requested_modules: parts.requested_modules.into_vec(),
            imports: parts.imports.into_vec(),
            exports: parts.exports.into_vec(),
            star_exports: parts.star_exports.into_vec(),
        },
    )
}

fn function_from_parts(parts: UnlinkedFunctionParts) -> UnlinkedFunction {
    let UnlinkedFunctionParts {
        code,
        constants,
        metadata,
        parameter_environment,
        func_name,
        argument_definitions,
        local_definitions,
        closure_variables,
        eval_environments,
        debug,
    } = parts;
    let mut function = UnlinkedFunction::fixture_with_closure_variables(
        code,
        constants,
        metadata,
        closure_variables,
    )
    .with_parameter_environment(parameter_environment)
    .with_name(func_name)
    .with_fixture_definitions(argument_definitions, local_definitions)
    .with_eval_environments(eval_environments);
    if let Some(debug) = debug {
        function = function.with_debug(debug);
    }
    function
}

fn ordinary_environment(binding: Option<EvalBinding<JsString>>) -> EvalEnvironment<JsString> {
    EvalEnvironment {
        scopes: vec![
            EvalScope {
                kind: EvalScopeKind::FunctionBody,
                bindings: binding.into_iter().collect::<Vec<_>>().into_boxed_slice(),
            },
            EvalScope {
                kind: EvalScopeKind::FunctionRoot,
                bindings: Box::new([]),
            },
            EvalScope {
                kind: EvalScopeKind::ProgramBody,
                bindings: Box::new([]),
            },
            EvalScope {
                kind: EvalScopeKind::FunctionRoot,
                bindings: Box::new([]),
            },
        ]
        .into_boxed_slice(),
        variable_environment: EvalVariableEnvironment::StrictLocal(1),
        caller_strict: true,
        super_call_allowed: false,
        super_allowed: false,
    }
}

fn eval_code(environment: u16, close_local: bool) -> Vec<Instruction> {
    let mut code = vec![
        Instruction::Undefined,
        Instruction::Eval {
            argument_count: 0,
            environment,
        },
    ];
    if close_local {
        code.extend([
            Instruction::Drop,
            Instruction::CloseLocal(0),
            Instruction::Undefined,
        ]);
    }
    code.push(Instruction::Return);
    code
}

fn lexical_local_function(
    environment: EvalEnvironment<JsString>,
    code: Vec<Instruction>,
) -> UnlinkedFunction {
    UnlinkedFunction::fixture(
        code,
        Vec::new(),
        FunctionMetadata {
            local_count: 1,
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    )
    .with_fixture_definitions(
        Vec::new(),
        vec![UnlinkedVariableDefinition::lexical(
            Some(JsString::from_static("binding")),
            false,
        )],
    )
    .with_eval_environments(vec![environment])
}

fn local_with_environment(
    binding: EvalBinding<JsString>,
    definition_name: &'static str,
) -> UnlinkedFunction {
    let environment = EvalEnvironment {
        scopes: vec![
            EvalScope {
                kind: EvalScopeKind::With,
                bindings: vec![binding].into_boxed_slice(),
            },
            EvalScope {
                kind: EvalScopeKind::FunctionBody,
                bindings: Box::new([]),
            },
            EvalScope {
                kind: EvalScopeKind::FunctionRoot,
                bindings: vec![EvalBinding {
                    name: JsString::from_static("<var>"),
                    source: EvalBindingSource::Local(1),
                    is_lexical: false,
                    is_const: false,
                    kind: ClosureVariableKind::EvalVariableObject,
                    is_catch_parameter: false,
                }]
                .into_boxed_slice(),
            },
            EvalScope {
                kind: EvalScopeKind::ProgramBody,
                bindings: Box::new([]),
            },
            EvalScope {
                kind: EvalScopeKind::FunctionRoot,
                bindings: Box::new([]),
            },
        ]
        .into_boxed_slice(),
        variable_environment: EvalVariableEnvironment::VariableObject {
            scope: 2,
            source: EvalBindingSource::Local(1),
        },
        caller_strict: false,
        super_call_allowed: false,
        super_allowed: false,
    };
    let mut code = vec![Instruction::VariableEnvironment, Instruction::PutLocal(1)];
    code.extend(eval_code(0, false));
    UnlinkedFunction::fixture(
        code,
        Vec::new(),
        FunctionMetadata {
            local_count: 2,
            eval_variable_object_local: Some(1),
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    )
    .with_fixture_definitions(
        Vec::new(),
        vec![
            UnlinkedVariableDefinition {
                name: Some(JsString::from_static(definition_name)),
                is_lexical: false,
                is_const: false,
                is_parameter_initializer: false,
                kind: ClosureVariableKind::WithObject,
            },
            UnlinkedVariableDefinition {
                name: Some(JsString::from_static("<var>")),
                is_lexical: false,
                is_const: false,
                is_parameter_initializer: false,
                kind: ClosureVariableKind::EvalVariableObject,
            },
        ],
    )
    .with_eval_environments(vec![environment])
}

fn captured_with_object_function(strict: bool) -> UnlinkedFunction {
    let relay = UnlinkedFunction::fixture_with_closure_variables(
        vec![Instruction::Undefined, Instruction::Return],
        vec![UnlinkedConstant::primitive(Value::String(JsString::from_static("<with>"))).unwrap()],
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentLocal(0),
            name: ClosureVariableName::Constant(0),
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::WithObject,
        }],
    );
    UnlinkedFunction::fixture(
        vec![
            Instruction::Undefined,
            Instruction::InitializeLocal(0),
            Instruction::FClosure(0),
            Instruction::Drop,
            Instruction::CloseLocal(0),
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(relay)],
        FunctionMetadata {
            local_count: 1,
            max_stack: 1,
            strict,
            ..FunctionMetadata::default()
        },
    )
    .with_fixture_definitions(Vec::new(), vec![UnlinkedVariableDefinition::with_object()])
}

fn script_with_child(child: UnlinkedFunction) -> UnlinkedFunction {
    UnlinkedFunction::fixture(
        vec![Instruction::Undefined, Instruction::Return],
        vec![UnlinkedConstant::child(child)],
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    )
}

fn empty_class_initializer(kind: ClassInitializerKind) -> UnlinkedFunction {
    UnlinkedFunction::fixture(
        vec![Instruction::Undefined, Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            super_allowed: true,
            arguments_forbidden: true,
            needs_home_object: true,
            class_initializer_kind: Some(kind),
            ..FunctionMetadata::default()
        },
    )
}

fn script_installing_instance_initializer(child: UnlinkedFunction) -> UnlinkedFunction {
    UnlinkedFunction::fixture(
        vec![
            Instruction::Undefined,
            Instruction::Undefined,
            Instruction::FClosure(0),
            Instruction::InstallClassInstanceInitializer,
            Instruction::Drop,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(child)],
        FunctionMetadata {
            max_stack: 3,
            ..FunctionMetadata::default()
        },
    )
}

fn script_running_static_initializer(child: UnlinkedFunction) -> UnlinkedFunction {
    UnlinkedFunction::fixture(
        vec![
            Instruction::Undefined,
            Instruction::FClosure(0),
            Instruction::RunClassStaticInitializer,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(child)],
        FunctionMetadata {
            max_stack: 2,
            ..FunctionMetadata::default()
        },
    )
}

fn derived_this_initializer(
    this_source: ClosureSource,
    active_function_source: ClosureSource,
    new_target_source: ClosureSource,
) -> UnlinkedFunction {
    derived_this_initializer_with_code(
        this_source,
        active_function_source,
        new_target_source,
        vec![
            Instruction::GetVarRef(1),
            Instruction::GetSuper,
            Instruction::GetVarRef(2),
            Instruction::MarkSuperCall,
            Instruction::ConstructSuper(0),
            Instruction::Dup,
            Instruction::InitializeDerivedVarRef(0),
            Instruction::Return,
        ],
        2,
    )
}

fn derived_this_initializer_with_code(
    this_source: ClosureSource,
    active_function_source: ClosureSource,
    new_target_source: ClosureSource,
    code: Vec<Instruction>,
    max_stack: u16,
) -> UnlinkedFunction {
    UnlinkedFunction::fixture_with_closure_variables(
        code,
        vec![
            UnlinkedConstant::primitive(Value::String(JsString::from_static("<this>"))).unwrap(),
            UnlinkedConstant::primitive(Value::String(JsString::from_static("<this_active_func>")))
                .unwrap(),
            UnlinkedConstant::primitive(Value::String(JsString::from_static("<new.target>")))
                .unwrap(),
        ],
        FunctionMetadata {
            closure_count: 3,
            max_stack,
            strict: true,
            super_call_allowed: true,
            super_allowed: true,
            ..FunctionMetadata::default()
        },
        vec![
            ClosureVariable {
                source: this_source,
                name: ClosureVariableName::Constant(0),
                is_lexical: true,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            },
            ClosureVariable {
                source: active_function_source,
                name: ClosureVariableName::Constant(1),
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            },
            ClosureVariable {
                source: new_target_source,
                name: ClosureVariableName::Constant(2),
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            },
        ],
    )
}

fn derived_parent(child: UnlinkedFunction) -> UnlinkedFunction {
    UnlinkedFunction::fixture(
        vec![
            Instruction::PushActiveFunction,
            Instruction::PutLocal(1),
            Instruction::PushNewTarget,
            Instruction::PutLocal(2),
            Instruction::CheckCtor,
            Instruction::FClosure(0),
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::ReturnDerived(0),
        ],
        vec![UnlinkedConstant::child(child)],
        FunctionMetadata {
            local_count: 6,
            derived_this_local: Some(0),
            active_function_local: Some(1),
            max_stack: 1,
            strict: true,
            super_call_allowed: true,
            super_allowed: true,
            constructor_kind: ConstructorKind::Derived,
            ..FunctionMetadata::default()
        },
    )
    .with_fixture_definitions(
        Vec::new(),
        vec![
            UnlinkedVariableDefinition::lexical(Some(JsString::from_static("<this>")), false),
            UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("<this_active_func>"))),
            UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("<new.target>"))),
            // A hand-authored unlinked tree can copy the sentinel spelling
            // onto an unrelated lexical; provenance must not follow it.
            UnlinkedVariableDefinition::lexical(Some(JsString::from_static("<this>")), false),
            UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("<this_active_func>"))),
            UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("<new.target>"))),
        ],
    )
}

fn eval_root_binding(
    name: &'static str,
    scope: u16,
    is_lexical: bool,
    is_const: bool,
    kind: ClosureVariableKind,
) -> EvalRootBinding<JsString> {
    EvalRootBinding {
        name: JsString::from_static(name),
        scope,
        is_lexical,
        is_const,
        kind,
        is_catch_parameter: false,
    }
}

fn eval_root_with_descriptors(
    eval_kind: EvalKind,
    descriptors: Vec<(ClosureSource, &'static str, bool, bool, ClosureVariableKind)>,
) -> UnlinkedFunction {
    eval_root_with_descriptors_and_strict(eval_kind, false, descriptors)
}

fn eval_root_with_descriptors_and_strict(
    eval_kind: EvalKind,
    strict: bool,
    descriptors: Vec<(ClosureSource, &'static str, bool, bool, ClosureVariableKind)>,
) -> UnlinkedFunction {
    let constants = descriptors
        .iter()
        .map(|(_, name, _, _, _)| {
            UnlinkedConstant::primitive(Value::String(JsString::from_static(name))).unwrap()
        })
        .collect();
    let closure_variables = descriptors
        .into_iter()
        .enumerate()
        .map(
            |(index, (source, _, is_lexical, is_const, kind))| ClosureVariable {
                source,
                name: ClosureVariableName::Constant(
                    u32::try_from(index).expect("test descriptor index fits u32"),
                ),
                is_lexical,
                is_const,
                kind,
            },
        )
        .collect::<Vec<_>>();
    UnlinkedFunction::fixture_with_closure_variables(
        vec![Instruction::Undefined, Instruction::Return],
        constants,
        FunctionMetadata {
            closure_count: u16::try_from(closure_variables.len())
                .expect("test descriptor count fits u16"),
            max_stack: 1,
            strict,
            eval_kind,
            ..FunctionMetadata::default()
        },
        closure_variables,
    )
}

fn recursive_eval_root(
    imported_body_kind: EvalScopeKind,
    binding_sources: [u16; 2],
    variable_target: u16,
) -> UnlinkedFunction {
    let names = ["<var>", "<var>"];
    let constants = names
        .iter()
        .map(|name| {
            UnlinkedConstant::primitive(Value::String(JsString::from_static(name))).unwrap()
        })
        .collect::<Vec<_>>();
    let descriptors = names
        .iter()
        .enumerate()
        .map(|(index, _)| ClosureVariable {
            source: ClosureSource::EvalEnvironment(
                u16::try_from(index).expect("test closure index fits u16"),
            ),
            name: ClosureVariableName::Constant(
                u32::try_from(index).expect("test constant index fits u32"),
            ),
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::EvalVariableObject,
        })
        .collect::<Vec<_>>();
    let imported_bindings = names
        .iter()
        .zip(binding_sources)
        .map(|(name, source)| EvalBinding {
            name: JsString::from_static(name),
            source: EvalBindingSource::Closure(source),
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::EvalVariableObject,
            is_catch_parameter: false,
        })
        .collect::<Vec<_>>();
    UnlinkedFunction::fixture_with_closure_variables(
        eval_code(0, false),
        constants,
        FunctionMetadata {
            closure_count: 2,
            max_stack: 1,
            eval_kind: EvalKind::Direct,
            ..FunctionMetadata::default()
        },
        descriptors,
    )
    .with_eval_environments(vec![EvalEnvironment {
        scopes: vec![
            EvalScope {
                kind: EvalScopeKind::ProgramBody,
                bindings: Box::new([]),
            },
            EvalScope {
                kind: EvalScopeKind::FunctionRoot,
                bindings: Box::new([]),
            },
            EvalScope {
                kind: imported_body_kind,
                bindings: Box::new([]),
            },
            EvalScope {
                kind: EvalScopeKind::FunctionRoot,
                bindings: imported_bindings.into_boxed_slice(),
            },
        ]
        .into_boxed_slice(),
        variable_environment: EvalVariableEnvironment::VariableObject {
            scope: 3,
            source: EvalBindingSource::Closure(variable_target),
        },
        caller_strict: false,
        super_call_allowed: false,
        super_allowed: false,
    }])
}

mod bindings;
mod eval;
mod modules;
mod parameters;
mod private;
