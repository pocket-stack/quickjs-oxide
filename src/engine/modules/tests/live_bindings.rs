use super::*;

#[test]
fn module_loader_cache_cycles_and_live_cells_follow_quickjs_order() {
    let runtime = Runtime::new();
    let (loader, loads, normalizations) = MapModuleLoader::new([
        (
            "pkg/a.js",
            r#"
            import { seen, read } from "./b.js";
            export { read };
            export let value = 1;
            export function bump() { value = 42; }
            globalThis.__aSeen = seen;
            globalThis.__aRead = read();
            "#,
        ),
        (
            "pkg/b.js",
            r#"
            import { value } from "./a.js";
            export var seen = 7;
            export function read() { return value; }
            globalThis.__bRuns = (globalThis.__bRuns || 0) + 1;
            "#,
        ),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let entry = context
        .compile_module_with_filename(
            r#"
            import "./a.js";
            import { value, bump, read } from "./a.js";
            globalThis.__before = value;
            bump();
            globalThis.__after = value;
            globalThis.__afterViaCycle = read();
            "#,
            "pkg/entry.js",
        )
        .unwrap();

    assert_eq!(&*loads.borrow(), &["pkg/a.js", "pkg/b.js"]);
    assert_eq!(normalizations.borrow().len(), 4);
    let first = module_evaluation_promise(&mut context, &entry);
    assert_script_true(
        &mut context,
        r#"
        __aSeen === 7 && __aRead === 1 && __bRuns === 1 &&
        __before === 1 && __after === 42 && __afterViaCycle === 42
        "#,
    );
    let second = module_evaluation_promise(&mut context, &entry);
    assert_eq!(first.object_id(), second.object_id());
    assert_script_true(&mut context, "__bRuns === 1");
}

#[test]
fn default_import_clauses_share_the_exporters_live_cell() {
    let runtime = Runtime::new();
    let (loader, loads, _) = MapModuleLoader::new([(
        "pkg/exporter.js",
        r#"
        export let value = 1;
        export { value as default };
        export function update() { value = 42; }
        "#,
    )]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            r#"
            import onlyDefault from "./exporter.js";
            import defaultWithNamed, { update } from "./exporter.js";
            import defaultWithNamespace, * as namespace from "./exporter.js";
            globalThis.__defaultImportBefore =
                onlyDefault === 1 &&
                defaultWithNamed === 1 &&
                defaultWithNamespace === 1 &&
                namespace.default === 1;
            try {
                defaultWithNamed = 2;
            } catch (error) {
                globalThis.__defaultImportReadOnly = true;
            }
            update();
            globalThis.__defaultImportAfter =
                onlyDefault === 42 &&
                defaultWithNamed === 42 &&
                defaultWithNamespace === 42 &&
                namespace.default === 42;
            "#,
            "pkg/importer.js",
        )
        .unwrap();

    assert_eq!(&*loads.borrow(), &["pkg/exporter.js"]);
    context.execute_module(&module).unwrap();
    assert_script_true(
        &mut context,
        r#"
        __defaultImportBefore === true &&
        __defaultImportReadOnly === true &&
        __defaultImportAfter === true
        "#,
    );
}

#[test]
fn default_function_declarations_are_hoisted_named_and_live_through_self_imports() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let anonymous = context
        .compile_module_with_filename(
            r#"
            import current from "./anonymous.js";
            const descriptor = Object.getOwnPropertyDescriptor(current, "name");
            globalThis.__anonymousDefault =
                current() === 23 && current.name === "default" &&
                descriptor.value === "default" &&
                descriptor.writable === false &&
                descriptor.enumerable === false &&
                descriptor.configurable === true;
            export default function () { return 23; }
            "#,
            "pkg/anonymous.js",
        )
        .unwrap();
    context.execute_module(&anonymous).unwrap();

    let named = context
        .compile_module_with_filename(
            r#"
            import current from "./named.js";
            export default function named() { return 23; }
            globalThis.__namedDefaultBefore =
                current === named && current() === 23 && current.name === "named";
            named = function replacement() { return 42; };
            globalThis.__namedDefaultAfter =
                current === named && current() === 42 && current.name === "replacement";
            "#,
            "pkg/named.js",
        )
        .unwrap();
    context.execute_module(&named).unwrap();

    assert_script_true(
        &mut context,
        r#"
        __anonymousDefault === true &&
        __namedDefaultBefore === true &&
        __namedDefaultAfter === true
        "#,
    );
}

#[test]
fn anonymous_default_generator_and_async_declarations_receive_the_default_name() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let generator = context
        .compile_module_with_filename(
            r#"
            import current from "./generator.js";
            globalThis.__defaultGenerator =
                current.name === "default" && current().next().value === 42;
            export default function* () { yield 42; }
            "#,
            "pkg/generator.js",
        )
        .unwrap();
    context.execute_module(&generator).unwrap();

    let async_function = context
        .compile_module_with_filename(
            r#"
            import current from "./async-function.js";
            globalThis.__defaultAsyncFunction = current.name === "default";
            export default async function () { return 42; }
            "#,
            "pkg/async-function.js",
        )
        .unwrap();
    context.execute_module(&async_function).unwrap();

    let async_generator = context
        .compile_module_with_filename(
            r#"
            import current from "./async-generator.js";
            globalThis.__defaultAsyncGenerator = current.name === "default";
            export default async function* () { yield 42; }
            "#,
            "pkg/async-generator.js",
        )
        .unwrap();
    context.execute_module(&async_generator).unwrap();

    assert_script_true(
        &mut context,
        r#"
        __defaultGenerator === true &&
        __defaultAsyncFunction === true &&
        __defaultAsyncGenerator === true
        "#,
    );
}

