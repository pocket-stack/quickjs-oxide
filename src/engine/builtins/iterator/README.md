# 迭代器内置行为

负责 Iterator 构造函数、原型入口、包装器、同步 Iterator Helpers 及消费操作。

- [mod.rs](mod.rs)：内置对象初始化、Iterator.from、惰性辅助迭代器、消费方法及关闭顺序；包含相关回归测试。
- [entry.rs](entry.rs)：Iterator.prototype 的 Symbol.iterator 方法及 Symbol.toStringTag 访问器。
- [concat.rs](concat.rs)：Iterator.concat 的创建、恢复与关闭行为。

使用 object 的属性语义和 VM 的调用能力，迭代器持久状态由 heap 保存。异步迭代器的执行与挂起由 engine/vm 负责。
