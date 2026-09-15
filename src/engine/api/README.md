# Rust 嵌入接口

api 是 Rust 调用者使用 quickjs-oxide 的边界，导出 Runtime、Context、
Value、错误和受 root 保护的句柄。它组织编译、执行、模块、任务与诊断
入口，将请求交给引擎内各职责模块。

Runtime 的共享存储由 heap 管理，语言算法由 compiler、code、VM、
object 和 builtins 等模块实现。公有入口不复制这些算法，也不暴露内部
字节码、帧或存储结构作为另一套嵌入接口。

调用者显式注入 HostServices；应用负责输入、模块加载策略、输出和任务
驱动。ModuleLoader 的 normalize、check_attributes 与 load 回调接收
发起请求的 Context，并保持重入与 JS 抛出值的语义。

Test262 与差分测试能力通过专用 feature 暴露，不扩大默认生产接口。
跨包职责见[架构说明](../../../docs/architecture.md)，后续 VM 内部重组
继续通过这一嵌入边界提供执行能力。