#[test]
fn default_class_declarations_keep_tdz_and_name_before_static_initializers() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let anonymous = context
        .compile_module_with_filename(
            r#"
            import Current from "./anonymous-class.js";
            try {
                typeof Current;
            } catch (error) {
                globalThis.__anonymousClassTdz = error instanceof ReferenceError;
            }
            export default class {
                static observedName = this.name;
            }
            globalThis.__anonymousClassName =
                Current.name === "default" && Current.observedName === "default";
            "#,
            "pkg/anonymous-class.js",
        )
        .unwrap();
    context.execute_module(&anonymous).unwrap();

    let named = context
        .compile_module_with_filename(
            r#"
            import Current from "./named-class.js";
            export default class Named {}
            globalThis.__namedClassBefore = Current === Named && Current.name === "Named";
            Named = 42;
            globalThis.__namedClassAfter = Current === 42;
            "#,
            "pkg/named-class.js",
        )
        .unwrap();
    context.execute_module(&named).unwrap();

    let static_name = context
        .compile_module_with_filename(
            r#"
            import Current from "./static-name-class.js";
            export default class { static name() { return "name method"; } }
            globalThis.__staticNameMethod = Current.name() === "name method";
            "#,
            "pkg/static-name-class.js",
        )
        .unwrap();
    context.execute_module(&static_name).unwrap();

    assert_script_true(
        &mut context,
        r#"
        __anonymousClassTdz === true &&
        __anonymousClassName === true &&
        __namedClassBefore === true &&
        __namedClassAfter === true &&
        __staticNameMethod === true
        "#,
    );
}

#[test]
fn imported_mutable_cell_has_an_immutable_importer_view() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([(
        "pkg/exporter.js",
        "export let value = 1; export function update() { value = 42; }",
    )]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            r#"
            import { value, update } from "./exporter.js";
            try { value = 2; } catch (error) { globalThis.__importReadOnly = true; }
            try { eval("value = 3"); } catch (error) { globalThis.__evalImportReadOnly = true; }
            globalThis.__nestedImportRead = () => value;
            globalThis.__evalNestedImportRead = eval("() => value");
            globalThis.__importBeforeUpdate = value;
            update();
            globalThis.__importAfterUpdate = value;
            "#,
            "pkg/importer.js",
        )
        .unwrap();
    context.execute_module(&module).unwrap();
    assert_script_true(
        &mut context,
        r#"
        __importReadOnly === true && __evalImportReadOnly === true &&
        __importBeforeUpdate === 1 && __importAfterUpdate === 42 &&
        __nestedImportRead() === 42 && __evalNestedImportRead() === 42
        "#,
    );
}

