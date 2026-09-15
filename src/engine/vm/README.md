# 执行与挂起

拥有宿主执行协议、完成结果、帧、指令分派、调用、异常展开、生成器和异步恢复。

执行已验证代码；heap 保存长期挂起状态；协议桥接对象语义和内置调用，不解析源码。

## 文件与子目录

- [activation.rs](activation.rs)：执行帧、挂起快照及恢复协议。
- [async_from_sync_iterator.rs](async_from_sync_iterator.rs)：Async-from-Sync iterator acquisition and Promise continuation.。
- [async_function.rs](async_function.rs)：Ordinary async-function intrinsics and suspension driver.。
- [async_generator.rs](async_generator.rs)：Ordinary async-generator intrinsics and FIFO Promise driver.。
- [call.rs](call.rs)：调用、构造与原生调用适配。
- [completion.rs](completion.rs)：完成结果、恢复输入与执行状态类型。
- [detached.rs](detached.rs)：detached 的类型和操作实现。
- [dispatch.rs](dispatch.rs)：指令或原生调用分派。
- [exception.rs](exception.rs)：待处理异常的保留与提取。
- [for_in.rs](for_in.rs)：for_in 的类型和操作实现。
- [frame_execution.rs](frame_execution.rs)：帧初始化、执行循环与操作数访问。
- [frames.rs](frames.rs)：活动帧登记、roots 和回溯屏障。
- [generator.rs](generator.rs)：Synchronous generator intrinsics and resumable activation plumbing.。
- [host_bridge/](host_bridge/README.md)：子模块职责与文件说明。
- [host_bridge.rs](host_bridge.rs)：Bytecode VM adapter and per-frame binding state.。
- [mod.rs](mod.rs)：模块入口、共享接口与子模块声明。
- [native_stack.rs](native_stack.rs)：Deterministic native-call stack budgeting.。
- [numeric.rs](numeric.rs)：共享原始值/数值转换入口、数值类型及比较辅助函数。
- [numeric_coercion_tests.rs](numeric_coercion_tests.rs)：原始值直通及对象转换顺序回归测试。
- [numeric_execution.rs](numeric_execution.rs)：数值指令执行。
- [protocol.rs](protocol.rs)：执行请求、宿主协议和 VM 入口。
- [tests.rs](tests.rs)：模块回归测试。
- [unwind.rs](unwind.rs)：异常和迭代器展开。
