# 已发布字节码与 VM 执行优化计划

> 历史归档（2026-09-12）：下文状态、任务和契约属于文中记录的旧 PR / 源码基线，不作为当前实施指令。当前设计见[架构说明](../architecture.md)和[栈 VM 计划](../primitive-vm-plan.md)；本次归档没有更新性能或验证结果。

状态：计划中的代码优化已实现，本地集成验收已完成；独立评审仍待完成，不将实施者自查算作独立评审。基于 PR #17 的 `b11f2be`，见 [PR #18](https://github.com/pocket-stack/quickjs-oxide/pull/18) 和[执行契约说明](published-execution-contracts.md)。

读者：共同维护 quickjs-oxide 的人和 agent。读完后应能选择一个依赖已满足的步骤，找到模块拥有者，说明该步骤的验证依据与动态语义，完成实现、评审、测量和交接，无需恢复聊天上下文。

## 当前实施计划与交接状态

本节是当前进度的唯一清单；第 4 节保留原步骤及其对应关系，不另行宣告完成。补齐工作以 `49cfa30` 为起点，代码已推进至 `c55d59d`，仍提交到原 PR #18 的 `perf/published-vm-execution` 分支，base 保持 `perf/indexed-data-structures`。P1–P4 的代码及本地集成验收已完成；独立评审尚未完成。

### 交付范围和顺序

按 **eval 环境共享 → 闭包捕获元数据 → 静态分支 → 栈重复操作 → 最终集成验收** 推进。四项均已实现，分步 A/B 与最终组合的本地集成验收已完成，证据入口见下方记录。已提交不等于已验收，阶段性收益不等于全计划完成。

| 项目 | 代码入口与具体改动 | 保留的动态语义与抽象边界 | 当前状态与下一步 |
| --- | --- | --- | --- |
| eval 环境共享（E06） | `code/executable.rs` 的 `PublishedEvalEnvironment` 共享已发布环境数组及索引，并持有字节码 root；`prepare_direct_eval_environment`、`materialize_direct_eval_environment` 与 `builtins/eval.rs` 传递该视图，用身份核对替换两次深拷贝及结构比较 | 私有构造检查索引；不缓存动态名字解析或实际槽；编译成功后才捕获，保留 strictness、实际槽和 cell 检查。静态规则只由发布器维护，旧测试专用验证器已在 `ed80e61` 删除 | 已实现并完成本地验收：`4b7bd08` 共享视图，`64785fa` 加强最后一个 root 释放测试；深拷贝与结构比较已移除。指令工作减少，最终耗时未证明稳定收益 |
| 闭包捕获元数据（E06） | `host_bridge.rs::instantiate_closure` 对已有 Captured 槽直接复用实际 cell；仅首次创建或 CloseLocal 后重新捕获才读取父局部定义、构造 canonical metadata | 由 `reuse_frame_capture` 统一已有 cell 的验证与复用；保留 cell/descriptor 合法视图验证、root 所有权及清理。FunctionName 的访问视图不能覆盖新 cell 的 canonical metadata；不复制完整捕获描述符表 | `d0d329e` 实现元数据复用，`64785fa` 消除随后重复的状态分派。十轮复核未复现初测的调用控制组大幅退化，但没有证明独立的耗时收益；最终组合已完成正确性、诊断负载和调用控制验证 |
| 静态分支（E07） | `VmHost::static_branch_target` 的默认实现验界；`RuntimeVmHost` 复用与自身 snapshot 配对的发布证明；`frame_execution.rs` 仅将 IfTrue/IfFalse/Goto 的立即数目标交给该入口 | 发布器检查所有目标，包括不可达指令；无 root 的合成 host 保留验界。异常展开、Gosub/Ret、恢复 PC 和取指检查不使用该入口；不新增可伪造的信任标志或 PC 表 | `7521630` 已实现并完成提交后的 A/B 和指令数确认；`64785fa` 补充通用/合成 host 反例、恢复组合及两个架构变异。最终隔离复测确认循环收益和指令减少；算术控制耗时有代价，未宣称该项全面提速。最终本地正确性验收已完成 |
| 栈重复操作（E07） | `frame_execution.rs::pop_pair` 用一个长度条件证明两次移动；`clone_at_depth` 用 wrapping_sub 与一次安全 get 代替两次验界 | 保持现有 Vec/Value 表示，不引入 unsafe。保留空栈和单元素失败时的消费、释放顺序，不能以 panic 替代内部错误；动态恢复检查保留 | `800242c` / `a3c988c` 已实现，最终 `c55d59d` 保留简单长度门槛和一次深度查询。机器码确认重复判断减少；cold/内联、切片占位值和 drain 实验不采用。最终本地集成验收已完成 |
| 最终集成（E09） | 在最终采用的代码上验证各优化组合及控制组，清除实验残留，更新本节和模块契约 | 所有入口沿用同一发布拥有权与帧配对契约；不复制静态验证器、opcode 表或依赖人工同步的元数据 | 本地集成已完成：最终 workspace/feature、Clippy、701 个架构变异、完整 Test262 行为比对、Node/WASM 及固定控制矩阵已验证。独立评审仍待完成 |

### 每项验收

