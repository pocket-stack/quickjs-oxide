# JSON

负责JSON相关实现，是 engine/builtins 的内部子模块。实现 ECMAScript 内置对象与函数，以及 qjs 兼容辅助行为；拥有原生调用选择器与分派。

使用 value 转换、object 语义和 VM 调用；环境能力通过 host 契约注入。

## 文件与子目录

- [mod.rs](mod.rs)：模块入口、共享接口与子模块声明。
- [parse.rs](parse.rs)：Strict, allocation-direct JSON parser from pinned QuickJS.。
- [raw.rs](raw.rs)：Pinned QuickJS Raw JSON construction and unforgeable brand checks.。
- [reviver.rs](reviver.rs)：QuickJS-shaped `JSON.parse` builtin and reviver internalization.。
- [stringify.rs](stringify.rs)：Pinned QuickJS `JSON.stringify` traversal and quoting semantics.。
- [tests.rs](tests.rs)：模块回归测试。
