# RegExp

负责RegExp相关实现，是 engine/builtins 的内部子模块。实现 ECMAScript 内置对象与函数，以及 qjs 兼容辅助行为；拥有原生调用选择器与分派。

使用 value 转换、object 语义和 VM 调用；环境能力通过 host 契约注入。

## 文件与子目录

- [compile.rs](compile.rs)：Legacy `%RegExp.prototype.compile%` mutation.。
- [constructor.rs](constructor.rs)：`%RegExp%` construction and derived allocation.。
- [escape.rs](escape.rs)：Pinned QuickJS `RegExp.escape`.。
- [exec.rs](exec.rs)：Builtin and abstract RegExp execution.。
- [match_all.rs](match_all.rs)：`RegExp.prototype[Symbol.matchAll]` and RegExp String Iterator.。
- [match_protocol.rs](match_protocol.rs)：`RegExp.prototype[Symbol.match]`.。
- [mod.rs](mod.rs)：模块入口、共享接口与子模块声明。
- [prototype.rs](prototype.rs)：`%RegExp.prototype%` accessors and generic `toString`.。
- [replace.rs](replace.rs)：`RegExp.prototype[Symbol.replace]`.。
- [result.rs](result.rs)：Construction of builtin RegExp match result arrays.。
- [search.rs](search.rs)：`RegExp.prototype[Symbol.search]`.。
- [split.rs](split.rs)：`RegExp.prototype[Symbol.split]`.。
- [tests.rs](tests.rs)：模块回归测试。