1. **eval**：环境身份与错误索引、root 生命周期、嵌套 eval、with 遮蔽、super、编译失败前不捕获；使用 `eval-wide`、`eval-capture`，加无 eval 的调用/循环控制组。共享结构的收益不能只靠 getter 或结构相等测试证明。
2. **闭包**：首次捕获、同槽反复捕获、参数捕获、FunctionName/导入视图、私有绑定、循环 CloseLocal、逃逸与重入；分别测 `closure-create`、`argument-capture`、`lexical-lifetime` 和 `func_closure_call`。十轮控制组复核已经完成，最终组合的耗时与指令数已记录在本地证据中。
3. **静态分支**：发布拒绝不可达坏目标、通用 host 拒绝越界、合法边界目标、分支与 finally/异常/恢复组合，以及保护执行入口和路由的架构变异；测空/递增/递减循环、整数运算、普通与闭包调用。
4. **栈**：空栈、单元素、双元素顺序，带 root 的值在成功/失败路径上的释放与异常清理；检查生成代码是否实际合并范围判断，再测整数运算、比较、Swap/Nip 及调用控制。不能只把 `.get()` 改成下标并称为优化。
5. **集成**：按第 5 节完成最终 workspace/feature 组合、严格 Clippy、全部架构变异、Test262 focused/full、Node/WASM 和 50+8 固定控制矩阵。冻结结果不改写；若只有引擎指纹变化，单独核对后明确说明。以前提交的验证不替代最终代码验收。

### 明确保留、不重复实施的部分

- **常量投影（E05）**：保留单一常量池和安全 enum match；已有静态属性 Atom 表继续使用。统一访问入口属于结构整理，不能记为常量分类已消除。不为去掉一次 match 引入复制常量池或逐指令类型表；rooting、新闭包和 RegExp 对象创建仍需执行。
- **PC 递增**：此前 checked_add 简化已有重复测量证明退化，保持回退，不重做同一实验。该结论不适用于静态分支目标的重复验界。
- **动态检查和表示**：保留 TDZ、实际捕获状态、参数别名、cell const、引用管理、动态返回地址、异常展开与恢复验证。不进行全栈表示重写、独立执行指令枚举或静态 PC 表建设；这不免除上表局部栈操作的检查合并工作。

### 提交、测量与完成规则

- 所有工作继续进入原 PR #18，一个可独立解释的优化一个小提交。提交前运行受影响的正确性检查；**只有实际优化提交之后才做 benchmark**，准备性抽象和纯文档提交不另做性能测量。
- 每项至少五轮交错 A/B，保留正确输出准入、耗时及 instructions/cycles；对无法区分的波动和疑似退化追加双方样本并调查。某个方案失败只撤回该方案，不自动关闭尚未探索的方向。
- 原 PR #17 的 `b11f2be` 用于整体比较；补齐阶段以重新构建的 `49cfa30` 为起点，各步骤再与直接前序比较。构建须核对源码、工具链、参数及 ELF 哈希；旧 receipt 与二进制不符时禁止直接测量或改写 receipt 冒充验证。
- 本轮原始数据仅保存在忽略的 `target/published-follow-*`，不提交 benchmark 结果文件。本文件记录方案、状态和验证入口，不复制测量表。更换工作区时须重新生成数据，不能假定本地 target 可用。
- 状态分别写“已实现”“已运行验证”“性能待确认”“已验收”或“保留且未优化”。只有所有待做项都有实现与验收证据、明确保留项没有被算成收益、最终集成完成，才可宣告交付完成。不得用“候选均有处置”替代这一条件。

### 可执行任务分解

以下任务号记录该阶段的提交说明与交接；复核时定位对应拥有者的源码契约。任务状态以上方清单为准；下列步骤保留实现和复核入口，不以修改任务描述代替实际优化。

#### P1：完成 eval 共享视图的验收

**输入与拥有者**：`4b7bd08`；`code/executable.rs`、`vm/host_bridge.rs`、`vm/host_bridge/eval_validation.rs`、`builtins/eval.rs`。已有类型为 `PublishedEvalEnvironment`，不再新增“环境计划”包装层。

实施检查：

1. 检查构造入口只从 snapshot 取得有效索引；私有字段必须同时保存只读环境数组、索引和对应字节码 root。构造失败不应产生可用视图。
2. 确认 `PreparedEvalEnvironment → 编译 → MaterializedEvalEnvironment` 移动或共享同一视图；比较只能验证同一数组和索引，不重新逐项比较名称、flags 和拓扑。
3. 确认编译前没有创建 VarRef；编译失败时视图与临时引用被释放。编译成功后才能捕获当前帧的真实绑定。
4. 搜索这条路径上的 `EvalEnvironment::clone` 和 descriptor 克隆，区分 Rc/root 引用复制与 scopes/bindings 深拷贝。若仍有深拷贝消费者，逐一说明用途或迁移；不以“已加 Rc 类型”作为验收依据。
5. 检查静态规则由发布验证层（含 `bytecode_publish` 调用的 `bytecode_validation`）统一拥有；VM 测试不得再保存一份静态验证算法。

测试落点：`executable::tests` 验证共享身份、不同索引、不同数组、root 存活及越界拒绝；`published_execution_tests` 验证同一环境多次 eval 仍观察新值、with 遮蔽、嵌套 eval、super；现有编译失败/捕获顺序测试必须通过。新增测试只覆盖现有测试未证明的契约，不复制 getter 实现。

采用条件：深拷贝与结构比较已实际移除；没有新增按绑定数复制的缓存；正确性通过；eval 目标负载指令或分配工作减少，且控制组无未解释退化。耗时无法区分时明确标为未证明耗时收益。需要修正时单独提交 `perf(eval): ...`，纯验收不制造空提交。

#### P2：核实并修正捕获复用路径

**输入与拥有者**：`d0d329e`；`host_bridge.rs::instantiate_closure`、`capture_frame_binding`、`close_frame_binding`；`published_execution_tests.rs`。优先处理初步测量中闭包调用控制组的异常变化。

实施检查：

