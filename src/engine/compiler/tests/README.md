# 编译器回归测试

按语法与语义主题组织编译器测试，覆盖解析、作用域、字节码生成及错误诊断。由父模块 tests.rs 注册，共用父模块提供的测试辅助函数。

这些文件仅用于测试；生产编译逻辑由 engine/compiler 的对应实现模块维护。

## 测试文件

- [array_literals.rs](array_literals.rs)
- [assignment.rs](assignment.rs)
- [async_functions.rs](async_functions.rs)
- [async_iteration.rs](async_iteration.rs)
- [calls.rs](calls.rs)
- [class_diagnostics.rs](class_diagnostics.rs)
- [closures.rs](closures.rs)
- [control_flow.rs](control_flow.rs)
- [debug.rs](debug.rs)
- [destructuring.rs](destructuring.rs)
- [dynamic_import.rs](dynamic_import.rs)
- [eval.rs](eval.rs)
- [exceptions.rs](exceptions.rs)
- [function_declarations.rs](function_declarations.rs)
- [generators.rs](generators.rs)
- [global_bindings.rs](global_bindings.rs)
- [lexical_bindings.rs](lexical_bindings.rs)
- [limits.rs](limits.rs)
- [literals.rs](literals.rs)
- [modules.rs](modules.rs)
- [object_literals.rs](object_literals.rs)
- [operators.rs](operators.rs)
- [parameters.rs](parameters.rs)
- [program_declarations.rs](program_declarations.rs)
- [raw_source.rs](raw_source.rs)
- [scopes.rs](scopes.rs)
- [statements.rs](statements.rs)
- [syntax.rs](syntax.rs)
