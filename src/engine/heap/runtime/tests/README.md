# runtime测试

保存所属模块的回归与协议验证，只在测试目标中使用。测试应验证可观察行为、失败路径和所有权收尾。

记录可以保存 realm、模块、Promise 和挂起帧的数据；语言算法仍归对应模块。存储层不实现独立的语言内置行为。

## 文件与子目录

- [accessors.rs](accessors.rs)：模块回归测试。
- [active_frames.rs](active_frames.rs)：模块回归测试。
- [arrays.rs](arrays.rs)：模块回归测试。
- [atoms.rs](atoms.rs)：模块回归测试。
- [backtraces.rs](backtraces.rs)：模块回归测试。
- [binary_apply.rs](binary_apply.rs)：模块回归测试。
- [binary_calls.rs](binary_calls.rs)：模块回归测试。
- [binary_objects.rs](binary_objects.rs)：模块回归测试。
- [binary_property_keys.rs](binary_property_keys.rs)：模块回归测试。
- [binary_publication.rs](binary_publication.rs)：模块回归测试。
- [binary_read_only.rs](binary_read_only.rs)：模块回归测试。
- [binary_scalar.rs](binary_scalar.rs)：模块回归测试。
- [binary_this.rs](binary_this.rs)：模块回归测试。
- [binary_throw.rs](binary_throw.rs)：模块回归测试。
- [boolean_objects.rs](boolean_objects.rs)：模块回归测试。
- [bound_functions.rs](bound_functions.rs)：模块回归测试。
- [calls.rs](calls.rs)：模块回归测试。
- [closures.rs](closures.rs)：模块回归测试。
- [coercion.rs](coercion.rs)：模块回归测试。
- [constructors.rs](constructors.rs)：模块回归测试。
- [debug.rs](debug.rs)：模块回归测试。
- [dynamic_functions.rs](dynamic_functions.rs)：模块回归测试。
- [dynamic_import.rs](dynamic_import.rs)：模块回归测试。
- [errors.rs](errors.rs)：模块回归测试。
- [eval.rs](eval.rs)：模块回归测试。
- [exceptions.rs](exceptions.rs)：模块回归测试。
- [function_objects.rs](function_objects.rs)：模块回归测试。
- [gc.rs](gc.rs)：模块回归测试。
- [globals.rs](globals.rs)：模块回归测试。
- [host_gc.rs](host_gc.rs)：模块回归测试。
- [host_policy.rs](host_policy.rs)：模块回归测试。
- [iterators.rs](iterators.rs)：模块回归测试。
- [lexical_cells.rs](lexical_cells.rs)：模块回归测试。
- [native_calls.rs](native_calls.rs)：模块回归测试。
- [ownership.rs](ownership.rs)：模块回归测试。
- [primitive_intrinsics.rs](primitive_intrinsics.rs)：模块回归测试。
- [properties.rs](properties.rs)：模块回归测试。
- [publication.rs](publication.rs)：模块回归测试。
- [realms.rs](realms.rs)：模块回归测试。
- [shapes.rs](shapes.rs)：模块回归测试。
- [source.rs](source.rs)：模块回归测试。
- [strings.rs](strings.rs)：模块回归测试。
- [symbols.rs](symbols.rs)：模块回归测试。
- [weak_references.rs](weak_references.rs)：模块回归测试。
