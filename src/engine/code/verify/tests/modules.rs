use super::*;

#[test]
fn module_root_rejects_forged_top_level_await_flag_in_both_directions() {
    for (source, forged_has_top_level_await) in [("0;", true), ("await 0;", false)] {
        let module = compile_unlinked_module_with_filename(
            source,
            "forged-top-level-await-flag.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap();
        let forged = module_with_top_level_await_flag(module, forged_has_top_level_await);
        let error = verify_unlinked_module_tree(&forged).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("module root metadata disagrees with its publication entry point"),
            "{source:?}: {error}"
        );
    }
}

#[test]
fn module_link_initializer_ledger_rejects_reordered_slots() {
    let module = compile_unlinked_module_with_filename(
        "var first; function second() {}",
        "forged-ledger-order.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();
    verify_unlinked_module_tree(&module).unwrap();

    let mut initializers = module.link_initializers().to_vec();
    assert_eq!(initializers.len(), 2);
    initializers.swap(0, 1);
    let forged = module_with_link_initializers(module, initializers);
    let error = verify_unlinked_module_tree(&forged).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("module link initializer ledger is not in declaration order")
    );
}

#[test]
fn module_link_initializer_ledger_rejects_value_mismatch() {
    let module = compile_unlinked_module_with_filename(
        "var first; function second() {}",
        "forged-ledger-value.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();
    verify_unlinked_module_tree(&module).unwrap();

    let mut initializers = module.link_initializers().to_vec();
    let function = initializers
        .iter_mut()
        .find(|initializer| {
            matches!(
                initializer.value,
                ModuleLinkInitializerValue::Function { .. }
            )
        })
        .expect("module function declaration has a link initializer");
    function.value = ModuleLinkInitializerValue::Undefined;
    let forged = module_with_link_initializers(module, initializers);
    let error = verify_unlinked_module_tree(&forged).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("module link entry initializer is not canonical")
    );
}

#[test]
fn module_import_collision_requires_an_exact_unique_ledger() {
    let module = compile_unlinked_module_with_filename(
        "import { value } from './dependency.js'; let value = 1;",
        "forged-collision-ledger.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();
    verify_unlinked_module_tree(&module).unwrap();

    let missing = module_with_import_collisions(module, Vec::new());
    let error = verify_unlinked_module_tree(&missing).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("module import table disagrees with its closure descriptor")
    );

    let module = compile_unlinked_module_with_filename(
        "import { value } from './dependency.js'; let value = 1;",
        "forged-duplicate-collision-ledger.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();
    let collision = module.import_collisions()[0];
    let duplicate = module_with_import_collisions(module, vec![collision, collision]);
    let error = verify_unlinked_module_tree(&duplicate).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("module import collision ledger reused a closure slot")
    );
}

#[test]
fn module_import_meta_descriptor_is_unique_and_exact() {
    let compile = || {
        compile_unlinked_module_with_filename(
            "globalThis.meta = import.meta;",
            "forged-import-meta.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap()
    };

    let malformed = module_with_mutated_function(compile(), |parts| {
        let descriptor = parts
            .closure_variables
            .iter_mut()
            .find(|descriptor| descriptor.source == ClosureSource::ModuleImportMeta)
            .unwrap();
        descriptor.is_const = false;
    });
    let error = verify_unlinked_module_tree(&malformed).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("import.meta descriptor has invalid binding metadata"),
        "{error}"
    );

    let unnamed = module_with_mutated_function(compile(), |parts| {
        let descriptor = parts
            .closure_variables
            .iter_mut()
            .find(|descriptor| descriptor.source == ClosureSource::ModuleImportMeta)
            .unwrap();
        descriptor.name = ClosureVariableName::None;
    });
    let error = verify_unlinked_module_tree(&unnamed).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("import.meta descriptor has invalid binding metadata"),
        "{error}"
    );

    let duplicate = module_with_mutated_function(compile(), |parts| {
        let descriptor = parts
            .closure_variables
            .iter()
            .find(|descriptor| descriptor.source == ClosureSource::ModuleImportMeta)
            .copied()
            .unwrap();
        parts.closure_variables.push(descriptor);
        parts.metadata.closure_count += 1;
    });
    let error = verify_unlinked_module_tree(&duplicate).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("module contains more than one import.meta binding"),
        "{error}"
    );
}

