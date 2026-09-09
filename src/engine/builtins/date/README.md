# 日期

负责日期相关实现，是 engine/builtins 的内部子模块。实现 ECMAScript 内置对象与函数，以及 qjs 兼容辅助行为；拥有原生调用选择器与分派。

使用 value 转换、object 语义和 VM 调用；环境能力通过 host 契约注入。

## 文件与子目录

- [calendar.rs](calendar.rs)：Gregorian calendar and time-value kernels for the pinned QuickJS `Date` intrinsic.。
- [constructor.rs](constructor.rs)：Pinned QuickJS `Date` constructor and static native handlers.。
- [format.rs](format.rs)：Pure string formatting for the pinned QuickJS `Date` intrinsic.。
- [mod.rs](mod.rs)：模块入口、共享接口与子模块声明。
- [parse.rs](parse.rs)：Byte-for-byte port of the pinned QuickJS `Date.parse` scanner.。
- [prototype.rs](prototype.rs)：Pinned QuickJS 2026-06-04 `Date.prototype` native handlers.。
