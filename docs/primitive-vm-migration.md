# 栈 VM：架构迁移与验收账本

状态：2026-09-15。用户已确定使用栈 VM，**S01–S08、S09（含 N1–N3）与新 S10–S12 已实施；性能退出条件未全部通过，S13（退役旧路径）未实施，整体计划尚未完成**。本表与[架构计划](primitive-vm-plan.md)、[实施设计](primitive-vm-implementation-plan.md)、[逐 commit 计划](primitive-vm-commit-plan.md)共同定义一个 PR 的交付。提交合并后，能力与结构条目仍逐项验收。

S10–S12 最终单轮 403/403 有效，58 fixed 仍有 25 项高于 S0（见[联合报告](performance/README.md)）；残余差值的根因与修复阶段见 [S14–S20 修复计划](primitive-vm-s14-s20-recovery-plan.md)。以下能力与结构条目的验收口径不变；历史 S07 回退分析保留为证据（[回退分析](performance/README.md)）。

## 1. 起点与范围

- PR19 基线：`c52d4dc7747641756dff8cb7159b9885e9cc8b17`，生产 Rust 源码与历史调查的 `1cc51bb5fcc5c36912d3197d877219ae513dc4b5` 一致。
- 当前分支 `vm/primitive-execution-core`；PR #21 基于 `perf/ordinary-property-kernel`，不称为默认分支。
- 目标是栈 ISA、线性栈 IR、有限局部改写、显式普通调用与内部 callback、集中执行槽所有权和准确观察边界。
- 本轮维持 RC/循环回收，明确运行 owning 值与长期挂起 raw heap 边交接。GC 重写、SSA/寄存器后端和 #20 的 Fiber 调度不属于此账本。
- 暂存/部分新入口通过不等于迁移完成。每个条目最后填写真实入口、提交、验证身份和剩余路径；现有历史 receipt 不改成新核心成绩。

## 2. 外部契约

| 表面 | 保持的契约 | 主要验证 |
| --- | --- | --- |
| Runtime/Context | 身份、realm、同步 compile/eval/execute/call/construct、byte-source | api/context/realm、Rust API 用例 |
| 值与生命周期 | owning handle 保活、clone/drop、wrong-runtime 拒绝；raw Clone 不负责对象边 retain | root/heap/value、强制 GC、最后引用与挂起交接 |
| 参数与 binding | actual arguments 与可写形参、mapped/unmapped、TDZ/const/cell/private/eval | 参数、闭包、eval、scope/private 语义测试 |
| host | 同步 callback ABI、JS→host→JS、回调前无内部借用 | native/web adapter、delimiter、异常退出清理 |
| 资源限制 | native 栈守卫、VM 帧/槽限额、中断/fuel、内存超限 | 默认/小栈递归、可捕获错误、融合前后预算契约 |
| debug/error | fault/resume、语义阶段 source site、backtrace、realm | code/debug、runtime trace、调试模式融合规则 |
| jobs | async 同步前缀、Promise assimilation、宿主 draining 与任务顺序 | Promise/async/modules traces |
| binary | 已支持外部 QuickJS 格式、round trip、畸形输入拒绝 | binary fixtures/翻译/发布、C oracle |
| 平台与 API | safe Rust/MSRV、原公有 API 边界、CLI/native/web/WASM | workspace/平台检查、[架构说明](architecture.md) |
| 一致性 | 完整既有语言支持和诊断，不加 skip 或重写冻结预期 | 完整回归/Test262 与相关 oracle |

内部结构允许重做，外部行为变化不因整理架构自动获得授权。

## 3. 状态所有权验收

下表同时包含结构改进和必须保持的不变量；其中的错误情形是迁移防护要求，不表示现有实现已经存在这些错误。

| 状态 | 唯一责任 | 目标不变量 | 状态 |
| --- | --- | --- | --- |
| 编译中的 scope/binding/IR | compiler 模型与解析阶段 | lowering 消费已解析身份，复用 binding 事实 | S01 已验收 |
| code/layout/sites/handlers | code 发布对象 | 继续统一验证后发布，保留已有不可变共享 | S02 已验收；后续融合归 S08 |
| 当前 frames/pc/sp/slots | RunningExecution 与 frame/stack 模块 | 一份动态状态；host/观察表引用其身份和已发布视图 | S03–S07 执行所有权已验收；PC 优化归 S08 |
| 参数与局部 owning 值 | SlotStore/binding 布局 | 复用调用容量；原始实参与可写形参保持正确关系 | S03/S04 所有权已验收；调用容量优化归 S09 |
| 语义 callback 进度 | value/object/builtins 中的领域状态 | 恢复点显式，VM 不复制领域算法或递归等待内部 JS | S05–S07 共享阶段及入口验收通过 |
| operation 调用与回复 | VM operation 登记及 driver | 正确 owner、单次回复、getter 不因恢复重复执行 | S04–S07 验收通过 |
| abrupt completion/cleanup | unwind 与有效控制区域 | 统一展开协议，保留各项语言清理优先级 | S04–S07 验收通过 |
| 长期挂起状态 | heap 原始记录；suspend 负责事务交接 | 无永久 Runtime-owning 环、半恢复或最后 root 丢失 | S06 验收通过；证据见本轮阶段验收 |
| host 重入观察 | 运行登记 guard/delimiter、已发布状态 | 回调前结束借用、视图准确、退出后解除登记 | S07 验收通过；真实边界与禁止内部嵌套根均已验证 |
| Number 语义 | value/number | 运行与折叠共用纯算法，通用转换保持独立责任 | S03 已验收 |