#[test]
fn module_evaluation_cannot_reenter_link_phase() {
    let module = compile_unlinked_module_with_filename(
        "import { value } from './dependency.js'; let value = 1;",
        "forged-collision-control-flow.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();
    let parts = module.into_parts();
    let mut function_parts = parts.function.into_parts();
    let insertion = function_parts.code.len() - 2;
    function_parts.code.insert(insertion, Instruction::Goto(2));
    let forged = UnlinkedModule::new(
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
    );
    let error = verify_unlinked_module_tree(&forged).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("module evaluation control flow entered the link phase"),
        "{error}"
    );
}

#[test]
fn module_initializer_flow_rejects_stack_valid_skips_repeats_and_shared_gosubs() {
    let cases = [
        (
            "forward-skip",
            vec![
                Instruction::PushThis,
                Instruction::IfFalse(4),
                Instruction::Undefined,
                Instruction::Return,
                Instruction::PushFalse,
                Instruction::IfFalse(9),
                Instruction::PushI32(1),
                Instruction::InitializeModuleImportCollision(0),
                Instruction::Goto(9),
                Instruction::Undefined,
                Instruction::Return,
            ],
            1,
        ),
        (
            "back-edge-repeat",
            vec![
                Instruction::PushThis,
                Instruction::IfFalse(4),
                Instruction::Undefined,
                Instruction::Return,
                Instruction::PushI32(1),
                Instruction::InitializeModuleImportCollision(0),
                Instruction::PushI32(2),
                Instruction::Goto(5),
                Instruction::Undefined,
                Instruction::Return,
            ],
            1,
        ),
        (
            "shared-gosub-repeat",
            vec![
                Instruction::PushThis,
                Instruction::IfFalse(4),
                Instruction::Undefined,
                Instruction::Return,
                Instruction::Gosub(7),
                Instruction::Gosub(7),
                Instruction::Goto(10),
                Instruction::PushI32(1),
                Instruction::InitializeModuleImportCollision(0),
                Instruction::Ret,
                Instruction::Undefined,
                Instruction::Return,
            ],
            2,
        ),
        (
            "caught-call-skip",
            vec![
                Instruction::PushThis,
                Instruction::IfFalse(4),
                Instruction::Undefined,
                Instruction::Return,
                Instruction::Catch(13),
                Instruction::Undefined,
                Instruction::Call(0),
                Instruction::Drop,
                Instruction::PushI32(1),
                Instruction::InitializeModuleImportCollision(0),
                Instruction::DropCatch,
                Instruction::Goto(14),
                Instruction::Nop,
                Instruction::Return,
                Instruction::Undefined,
                Instruction::Return,
            ],
            2,
        ),
    ];

    for (label, code, max_stack) in cases {
        let module = compile_unlinked_module_with_filename(
            "import { value } from './dependency.js'; let value = 1;",
            "forged-stack-valid-initializer-flow.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap();
        let forged = module_with_code(module, code, max_stack);
        verify_parts(
            forged.function().code(),
            forged.function().constants().len(),
            forged.function().metadata().max_stack,
        )
        .unwrap_or_else(|error| panic!("{label}: generic verifier rejected fixture: {error}"));
        let error = verify_unlinked_module_tree(&forged).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("module lexical initializer is not a one-shot control-flow cut"),
            "{label}: {error}"
        );
    }
}

