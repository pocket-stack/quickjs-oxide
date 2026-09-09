use super::*;

#[test]
fn module_loader_error_keeps_eq_with_representation_exact_exceptions() {
    assert_eq_implemented::<ModuleLoaderError>();
    let nan = f64::from_bits(0x7ff8_0000_0000_0042);
    assert_eq!(
        ModuleLoaderError::exception(Value::Float(nan)),
        ModuleLoaderError::exception(Value::Float(nan))
    );
    assert_ne!(
        ModuleLoaderError::exception(Value::Float(nan)),
        ModuleLoaderError::exception(Value::Float(f64::NAN))
    );
    assert_ne!(
        ModuleLoaderError::new("JavaScript exception"),
        ModuleLoaderError::exception(Value::String(JsString::from_static("JavaScript exception")))
    );
}

#[test]
fn module_bytecode_and_compiled_load_results_compare_by_module_identity() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let first = context
        .compile_module_with_filename("export const value = 1;", "same.js")
        .unwrap();
    let second = context
        .compile_module_with_filename("export const value = 2;", "same.js")
        .unwrap();

    assert_eq!(first, first.clone());
    assert_ne!(first, second);
    assert_eq!(
        ModuleLoadResult::Compiled(first.clone()),
        ModuleLoadResult::Compiled(first)
    );
}
