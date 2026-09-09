# 上下文操作

实现 Context 生命周期、编译、求值和可信代码读取等嵌入操作。

上下文入口负责结果与异常收尾，实际语言行为委托给 code、realm、modules 和 VM。

## 文件与子目录

- [bytecode.rs](bytecode.rs)：Read and publish the existing trusted bytecode compatibility profiles.。
- [calls.rs](calls.rs)：Execute published scripts, call functions, and construct objects.。
- [mod.rs](mod.rs)：模块入口、共享接口与子模块声明。
- [objects.rs](objects.rs)：Context-aware object creation and property operations.。
- [realm.rs](realm.rs)：Access this realm’s global objects and intrinsic roots.。
- [script.rs](script.rs)：Script compilation and evaluation, including execution options.。
- [test262.rs](test262.rs)：Feature-gated constructors used by the Test262 host.。