#[test]
fn module_initializer_flow_tracks_cleanup_discarded_gosub_returns() {
    let cases = [
        (
            "nip-catch",
            vec![
                Instruction::PushThis,
                Instruction::IfFalse(4),
                Instruction::Undefined,
                Instruction::Return,
                Instruction::Gosub(7),
                Instruction::Undefined,
                Instruction::Return,
                Instruction::Catch(18),
                Instruction::Gosub(13),
                Instruction::PushI32(1),
                Instruction::InitializeModuleImportCollision(0),
                Instruction::Undefined,
                Instruction::Return,
                Instruction::Undefined,
                Instruction::NipCatch,
                Instruction::Gosub(20),
                Instruction::Drop,
                Instruction::Ret,
                Instruction::Throw,
                Instruction::Nop,
                Instruction::Ret,
                Instruction::Undefined,
                Instruction::Return,
            ],
            4,
        ),
        (
            "iterator-drop-preserve",
            vec![
                Instruction::PushThis,
                Instruction::IfFalse(4),
                Instruction::Undefined,
                Instruction::Return,
                Instruction::Gosub(7),
                Instruction::Undefined,
                Instruction::Return,
                Instruction::ArrayFrom(0),
                Instruction::ForOfStart,
                Instruction::Gosub(14),
                Instruction::PushI32(1),
                Instruction::InitializeModuleImportCollision(0),
                Instruction::Undefined,
                Instruction::Throw,
                Instruction::Undefined,
                Instruction::IteratorDropPreserve,
                Instruction::Gosub(20),
                Instruction::Drop,
                Instruction::Ret,
                Instruction::Nop,
                Instruction::Ret,
                Instruction::Undefined,
                Instruction::Return,
            ],
            6,
        ),
        (
            "return-derived-throw",
            vec![
                Instruction::PushThis,
                Instruction::IfFalse(4),
                Instruction::Undefined,
                Instruction::Return,
                Instruction::PushI32(1),
                Instruction::InitializeModuleImportCollision(0),
                Instruction::Catch(13),
                Instruction::Catch(10),
                Instruction::Undefined,
                Instruction::Throw,
                Instruction::ReturnDerived(0),
                Instruction::Nop,
                Instruction::Nop,
                Instruction::Goto(5),
                Instruction::Undefined,
                Instruction::Return,
            ],
            3,
        ),
    ];

    for (label, code, max_stack) in cases {
        let module = compile_unlinked_module_with_filename(
            "import { value } from './dependency.js'; let value = 1;",
            "forged-cleanup-initializer-flow.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap();
        let forged = module_with_code(module, code, max_stack);
        verify_parts(
            forged.function().code(),
            forged.function().constants().len(),
            forged.function().metadata().max_stack,
        )
        .unwrap_or_else(|error| panic!("{label}: generic verifier rejected fixture: {error}"));
        let error = verify_unlinked_module_tree(&forged).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("module lexical initializer is not a one-shot control-flow cut"),
            "{label}: {error}"
        );
    }
}

#[test]
fn module_lexical_initializers_allow_expression_joins() {
    for import in [
        "",
        "import value from './dependency.js';",
        "import { value } from './dependency.js';",
        "import * as value from './dependency.js';",
    ] {
        for expression in [
            "globalThis.flag ? 1 : 2",
            "globalThis.flag && 42",
            "globalThis.flag || 42",
            "globalThis.flag ?? 42",
        ] {
            let source = format!("{import} let value = {expression};");
            let module = compile_unlinked_module_with_filename(
                &source,
                "module-lexical-expression-join.mjs",
                DebugInfoMode::StripDebug,
            )
            .unwrap_or_else(|error| panic!("{source}: {error}"));
            let initializer_pc = module
                .function()
                .code()
                .iter()
                .position(|instruction| {
                    matches!(
                        instruction,
                        Instruction::InitializeVarRef(_)
                            | Instruction::InitializeModuleImportCollision(_)
                    )
                })
                .unwrap_or_else(|| panic!("{source}: lexical initializer disappeared"));
            assert!(
                module.function().code()[..initializer_pc]
                    .iter()
                    .any(|instruction| {
                        explicit_control_flow_target(instruction) == Some(initializer_pc)
                    }),
                "{source}: expression lost its initializer join edge"
            );
            verify_unlinked_module_tree(&module)
                .unwrap_or_else(|error| panic!("{source}: {error}"));
        }
    }
}