1. 对 ParentLocal 按实际 FrameBinding 分两种职责：已有 Captured 只验证并复用 cell；Direct、Uninitialized、Private 等首次捕获状态取得父定义，用 canonical flags 建立 cell。
2. 保持 ParentArgument、ParentClosure、ParentGlobal 各自的来源语义，不能为了共用函数把所有来源伪装成局部槽。
3. 对 FunctionName 的可变访问视图以及 ModuleImportView，检查实际 cell 验证仍允许发布器认证的差异；首次创建必须使用 canonical metadata，不能直接复制视图 flags。
4. 检查新增状态分支是否随后又进入相同的完整状态 match，抵消已省的定义访问。若反汇编或诊断负载证实重复分派有成本，可提取小型“验证并复用已有 cell”函数供真实调用者共享；不引入布尔模式参数、通用 capture trait 或完整副本表。
5. 检查借用在任何可能重入的运行时操作前结束；错误与中途失败不得泄漏已捕获 roots。CloseLocal 后必须重新进入首次捕获路径。

测试落点：在发布到执行测试中分别覆盖第一次捕获、同槽两个闭包共享更新、循环每轮独立 cell、命名函数与直接 eval 共用 cell、私有字段/方法。保留模块 live-binding 与错误 cell 元数据的既有边界测试。

测量步骤：先对 `4b7bd08 → d0d329e` 的上述四个负载补十轮交错 A/B，双方使用相同机器、CPU 与普通 release 构建；同时比较 instructions/cycles，检查退化是否可复现。若复现，先定位代码布局或新增分派成本，实施一个具体修正后提交并重新五轮 A/B。不能用目标微负载略少指令掩盖调用控制组变慢。若最终撤回，保留不采用该实现的证据，明确首次/重复捕获仍有哪些工作，而不是宣布捕获方向全部完成。

#### P3：验证静态分支确实减少运行时工作

**输入与拥有者**：`7521630`；`bytecode.rs::validate_target`、`protocol.rs::VmHost`、`host_bridge.rs::static_branch_target`、`frame_execution.rs::execute_inner`。

证明链必须逐点核对：

1. verifier 对所有 IfTrue/IfFalse/Goto 操作数执行范围验证，不能只检查可达块。
2. `execute_published`、`start_published` 经 `new_activation` 取得同一 host 的代码和布局；恢复路径经 `decode_vm_activation` 保持对应关系。任何能混配代码与 host 的生产入口都必须先封闭，不能靠注释假定不存在。
3. `VmHost` 默认入口继续调用受检目标转换；Runtime host 仅复用已发布目标范围证明。无 root 的测试 fixture 必须继续拒绝坏目标。
4. 搜索新入口的全部调用者，只允许三个立即数分支使用；Catch/Gosub/Ret、异常 region、恢复 PC 继续用原检查。u32 到 usize 的必要转换保留，不能通过截断扩大平台假设。
5. 检查生成代码中 Runtime host 路径的目标范围比较确实消失，且未新增每次指令的信任标志分支或间接调用。

测试落点：现有 `static_branch_targets_remain_checked_at_untrusted_boundaries` 要断言明确的目标错误，防止因 max_stack 等无关错误“通过”；补合法首/尾目标、两个条件方向、finally 内分支、挂起后分支及回溯 PC 的缺口。架构规则不仅更新 hash：执行现有 tail/throw 路由变异，确认错误拦截仍被拒绝；补充“让通用/合成目标入口不验界”的反例覆盖。

提交后测量：以 `d0d329e` 为直接前序，对 `7521630` 做至少五轮 loop/int/call/closure A/B 和 instructions/cycles。若 P2 后续修正或撤回，在最终前序版本上重新比较，不能仅沿用这组历史结果验收。若发现可复现退化，调整具体实现或撤回它；不使用已失败的 PC 递增实验代替本步骤测量。

#### P4：实现一次范围判断的双操作数取出

**输入与拥有者**：最终采用的 P3 版本；先检查现有 release 机器码是否仍有重复范围判断，再决定是否实施候选算法。`frame_execution.rs::pop_pair`，消费者为同文件 Nip/Swap、`numeric_execution.rs` 和 `dispatch.rs` 的二元操作。先改共同拥有者，不逐 opcode 复制快路径。

当前语义是先 pop 右值，再 pop 左值，返回 `(left, right)`。新实现必须保留下面的可观察状态：

| 输入栈 | 返回与剩余状态 |
| --- | --- |
| 空栈 | 返回原 underflow 错误，栈仍为空 |
| 单元素 | 消费并释放该元素，返回原 underflow 错误，栈为空 |
| 至少两元素 | 返回原顺序的左右值，前缀不变；取出的 roots 不提前释放 |

实际实现方案：先统一检查 `len >= 2`，成功后直接使用两次 Vec::pop 移出右、左值，不再分别构造 VM 的 Result；失败分支保留原来的消费、错误构造及释放顺序。这个动态范围检查证明两个 expect 不会失败，非法输入仍返回原内部错误。备选切片匹配加 mem::replace/truncate 不先采用，避免引入占位值写入/释放；所有移动使用安全 Rust，不使用 unchecked/set_len，不新增临时 Vec，不 clone Value。最终以生成代码是否合并范围判断及实测决定采用。

实施顺序：

1. 先为上表三个状态增加直接行为测试，包含对象或其他持有 root 的值，检查失败释放及成功后值仍存活。
2. 修改单个 `pop_pair`，让所有现有消费者自然使用；不同时改变算术或属性语义。
3. 检查普通二元运算、Swap/Nip、属性写入和异常路径；保持用户转换回调发生时机不变。
4. 检查 release 反汇编中的范围判断、Value 移动及 drop 数量。若两次 Vec::pop 的成功条件未被消除，再比较局部切片分解的等价实现，不引入独立栈类型；仍不成立时记录此方案不采用。
5. `clone_at_depth` 的基线 release 机器码仍有两个范围分支；最终使用 len.wrapping_sub(usize::from(depth) + 1) 后一次 get；depth 为 u8，减数非零且不会溢出 usize，过深时环绕索引大于 len，由同一个 get 拒绝。穷举全部 u8 深度与空栈、255/256/257 长度边界，验证值、错误、栈不变及 root 克隆；release 机器码只保留一个范围条件。