#[test]
fn import_declaration_collisions_match_pinned_quickjs_single_slot_semantics() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([
        ("pkg/named-let.js", "export let value = 7;"),
        ("pkg/named-const.js", "export let value = 7;"),
        ("pkg/namespace.js", "export const value = 7;"),
        ("pkg/class.js", "export default 7;"),
        ("pkg/function.js", "export function value() { return 7; }"),
        ("pkg/default-expression.js", "export default null;"),
        ("pkg/default-var.js", "export default 7;"),
        ("pkg/destructure-array.js", "export let value = 7;"),
        ("pkg/destructure-object.js", "export let value = 7;"),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            r#"
            import { value as letValue, value as letAlias } from "./named-let.js";
            let letValue = 11;

            const constValue = 13;
            import { value as constValue, value as constAlias } from "./named-const.js";

            import * as namespaceValue from "./namespace.js";
            import * as namespaceAlias from "./namespace.js";
            let namespaceValue = 12;
            export { namespaceValue as collidedNamespace };
            import { collidedNamespace as namespaceExportAlias } from "./collision.js";

            import classValue from "./class.js";
            import classAlias from "./class.js";
            class classValue {}

            import { value as first, value as second } from "./function.js";
            { var first; }
            function second() { return 2; }
            function first() { return 1; }

            import defaultFunction from "./default-expression.js";
            import defaultFunctionAlias from "./default-expression.js";
            function defaultFunction() { return 42; }

            import defaultVar from "./default-var.js";
            var defaultVar;

            import {
                value as arrayValue,
                value as arrayAlias,
            } from "./destructure-array.js";
            let [arrayValue] = [17];

            import {
                value as objectValue,
                value as objectAlias,
            } from "./destructure-object.js";
            const { answer: objectValue } = { answer: 19 };

            let readonly = 0;
            try { letValue = 90; } catch (error) { readonly += error instanceof TypeError; }
            try { constValue = 91; } catch (error) { readonly += error instanceof TypeError; }
            try { namespaceValue = 92; } catch (error) { readonly += error instanceof TypeError; }
            try { classValue = 93; } catch (error) { readonly += error instanceof TypeError; }
            try { first = 94; } catch (error) { readonly += error instanceof TypeError; }
            try { defaultFunction = 95; } catch (error) { readonly += error instanceof TypeError; }
            try { defaultVar = 96; } catch (error) { readonly += error instanceof TypeError; }
            try { arrayValue = 97; } catch (error) { readonly += error instanceof TypeError; }
            try { objectValue = 98; } catch (error) { readonly += error instanceof TypeError; }

            globalThis.__importDeclarationCollision =
                letValue === 11 && letAlias === 11 &&
                constValue === 13 && constAlias === 13 &&
                namespaceValue === 12 && namespaceAlias.value === 7 &&
                namespaceExportAlias === 12 &&
                classValue === classAlias && classValue.name === "classValue" &&
                first() === 1 && second() === 1 &&
                defaultFunction === null && defaultFunctionAlias === null &&
                defaultVar === 7 &&
                arrayValue === 17 && arrayAlias === 17 &&
                objectValue === 19 && objectAlias === 19 &&
                readonly === 9;
            "#,
            "pkg/collision.js",
        )
        .unwrap();
    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__importDeclarationCollision === true");

    let var_initializer = context
        .compile_module_with_filename(
            "import failed from './default-var.js'; var failed = 42;",
            "pkg/var-initializer-collision.js",
        )
        .unwrap();
    let snapshot = module_evaluation_snapshot(&mut context, &var_initializer);
    assert_eq!(snapshot.state, PromiseState::Rejected);
    assert!(matches!(snapshot.result, RawValue::Object(_)));
}