#[test]
fn module_import_collision_destructuring_initializers_publish() {
    for source in [
        "import { value } from './dependency.js'; let [value] = [1];",
        "import { value } from './dependency.js'; const [value] = [1];",
        "import { value } from './dependency.js';\
         let { answer: value } = { answer: 1 };",
        "import { value } from './dependency.js';\
         const { answer: value } = { answer: 1 };",
        "import { first, second } from './dependency.js';\
         let [first, second] = [1, 2];",
        "import { first, second } from './dependency.js';\
         const { first, nested: { second } } =\
             { first: 1, nested: { second: 2 } };",
        "import { value } from './dependency.js';\
         let [[value] = [1]] = [];",
        "import { first, second } from './dependency.js';\
         let [[first] = [1], second] = [[], 2];",
        "export let [first, ...second] = [1, 2, 3];",
    ] {
        let module = compile_unlinked_module_with_filename(
            source,
            "module-import-collision-destructuring.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap_or_else(|error| panic!("{source}: {error}"));
        verify_unlinked_module_tree(&module).unwrap_or_else(|error| {
            panic!(
                "{source}: {error}\nbytecode: {:#?}",
                module.function().code()
            )
        });
    }
}

#[test]
fn module_initializer_flow_handles_many_shared_finally_entries() {
    let mut source = String::from("outer: { try {");
    for _ in 0..384 {
        source.push_str("if (globalThis.flag) break outer;");
    }
    source.push_str("} finally {");
    for _ in 0..384 {
        source.push_str("globalThis.observed;");
    }
    source.push_str("} } export let answer = 42;");

    let module = compile_unlinked_module_with_filename(
        &source,
        "module-many-finally-entries.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();
    verify_unlinked_module_tree(&module).unwrap();
}

#[test]
fn module_initializer_flow_summarizes_nested_shared_gosubs() {
    const DEPTH: usize = 48;

    let mut code = vec![
        Instruction::PushThis,
        Instruction::IfFalse(4),
        Instruction::Undefined,
        Instruction::Return,
        Instruction::Gosub(0),
        Instruction::Goto(0),
    ];
    let mut layers = Vec::with_capacity(DEPTH);
    for _ in 0..DEPTH {
        let start = code.len();
        layers.push(start);
        code.extend([
            Instruction::PushTrue,
            Instruction::IfFalse(u32::try_from(start + 4).unwrap()),
            Instruction::Gosub(0),
            Instruction::Goto(u32::try_from(start + 5).unwrap()),
            Instruction::Gosub(0),
            Instruction::Ret,
        ]);
    }
    let leaf = code.len();
    code.extend([
        Instruction::PushI32(1),
        Instruction::InitializeModuleImportCollision(0),
        Instruction::Ret,
    ]);
    let end = code.len();
    code.extend([Instruction::Undefined, Instruction::Return]);

    code[4] = Instruction::Gosub(u32::try_from(layers[0]).unwrap());
    code[5] = Instruction::Goto(u32::try_from(end).unwrap());
    for (index, start) in layers.iter().copied().enumerate() {
        let next = layers.get(index + 1).copied().unwrap_or(leaf);
        code[start + 2] = Instruction::Gosub(u32::try_from(next).unwrap());
        code[start + 4] = Instruction::Gosub(u32::try_from(next).unwrap());
    }

    let module = compile_unlinked_module_with_filename(
        "import { value } from './dependency.js'; let value = 1;",
        "nested-shared-gosub-initializer-flow.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();
    let max_stack = u16::try_from(DEPTH + 2).unwrap();
    let forged = module_with_code(module, code, max_stack);
    verify_parts(
        forged.function().code(),
        forged.function().constants().len(),
        forged.function().metadata().max_stack,
    )
    .unwrap();
    verify_unlinked_module_tree(&forged).unwrap();
}

#[test]
fn module_initializer_flow_does_not_invent_throws_for_pure_bytecode() {
    let module = compile_unlinked_module_with_filename(
        "import { value } from './dependency.js'; let value = 1;",
        "pure-catch-initializer-flow.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();
    let forged = module_with_code(
        module,
        vec![
            Instruction::PushThis,
            Instruction::IfFalse(4),
            Instruction::Undefined,
            Instruction::Return,
            Instruction::Catch(9),
            Instruction::PushI32(1),
            Instruction::InitializeModuleImportCollision(0),
            Instruction::DropCatch,
            Instruction::Goto(10),
            Instruction::Return,
            Instruction::Undefined,
            Instruction::Return,
        ],
        2,
    );
    verify_parts(
        forged.function().code(),
        forged.function().constants().len(),
        forged.function().metadata().max_stack,
    )
    .unwrap();
    verify_unlinked_module_tree(&forged).unwrap();
}

#[test]
fn module_function_initializer_requires_a_strict_declaration_child() {
    for private_name in [false, true] {
        let module = compile_unlinked_module_with_filename(
            "export function value() {}",
            "forged-function-child-metadata.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap();
        let constant = match module.link_initializers()[0].value {
            ModuleLinkInitializerValue::Function { constant, .. } => constant,
            _ => panic!("module function lost its initializer"),
        };
        let parts = module.into_parts();
        let mut function_parts = parts.function.into_parts();
        let original = std::mem::replace(
            &mut function_parts.constants[constant as usize],
            UnlinkedConstant::primitive(Value::Undefined).unwrap(),
        );
        let (_, _, child) = original.into_parts();
        let mut child_parts = child
            .expect("module function stopped referencing a child")
            .into_parts();
        if private_name {
            child_parts.metadata.function_name_local = Some(0);
        } else {
            child_parts.metadata.strict = false;
        }
        function_parts.constants[constant as usize] =
            UnlinkedConstant::child(function_from_parts(child_parts));
        let forged = UnlinkedModule::new(
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
        );
        let error = verify_unlinked_module_tree(&forged).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("module link entry initializer is not canonical"),
            "{error}"
        );
    }
}

#[test]
fn anonymous_default_function_initializer_requires_exact_name_inference() {
    let module = compile_unlinked_module_with_filename(
        "export default function () {}",
        "forged-default-function-name.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();
    verify_unlinked_module_tree(&module).unwrap();

    let mut initializers = module.link_initializers().to_vec();
    let [initializer] = initializers.as_mut_slice() else {
        panic!("anonymous default function lost its initializer");
    };
    let ModuleLinkInitializerValue::Function { inferred_name, .. } = &mut initializer.value else {
        panic!("anonymous default function initializer changed kind");
    };
    assert!(inferred_name.take().is_some());
    let forged = module_with_link_initializers(module, initializers);
    let error = verify_unlinked_module_tree(&forged).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("module link entry initializer is not canonical")
    );
}

#[test]
fn anonymous_default_function_initializer_rejects_a_coordinated_wrong_name() {
    let module = compile_unlinked_module_with_filename(
        "export default function () {}",
        "forged-default-function-wrong-name.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();
    let name = match module.link_initializers()[0].value {
        ModuleLinkInitializerValue::Function {
            inferred_name: Some(name),
            ..
        } => name,
        _ => panic!("anonymous default function lost its inferred name"),
    };
    let parts = module.into_parts();
    let mut function_parts = parts.function.into_parts();
    function_parts.constants[name as usize] =
        UnlinkedConstant::primitive(Value::String(JsString::from_static("not-default"))).unwrap();
    let forged = UnlinkedModule::new(
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
    );
    let error = verify_unlinked_module_tree(&forged).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("module link entry initializer is not canonical")
    );
}

#[test]
fn named_function_initializer_cannot_claim_default_name_inference() {
    let module = compile_unlinked_module_with_filename(
        "'default'; export default function named() {}",
        "forged-named-default-function.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();
    let name = module
        .function()
        .constants()
        .iter()
        .position(|constant| {
            constant.as_primitive()
                == Some(&crate::engine::value::PrimitiveValue::String(
                    JsString::from_static("default"),
                ))
        })
        .and_then(|index| u32::try_from(index).ok())
        .expect("root default String constant is missing");
    let mut initializers = module.link_initializers().to_vec();
    let [initializer] = initializers.as_mut_slice() else {
        panic!("named default function lost its initializer");
    };
    let ModuleLinkInitializerValue::Function { inferred_name, .. } = &mut initializer.value else {
        panic!("named default function initializer changed kind");
    };
    assert_eq!(*inferred_name, None);
    *inferred_name = Some(name);
    let forged = module_with_link_initializers(module, initializers);
    let error = verify_unlinked_module_tree(&forged).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("module link entry initializer is not canonical")
    );
}

#[test]
fn anonymous_default_function_initializer_authenticates_its_private_export_cell() {
    let module = compile_unlinked_module_with_filename(
        "export default function () {}",
        "forged-default-function-export.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();
    let parts = module.into_parts();
    let mut exports = parts.exports.into_vec();
    exports[0].export_name = JsString::from_static("forged");
    let forged = UnlinkedModule::new(
        parts.name,
        parts.function,
        parts.has_top_level_await,
        UnlinkedModuleTables {
            declaration_order: parts.declaration_order.into_vec(),
            link_initializers: parts.link_initializers.into_vec(),
            import_collisions: parts.import_collisions.into_vec(),
            requested_modules: parts.requested_modules.into_vec(),
            imports: parts.imports.into_vec(),
            exports,
            star_exports: parts.star_exports.into_vec(),
        },
    );
    let error = verify_unlinked_module_tree(&forged).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("module link entry initializer is not canonical")
    );
}

#[test]
fn anonymous_default_function_initializer_authenticates_its_private_descriptor() {
    let module = compile_unlinked_module_with_filename(
        "export default function () {}",
        "forged-default-function-descriptor.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();
    let parts = module.into_parts();
    let mut function_parts = parts.function.into_parts();
    let forged_name = u32::try_from(function_parts.constants.len()).unwrap();
    function_parts
        .constants
        .push(UnlinkedConstant::primitive(Value::String(JsString::from_static("forged"))).unwrap());
    function_parts.closure_variables[0].name = ClosureVariableName::Constant(forged_name);
    let forged = UnlinkedModule::new(
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
    );
    let error = verify_unlinked_module_tree(&forged).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("module link entry initializer is not canonical")
    );
}

#[test]
fn anonymous_default_function_initializer_authenticates_its_anonymous_child() {
    let module = compile_unlinked_module_with_filename(
        "export default function () {}",
        "forged-default-function-child.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();
    let constant = match module.link_initializers()[0].value {
        ModuleLinkInitializerValue::Function { constant, .. } => constant,
        _ => panic!("anonymous default function lost its initializer"),
    };
    let parts = module.into_parts();
    let mut function_parts = parts.function.into_parts();
    let original = std::mem::replace(
        &mut function_parts.constants[constant as usize],
        UnlinkedConstant::primitive(Value::Undefined).unwrap(),
    );
    let (_, _, child) = original.into_parts();
    function_parts.constants[constant as usize] = UnlinkedConstant::child(
        child
            .expect("default function initializer stopped referencing a child")
            .with_name(Some(JsString::from_static("forged"))),
    );
    let forged = UnlinkedModule::new(
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
    );
    let error = verify_unlinked_module_tree(&forged).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("module link entry initializer is not canonical")
    );
}

#[test]
fn named_default_function_initializer_cannot_impersonate_the_private_default_cell() {
    let module = compile_unlinked_module_with_filename(
        "export default function named() {}",
        "forged-named-default-private-cell.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();
    let (constant, closure_index) = match module.link_initializers()[0] {
        ModuleLinkInitializer {
            closure_index,
            value:
                ModuleLinkInitializerValue::Function {
                    constant,
                    inferred_name: None,
                },
        } => (constant, closure_index),
        _ => panic!("named default function lost its canonical initializer"),
    };
    let parts = module.into_parts();
    let mut function_parts = parts.function.into_parts();
    let original = std::mem::replace(
        &mut function_parts.constants[constant as usize],
        UnlinkedConstant::primitive(Value::Undefined).unwrap(),
    );
    let (_, _, child) = original.into_parts();
    let private_name =
        JsString::from_static(crate::engine::code::module::MODULE_DEFAULT_BINDING_NAME);
    function_parts.constants[constant as usize] = UnlinkedConstant::child(
        child
            .expect("named default function stopped referencing a child")
            .with_name(Some(private_name.clone())),
    );
    let private_name_index = u32::try_from(function_parts.constants.len()).unwrap();
    function_parts
        .constants
        .push(UnlinkedConstant::primitive(Value::String(private_name)).unwrap());
    function_parts.closure_variables[usize::from(closure_index)].name =
        ClosureVariableName::Constant(private_name_index);
    let forged = UnlinkedModule::new(
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
    );
    let error = verify_unlinked_module_tree(&forged).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("module link entry initializer is not canonical")
    );
}
