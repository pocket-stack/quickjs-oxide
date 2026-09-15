# 代码表示与发布

code 连接编译、外部代码输入和执行，拥有指令、常量、函数与模块草稿、
位置和异常信息，以及验证后的已发布代码。修改指令契约、代码布局或
发布保证时，从这里确定编译器和 VM 共同依赖的约定。

草稿先经过对应入口的验证，再链接名称、保留运行时引用并事务性发布。
验证的对象必须是实际发布的代码；失败清理、Runtime 身份和 realm 检查
属于这条流程。静态验证不能代替挂起恢复时对实际动态状态的检查。

已发布执行快照共享不可变的代码和常量存储，并持有相应 root。当前 PC、
操作数栈和调用状态由 VM 管理。共享静态存储不代表跨 Runtime 共享身份，
也不消除调用期间对动态值的所有权责任。

[私有 binary-object 子模块](binary_object/README.md)负责固定 QuickJS
格式的读取和准入，通过独立发布桥进入同一验证、发布流程。
[栈 VM 实施设计](../../../docs/primitive-vm-implementation-plan.md)
记录后续布局与验证流程的调整。

`verify/` 拥有只读草稿验证及 `VerifiedFunction`；`bytecode_publish.rs`
和其私有绑定子模块负责链接与展平，`runtime.rs` 负责 Atom/heap 发布事务。
编译请求和源码错误边界由 `api/compile.rs` 编排。
