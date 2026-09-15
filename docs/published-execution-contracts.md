# 已发布代码与 VM 执行契约

本轮基于 PR #17 的 `b11f2be`，在同一 PR 中按发布所有权、建帧、绑定、常量/捕获和分派拆分提交。[实施计划](published-execution-plan.md)仍是范围与后续工作的入口。

## 采用的抽象

`VerifiedFunction` 消费并拥有通过对应 verifier 的确切草稿；它没有可伪造的独立 token，也不暴露可变草稿。Script、受限 ordinary BC5、eval、module 使用各自的验证入口。模块只把 `UnlinkedModuleParts` 中函数的所有权状态由草稿转为已验证，其余表仍由原 module 发布流程处理。发布器负责原有的链接、分配、roots 和失败回滚。

`PublishedFunctionSnapshot` 移至 code/executable，私有字段将 bytecode root 与只读数据放在一起。Runtime/realm 检查之后才可构造；读取投影只借出数据，不提供生产 DerefMut。测试 fixture 可以修改无 root 的数据，真正发布的 snapshot 在测试中也不可变。没有新增指令表示、常量池、opcode 分类表或逐条指令的 rooted Value。

`RuntimeVmHost` 持有整个 snapshot。`CallInput` 只表示本次调用的动态输入；`new_activation` 从 host 本身获得代码、布局、realm 与当前函数。上游 callable 与其 bytecode/capture roots 继续由现有 `bytecode_for_callable` 的生产路径提取；这不是一次全面重写 callable 类型系统。恢复仍经过 `decode_vm_activation`，返回字段私有的 `RootedVmActivation`。普通调用和可挂起调用保留不同驱动返回类型，维持原有本机栈边界。

## 静态事实如何进入执行

共同链路：来源专用 verifier → 拥有草稿的 `VerifiedFunction` → 原事务发布器 → heap 的不可变 bytecode → 持有 root 的 snapshot → 对应 host 的建帧/恢复 → VM。下表按真实符号定位，避免依赖会漂移的源码行号。

| 候选 | 静态依据与消费位置 | 本轮处置与仍然动态的部分 |
| --- | --- | --- |
| local 普通/词法访问 | `verify_unlinked_tree_with_root`、`private_elements::verify_unlinked` 验证 opcode 与定义；`get_local`、`put_local`、`set_local_uninitialized`、`get_local_checked` 消费 | 删除生产路径重复的静态模式查询；保留下标访问错误、TDZ、Direct/Captured/private 动态状态和重新进入作用域 |
| argument | 发布参数布局；同一 snapshot 初始化缺省槽；`get_argument`/`put_argument` | 与 local 复用内联的 `read_frame_binding`/`write_frame_binding`；不把参数直接槽永久化，mapped arguments 与捕获仍有效 |
| VarRef | `verify_unlinked_tree_with_root` 验证 descriptor 与读写 opcode；`get_var_ref`、`put_var_ref`、`get_var_ref_checked` | 去掉普通/checked 读取中的重复描述符模式检查；实际 root、cell、TDZ 和 live binding 不省略 |
| checked 写入、初始化、CloseLocal | 发布器拒绝 checked 写入普通/private/const 定义；验证 InitializeLocal、InitializeVarRef 和 CloseLocal 的模式 | 成功写入不再查询模式与错误名字；初始化/CloseLocal 的静态模式检查仅在合成 fixture 保留。实际 TDZ、cell const、with 值身份、捕获复用和派生构造器初始化协议保留 |
| 常量和静态名字 | verifier 区分 PushConst、FClosure、RegExp、字符串名字；发布已有 property Atom 表 | 共享 `snapshot.constant` 的下标投影，继续使用原 Atom 表。安全 Rust 的 enum match 保留；无第二份种类表，不宣称消除了所有常量分类 |
| 父子捕获 | `verify_unlinked_tree_with_root` 与 `verify_capture_flags` 验证来源、flags、名字及 FunctionName 视图；`instantiate_closure` | ParentLocal/ParentArgument 不再重复静态匹配；保留 canonical local metadata、`capture_frame_binding`、`validate_var_ref_metadata`、实际共享 cell 和失败清理 |
| eval 环境 | `verify_eval_environments`、`verify_eval_scope_topology`、专用 eval verifier；`prepare_direct_eval_environment` 与 `validate_eval_frame_bindings` | 从同一 snapshot 取环境，不再重复扫描静态拓扑与匹配名字/flags；编译前检查 caller strictness、实际槽与 closure cell 元数据。动态捕获仍由原捕获入口验证；没有缓存 eval 查找结果 |
| 静态控制流和栈 | 原栈/目标 verifier；`execute_inner`、unwind 与 decode/resume | PC 递增实验已撤回。`static_branch_target` 仅对已发布 host 的 IfTrue/IfFalse/Goto 复用目标范围证明，通用和合成 host 继续验界。`pop_pair` 合并两次长度判断，`clone_at_depth` 保留一次安全查询；非法栈仍返回原错误。异常、Gosub/Ret、恢复检查保留，验收状态见计划 |
| dispatch | immutable opcode；`execute_inner` | 常用绑定、字面量、简单栈操作与条件/无条件分支在顶层 match 直接执行。复杂语义仍委派原处理器，以限制普通递归的本机帧；PC 发布、异常处理、挂起点不变，不新增分类表 |

