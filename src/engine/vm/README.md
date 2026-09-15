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
- [published_execution_tests.rs](published_execution_tests.rs)：真实发布入口的绑定、eval、挂起与非法访问模式回归。
- [protocol.rs](protocol.rs)：执行请求、宿主协议和 VM 入口。
- [tests.rs](tests.rs)：模块回归测试。
- [unwind.rs](unwind.rs)：异常和迭代器展开。

## 已发布执行入口

RuntimeVmHost 持有 code 模块的完整只读执行快照，而非独立常量、绑定定义
和代码片段。`CallInput` 只携带本次调用的动态输入；`new_activation` 从 host
自身的快照取得代码与布局。普通与可挂起驱动共享构造契约，保持不同返回类型，
避免扩大普通递归栈帧。恢复状态由 `decode_vm_activation` 验证后封装为字段
私有的 `RootedVmActivation`，只通过其 run 消费；动态恢复检查不能由发布验证
替代。合成 host fixture 不能进入生产 published 执行入口。

普通 local/VarRef 的访问模式由发布验证保证；生产读取直接定位动态槽，
checked 读取仍在每次访问时检查 TDZ，只在构造异常时查询名字等静态元数据。
参数与局部变量共享 `read_frame_binding`/`write_frame_binding`，Captured 仍走
实际 VarRef 的读写。无 root 的合成测试保留原内部错误检查，不作为生产入口。
新增绑定指令时必须同步发布验证、动态状态测试与外部字节码反例。

新帧仍按参数/局部变量数量分配动态槽；没有引入帧池或每指令派生表。
`execute_inner` 直接执行常用绑定、字面量、简单栈操作和分支；
复杂语义处理器继续负责 JS 转换和用户回调，以限制递归帧大小。
不能将跨回调的可变 Runtime 借用移入执行快照。active-frame guard 和挂起
编码继续拥有异常/返回时的清理责任。新增执行类别时同步原处理器、分派与
架构变异测试，不能只改源码指纹。checked 写入与初始化复用发布的模式保证，仍检查实际 TDZ、const cell
和生命周期。eval 复用发布拓扑，在编译前验证实际槽与 closure cell；
异常栈、恢复和特殊初始化协议仍由原边界检查。

捕获复用由 `reuse_frame_capture` 验证实际 cell 与 descriptor 的视图关系并保留 root；
已 Captured 的局部槽不再进入完整建单元分派，首次捕获仍由父定义提供 canonical metadata。
`static_branch_target` 只消费 IfTrue/IfFalse/Goto 的已验证立即数；生产 code/host
必须来自同一 snapshot。通用 host 和无 root fixture 继续验界，异常/恢复地址不走此入口。

`pop_pair` 用一次长度门槛证明两个直接 Vec::pop；没有用户代码或回调能在
检查和移动间改变栈。单元素失败仍先消费右值、构造错误，再释放右值。
`clone_at_depth` 用尾部索引的 wrapping_sub 加一次 get 验界；过大深度会得到大于栈长的索引，仍返回原越界错误。正常取出保留 root 克隆。
