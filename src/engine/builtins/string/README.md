# 字符串

负责字符串相关实现，是 engine/builtins 的内部子模块。实现 ECMAScript 内置对象与函数，以及 qjs 兼容辅助行为；拥有原生调用选择器与分派。

使用 value 转换、object 语义和 VM 调用；环境能力通过 host 契约注入。

## 文件与子目录

- [regexp.rs](regexp.rs)：RegExp-backed String prototype methods.。
- [replace.rs](replace.rs)：`String.prototype.replace` and `String.prototype.replaceAll`.。
- [tests/](tests/README.md)：子模块职责与文件说明。
- [tests.rs](tests.rs)：模块回归测试。
