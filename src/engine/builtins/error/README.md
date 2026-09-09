# 错误内置行为

负责 Error、各原生错误子类及 AggregateError 的语言行为，集中维护错误对象构造和 stack 属性生成。

- [mod.rs](mod.rs)：内置对象初始化、构造函数调用、toString、isError 与 AggregateError 迭代处理。
- [construction.rs](construction.rs)：内部错误对象创建、消息转换及堆栈捕获时机。
- [backtrace.rs](backtrace.rs)：从活动调用帧生成堆栈文本，按需定义错误对象的 stack 属性。

依赖 object 的属性操作、heap 的对象存储及 VM 的活动帧信息。Rust 嵌入边界的错误类型仍由 engine/api 负责；VM 的异常传播与展开由 engine/vm 负责。