代码结构独立验收如下，证据与拆分规则见[实施设计第 15 节](primitive-vm-implementation-plan.md#15-代码结构的独立改进清单)。

| 结构任务 | 提交与验收 | 状态 |
| --- | --- | --- |
| compiler 入口、共享模型、解析临时状态 | S01；复用现有阶段，消费式交接，不生成新的巨型 model | S01 阶段验收通过 |
| host_bridge 的绑定/构帧/挂起/领域操作分归属 | S03–S07；已有读写 helper 复用，测试/恢复特殊验证保留 | S03–S07 分责及入口验收通过；旧路径删除归 S10 |
| 验证大函数、长 tuple 与发布流程 | S02；命名工作项、明确检查顺序，已有 VerifiedFunction 和反例保持 | S02 阶段验收通过 |
| code/compiler 反向依赖 | S01–S02/S07；发布输入归 code，编译请求编排归 api，生产验证不导入 compiler | S02 验收和 S07 入口审计通过 |
| 显式导入与 Number 文件职责 | S01–S03/S10；imports 可追踪，运算/转换/格式化分责，公有边界不扩大 | 待做 |
| 测试、物理归属检查与架构/源码契约 | 随迁移更新，S10 收口；旧反例和 mutation 有效，无失联检查 | 待做 |

## 4. 能力迁移账本

| 能力 | 目标责任/提交 | 关键验证 | 状态 |
| --- | --- | --- | --- |
| 完整语法与名字解析 | parser/model/resolution，S01 | hoist、eval、class/private、错误顺序；无数字子集前端 | S01 已验收 |
| 栈 lowering/控制流/发布 | lowering/flow/code，S01–S02 | 合流栈形状、异常/恢复、TDZ、源码重定位与畸形 code | S01–S02 已验收 |
| Frame/Slot 容器 | execution/frame/stack，S03 | 区间独立、容量复用、clear/move、原始实参、运行登记 | S03 已验收 |
| 栈原语与 Number | number/run，S03 | Int/Float、NaN/-0、BigInt/String 慢路、目标旧引用释放 | S03 已验收 |
| 普通调用/constructor | call/driver，S04 | 限额、参数/this/new.target/realm、bound、derived return | S04 已验收；exotic/native 领域调用点归 S05 |
| 转换/异常/finally | conversion/operation/unwind，S04 | getter 次数、不同抛错点、清理优先级、单次释放 | S04 已验收；其余领域回调归 S05 |
| binding/eval/arguments | resolution/bindings，S04 | captured、每迭代 cell、mapped/unmapped、private/readonly | S04 已验收 |
| 局部 update/条件融合 | optimize/run/code，S08 | 快照、prefix/postfix/discard、NaN、效果/site/预算 | 待做 |
| properties/Proxy | object + 对应 builtin，S05 | receiver、trap invariant、递归 getter、PR19 回退用例 | 全部属性/Proxy 协议及 Object/Reflect 已接入共享阶段；S05 已验收 |
| Array/iterator | 对应 builtin，S05 | holes、species、sort、动态 length、IteratorClose | callback/sort/species/同步迭代与 close 已接入；S05 已验收 |
| String/RegExp/buffer | 对应 builtin，S05 | replacement/Unicode、resize/detach、共享内存与 BigInt | String/RegExp 协议、buffer/TypedArray/Atomics 转换已接入；S05 已验收 |
| 其余同步内置 | 各领域 owner，S05 | intrinsic 逐项审核、toJSON/replacer、修改中迭代、realm | Function/scalar/Math/collections/Date/JSON/Error/weak 已接入；逐调用点审计与 S05 统一验收通过 |
| generator | suspend + generator 驱动，S06 | next/throw/return、yield*、reentry、关闭/失败/GC | S06 验收通过；证据见本轮阶段验收 |
| async/Promise | suspend/async/jobs，S06 | 同步前缀、assimilation、微任务次序、pending roots | S06 验收通过；证据见本轮阶段验收 |
| async generator/iteration | 专用队列与恢复，S06 | 交错请求、finally await、异步 close | S06 验收通过；证据见本轮阶段验收 |
| modules | modules + driver，S07 | cycles/live import、TLA、dynamic import、loader 重入 | S07 验收通过；新旧完整 oracle/Test262 均通过 |
| API/host/binary/platform | 入口适配与统一验证，S07 | 所有入口、delimiter、round trip、畸形输入和平台 | S07 验收通过；native/API/binary 及两配置 Node/WASM 通过 |
| PC/observer/interrupt | observe + run，S08 | fault/resume、GC/release、host/debug、融合 fuel 权重 | 待做 |
| 执行回退修复 | run/stack 与领域请求/结果，S08 | 认证窗口、immediate copy、小型 payload、无回调直接完成；独立 A/B、机器码和零桥计数 | 待做；已取得 S07 热点证据 |
| 调用与等待状态复用 | call/frame/stack 与 continuation owner，S09 | argv/形参语义、直接窗口初始化、冷容量/metadata、分配/RC/活跃与峰值、挂起回收 | 待做；已取得 S07 每调用分配证据 |
| 编译与布局回退修复 | compiler/code 与 run/driver，S09 | compile-only 分阶段归因、`.text`/native 栈、WASM、缓存/编码的去留 | 待做；不得靠弱化验证或提前删除旧路径结案 |

S05 收口同步内置调用点并列明异步/模块/入口的后续责任，S07/S10 审核全部内部 JS 调用。真实 host 同步边界必须能指出实际 embedder callback，不能用来藏未迁移内部递归。

## 5. 最终完成标准

| 项目 | 必需证据 | 状态 |
| --- | --- | --- |
| #1 | 默认预算原始 Earley-Boyer 独立/组合；有限/无限递归、小栈与 JS→native→JS | 待做 |
| #5 | Number 实际直接路径、完整语义、算术/Crypto/Navier-Stokes A/B | 待做 |
| #7 | 分配、初始化、retain/release、峰值/活跃槽及调用吞吐；参数/捕获语义正确 | 待做 |
| #9 | 最终码、动态分派；更新/条件融合的快照、错误和 source site | 待做 |
| #10 | 独立成本归因与完整观察点；成本不显著也如实记录 | 待做 |
| S07 回退修复 | S08/S09 相对父阶段、S07、PR19 的逐项比较；固定 58、原始 8+combined、compile 67 与内存/暂停证据，确认的剩余回退闭环 | 待做；S09 退出目标为恢复到 PR19 水平或更好，口径见逐 commit 计划第 4 节 |
| 架构 | 小型协议、一个 driver、一个热分派、领域状态就地归属、清楚拥有/恢复 | 待做 |
| 可维护性 | 新 Number、转换顺序、异常/恢复、binding/eval 四种修改演练 | 待做 |
| 完整验证 | 50+8 固定、原始 V8、相关 oracle、全回归/Test262、native/Web/WASM | 待做 |
| 旧路径退出 | S10 完成后所有默认入口只进新栈核心；VmHost/重复动态帧和迁移桥删除 | 待做 |

S10 内先完成新核心全量验收，再默认切换和删除旧路径，最后在最终源码上重新核验并更新真实实现文档；这些顺序合并到同一提交，验收记录仍分开。构建/测试/正式计时/诊断分开，不拿旧 receipt 或无优化代码的历史成绩填本表。本轮不关闭 #16 其他问题，也不声称完成 #20 的 Fiber 调度与取消。

## S03 资源与所有权审查（已验收）

资源策略分两层：S03 的 FrameStore/SlotStore 限制存储范围与身份，原有
native 栈守卫仍约束临时桥上的递归；S04 才把普通调用改为同一执行中的
显式帧预算。当前帧上限为 65,535，槽上限为 `isize::MAX / size_of::<Option<FrameBinding>>()`。
它们是内部存储边界，不是新增的用户内存配额。槽区间相加与帧/窗口身份递增
均使用 checked arithmetic；Vec 扩容、原始实参快照和单向交接目标先可失败
预留，再安装或转移 owner。固定大小 Box/Rc 的分配仍沿用项目的 Rust
分配失败策略，不声称能把所有进程 OOM 转为 JS 异常。

| S03 要求 | 实现与验收证据 | 本阶段边界 |
| --- | --- | --- |
| 独占区间、扩容与活跃窗口 | `vm/stack.rs` 的身份、区间、父窗口扩容与跨 arena 拒绝测试 | 窗口仅保存索引，无 Vec 元素地址 |
| move/copy/clear、返回最后引用 | pop 立即 take；原始实参与参数隔离；返回先持有 pending；重排与重复值生命周期测试 | 普通值栈完成；控制协议归 S04 |
| 帧限额与身份耗尽 | `vm/frame.rs` 的限额、过期/跨执行身份、耗尽测试 | 拒绝帧释放其冷 owner，已安装帧保持有效 |
| 运行登记无 owning cycle | `vm/execution.rs` 的嵌套 panic 清理及弱 Runtime 生命周期测试 | 登记只含 domain/execution ID |
| 引用预检、不重复提交 | `heap/slot_ownership.rs` 的 7 项测试；`vm/stack.rs` 的部分复制失败清理测试 | 需要 drain/deferred/最后 primitive 析构时退出热路；复制错误不桥接重试 |
| Number 与慢路 | `value/number/` 的 13 项测试及 `vm/run.rs` 的数值边界、String/BigInt 和一次转换测试 | Number 计算无 Runtime 服务；动态转换暂经一次旧桥 |
| 实际新核心路径 | 独立 ordinary-call 循环及字面量返回要求零旧分派、零桥接 | CLI 脚本 wrapper 的混合路径单独标明 |
| 初步成本 | 同一 CostSnapshot 中的真实容量增长、初始化、活跃/预留峰值及转移/引用事件；复用与 CLI JSON 测试 | 统计范围详见 profiling.md；完整调用成本归 S09 |

S03 最终同源检查已收齐：711 个边界反例全部拒绝；新旧入口各 907 项
常规 oracle 与 1 项单独执行的 65K 实参用例通过；Number 13 项、引用事务
7 项以及帧/槽/执行/登记 18 项通过。完整 VM 组为 128 项通过、2 项已有
小栈失败；后者继续保留在迁移账本中，S04–S10 必须解决，不能通过扩大
预算或跳过测试消除。源码布局为 496 个可达 Rust 文件，格式与关闭
profiling 的 stack-vm 构建检查通过。S03 验收仅覆盖本阶段范围。

## S04 显式普通子调用（实施中）

同一 driver 内普通字节码子调用的 9 项 run 测试通过；独立限额与外域参数
拒绝测试 2 项通过。256 层命名自递归在 2 MiB 栈内全部走 owned 路径，
尾调用与子帧旧路径抛错也有覆盖。公开无限递归输出可捕获的
`InternalError:stack overflow`。常规 oracle 907 项通过，非 profiling
构建、边界扫描、格式检查通过。当前源码布局为 498 个文件。

这些证据不代表 S04 验收：全局函数链和 TypedArray 的两个既有小栈测试
仍失败；constructor、转换回调、控制记录展开、capture close 和完整绑定
尚未接入。尾调用当前保留显式父帧，尚未做 S09 的帧复用优化。

绑定到普通字节码的调用也已迭代归一化后进入同一 driver。5 项 driver
测试覆盖逻辑限额、外域参数拒绝、嵌套绑定参数顺序、绑定 receiver 的
子帧交接及 native 目标原始调用交接；全部通过。`PushThis` 已处理严格模式、对象、callee global 和原语装箱；
装箱在发布 PC 后由 driver 执行，不调用 JS。FrameCold 缓存包装对象，
交接时转移到旧 activation；测试验证交接前后的 `this` 对象身份一致。归一化后的常规 oracle 907 项、
非 profiling 构建、边界扫描和格式检查通过；完整 S04 仍未验收。

S04 getter 恢复的准备接口已接入 `object/ordinary.rs`：普通查找返回拥有值、
getter/receiver 或特殊节点的 OrdinaryRead，旧入口消费同一结果。准备阶段
不执行 getter；新增测试在准备后替换属性，验证已选择 getter 仍以原 receiver
执行一次。object 79 项、常规 oracle 907 项、非 profiling 构建和边界扫描
通过。此项只完成查找/执行边界；getter 仍由旧入口调用，尚未形成完整的
显式 getter 恢复流程，不能作为 S04 回调验收。

S04 随后接入 `BytecodeCallRequest`：已分类的字节码调用拥有 callee、receiver、
new.target、参数、realm 与返回帧身份；准备阶段不借用父帧槽。
`GetField/GetField2` 现在消费普通查找结果，普通字节码 getter 在同一 driver
内安装子帧，返回值恢复父帧；继承 getter 的 receiver 保持原对象。特殊节点、
原语 base 和非普通字节码 getter 仍在消费原输入前交接。此时尚无完整
parent-operation 协议，ToPrimitive 多阶段恢复和 constructor 仍待实现。

S04 ToPrimitive 的阶段已移入 `value/conversion/primitive.rs`，输出 Get、Call
或 Complete；每次回复消费一个不可 Clone 的 continuation，包含原对象、realm、
hint 和阶段。普通转换也使用同一状态，现有同步入口负责消费；尚未接入
显式 driver。新旧配置常规 oracle 各 907 项通过。旧函数体和位置保护已迁移：入口路由及完整状态源码（包含 hint、realm、
方法顺序与完成值）有精确校验。边界扫描通过；原有 hint、错误 realm、
普通方法顺序三项故障注入全部拒绝，检查集成、非 profiling 构建、格式及
diff 检查通过。这只验证状态迁移门禁，S04 仍未验收。

S04 一元 `+` 已消费上述转换状态：父帧以独立 operation 身份持有等待回复的
continuation，普通字节码 getter/valueOf 安装为同一执行的子帧；回复匹配
帧身份和 operation 后消费一次。嵌套转换测试达到 3 层显式帧、4 次构帧，
没有 legacy 分派或帧交接。转换输入已从栈移入状态，成功后推进一次 PC；
抛错沿 pending completion 传播。非普通字节码调用与特殊属性读只交接当前
步骤，已执行 getter 不会重放。这个临时同步步骤路径仍待 S05 替换，现有
bridge counter 仅统计整帧交接，不是全部 operation 交接的成本。

构造调用、其他转换指令、完整 operation 协议与统一展开仍未完成；控制和
capture 设置指令仍在执行前交接，当前 owned 父帧直接传播 Throw 的适用范围
没有扩大到有 handler 的帧。

S04 二元 `+` 也已接入：Finish::AddLeft 持有已求值右操作数，左侧完成后
Finish::AddRight 持有左侧 primitive，再开始右侧转换。新旧执行器共用
`vm/numeric.rs::add_primitives` 的 String/BigInt/Number 后处理。测试验证
`x+(x=2)` 保留左值，以及左侧回调替换右侧 valueOf 后按新方法执行一次；
字符串与 BigInt 加法不再交接整帧。driver 12 项、run 9 项、新旧配置常规
oracle 各 907 项通过；边界扫描及非 profiling 构建通过。此进展不代表
constructor、其他转换、统一展开或完整 S04 已完成。

S04 Construct/ConstructSuper 已为普通及 bound 字节码构造器安装显式子帧。
bound 链保留实参顺序，并只在身份匹配时重定向 raw new.target。FrameCold
持有 base 的 this 返回后备值或 derived 返回约束，guard 结束后处理完成值。
PushNewTarget 已直接读取帧输入。测试覆盖 base 原语/对象返回和嵌套 bound
new.target；driver 14 项、run 9 项、常规 oracle 907 项、非 profiling 构建及
边界扫描通过。原型查找仍是当前步骤的同步旧路径调用，在调用前结束帧及
heap 借用；它尚未接入 operation 回复，完整 constructor/S04 尚未验收。

S04 构造原型的普通字节码 getter 已接入回复：PendingConstructor 拥有构造请求、
实参和 operation 身份；回复目标区分 Conversion/Constructor。getter 结束后
直接使用返回原型，或按 raw new.target 取得 realm 后备原型，再安装构造器
主体，不重新读取 prototype。可信字节码 raw new.target 测试验证三次构帧、
最大深度二、零 legacy 分派，且实例原型身份正确。该测试、driver 14 项、
常规 oracle 907 项、非 profiling 构建通过，布局检查为 503 个 Rust 文件。

新增测试独立于历史 binary_calls.rs；tests.rs 的冻结值仅因新增 cfg 模块声明
更新，更新前已验证删去该声明后的字节哈希等于历史冻结值。边界扫描通过。
特殊对象的原型读取和非普通 getter 仍交接当前同步步骤；完整 derived 控制流、
统一展开及 S04 整体验收仍未完成。

S04 ReturnDerived 已由新旧路径共用 `bindings::finish_derived_return`：对象直接
返回，undefined 读取直接或 captured 的 this，未初始化时在 caller realm
产生 ReferenceError，其他原语产生 TypeError。新核心接入该终止指令及
PushActiveFunction、合法 new.target 的 CheckCtor 路径。class extends null
显式返回对象/undefined/数字三种测试均零 legacy 分派；driver 15 项、新旧
配置常规 oracle 各 907 项通过，边界扫描、非 profiling 构建、格式检查通过。
这不包含 super 初始化、捕获关闭或完整 derived 控制流；SetLocalUninitialized
仍交接，未扩大当前 owned 帧的控制/capture 设置范围。

S04 显式 super() 的 direct this 初始化已接入：GetSuper 读取实时原型，
InitializeDerivedLocal 消费 shared bindings::initialize_derived_binding 的结果；
新旧路径保留 captured 更新与重复初始化检查。MarkSuperCall 沿用原空操作，
没有实例字段初始化器时复用原类/realm/receiver 验证并继续执行；有初始化器
仍在该步骤交接。测试证明第二次 super() 先执行父构造器再抛重复初始化错误，
两种路径均零 legacy 分派。driver 16 项、run 9 项、常规 oracle 907 项、
非 profiling 构建及边界检查通过；默认配置常规 oracle 也通过 907 项。
默认 derived 初始化、完整字段调用、binding/capture 生命周期及统一展开仍待做。

S04 InitDerivedConstructor 已通过共享 enter_request 安装父构造器子帧；它按
实际 argv 范围复制当前参数绑定，读取当前函数的实时 [[Prototype]]，保留
raw new.target。显式 Construct 消费栈中的 callee/new.target/实参，默认
入口消费零个 operand，二者共用 bound、原型回复及构帧流程。测试验证
零声明参数的默认 derived 转发三项实参，以及修改父构造器后仍传递 derived
new.target，均为三层显式帧、零旧分派。driver 17 项、run 9 项、常规 oracle
907 项、非 profiling 构建与边界扫描通过。完整 binding/arguments、捕获关闭、
统一展开及所有初始化器调用仍待完成，S04 未验收。

S04 普通 lexical 槽已接入 SetLocalUninitialized、InitializeLocal、
GetLocalCheck、PutLocalCheck、SetLocalCheck 及非捕获 CloseLocal。
槽覆盖与重置仍先证明无回收；捕获槽或不满足释放预检时在修改前交接。
初始化仅覆盖 Normal lexical，with/private 保留原领域协议。TDZ 在发布故障
PC 后由驱动器构造当前 realm 的 ReferenceError，新旧路径共享变量名可见性、
隐藏 this 和 strip-debug 诊断规则。普通块退出保持槽值，下一次进入重置 TDZ。
新增测试覆盖 let/const 初始化、赋值表达式、读取/赋值/自初始化 TDZ、循环块
复用及第二轮 TDZ，均零旧分派、零交接。最终 driver 18 项、run 9 项、新旧配置
各 907 项常规 oracle 通过（各 1 项 65K 压力用例本轮未运行）；非 profiling
构建、边界 scan-only、格式及 diff 检查通过，源码仍为 503 个文件。
捕获生命周期、完整 arguments/eval 与统一展开尚未迁移，S04 未验收。

S04 闭包单元的 GetVarRef/GetVarRefCheck、PutVarRef/SetVarRef 和
PutVarRefCheck 已由 driver 消费：先发布故障 PC，独立持有 cell root 与输入，
再使用既有 Runtime VarRef 读写。checked 读取/写入共享原 host 的 TDZ、cell
只读检查及诊断名称规则；全局/模块语义名称与 strip-debug 的区别不变。
出错在当前帧 realm 构造异常并沿 driver 返回；每次读取都取得 cell 当前值。
新测试覆盖 var/let 多次调用共享写入、const 读取及逃逸未初始化绑定的读写
TDZ，均为两层显式帧、零旧分派与零交接。最终 driver 19 项、run 9 项、新旧
配置各 907 项常规 oracle 通过；非 profiling 构建、边界 scan-only、格式与
源码布局检查通过。创建闭包、每迭代 cell 和捕获关闭仍未接通，S04 未验收。

S04 catch/finally 的首段控制迁移：FrameCold 持有 VmUnwindRegion，Catch、
DropCatch、NipCatch 安装/清理 catch 区域；Gosub/Ret/DropGosub 保留已有 Int
返回 PC 协议。Throw 及子帧异常返回在当前帧弹出前检查 catch，先补齐错误
backtrace，再按记录深度清理操作数、压入异常并恢复目标 PC。NipCatch 和
catch 恢复按既有规则标记捕获槽可复用；pending 值在清理期间始终有 owner。
旧路径交接移动 regions，已有 catch/finally 状态不会丢失。Iterator 区域尚未
由新循环安装；IteratorClose、完整捕获关闭仍需后续迁移，统一展开未完成。
测试覆盖本帧/子帧抛错、finally 覆盖 return、嵌套 finally 和带活动 catch
交接；纯控制用例零旧分派、零交接。最终 driver 20 项、run 9 项、常规 oracle
907 项通过；非 profiling 构建、边界 scan-only、格式/diff 和源码布局检查
通过，仍为 503 个文件。S04 未验收。

S04 FClosure 已在 driver 中读取子 executable、捕获 ParentLocal/ParentArgument、
复用 ParentClosure 并转发 ParentGlobal，使用既有 Runtime 构造闭包。
新旧路径共享 capture_local_binding：新 cell 使用父定义的 canonical metadata，
已有 cell 校验子 descriptor view，保持身份。SlotStore 提供受窗口校验的可变
绑定访问，仅用于 NoJS 捕获/关闭操作；引用不跨帧增长或 JS 回调保留。
CloseLocal 的 captured 分支接入 shared close_frame_binding，先 root 当前值
再 detach；返回闭包自身持有独立 VarRef，所以父窗口清理后仍可读取。
SetName 使用既有名称初始化规则，JS 错误进入当前帧异常展开。
新增四条构造路径验证局部/参数捕获、块退出和循环不同轮次的独立 cell，创建
与后续调用均零旧分派、零交接。最终 driver 21 项、run 9 项、新旧配置各
907 项常规 oracle、非 profiling 构建、边界 scan-only、源码布局与格式/diff
检查通过，源码仍为 503 个文件。captured 局部读写/重置仍会交接；arguments、
eval/with/private 和 IteratorClose 等余项未完成，S04 未验收。

S04 captured 局部/参数访问已并入 Binding 出口，以 Closure/Local/Argument
显式区分来源。驱动器独立持有 root，再复用 VarRef 读写和 checked 规则；
局部诊断由父定义构成 ParentLocal view，因此 strip-debug 不被当作全局名称
处理。普通 captured lexical 初始化写入已有 cell，不替换其身份。
SetLocalUninitialized 的 captured 分支调用与旧 host 共用的
reset_captured_binding：初次未初始化 cell 不变；已初始化 cell 只有 catch/
NipCatch 标记可复用后才能原位重置，否则报告跳过 CloseLocal 的内部错误。
新增用例覆盖闭包更新父 lexical/参数、函数声明先捕获后初始化、父槽 TDZ 被
catch 捕获，以及异常退出后下一轮复用原 cell，均零旧分派与零交接。
最终 driver 22 项、run 9 项、新旧配置各 907 项常规 oracle、非 profiling
构建、边界 scan-only、格式/diff 与源码布局检查通过，仍为 503 个文件。
arguments、eval/with/private、IteratorClose 与统一展开余项仍待完成，S04 未验收。

S04 arguments/rest 创建已归 vm/arguments_driver。mapped arguments 捕获实际
传入的声明参数，额外实参建立独立 cell；unmapped 与 rest 在指令执行时读取
当前参数，数量只取实际 argv，不包含补齐形参。SlotStore 的尾部快照保留原有
参数绑定读法；原始 argv 区间提供 arity，默认 derived 转发复用 start=0。
五条测试验证严格模式赋值后 arguments 保留初始值、mapped alias、额外实参、
缺少实参及 rest，创建到返回对象均零旧分派/交接，返回后检查元素和 length。
初版把分配/错误处理展开在 driver 中，触发 Proxy 有限栈 oracle 回归；移入
独立且禁止内联的 step 后定向与完整 oracle 恢复通过，预算和预期均未修改。
最终 driver 23 项、run 9 项、常规 oracle 907 项及单独 65K 实参压力 1 项通过；
非 profiling 构建、边界 scan-only、格式/diff 与源码布局检查通过（504 文件）。
arguments 特殊属性操作仍可能交接，完整默认参数、eval/with/private、
IteratorClose 与统一展开余项未完成，S04 未验收。

S04 标识符默认参数的 undefined 判断已接通完整 StrictEq/StrictNeq：Number
仍走热路径，其余值在发布 PC 后调用独立 strict_comparison，复用 Value 的
strict_equal，结果建立后再释放操作数，无转换回调。七条默认参数测试覆盖
缺参/显式 undefined、已提供参数、null/false、不依赖后续参数的初始化顺序、
后续参数 TDZ，以及初始化器 closure cell 与函数体参数副本分离的固定 QuickJS
行为。TDZ 用例返回捕获的异常，在测量外检查 ReferenceError，避免混入尚未
迁移的 Error 特殊属性访问。相关执行均零旧分派/交接。driver 最终 24 项、run
9 项、常规 oracle 907 项通过；非 profiling 构建、边界 scan-only、格式/diff
和源码布局通过（504 文件）。解构参数的对象/迭代协议及 S04 其余项仍未完成。

S04 ThrowReadOnly/ThrowRedeclaration 已在发布 PC 后由 exception::binding_error
消费静态链接 Atom，使用既有 native_atom_error 与当前 realm 构造异常，再交给
driver catch/finally 展开；不提前弹出赋值操作数。readonly 定向用例覆盖 const、
严格函数名、闭包 const、声明前 const 写入（固定 QuickJS 直接 TypeError）及
右侧表达式抛错优先，均零旧分派。新增出口再次暴露常驻 driver 的 Proxy 有限栈
敏感性；把原有闭包创建完整移到禁止内联的 closure_driver::instantiate 后，
定向 Proxy 与完整 oracle 恢复通过，预算与预期未改。最终 driver 25 项、run
9 项、常规 oracle 907 项、非 profiling 构建、边界 scan-only、格式/diff 和
源码布局检查通过（505 文件）。eval/with/private、IteratorClose 等仍待迁移，
S04 未验收。

S04 private 初始化规则已归 vm/private_bindings：私有定义校验、字段身份创建、
方法名称与 HomeObject、访问器种类及 captured cell 初始化由新旧路径共享。
InitializePrivateName/Method/Accessor 在发布 PC 后交给独立 step，方法/访问器
仍消费 home+callable、成功后保留 home；JS 错误进入当前 realm 的异常展开。
发布帧定向用例从真实 InitializePrivateName PC 执行，验证两次初始化产生
不同身份、PC 正确前进且外部持有者在帧释放后保持身份；它不是完整 class
零交接证明，class 创建及 private get/set/in 仍有待迁移路径。
已有 private 相关测试 70 项、driver 26 项、run 9 项、新旧配置各 907 项常规
oracle、非 profiling 构建、边界 scan-only、格式/diff 与源码布局检查通过
（506 文件）。S04 未验收，private 访问、eval/with、IteratorClose 等继续待办。

S04 private 字段 get/get2/put/define/in 已接入独立 private_access step，使用
既有 get/set/define/has_private_field_own 内核。FieldSource 将局部定义与绑定、
闭包 descriptor 与 root 配对；新旧路径共享 descriptor 验证及 optional_field_name，
保留 captured metadata 检查、未初始化状态和普通值/private callable 错配错误。
Get2 保留接收者，Define 成功保留 base，Put 不回推结果；private-in 未初始化
名称仍执行固定 QuickJS 的 [unsupported type] 自有属性探测。JS 错误进入当前
realm 展开。非 PrivateField 的方法/访问器分支在消费操作数前交接。
新测试通过真实方法调用验证字段读写、成员检查 true/false、字段函数的 this
及错误接收者的 TypeError，均零旧分派。最终 driver 27 项、private 72 项、run
9 项、新旧配置各 907 项常规 oracle、非 profiling 构建、边界 scan-only、格式/
diff 和源码布局检查通过（507 文件）。private 方法/访问器、eval/with、
IteratorClose 等余项仍待迁移，S04 未验收。

S04 私有方法 get/get2/in 及非法赋值已进入 private_access；PrivateSource 同时
描述字段和 callable 来源。新旧路径共享 optional_callable 的状态与 metadata
校验，以及 branded_receiver 的 HomeObject-brand → receiver → brand 顺序。
方法读取返回同一 callable 身份，Get2 保留接收者；方法 in 保持先检查对象、
再解析 cell 和品牌的顺序，未初始化 cell 的固定探测行为保留。非法方法赋值
沿用静态名称只读错误，不先检查接收者品牌。访问器分支仍在修改前交接。
新增真实方法调用用例验证身份稳定、this、in true/false、错误对象和只读写入
TypeError，均零旧分派。最终 driver 28 项、private 73 项、run 9 项、新旧配置
各 907 项常规 oracle、构建、边界 scan-only、格式/diff 和源码布局检查通过
（507 文件）。私有访问器调用、eval/with、IteratorClose 等继续待办，S04 未验收。

S04 普通 bytecode 私有 getter/setter 已由 private_access 安装同一 execution 的
子帧。读取 cell、品牌/receiver 校验、callable 分类及预算检查完成后才消费
操作数；getter GetKeep 保留父接收者。ReturnTarget 的 ReturnValue::Push/Discard
明确返回值用途：setter 正常返回丢弃结果，异常不丢弃并按原路径展开。
其他既有调用目标保持 Push，转换/构造器 operation 回复路由不变。
访问器 in 与只读方向错误也已接入原有无 JS 规则；非 Normal bytecode 访问器
在消费前交接。测试验证 getter/setter 往返为五次显式构帧、最大深度三，setter
返回对象不会破坏 catch entry 深度；getter/setter 抛错到达父 catch。getter
返回函数的调用保留 this 且 getter 只执行一次，同样零旧分派/交接。
最终 driver 30 项、private 75 项、run 9 项、常规 oracle 907 项、构建、边界
scan-only、格式/diff 和源码布局检查通过（507 文件）。eval/with、IteratorClose、
完整类初始化调用和其他 S04 余项仍待迁移，S04 未验收。

S04 实例初始化器调用已由 construct_driver::instance_initializer 处理，替换
原先仅允许无初始化器的探测。Runtime::begin_class_instance_initializer 由新旧
路径共享，按原顺序校验 constructor/realm/prototype/receiver/initializer owner，
并在调用前安装 private brand；该阶段不调用 JS。随后新路径验证已认证的普通
bytecode、检查预算、消费 constructor 并保留 receiver，安装零参数子帧，正常
返回丢弃结果。品牌安装后不存在交接/重放出口；发布 metadata 已限定此类
initializer 为 Normal。无初始化器直接前进；JS 错误进入当前帧展开。
测试覆盖带私有字段/方法的基类、派生类初始化以及字段初始化函数抛错，均零
旧分派/交接。最终 driver 31 项、class 77 项、run 9 项、新旧配置各 907 项常规
oracle、非 profiling 构建、边界 scan-only、格式/diff 与源码布局通过（507 文件）。
静态初始化、eval/with、IteratorClose 和 S04 其他余项仍未完成，S04 未验收。

S04 静态元素初始化器与 static block 调用已接入同一显式帧 helper。
Runtime::begin_class_static_initializer 和 begin_class_static_block 从原调用入口
提取，保留原有身份/realm/fresh 状态验证、一次性静态状态提交、HomeObject
安装与私有品牌操作。实例、静态元素、static block 共用 InitializerKind 分支后
的普通 bytecode 验证、预算检查、零参数构帧和 ReturnValue::Discard；begin
之后没有交接出口。静态元素保留 constructor，static block 消费 block，接收者
来自已认证父初始化器的 this。旧路径调用同一 begin 后继续原 call_internal。
新增真实发布字节码入口检查点测试验证静态元素 → block 的三层显式帧、抛出
42、初始化启动后不可再次启动，以及帧释放后 active guard 清空；该检查点为
零旧分派/交接，但不是完整 class 表达式零交接证明。
最终 driver 32 项、class 77 项、run 9 项、新旧配置各 907 项常规 oracle、
非 profiling 构建、边界 scan-only、格式/diff 和源码布局检查通过（507 文件）。
完整 class 创建/安装、eval/with、IteratorClose 和 S04 其他余项仍待完成，
S04 未验收。

S04 InstallClassInstanceInitializer 已由 InitializerKind::Install 进入共享冷步骤，
调用原有无 JS 安装内核；成功只消费 initializer 并前进 PC，保留 constructor/
prototype，不创建子帧。发布入口检查点增加了安装后的深度、身份和 PC 断言。
DefineClass 的非对象父类分支进入 construct_driver::define_class：无 heritage
的基类、extends null 和非法 primitive heritage 使用原 define_class_pair 规则，
创建成功后压入 constructor/prototype，错误按当前 realm 进入父 catch。对象
heritage 在消费前交接，prototype getter 的显式恢复状态仍待实现。
新增完整函数执行用例覆盖静态块抛 42、私有实例字段初始化抛 42、extends null
正常完成及非法父类 TypeError，类创建/初始化/异常展开均零旧分派与交接。
函数对象的 C.prototype 读取及错误对象属性读取仍会交接，测试将结果直接返回，
在测试端检查错误类型，未把这些未迁移的读取路径计入零交接声明。
最终 driver 33 项、class 79 项、run 9 项、stack-vm 常规 oracle 907 项、
非 profiling 构建、边界 scan-only、格式/diff 与源码布局通过（507 文件）。
对象 heritage、完整属性协议、eval/with、IteratorClose、host 重入及成本余项
仍待迁移，S04 未验收。

S04 对象 heritage 的普通 prototype 读取及普通 bytecode getter 已进入类创建
恢复流程。原 define_class_pair 的父类验证、prototype 结果类型检查和最终
constructor/prototype 发布拆成共享 helper；getter 回复后的 finish 不再次
读取或验证 parent。PendingClass 在 FrameCold 中拥有 parent、候选 constructor、
name、realm、父帧和 operation identity。getter 使用零参数子帧，返回后先验证
identity、取走 pending，再消费一次读取结果并发布类；抛错进入父 catch。
类专用读取先使用普通 probe，函数对象的 lazy 自有 prototype 使用已有
get_own_property 内核；普通/函数原型链可继续查找。Proxy/其他 exotic 和非普通
getter 在调用前保留旧路径交接，尚未计为完整内部属性协议迁移。带 class_wait
的父帧禁止交接，所有字段初始化入口均已设置该 owner。
新测试用带 prototype getter 的 bound 父类，验证 getter 正常返回与抛错各只
执行一次、非法 prototype 产生 TypeError、最大显式帧深度二、零旧分派/交接
及 active guard 清空。最终 driver 34 项、class 80 项、run 9 项、新旧配置各
907 项常规 oracle、非 profiling 构建、边界 scan-only、格式/diff 与源码布局
通过（507 文件）。初次构建的 AtomError 映射类型错误已修正，最终证据通过。
exotic/non-Normal getter、其他类属性协议、eval/with、IteratorClose 和 host
重入/成本等余项仍待迁移，S04 未验收。

S04 固定名称 DefineMethod 已进入 construct_driver 的独立冷步骤，普通目标/
bytecode constructor 与 bytecode 方法在命名或 HomeObject 安装之前完成分类。
其余目标或 callable 在消费前交接；已接入分支调用原 define_object_literal_method，
保留 method/getter/setter 名称、HomeObject、descriptor 和访问器合并规则。
成功消费 function、保留 base 并前进 PC，属性定义错误按当前 realm 展开。
测试完整类创建后的私有字段方法、getter、继承方法和 getter/setter 合并读取，
均零旧分派/交接。最终 driver 35 项、class 81 项、run 9 项、stack-vm 常规
oracle 907 项、非 profiling 构建、边界 scan-only、格式/diff 和源码布局通过
（507 文件）。默认配置没有本轮生产代码变更，未重复其 oracle。计算名称、
公共字段/其他属性协议、eval/with、IteratorClose、host 重入和成本余项仍待迁移，
S04 未验收。

S04 固定名称 DefineField 接入共享 DefineProperty 冷步骤，方法安装改为同一
入口的 method 参数分支。普通对象与 bytecode constructor 通过目标分类后，
字段使用既有 define_public_class_field，不走普通 Set，不触发继承 setter；
方法保留原有 callable 分类及命名/HomeObject 行为。成功消费 value、保留 base，
字段与方法共享 PC 前进、诊断计数和当前 realm 错误展开。
新用例验证普通字段、继承 setter 不调用、字段按序读取先前字段、函数字段的
this 和初始化函数抛错，均零旧分派/交接。最终 driver 36 项、class 81 项、
run 9 项、新核心常规 oracle 907 项、非 profiling 构建、边界 scan-only、格式/
diff 和源码布局通过（507 文件）。计算名称、exotic 字段目标、eval/with、
IteratorClose 和 host 重入/成本余项仍待迁移，S04 未验收。

S04 DefineFieldComputed/DefineMethodComputed 接入共享 DefineProperty 步骤：静态
名称为 Some(linked index)，计算名称为 None，后者只解析 ToPropKey 的规范化
结果，成功消费 value+key 并保留 base。vm/property_keys::canonical 从旧 host
提取，保持 integer/string/symbol 规则与非法值拒绝，新旧路径共享且不调用 JS。
ToPropKey 的 Int/String/本 runtime Symbol 直接前进；其他输入进入已有
ConversionTask，新增 PropertyKey 完成阶段和 String-hint PrimitiveResume。
对象 key 的 getter/方法按现有 operation identity 恢复一次，结果保留 Symbol、
String，其他原始值按原规则转为 String；直接输入 Int 保留原表示。
测试对象 key 用于字段、方法与 getter，toString 计数为一且 valueOf 未调用；
key 转换抛错先于字段初始化器。数字与 Symbol 用例返回实例，再在测试端按实际
key 检查字段值，避免把尚未迁移的通用方括号读取计入零交接声明。
最终 driver 37 项、class 82 项、run 9 项、新旧配置各 907 项常规 oracle、
非 profiling 构建、边界 scan-only、格式/diff 和源码布局通过（508 文件）。
SetNameComputed、通用属性协议、eval/with、IteratorClose、host 重入/成本等
余项仍待迁移，S04 未验收。

S04 SetNameComputed 已接入共享命名冷步骤。property_keys::computed_name 从旧
host 提取 Int/String/Symbol 描述规则，新旧路径共享，不重复执行 key 转换。
RunExit::SetName 改为 Option(index)，Some 使用发布常量，None 使用栈中 key；
独立 set_name 调用原 define_object_name，保留所有操作数，成功前进 PC，JS
错误按当前 realm 展开。原静态命名代码也移出 resident driver。
新用例验证计算字段匿名函数的字符串/整数名称、带描述 Symbol 的 [x]、无描述
Symbol 的空名称、显式空描述的 []，以及已有函数名 keep 不覆盖，均零旧分派/
交接；字段与函数属性在执行结束后由测试端检查。最终 driver 38 项、class
82 项、run 9 项、新旧配置各 907 项常规 oracle、非 profiling 构建、边界
scan-only、格式/diff 与源码布局通过（508 文件）。通用对象协议、eval/with、
IteratorClose 和 host 重入/成本等余项仍待完成，S04 未验收。

S04 环境入口的无 JS 步骤已接入 environment_driver：VariableEnvironment 校验
发布 metadata 的 eval/参数 eval local 后创建 null-prototype 对象；ToObject
保留对象，其他值使用原 native_to_object 装箱/拒绝 nullish 规则；WithObject
初始化使用 bindings::initialize_local_binding，与旧 host 共享对象/domain 校验、
Direct/Uninitialized 替换及 Captured cell 写入。成功后前进 PC，错误按当前
realm 展开，未迁移动态名称查询或 eval 执行。
测试完整 with 入口的 number/boolean/string 装箱与 null/undefined 可捕获错误，
均零旧分派/交接；真实发布 VariableEnvironment PC 检查点验证 null prototype、
PC 前进和 guard 释放，不作为完整 eval 零交接证明。初次测试发现分派插入未
匹配已格式化源码，以及 eval 函数先执行 Arguments；补齐分派并定位真实 PC
后通过。最终 driver 40 项、with 过滤测试 129 项、run 9 项、新旧配置各
907 项常规 oracle、非 profiling 构建、边界 scan-only、格式/diff 与源码布局
通过（509 文件）。动态环境查找/读写、direct eval、IteratorClose、host 重入
及成本余项仍待完成，S04 未验收。

S04 eval/with 隐藏对象解析提取为 environment_bindings，新旧路径共享发布 local
身份、closure descriptor/捕获 cell metadata、对象/domain 与 eval Ordinary
payload 校验；eval 对象泄露后可改 prototype，未重新强制 null prototype。
新帧通过其当前窗口读取 local，closure root 保持显式 owner。
HasDynamicBinding/GetDynamicBinding 的普通数据路径、DynamicEnvironmentObject
和 GetRefValue/GetRefValueUndef 的普通对象引用读取进入 environment_driver。
Has 检查属性存在后按 with 规则读取 Symbol.unscopables 及同名排除项，Get 在
执行时重新 probe。引用读取保留环境对象并压入值，以支持 with 方法 this。
getter/exotic 探测在调用前交接；普通缺失读取保留 strict/undefined 规则，
尚未迁移动态写入、删除及完整 getter/Proxy 恢复。
新用例验证数据读取、unscopables 排除与不排除、外层 lexical 回退、闭包中的
with 环境与方法 receiver，均零旧分派/交接。初次 with 方法用例暴露 GetRefValue
缺口，补齐普通引用读取后通过。最终 driver 41 项、with 过滤测试 130 项、
run 9 项、新旧配置各 907 项常规 oracle、非 profiling 构建、边界 scan-only、
格式/diff 和源码布局通过（510 文件）。direct eval、IteratorClose、host 重入
及成本等 S04 余项仍未完成，S04 未验收。

S04 GetDynamicBinding/GetRefValue 的普通 bytecode getter 已接入显式子帧。
BindingRead 区分数据值、选中的 getter/receiver 和交接；名称查询完成后普通
getter 创建拥有零实参、realm、receiver、父 FrameId 和 Push 返回目标的请求。
预算检查在推进 PC 前完成，父环境引用保留；安装子帧后沿通用返回/抛错展开，
不重放 lookup 或 getter。非 Normal getter 与 exotic 仍在调用前交接。
新用例验证 getter 单次读取、抛错、闭包中的 with 以及 getter 返回函数后
仍使用环境对象作为 this，均零旧分派/交接且至少两层显式帧。最终 driver
42 项、with 过滤测试 131 项、run 9 项、新核心常规 oracle 907 项、非 profiling
构建、边界 scan-only、格式/diff 和源码布局通过（510 文件）。unscopables
及排除项 getter 的 HasBinding 恢复、动态写入/删除、eval、IteratorClose 和
host 重入/成本余项仍待完成，S04 未验收。

S04 HasDynamicBinding 由 with_driver 推进，PendingHas 在 FrameCold 中拥有 key、
realm、父 FrameId、operation identity 及 Unscopables/Excluded 阶段。初始普通
HasProperty 为 false 时立即返回；为 true 后逐步读取 Symbol.unscopables 与
同名排除项。普通 bytecode getter 使用显式子帧，回复取走并核对唯一 pending，
继续下一阶段而不重放已完成检查；最终 Get 指令仍按语义重新查询目标属性。
初始 exotic HasProperty 在调用前交接；后续特殊读取/非普通 getter 仅允许
当前步骤的旧同步 fallback，不重放整条指令，此部分仍归后续完整协议迁移。
父帧带 PendingHas 时禁止交接，所有 FrameCold 构造处已初始化该字段。
测试计数验证 unscopables→排除项→实际 getter 顺序 123、排除时不读取实际
getter、两阶段抛错及缺失属性不读取 unscopables。getter 返回新建排除对象的
用例暴露 Object 指令缺口，接入原 realm 普通对象创建内核后整条路径零交接。
最终 driver 43 项、with 过滤测试 131 项、run 9 项、新核心常规 oracle 907 项、
非 profiling 构建、边界 scan-only、格式/diff 和源码布局通过（511 文件）。
动态写入/删除、direct eval、IteratorClose、host 重入/成本及剩余特殊对象
协议仍待完成，S04 未验收。

S04 DeleteDynamicBinding/DeleteEvalVariable 的普通目标进入环境冷步骤，使用
共享隐藏对象身份解析与 linked key，确认 Ordinary kind/payload 后调用原
internal_delete_property。删除不读取属性值或调用 getter，保留 sloppy Boolean
结果；exotic 目标在修改前交接。动态读取与删除共用 linked_key 解析。
新用例验证可配置属性删除、不可配置属性 false、getter 不被调用、继承属性
不受影响、unscopables 排除及缺失属性回退外层 lexical，均零旧分派/交接；
删除后自有属性状态由测试端复核。最终 driver 44 项、with 过滤测试 131 项、
run 9 项、新核心常规 oracle 907 项、非 profiling 构建、边界 scan-only、格式/
diff 与源码布局通过（511 文件）。动态写入、完整 eval、IteratorClose、host
重入/成本及特殊对象协议仍待完成，S04 未验收。

S04 HasEvalVariable/GetEvalVariable 复用共享环境身份解析、普通存在检查和读取，
eval 查找跳过 unscopables；普通 getter 保持显式子帧与原接收者。
DefineEvalVariable 在确认普通目标后消费值，以原完整 writable/enumerable/
configurable 数据描述符调用 define_own_property_in_realm；拒绝定义仍产生
当前 realm 的 TypeError，不调用被替换的 getter，也不重放已提交操作。
测试通过 eval 专用发布验证和真实外部根进入已发布 eval 体，补齐 Global
回退 closure 根，未放宽发布校验或增加生产 VM 对 compiler 的依赖。
声明顺序、函数声明调用、删除、继承 getter/抛错、访问器替换、不可配置拒绝
和跳过 unscopables 均零旧分派/交接。最终 driver 45 项、eval 过滤测试 137 项、
run 9 项、新核心常规 oracle 907 项（1 项忽略）、非 profiling 构建、边界
scan-only、格式/diff 与源码布局通过（511 文件）。本轮未更改默认路径生产
代码，未重复默认 oracle。完整 direct eval 编译/调用入口仍未迁移，动态写入、
IteratorClose、host 重入/成本及特殊对象协议仍待完成，S04 未验收。

S04 PutDynamicBinding/PutEvalVariable/PutRefValue 进入环境冷步骤。WriteTarget
区分隐藏动态环境和操作数引用；普通 kind/payload 与只读属性链探测确认无
exotic 协议后，复用原 prepare_set_property_with_receiver_in_realm。引用和动态
写入在 RHS 后重新检查存在性，eval 专用写入保留原 sloppy Set 规则。
普通 setter 使用带一个参数的显式子帧，ReturnValue::Discard 丢弃正常返回，
抛错仍展开；非普通 setter 在调用和操作数消费前交接。特殊对象仍待迁移。
PutRefValue 的自有 VarRef 分支以共享 cell 证明属性存在，保留 TDZ、strict
const 拒绝/sloppy const 忽略与 write_var_ref，不把 lexical 槽替换成数据属性。
完整函数测试覆盖数据/继承属性、setter 接收者与单次调用、返回值丢弃、抛错、
只读/无 setter 拒绝以及 RHS 删除目标后的重新写入；已发布 eval 体补充赋值。
真实 PutRefValue PC 检查点验证可变/const/TDZ、严格模式及 VarRef 身份保留。
普通 PutField 尚未迁移，setter 测试以捕获变量记录写入并读取 this 验证接收者，
不把该依赖的旧路径交接当成本轮动态写入的零交接证据。
最终 driver 47 项、eval 过滤测试 137 项、with 过滤测试 131 项、run 9 项、
新核心常规 oracle 907 项（1 项忽略）、非 profiling 构建、边界 scan-only、
格式/diff 与源码布局通过（511 文件）。默认生产路径未改，未重复默认 oracle。
完整 eval 编译/调用入口、IteratorClose、host 重入/成本与剩余协议继续待办，
S04 未验收，S05–S10 未开始。

S04 GetRefValue/GetRefValueUndef 的 undefined 环境直接产生原 ReferenceError，
普通自有 VarRef 先确认属性存在，再复用 get_own_property 的 cell 读取与 TDZ
诊断。该分支不替换属性槽，环境对象保留在操作数栈，读取值追加到栈顶。
真实 compound-assignment 的读取 PC 检查点覆盖初始化值、TDZ 和未解析引用，
复核栈形状、恢复 PC、VarRef 身份、零旧分派/交接及帧 guard 释放。
最终 driver 48 项、eval 过滤测试 137 项、run 9 项、新核心常规 oracle 907 项
（1 项忽略）、非 profiling 构建、边界 scan-only、格式/diff 与源码布局通过
（511 文件）。默认生产路径未改，未重复默认 oracle。GlobalReference、完整
直接 eval 编译/捕获/调用入口、IteratorClose 等仍待完成，S04 未验收。

S04 GlobalReference 的 closure 描述符/所属 runtime 校验与当前全局 lexical
绑定解析抽入 environment_bindings，新旧路径共享。后发布的同名 lexical
VarRef 优先于旧 closure 的全局属性解析，TDZ/const 仍在 RHS 执行前抛错。
owned 环境步骤对全局对象复用既有 HasOwnProperty，自有属性缺失时用不调用
getter 的普通原型链探测，exotic 原型在调用前交接。全局对象有专用 payload，
不把它误认成普通存储；现有普通 Set 白名单暂未涵盖该 payload。
完整函数测试验证发布后新增 mutable/const/TDZ lexical 的选择与错误优先级，
mutable 写入共享 cell 且原全局属性不变，均零旧分派/交接。真实 GlobalReference
PC 检查点覆盖数据属性、自有/继承 getter 与缺失属性，getter 抛错体不被执行。
普通全局属性完整写入的初始测试暴露专用 payload Set 尚未迁移，保留为待办，
未把检查点证据扩大为完整全局赋值的零交接证明。
最终 driver 50 项、eval 过滤测试 138 项、run 9 项、新旧配置各 907 项常规
oracle（各 1 项忽略）、非 profiling 构建、边界 scan-only、格式/diff 和源码
布局通过（511 文件）。完整直接 eval、全局对象读写协议、IteratorClose、
host 重入/成本及其余 S04 协议继续待办，S04 未验收。

S04 全局对象的环境引用读写接入专用 payload 分支。prepare_environment_read
复用 get_own_property 返回数据/getter，getter 仍在 owned 子帧调用；缺失属性
走普通原型探测，auto-init 与特殊原型保留调用前交接。字段读取也复用该步骤，
使全局 setter 的 this 属性读取保持 owned 执行，并将 JS 可见读取错误转为
当前 realm 的 Throw。全局写入复用原 Set/全局存储内核，保留 VarRef 和未解析
名称表；引用到自有 VarRef 时沿用上轮的 TDZ/const 与 write_var_ref 规则。
上轮普通全局属性完整赋值的缺口现已补齐，恢复该用例的零旧分派/交接要求。
新用例覆盖缺失名称创建、getter+setter compound assignment、全局 this、
setter 单次执行及返回值丢弃、getter 抛错先于 RHS、sloppy 只读忽略和 strict
未解析名称错误；执行后由测试端复核存储值。
最终 driver 51 项、eval 过滤测试 138 项、run 9 项、新核心常规 oracle 907 项
（1 项忽略）、非 profiling 构建、边界 scan-only、格式/diff 与源码布局通过
（511 文件）。默认生产代码未改，未重复默认 oracle。auto-init/特殊原型、
完整直接 eval、IteratorClose、host 重入/成本及其余 S04 协议仍待完成。

S04 direct eval 的编译/捕获准备与本体执行拆分。新增 DirectEvalPreparation，
Complete 承载非 String 直接结果或编译异常，Ready 持有已完成声明实例化的
closure 与 this。prepare_direct_eval_original 保留原身份/环境检查、编译发布、
编译成功后的 FnOnce 捕获及 new_eval_bytecode_closure 顺序，不执行 eval 本体。
旧 call_direct_eval_original 消费准备结果，Ready 仍交给原 call_internal；
因此本轮是 owned eval 入口的必要边界调整，尚未把完整 eval 调用迁移到 owned
子帧，也未新增编译器生产依赖到 VM。
最终 eval 过滤测试 138 项、driver 51 项、新旧配置各 907 项常规 oracle（各
1 项忽略）、非 profiling 构建、边界 scan-only、格式/diff 与源码布局通过
（511 文件）。后续须将 eval_validation 的帧身份检查和调用者 capture 接入
owned slots，再消费 Ready 推入显式子帧；IteratorClose 等 S04 余项继续待办。

S04 新增 vm/eval_bindings，共享 eval 调用者 strictness、local/argument 边界及
closure cell metadata 校验。local/argument 以实际存储长度检查，closure 仍按
发布描述符验证实际 root；原错误顺序和消息不变，原反例测试继续经 host 适配
调用共享验证器。materialize 按原 scope/binding 顺序遍历，closure root 直接
保留，local/argument 通过存储适配回调交给 capture_frame_binding。
PreparedEvalEnvironment/MaterializedEvalEnvironment 移入共享模块，旧 host
重导出类型以保留现有调用点；共享模块不依赖旧 host 的存储结构。旧 host 的
capture descriptor 构造和 roots 遍历已删除，改用共享实现。编译成功后才捕获
的时序仍由上一轮 prepare_direct_eval_original 保证，owned Eval 尚未接入。
最终 eval 过滤测试 138 项、driver 51 项、新旧配置各 907 项常规 oracle（各
1 项忽略）、非 profiling 构建、边界 scan-only、格式/diff 与源码布局通过
（512 文件）。下一步接入 owned slots 适配和显式 eval 子帧，S04 仍未验收。

S04 新增 vm/eval_driver，普通 Eval 先按当前 realm 验证原始 eval 身份；被替换
的 callee 按普通调用传递全部实参。原始 eval 仅对 String 输入规范化 this、
校验环境并编译，编译成功后通过 owned local/parameter 槽适配共享 capture。
Ready closure 进入显式子帧，不增加原生 eval 调用帧；非 String 输入直接返回。
父帧保留 function 和全部实参，OperationTarget::Eval 在子帧返回/抛错后清理，
再交付结果或展开，保持额外实参的存活期。SlotStore 提供经过 window 校验的
binding_counts，现有帧/槽预算在进入已准备 eval 子帧前检查。
完整测试首先发现 GetVar 读取原始 eval 仍会交接，已补齐 GetVar/GetVarUndef：
保留已初始化 cell 快路、由 closure descriptor 决定的 TDZ，以及普通全局
getter/缺失读取规则；特殊协议仍在调用前交接。普通 Eval 测试验证 caller
lexical 读写、sloppy var 声明、strict/nested eval、Throw/语法错误/TDZ、非
String 与额外实参、副本 eval 函数调用，均零旧分派/交接。bound 数字 this
用例验证相同 boxed 对象及三层 owned 帧。
最终 driver 53 项、eval 过滤测试 140 项、run 9 项、新核心常规 oracle 907 项
（1 项忽略）、非 profiling 构建、边界 scan-only、格式/diff 与源码布局通过
（513 文件）。默认执行路径未改，未重复默认 oracle。ApplyEval 展开实参、
IteratorClose、host 重入/成本及剩余 S04 协议仍待完成，S04 未验收。

S04 ApplyEval 的实参准备先建立无回调的 Array 预检边界。新增 Runtime
prepare_fast_array_arguments：检查 runtime 和 Array kind/payload，通过原
get_own_property 读取不可变种类的数据 length，保留 65534 实参限制，再复用
fast_array_like_values。非 Array、访问器索引或非快存储返回 None，留给后续
显式属性读取协议；不尝试调用 getter。现有 build_array_like_argument_list
消费该预检结果，其余情况继续原有 length/ToLength/逐项读取顺序。
新测试覆盖 length/index getter 不执行、非 Array 原型 getter 不执行、稠密
值顺序及超限 RangeError。最终 Reflect 过滤测试 8 项、新旧配置各 907 项常规
oracle（各 1 项忽略）、非 profiling 构建、边界 scan-only、格式/diff 和源码
布局通过（513 文件）。ApplyEval 仍未进入 owned 调用：实参数组不能直接展开
到仅有两个操作数位置的父帧，后续还须持有完整实参快照至子帧结束，并处理
替换 callee、属性读取回调与恢复。S04 未验收。

S04 ApplyEval 的稠密 Array 快路进入 eval_driver：先完成实参预检，再检查原始
eval 身份，保持读取/超限错误优先于 callee 检查。原始 eval 的快照由父帧
FrameCold::eval_arguments 持有，输入取快照首项，父帧仍保留 callee/array 两个
操作数。普通与 bound 替代 callee 使用完整列表及原 bound 拼接规则进入子帧。
Eval 回复统一清理两个操作数与快照；立即结果、编译/准备失败和预算拒绝也
释放快照。所有 FrameCold 初始化和旧路径交接检查均包含新 owner。
execute 的同一运行循环抽为 execute_running，便于从真实已发布 ApplyEval PC
检查点完成子帧与父帧恢复，未复制执行循环。检查点覆盖 String/non-String/
空输入、Throw、普通/绑定替代函数，均零旧分派/交接。额外实参保活测试在
子帧启动后删除原数组元素，确认对象仍有强引用，调用返回后该引用释放。
最终 driver 54 项、eval 过滤测试 141 项、run 9 项、新核心常规 oracle 907 项
（1 项忽略）、非 profiling 构建、边界 scan-only、格式/diff 与源码布局通过
（513 文件）。默认生产路径未改，未重复默认 oracle。通用属性实参读取仍
在回调前交接；ArrayFrom/Append/迭代器等展开前置构造尚未迁移，检查点不
代表完整 spread 表达式零交接。IteratorClose 等 S04 余项继续待办。

S04 ArrayFrom 进入 environment_driver::CreateArray 冷步骤：移动并按原顺序
整理操作数，交给原 new_array_from_values 创建 callee realm 的新数组，再将
数组压回栈。未另写元素属性规则，继承 setter 仍由原 fresh-array 内核绕过。
完整函数测试覆盖空数组、实参与常量顺序、eval 返回数组，以及从另一 realm
调用时使用 callee Array.prototype；原型索引 setter 的抛错体未执行，均零
旧分派/交接。最终 driver 55 项、eval 过滤测试 141 项、run 9 项、新核心常规
oracle 907 项（1 项忽略）、非 profiling 构建、边界 scan-only、格式/diff 与
源码布局通过（513 文件）。默认生产路径未改，未重复默认 oracle。
Append 的 GetIterator/迭代读取、通用实参读取和 IteratorClose 等 S04 协议
仍待完成，尚不证明完整 spread 表达式零交接。

### S04 数字索引元素定义检查点

DefineArrayEl 经现有 Environment 冷步骤出口进入 array_driver，普通对象和
Array 的数字索引复用原 define_own_property_in_realm 内核，定义完整可写、
可枚举、可配置数据属性；成功只弹出值，保留数组和索引。其他对象/键类型
在转换或写入前交接，未接通对象键转换和 Array length 转换。
测试从真实展开字面量发布的 DefineArrayEl PC 恢复，核验原型 setter 不执行、
孔位保留、长度扩展、栈布局与后续返回；不包含此前的 Append。
最初另加 driver 分支使有限栈 Proxy oracle 失败，接回现有冷步骤入口后
恢复；未调整原生栈预算或测试预期。最终 driver 56 项、eval 141 项、run 9 项、
新核心常规 oracle 907 项（1 项忽略）、构建、边界 scan-only、源码布局通过
（514 文件）。生产改动仅在 stack-vm 配置下启用，默认 oracle 未重复。
Append 的两次 Symbol.iterator 读取、迭代状态和异常 IteratorClose 仍待接通，
S04 尚未验收。

### S04 Append 的迭代调用与关闭状态

Append 通过现有冷步骤入口进入 append_driver。父帧冷数据保存待返回操作，
输入数组、索引、源值保留至操作结束；FrameId 与单调增长的帧内操作代次
校验返回目标，禁止带 pending 的父帧整体交接。驱动循环推进两个独立的
Symbol.iterator Get、方法调用、一次 next 读取、next 调用、done/value 读取
与元素定义。普通及 bound bytecode getter/method 使用显式子帧，无持有
heap/slot 借用跨越调用；原生/特殊属性和非普通函数仍按选定单步骤同步执行，
留待对应 S05/S06 领域迁移，不重放完整 Append。

新旧路径共享 iterator_support 中的原 native target 分类与快数组快照内核，
保持第一查询结果在第二查询前释放、快照在 iterator/next 创建后取得的顺序。
元素写入使用原完整自有数据描述符内核，索引按 u32 回绕。done 为真时不读取
value；迭代记录尚未完成时抛错不关闭。next、done/value 或写入失败则读取并
调用 return，保留原异常；关闭错误或正常返回原始值都不覆盖它。迭代结果
对象在写入/异常关闭前释放，源值则独立保活到整个操作结束。该关闭状态
目前只接 Append，尚不代表 for-of 或通用 unwind 的 IteratorClose 已接通。

新增完整函数测试覆盖稠密数组/String 展开、eval(...数组)、两次查询与 cached
next 的回调日志、完成时跳过 value、正常结束不关闭 iterator，以及
next/done/value 失败和关闭 getter/call/非 callable 的原异常优先级。真实 Append
检查点另验证不可扩展目标写入失败仍关闭及 u32 索引回绕。2048 元素的普通
iterator 在 2 帧上限下完成，峰值为 2 帧，零旧分派/交接。最终 driver 61 项、
eval 142 项、run 9 项、新旧配置各 907 项常规 oracle（各 1 项忽略）、构建、
边界 scan-only 和源码布局（516 文件）通过。S04 仍待 for-of 迭代记录、
通用异常关闭等协议完整迁移；既有两项小栈基线未在此声明解决。

### S04 for-of 记录与同步异常关闭

原 append_driver 扩为共享 iterator_driver，冷状态归 PendingIterator，使用
iterator_wait/iterator_generation 校验返回。Append 保留原两次查询与异常
关闭策略；ForOfStart 只读取一次 Symbol.iterator，调用后缓存 next，并把
iterator/next 两个 owner 安装到操作数槽，区域仅保存 record_base 和 enabled。
源操作数在 Start 入口移走，在调用 iterator 方法时转移 receiver，避免把
已不需要的 source 保活到 next getter。ForOfNext 验证 offset 与最内层同步
记录，返回 value/done；完成或 next/done/value 失败时禁用并释放 iterator 槽，
后续关闭不再调用 return。该失败策略与 Append 分开。

IteratorClose、IteratorClosePreserve 和对应 drop/detach 清理使用同一记录
提取规则，保留顶端返回值并清除中间操作数。普通关闭检查 return 的调用
结果必须为 Object，消息沿用原 “not an object”；带原异常的关闭忽略 getter、
调用、非 callable 及返回值错误，保留原异常。iterator_driver/regions 接管
原 catch unwind 与 iterator unwind：循环体异常移走记录并通过显式子帧关闭，
随后继续外层区域；catch 捕获复用标记和操作数清理顺序保持。普通及 bound
回调继续使用显式子帧，原生/特殊读取与非普通函数的单步骤同步回退仍待
相应 S05/S06 领域迁移。

完整函数用例覆盖 for-of/continue、跨迭代闭包、数组解构、一次迭代查询与
cached next、完成时跳过 value、break/return/body throw、next/done/value
失败不关闭、普通关闭返回原始值报错、内外 iterator 关闭顺序与 finally
覆盖异常；均零旧分派/交接。Append 原有测试同时保留。最终 driver 65 项、
eval 142 项、run 9 项、新核心常规 oracle 907 项（1 项忽略）、构建、边界
scan-only 和源码布局（517 文件）通过。此次生产改动仅在 stack-vm 下启用，
未重复默认 oracle。S04 仍需 Apply/构造展开、host delimiter 和完整阶段验收；
本记录不代表 S04 已完成，也不代表历史小栈基线已解决。

### S04 Apply 与构造展开入口

Apply(Call/Construct) 和 ApplySuper 进入 apply_driver，沿已有 driver 构造出口
分配 operation identity；未新增每条调用独立等待的 Rust 递归路径。先复用
callable_from_value 检查 callee，再复用稠密 Array 实参预检/快照。null/undefined
列表保留原 QuickJS 普通调用快捷分支，即使 magic 是 Construct 也不检查构造
能力；其他列表先构造实参，随后才做 constructor_from_value 检查。

普通/bound bytecode callee 经原 bound 参数拼接内核进入显式子帧，保留实际
receiver 和参数顺序；callee entry 拥有实参后才释放三个原操作数。构造路径
直接复用 construct_driver::enter_request，operand_count 为 3，保留 raw
new.target、bound new.target 重定向、prototype getter continuation 与
base/derived 返回规则。非稠密 Array 或需通用读取的列表，以及未迁移的
callee 领域，仍在无 JavaScript 副作用且原操作数/PC 未改时交接。

完整函数测试覆盖普通/方法/bound 调用、空列表、抛错、256 层递归展开、
new C(...args)、显式 super(...args)、默认 derived 转发与 bound constructor。
真实 Apply 指令检查点覆盖调用能力早于列表读取、列表上限早于构造能力、
非对象列表错误和 nullish Construct 的普通调用快捷分支。构造 prototype
getter 子帧运行期间删除载体元素，原参数仍由 pending request 保活并作为
构造结果返回；释放结果后对象引用归零。上述用例均零旧分派/交接。
最终 driver 69 项、eval 142 项、run 9 项、新核心常规 oracle 907 项（1 项忽略）、
构建、边界 scan-only 与源码布局（518 文件）通过。生产改动仅在 stack-vm
配置下，未重复默认 oracle；S04 仍待全局写绑定、host delimiter、readonly
view/调用成本与小栈等完整验收，尚不表示整个阶段完成。


### S04 全局写绑定与小栈恢复（实施中）

PutVar/PutVarInit/DeleteVar 现在与旧入口共享 global binding 分类，保留
TDZ、const、属性 setter 和删除时先探测存在性的规则。未迁移 callee 使用
单次同步调用边界，父 owned 帧不重放已经完成的指令；它仍可能递归等待内部
原生回调，不能计作 S05 continuation 完成。Array 的命名/符号属性缺失可以
沿普通原型探测，数字索引缺失仍走既有完整数组规则。

GDB 确认了有限 TypedArray 回调的额外原生栈占用：根帧虽然释放 owned
arena，原 driver 的 Rust 帧却仍等待旧执行器返回。现在 RunningExit 穿过
owned driver 和入口准备函数，原 bytecode 入口再消费 RootHandoff；原
ActiveFrameGuard 在交接结果返回后结束。共享构帧及 owned 入口准备均在
执行前返回，未增加堆分配。旧 Call/CallMethod 分派单独保留原 checked
argument suffix，扩展调用仍执行原有 tail/eval/construct/import 代码；
源码契约分别密封两个函数，增加普通/方法 receiver 和扩展路由反例。

原有有限用例和 native 栈预算保持不变：32 层调用、TypedArray join 18 层、
toLocaleString 20 层及 sort/toSorted 12 层均已通过。旧溢出用例把 1000 层
有限递归当作必然溢出，这不再适用于显式帧；用 Infinity 作为递归参数后，
原来的两个 InternalError 与恢复断言同时覆盖新 VM 帧预算和旧 native
预算。另将 owned 小栈有限递归从 256 层增强到 1000 层，明确断言 1001 个
显式帧、零旧 VM 分派和零交接。没有提高任何预算或增加 skip。

此版本新旧 native_stack 各 6 项、新旧常规 oracle 各 907 项通过（每配置
另有 1 项手动压力用例忽略，本轮未运行）；owned driver 76 项、run 9 项、
eval 142 项、整个 VM 209 项及 modules 134 项通过。非 profiling stack-vm
构建和源码布局（521 文件）通过。完整边界反例 714 项全部拒绝；真实 host
重入 delimiter、readonly view 的 owned 定向证据及完整调用成本审计尚待
完成，S04 仍未验收，S05–S10 尚未开始。


### S04 host 重入、只读视图与调用诊断

HostBoundaryGuard 只登记 domain、执行和父 active-frame 身份，回调期间不
持有内部借用或 Runtime-owning registry entry。同步 module host callback
在既有 native guard 内建立 delimiter，返回时检查子执行已结束和父帧已
恢复；错误或 Rust unwind 通过 Drop 清理。它不把内部 native callback
标成 host，也不新增可挂起 ABI。

实际重入用例通过既有 Promise rejection tracker 触发同步模块编译，在
owned 父帧仍存活时进入 loader。loader 重入闭包、把捕获绑定从 1 改为 41，
并调用 GC；正常及 loader 拒绝时父帧继续得到 42，Rust panic 时检查精确
payload 与登记清理，随后闭包和普通求值仍正常。tracker 只作为测试触发器；
Promise 和其 host 入口的完整集成仍归 S06/S07，未改变 dynamic import 的
job 调度。登记生命周期测试共 6 项通过。

模块 live import 的定向测试在模块初始化后分别测量普通读取、exporter
更新、直接写入和 eval 写入。读取观察到 live cell，写入仍抛出原只读消息，
失败后保持值 42；每个测量调用均为零旧分派、零交接。此证据验证 S04
readonly view，不把旧模块初始化计作 S07 完成。

调用准备诊断计入实际参数/局部 Vec 的容量分配和初始化、参数复制中的
Object/Symbol root 子集、共享构帧的 callee root 复制、owned FrameCold
Box 及捕获复用表 Vec 分配。incoming argument Vec 的容量只作观察，不能
与 allocation 相加。CLI JSON 和 human 输出使用同一快照；缺参、多参、
对象实参及函数体抛错用例验证这些计数。完整调用分配、Runtime retain/
release、临时 bound/apply buffer 等未覆盖量明确排除，不报成零或正式性能
成绩；S09 负责完整调用成本审计与优化。

完整库检查补获一元 `+` 的恢复偏差：原恢复调用 generic native_to_number，
BigInt 消息变为通用转换错误，回调产生的整值 Float 也会被压成 Int。现在
两条 VM 路径共用 unary_plus_primitive，保留原 BigInt 诊断及 Float tag 和
bits。既有 binary scalar 断言未修改；新增 ordinary valueOf 回调验证 Float、
负零、NaN payload 与 BigInt throw，在 2 个显式帧内完成且零旧分派。

修正后完整库测试 owned 2092 项、默认 1981 项通过。以下最终阶段审查
及同源门禁结果构成 S04 范围内的验收，不代表完整执行迁移完成。


S04 逐项阶段审查（已验收）：

| 要求 | 当前实现与本轮证据 | 后续边界 |
| --- | --- | --- |
| ordinary/bound/constructor 与 realm | call/construct/apply driver 的显式子帧、原始实参与 this/new.target/derived-return 测试 | exotic/native 领域调用点归 S05 |
| 一条完整 getter/valueOf 恢复 | conversion 的消费式阶段与父 operation 身份；嵌套回复、单次调用、绑定修改及抛错测试 | 其他同步内置转换归 S05 |
| catch/finally/return/throw 与 IteratorClose | regions 和 pending completion；for-of/Append 测试覆盖 break、return、next 失败、清理抛错和原异常优先级 | async close 和挂起归 S06 |
| lexical/capture/eval/private/readonly | driver 的绑定、每迭代 cell、默认参数、mapped/unmapped arguments、with、direct eval、class/private 与 live-import 定向测试 | 模块实际入口归 S07 |
| native/VM 限额及真实 host 重入 | 原有限小栈回归、1000 层显式帧、可捕获无限递归；真实 loader 重入的 GC/错误/panic 与登记清理 | 全部 host 入口集成归 S07 |
| 调用存储成本 | 同一 profiling 快照中的构帧分配、初始化和受限引用事件；参数与主体抛错计数测试及 CLI JSON 输出 | 全调用分配、引用总账和布局优化归 S09 |

新旧常规 oracle 各 907 项通过后，另行执行的 65K 实参压力用例也各通过
1 项，合计每配置 908 项。新旧 CLI profiling 各 5 项以及关闭 profiling 的
stack-vm 构建通过。上述验证使用一元 `+` 修正后的生产源码；期间仅更新
迁移文档，没有继续修改生产或 checker 源码。


完整 binary-object 门禁最终退出码 0，714 个隔离反例全部拒绝；源码布局
521 个可达 Rust 文件、格式及 diff 检查通过。修正前那次库检查的一元 `+`
错误已由共享规则修复；其边界任务因源码变更终止，不计作通过。本次结果
来自修正后的新任务。S04 阶段验收通过，S05–S10 尚未开始。


## S05 首批属性读取与显式调用请求（实施中）

object 的 prepare_ordinary_read 现在复用完整的非 Proxy 描述符读取内核：
Array 孔位继续查原型，TypedArray 的 invalid/detached 整数索引完成 undefined
而不读原型，Arguments/String/namespace live cell/autoinit 仍调用原存储规则。
准备阶段只返回已选 getter/receiver 或 unresolved Proxy，不调用 JS。
旧同步读取与 owned 读取共用这一步，删除了特殊节点上的重复 getter 调用。
新增源码契约及 mutation 保证准备函数不回调且验证输入域。

GetField/GetField2 的调度归 vm/property_driver；GetArrayEl/2/3 对 object
base 和原语 key 使用同一入口。GetArrayEl3 在 getter 前保留 canonical key，
即使 getter 改写 key 变量，之后的旧 Set 交接仍使用原键；nullish 检查保留
普通读取与更新读取的不同消息，先于对象键回调。对象 key 的 ToPrimitive、
原语 base、bound/native getter 与 Proxy 仍在未消费输入前交接，继续属于 S05
待办，不把这批改动称为完整 Get 迁移。

非 Proxy 查找覆盖扩大后，TypedArray 的命名读取不再提前交接根帧，暴露了
临时 native 调用保留整个 resident dispatcher 原生帧的额外开销。调用现在先
形成独立 PendingCall，RunningExit::Call 把父执行、转换等待和 operation
代次一起交回外层 driver；分派函数返回后才调用原 Runtime 入口，回复再
恢复同一父帧。原有 native 栈预算和有限深度不变，VM 组的小栈回归已恢复。
它仍是内部同步桥，后续各内置必须继续改为领域恢复，不能以此完成 S05。

新计数 owned_sync_call_bridges 与旧分派/整帧交接分开；无回调 native callee
也计入，不代表全部内部嵌套调用总数。新增属性读取用例三个计数均为零，
Array.map 对照用例仍报告一个同步桥。放弃已准备请求的测试验证未调用
callee 且 Runtime 可释放。临时请求/continuation Box 分配未包含在 S04
call_preparation 范围，profiling.md 已明确披露。

本批 driver 82 项、完整 owned 库 2097 项、默认库 1981 项、新旧常规 oracle
各 907 项及新旧 CLI profiling 各 5 项通过；各配置另有 1 项 65K 手动压力
测试本批未重跑。非 profiling stack-vm 构建、边界 scan-only、2 项属性契约
测试（包括新增非法回调 mutation）、格式/diff 与源码布局 522 文件通过。
本批没有重跑完整 714 项 boundary，不借用 S04 结果充当当前完整门禁。

[同步回调调用点账本](primitive-vm-sync-callbacks.md) 记录首轮 121 个直接
call/construct 表达式及后续阶段责任；间接 helper 调用图仍须补查。
S05 尚未验收，整体 S01–S10 目标保持不变。


## S05 对象键与 Proxy Get 阶段推进（实施中）

Get 的共享准备入口已覆盖原语 base、String code unit/length，以及对象 key
的 string-hint ToPrimitive；保留先求值的 base、GetArrayEl3 的 canonical key、
跨 realm 转换错误和异常身份。bound getter 与转换方法共用调用归一化。

Proxy Get 的方法查找、调用和 invariant 阶段归 object/internal_methods/get；
旧同步入口和 owned VM 消费同一协议。新 proxy_get_driver 在 frame/operation
身份下安装字节码子帧，嵌套 handler Proxy 的读请求用继续状态栈推进，回复
不重放 handler getter 或 trap。查找保存的 target/handler 与方法深度 guard
都有唯一释放责任；null/undefined trap 转发、SameValue（含 NaN 与 ±0）、
无 getter 的不可配置访问器、异常与读取 realm 保持原规则。

本批新配置库 2108 项、旧配置库 1983 项通过，包含 91 项 owned driver 回归；
两种配置的 Proxy/Reflect oracle 各 16 项通过。新增丢弃/错误回复测试检查
roots 与方法 guard 释放；新增源码反例拒绝 Proxy Get 阶段直接执行回调。
新旧配置完整常规 oracle 各 907 项通过（各 1 项 65K 手动压力测试本批未重跑）。
边界 scan-only、2 项属性契约/mutation、源码布局 524 文件及格式/diff 通过；
本批未重跑完整 714 项 boundary，本段不是 S05 阶段验收。

尚未迁移的 Proxy GetOwnProperty target invariant 查询，以及 native/Proxy
callable 继续通过显式计数的同步桥；Set、Object/Reflect 与其他 S05 内置仍须
按逐调用点账本收口。用户已再次指定先验收 S05，再依次完成 S06、S07。
提前准备的 S06 freeze/thaw 改动保存在独立 stash 中，当前生产源码不包含它；
正式 benchmark/profile 对比与 PR #21 comment 在 S07 完整验证后执行。


## S05 属性查询与 Proxy Call continuation（实施中）

Proxy Get 的 target GetOwnProperty 查询已拆为共享阶段；GetOwnProperty 的
trap 选择、目标 descriptor、IsExtensible、ToPropertyDescriptor 和最终兼容性
检查保持原顺序。descriptor 转换按 enumerable/configurable/value/writable/
get/set 的顺序发出 Has/Get，保留 pinned Has 抛错当作存在的行为，以及 get/set
读取错误在当前操作 realm 替换为 TypeError 的规则。Has 与 IsExtensible 的
Proxy 依赖也由同一查询状态栈推进；普通存储 Has 准备只返回结果或未处理的 Proxy。

ToPrimitive 的 Proxy 属性读取已接入同一查询协议，最终值回复给原转换状态，
不向调用者提前压栈或推进 PC。Proxy [[Call]] 的 apply 读取与实际调用另有共享
阶段，保留逐层 Get 后再检查 cached Call bit、先分配 argv Array 再验证 trap
以及整个调用保留深度 guard 的规则。普通/尾调用、bound Proxy、Proxy getter、
转换方法和属性 trap 的 Proxy callable 都由 owned 子帧返回，原 native 和非普通
字节码调用仍保持可见的同步桥。

本批最终完整 owned 库 2117 项、默认库 1986 项；新旧常规 oracle 各 907 项
通过，各 1 项手动 65K 压力测试未运行。新旧 CLI profiling 各 5 项通过。
新增行为用例同时检查旧分派、整帧交接、同步调用桥三个计数为零；放弃
ToPropertyDescriptor 和 Proxy Call 两种阶段时，源对象、receiver、参数和
已读取值能释放，Runtime 无 owning cycle，Proxy 深度 guard 恢复为零。
源码布局为 529 个可达 Rust 文件；2 项属性契约测试包含各阶段隐式同步调用
反例。Stage3B 的 Proxy Call 顺序契约已随算法移到 read/resume 与旧入口消费器，
保留调用能力、逐层推进、转发实参和 trap 实参的原反例责任。

自引用 apply trap 的检查发现，显式 query continuation 必须独立检查深度，
不能再只依赖原生栈地址。查询嵌套现在使用既有 execution frame limit，计入
当前查询的父操作及已停放帧的查询父操作；原生预算不变。普通 Proxy Call、
Get trap 和 ToPrimitive 回调的循环 apply 在 32 层测试限额下产生可捕获的
stack overflow，未安装递归字节码帧，所有 continuation 与 guard 都能释放。

完整 714 项 boundary 在该修正前运行到 300 项后主动停止，不能算作最终
完整门禁。修正后重新执行源码扫描、属性契约 mutation 和迁移涉及的四项
Stage3B Proxy Call 顺序/实参反例；S05 最终验收仍须完整门禁。

这仍是 S05 实施证据。Set/其他 Proxy traps、Object/Reflect native 入口、
Array/iterator、String/RegExp/buffer 及其余同步内置尚未全部迁移；原始
Earley-Boyer 默认预算的新核心覆盖仍待 S05 最终验收。S06 的提前准备仍只
保存在独立 stash，只有 S05 完整验收后才恢复；S07 后再运行完整 benchmark/
profile 并报告到 PR #21。


## S05 普通 Set、Proxy Set 与 receiver Define 阶段（实施中）

普通 Set 的存储探测和 receiver 判断移动到 object/ordinary/set 的共享阶段；
旧 prepare 入口同步消费同一协议，owned 路径在 descriptor/define 回复后继续。
保留原存储内核、原型 setter 与 receiver accessor 的不同规则、Arguments
mapped slot 更新，以及 Proxy 转发失败时使用最终拒绝对象分类错误。
特殊对象原型的推进也返回继续步骤，不引入新的原生递归链。

Proxy Set 和 DefineOwnProperty 分别共享方法查询、trap、转发及目标 descriptor
invariant 阶段。写入 driver 接入 PutField/PutArrayEl；对象键转换持有已求值的
base 和 RHS，完成后一次性进入 Set。赋值保留先 RHS、后 ToPropertyKey、再检查
nullish base 的原顺序；setter 返回值被丢弃，完成后只推进原赋值 PC。严格模式
错误分类与旧 host_bridge 共用 finish_property_set；Proxy 的 NaN/±0 SameValue
及缺少 setter 的不可配置 descriptor 保持原规则。读/写/转换的 typed reply
连接被拆到 VM 子模块，算法继续归 object/value。

本批最终 owned 库 2122 项、默认库 1988 项通过，其中 owned driver 100 项。
新旧常规 oracle 各 907 项通过（各 1 项手动 65K 压力测试未运行），新旧 CLI
profiling 各 5 项通过，非 profiling stack-vm 构建无警告。新增写入案例验证
旧分派、整帧交接、同步调用桥均为零；两项旧 Set 交接预期加强为零交接。
放弃等待中的 receiver descriptor/define 状态可释放 target、receiver、handler
和 value，Runtime 无 owning cycle；错误类型的回复在应用前拒绝。
边界 scan-only、2 项属性契约/mutation、格式/diff、源码布局 534 文件通过；
本批未重跑完整 714 项 boundary，不能据此宣告 S05 完整验收。

Array length 与 TypedArray 的对象参数转换仍由明确的同步请求消费，特殊
Define 的转换也尚未全部改为 continuation；计数继续披露这些路径。其他 Proxy
traps、Object/Reflect native 入口及各同步内置仍按 S05 清单继续。S06 仍保持
延后，S07 及其后的完整 benchmark/profile 和 PR #21 comment 尚未执行。

## S05 Array length 与 TypedArray 写入转换（实施中）

Array length 的非 Number 参数由 object/array_length 保存原值与第一次 Uint32
结果，按 pinned 顺序发出两次 ToNumber 请求；value/conversion/number 复用
ToPrimitive 的 getter/方法阶段及原始错误 realm。第二次转换重新读取方法。
普通赋值及 Proxy receiver 的 Define 转发均已接入 owned 查询 driver，长度
写入与稀疏截断继续复用原内核，并在转换完成后重读 length/writable。

TypedArray element/write 阶段按元素类型完成 Number 或 BigInt 转换。Set 保留
无效/越界整数键仍转换、不同 receiver 的跳过/转交规则；Define 保留转换前
的 descriptor/view 检查。请求只持有 view、原值、元素类型和索引，不持有跨
回调 buffer access；完成后重新获取 view/buffer 凭证，保留 resize/detach 后
写失败仍接受的 pinned 行为。旧/native 入口同步消费相同领域阶段。

新增 Array length/TypedArray 定向测试覆盖方法替换、两次转换的 throw 身份、
Number hint、只读重检、稀疏截断回滚、BigInt、无效索引、不同 receiver、
resize、detach 和共享内存。请求放弃测试对原值、view、buffer 强制 GC，确认
保活/释放和 Runtime 无 owning cycle。transfer() 回调暴露的最后临时引用
Drop/Nip 回退改由 driver 冷路径完成：热预检不改变所有权，幸存栈先发布，
再使用原 Drop 释放；没有调整堆释放规则或 native 栈预算。String(1) 临时值
释放的既有测试加强为零交接，并保留独立未迁移操作的 handoff/catch 回归。

本批最终 owned 库 2127 项、默认库 1990 项通过；新旧常规 oracle 各 907 项
通过（各 1 项手动 65K 压力测试未运行），新旧 CLI profiling 各 5 项通过；
非 profiling stack-vm CLI 构建无警告。属性契约/mutation 2 项、boundary
scan-only、格式/diff、源码布局 538 文件通过。直接调用账本刷新为 127 个
表达式，仍不等于完整间接回调图；本批未运行完整 714 项 boundary。

super 属性、其他 Proxy traps、Object/Reflect native 入口和各同步内置继续
按 S05 范围迁移。S05 尚未验收；S06 stash 继续延后，S07 后的完整新旧 VM
benchmark/profile 与 PR #21 comment 尚未执行。


## S05 super 属性（实施中）

PushHomeObject 从当前已验证函数读取隐藏边，GetSuperValue、调用点读取和
PutSuperValue 进入 owned 属性/转换驱动。读取先转换键，再检查 base；写入
在 RHS 后先拒绝无效 base，再转换键。保留 pinned QuickJS 调用点的特殊
getter receiver：普通 super 读取使用实际 this，super[key]() 的 getter 使用
base，返回的方法仍使用原 this。复合赋值保留已转换的键，不重复转换。

冻结的 base 与实际 receiver 分别保活，避免键转换改动 HomeObject 原型后，
原 base 在 getter 期间过早回收。GC/放弃测试确认两者保持到等待结束，再唯一
释放；Runtime 登记与 owning cycle 检查通过。对象存储、错误消息和 strict
写入完成继续复用既有领域规则，没有新增语义算法。

本批 super 相关 15 项测试、owned 库 2129 项、owned 常规 oracle 907 项
（1 项手动 65K 压力测试未运行）、CLI profiling 5 项通过；非 profiling
stack-vm CLI 构建无警告。boundary scan-only、属性契约/mutation 2 项、
格式/diff 与源码布局 539 文件通过。默认配置本批未重跑；其共享领域代码
未修改，前一检查点的证据单独保留。完整 714 项 boundary 仍待 S05 验收。

S05 的其他 Proxy 操作、Object/Reflect native 入口及同步内置仍未全部迁移，
原始 Earley-Boyer 默认预算覆盖筛查尚待完成。S06/S07 和最终 benchmark/
profile、PR #21 comment 保持原顺序，尚未执行。


## S05 属性谓词与 Proxy 删除/阻止扩展阶段（实施中）

`in` 和 `delete` 由 owned 谓词驱动消费输入及键转换。`in` 先拒绝非对象 RHS，
`delete` 先执行 ToPropertyKey，再拒绝 nullish base；原语 String 的虚拟索引/
length 删除规则和严格删除完成由 object helper 与旧 host 共用。

Proxy boolean 阶段增加 deleteProperty 和 preventExtensions。删除 trap 返回
false 立即完成；true 则查询 target descriptor，拒绝不可配置属性，对仍存在
的可配置属性要求嵌套 IsExtensible 为 true。preventExtensions 成功则要求
嵌套 IsExtensible 为 false。Has 的 pinned raw extensible bit 规则保持独立。
转发与两次不变量查询持续保活 Symbol 键、target/handler；强制 GC 与放弃
测试确认键在等待期间存在，放弃后释放，Runtime 无 owning cycle。

完整 oracle 暴露了先前被删除指令旧回退掩盖的 Reference/with 检查：严格
赋值 RHS 删除对象属性后，TypedArray 原型上的无效数字键 Get 返回 undefined，
不能由此判为存在。读/写 reference 的普通重查改用共享 HasProperty，保留
原始 oracle 的 ReferenceError 预期，并增加零旧分派的定向回归。混合路径
测试保留未迁移 instanceof 反例；没有改变 oracle 预期或 native 栈预算。

本批最终 owned 库 2133 项、默认库 1991 项、新旧常规 oracle 各 907 项通过
（各 1 项手动 65K 压力测试未运行），新旧 CLI profiling 各 5 项通过；
非 profiling stack-vm CLI 构建无警告。boundary scan-only、属性契约/mutation
2 项、格式/diff、源码布局 540 文件通过。完整 714 项 boundary 本批未运行。

preventExtensions 的 owned 域查询测试不代表 Object/Reflect native 入口
已经迁移。prototype/ownKeys/construct 等剩余 Proxy 操作、动态环境剩余
路径、instanceof 与各同步内置仍按 S05 继续；S05 尚未验收，S06/S07 及最后的
完整 benchmark/profile、PR #21 comment 尚未执行。

## S05 本轮阶段验收

同步领域已按共享 Step/Resume 接入 owned driver，最后引用覆盖、literal/method 定义、
非法调用错误出口、子帧安装事务与放弃执行的释放顺序一并收口。新库 2197、旧库 2032、
新旧 oracle 各 911 加压力用例各 1、CLI profiling 各 5，以及 723 个完整边界反例通过。
默认预算原始 Earley-Boyer 独立及组合完整运行，旧分派/整帧交接均为零；
残余 3/10 个同步桥逐个由目标实参与调用栈确认是 S07 的 QjsConsoleLog。
初次组合诊断的 600 秒外部超时保持失败记录；放宽外部 watchdog 的完整归因运行
退出 0，未改变 VM 预算或脚本，也不作为性能测量。详见逐 commit 计划当前实施记录
和同步回调账本。本节取代上方各历史 checkpoint 的 S05 待办状态；S06/S07 仍未实施。

## S06 本轮阶段验收

五类挂起共用 freeze/thaw 与真实 owned frame；generator、async、async generator、
Async-from-Sync、Promise selector 和 jobs 保留各自状态机，通过 typed continuation 推进。
原始 argv 的 raw/Atom 边、恢复持根、放弃状态和最后引用在统一门禁中复核。
新库 2208、旧库 2035；新旧 oracle 各 911 加压力各 1；CLI profiling 各 5 通过。
非 profiling 构建、属性契约 4 项、完整边界反例 725 项、653 文件布局及格式/diff 通过。
既有小栈测试要求查询分派拆成有界 Rust 帧；77 个原分支保持原转换。
有限 1000 层委托零桥接用例通过，无限委托保留原溢出和恢复断言，预算未变。
单次验收内所有失败、修复及最终 1024 个源码/构建输入哈希保存在
`target/primitive-vm-s06-acceptance/`，最终判定见 `stage-verdict.json`。
本节取代历史 S06 待办状态；模块、宿主、API 与二进制入口仍属于 S07。
默认切换和旧路径删除仍属于 S10，最终 benchmark/profile 尚未执行。

## S07 本轮阶段验收

模块 link/evaluate/Import/body/private handlers 保留原 DFS/SCC、realm、TLA 和 FIFO
规则，以领域 continuation 和显式根操作进入 driver。Context call/construct/属性及
binary callable 复用相同入口，真实 host 边界使用 delimiter 和原生预算；没有边界的
内部嵌套根被拒绝。Test262 host、CLI、native/web adapters 的显式配置均已覆盖。

14 项统一门禁全部通过：owned/default 库 2212/2035、CLI 各 32、常规 oracle 各 911
加单独压力各 1、CLI profiling 各 5、Test262 runner 单元各 122。两配置完整 Test262
各 79982 pass / 80032 eligible / 102037 variants，结果字节与冻结基线严格一致。
两配置 Node/WASM 各 15 个 playground 示例、metadata、有限委托和溢出恢复通过。
完整边界反例 726 个全部拒绝；属性契约 4 项、非 profiling、662 文件布局、格式/diff 通过。
首次模板常量、Promise nullish 读取、receipt 来源哈希和 WASM 有限深度预期失败均保留；
修复与复验仍属同一次 S07 验收，原用例/预算/结果契约的保留情况见逐 commit 计划。
最终 1163 个源码/构建/fixture 哈希和全部原始证据在
`target/primitive-vm-s07-acceptance/`，判定见 `stage-verdict.json`。
本节取代历史 S07 待办状态；S08/S09 优化和 S10 默认切换、旧路径删除未实施。
完整 benchmark/profile 在本阶段唯一 commit 后执行，另在 PR #21 comment 汇报。