所有移除静态模式检查的 host 方法，对无 root 的合成 fixture 仍保留拒绝检查。该 fixture 无法进入 `execute_published`/`start_published`。这保留了内部错误契约测试，不建立生产兼容分支。

## 成本与未采用方案

snapshot 复用原 Rc 数组；构造增加固定数量引用，数据空间不随指令数新增一份表。它确实把 code/metadata 保留在 host 中，不能据此声称单个 host 更小。动态参数、locals 与 capture roots 的分配方式不变。空间成本主要是固定数量的 Rc 和 root 持有；没有新增随指令数量增长的派生表。

没有采用独立执行指令枚举、预解码类别、帧池、下标机械包装、通用访问 trait 或由多个布尔参数控制的万能绑定访问函数。现有 opcode 已表达访问模式，额外复制这些事实会增加同步与审查成本。

建帧时将已拥有的输入 bytecode root 直接交给 active frame，避免重复 clone；snapshot 保留自己的 root。

## 测试与后续修改入口

真实发布到执行的契约测试位于 `src/engine/vm/published_execution_tests.rs`；snapshot 的 Runtime 身份、root 生命周期与只读保证测试位于 `src/engine/code/executable.rs`。发布边界的拒绝规则与变异测试位于 `scripts/checks/binary_object/`。修改这些契约时同步维护对应正例、反例和模块 README。

此前候选处置不等同于全部优化完成；补充实施状态见计划。常量 enum match 是安全取得变体的操作；给 getter 改名或增加种类表不自动消除它，因此沿用单一常量池。本轮不重写栈表示或新增静态 PC 表，静态分支、双操作数范围判断及深度查询已按计划实现；最终集成状态以计划为准。

`host_bridge/eval_validation.rs` 只检查实际帧；静态反例由发布器的 `eval_super_capabilities_are_authenticated_at_publication`、`strict_script_global_eval_anchor_does_not_leak_to_strict_functions` 和环境来源/flags 测试维护，不复制第二套测试专用验证器。真实发布到执行的 eval 测试覆盖遮蔽、捕获、嵌套 eval 与 super，另有实际槽缺失和错误 cell 元数据的反例。

## 补充实施：eval 环境共享

`PublishedEvalEnvironment` 私有构造只接受 snapshot 中存在的索引，持有同一环境数组和字节码 root。`PreparedEvalEnvironment` 与 `MaterializedEvalEnvironment` 传递该视图，编译边界的身份检查比较 Rc 数组和索引；不再深拷贝 scopes/bindings 或重新比较结构。编译失败前不捕获、实际帧槽和 cell 元数据检查不变。没有新增每环境的预计算数组或动态查找缓存。

## 补充实施：已捕获局部槽

`instantiate_closure` 仅在建立新捕获单元时读取父局部定义并构造 canonical metadata。已有 `FrameBinding::Captured` 直接由 `reuse_frame_capture` 验证实际 cell 与子 descriptor 的合法视图关系并复用 root，避免再次取静态定义。CloseLocal 后的槽会重新走建立路径；FunctionName 的视图差异不能覆盖新 cell 的 canonical metadata。

## 补充实施：栈访问

`pop_pair` 在一个长度门槛后移动右、左值；检查与移动之间不调用用户代码。失败时保留先消费右值、构造错误、再释放右值的顺序。`clone_at_depth` 由 u8 深度计算的非零偏移通过 wrapping_sub 映射尾部索引；过深索引环绕到栈长以外，仍由安全 get 返回原错误。没有 unsafe、临时 Vec 或新的栈表示。切片占位值、drain 和强制内联实验增加了清理工作或造成耗时退化，因此未保留。
