# 原子操作

负责原子操作相关实现，是 engine/builtins 的内部子模块。实现 ECMAScript 内置对象与函数，以及 qjs 兼容辅助行为；拥有原生调用选择器与分派。

使用 value 转换、object 语义和 VM 调用；环境能力通过 host 契约注入。

## 文件与子目录

- [tests.rs](tests.rs)：模块回归测试。
- [waiter.rs](waiter.rs)：Process-wide sequentially-consistent ordering and waiter coordination.。
