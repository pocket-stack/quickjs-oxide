# Promise

负责Promise相关实现，是 engine/builtins 的内部子模块。实现 ECMAScript 内置对象与函数，以及 qjs 兼容辅助行为；拥有原生调用选择器与分派。

使用 value 转换、object 语义和 VM 调用；环境能力通过 host 契约注入。

## 文件与子目录

- [all.rs](all.rs)：QuickJS's shared `Promise.all`/`allSettled`/`any` aggregate loop.。
- [convenience.rs](convenience.rs)：Promise convenience constructors and the callback-free race combinator.。
- [finally.rs](finally.rs)：`Promise.prototype.finally` and its typed internal callbacks.。