验证负载：整数与浮点运算、比较、Swap/Nip 专用循环，外加调用与属性访问控制组。新增诊断脚本固定循环次数和输出，并在第一次 A/B 前冻结哈希；结果不代替完整控制矩阵。已完成上述实现及分步测量。切片占位值方案保留了额外写入和析构循环；drain 保留额外边界与清理；cold/强制内联策略改变调用布局并造成耗时退化，均已撤回。最终采用普通辅助函数和局部错误路径，未添加内联策略。

#### P5：最终集成、文档与独立评审

依赖 P1–P4 的最终采用版本，不依赖旧的“候选处置完成”结论。

1. 清点每项最终减少的操作，注明只属于维护性改善或必要保留的部分。检查没有测试专用静态验证器、双 opcode 表、可伪造凭据或临时兼容入口残留。
2. 按下面命令入口执行最终正确性验收；所有失败都要定位到本次变更或冻结基线，不能直接更新 oracle/Test262 期望。完整矩阵按最终采用代码重建，构建和测试不得与测量争抢同一机器。
3. 比较完整 50 microbench + 8 V8 控制组，按固定输出准入；与 `49cfa30` 比较补齐部分，与 `b11f2be` 比较累计效果。不得将固定工作量的比率称为上游 adaptive score。
4. 独立评审重点为 eval 视图原子生命周期、FunctionName/cell 视图差异、静态分支的代码配对及栈错误路径的释放语义。自查单独记录，不冒充独立评审。
5. 在本文件更新任务状态、提交和未解决项，在所属源码契约与架构说明中记录最终职责。PR 描述同步实际范围；原始 benchmark 文件不提交。没有满足本任务时保持“集成未完成”。

### 最终本地验收记录

实施者：Codex；生产代码为 `c55d59d`，与 `04bf79d` 的 src/Cargo 输入完全一致。最终普通 release 二进制重新构建，并与此前保留副本逐字节核对；后续文档提交不改变生产源码。

- Rust 1.88.0：workspace/all-targets（引擎 1,969 项）、profiling、doc、test262-host 组合及五组严格 Clippy 通过；额外 65K 参数边界测试通过。14 个架构规则测试、16 个 benchmark 工具测试，以及源码、registry、布局、rust-only 和 runner provenance 门禁通过。
- 架构：完整 701 个变异全部被拒绝，包括新增的通用/合成静态目标验界反例。没有只更新指纹后跳过变异测试。
- Test262：完整 102,037 条向量中，79,982 pass、原有 50 fail、3,530 unsupported、18,475 skipped 不变。原 full 命令因源码指纹变化返回 receipt checksum drift；只规范化唯一引擎指纹字段后，完整 TSV/JSONL 哈希与冻结记录相同。从本次 full 中逐列核对的 6,844 个 focused 条目全部一致；没有更新冻结基线。
- Node/WASM：15 个 playground 示例、direct eval、指标与构建元数据、可捕获的深层 yield-star 溢出通过。
- 性能：优化提交后执行分步 A/B、最终三版本 50+8 控制矩阵和 8 个诊断负载；最终 990 个样本均通过输出准入。疑似退化项追加双方各十轮；单独复核最终代码中的静态分支和栈改动，并对静态分支的算术/调用控制追加十轮。静态分支隔离对照的算术耗时仍上升，保留该实现的依据是目标循环收益与指令工作减少；最终完整组合的算术耗时仍低于本轮起点，不能用这一点掩盖单项代价。累计优化改善固定工作量耗时，本轮补齐的综合耗时基本持平；闭包写入、数组操作及宽 eval 环境仍有小幅耗时代价，不据此宣称全面提速。常量分类和 PC 递增保留项没有计入已消除的工作。
- 证据在忽略的 `target/published-follow-*`：`validation.log`、`boundary-full.log`、`test262-full.log`、`test262-comparison.json`、`web.log`，以及 `complete-ab`、`complete-recheck`、`static-control-ab`、`static-control-recheck`、`final-stack-recheck` 与各 `*-counters.json`。文件前缀均为 `published-follow-`；原始报告不进入 PR。迁移工作区须按下方入口重建并复测。

实施者已自查构造封闭性、实际 cell 视图、eval root 生命周期、静态目标配对和栈失败释放顺序。独立评审仍待完成；这是剩余评审事项，不是未实施的优化，也不以自查代替它。

### 验证与复现入口

命令从仓库根目录执行。下列定向命令对应 P1–P4；工作区集成组合以 README 和 `.github/workflows/ci.yml` 为准，不在计划里维护第二份完整 CI 配置。

```sh
cargo fmt --all -- --check
cargo +1.88.0 test --locked -p quickjs-oxide --lib published_execution_tests
cargo +1.88.0 test --locked -p quickjs-oxide --lib eval_
cargo +1.88.0 test --locked -p quickjs-oxide --lib
cargo +1.88.0 clippy --locked -p quickjs-oxide --lib -- -D warnings
./scripts/checks/check-binary-object-boundary.sh --scan-only "$PWD"
PYTHONPATH=scripts/checks python3 -m unittest discover -s scripts/checks/binary_object/tests
./scripts/checks/check-binary-object-boundary.sh
TEST262_WORKERS=2 ./scripts/test262/test-test262.sh --full
./scripts/web/test-web-playground.sh
```

