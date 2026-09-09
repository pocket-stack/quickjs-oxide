#[cfg(feature = "test262-host")]
use super::*;

#[cfg(feature = "test262-host")]
#[test]
fn dynamic_import_policy_rejects_precompiled_trees_before_instantiation() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function = context
        .compile("function hidden() { return import('fixture'); }")
        .unwrap();

    runtime.set_dynamic_import_bytecode_allowed(false);
    let RuntimeError::Engine(error) = context.execute(&function).unwrap_err() else {
        panic!("restricted precompiled tree did not return an engine policy error");
    };
    assert_eq!(error.kind(), ErrorKind::Internal);
    assert!(error.message().contains("dynamic-import bytecode policy"));

    runtime.set_dynamic_import_bytecode_allowed(true);
    assert_eq!(
        context
            .eval("Object.prototype.hasOwnProperty.call(globalThis, 'hidden')")
            .unwrap(),
        Value::Bool(false)
    );
}

#[cfg(feature = "test262-host")]
#[test]
fn dynamic_import_policy_vm_guard_precedes_specifier_conversion() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context
        .eval(
            "var policyConversionCount = 0;\n\
             var policyImport = function () {\n\
               return import({ toString: function () { policyConversionCount++; return 'fixture'; } });\n\
             };",
        )
        .unwrap();
    let wrapper = context.compile("policyImport();").unwrap();

    runtime.set_dynamic_import_bytecode_allowed(false);
    let RuntimeError::Engine(error) = context.execute(&wrapper).unwrap_err() else {
        panic!("restricted instantiated callable did not return an engine policy error");
    };
    assert_eq!(error.kind(), ErrorKind::Internal);
    assert!(error.message().contains("dynamic-import bytecode policy"));

    runtime.set_dynamic_import_bytecode_allowed(true);
    assert_eq!(
        context.eval("policyConversionCount").unwrap(),
        Value::Int(0)
    );
    assert!(!runtime.is_job_pending());
}
