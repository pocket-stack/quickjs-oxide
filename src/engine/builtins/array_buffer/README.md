# 缓冲区

负责缓冲区相关实现，是 engine/builtins 的内部子模块。实现 ECMAScript 内置对象与函数，以及 qjs 兼容辅助行为；拥有原生调用选择器与分派。

使用 value 转换、object 语义和 VM 调用；环境能力通过 host 契约注入。

## 文件与子目录

- [data_view/](data_view/README.md)：子模块职责与文件说明。
- [data_view.rs](data_view.rs)：`%DataView%` over the ArrayBuffer-family backing stores.。
- [tests.rs](tests.rs)：模块回归测试。
- [typed_array/](typed_array/README.md)：子模块职责与文件说明。
- [typed_array.rs](typed_array.rs)：The twelve concrete TypedArray classes over the shared ArrayBuffer store.。