源码指纹变化时，冻结的 `--focused` 入口会拒绝运行。此时执行 `--full`，再按 `(path, variant)` 从完整结果逐列核对冻结 focused 的全部条目；完整 TSV/JSONL 只规范化唯一引擎指纹字段后比较冻结哈希，不能改写冻结记录或忽略行为列差异。

性能入口使用 `scripts/benchmark/fixed.py`。当前本地 manifest 为 `target/published-follow-manifest.json`，仅适用于哈希校验仍通过的工作区；迁移机器必须从固定负载来源重新生成。先构建已提交版本，分别复制为不可变 before/after 二进制并记录 receipt，再执行，例如 P3：

```sh
python3 scripts/benchmark/fixed.py \
  --manifest target/published-follow-manifest.json \
  --engine before=/absolute/before/qjs --engine after=/absolute/after/qjs \
  --case empty_loop --case int_arith --case func_call --case func_closure_call \
  --repeat 5 --cpu 2 --output target/published-follow-branch-ab
```

CPU 2 是当前机器的绑定设置；更换机器须重新记录实际 CPU。另用 `perf stat -x, -e instructions:u,cycles:u -- taskset -c 2 ...` 对同一冻结负载按相同轮次交错采样，逐样本验证退出码和标准输出，不混入 profiling 特性或后台构建。当前 P1/P2 的本地证据位于 `target/published-follow-eval-*`、`target/published-follow-capture-*`；P3/P4 不得引用这些结果声称自己的优化已经测量。

## 1. 目标与基线

将发布阶段确认的静态事实可靠地传递给 VM，减少绑定访问、常量访问、闭包创建与控制流中的重复工作。共同建立执行契约，分别验证优化收益；可维护性、可读性与语言兼容是验收条件。

计划依据为 [Issue #16](https://github.com/pocket-stack/quickjs-oxide/issues/16)、[PR #17](https://github.com/pocket-stack/quickjs-oxide/pull/17)，以及本地 `b11f2be` 的源码与报告。实施时核对 PR 状态和基线提交；这里不假定 PR 已合入。上一轮范围见[数据结构计划](data-structure-plan.md)。

- [最终硬件与采样报告](../performance/README.md)：空循环 `read_frame_binding` 自身成本为 15.1–23.8%，hot/numeric dispatch 与执行循环也占明显比例；优化后的空循环约执行 110.47 亿条指令，QuickJS 约 5.62 亿条。
- [最终固定工作量报告](../performance/README.md)：保留全部 50 项 microbench 和 8 项 V8 子套件，作为控制组；不能只选此次优化收益最大的程序。
- [数值栈与调用参数报告](../performance/README.md)：数值原地更新有收益，参数借用的调用结果基本持平；不能据此直接优先引入帧池。

上述数字属于历史测量，不是本计划的基线复测结果。Issue #16 与 PR #17 的固定源文件组织不同，绝对时间不能直接拼接。采样占比不能当作可取得的加速幅度。

本轮不包含 JIT、unsafe、新的公开字节码格式、全局 Value 表示重写、帧池或另一套 GC。属性 inline cache 和一般对象语义优化也不混入本轮。

## 2. 已发现的问题与检查归属

规划时的基线发布验证已覆盖多种指令和元数据关系，但 `VerifiedBytecode` 只返回 `max_stack`；`PublishedFunctionSnapshot` 将代码、常量和定义分别交出；`CallInput` 与 VM 执行入口仍接受指令切片及独立 host。现有生产调用链具有约定，不等于类型已保证代码与帧来自同一发布对象。

下表是检查迁移候选，不是删除检查的授权清单。实施前逐项补全所有生产入口、证明范围和反例。尤其不能把相似错误字符串当作两次检查等价的证据。

| 领域 | 发布时的静态事实 | 建帧或恢复时的保证 | 执行时必须保留 |
| --- | --- | --- | --- |
| 局部变量 | 下标与定义匹配；普通、词法、private/with 等访问是否合法；写指令权限 | locals 布局来自对应代码；初始状态与定义一致 | TDZ、作用域重新进入、Direct/Captured 变化、初始化次序 |
| 参数 | 指令下标与形参布局匹配 | 形参槽补 Undefined；实际参数数独立保留 | mapped arguments、捕获、实际参数语义；参数路径本来较短，不假设有同等重复检查 |
| 闭包与全局描述符 | 指令访问模式与描述符匹配；静态 const/特殊绑定限制 | 实际 closure slots 数量、来源和必要 cell 元数据与代码匹配 | TDZ、live binding、全局属性行为、实际 cell 状态与引用所有权 |
| 常量与名字 | PushConst/FClosure/RegExp/字符串名字操作数的种类和范围 | 常量与 Atom 受同一发布对象的 roots 保护 | rooting、新对象创建、属性访问与异常语义 |
| 闭包创建 | 父子捕获来源、名称、flags 和特殊视图关系合法 | 当前父帧与捕获计划相符 | 建立/复用实际 VarRef，捕获逃逸、失败回滚与引用管理 |
| 静态控制流 | 分支目标合法；可达路径不越出代码；栈效果、汇合与特殊标记协议 | 初始 activation 符合布局；恢复 PC、栈与 regions 有效 | 动态返回地址、异常展开、挂起恢复、backtrace PC |
| eval 环境 | 描述符拓扑、静态名字与来源、能力来源合法 | 当前调用方身份、环境与实际引用匹配 | direct-eval 身份门槛、动态查找、遮蔽、重入与活引用 |
| dispatch | 指令类别由不可变 opcode 决定 | 执行视图与代码一致 | 每次操作的转换、异常、暂停与调用行为 |

每项候选建立一条简短记录：静态事实 → 验证函数 → 发布入口 → 不可变拥有者 → 建帧/恢复约束 → 使用者 → 拒绝反例。不能完成此链条的检查继续保留。

## 3. 统一抽象与职责

职责边界的当前入口为[工作区架构](../architecture.md)。以下类型名为设计名称，实施前按真实调用者定稿；不要求机械地新增三个包装类型。

```text
编译器 / 受限 BC5 解码 / 模块与 eval 草稿
    → 验证、链接、事务发布
    → PublishedExecutable（不可变代码与执行布局）
    → 建帧或恢复边界
    → ExecutionFrame（对应代码与动态状态）
    → VM 执行
```

### 3.1 已发布代码

由 code 模块拥有不可变执行描述与受限构造入口，将指令、常量布局、绑定布局和必要派生计划作为一个整体提供。优先收拢现有 snapshot 和 bytecode 数据，而不是再复制一套完整数据。

- 只允许成功完成相应验证与链接的生产发布路径构造；覆盖 Script、模块、eval、动态函数和受限 BC5，不扩大现有解码信任范围。
- 验证凭据必须关联实际被验证的数据。禁止公开可伪造的布尔标志、脱离所属函数的通用 token，以及验证后可变更代码的别名。
- 现有 heap 节点仍拥有强边和 Atom 引用。执行视图保留已有字节码 root，派生表默认不额外拥有 JS roots；如需例外，明确 retain/release/trace 与回滚。
- Runtime 身份、realm 和实际 heap 对象的检查仍由相应边界负责。静态结构合法不代表某个运行时句柄有效。

### 3.2 帧布局与绑定访问

由 VM 拥有帧的可变状态，构造时从同一 executable 推导布局，避免调用方手工组合 code、metadata、locals 与 host。借用或拥有的实现选择须满足递归、重入和挂起；不能跨用户代码持有 Runtime 的 RefCell 借用。

统一 local/argument/closure 的访问契约和共享值读写机制，但保留语义不同的明确入口，例如普通读取、词法检查读取和初始化。不要用多个布尔参数组成一个万能访问函数，也不把全局属性查找伪装成直接槽读取。

静态访问权限与动态存储状态分开：发布时知道普通读取合法，不意味着变量永远是 Direct；捕获和 eval 可以改变其存储。TDZ、初始化、CloseLocal 复用与别名关系不能被静态标签覆盖。

### 3.3 常量、捕获与环境计划

优先复用现有 opcode 表达的访问模式及已链接的属性 Atom。确有重复解析时，由 code 模块在发布时生成紧凑的种类明确的操作数或计划，VM 只应用计划。

- 常量计划不重复 intern、不复制完整常量池，也不为每条指令创建独立的 rooted Value。
- 捕获计划记录静态来源和必要的 canonical metadata；保留 FunctionName、ModuleImportView 等合法视图差异，不能简单要求所有 flags 完全相同。
- eval 计划仅描述静态部分，不缓存动态名字解析结果或跨回调保存可能失效的槽位置。
- 派生数据与源数据只有一个构建拥有者，不能由调用者同步维护。空间、发布时间与销毁成本列入验收。

### 3.4 控制流与执行循环

静态跳转与动态恢复分开处理。已验证的目标可考虑在发布时转换为内部 PC 表示；保持与原指令 PC 的对应关系，不破坏源码定位、回溯、异常和暂停位置。

栈验证不会自动证明任意恢复快照合法；取消栈检查之前必须覆盖 handlers、regions、Gosub/Ret、yield/await、异常路径。安全 Rust 的新类型不会自动消除边界检查，`.get()` 改成 `[]` 也可能只是将错误变为 panic，不算优化依据。

dispatch 分类可统一拥有，先评估单一明确分派结构；仅在实测支持时增加预解码类别。不要让类别表与执行 match 靠人工双向同步。普通完成与可挂起执行共享内部契约，但保留各自驱动返回类型，避免扩大每层普通递归的本机栈帧，维持已有两 MiB 栈回归边界。

### 3.5 可读性与抽象选择

- 使用具体类型、受限可见性和少量明确入口；不新增独立 crate、插件注册层、策略框架或仅为此次改动服务的通用 trait。
- 保留现有 VmHost 的语义桥接职责，按真实生产与测试调用者评估边界，不为减少参数再造一层 host。
- 不默认复制整套 Instruction 枚举。比较“收拢现有数据”“紧凑派生计划”“独立执行指令”三种方案，优先第一种，按证据局部采用第二种；第三种必须证明额外代码量和映射成本值得。
- 不给每个整数机械地加新类型。类型须防止真实误用；LocalSlot 与 ArgumentSlot 可以区分，但裸下标包装本身不能证明所属函数身份。
- 按验证、发布存储、绑定访问、帧构造、恢复等职责组织模块，避免继续堆入巨型 host bridge 或转移到通用 utils。
- 注释解释证明依赖、回调边界、所有权和例外原因，不逐行复述操作。重要不变量应由私有字段、构造器和行为测试共同落实。

## 4. 实施步骤与依赖

实现者：Codex。下表保留原计划范围，并记录实际处置；具体证明链与保留项见[执行契约说明](published-execution-contracts.md)。本 PR 保留代码、测试和维护说明，不提交本轮 benchmark 测量结果。准备工作不等于性能收益，独立评审尚未完成。

| 步骤 | 依赖 | 原计划交付与范围 | 完成条件 | 实际处置 |
| --- | --- | --- | --- | --- |
| E01 证明清单与基线 | 无 | 清点全部发布、建帧、执行、恢复及测试入口；填充第 2 节证明链；复测固定负载 | 每个候选有已证明/待证明状态、反例和测量口径；记录基线身份 | 当前代码的证明链已记录；基线身份沿用 PR #17。测量结果不纳入本 PR。 |
| E02 已发布执行描述 | E01 | 收拢不可变代码及布局；封闭构造；迁移生产发布与 snapshot | 未验证草稿或不匹配元数据无法经正常接口进入生产 VM；发布失败回滚与原有边界测试通过 | 已实现：`VerifiedFunction` 消费准确草稿；snapshot 收拢只读拥有权。`7a343e5` / `5b96313`。 |
| E03 代码与帧配对 | E02 | 统一建帧契约；普通调用、模块特殊入口、generator/async 恢复接入；隔离合成测试入口 | 错代码、错 Runtime、错 closure 布局和非法恢复被拒绝；重入、清理、递归上限不退化 | 已实现：`new_activation` 从 host 推导代码/布局；恢复封装字段私有。`5b96313`。上游 callable 来源仍由原提取边界负责。 |
| E04 绑定访问 | E03 | 一起处理 local、argument、VarRef 的共享机制与静态模式检查 | 普通/词法/捕获语义测试、错误反例通过；逐项记录减少的工作与 loop/call/closure A/B | 已实现：普通/checked 读写复用发布保证；初始化和 CloseLocal 移除已证明的模式检查；捕获写入不再 clone cell，错误名字按需读取。TDZ、实际 cell const、特殊初始化协议保留。`a4b7830` / `27f5f30`。 |
| E05 常量与静态名字 | E03 | 种类明确的常量访问；复用链接 Atom；避免重复分类 | 常量种类混用在边界拒绝；root 生命周期与异常一致；执行、发布、内存数据齐全 | 明确保留分类操作：统一常量投影，复用既有 Atom 表。安全 enum match 仍在，不计作已消除的重复分类；不新增复制池/种类表。`51065a5` / `a23ec52`。 |
| E06 捕获与 eval 计划 | E04、E05 | 将静态父子描述符匹配和环境整理移至发布；实际 cell/调用方验证留在边界 | 特殊视图、eval 遮蔽/捕获、逃逸、失败回滚通过；无回调重放或缓存失效问题 | 初始静态验证迁移已实现（`51065a5` / `8d4a2da`）；补齐环境共享与捕获元数据分别见 `4b7bd08` / `d0d329e`。`64785fa` 补齐已有 cell 的直接复用；最终本地正确性和性能验证已完成，耗时结论以上方清单及本地证据为准。 |
| E07 静态控制流与栈 | E03、E04 | 评估目标转换、PC 递增与栈检查；只迁移证明完整的部分 | 分支、finally、异常、恢复、回溯及畸形字节码覆盖；无 panic 替代原错误；记录保留项 | 已实现：静态分支 `7521630`、双操作数检查合并与一次深度查询 `c55d59d`，分步测量已完成，最终本地集成验收已完成。PC 递增实验撤回；动态目标与恢复检查保留。 |
| E08 dispatch | E04、E05、E07 | 统一指令分类拥有者，评估减少多层分类；不改变操作语义 | opcode 覆盖完整，原 PC 对应不变；机器码/指令数与综合负载支持选型 | 已实现：常用绑定、字面量、简单栈操作和分支在循环中直接执行；调用、复杂数值转换与其他语义处理器独立保留以控制递归帧。无预解码表或第二指令枚举。`7a7fab7` / `254581d`。 |
| E09 集成与交接 | E06、E08 | 清除临时兼容入口与双实现；更新职责文档；完整正确性与性能复测 | 第 5、6 节验收齐全，保留无收益/退化结果，每项候选有最终处置 | 本地集成已完成：最终代码重新通过第 5 节测试和固定控制矩阵；独立评审仍待进行，不将 E09 标成全部评审完成。原始测量结果只在本地 target。 |

补齐前已经实施 E04 写入与生命周期、E08 常用指令直接执行、E06 eval 发布保证，并撤回 PC 递增实验。当前后续顺序、必要保留项与验收标准见文首清单；补齐后的实际完成范围以上方状态表为准，独立评审仍未完成。计划不承诺消除 QuickJS 与本解释器的全部机器指令差距。

E04/E05/E07 在设计上部分独立，但共享 VM 与代码存储，不默认并行编辑同一核心文件。E01 确认证明缺口后可细分步骤或调整依赖，并在此表记录理由。

实验无收益时可以保留能显著简化契约且成本可接受的结构改进，但必须明确称为维护性改进。增加复杂度的快路径若无可信收益，应撤回；证据不足的检查继续保留。每个候选最终只能是已验证实现、实测后不采用，或有明确原因与后续入口的延期，不能从清单消失。

## 5. 正确性与性能验收

### 5.1 正确性

先补会在旧错误实现上失败的契约测试，再迁移对应行为。合成测试需要独立的 checked 入口；测试构造便利性不能成为生产绕过验证的能力。验证器反例与真实发布到执行的端到端测试同时维护，不能只有封装 getter 的镜像测试。

必测类别：

- 普通与 checked local/closure、const、TDZ、缺省和多余实参、mapped/strict arguments、循环词法绑定、CloseLocal 与异常后的复用。
- closure 在捕获前后读取、逃逸与嵌套 relay；FunctionName、模块导入视图与 live binding；direct eval、with、动态函数与动态环境遮蔽。
- 错常量种类、错索引、错帧/代码配对、错 Runtime、非法闭包引用、受限 BC5 和模块/eval 能力伪造。
- generator、async、async generator 的 yield/await/throw/return、重复恢复与异常展开；finally/Gosub/Ret、iterator close、回溯 PC 和源码位置。
- 用户回调重入、GC、发布中途失败、Return/Throw/internal error 的清理；普通与混合递归的现有栈边界。

边界变更要同步修改 architecture checker 的规则、证据及 mutation canaries；必须证明错误变体仍命中实际入口并被拒绝，不能仅更新源码指纹让检查变绿。冻结 oracle/Test262 历史记录不改写；新结果单独生成并比较。

每步跑拥有者相关测试与受影响边界检查。集成时按当前 CI 固定工具链跑完整 workspace、profiling、doc、test262-host 组合、严格 lint、源码布局和全部架构 canaries，补 focused 与 full Test262、native release 和 Node/WASM playground 验收。具体命令以 [README 验证入口](../../README.md#verify) 与当前 CI 为准，不复制一份长期漂移的组合清单。

性能和正确性分别报告；stdout 一致仅是负载准入，不替代上述测试。不能以 quickjs-oxide 已有失败为由接受新增失败，也不能静默刷新冻结数据。

### 5.2 测量

E01 固定并保存源码提交、引擎 fingerprint、工具链、编译参数、ELF/workload 哈希、机器、CPU 绑定、重复次数及超时。普通 timing binary 关闭 profiling；CPU sampling 使用独立带符号/帧指针 binary。构建、测试与基准不争用同一测量机器。

每个性能步骤至少五轮交错 A/B，负载足够长，报告全部样本、中位数、范围及 retired instructions/cycles。重复数和准入规则在实验前固定；噪声大时按事先约定扩展双方样本，不挑最快值。性能退化超过基线噪声范围或分布无法区分时，调查或记录为未证明收益，不预先承诺统一加速阈值。

重点负载包括 empty/up/down loop、int/float arithmetic、普通/闭包调用、local/argument/捕获绑定混合访问、反复创建闭包、静态属性名、常量与 RegExp literal、eval、分支/finally，以及 generator/async 恢复。新增诊断负载用来隔离机制，不能替换现有完整 50+8 控制矩阵。保留原 adaptive harness 的成功/超时状态，不用固定工作量冒充其 score。

新增派生计划还要覆盖空函数、小函数、大常量池、宽局部变量、深闭包树以及反复发布/销毁。记录代码/计划长度、容量和可测的拥有存储，检查是否按指令重复存数据；不要用窄 profiler 覆盖推断总分配或 RSS。只能取得整进程时间时如实标注，不能用相减构造未经验证的 compile-only 时间。

E09 对 E01 基线做完整对比，逐项列出收益、持平、退化与成本转移。优化只改变热点占比而没有更少工作或更好时间，不足以认定成功。

## 6. 人与 agent 的维护协议

### 6.1 模块入口和扩展方式

| 要修改的行为 | 首先阅读的拥有者 | 同步维护 |
| --- | --- | --- |
| 新 opcode 或操作数种类 | code 的指令、验证与发布 | 栈效果、访问模式、执行分派、拒绝反例、PC/序列化对应 |
| 新发布来源 | code 的事务发布；相应编译或解码入口 | 能力来源、构造封闭性、roots/回滚、生产边界 canary |
| 绑定读写与捕获 | VM 的帧绑定；code 的绑定布局 | TDZ/const、别名和复用、实际 VarRef 与静态计划一致性 |
| 常量或环境计划 | code 的已发布执行描述 | 构建拥有者、派生表空间、Atom/GC 生命周期与销毁 |
| 新调用或恢复方式 | VM 的建帧、activation 与 suspension | 代码身份、动态状态验证、异常清理、本机栈预算 |

局部契约应随其源码所有者说明：拥有/借用什么、允许哪些调用者、哪些函数可能调用用户代码、错误与回滚契约、复杂度/空间成本，以及正向和反例测试入口。以模块名和符号定位，不把历史行号当接口。本文记录跨模块决策，局部契约留在所属模块，避免两处维护完整重复说明。

新增一个检查时，先回答它依赖的是草稿结构、已发布布局、实际帧还是当前 JS 值；据此放到相应边界。新增一个缓存或计划时，先回答谁构建、谁拥有、何时失效、是否跨回调，再决定是否需要抽象。

### 6.2 实施、评审与交接

人和 agent 使用同一标准。每步开始先核对本计划状态和实际提交，读拥有者说明，明确准备迁移的检查及证明链；不要仅凭上一位维护者称“已验证”就删除检查。

E02/E03 的执行契约，以及 E06/E07 的捕获、恢复和控制流变化，应安排独立评审，评审者可以是另一位人或 agent。重点检查构造能力是否泄漏、是否混配所属函数、静态证明是否覆盖实际入口以及动态语义是否遗漏。实施者自己的通读不称为独立评审。本计划不要求每个可逆操作额外请求许可。当前实现者的通读只记为自查；PR 的独立评审仍待完成。

每步完成后在本计划状态表与对应报告保留以下信息；报告是证据附件，不另建冲突的总任务清单：

```text
步骤/状态/实施者：
基线与结果提交：
实际拥有者与公开到模块间的接口：
迁移的检查、静态依据、保留的动态条件：
所有权、回调/重入、错误与回滚契约：
采用/未采用方案及原因：
验证命令、环境、退出结果、报告位置：
性能/空间样本、收益、持平与退化：
独立评审状态及未解决意见：
剩余工作、下一入口和复现命令：
```

交接必须标明测试是“已编写”“已运行”还是“通过”，以及当前工作树和正在运行的任务。没有实测的结论标为假设；未完成步骤保持未完成。最终收口要求所有入口使用统一契约、临时迁移接口已移除、维护文档可供新读者操作，且第 2 节每项候选都有可追溯处置。
