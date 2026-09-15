# 栈 VM：分阶段 commit 计划

**本次执行指令（覆盖旧验收流程）：**分阶段提交全部实现，代码全部完成后额外逐项覆盖 review，遗漏补齐后仅对最终新核心执行一轮 benchmark/Profile；S0 与 QuickJS 复用本地已有数据。实现期间不运行 benchmark、cost profile 或 CPU Profile，不按阶段重复测量。测量结果仅保留在本地 `target/performance-retained/`，不提交 Git。阶段提交已获授权，无需再次等待提交指示。

**阶段指引的适用范围：**本文中的技术选型、覆盖顺序和实现策略描述对应阶段的实施方案与历史结果，不构成项目级优化禁令。后续可以采用运行期 shape/位置/属性值缓存、解析结果缓存、多态 IC、PGO 及其他优化；按语义、失效、所有权和性能证据评估方案。用户明确的执行约束继续有效。

状态：S01–S07 验收通过，S08 已收口；S09.1–S09.3 与 N1–N3 保留。**新 S10–S12 的计划内实施及额外覆盖 review 已完成，发现的遗漏已补齐；最终新核心唯一一轮 benchmark/Profile 为 403/403 有效，S0 复用旧三轮。性能目标未全部达成，S09 仍未完成。**58 fixed 中仍有 25 项高于 S0（直接前版为 31 项），67 compile 中 51 项为正差值，可比探针 12/20 更慢，RSS 3/3 更高。普通调用已快于 S0，Map/WeakMap 等仍回退且本轮部分恶化。G 与 S13 未实施。最新完整结果见[新 S10–S12 联合报告](performance/README.md)，[N1–N3 报告](performance/README.md)保留为直接前版证据。全部残余差值（fixed/探针/Score/RSS/编译五类清单）的实现层根因（十类，含 Profile 计数、probe 事件计数与 file:line 证据；其中编译 5 项判定为单轮测量伪影、RSS 2 项判定为映像差归 S13）与修复阶段见 [S14–S20 修复计划](primitive-vm-s14-s20-recovery-plan.md)，对应提交单元见文末[《S14–S20 修复阶段》](#s14s20-修复阶段2026-09-15-立项)。

**用户最新执行约束（2026-09-15）：后续仅构建、测试、benchmark 和 Profile 新执行核心（当前 `--features stack-vm`）。不再构建或运行旧 default 执行路径，也不重跑 S0/S07/S08 等历史候选；旧数据只读取已有记录。本文以前要求 default/stack-vm 双配置或新旧对跑的流程不再适用。历史已执行记录保留，但不能据此再次启动旧路径。Test262/语义 oracle 仍用于核对新核心，不把它们误称为另一个性能候选。**

**本次明确例外（三轮公平复测，2026-09-15，已完成）：**用户随后明确要求重新运行 S0 与当前实现各三轮完整 benchmark/Profile。本次允许构建和运行精确 S0（`c52d4dc`，与历史 S0 普通二进制源码 `1cc51bb5` 的差异仅为 Markdown）和当前新核心两个候选；不加入当前旧 default 路径或 S07/S08 等中间版本。两边同工具链、release 参数、相同编译探针源码和冻结输入，逐用例交错，正式耗时与 Profile 分开；58 fixed、67 compile、原始八项/combined、33 探针及内存控制各三轮，CPU/成本 Profile 覆盖全部 58 fixed。原始单项上限 180 秒、combined 600 秒，两边相同，失败与超时原样保留，不补样替换。新报告以两边本次三轮中位数、配对差值及原始范围为准；旧单轮报告保留为历史，不混入新统计。此例外不恢复以后例行运行旧 default 的流程。

S07 commit 后的完整 benchmark/profile 已完成：固定 58 项耗时均回退，为 PR19 的 1.17–5.28 倍；67 个新核心成本样本零旧分派、零桥接。[结果与源码归因](performance/README.md)已用于重写第 4 节：S08 先压低执行与状态推进成本并完成融合/PC 优化，S09 收口调用存储、编译和布局，修复剩余回退。该报告用于确定优化顺序；当前 S08 实施状态见下方开发记录。

目标见[架构计划](primitive-vm-plan.md)，目录与算法见[实施设计](primitive-vm-implementation-plan.md)，能力和结构验收见[迁移清单](primitive-vm-migration.md)。

用户于本轮要求：从当前 S05 剩余实现继续，按 S05 → S06 → S07 顺序，
每阶段仅一次正式验收与一次 commit，不创建中途 checkpoint commit。
开发中的定向构建/排错不作为阶段验收；S07 完成后再运行新旧 VM 完整
benchmark/profile，并在 PR #21 comment 汇报。此要求优先于下文开发临时提交规则。

S08 本轮开发中的候选、定向验证及剩余工作见[开发记录](performance/README.md)。
该记录保留历史过程。最新状态见 [S08 收口记录](performance/README.md)：已结束 S08 迭代，逐项保留回退、未合入候选与验证缺口；不再以完全修复函数调用回退阻塞 S08。

历史实现账本（记录 S08 以 source-23 收口、S09 三项机制实施前的状态；其中 S09.1–S09.3 的“尚待完成”已由 `c9a2607c` 实现，不再作为当前待办。当前剩余范围见下文 S09 待办索引）：

| 计划项 | 已落实 | 尚待完成 |
| --- | --- | --- |
| S08.1 诊断 | PC、状态推进、调用容量与机器码归因已接入 | 最终同源 67 项计数与完整成本报告 |
| S08.2 无回调直接完成 | 标量属性、Array/TypedArray、部分 iterator 和 native 完成路径 | 剩余状态热点与最终完整门禁 |
| S08.3 运行窗口 | 窗口认证和 Number/标量槽快路，已有独立 A/B | 最终矩阵与跨出口验证汇总 |
| S08.4 PC/融合 | 局部 resume PC、UpdateLocal/CompareBranch/AddStore，已有隔离测量 | 最终发布码、PC/融合成本收口 |
| S09.1 调用事实/环境 | 已有 executable 共享；确认全闭包环境仍逐调用复制 | 一次认证的私有调用视图、O(1) 环境取得与 owner 契约 |
| S09.2 普通调用协议 | 参数直接初始化与容量平台已实现；十轮确认普通调用仍慢 49.5% | 紧凑就地帧、短 Call/Return 路径和必要 argv 设计 |
| S09.3 增量预算 | 深度缩放及既有缓存候选确认祖先扫描问题 | FrameStore/Query 热检查 O(1)，失败/恢复/host 预算一致 |
| S09.4 最终性能/编译 | CompactVisits、坐标游标已实施，TOS/ASCII 有负面证据 | 最终矩阵、剩余回退及 S08 留存验证缺口 |

已有 source-23 完整 owned/default workspace 日志分别为 3473/3156 项通过；
独立 S08 为 3413/3137 项通过，各配置另有 ignored 压力用例单独通过。
最新同批固定 58 项四轮筛查 928/928 样本有效：当前组合 41 项中位数高于 S0，
空循环 / S0 为 0.5927、TypedArray 写入 0.6816、普通函数调用 1.5145。
这些是筛查结果，不是十轮正式验收；独立 S08-25 和组合 source-25 仍是未合入候选。
完整逐项结果、profile 版本边界、未完成验证及证据身份见 [收口报告](performance/README.md)。

## 当前实施记录

- **S01 阶段验收通过。** compiler 共享模型归 model/{ir,bindings,scope}；parser 的 context/builder 区分临时解析状态与完成产物，消费式 finish 移动原有存储。语法 helper 按领域归属，生产依赖显式导入。relocation/flow/optimize 复用原有绑定解析、栈验证与窄优化，保留 QuickJS 错误顺序和 source projection。
- S01 profiling 诊断覆盖 parse/resolution/lowering inclusive wall time、最终草稿指令与 inline bytes、部分 owned-Vec 容量，以及 legacy 分派/PC 发布/操作数深度。容量采样不是分配峰值；未实现的 frame/slot 成本不报为零，正式构建关闭诊断。
- **S01 最终证据（对应 S01 源码）。** 302 项 compiler 测试、2 项诊断测试、5 项 CLI profiling 测试通过；默认 CLI cargo check 通过。完整 QuickJS oracle 907 项及另行执行的 65K 实参压力用例通过，合计 908 项；701 个完整 binary-object 反例全部拒绝，源码布局和 diff 检查通过。
- **S02 阶段验收通过。** 发布输入归 code/function/publication，源代码请求和编译错误边界归 api/compile；VerifiedFunction 保留原草稿所有权。纯验证及测试归 code/verify；Atom 链接、展平、私有绑定发布和 heap 事务留在发布侧，生产验证不导入 compiler。
- S02 验证主流程按 roles、parameters、bindings、private_elements、closures、flow、eval、operands、children 组织。命名工作项和命名分析结果复用原有数组、声明索引、闭包位置与迭代队列；检查顺序不变。76 项既有测试按 modules/private/parameters/eval/bindings 分组，未增加镜像测试或改变预期。
- S02 上轮证据：436 项 code 测试、253 项 Runtime 测试通过；eval/children 拆分后完整 oracle 908 项通过。38 项定向发布反例拒绝。迁移前启动的完整反例验证因源码路径改变主动终止，不计为 S02 完整验收。
- S02 本轮 FrameLayout：只读视图从同一个 rooted executable 借用参数、局部、闭包定义和 metadata，构帧/恢复形状检查及 operand 容量已接入。RootedVmActivation 删除重复的 bytecode/code/metadata 字段，运行时从 host 的 executable 取用；动态恢复检查保留。14 项 published execution 测试与边界 scan-only 通过，源码布局为 484 个文件；完整 oracle 907 项及另行执行的 65K 实参压力用例通过，合计 908 项；40 项定向发布反例全部拒绝。
- S02 指令契约已接入：code/instruction.rs 对 198 个变体穷尽描述栈数量与类型化状态、控制流、操作数角色以及保守的 JS 异常/回调/分配效果。编译器块边界和现有窄优化、普通栈与参数验证、module initializer flow、静态名称链接、eval 环境选择和分支目标消费同一描述。动态 region/private/resume 检查保留。
- profiling 可显式捕获最终码反汇编，逐 PC 输出同一契约；默认关闭，只保留文本。诊断测试 3 项、CLI profiling 测试 5 项、最终 compiler 302 项、code 436 项和 Runtime 253 项通过；完整 QuickJS oracle 907 项及另行执行的 65K 实参压力用例通过，合计 908 项。源码布局为 485 个文件。上轮定向反例运行因描述结构更新主动终止，不计通过；本轮完整边界结果见下述最终验收。
- **S02 最终验收：**修正后的完整 boundary 套件退出码 0，709 个反例全部拒绝；此前四个简化 fixture 配置错误已改为完整源码 fixture，并先通过四项定向验证。逐项复查确认原草稿所有权、验证顺序、共享 executable 布局、动态检查及合法范围保留。
- S03 当前改动：纯 Number 实现与原有 13 项测试按 operations/integer/format/float16 归属，公共入口不变；既有帧绑定读写、捕获、关闭及闭包视图规则归 vm/bindings，现有 host 与 heap cell 验证直接消费，挂起编码和恢复验证保留。Number 13 项和 host_bridge 12 项测试通过，完整 oracle 907 项与单独执行的 65K 实参压力用例通过，合计 908 项；源码布局为 490 个文件。FrameStore/SlotStore/RunningExecution 与新主循环的初始实现见下述迁移配置；S03 尚未验收。
- S03 `stack-vm` 非默认配置已接入 ordinary bytecode 调用入口：FrameStore 持有单一 executable 和冷状态，SlotStore 分开原始实参、可写形参、局部与操作数窗口；运行登记只保留身份。主 match 完成字面量、普通槽操作、Number 算术与静态分支，未覆盖的操作在消费输入前通过显式旧路径桥交接。正常返回先安装 pending 结果，再清理窗口。
- S03 初步证据：3 项执行测试通过，其中独立测量的循环和 Number 边界为零旧 VM 分派、零交接；转换交接保持原操作数和一次回调。4 项窗口测试、2 项登记生命周期测试、迁移配置下 5 项 CLI 诊断测试通过。共享 Number 表示/更新已接入新旧执行路径和 PrimitiveValue 构造；新配置完整 oracle 907 项及单独执行的 65K 实参压力用例通过，合计 908 项；默认配置 37 项 Number 相关测试与迁移配置 3 项诊断测试通过。两个新增普通调用出口反例均拒绝。
- S03 引用事务已接入普通槽覆盖、Drop 和 Nip：修改计数前检查 runtime 域、可变借用、deferred references、zero queue 状态/容量及共享 primitive 存储。需要回收的操作保留原值交接；Symbol copy 使用可失败 retain；交接目标容器在移动任何源 owner 前完成可失败预留。7 项引用生命周期/溢出测试、4 项执行测试、4 项窗口测试、2 项登记测试通过；对象参数覆盖的独立调用为零旧 VM 分派、零交接，返回后没有额外参数 root。本次源代码下新配置完整 oracle 907 项与单独执行的 65K 实参用例通过，合计 908 项；源码布局 496 个文件。
- S03 普通值栈已补齐 Insert2/3/4、Dup1/Dup3、Perm3/4/5、Rot4Left：重排在预留窗口内移动 owner，单次插入先检查容量再 retain；Dup3 的 retain 错误是终止错误，已提交前缀留在逻辑窗口中由驱动器清理，不在热循环回滚释放。String/BigInt 常量直接共享已有 primitive 存储；动态加法仍一次性交接。7 项槽存储测试、6 项执行测试通过，含失败前缀清理、返回后字面量存活与 String/BigInt 慢路；更新后的新配置常规 oracle 907 项与单独执行的 65K 实参用例通过，合计 908 项。
- S03 初步存储成本已接到真实操作并输出到同一 CostSnapshot/CLI：槽与帧 Vec 增长、逐存储区峰值、初始化、逻辑转移、帧清理、值复制和受限 Object/Symbol 热引用计数；未覆盖的完整调用分配、primitive Rc 与回收级联明确排除。容量复用测试区分累计初始化与峰值，并验证空 arena 的析构不重复计数。迁移配置 VM 组 126 项通过、2 项既有小栈失败；新旧两种 CLI 配置各 5 项、引用事务 7 项、诊断作用域 2 项通过，真实 CLI JSON 解析通过。此前启动的 711 个边界反例全部拒绝；计数改动后的源码边界扫描与 496 文件布局检查通过。
- S03 资源审查已记录在迁移账本：原始实参快照先可失败预留，帧限额与身份失败的 2 项测试通过。最终同源检查中新旧配置各 908 项 oracle、Number 13 项、引用事务 7 项通过；VM 组 128 项通过、2 项既有小栈失败，包含的 S03 帧/槽/执行/登记 18 项全通过。最终同源边界反例 711 项全部拒绝，退出码 0；S03 阶段验收通过。现有桥仍递归等待未迁移的 JS 调用；S04–S07 必须按计划替换，不能把配置启用或混合路径 oracle 视为完整新核心覆盖。
- S04 绑定到普通字节码的调用已归一化后进入同一 driver；5 项 driver 测试通过。`this` 读取与原语装箱已接入，缓存对象身份在交接后保留；native/Proxy/挂起目标保留原始调用交接，constructor 与统一恢复尚未完成。
- S04 一元 `+` 的 ToPrimitive 已通过 operation 身份把 getter/valueOf 回复路由到父帧，嵌套转换走同一显式执行。特殊属性与非普通字节码调用只执行当前步骤的临时交接；其他转换、constructor、完整展开和绑定仍未完成。
- **S03 阶段验收通过，S04 实施中，S05–S10 尚未开始。** 非默认原语栈核心、帧/槽所有权与初步成本已达到本阶段要求；完整调用/回调、挂起和入口迁移尚未完成。正式十个提交在 PR 整理时归并。
- S04 已抽出共享构帧准备，并接入普通字节码 Call/Method/TailCall 的显式 driver：子帧共用 FrameStore/SlotStore，结果先持根再清帧，子帧 guard 在恢复父帧前结束。命名自递归 256 层在 2 MiB 栈通过且无 legacy 分派；执行核心 9 项、限额/外域参数 2 项及常规 oracle 907 项通过。公开无限递归得到可捕获的 `InternalError:stack overflow`。原有 native_stack 仍为 4 通过、2 失败（全局函数链与 TypedArray 路径仍经桥）；构造调用、转换状态、统一展开和完整绑定仍待实现，S04 尚未验收。
- S04 普通 lexical 初始化、无回收槽重置、checked 读写及非捕获 CloseLocal 已接入；共享 TDZ 诊断保留变量名可见性和错误 realm。循环块复用及下一轮 TDZ 为零旧分派、零交接。driver 18 项、run 9 项、新旧配置各 907 项常规 oracle、非 profiling 构建和边界扫描通过。捕获生命周期、完整 arguments/eval 与统一展开仍待完成，S04 未验收。
- S04 闭包单元普通/checked 读写已接入 driver，共享 TDZ、cell 只读及名称诊断规则。var/let 活单元、const 读取和逃逸 TDZ 测试为零旧分派；driver 19 项、run 9 项、新旧配置各 907 项常规 oracle 与构建/边界检查通过。闭包创建、每迭代 cell、捕获关闭和统一展开仍待完成。
- S04 catch/finally 首段控制已接入：帧持有 catch 区域，Throw/子帧抛错在弹帧前恢复 handler，Gosub/Ret 保持原返回 PC 协议，交接保留 regions。driver 20 项、run 9 项、常规 oracle 907 项及构建/边界检查通过。IteratorClose、捕获关闭和统一展开余项仍未完成，S04 未验收。
- S04 FClosure、captured CloseLocal 和 SetName 已接入，共享新 cell canonical metadata 与既有 cell view 校验。闭包跨父帧/块退出、参数捕获及循环独立 cell 用例为零旧分派。driver 21 项、run 9 项、新旧配置各 907 项常规 oracle、构建和边界检查通过；captured 局部读写/重置与统一展开余项仍待迁移。
- S04 captured 局部/参数读写、普通 lexical 初始化与 TDZ 重置已接入；重置与旧 host 共享初次 cell/异常复用规则。父槽更新及异常跨作用域 cell 身份用例为零旧分派。driver 22 项、run 9 项、新旧配置各 907 项常规 oracle 和构建/边界检查通过；arguments、eval/with/private、IteratorClose 等余项继续待办。
- S04 mapped/unmapped arguments 与 rest 创建已接入独立步骤，保留实际 arity、参数 alias 和额外实参独立 cell。修正了分配处理展开在 driver 中导致的 Proxy 有限栈回归，未更改预算或预期。driver 23 项、run 9 项、常规 oracle 907 项及单独 65K 压力用例、构建和边界检查通过；S04 其余语义仍待完成。
- S04 标识符默认参数的严格比较缺省判断已接通，Number 保持热路径，其余类型复用 strict_equal 的独立步骤。初始化顺序、TDZ、null/false 及初始化器 cell/函数体副本分离测试为零旧分派。driver 24 项、run 9 项、常规 oracle 907 项及构建/边界检查通过；解构参数和 S04 其余协议仍待迁移。
- S04 readonly/redeclaration 抛错接入当前帧展开，保留静态 Atom 消息及 RHS 优先级；闭包创建临时状态移出常驻 driver，Proxy 有限栈回归通过且预算不变。driver 25 项、run 9 项、常规 oracle 907 项及构建/边界检查通过；eval/with/private、IteratorClose 等仍待完成。
- S04 private 字段/方法/访问器初始化已接入独立步骤，新旧路径共享身份、种类、命名与 HomeObject 规则。private 相关 70 项、driver 26 项、run 9 项、新旧配置各 907 项常规 oracle 和构建/边界检查通过；完整 class/private 访问与 S04 其他协议仍未迁移完。
- S04 private 字段 get/get2/put/define/in 接入独立 step，共享私有名称解析并保留接收者和错误顺序。真实字段读写、成员检查及字段函数 this 用例为零旧分派；driver 27 项、private 72 项、run 9 项、新旧配置各 907 项常规 oracle 和构建/边界检查通过。private 方法/访问器及 S04 其他协议仍待完成。
- S04 私有方法读取、品牌检查、in 和只读写入已接入，身份解析与检查顺序由新旧路径共享。driver 28 项、private 73 项、run 9 项、新旧配置各 907 项常规 oracle 及构建/边界检查通过；私有访问器 JS 调用与 S04 其他协议仍待完成。
- S04 普通私有 getter/setter 接入显式子帧，ReturnValue 区分压入/丢弃结果，setter 抛错仍展开。访问器 in/只读方向也接入；getter 单次调用、接收者和 setter 返回栈形状用例为零旧分派。driver 30 项、private 75 项、run 9 项、常规 oracle 907 项及构建/边界检查通过；S04 仍未验收。
- S04 实例字段初始化器接入显式子帧，共享原有身份/realm/原型校验和品牌安装，品牌安装后不交接或重放。基类、派生类和初始化抛错用例为零旧分派；driver 31 项、class 77 项、run 9 项、新旧配置各 907 项常规 oracle 及构建/边界检查通过。静态初始化和 S04 其他协议仍待完成。
- S04 静态元素初始化器/static block 共用显式子帧入口，原有单次状态提交与 HomeObject/品牌安装由新旧路径共享。发布入口检查点验证三层帧、抛错和重复启动拒绝；driver 32 项、class 77 项、run 9 项、新旧配置各 907 项常规 oracle 及构建/边界检查通过。完整 class 创建、eval/with、IteratorClose 等仍待完成，S04 未验收。
- S04 接通实例初始化器安装和非对象父类的 DefineClass 分支，复用原有无 JS 内核。完整函数的类创建/初始化抛错、extends null 和非法 primitive 父类用例零交接；driver 33 项、class 79 项、run 9 项、新核心常规 oracle 907 项及构建/边界检查通过。对象父类 getter 恢复及其他 S04 余项仍未完成。
- S04 对象父类 prototype 的普通读取/getter 接入 PendingClass 和唯一 operation 回复，共享原类发布内核，getter 之后不重读 parent。单次调用、抛错、非法 prototype 用例零交接；driver 34 项、class 80 项、run 9 项、新旧配置各 907 项常规 oracle 及构建/边界检查通过。exotic/非普通 getter、eval/with、IteratorClose 等余项仍待完成。
- S04 固定名称方法/getter/setter 安装复用原命名、HomeObject 与 descriptor 内核，完整类方法/继承/访问器合并用例零交接。driver 35 项、class 81 项、run 9 项、新核心常规 oracle 907 项及构建/边界检查通过；计算名称、字段、eval/with、IteratorClose 等余项仍待完成。
- S04 固定名称公共字段定义接入共享 DefineProperty 冷步骤，复用 CreateDataProperty 规则并绕过继承 setter。字段顺序、函数 this、抛错等用例零交接；driver 36 项、class 81 项、run 9 项、新核心常规 oracle 907 项及构建/边界检查通过。计算名称和其他 S04 余项仍待完成。
- S04 computed 字段/方法定义共享 canonical key 解析，ToPropKey 接入 String-hint 转换恢复。单次转换、抛错优先级、数字和 Symbol 用例通过；driver 37 项、class 82 项、run 9 项、新旧配置各 907 项常规 oracle 及构建/边界检查通过。SetNameComputed、eval/with、IteratorClose 等 S04 余项仍待完成。
- S04 SetNameComputed 与静态命名进入共享冷步骤，保留 Symbol 描述命名和已有名称规则。driver 38 项、class 82 项、run 9 项、新旧配置各 907 项常规 oracle 及构建/边界检查通过；eval/with、IteratorClose、host 重入等 S04 余项仍待完成。
- S04 接通 with 的 ToObject/对象绑定初始化及 eval 变量环境创建，共享旧绑定规则。with 入口与 nullish 错误零交接，环境创建有真实 PC 检查点；driver 40 项、with 过滤测试 129 项、run 9 项、新旧配置各 907 项常规 oracle 及构建/边界检查通过。动态查找、eval 执行、IteratorClose 和 host 重入等仍待完成。
- S04 共享 eval/with 隐藏对象身份解析，接通普通动态数据查找、unscopables 和引用读取，保留 with 方法 this。driver 41 项、with 过滤测试 130 项、run 9 项、新旧配置各 907 项常规 oracle 及构建/边界检查通过；getter/Proxy、写入、eval 执行和其他 S04 余项仍待完成。
- S04 动态读取/引用读取的普通 getter 接入显式子帧，验证单次调用、抛错、捕获环境和返回函数 this。driver 42 项、with 过滤测试 131 项、run 9 项、新核心常规 oracle 907 项及构建/边界检查通过；unscopables getter、写入、eval 和其他 S04 余项仍待完成。
- S04 HasBinding 的 unscopables/排除项 getter 使用独立两阶段恢复，保留检查顺序与唯一回复，并接通普通对象分配。driver 43 项、with 过滤测试 131 项、run 9 项、新核心常规 oracle 907 项及构建/边界检查通过；动态写入、eval、IteratorClose 等 S04 余项仍待完成。
- S04 普通动态/eval 变量删除接入环境冷步骤，保留自有属性、不可配置结果和 unscopables 规则。driver 44 项、with 过滤测试 131 项、run 9 项、新核心常规 oracle 907 项及构建/边界检查通过；写入、eval 执行、IteratorClose 等余项仍待完成。
- S04 Has/Get/DefineEvalVariable 接入环境步骤，保留 eval 隐藏对象身份、普通 getter 子帧和完整数据描述符定义规则。已发布 eval 体的声明、继承 getter、抛错、不可配置拒绝及忽略 unscopables 用例零交接；driver 45 项、eval 过滤测试 137 项、run 9 项、新核心常规 oracle 907 项及构建/边界检查通过。完整 eval 调用入口、动态写入、IteratorClose 等 S04 余项仍待完成。
- S04 PutDynamicBinding/PutEvalVariable/PutRefValue 的普通目标进入环境写入步骤，setter 使用显式子帧并丢弃返回值，引用写入保留 VarRef cell。driver 47 项、eval 137 项、with 131 项、run 9 项、新核心常规 oracle 907 项及构建/边界检查通过；完整 eval 入口、IteratorClose、普通 PutField 和特殊对象协议等余项仍待完成。
- S04 引用读取补齐普通自有 VarRef 与未解析引用错误，复用原属性读取内核并保留环境/值栈形状。driver 48 项、eval 137 项、run 9 项、新核心常规 oracle 907 项及构建/边界检查通过；GlobalReference、完整 eval 入口、IteratorClose 等余项仍待完成。
- S04 GlobalReference 接入共享身份/当前 lexical 解析与无 getter 的全局存在检查，保留发布后新增 lexical 优先及 RHS 前 TDZ/const 错误。driver 50 项、eval 138 项、run 9 项、新旧配置各 907 项常规 oracle 及构建/边界检查通过；专用全局 payload 写入、完整 eval 入口、IteratorClose 等余项仍待完成。
- S04 全局对象引用读写及字段读取复用专用描述符/存储内核，普通访问器走显式子帧；上轮全局属性赋值缺口已补齐。driver 51 项、eval 138 项、run 9 项、新核心常规 oracle 907 项及构建/边界检查通过；auto-init/特殊原型、完整 eval 入口、IteratorClose 等余项仍待完成。
- S04 direct eval 准备与执行分离：准备结果持有 Completion 或已实例化 closure/this，保留先编译后捕获的顺序。eval 138 项、driver 51 项、新旧配置各 907 项常规 oracle 及构建/边界检查通过；owned frame 的 eval 校验、捕获和子帧入口尚待接通，S04 未验收。
- S04 eval 帧绑定校验、按作用域顺序捕获和准备/捕获结果类型移入共享 eval_bindings，旧 host 适配自身槽位。eval 138 项、driver 51 项、新旧配置各 907 项常规 oracle 及构建/边界检查通过；owned Eval 指令与子帧入口尚待接通，S04 未验收。
- S04 普通 Eval 指令接入准备/捕获与显式子帧，父帧保留全部实参至返回并统一清理；GetVar/GetVarUndef 补齐其前置读取。driver 53 项、eval 140 项、run 9 项、新核心常规 oracle 907 项及构建/边界检查通过；ApplyEval、IteratorClose 等余项仍待完成，S04 未验收。
- S04 为 ApplyEval 整理无 getter 的稠密 Array 实参预检，复用既有快存储并保留 65534 上限；通用实参入口已使用该步骤。Reflect 8 项、新旧配置各 907 项常规 oracle 及构建/边界检查通过；ApplyEval 快照保活与 owned 调用入口尚待接通。
- S04 稠密 Array 的 ApplyEval 接入 owned 原始 eval/普通及 bound callee，父帧快照保活并在返回后释放。driver 54 项、eval 141 项、run 9 项、新核心常规 oracle 907 项及构建/边界检查通过；通用实参读取、展开构造、IteratorClose 等余项仍待完成。
- S04 ArrayFrom 接入 owned 冷步骤，复用原数组创建内核并保留元素顺序与 callee realm，跨 realm/继承 setter 用例零交接。driver 55 项、eval 141 项、run 9 项、新核心常规 oracle 907 项及构建/边界检查通过；Append/迭代读取和 IteratorClose 等余项仍待完成。
- S04 DefineArrayEl 的普通对象/Array 数字索引写入接入现有 owned 冷步骤入口，复用完整自有数据描述符定义并保留数组/索引。真实发布指令检查点验证继承 setter 不执行、孔位/长度与后续返回零交接；不把该检查点计作完整展开覆盖。driver 56 项、eval 141 项、run 9 项、新核心常规 oracle 907 项及构建/边界检查通过；Append/IteratorClose 等仍待完成。
- S04 Append 接入显式迭代状态与普通/bound 回调子帧，保留两次 Symbol.iterator 读取、cached next、快数组快照、u32 索引和异常关闭原异常优先级；父帧保活输入及恢复代次。完整数组/String 展开与 eval(...数组)、回调顺序和写入失败关闭通过；2048 元素在 2 帧上限下零旧分派完成。driver 61 项、eval 142 项、run 9 项、新旧配置各 907 项常规 oracle 及构建/边界检查通过。原生/特殊读取仍有单步骤同步回退；for-of 与通用 IteratorClose 等 S04 余项尚未完成。
- S04 for-of 与同步 IteratorClose 接入 owned 迭代记录和异常区域；Append 状态扩为共享 iterator_driver，按操作区分一次/两次 iterator 查询及 next 失败的关闭策略。普通/保留返回值的关闭、嵌套异常关闭、解构和每迭代捕获用例零旧分派；driver 65 项、eval 142 项、run 9 项、新核心常规 oracle 907 项及构建/边界检查通过。Apply/构造展开入口、host delimiter 与完整阶段验收等仍待完成，尚未进入 S05。
- S04 Apply/ApplySuper 接入普通/bound 调用及既有构造 continuation，稠密实参快照保留 receiver、new.target 和错误顺序，包括 nullish 列表按普通调用执行的原行为。完整调用/构造/super 展开、递归展开与 prototype getter 跨越实参载体修改通过；driver 69 项、eval 142 项、run 9 项、新核心常规 oracle 907 项及构建/边界检查通过。通用实参读取仍在无副作用预检后交接；全局写绑定、host delimiter 与 S04 完整验收等仍待完成。
- S04 工作区检查点（2026-09-13）：全局 PutVar/PutVarInit/DeleteVar 接入共享绑定规则；未迁移 callee 使用单次同步调用边界，Array 命名属性缺失可沿普通原型读取。帧退出和冷绑定操作已从常驻 driver 拆出。当前 driver 76 项、run 9 项、eval 142 项及格式/边界扫描/源码布局（521 文件）通过。当前 native_stack 为 3 通过、3 失败：TypedArray 字符串转换、有限嵌套排序、递归调用/构造的旧溢出预期；未修改预算、原测试或 skip。最新冷操作拆分后尚未重跑完整 oracle。此提交仅保存实施中的工作区，S04 未验收，S05–S10 尚未开始；正式十个提交仍需后续整理。
- S04 小栈修正：根帧交接通过 RunningExit 返回到原 bytecode 入口后才执行，先退出 owned 准备/driver 的全部 Rust 帧；普通 Call/CallMethod 与旧扩展调用分派分开，准备步骤提前返回。原有 32 层调用及 TypedArray 有限字符串转换、排序用例现已通过，原生栈预算未变。溢出用例改用真正无限递归，保持原错误/恢复断言；owned 有限递归加强为 1000 层、1001 个显式帧且零旧分派。新旧 native_stack 各 6 项、新旧常规 oracle 各 907 项、owned VM 209 项和 modules 134 项通过。完整边界反例 714 项全部拒绝；S04 的 host delimiter、调用成本和阶段审计继续待办。
- **历史小栈基线已修正。** 默认和 profiling debug 构建的 32 层 bytecode 调用、TypedArray 字符串转换两项失败，曾在隔离导出的改动前提交 5fe11ea 同样复现（native_stack 测试 4 passed、2 failed）。上述 S04 修正后，原有限用例通过，预算保持不变；这段保留为历史基线记录。


- S04 host delimiter 骨架已接到真实同步 module host callback，登记只保存 runtime/执行/父帧身份；返回检查父登记恢复，错误和 Rust unwind 自动解除登记。实际 loader 在 owned 父帧存活时重入 JS、更新捕获绑定并执行 GC，正常返回、loader 拒绝及 Rust panic 均验证恢复。普通函数读取和写入模块 import view 的独立测量保持 readonly 规则，零旧分派、零交接；模块实际入口迁移仍属 S07。
- S04 调用准备诊断已接入同一 CostSnapshot/CLI：参数和局部 Vec 容量分配、初始化、实参/heap-root 复制、owned 冷帧及捕获复用表分配；传入实参 buffer 只统计观察容量，不伪称其分配次数。完整调用分配和全部 retain/release 仍属 S09；统计范围见 profiling.md。缺参/多参/对象实参/主体抛错的诊断测试与两种 CLI 配置均通过。
- S04 完整库门禁发现并修正了一元 `+` 恢复使用通用 ToNumber 的偏差：新旧执行路径现在共享原语 OP_plus 处理，保持 BigInt 特定消息和 Float 原表示。二进制既有断言未改，回调 Float/-0/NaN 位模式与 BigInt 抛错测试零旧分派通过。修正后新配置库测试 2092 项、默认配置 1981 项通过；最终阶段结果见下条。

- **S04 阶段验收通过（2026-09-13）。** 一元 `+` 修正后的同源门禁：owned 库 2092 项、默认库 1981 项；两种配置的常规 oracle 各 907 项加单独 65K 压力 1 项，合计各 908 项；CLI profiling 各 5 项通过。完整 boundary 714 个反例全部拒绝，退出码 0；非 profiling stack-vm 构建、格式、diff 和源码布局（521 文件）通过。普通调用/构造、完整绑定、同步展开、一条完整转换回调、host delimiter 骨架和初步调用成本按本阶段范围验收；逐项证据与后续边界见迁移账本。S05–S10 尚未开始，整体目标未完成；正式十个提交仍在最终 PR 整理时归并。

- S05 首批属性读取迁移：object 共享准备阶段覆盖 non-Proxy 描述符/原型查找，保留 Array 孔位、TypedArray 整数索引终止、Arguments/String/namespace/autoinit 的原存储规则。GetField/GetField2 与原语动态键的 GetArrayEl/2/3 由 property_driver 消费已选 getter，原对象键转换/Proxy/Set 等继续待办。相关 getter、receiver、旧键保留、nullish 顺序和请求放弃测试通过。
- S05 临时同步调用请求先拥有 callee/实参并返回到外层 driver，常驻分派帧退出后才调用未迁移 Runtime 入口；父执行和 operation 代次保留，回收或错误不重放调用。此桥仍可能同步等待内部 JS，不能计作 S05 callback continuation 完成。新增 owned_sync_call_bridges 单独揭示这些调用；原 native 小栈用例保持预算并通过。完整 owned 库 2097 项、默认库 1981 项、新旧常规 oracle 各 907 项（各 1 项手动压力用例本批未重跑）、新旧 CLI profiling 各 5 项、非 profiling 构建、边界扫描/定向 mutation 与源码布局 522 文件通过。第一轮 [121 个直接调用点账本](primitive-vm-sync-callbacks.md) 已建立，间接回调图仍须补全；S05 未验收。

- S05 后续属性迁移：对象键 string-hint 转换、原语 base、bound 字节码 getter 已接入。Proxy Get 的 handler 读取、trap、转发与普通 target invariant 由 object 阶段状态和 owned driver 共用；嵌套 Proxy handler 与字节码回调不再通过原 Get 同步桥。新库 2108 项、旧库 1983 项及新旧 Proxy/Reflect oracle 各 16 项通过；native/Proxy callable、Proxy descriptor target、Set 和其他同步内置仍待迁移。S05 尚未验收；按用户最新指示先完成 S05，再 S06、S07，最后提交完整新旧 VM benchmark/profile 对比到 PR #21 comment。

- S05 查询与调用继续推进：Proxy target GetOwnProperty、descriptor Has/Get、嵌套 Has/IsExtensible 和 ToPrimitive 的 Proxy Get 接入共享 continuation；Proxy apply 读取、可调用标记顺序、转发和 trap 也已接入普通调用/getter/转换/查询路径。最终 owned 库 2117 项、默认库 1986 项、新旧常规 oracle 各 907 项（各 1 项手动压力未运行）、新旧 CLI profiling 各 5 项通过。Set、其他 traps、Object/Reflect native 入口及其余同步内置仍待收口；S05 仍未验收，未提前进入 S06/S07。

- S05 写入继续推进：普通 Set、Proxy Set、receiver GetOwnProperty/DefineProperty 与 strict/sloppy 完成改为共享阶段，PutField/PutArrayEl 的 setter 和对象键转换接入 owned driver。新库 2122 项、旧库 1988 项、新旧常规 oracle 各 907 项（各 1 项手动压力未运行）、CLI profiling 各 5 项及扫描/契约/布局通过。Array length/TypedArray 对象参数转换、其他 traps 与 native 内置仍待继续；S05 未验收，S06/S07 仍按顺序延后。

## 1. 提交顺序与关口

| 提交 | 完整交付单元 |
| --- | --- |
| S01 | 编译器结构、线性栈 IR 流程与基础诊断 |
| S02 | 指令契约、验证流程、发布与 executable 布局 |
| S03 | 帧/槽所有权、纯 Number 算法与栈执行核心 |
| S04 | 显式调用、转换恢复、异常与完整绑定 |
| S05 | 属性及同步内置 JS 回调迁移 |
| S06 | generator、async/Promise 与 async generator |
| S07 | 模块、宿主、API 与二进制入口集成 |
| S08 | 局部指令融合与 PC 观察边界 |
| S09 | 调用存储优化与同架构布局实验 |
| S10 | 完整验收、默认切换、旧路径清理与结果文档 |

- **A：S01–S03。** 编译/发布边界、帧/槽所有权和原语栈核心可运行、可测量。
- **B：S04–S07。** 普通调用、内部回调、挂起与全部既有入口完成新核心覆盖。
- **C：S08–S10。** 在完整语义上优化，完成 #16 验收并切换、删除旧路径。

每个提交中的实现、对应测试、源码契约及相关架构说明和结构检查一起完成。语义检查随提交执行，不推迟到 S10；诊断样本不充当正式性能成绩。临时桥明确记录覆盖；经过旧 VM 的样本不能宣称新核心已覆盖，禁止按测试或 benchmark 名选择路径。

- S05 特殊写入转换继续推进：Array length 两次 ToNumber、后置 writable 检查，TypedArray 整数索引 Set/Define 的 Number/BigInt 转换与 buffer 凭证重取已接入 owned 阶段；Drop/Nip 最后临时引用交给 driver 冷路径。新库 2127 项、旧库 1990 项、新旧常规 oracle 各 907 项（各 1 项手动压力未运行）、CLI profiling 各 5 项、扫描/契约/布局通过。super 属性、其他 traps 与同步内置继续待办；S05 未验收，S06/S07 仍按顺序延后。

- S05 super 属性继续推进：HomeObject、冻结 base、读取/调用/写入及对象键转换接入 owned 驱动，保留 pinned getter receiver 差异与读写顺序，并验证 base 独立 GC 保活。super 15 项、owned 库 2129 项、owned 常规 oracle 907 项（1 项手动压力未运行）、CLI profiling 5 项及构建/扫描/契约/布局通过。本批仅修改 stack-vm；S05 其余 Proxy/native 同步回调仍待完成，未进入 S06/S07。

- S05 属性谓词继续推进：in/delete 接入 owned 转换/查询，Proxy deleteProperty/preventExtensions 共享阶段并保留不同的 target 不变量；Symbol 键跨等待保活。修正删除后 reference/with 用 Get(undefined) 误判 TypedArray 属性存在的问题，原 oracle 预期不变。新库 2133 项、旧库 1991 项、新旧常规 oracle 各 907 项（各 1 项手动压力未运行）、CLI profiling 各 5 项及构建/扫描/契约/布局通过。剩余 Proxy/native 同步路径继续待办，S05 未验收，未进入 S06/S07。

- S05 Proxy 原型操作继续推进：getPrototypeOf/setPrototypeOf 共享有序阶段，owned 查询覆盖嵌套 target 的可扩展性与原型身份检查，保持各阶段抛出值身份，并验证等待/放弃的 GC 所有权。新库 2136 项、旧库 1992 项、新旧常规 oracle 各 907 项（各 1 项手动压力未运行）、CLI profiling 各 5 项及构建/扫描/契约/布局通过。独立查询已验证，Object/Reflect 生产 native 入口、其余 traps 和同步内置仍待迁移；S05 未验收，未进入 S06/S07。

- S05 native 调用所有权准备：从原调用器抽出参数/帧所有者，保留完整 argv、错误的定义 realm 和 native 栈记录，验证放弃/异常展开及 metadata/runtime 拒绝。新库 2140 项、旧库 1996 项、新旧常规 oracle 各 907 项（各 1 项手动压力未运行）、CLI profiling 各 5 项及构建/扫描/契约/布局通过。当前仍由原同步调用器消费，下一步接 native 内置 continuation；S05 未验收，S06/S07 仍延后。

## 2. 编译与执行基础

S05 本轮统一收口（阶段验收通过）：同步 native 全领域共享 Step/Resume，
VM 查询只调度有类型的请求；对象/Proxy、iterator/collection、String/RegExp、
buffer/TypedArray/Atomics、Function/scalar/Date/Math/Error/JSON/weak 与间接转换已接入。
最后引用释放、literal/method 定义、非法调用/派生构造器、子帧安装事务和放弃执行的
释放顺序一并收口。现有默认预算不变，统一阶段门禁已完成；同一次验收中
修复失败并完成复核：非法 Construct 的 TypeError 已改由共享异常出口返回；有限递归
资源探测区分默认 VM 的物理栈和 owned VM 的逻辑帧，原错误与恢复断言保留；
async 预检探测改用直接索引收集 Promise，避免 Array.push 先耗尽预算。
具体入口审计见同步回调账本。S06/S07 未开始，不创建中途 commit。

本次门禁的语义与结构部分已完成：新库 2197 项、默认库 2032 项；新旧 oracle
各 911 项及单独执行的 65K 实参压力用例各 1 项；CLI profiling 各 5 项；
非 profiling 构建、4 项属性契约测试、723 个完整边界反例、631 文件源码布局与
格式/diff 检查通过。CLI 原有 `instanceof Array` 探测脚本保持不变，计数预期更新为
owned 配置零旧分派、零整帧交接和一次 S07 print 桥；默认配置保持旧分派预期。
原始 Earley-Boyer 独立执行与调用栈归因均完成，三个残余桥逐个确认是
`QjsConsoleLog`。原始组合脚本的八项 suite 与总分在完整归因运行中全部输出；
零旧分派、零整帧交接，十个同步桥事件逐个确认属于同一 S07 输出入口。
组合诊断首次触及外部 600 秒超时的失败记录保留；完整归因使用 1800 秒外部
watchdog，610.16 秒退出 0，VM 默认预算与原始脚本均不变。诊断成绩不作性能结论。

统一验收证据保存在 `target/primitive-vm-s05-acceptance/`，含所有失败/复核日志、
905 个 Rust/Cargo 输入的最终哈希及 `coverage/coverage-verdict.json`。覆盖二进制
SHA-256 为 `e458c028531a958baf724eac7dc570f0861de62173d1b5b81ba873c797d80b8a`。
S05 阶段验收通过；以本阶段计划消息提交一次后，才恢复 S06 的独立 stash。

### S01 — `refactor(compiler): organize stack compilation and diagnostics`

把既定栈架构、基础诊断与编译器结构整理合在一个提交：

- 明确本 PR 的五项问题、状态所有者和模块边界，采用线性栈 IR、有限局部优化与现有 RC/循环回收。
- 将 compiler 的共享模型按 ir/bindings/scope 归属；parser 的 context/builder 区分临时解析状态和后续产物。复用已有 resolution/lowering，保持完整语法、hoist/capture/eval 与声明顺序。
- 整理 resolved IR → 栈码草稿、块边界、正常/异常/恢复栈状态及片段重定位。消费已有存储；可选分析超预算时不使用未收敛事实。
- 建立最终码、编译阶段和执行成本的诊断入口；先接现有基线，新 frame/slot 路径的计数在 S03/S04 接入同一口径。复用 benchmark harness，正式热路径关闭诊断。

**验收：**编译器/语法/绑定 oracle、TDZ/private Reference、try/finally、恢复与源码位置用例；错误顺序保持。显式导入、源码契约和架构说明同步，不为文件移动增加镜像测试。不恢复 Tachyon 工具或历史结果文件。

### S02 — `refactor(code): unify stack contracts verification and publication`

形成一套可由 compiler、VM 和外部格式共同消费的代码契约：

- 统一指令栈效果、潜在效果和操作数描述。保留不同验证层分别证明的事实，不能删掉动态恢复检查。
- 把发布验证大函数按参数/绑定、控制流、私有状态、模块与函数树拆分；长 tuple 改成命名工作项，保持检查顺序，共享必要上下文，避免重复扫描整树。
- 分开纯验证与 Atom/heap 发布事务，保留已有 VerifiedFunction。发布输入归 code，编译请求编排归 api，生产验证不再依赖 compiler::EvalCompileContext。
- 定义 FrameLayout、code/sites/handlers 与 Runtime bindings 的边界；每帧通过一个 executable owner 取得 metadata，复用已有共享数组。

**验收：**真实发布、外部格式、畸形 code、mutation 反例与失败回滚；合法参数/局部/跳转范围不缩小。测试按语义组织，物理归属及源码检查同步迁移，不以只换哈希代替验证。

### S03 — `perf(vm): build the owned stack execution core`

把新存储和首个实际消费者一起交付：

- 建立 RunningExecution、FrameStore、SlotStore、typed binding、冷状态、运行登记 guard 和资源限额。按 max_stack 预留窗口、复用容量，定义 move/copy/clear；区分原始实参与可写形参。
- 从 host_bridge 提取已有绑定共享读写/捕获/关闭规则，建立运行 owning 值与长期挂起 raw 边的交接契约。测试构造、调用和恢复保留各自验证。
- 复用现有 pow/ToInt32，按 operations/integer/format/float16 整理纯 Number 算法；对象转换与栈操作各有所有者。
- 一个主 match 执行字面量、栈操作、普通局部、确切 Number 和静态分支。冷 payload 由 pending 拥有，RunExit 保持小型；经证明无分配/回收/回调的槽引用事务可留在循环内。

**验收：**区间互斥、扩容后无旧引用、逻辑 pop 的引用处理、返回最后引用、Runtime owning cycle 防护；zero queue 容量预检和事务不重复提交。验证 NaN、±0、溢出、位移、乘除余幂及 BigInt/String 慢路，记录 #5 路径和初步成本。调用/回调的未迁移范围明确归 S04–S07。

## 3. 调用与完整语义迁移

### S04 — `refactor(vm): drive calls conversions and unwinding with explicit frames`

在同一 driver 中完成普通调用与一条完整回调/恢复流程：

- 调用请求拥有 callee、receiver、实参、realm 和正常/异常恢复点；ordinary/bound/constructor 调用通过显式帧 push/pop，完整处理 this、new.target 与 derived return。
- ToPrimitive 状态和 step 归 value/conversion；建立 parent operation、单次回复和恢复阶段。一条 getter/valueOf 回调完整通过新 driver，其余领域调用点归 S05。
- unwind 统一安排 catch/finally、break/continue、IteratorClose 和 return/throw，保留各自错误优先级；关闭 capture 后才清帧，pending 和结果始终有明确 owner。
- 接通 lexical/const/TDZ、closure、每迭代 cell、默认参数、mapped/unmapped arguments、direct eval/with、private 与 readonly view。建立真实 host 重入的 delimiter 骨架，回调前结束内部借用。

**验收：**有限递归、小栈、可捕获无限递归、缺少/额外实参、跨 realm、嵌套 finally 和清理抛错；getter 次数与转换顺序。验证 `x+(x=2)`、回调改绑定后抛错，以及 `function f(a){'use strict';a=2;return arguments[0]}` 保留原实参。统计调用分配、初始化和引用记账；保留真实 native 栈保护。

### S05 — `refactor(runtime): resume synchronous JavaScript callbacks through the driver`

按领域完成同步回调迁移，保留逐调用点清单：

- **object/Proxy：**Get/Set、getter/setter、Object/Reflect、trap 与 receiver；复用现有属性存储内核。
- **Array/iterator：**callback、sort、species、迭代与 close；状态保存阶段、下标和必要值，回调后按语义重读可变化内容。
- **String/RegExp/buffer：**replacement 和 TypedArray/buffer 参数转换；回调后重新取得 view/buffer 凭证。
- **其余同步内置：**逐项核对 Function、scalar、Math、collections、Date、JSON、Error 和 globals。无回调 helper 保持普通函数。

**验收：**trap invariant、holes/原型访问器、修改 length、排序/迭代副作用、IteratorClose、Unicode/零长度匹配、resize/detach、共享内存、BigInt 与 GC；保留 PR19 的 Array/TypedArray 回退。全部内部同步回调由 driver 推进，不递归等待 JS，也不假装成外部 host。默认预算原始 Earley-Boyer 进行新核心覆盖筛查；异步/模块/API 余项明确归 S06/S07。

### S06 — `refactor(vm): unify suspension across generators and async execution`

语言状态机实现已收口，S06 统一阶段验收通过。挂起记录保存独立原始 argv
及对应 raw/Atom 边；共同 freeze/thaw 与 owned frame 处理五类挂起、恢复输入
和异常展开。generator 创建、resume、async body/await/settle、async generator
队列/await/completed return、Async-from-Sync、Promise 全部 selector 和作业均
通过有类型的请求/回复推进；getter、转换中的非 Normal 字节码也接入同一路径。
root 请求持有真实 continuation，不创建占位字节码帧。语言状态机保留原有
同步前缀、队列重入与微任务政策；模块和宿主入口按计划留在 S07。
开发定向 oracle、逐作业 GC/零桥接测试、原始实参与放弃状态检查已完成；
这些不单独计作阶段验收；半转换失败、wrong-runtime、最后引用等已在该次
完整新旧配置验收中一并复核。默认旧 VM 保留到 S10，freeze/thaw 表示适配
本身不执行旧分派。S07 尚未迁移的 opcode 仍可进入已计数的过渡桥；该桥
必须同时返回完成或挂起，避免 async 中 await import 被错误当作同步终点。
既有 256 KiB 模块图测试暴露查询 dispatcher 的 debug 原生帧过大；
仅按请求领域拆分原有分派和调用准备，保持状态转换、测试栈及预算不变。
旧 generator 栈测试假定 1000 层有限委托必然溢出；改用 Infinity 验证同一
溢出错误与恢复断言，并把 owned 的零桥接有限委托/finally 用例加强到 1000 层。

本阶段唯一统一验收结果：owned 库 2208 项、默认库 2035 项；新旧 oracle
各 911 项及单独 65K 实参压力各 1 项；CLI profiling 各 5 项通过。
非 profiling 构建、4 项属性契约测试、725 个完整边界反例、653 文件布局、
格式与 diff 检查通过。全部失败和修复日志保留在
`target/primitive-vm-s06-acceptance/`；最终 1024 个源码/构建输入哈希与
`stage-verdict.json` 记录同一次阶段验收，不创建 checkpoint commit。

共用 frame/stack/control 与 freeze/thaw，分清各语言状态机：

- **generator：**初始状态、next/throw/return、yield/yield*、finally 与 reentry。
- **async/Promise：**同步前缀、await、thenable assimilation、job 恢复；保持已有微任务顺序与 draining 政策。
- **async generator/iteration：**请求队列、各 Promise capability、异步迭代/close，以及 finally 内 await。

挂起时源 owner 保活到目标 raw 边全部发布；恢复时 heap owner 保活到运行 roots 和状态验证全部完成。关闭、失败和被放弃状态有唯一释放责任，不永久注册 owning wrapper 或全局 root。

**验收：**强制 GC、最后引用、半转换失败、单次 completion、重复恢复拒绝、交错请求、yield*、私有/捕获状态与关闭回收。内部 poll 不新增调度时机；不引入 #20 的 Fiber 调度或新的可挂起 host ABI。

### S07 — `refactor(api): integrate modules host and binary entries with the stack driver`

当前实现（S07 统一验收通过）：模块 link/evaluate 保留原 DFS/SCC 状态机，
分别由 `modules/link.rs`、`evaluation.rs` 拥有等待状态；body、异步完成回调和
Import 参数算法归同领域共享 Step/Resume。dynamic import load job 依次进入
link、evaluate、Promise attach 和 settle 根操作，保留每次操作之间的 RuntimeError
转换及原有 FIFO 政策。编译请求仍归 API，所有输入仍通过 code 验证/发布。

Context call/construct 和属性 API 使用带原始域验证的 owned 根请求；own descriptor
通过有类型的根结果返回。binary 翻译后的 callable 使用相同入口。Test262 evalScript、
Agent 转换及 qjs 输出已经登记；native/web/Test262 构建支持显式 `stack-vm`。
真实 loader/rejection tracker 边界使用 delimiter 和原 native 栈预算；活跃 Runtime
内没有真实宿主边界的嵌套根执行被拒绝。HostServices 的时钟/时区仍遵循既有
禁止同 Runtime 重入的同步值服务契约，未修改其 infallible ABI。

开发回归记录：首次 owned 库完整检查 2203 通过、7 失败。失败暴露了
ActiveFrameProbe 测试入口误登记为无回调操作，以及 checkpoint 测试在手动暂停
执行仍登记时调用 Context 的旧做法。probe 改用共享 InvokeStep；测试先准备输入、
或结束暂停执行再读错误，原错误和 active frame/backtrace 预期不变。放弃测试改在
实际 JS 子帧尚未执行时停住，继续检查 native owner 与 Runtime 释放。binary 测试
在原 25642 字节完全不变的文件末尾新增 GC 后执行及零桥接断言。

S07 首次正式 workspace 检查中，owned 库 2211 项、CLI 32 项通过，oracle
905 通过、6 失败（另 1 项手动压力尚未运行）。六项失败共同暴露模板对象
`PushConst` 仍走旧整帧交接；补入共享 value-constant 检查/持根冷操作，默认 host
消费同一规则。原 oracle 输入/预期不变，定向 11 项 QuickJS 对照通过，并增加
模板对象跨回调/GC 的身份及零桥接测试。失败日志保留，仍是同一次正式验收。

首次完整 owned Test262 执行 102037 variants：79966 pass，比冻结基线少 16。
新增失败均为 Promise all/allSettled/any/race 的 null/undefined iterable：
ReadValue 的准备错误越过了选定 continuation。修复复用原 value-property 读取
的 nullish TypeError → Throw 转换，让 Promise 算法按原阶段拒绝 capability。
未修改用例、admission、配置或预期；原失败向量与 runner 凭证完整保留，继续同一次验收。
原 16 项定向复验全部通过；再次完整执行恢复 79982 pass，TSV/JSONL 分类投影
通过，但冻结 receipt 哈希因首行 current-source 指纹不同而拒绝。逐字节验证确认
只还原首行指纹即可得到两个原冻结 SHA-256。门禁先认证当前 runner/报告来源，
再仅在临时副本中还原首行来源字段，严格比较全部结果字节；原报告不修改。
五项定向门禁检查覆盖有效来源、结果字节篡改、其他 metadata 篡改、错误来源与
重复来源，均按预期接受/拒绝。门禁修改后重新生成完整新旧 receipt。


WASM 首次运行要求有限 1000 层 yield* 溢出，新 VM 返回成功而失败。
默认配置的原输入/预期保持不变；owned 配置明确验证 1000 层返回 42，再用
Infinity 委托验证相同的可捕获 InternalError 和后续执行，未改变生产代码或预算。
两种配置重新构建并通过全部 15 个 playground 示例、metadata 和 Node/WASM 检查。

**S07 统一阶段验收通过（2026-09-14）。** 最终 owned/default workspace 全部通过：
库测试分别 2212/2035，CLI 各 32，常规 oracle 各 911，另行执行的 65K 实参压力
各 1，CLI profiling 各 5，Test262 runner 单元测试各 122。两种配置完整 Test262
均为 102037 variants、80032 eligible、79982 pass；全部结果字节与冻结基线一致，
只在认证后对临时副本还原首行源码指纹。完整 boundary 726 个反例全部拒绝；
属性契约 4 项、非 profiling 构建、662 文件布局、格式和 diff 通过。14 项门禁、
全部失败与修复、原始报告和最终 1163 个源码/构建/fixture 输入哈希保存在
`target/primitive-vm-s07-acceptance/`，最终判定为 `stage-verdict.json`。
本阶段只创建一次 commit；此后已在该干净 commit 上执行完整 benchmark/profile，
结果及原始样本说明写入 PR #21 comment，见[回退分析](performance/README.md)。S08/S09 优化、S10 默认切换和旧路径删除未实施。

完成全部既有生产入口的可测新核心路径：

- 模块 link/evaluate、live imports、循环模块、TLA、dynamic import 与 loader 失败/重入；保留模块自身状态机和特殊 linking 验证。
- Context、native/web adapters、CLI 和已支持的二进制入口接入统一 driver；外来 code 翻译后进入统一验证。
- 真实同步 host 边界保留 ABI，以 delimiter 和 native 栈预算处理 JS→native→JS；正常、异常及 unwinding 退出均解除运行登记。
- 审核 S05/S06 留出的全部调用点，收口编译请求与 code 发布的责任；测试和文件归属检查跟随真实入口。

**验收：**模块声明/初始化顺序、TLA 错误和 realm、binary fixtures/round trip/畸形输入、wrong-runtime 拒绝、平台与宿主重入。测试配置能让全部入口使用新核心；默认切换和旧桥删除在 S10。

## 4. 优化与最终交付

### 两阶段共同基线与验收口径

本节供实现者按依赖顺序执行；S08/S09 仍各为一个完整提交单元，下面的工作编号表示内部步骤与隔离实验，不增加正式 commit。原定 #5/#7/#9/#10 优化与迁移回退一起验收，不能只交付融合微基准，也不能把回退留到 S10 切换默认入口后再处理。

保留三个比较层次：PR19 是整个 PR 的性能参照；冻结的 S07 `23f5dcfe` 是本次修复基线；S08 最终源码是 S09 的增量基线。原始 S07 样本和失败记录保持不变。每个候选从固定父版本隔离一个变量，再测合并后的交互；记录源码/补丁、编译器、features/flags、二进制及 workload 哈希。后续按用户最新约束只运行新核心，PR19/S0 等旧值只读取历史记录，不再使用 default 执行路径作运行对照。

| 证据 | 共同要求 |
| --- | --- |
| 正式吞吐 | 普通 release、profiling 关闭；固定 50+8 全部用例交错 10 轮，原始 V8 八项和 combined 至少 5 轮。阶段内定向 A/B 不能替代完整矩阵 |
| 比值 | 固定和 compile 统一报新/基线耗时；原始分数报新/基线 score。某项任一轮失败即不提供该项有效比值，不从子集拼 combined |
| 独立编译 | 相同公开 compile API 和 67 个冻结源码，10 轮；源码读取/Context 创建在计时外，不执行 JS。不得用完整进程减去另一批编译时间估算执行时间 |
| 诊断 | 每个冻结 workload 验证 owned 指令为正、三种旧分派/桥计数为零。动态分派、逻辑搬运、分配、RC、PC 发布分开定义；尚未覆盖的值报 unavailable |
| CPU 与内存 | 单独构建/串行采集。对短热点另加标明用途的等工作量放大探针，获得足够采样后再归因，不改冻结输入。普通构建 RSS 与诊断 RSS 分开；self 占比不充当绝对耗时或收益 |
| 误差与身份 | 保留所有轮次、失败、min/max 和交错顺序；噪声敏感项预先固定配对区间估计方法并复测。若改绑核/频率政策，两边一起重测，不能混入既有样本 |

阶段报告逐项列出“目标热点 → 机制改动 → 计数/机器码变化 → 正式 A/B → 剩余回退与归属”。中位差超过 5% 是必须复核的工作阈值，**不是允许永久回退的额度**；小于该值但持续、可复现的下降仍要解释。S09 的退出目标是双方有效固定用例恢复到 PR19 水平或更好，并保留 #5/#7/#9/#10 的独立优化证据；这是验收目标，不是速度承诺。确认存在的剩余回退不得用平均数、栈深收益或“已完成架构迁移”抵销。

### S08 — `perf(vm): streamline owned execution and fuse stack operations`

**阶段目标：**先修复所有普通操作都在支付的执行/状态推进开销，再在相同所有权和观察契约上完成原定局部融合与 PC 优化。S07 的空循环约 2.03 倍回退与 TypedArray 写入约 5.28 倍回退属于两种不同路径，必须分别取得机制证据。

#### S08.1 — 建立可归因的执行诊断

- 在现有诊断内补充 run 出口原因、无 JS 回调即完成/实际发起回调的次数、各领域状态转移和 parent continuation 压入/弹出、Frame PC 写入与 Runtime 观察发布次数；普通计时构建不携带诊断开销。
- 记录 `Step/Resume/Next`、转换任务及其最大变体的实际布局，检查按值参数/返回的 memcpy 调用点、函数 `.text` 与 prologue/实际 native 栈消耗。已有 472/560 B 复制和数 KiB 栈预留是调查起点，不能当作完整类型大小或动态复制总量。
- 区分 immediate copy、String/BigInt Rc、Object/Symbol fallible retain、槽认证、逻辑 slot move 和机器码 payload 搬运。旧版没有的新计数不得补零；模型估算字节必须标明估算依据。
- 冻结热点组：循环/Number；属性和 TypedArray 写；iterator/for-of；String/BigInt/Math 的无回调转换；普通/closure/global 调用。同时保留 getter/Proxy、异常、GC 与挂起用例作为语义对照。

#### S08.2 — 压缩状态传递，让无回调步骤直接完成

- 查询驱动只传递小型动作/结果；跨回调 payload 由明确 owner 保存，避免完整聚合枚举在领域函数、`Next::Continue` 与中央循环间反复移动。领域内部步骤优先在同一领域推进，跨领域请求保留类型约束；不是把所有算法重新塞入巨型 match。
- 对 Primitive/Number/Element 已确定可完成的输入、已完成 NumericStep、两侧均为 primitive 的加法，直接消费共享领域结果。String/BigInt 可能分配/报错，仍在发布观察状态后的冷步骤执行；只省调度往返，不另写一份转换或数值规则。
- 普通 data property、稠密 Array、TypedArray 的安全读取/写入及 iterator 已完成结果，先尝试共用存储/领域内核；只在确实需要下一语义阶段、getter/trap/species/用户转换时安装等待状态。普通属性的 flags/prototype/receiver 检查与 PR19 Array/TypedArray 回退保留。
- 审核 TypedArray key/value 转换、resize/detach 和重新取得 view 的顺序；转换可以执行 JS 时必须保存阶段，回调后重取凭证。不能按“数值索引”跳过 canonical key、原型或失败规则。
- 不以每步 `Box::new` 替代 memcpy，不给普通快路增加每操作堆分配；只有需持久等待的冷状态才取得长期 owner。落地一个共同协议后，由 S09 继续处理其容量复用，不再更换推进模型。
- 立即完成仍保留 Return/Throw/引擎错误的区别、已选 continuation 的错误接收点，以及 native activation、错误 realm 和资源记账。用 S07 的 Promise nullish iterable 回归检验准备错误不会越过父状态；等待/放弃时 roots 与 native guard 仍按原顺序释放。

**定向证据：**typed_array_write、prop_write、array_write/update、array_for_of、math_min、bigint64_arith、string_build 系列；记录每操作的通用分派/parent 状态次数、实际复制调用点和时间。零回调路径应少建状态；真实回调仍由显式 driver 推进、单次回复，不递归等待 JS、不走旧桥。

#### S08.3 — 认证运行窗口，减少普通槽操作的重复工作

- 在进入 run 时认证 FrameId、窗口、已验证布局和容量，建立只在本次连续执行内有效的窗口视图。简单局部和操作数使用已认证索引；在切帧、扩容、释放 drain、分配/GC、回调或挂起前结束借用，恢复后重新取得视图。该阶段使用 safe Rust 实现；项目当前 lint 配置见 Cargo.toml。
- immediate 的 copy 和 Number 运算保持短小，Object/Symbol 可失败 retain 使用独立 helper；已证明无分配/回收/回调的引用事务可继续留在热路，复杂释放才退出到冷边界。String/BigInt 共享存储和最后引用释放规则不变，不将 value copy 一概视为可删除的 RC。
- 两个已认证的 Number 操作数就地消费/替换，省掉重复 peek/pop/push 认证与包装；按实际活跃区更新 sp，死槽立即失去 owner。先证明操作不会分配/回收/回调，才允许在连续运行区内完成。
- 本步先使用规范内存栈，保留绑定的 Direct/Captured/Uninitialized 区分；不同时引入 S09 的栈顶缓存或紧凑编码。原始实参、checked/readonly 绑定仍走各自规则。

**定向证据：**empty_loop/down_loop、int/float_arith、局部读取、Crypto/Navier-Stokes；展示槽认证和 helper 调用减少、逻辑 copy/move 的变化，以及正式吞吐。跨窗口/过期身份、容量失败、最后引用与中途 retain 失败必须仍可检出且只提交一次。

#### S08.4 — 精确观察状态与有限融合分别 A/B

- 在 S08.3 的借用边界上用局部 pc/sp；正常、错误和冷出口统一物化到已知 FrameId。分别计量 Frame fault/resume 写入与 Runtime 活跃帧发布，避免把 S07 已经按 run 出口发布的行为误算成逐指令 Runtime 更新。
- 列全 throw/backtrace、debug/hook、GC/分配/release drain、interrupt/fuel、JS/host 调用、yield/await 与恢复的观察点；pending 保存准确 read/convert/write site。异步 CPU 采样不宣称任意时刻精确 JS PC。不得每 N 条盲目发布。
- 在 S01/S02 的效果、栈、块入口、重定位和源码契约上加入有限 UpdateLocal（discard/prefix/postfix）与 CompareBranch；只有读取时刻已证明相同才加入少量 AddLocal 模式。不可进入融合中段，不跨 callback/handler/resume 边界偷移求值。
- 先测窗口优化后不融合的内核，再独立测 PC、UpdateLocal、CompareBranch，最后合并验证交互；只用于实验的开关和失败候选不留在正式代码。记录最终发布码、动态频率、逻辑操作 fuel 权重及编译新增成本。

**语义验收：**`x += g()`、`x+(x=2)` 保留旧值读取；postfix 返回转换后的旧 Numeric；const 更新先转换再 PutValue 报错；对象转换修改绑定后正常/抛错均正确；NaN 下否定 `<` 不替换为 `>=`。同时覆盖 ±0/BigInt/TDZ/captured、finally、恢复 PC 和单步源码位置。调试契约无法保持的模式在该模式下禁用融合。PC 收益不显著时保留真实结论。

**2026-09-14 收口调整（用户最新要求优先）：**“先收口 S08，如实记录当前性能情况和 profile 结果就可以了”。据此结束 S08 优化迭代，以独立 source-s08-23-checks 划定已实现阶段范围，当前工作树保留已有 S09 组合改动。原退出条件未完成项在收口报告逐项留账，后续 S09 联合交付继续验证；不宣称原门禁全通过，也不把全部函数调用性能恢复设为 S08 的结束前提。

**原 S08 退出条件（保留用于核对证据缺口）：**上述两类执行热点均有独立 A/B 与机制收敛证据；相关语义、完整 owned/default 回归/oracle/Test262、boundary、native/Web/WASM 与默认预算 Earley-Boyer 通过，完整性能矩阵和零桥计数齐全。非目标路径出现新回退先修复。本阶段不宣称全部恢复到 PR19；剩余项逐项列明 S07/PR19 差距和 S09 的存储/编译/布局责任。尚未解决的 S08 状态或槽热路问题留在本阶段，不能笼统转交“布局调优”。

### S09 — `perf(vm): make ordinary calls compact and remove depth-dependent bookkeeping`

**历史 S09.1–S09.3 执行边界（已由 c9 实现）：**当时仅实施三个调用/闭包/预算问题。本轮最新授权以新章节的 R1–R5、37 项回退及同一次最终联合验收为准；S09.1–S09.3 不重复实施，S10 不在范围内。

**当前执行状态：**S09.1–S09.3 保留，R1/R2/R5 确认漏项已补齐；实现未因本次测量改变。最新证据为 S0/当前各三轮的 58 fixed、67 compile、原始九项、33 探针、14 内存控制、3 RSS 和全部 58 fixed CPU/成本 Profile。32 项执行中位正差值、9 项编译中位正差值的确定程度见本节末“三轮公平复测”；S09 尚未达标。上次单轮验收仅保留历史。

**当前待办索引：**S09 剩余工作统一由下文“已提交后的全部 S0 回退修复计划”承接，包括 R1–R5、37 项逐项 S0 验收，以及最终候选的编译/语义/所有权/native/Web/WASM 等联合验证；后面的 S09 退出条件是同一次收口的验收要求，不是另一轮独立优化。S09.1–S09.3 的下列条目保留实施前问题、设计和验证契约，S09.4 的剩余工作由新计划具体化，不重复实施已完成机制。完成 S09 后，全文仍有 S10 的默认入口切换、旧路径删除、切换后验证和文档交付。

**2026-09-14 修订依据：**当前 `b36ad884` 的重新诊断见 [调用成本复查与修复设计](performance/README.md) 和 [数据记录](performance/README.md)。三个冻结调用用例十轮均有效，普通/全局/闭包调用相对 PR19 分别为 **1.4952 / 1.4648 / 1.2931**。普通调用的参数/冷帧分配已经达到平台，继续以 S07 的约 160 万次分配解释当前回退是过时判断。

**阶段目标：**重做普通调用的内部执行协议和所有权投影，消除闭包环境的逐调用复制与预算的祖先扫描，同时修复其余已确认回退。保留显式 JS 调用栈，不能返回 Rust 递归。既有参数窗口/容量复用是保留基础，不再作为足以结案的主要方案。

当前确定的三个问题：

| 问题 | 证据 | 修复要求 |
| --- | --- | --- |
| 普通 Call/Return 经多层通用协议，完整冷帧逐次构造/搬运/清理 | 无逐调用分配仍慢 49.5%；当前 perf 的准备、driver、安装和清理成本分散；机器码保留聚合副本 | 紧凑就地运行记录和专用普通调用/返回循环；不得只换 allocator 或全局强制内联 |
| 调用入口复制全闭包环境，再逐 cell 建 root、返回时逐 cell release | 单捕获仍约两次 malloc/调用；50000 次调用，捕获 1→256 耗时约 41→263 ms，实际不执行捕获读取 | 发布时构造共享环境，运行 callee owner 保活，入口对捕获数 C 为 O(1) |
| continuation 预算每次扫描所有祖先 | 固定 100000 次 native 叶子，深度 0→2048 耗时 60→343 ms；已有仅缓存等待深度的候选恢复约 62 ms | 增量维护总费用，热检查 O(1)，保留预算和溢出语义 |

普通浅层 `func_call` 不走等待深度扫描；闭包环境算法和部分准备/RC 在 PR19 已存在。不得把这些问题中的任何一个宣传为全部回退的唯一原因。紧凑调用方案的实际收益仍须独立实现和 A/B 确认。

#### S09.1 — 已发布调用事实和共享闭包环境

- 在 code/heap 的发布与闭包构造边界确认 bytecode、realm、function kind、binding layout、closure count/IDs 的不变量；普通入口一次认证形成私有调用视图。合并 `direct_call_target_from_value`、`bytecode_for_callable`、kind 获取、snapshot 与 active-frame 发布中已证明重复的查询；保留外部输入、跨 Runtime、损坏输入和恢复入口的验证及错误顺序。
- 以一个真实 callee owner 保活其 bytecode/closure 强边；提供只在该 owner 下使用的 immutable environment view，或发布时创建的共享索引数组。普通调用不再 clone `Vec<VarRefId>` 并 collect 全环境 `Vec<VarRefRoot>`。不能只做单捕获特例，也不能把同样的逐 cell 工作藏到另一层缓存。
- 捕获值继续通过原 cell 读写内核动态访问，TDZ/const/private、重复 ID 强边计数和 eval 可见性不变。共享 ID 数组不是 GC root；不得在 heap 的缓存中存 Runtime-owning roots 造成环。独立挂起/外部生命周期通过显式 owner 转交保持存活。
- 列出 function、bytecode、realm/global、closure、参数、结果的 owner 表和观察边界。运行帧持根时，登记只持 token/身份；移除与运行帧重复的 guard roots 必须先完成契约改造。单次借用不跨 GC、释放 drain、JS/host 回调或 arena 扩容；该阶段采用 safe Rust 的 owner/token 方案。
- 把不变的调用布局/原始实参观察需求放在已发布事实中，避免逐调用重新扫描代码。函数对象的可变属性和捕获值不作不失效的调用缓存。

**验证：**固定调用数，环境宽度 0/1/8/64/256，包含本次完全不读取捕获的分支；分别测创建、稳态调用和最终回收。入口和退出的环境复制/逐 cell root 工作必须消失，调用成本不随 C 线性增长。覆盖外域/损坏 metadata、callee 最后引用、返回逃逸闭包、重复捕获、重入、挂起和释放。

#### S09.2 — 普通调用/返回驻留于短循环，帧直接就地初始化

- 依赖 S09.1 的调用事实，建立普通 bytecode Call/Return 的短路径：结束当前运行窗口借用、发布必要 PC、预留并直接安装子帧、取得子窗口继续执行；普通 Return 先移交结果 owner，原位解除登记和清理，再取得父窗口继续。不通过通用 `RunningExit`/`FrameExit`/完整 FrameEntry 再包装同一普通完成，不把所有冷分派内联进 run。
- 保留小型热帧：executable/环境视图、pc/sp、窗口边界、返回目标与登记 token。Query、异常区域、constructor、generator/async 等真正需要的冷 payload 首次触达时再物化，容量仍归 execution 复用。普通调用不能先构造完整 FrameCold 再 memcpy 到已复用 Box；销毁也不先把整块冷帧 take 到 Rust 栈再丢弃。
- 用同一槽事务实现 normal entry，恢复/外部 materialized entry 为显式慢路。消除通用 `push_frame_storage` 为 fresh/restore/source 等所有组合携带的中间 FrameStorage；正常路径所有可失败预留先于 owner 转移。保留实际 arity，参数/局部直接初始化，只清理实际拥有值的区域。
- 原始 argv 与可写参数按已发布观察需求分开：无 arguments/eval 等观察者时避免无条件双份快照；有观察者时保留独立来源或有证明的写时分离。优化必须保持已求值实参的持根和释放顺序，不能因函数未读取参数而提前释放 Object/Symbol owner。
- 普通完成与 throw/unwind、finally/IteratorClose、tail、constructor、真实 callback、host 重入、yield/await 分开路由，但都消费同一语义内核。保存精确 fault/resume PC、错误 realm、fuel、结果先持根及登记清理顺序；Rust unwind 和中途失败仍有唯一清理责任。无需先实现通用尾调用复用才能得到普通调用收益。

**验证：**普通/全局/闭包、zero/多参数、method、对象参数/结果、缺失/重复参数、strict/non-simple/mapped arguments、rest/spread、65K 实参、正常/抛错/挂起分开。对固定调用数按 N/2N 和 arity/locals 缩放，记录完整机器指令、分类/认证/登记次数、聚合副本和 native 栈；既有分配平台不能代替吞吐。单独 A/B 共享环境、帧布局、调用/返回分派，再测组合交互。

#### S09.3 — 预算改为增量记账，消除祖先扫描

- FrameStore 维护已安装等待的总深度；Query 维护 parents/native scopes 的累计费用。push/pop、等待安装/取出、回复和放弃时更新，正常 `can_push_with_continuations` 和 `continuation_depth` 都为 O(1)。已存在的 wait-depth-v2 是外层扫描的实证候选，未合入且尚未完成内部 Query 扫描修复。
- checked overflow、拒绝前后状态、None/溢出恢复、重复/过期身份和错误优先级保持。缓存值由拥有状态变更的 API 维护，不允许调用方绕过更新；失败预留不增加费用，取出后推进的 Query 不重复计入已安装总和。
- 逐项审计 Runtime native/host 预算：`native_call_would_overflow` 仍有 active_frames 权重和 family 扫描；先确认实际触达路径，再用累计权重和 family 计数替换。native_continuation/Function.prototype.call 的既有费用规则、真实宿主栈检查与所有限额不变，不把该扫描误称为已测 Math.abs owned 路径的根因。
- 正常 LIFO 删除 O(1)；异常清理按实际删除项 O(k) 更新，不扫描未删除祖先。主动 finish、Drop、deferred pop/truncate、host 重入与挂起恢复必须共用更新责任。避免只缓存外层和在每次检查中重扫内层。

**验证：**固定 K 次叶子调用，独立改变 D=0/32/128/512/2048，JS/native/getter/Proxy/混合 callback 分组；另对每层一次 native 的递归做 D/2D。记录访问的祖先/Query 项数，记账不应为 Θ(KD) 或 Θ(D²)。预算边界、溢出恢复、有限/无限递归、两 MiB 小栈、host 重入和默认预算 Earley-Boyer 必须保持。此项可与 S09.1 的只读设计/独立实现推进，性能测量与构建测试仍分开排程。

#### S09.4 — 最终回退、编译和条件布局评估

- S09.1–S09.3 定向机制成立后固定组合版本，重新 profile 其余 Array/Math/Map/属性/String 热点；明确每项的状态、调用、存储或算法原因。不能未经归因笼统归给“布局”，也不能以调用微基准抵销其余回退。
- 对同一公开 compile API 和 67 个冻结源码归因 parse/resolution/lowering、融合/relocation、verify/publish 的时间和容量；inclusive/exclusive 分清，不以不同批次完整进程减编译估算执行时间。已实施 CompactVisits/坐标游标仍以最终矩阵确认，保留被否定的 ASCII 候选结论。
- `.text`、静态 memcpy 和栈预留分别对照实际 instructions/cycles、可用的 branch/cache 事件、普通吞吐和 native/WASM 结果。当前 perf IPC 下降不能自动证明 I-cache 问题；不通过提前删除 S10 旧路径或全局强制内联隐藏成本。
- TOS 0/1/2 和紧凑编码仅在剩余 profile 支持时继续，历史负面证据可支持不采用。它们不是修复已知调用算法的前置条件，不再增加无依据的布局候选组合。若实施，仍验证所有正常/异常/GC/host/挂起物化和同一外部字节码契约。

#### S09 已提交后的全部 S0 回退修复计划

**范围与状态。** 实现基线是 `c9a2607c`，本轮组合候选为 `61e585a4`。用户要求 **所有相对 S0 的实际回退都要解决**；只处理 native 重复认证和字符串转浮点的旧提案已作废。S09.1–S09.3 保留。R1–R5 本轮已产生组合实现，但 R2 外层内置 replace 调用和 R5 新属性选定事实复用仍有漏项；不能声称计划契约全部实现，更不能声称 S0 性能验收完成。以下保留原契约和修复前账本，当前结果以本节“本轮执行结果”及链接报告为准。

固定 58 项的已有正式四轮中位数中，38 项高于 S0。按用户此前明确判断保留浮点转字符串的排除，待处理 **37 项**；其中 34 项超过 5%，另有 `arguments_read`、`string_build1`、`string_build3` 三项低于 5%，同样保留。TypedArray 写入按用户判断排除，且当前本就比 S0 快 28.38%。5% 只是此前筛选诊断的阈值，**不是允许的回退，也不是关闭问题的条件**。相对父提交新增两项与相对 S0 遗留 37 项是不同口径；native 重复认证是共用成本原因，不是额外一个 benchmark。

**修复前证据和计数口径。** c9 正式耗时复用 `target/primitive-vm-s09-regression-audit/metadata.json` 的 `formal_medians_ns`；源码/机器码、36 项当前 CPU/成本 profile 和准入的历史 profile 见同目录 `report.json`、`report.md`、`validation.json`。只给此前缺少当前诊断的三个低于 5% 用例补了一次同源诊断，保存在 `plan-supplement/`：12 条 stat/record/cost/report 任务均成功，执行输出与冻结 receipt 一致；未构建或重跑 S0、S07、S08、父提交。正式数据不被这些单次诊断耗时覆盖。

本节中的调用次数、认证次数、Box 容量是机制证据；CPU self 百分比仅定位热点，不能当成可追回的耗时比例。已有 S0/当前硬件指令对比用于确认额外工作，不能从不同采样分母的 self 百分比相减得到回退贡献。小幅回退保留为待验收项，不擅自称为噪声；单份当前 profile 也不能证明其全部统计差异由某个热点造成。

**修复前逐项账本。** 下表数值是 c9 的历史状态，不是本轮结果。R1–R5 是五组共用实现；每行仍独立验收，综合用例不能用微基准代替。[本轮 37 项新旧对照](performance/README.md)中，5 项单次不高于 S0、32 项继续未完成；5 项也不伪称统计置信通过。

| 用例 | 修复前 c9 相对 S0 耗时 | 修复组 | 具体覆盖 |
|---|---:|---|---|
| `prop_create` | +29.07% | R5 | Set/Define 就地推进、复用键和选定的存储事实 |
| `prop_clone` | +14.18% | R5 | spread 的 snapshot/read/define 就地游标 |
| `prop_delete` | +35.49% | R5 | 同时覆盖 fixture 中的 spread 和 Delete 完成协议 |
| `array_prop_create` | +21.42% | R5 | Array/普通属性 Set 状态与键转换 |
| `array_slice` | +29.17% | R3、R5 | 已有本地 copy loop 的 SliceResume 搬运和重复 key |
| `array_length_read` | +26.84% | R5 | Array.length 直接读取及结果事务 |
| `array_length_decr` | +32.72% | R5、R3 | primitive length Set 本地完成；fixture 还包含 slice |
| `array_push` | +52.60% | R1、R2、R5 | native 入口、Mutation 状态及普通 Set |
| `array_pop` | +46.57% | R1、R2、R5 | native 入口、Read/Delete/Set 本地推进 |
| `arguments_read` | +1.26% | R5 | Arguments 下标读取退出；保留对象创建和映射 cell 语义 |
| `local_destruct` | +27.98% | R3 | 数组 rest 的 next/append；普通对象字段已有直接读取 |
| `bigint64_arith` | +33.67% | R4 | Mul 直接完成以及两次已有 Add/store 的事务成本 |
| `map_set_string` | +50.70% | R1、R2、R5 | native 入口、String(number) 无回调转换及属性读取 |
| `map_set_int` | +17.85% | R1、R5 | 已是 Pure 的 Map 调用入口及属性/槽事务 |
| `map_delete` | +47.13% | R1、R2、R5 | String(number) 与 native 入口；不归为哈希平方算法 |
| `weak_map_set` | +19.87% | R1、R5 | 已同步的 WeakMap 调用入口及属性/槽事务 |
| `array_for_in` | +54.04% | R3、R5 | 普通候选状态、String 索引读和最后键 owner 释放 |
| `array_for_of` | +12.69% | R3、R1 | 同步 next 的 pending Box、native 进入和状态搬运 |
| `math_min` | +41.20% | R1 | 重复认证、逐参数槽操作及同步完成外围工作 |
| `regexp_ascii` | +22.20% | R1、R2 | 原语输入/lastIndex 转换在 exec 内直接推进 |
| `regexp_utf16` | +23.35% | R1、R2 | 同 exec 协议；保留现有 UTF16 matcher |
| `regexp_replace` | +26.83% | R1、R2、R5 | 转换/属性协议本地推进，复用已有标准替换内核 |
| `string_length` | +12.80% | R5 | linked length 键、直接读 UTF16 长度、结果事务 |
| `string_build1` | +2.87% | R4 | 已有拼接/store 融合的重复认证和完成协议 |
| `string_build3` | +3.27% | R4 | 同上，覆盖前置拼接及 owner 释放 |
| `string_build_large1` | +8.69% | R4 | 同一 Add/store 协议；保留字符串内核 |
| `string_build_large2` | +9.46% | R4 | 同上，覆盖前置拼接 |
| `int_to_string` | +19.50% | R4 | number + empty string 已融合，压缩完成协议 |
| `string_to_int` | +13.69% | R4 | 原语数值运算直接完成与结果事务 |
| `string_to_float` | +35.26% | R4 | 字符串减法直接完成；解析放在 RunSlots 借用之外 |
| `v8-richards` | +9.11% | R5、R4 | 137,910 次直接属性读仍走外围协议；数值/调用混合 |
| `v8-deltablue` | +6.80% | R5、R1、R2 | 261,858 次直接属性读及 Array/native 状态 |
| `v8-crypto` | +11.36% | R5、R4、R1 | 1,501,564 次 GetElement、868,703 次 SetProperty 退出 |
| `v8-raytrace` | +25.97% | R5、R1、R2 | 属性/创建、native 与真实 callback 混合 |
| `v8-earley-boyer` | +39.07% | R5、R1、R2 | 属性、native、真实 callback 和 owner 搬运混合 |
| `v8-regexp` | +40.61% | R2、R5、R1 | RegExp/UTF16 内核外围约 839 万次 Query dispatch |
| `v8-splay` | +16.52% | R5、R1、R4 | 对象定义/属性读、调用、转换及 owner 搬运混合 |

##### 前轮执行结果（61e585a4，2026-09-15，历史记录）

以下保留 61e585a4 的历史结果；当前状态见本节末尾“漏项补齐后的单轮验收”。[完整报告](performance/README.md)和[可核对数据](performance/README.md)绑定同一最终源码、普通/诊断二进制、冻结输入与输出。先冻结组合代码并完成正确性门禁，再执行一轮 benchmark/profile；S0/S07/S08/base23/c9 只读取历史记录，没有重跑。

| 组 | 已确认达成 | 本轮仍未完成/残余 |
|---|---|---|
| R1 | 一次 argv-domain、native 批量转移、publication witness、同步 native 不等待；Math 槽认证减少约 553 万次 | ordinary/native 前置分类和一般入口仍重；新 classification retain 有机器码；Math 仍 +34.23% / S0 |
| R2 | String(number)、exec 原语阶段、Mutation 和 replacement cursor 本地推进 | **明确漏项：**已选标准 @@replace 的外层仍走通用 Call/Query，固定 replace 仍有 115,000 次外层 direct wait；Step/Resume/Next 各增 24 B，等待搬运未消掉 |
| R3 | for-of pending Box 897,130→17,420；数组 rest 128,000→32,000；slice resident 和 for-in String 索引即时读取生效 | Start/Close 保留符合计划；for-in/rest 其余必要及外围协议仍重，分别 +15.59% / +20.90% / S0 |
| R4 | primitive Numeric 直接完成，原 fusion 保留，槽认证约减半 | 新共享 push pending 搬运/tag 检查保留在机器码；指令减少而部分周期上升，全部回退原因尚未精确分解 |
| R5 | linked/即时读、短结果事务、resident Set/Copy/Delete/ArrayLength 生效 | **明确漏项：**新属性初始 Missing/descriptor 事实未复用，同 receiver 重复 probe/lookup；prop_create / array_prop_create 仍 +25.15% / +11.94% / S0 |

原 37 项中，本轮不高于 S0 的 5 项是 `array_slice`、`array_length_read`、`arguments_read`、`array_for_of`、`string_length`，其余 32 项保持未完成。原账本之外另有 `global_read +5.24%`、`global_func_call +0.72%`、`func_call +3.03%`、`float_arith +9.24%`，全部留在本节，不以小幅或控制项为由排除。TypedArray 写入和浮点转字符串继续按用户旧判断单列排除。

67 项 compile 有效，但 8 项单次高于 S0（包括 `v8-splay +69.25%`），报告逐项保留；没有据一个样本就断言原因或关闭为噪声。原始独立 8 项成功，combined 在预定 180 秒上限超时。default/stack-vm 正确性门禁、33 项调用/深度探针和 41 项成本 profile 有效，三个 legacy bridge 为零；Test262 结果向量与 c9 一致，既有失败未隐藏。这些证据不能替代性能目标。

本轮没有继续改代码或追加第二轮性能测试。**下一次继续时仍在本节处理 R2/R5 漏项、已有残余和本轮新增正差值；本轮不能宣布 S09 完成。**下面 R1–R5 与最终退出条件是同一验收契约，未满足的条目不另立阶段、不转入 S10。

##### R1 — native 入口一次认证、批量参数转移与紧凑同步完成

**确认的成本。** `driver/ordinary.rs` 在普通函数筛选之前扫描 receiver/argv，native 退出筛选后 `driver.rs::enter_call` 又扫描一次；同一个 `Math.min(1,2)` 的旧断点记录是父提交一次、当前两次。`enter_call` 还逐参数检查/pop 后 reverse；`call/native.rs` 和 `frames.rs` 在准备与发布之间重复核验 callable/runtime/realm/固定 native 元数据。`proxy_get_driver.rs::start_classified_native_call` 在同步结果确定前就更新 Query generation、准备等待缓存和通用结果容器。

Math.min 的 1,105,000 次主体调用**已经**不分配域内 argv 且没有逐调用 Query，却仍有整用例 18,785,479 次槽认证；Map 整数和绝大多数 WeakMap 调用也已经 Pure。修复应删除剩余外围工作，不能把已有零 Query/argv 优化再次列作新成果。

1. 私有 callee 分类先无副作用地区分 ordinary/native/general，再按现有顺序认证 receiver 和实参。普通分类与 `OrdinaryCall::authenticate` 复用同一次 bytecode/closure 事实；无副作用分类先借用现有槽 owner，实参检查通过后才把 callable owner、target、defining realm、ABI 和 operation kind 提升为准备结果。不能提前执行可能抛错的完整 metadata 准备而改变 foreign-domain、坏槽、不可调用对象的错误优先级；无法无副作用分类时原槽/PC 不动，回现有通用入口。
2. `SlotStore` 增加一次认证的 native operand 转移事务：确认 frame/range、receiver 优先及实参从左到右的域检查；复用 argv 容量，必要容量准备成功后才消费 owner；按参数顺序移出 tail，一次取出 receiver/callee，去掉逐参数重新认证和 reverse。**保留现有 owned NativeArguments ABI**、actual arity 与 readable padding 的区别，不扩展所有 builtins 的参数表示。
3. 私有、一次消费的 `NativePublicationWitness` 将已核验的 native 固定事实传给 frame publication。witness 绑定同一 runtime、被保活的 callable 和实际执行 realm；独立 host/test/general publication 仍完整检查，不能把不可信元数据全局标成 trusted。历史 `source25-native-publication-witness` 仅是未验证草稿，不作为已实现或测试通过的证据。
4. 已注册同步 native 家族使用小型 Completion/InvokeOutcome，直接接一次结果提交事务。保留本地 NativeActivation、诊断帧、realm、错误物化、logical/physical budget 和 host reentry；logical/native budget、token 及必要发布检查仍须在任何 builtin 副作用之前完成，不能延后到 callback，否则 Map.set/push 可能先修改对象再被预算拒绝。只有真正需要等待才迁移 activation、安装 Query/等待身份。错误须在诊断帧和选定 realm 仍有效时物化，再按原 finish_reusing 顺序退出 active frame、释放 callable，最后释放全部 readable argv owner（包括多余参数）。Pure 可以省去等待容量；可能回调的操作必须在执行该可观察动作前准备好保存选定状态的容量，不能先产生副作用再因无法安装状态而重放调用。

**机械验收。** 有效 native 和普通直接调用均仅一次 argv-domain 认证；native publication 不重读同一组固定元数据；参数转移没有逐参数 frame 认证；Pure 不递增 Query identity、不占等待缓存。必要 native activation 的进入/退出计数仍与调用一致，不能删除预算/诊断帧来达标。

**语义验收。** zero/method/多余参数、padding 与 actual-count；Math NaN 后仍转换后续实参、负零、BigInt/Symbol；foreign-domain/坏槽同时存在、bound/Proxy/eval fallback/tail、realm 与调用方 realm、native 错误栈、iterator hidden flag、拒绝前的槽状态、callee/额外参数最后 owner、OOM/预算拒绝及 host 重入。复用现有 native/ordinary/domain 测试并补真实入口次数断言。

##### R2 — 无回调的内置阶段在域内完成，状态保留在原位

**共同设计。** 各家族继续拥有唯一语义状态，以 `advance(&mut state, reply)` 本地推进，返回小型 Complete/NeedRead/NeedCall 等结果。同步阶段不反复把完整 Resume 塞进通用 Step；真正 getter/Proxy/转换回调出现时才将**确切选定进度**移入 owned pending。读取、写入、转换继续使用共享内核；不探测一次再从头执行一次，不为每个同步步骤新建 Box。

- **String(number)：** `builtins/primitive/constructor.rs::start` 当前为原语 Number 也发出 String/Primitive 请求，`converted` 才返回；在构造器内用原转换内核直接推进到 `converted`，并在 `builtins/continuation.rs::output::deliver` 的小结果入口完成。`map_set_string` 的 40,013 次 native activation 中 20,012 次迁移等待；`map_delete` 为 68,020/34,019，均包含大量无需回调的 `String(i)`。这部分等待应按实际 String(number) 调用数消除。保留 String(Symbol) 与 new String(Symbol)、无参数与 undefined、Number(BigInt)、new.target.prototype 观察和 realm 的区别。
- **Array push/pop：** `builtins/array/mutation.rs` 当前把 length Read → Number → 元素 Read/Delete/Set 等逐步交给通用协议；push 有 268,002 次等待迁移、1,072,014 次 Query，pop 为 179,216/1,431,941。一个 resident MutationState 持有 argv、receiver、长度和游标，本地完成普通属性操作及原语转换，再经 R1 直接提交。复用 R5 的 prepared Set/Delete；genuine Array 不能绕过 inherited setter、不可写 length 或拒绝。标量 push 的 inline argv 和避免冗余 length 写入**已经存在**，不重复实施。保持 Pop 的 Get→Delete→Set 顺序、部分副作用及共用 shift/unshift 的方向和 holes 语义。
- **RegExp exec：** `builtins/regexp/exec.rs` 为已经是 String 的 input 和数值 lastIndex 发出两次原语转换步骤；ASCII/UTF16 用例有 226,023/228,023 次 Query。brand、input 转换、lastIndex 读取/转换和 `finish_builtin_regexp_exec` 在同一状态内推进；只有实际 Object 转换或自定义 exec 才等待。matcher、结果数组与 lastIndex 更新仍用原内核。
- **RegExp replace：** `builtins/regexp/replace.rs::prepared` **已经有标准 RegExp matcher 快路**。优化其前面的原语 input/replacement 转换，以及 `builtins/string/replace.rs` 外层方法读取/调用；已认证的 builtin @@replace 经 R1 完成，保留需要的两层诊断 activation。真实自定义 exec/@@replace、functional replacement、groups getter 保留回调；同步阶段的 replacement cursor/buffer 留在原位，不反复搬运累计结果。UTF16 遍历和 regex 执行成本单独保留可见，不把所有时间归给 Query。

**验收。** dense scalar push/pop、String(Number)、标准 exec 的无回调主体不再安装 native waiting scope；必要存储写/删除/转换次数和分配仍准确保留。测试 frozen/sealed/non-writable length、稀疏和原型 getter/setter、Proxy trap 顺序、callback 改变 length/prototype、重入、失败后的部分写入；RegExp 输入转换修改 lastIndex、exec override、global/sticky/non-global、UTF16 surrogate/空匹配推进、替换 tokens/命名捕获、callback 抛错与预算中断。现有标准快路谓词原样保留，不能按属性名认定任意函数是内置函数。

##### R3 — 迭代同步 next 先完成，for-in/slice 用就地游标推进

**for-of / 数组解构的已确认分配。** `iterator_driver.rs::operation(Operation::Next)` 在尝试已有 raw Array-next 之前调用 `PendingIterator::new`，每次分配 312 字节通用等待状态。`array_for_of` 共 897,130 次、累计 279,904,560 字节；`local_destruct` 共 128,000 次、累计 39,936,000 字节。这是累计分配量，不是常驻内存。前者已有 879,710 次无 Query next、871,000 次 dense immediate，后者有 96,000 次同步 next；“没有 Query”显然没有消除前置 Box。

1. 在 iterator region/frame/operand 认证后、构造 PendingIterator 前尝试同步 next。iterator 和已捕获 next 方法由原 record 槽 owner 保活；复用 R1 的私有认证/紧凑 native guard，零 JS 实参不经过一般 argv 准备。非原生、Proxy、bound、custom next 保留精确 fallback。
2. 复用 `builtins/iterator/array/local.rs::dense_immediate_next` 的现有守卫，并让一般 ArrayNextResume 的 Length→Number→Value 同步阶段在单一 cursor 中推进。保持每次 next 的 live length、index 在 getter 前推进、done 时释放 source edge，以及 value/done/iterator region 提交顺序。遇到真实等待才保存已推进的 index 和已选 getter，绝不重启 next。
3. iterator_generation、property_generation 和 Query 容量仅在相应真实 wait 安装时发生。最后 owner 释放、typed-array detach、孔洞、结果对象分配等仍遵守原操作边界；不能为了无分配数字把实际语义工作移走。
4. 固定用例的**结构目标**：若 start/close 不变，for-of pending Box 从 897,130 降至最多 17,420，local_destruct 从 128,000 降至最多 32,000。保留 native guard 计数并检查实际字节分配，不能只删除 profiling 事件。此目标不是耗时收益估计。

**for-in。** `for_in/operation.rs` 已有普通 Keys/Enumerable/Own/Prototype 本地循环，整用例仅启动阶段的 14 次 Query dispatch；没有新的平方扫描证据。剩余 2,123,425 次本地 step 和 1,379,550 次 ForIn exit 仍逐次构造/搬运 Own/Candidate 状态。保留 `next_for_in_candidate` 的 heap snapshot/visited 游标，把非 Proxy skip/own/prototype 推进改为借用原 iterator owner 的小结果循环；只为实际等待提升 key/object owner。已知数字索引使用现有 canonical integer key 完成内部检查，仍向 JS 产生真实 String。另依赖 R5 处理 1,352,500 次 String 索引 GetElement exit 和 1,352,499 次最后 String owner ReplaceBinding exit。不能声称消除了每元素 Query，也不能缓存整个历史 key 列表掩盖释放成本。

**slice。** `builtins/array/slice/local.rs` 已本地执行 872,000 次 Has、872,872 次 Read、872,000 次 Define；CPU self `SliceStep::advance_local` 为 18.93%，指令数 S0 5.003 B→当前 5.889 B。剩余问题是按值传递的 SliceResume、重复索引 key 和 source/result root。将它改为唯一 resident SliceCursor，`&mut cursor` 推进 Has→Read→Define；同一个源索引 key 用于相邻 Has/Get，source/result/argv/values 只持有一份，真实 getter/Proxy/species 才保存 owned continuation。复用共享读写内核及 splice/toSpliced 的原阶段，不做无条件 dense memcpy。

**语义验收。** for-in 顺序、non-enumerable 遮蔽、删除和原型变化、Proxy 的 ownKeys/descriptor/prototype 顺序；next 方法捕获、brand/realm/live length、getter 推进顺序、done 释放、IteratorClose/break/throw、elision 与数组 rest；slice holes、继承 getter、species Proxy/source alias、Has 后状态改变、copy/define 抛错和最终 owner。`local_destruct` fixture 是**数组 rest**，128,000 次普通对象字段读取已在 run 中完成，不能借机重做对象 rest。每个元素的必要语义观察数不变，owned cursor 搬运只随真实等待增长。

##### R4 — 原语数值和已有 Add/store 融合共用输入、输出事务

**覆盖不能只限字符串减法。** `string_to_float/int` 的 500,000 次 Numeric 已无 Query，但各有约 3,000,479 次槽认证；BigInt 的 3,200,000 次 Add/store、int_to_string 的 1,500,000 次、四项字符串构建的 1,600,000 次均**已经融合**。`conversion_driver::complete_primitives` 的当前 CPU self 在 BigInt/int_to_string/large1/large2 为 27.31%/28.76%/33.29%/28.72%；新补 small1/small3 为 35.02%/32.53%，两者各 9,603,679 次槽认证、仅启动阶段的 14 次 Query dispatch。主要新工作是压缩已有完成协议，不是新增 fusion 或改字符串算法。

1. `driver/ready.rs` 不再提前对同一 Numeric/Add 输入各做独立 checked peek。共享 primitive 输入入口在一个短期 RunSlots 中按**该旧入口的错误/域检查次序**完成分类、现有 add_store 目的地认证和 owner 取出。输入是拥有式 Value 和小型 Operand/ExistingDirectLocal 目的地，不携带跨分配的槽引用。Object/不支持 kind 退回原路径；Numeric 的坏槽/缺左槽保留原 right-pop/left-pop 失败时的部分消费语义，不能擅自变成另一种原子性。
2. 结束输入窗口后调用现有 `to_numeric_primitive` + `binary`、`add_primitives`/原有 unary-plus 内核，严格左到右转换；纯二元算术不再创建 Left/Right NumericResume，直接返回值或原错误。BigInt 必须覆盖 Mul **和**两次 Add。该阶段复用字符串 parsing/格式化/flat/rope 内核及共享语义实现。
3. `string_to_number` 目前会分配 UTF16 Vec 和 String，BigInt/拼接也可分配和释放，所以运算保留在 RunSlots 之外。一个输出事务重新认证、写回 operand 或现有 direct-local fusion；若有 previous/value 两次 push，仍保留 previous→value 的原提交及失败顺序，不能借“事务”改为全有或全无；写 local 前发布原 store fault/resume PC，离开借用后才释放被替换的旧 owner，保留 span 2/3、discard、const/TDZ/captured fallback 和结果/异常物化顺序。
4. 同步成功直接回到运行状态，通用 CallStep/Query 适配只保留在冷分支。`proxy_get_driver.rs::start_numeric` 的 generation checked_add/store 移到 Complete/Throw 之后的真实等待分支；同步数值不签发回调身份。Add 的 ConversionTask next_operation 属于另一协议，若统一入口需要移动它，必须显式覆盖其溢出和对象 fallback，不能悄悄混用两种身份。

**机械验收。** 原有 fusion 命中数保持；primitive Numeric 的 Left/RightNumeric、通用 start_numeric 热调用及无等待 generation 写入消失；输入、输出及恢复 run 约三次必要认证替代当前约六次/运算，是待验证的结构目标。PC 发布和分配边界不要求归零；检查实际认证/owner clone/机器码搬运实减，函数改名不算完成。

**语义验收。** String 空白/空串/指数/进制/Infinity/NaN/-0/非法 UTF16，左右类型及转换顺序；BigInt/Number 混合、Symbol、除零/移位的共用内核保护；append/prepend、别名和最后 String/BigInt owner；旧 local 是 Object 的释放观察 PC、融合 span/TDZ/captured、Object valueOf/@@toPrimitive 重入与 throw/finally；坏槽、外域值和身份 MAX/过期回复。

##### R5 — 属性读写的短事务、键复用与普通存储阶段就地推进

**读路径证据。** `property_driver::read_progress` 已借用 receiver，静态键也已有 `property_key_atoms`，但静态读取仍建立 PropertyKey owner、进入外围 driver 再提交。Array.length 有 3,054,277 次 GetField exit、9,163,312 次槽认证；String.length 为 1,946,213/5,839,115。`stack.rs::array_immediate_read_current` 只接受 Int key，for-in 的 String 索引全数退回。新补 Arguments profile 有 230,388 次 GetElement exit 和 2,304,363 次槽认证；`object/ordinary.rs` 对 Arguments 转到 special→完整 property descriptor→VarRef 值的路径，`arguments_test` 本身无形式参数、按三个 extra-argument cell 读取。这里的额外读取协议已定位，尚不能宣称它解释了全部 1.26% 总耗时差异。

1. **单次读取事实和结果提交。** 扩展共享 property read probe 的短期借用键视图，静态 atom 的生命周期绑定已有 PublishedFunctionSnapshot owner；同步路径不再为该键复制独立 root，需要 pending 才提升。真正 owning Object/String 等结果仍在允许分配/清理的 driver 边界由原内核取得一次并一次提交；receiver 已有 borrowed 优化保留。不要把 Object root retain/最终 release 塞入无分配 RunSlots。
2. **受守卫的即时读取。** 在现有 shared storage/RunSlots leaf 上补 genuine Array 的 linked `length`、primitive String 的 UTF16 `len`、canonical String 数组下标和 Arguments 即时 own-index 读取。String.length 使用已有 linked atom，不在每次读取中重新 `intern_property_key("length")`；不创建 boxed String。String key 与 base 必须分别通过 release preflight：当前 Int-key helper 直接 drop key，扩展后仅 Ready 的 String Rc 可以在窗口内释放，最后一个 key owner 必须走下述边界释放。for-in 的局部变量仍持有 key，故该守卫允许此 fixture 的索引读取命中。数组下标复用 canonical 规则，`'01'`、`'-0'`、`'4294967295'` 不能当普通数组索引。Arguments 在同一次 heap 借用中定位当前 own slot/VarRef 并读取原 cell，base owner 全程保活；仅即时值且 release preflight 成功才直接完成，Uninitialized、失效 cell、引用值回原错误或 lookup，不能把 VarRefId 跨释放边界保存；删除/重定义/断开映射/accessor/缺项走共享 lookup，读取当前有效的映射值，保持 arguments 创建/逃逸语义。Array holes、prototype、AutoInit、namespace/typed-array 等沿用精确 fallback。
3. **primitive 最后 owner 的紧凑释放。** `heap/slot_ownership.rs` 已区分 PrimitiveStorage 与可能触发 heap cleanup 的释放。for-in 的最后 String Rc 必须继续在 RunSlots 之外析构；为已选 ReplaceBinding/ReleaseOperand 提供同帧短完成入口：先发布准确 PC/槽状态，结束借用，再按原顺序释放、恢复运行。不能把所有 PrimitiveStorage 直接改为 Ready，不能推迟到循环结束或人工保留历史键。

**写入/复制/删除证据。** prop_create/array_prop_create 各有 320,002/429,002 次 SetProperty exit，但 Query dispatch 均仅 14 次启动成本：普通 Set 已有本地推进。其余仍反复搬运 `SetStep/SetResume`，`start_write_adapted` 即使完成也提前更新 generation。prop_clone 的 CopyResume 经 OwnKeys/Read/Define 协议，48,037 次 Query；prop_delete 同时含 spread 和 Delete，共 592,359 次 Query。array_length_decr 同时含 slice 及反复 length Set，有 1,107,050 次 Query；`SetStep::advance_without_callback` 当前不本地处理 ArrayLength，而原 `ArrayLengthStep` 对非负 Int 早已能直接返回长度。

4. **Set/Define 保留一次选定的状态。** `object/ordinary/set.rs` 的 State/phase 改为 resident 状态，普通 Walk/Receiver/Descriptor/Define 在 `&mut state` 中推进，key/receiver/value 只拥有一次。复用 existing writable-own probe 与新属性定义内核，不重复做已经完成的分类/查找；用短期 borrow 中的选定 slot/flags 直接提交，可观察边界之后必须重新 lookup，不能建立无失效机制的全局 shape cache。`property_write_driver` 已批量 pop 输入，继续复用；优化重复 key 转换/状态搬运和完成时 generation，而不是再实现一次 batch pop。
5. **Array length 本地完成。** 在既有 Set 状态中直接消费 `ArrayLengthStep::Complete`，非 Object 原语用原转换规则本地推进；Object 保留两次可观察 ToNumber 及转换后重新读取 length/writable 的顺序。最终使用 `apply_set_array_length/apply_array_length_descriptor` 的原 dense/sparse truncate 内核，保留不可删除索引导致的部分缩短、恢复 length 和 writable:false 行为。批量 sparse truncate 已存在，不再把它当新算法或把长度直接赋值越过 descriptor 验证。
6. **spread/copy 与 Delete 本地推进。** `builtins/object/copy.rs` 改为一个 resident CopyCursor，保留当前 non-Proxy enumerable snapshot 时点、顺序和后续 live value read；复用同一 key/source/target owner，在本地消化普通 OwnKeys/Enumerable/Read 和现有 fresh-target Define。Proxy/getter 才返回待续状态，不能预读全部值或跳过真实 getter。`start_boolean` 的普通 Delete 用已认证的共享删除内核直接回小结果；strict rejection、键转换及抛错保持原规则。二者 Query identity 仅在真实等待安装时签发。

**机械验收。** guarded length/index fixture 的一般 GetField/GetElement 退出消除；其余 owning property read 的槽认证/结果搬运实减；普通 Set/Define/Copy/Delete/length 的完整状态搬运不再随同步 phase 数量增长，真实 wait 的原身份和预算计数仍正确。profiling 必须同时记 selected effect 消费一次、Query 安装次数、owner copy/release、状态搬运和必要 heap 操作，不能仅移除事件。没有证据表明哈希或 for-in 出现新的平方级算法，因此不重写 HashMap/GC 或承诺此组可追回全部百分比。

**语义验收。** 数据属性/descriptor/strict 拒绝、Array.length 边界与不可删除索引、动态原型/getter/Proxy/AutoInit、String UTF16 长度/索引、Arguments mapped/unmapped/别名/额外参数/define/delete、spread snapshot/live-read/顺序/Symbol/excluded 与部分结果；错误 realm、转换/读写顺序、getter 重入后重新 lookup、最后 owner 和 deferred cleanup、拒绝前无副作用。复用现有 property/slot/copy/length/arguments 测试，针对新增叶路径补动态命中与失效测试。

##### 综合用例、实施顺序与最终验收

七个 V8 用例全部保留独立待办。表中的 R1–R5 对应其已测热点；尤其 Earley-Boyer、RegExp、Raytrace/Splay 含真实 callback、GC 和字符串内核，不能把 Query 总数当“都可删除”的成本。固定 Earley-Boyer 的有效正式 S0 数据可以比较；历史失败的三份 S0 CPU profile 不作归因证据，原始 adaptive Earley/combined 无有效 S0 分数也不能伪造基线。现有材料支持修复共用协议，不支持断言每个综合用例的全部回退已经归因或必然被上述收益覆盖。最终还慢的用例继续保持未完成，按其残余实测定位，不能用调用微基准进步抵销。

1. **先确定共有接口，再并行实现相互独立的部分。** R1 的 callable/activation/wait 契约、R4 的短输入输出事务、R5 的 prepared property effect 先固定；数值、迭代、各内置域可并行实施并复用它们。R2/R3 共享属性内核，禁止各复制一套 JS 语义。保留显式 JS 栈、S09 已成立的 O(1) 预算和共享 closure owner，不倒退为 Rust 递归调用。
2. **一个合并候选，统一验证。** 完成这些机制后，先跑受影响的语义/所有权/回调顺序测试，再仅以新执行核心配置对同一最终源码完成工作区、QuickJS oracle、boundary/GC canary、小栈/host reentry、预算/暂停及 native/Web/WASM 相关既定门禁。阶段测试可随代码变化执行；不为每个局部改动重建五套版本或重跑全部历史矩阵。原始成功记录继续保留，修改后的源码不能借用旧测试结果宣称通过。
3. **性能只测最终候选一轮。** 使用冻结 50+8 固定工作量覆盖全部 37 项，同时保留已改善的普通/全局/闭包调用等控制项；67 项 compile 及已有必要探针也只补最终候选，旧值全从有效记录读取。不重构或重跑 S0/S07/S08/父提交/c9，不恢复已停止的旧版原始 V8 长跑队列。下文 S09 退出条件保留的原始八项/combined 门禁，只运行最终候选各一次，旧基线读取已有有效记录；失败或达到事先记录的时间上限如实保留未完成，不能追加旧版本长跑。profiling-off 构建用于耗时，同源 profiling 构建只提供诊断，不把它们称为两个优化版本。
4. **逐项报告与机械证据绑定。** 最终候选一轮计数/CPU 诊断覆盖本账本；同时绑定源码、二进制、冻结 workload/output 哈希，分别列相对 S0、父提交、c9 的耗时和可用硬件事件。当前诊断不混入正式多轮历史中位数；单轮新值不伪称统计置信区间。窗口认证、状态分配/搬运、真实 Query 与必需内核工作分开计数，三个 legacy bridge 保持零。
5. **S0 是最终目标。** 全部实际回退必须消除；低于 5% 不自动忽略，微基准改善不抵销综合项。删除重复认证、Box 或状态搬运只满足机制验收，尚有真实性能差距、证据不足或误差无法分辨的行继续标为未完成，记录残余原因并继续在本账本内处理。不能“只修新增两项后停止”，不能把剩余回退挪到 S10；也不未经证据扩展到编译器、全局 inline/LTO、帧布局、解析器重写或无关内置。新增残余修复须说明其对应哪一行和哪项当前证据，旧基线不重复测。

本节是唯一的后续修复计划。原 37 项映射与前轮完整结果保留在[历史数据](performance/README.md)；当前以[漏项补齐报告](performance/README.md)和[同源完整数据](performance/README.md)为准。未达标项继续属于同一个 S09 验收，不另立阶段。

##### 历史：漏项补齐后的单轮验收（3860cba3 后，2026-09-15）

以下是此前单轮记录，当前性能结论已由后面的三轮公平复测取代，不混合两批数据。

开工前已提交并推送前轮记录 `3860cba3`。随后补齐 R1 统一借用 callee 分类、R2 已选标准 @@replace 外层本地执行、R5 同次无回调借用内的 Missing/descriptor 事实复用直到最终 append；普通 push/replace 去掉临时 pending 包装。S09.1–S09.3 没有重做，没有扩展到 S10 或全局布局/解析器/HashMap 重写。当前实现尚未提交，以完整数据中的源码清单 SHA-256 绑定。

先完成全部代码、契约复核、针对性语义修正及 10 项同源门禁，再统一只测最终候选一轮；实现期间无性能 benchmark/Profile，没有构建或重跑 S0/S07/S08/base23/c9。58 项执行、67 项编译、原始八项/combined、33 项探针全部有效；43 项成本 Profile 三个 legacy bridge 均为零。默认/stack-vm 工作区共 6,704 项测试通过，另各 1 项显式 oracle 压力测试，749 边界变异全部拒绝；两套 102,037 变体 Test262 向量与历史一致，两套 Web/Node/WASM 门禁通过。

| 项目 | 实现与本轮证据 | 当前残余 |
|---|---|---|
| R1 | 单次借用普通/native/general 分类；复用 sealed metadata，先保留域错误顺序再提升 owner。普通/全局/闭包调用相对 S0 −2.13% / −0.29% / −19.97% | 独立 classification owner retain 仍在机器码；Math.min 仍 +29.34% / S0 |
| R2 | 标准 replace Query 230,035→36，外层 direct wait 115,000→1，本地协议完成 114,999 次，两层 activation 保留 | replace 仍 +23.94% / S0；真实等待成功路有两次 584 B memcpy，记录由 752 B 增至 776 B，Step/Resume/Next 较 c9 仍各增 24 B；不能说搬运已全部消除 |
| R5 | 普通新属性 probe 960,008→320,008，State 320,002→2；数组 probe 1,716,008→1,287,008，selected dense append 429,000 次；底层 unique append 也复用选定缺失事实 | prop_create / array_prop_create 已为 −12.36% / −4.30% / S0；其他属性/综合用例差额仍未关闭 |
| 普通槽提交 | 成功 push 直接把 32 B Value 写最终槽，不再临时 pending take；真实 pending 失败 owner 仍保留 | 其他必需槽操作/primitive 边界成本仍在，不能以空循环 −39.42% / S0 抵销 BigInt/转换回退 |

**当前仍有 31 项未排除的执行正差值：原 37 项残余 30 项，另有 `arguments_strict_read +6.61%`。**原 37 项不高于 S0 的 7 项是 prop_create、array_prop_create、array_slice、array_length_read、arguments_read、array_for_of、string_length。前轮额外 global_read/global_func_call/func_call/float_arith 四项本轮均不高于 S0。此前用户排除的 TypedArray 写入和浮点转字符串仍在完整表中单列；没有扩展排除范围。

残余包括 Array for-in +11.51%、push +12.19%、pop +14.52%、Map string +18.19%、Math.min +29.34%、BigInt64 +34.46%、字符串转浮点 +27.12%、固定 Earley +30.03%、固定 V8 RegExp +41.92%。全部 58 项及 37 项账本逐项值见报告，不只保留这几个示例。新增 strict Arguments 的原因未确认；它不在预先冻结的 43 项 CPU 集合内，本轮没有事后追加 Profile。

**编译有 23 项单次值高于 S0，尚不能关闭或宣称统计显著回退；全部逐项记录。**原始八项成功，combined 在开跑前固定的 600 秒上限内成功：381.84 秒，Score 75.5；前轮独立八项约 404 秒，本轮独立八项 384.17 秒。原始 Earley 本轮 99.5，相对有效 S07 71.8 提高 38.58%；旧 S0 原始 Earley/combined 无有效分数，不伪造对比。

本轮按用户“补齐实现后统一一轮再汇报”的要求结束迭代。**漏项的指定代码已落实，不等于 S09 已达标：31 项执行正差值、23 项编译单次正差值、残余 owner/宽 Step 搬运及未完成的因果归因仍开放。**后续仍使用本节原有验收条件，不转入 S10。

##### 三轮公平复测（2026-09-15，最新性能证据）

用户明确授权只对精确 S0 和当前实现重新各测三轮。2,148 项采样全部完成，无第四轮、无失败补样、无中途代码优化。详见[完整报告](performance/README.md)和[CSV](performance/README.md)；完整 JSON（56 MB）不入 git，保留在本地 `docs/reports/primitive-vm-s09-fair-three-rounds.json` 与原始证据目录。

58 fixed 中 32 项中位耗时高于 S0；28 项三次配对均更慢且范围分离，另外 string_to_int、string_build3、global_func_call、array_slice 四项方向不一致，保留待确认。之前排除的两项本次均更快，故排除前后均为 32。普通调用 -4.77%、闭包 -20.71%；主要残余包括 v8-regexp +41.28%、BigInt64 +37.31%、Math.min +29.72%、string_to_float +26.70%、固定 v8-earley-boyer +26.61%、replace +19.98%。arguments_strict_read 此次 -14.42%，此前单轮正差值未复现，不能说发生了新的优化。

67 compile 有 9 项中位正差值，仅 earley-boyer 三次配对均更高，9 项全部范围重叠；不得把它们直接称为已确认编译回退，也不能直接关闭。当前原始九项三轮均成功，combined Score 中位 76.2；S0 原始 Earley/combined 三轮栈溢出，无有效整体分数比。原始其他七项中六项得分下降，navier-stokes 上升。

全部 58 fixed 的三轮 CPU Profile/硬件计数有效，当前三种 legacy bridge 全部为零。S0 仅支持内存 Profile，不存在后加 VM 成本 schema；174 条原始解析状态经 stdout/退出码/JSON 核验按接口能力修正，缺失计数不是零。实际失败 45 条全部为 S0 栈溢出，完整保留。RSS 普通调用/闭包/固定 Earley 的三轮中位差分别 +19.84%/+19.32%/+1.71%，同样保留为内存成本。

本次完成复测，不是完成全部性能修复。原 37 项及 R1–R5 的历史实现账本保留，当前待办以完整 58 项最新差值为准，不漏掉新增正差值、不把波动当成确定因果。原退出条件与未完成归因继续属于同一次 S09 验收。

##### 三轮复测后的根因归纳与分簇修复计划（2026-09-15）

证据来源为三轮复测 JSON 的 `hardware_counters`（三轮中位）与 `cost_profiles`（同源 profiling 二进制第 1 轮计数；正式耗时不含插桩）。计数与热点份额支持下述机制目标，但每项收益仍须按本节既有纪律单变量消融验证；不据 self 份额或计数宣称因果，也不据此改动结案任何回退行。

**两种病，可用硬件计数区分：**

1. **协议往返次数超过必要下限（纯指令增量）。** `bigint64_arith` 是纯样本：IPC 持平（2.65→2.66），指令 +34.5%。3,201,603 次 ConvertAdd 付出 22,438,715 次 fault-PC 写、14,411,706 次槽认证、30,440,417 次槽搬运、7,225,604 次 `slot_copy.BigIntImmediate`。`math_min` 的 1,105,000 次调用付出 11,050,461 次槽认证（约 10/调用）、14,382,965 次 fault-PC 写（约 13/调用）、18,799,727 次槽搬运、3,315,204 次 PC 发布，另有 1,105,097 次 `run_exit.Environment`（环境读也整程退出 run）与 1,105,015 次方法属性 probe。R4 机械验收自设目标为约 3 次必要认证/运算，现状约为其两倍以上。
2. **驱动器结构破坏流水线（IPC 下降）。** `local_destruct` 是纯样本：指令仅 +0.3%，周期 +20.1%（IPC −16.5%，branch-misses +30.1%）。同病：`string_to_float` IPC −15.7%、`v8-regexp` −14.2%、`math_min` −13.5%、`map_set_int` −11.1%、`array_for_in` −11.1%。调用类 branch-misses 大涨（func_call +78.0%、闭包 +66.8%、global_func_call +62.8%、array_pop +51.2%）表明 `run_frames_with_state` 约 20 臂顺序扫描、`frame_operations::step` 二级扫描与宽 `Step`/`Resume` 按值搬运（真实等待成功路两次 584 B memcpy，见 R2 残余行）的分支/数据依赖结构是共同来源——这三项用例本身已更快，佐证该结构成本是全局的。

已区分协议性成本与实现选择：借用边界（分配/retain-release/回调必须在 `RunSlots` 之外，`stack/window.rs:12`）、原始错误顺序、预算先于副作用、真实等待的 identity/Query 容量为协议性，不动；同一逻辑操作的重复认证（`property_driver.rs:427/:467` 两次 `run_window`；`frame_operations/numeric.rs` 输入/输出各一次外加散落 peek/pop/push）、每次调用重跑 `NativeOperation::for_target` 约 113 臂 match、`reserve_native_argument_depth` 每次借 state 读 `active_frames.len()`（`native.rs:118`）、`publish_selected` 的 `clone_set_key/value/receiver`（`ordinary/set.rs:891`）为实现选择，均可消除。

**四条共同机制原则（均为既有模式的推广，不新设特例）：**

- **P1 一次借用一个事务。** 一个逻辑操作 = 输入事务（域检查+分类+取 owner，1 次认证）→ 借用外风险区（分配/解析/错误物化）→ 输出事务（提交+一次打包 PC 发布，1 次认证）。`with_linked_own_read` 为范本；owning 值经 `&mut Option` pending 模式传出借用（`push_pending` 同款），消除第二次 `run_window`。
- **P2 发布期事实随代码走。** sealed metadata / `PublishedFunctionSnapshot` 携带 native 的 `NativeOperation` kind、静态键 atom（`linked_field_atom` 已有）、fusion span；运行时零重推导。发布期事实不可变；运行期缓存另按其依赖事实维护有效性和所有权。
- **P3 驻留状态 + 窄效果枚举。** `State` 驻留、`&mut` 推进，阶段间只传窄 action；宽 `Step`/`Resume`/`Next` 仅在真实等待发布时构造一次。`MutationAction`/`SelectedSet`/`IteratorAction` 三处已落地，推广为全家族规范。
- **P4 run 内受守卫叶子。** 按 `array_immediate_read` 模式扩展 run 可原位完成的操作集（环境 Direct/immediate captured 读、Array/String length、for-in fast-array 推进、`ReplaceBinding`/`ReleaseOperand` 原地处理）；每叶子显式 guard，不命中原路退驱动器，语义路径不变。

**独立消融候选（需先审计，不与其他项捆绑）：惰性 fault-PC。** `run_frame_fault_pc_write` 在多用例中为第一大计数。借用内不发生回调/GC，理论上 run 可持局部 PC，仅在退出点与抛错前写回。动手前必须枚举 run 借用期间全部 fault-PC 读取方（异常物化、backtrace、诊断帧、溢出恢复、profiling），审计不通过则放弃，不得以性能理由削弱可观察性。

**分簇计划（编号供残余修复引用；验收计数一律用既有 profiling 事件）：**

- **簇 A 转换完成协议**（BigInt64 +37.31%、string_to_float +26.70%、int_to_string +12.01%、string_build 系）。输入借用已是单事务，残余在输出与频次。A1：输出事务化，一次认证+一次打包 PC 发布（现融合分支发布 resume/fault/active 三处）。A2：`ready::run` 内驻留转换循环，ConvertAdd 完成后直接重入 run，不回 `run_frames_with_state`（普通 Call/Return resident 同款）。A3：为 BigInt/String 加法链补 fusion span，压 `slot_copy.BigIntImmediate`。借用边界：String/BigInt 分配在短 RunSlots 借用结束后进行；run 函数可以保持驻留，具体完成协议见 N1。验收：每转换认证 ≤2、PC 发布 ≤1 组。
- **簇 B native 同步调用**（Math.min +29.72%、Map/WeakMap +14~18%、depth-native +15~19%）。`begin_synchronous` 已零 Query/零 argv 分配，残余为逐项费用。B1：P2 落地，sealed metadata 携带 operation kind，运行时 `for_target` 调用数归零。B2：`reserve_native_argument_depth` 改增量记账（`installed_wait_depth` 先例）；`identity_completion` 与 generation 推进移入真实等待分支（同 R4.4 遗留）。B3：环境读叶子（P4），消除 `run_exit.Environment` 每调用一次的整程退出。B4：静态键方法取用+调用合并为一次借用内 probe+分类+安装；复用查找事实时验证其有效性。验收：math_min 认证 ≤4/调用、run 退出 ≤1/调用，depth-native 差 <5%。
- **簇 C 属性读写驱动**（richards +14.97%、raytrace +14.88%、deltablue +11.27%、splay +10.57%、固定 Earley +26.61%、crypto +9.79%）。C1：读输出单事务，owning 键 pending 传出，`complete_read` 合并为一次认证。C2：R5.4 落实，`SetResume` 驻留 `&mut` 推进，`publish_selected` 及 `clone_set_*` 仅在真实等待发生。C3：R5.5 落实，`advance_without_callback` 本地消费 `ArrayLengthStep::Complete`（`array_length_decr` 现付 1,107,050 次 Query）。C4：R5.2 落实，补 Array length/String length/Arguments own-index 即时读叶子。验收：`set_owner_clone.*` ≈ 真实等待数。
- **簇 D 迭代协议**（local_destruct +18.65%、for-in +8.22%）。D1：R5.3 落实，`ReplaceBinding`/`ReleaseOperand` 在 run 内借用结束后原地处理再继续（`frame_operations/direct.rs` 逻辑前移；for-in 现付 1,352,500 次 String owner 退出）。D2：for-in fast-array 推进叶子化，`ForInResume::Phase` 驻留帧内（现 2,123,425 次本地 step + 1,379,550 次 ForIn 退出）。D3：非 Array 迭代器的 `PendingIterator` Box（312 B）延迟到真实等待（Array 路径 `iterator_driver.rs:226` 已做）。验收：for-in 每迭代 run 退出 ≤1，local_destruct IPC 回升。
- **簇 E 正则外围**（v8-regexp +41.28%、replace +19.98%、regexp_ascii/utf16 +11%）。执行器两边相同，回退在外围：整用例 6,981,639 次 query_dispatch、7,284,996 + 3,634,260 次 set owner 克隆、2,531,208 次 dispatch_write.define。E1：直接受益于 C2/C3，先落 C 再复测。E2：match 结果对象/数组按发布期已知键序批量 define，复用 R5 prepared property effect。E3 归因先行：解析多用例 5~10% 的未解析地址 `0x18f413`（原始 perf.data 在本地证据目录）；核对 query_dispatch 的操作构成后再定 E2 覆盖面。`Utf16Units::next` 为两边共有成本，不属于本回退账本，另行提案。验收：query_dispatch 与 set_owner_clone 各降一个数量级。
- **簇 F 数组变异**（push +10.57%、pop +11.48%、length_decr +6.63%、prop_delete +10.04%）。F1：C3 落地后 `local_set_result` 覆盖 ArrayLength 分支，无 proxy/getter 时 push/pop 等待迁移与 Query 归零（现 push 268,002 次迁移 / 1,072,014 次 Query，每 push 2 次 `set_owner_clone.ObjectRef`）。F2：dense 快路三 probe 合一，读 length+写元素+写 length 为单借用复合 effect（`SetProbe::Stored` 已证 dense 写免 continuation root）。F3：数字索引键不经 JsString→intern（push 第一热点为 `intern_property_key_js_string`）。验收：四项差 <5%。
- **簇 G RSS**（普通调用/闭包 +19.84%/+19.32%，约 2 MiB）。resident 槽/帧预分配成本，换得 width-256 −90% 与深栈全通过。初始容量降档+高水位增长；只记账，排最后，不以执行更快抵销。

**实施顺序：**第 1 步事务原语与打包 PC 发布（P1 基建，C1 首个使用者）→ 第 2 步簇 B → 第 3 步簇 C → 第 4 步簇 A → 第 5 步 F、D、E（E 依赖 C 后复测）；惰性 fault-PC 独立审计与消融；G 记账。四项三轮方向不一致用例（string_to_int、string_build3、global_func_call、array_slice）暂不投入。本计划不改变本节任何验收条件与证据纪律；每条目完成时须回填对应计数变化与 A/B 结果，未达标行继续保持未完成。

##### 本轮分簇修复实现与最终验收（2026-09-15）

本轮新核心六项同源正确性门禁已通过（3,546 项工作区、独立 oracle 压力、749 canary、Test262 全向量一致及 Web/Node/WASM），代码已以 `ade0f559` 提交推送，随后唯一一轮 358 项性能/诊断采样全部有效。当前工作按用户新指令执行：先提交推送计划 `f27088a3`，补齐全部适用分簇实现，完成同源正确性门禁后提交推送代码，再仅测最终代码一轮。旧 S0 复用三轮记录，不重跑；本指令取代上文局部消融/中途复测安排。可选惰性 fault-PC 未混入组合，也不把它标为完成。

[分簇实现与计划核对](performance/README.md)逐项记录实际改动、已有实现和不成立的假设。P1/A1/C1 共享一次认证事务，A3 局部加法 span，B1–B4 native 事实/同步完成/global own/字面量方法调用，C/F 受守卫 Set 与 dense 复合操作，D1–D3 owner/for-in/iterator 驻留均已接入。A2/C4、C3/E2/F3 的部分机制原本已有，未重复重做。G 的“大初始预分配”源码假设不成立，保留 RSS 成本而不进行无依据的容量修改。

上文旧计划的 push 1,072,014 和 length_decr 1,107,050 Query 数不是当前三轮候选计数；当前分别为 10 和 2,494。硬件计数和热点只能支持调查方向，不能独自证明全部因果。最新性能结果见[最终单轮报告](performance/README.md)：58 fixed 中 23 项单次高于 S0 三轮中位数，67 compile 中 30 项，另有 12 项可比探针及 3 项 RSS 成本未追回。九项原有 fixed 正差值本轮不高于 S0；没有新增 fixed 正差值。B4 仅覆盖字面量参数，实际 Math.min(i,500) 不命中，认证仍约 8/调用而非目标 ≤4；BigInt active-PC 发布增加，V8 RegExp Query 仅下降约 36.3%，相关机械目标未达。**S09 尚未完成，也不能声称全部计划实现/覆盖已经补齐。**用户要求只测最终一轮，本轮不追加修改或补跑；下文原退出条件仍属同一次验收。

##### G 之外遗漏补齐的最终单轮验收（2026-09-15）

本次用户要求覆盖除 G 外的遗漏，实现完全部代码后只测最终代码一轮，再更新状态提交推送。该顺序取代旧节“先提交实现再测量”；既有 S0 三轮数据只读复用，既有 S09.1–S09.3 不重复实施。

- B4：直接 GetLocal/GetLocalCheck/GetArg 参数与字面量进入受守卫方法 span，调用深度/分类/域检查/native argv 转移共享一次认证事务；实际 Math.min(i,500) 命中有语义断言。保留 TDZ/captured/getter/Proxy/effectful 参数与 tail-call 通用路径；复用跨回调的查找事实时重新验证相关依赖。
- A1：融合转换输出在最终成功位置提交 fault/resume；原语旧值释放删除冗余 active-PC 发布，Object/Symbol 与错误边界保留 canonical PC。
- 惰性 fault-PC：完成全部观察读取边界审计并实现局部 PC guard，run 退出/错误/unwind 物化，可观察 owner 释放前显式发布。仅本轮组合验证，不伪造独立消融收益。
- E2：补齐 global @@match/@@split 私有输出数组逐项 generic Define 的实际遗漏，复用连续构造内核，保持创建/追加时机和 realm/回调语义。
- E3：完成新增穷尽 Query Step、直接 owner、Define helper stage 的分区诊断，所有 58 个样本求和校验；当前组成、旧数据无法恢复的调用链边界见报告。
- G：不实施；RSS 成本继续列出，不抵销执行回退，也不宣称修复。

完整门禁包含工作区 3551 通过、独立 oracle 压力、最新边界扫描、688 Rust 文件布局、完整 Test262 结果向量与既有一致、Web/Node/WASM。749 checker canary 的历史成功回执仅复用相同 checker 哈希，生产源码已重新扫描，不冒充本轮重跑全部 canary。

最终 358/358 项有效，58 项三个 legacy bridge 均为零。固定执行 33 项、编译 12 项、可比探针 12 项仍为正差值；单轮不声称统计显著。[完整报告](performance/README.md)逐项列出相对 S0、相对 ade0f559 及机械目标结果。代码遗漏补齐不等于全部性能目标达到；**S09 仍未完成，原退出条件继续生效**。

##### 非 Number 原语数值运算驻留修复（2026-09-15，起点 7dc70fbe）

用户提供的对照实验以 ade0f559 为被测源码/二进制：Number subtraction 快于 S0，而 bool（无解析）及 String subtraction 均慢，支持把缺失驻留路径列为修复对象。当前仓库已是干净的 7dc70fbe，包含此前 A1/B4/PC/regexp 补齐；“15 个未提交源码文件”是旧检查时点，不是当前状态。当前源码仍在 Number 快路径 miss 后通过 RunExit::Numeric 退出 run，再由 ready 的 try_complete_primitive 新建事务并重入，根因路径仍在。用户给出的绝对 cycles 分组和 stall 归因作为外部实验结论记录；本轮不重跑这些对照，不把 self/movups 单独当成 stall 的硬件证明。

“ready 驻留”和“run 栈帧驻留”必须区分：ConvertAdd 的 ready loop 也调用了返回后的 run，不能用已有 ready 循环冒充本次完成。要消除的是非 Object 原语运算的 run 返回、第二次认证和大返回值跨 driver 搬运。

实施条目（按序完成全部代码后统一测量）：

1. **N1 同一 run 内的原语完成边界。** 保留 Number 原位运算；其余 NumericKind 的原语算术（Sub/Mul/Div/Mod/Pow、移位/位运算，以及 Neg/BitNot/Inc/Dec/PostInc/PostDec）在原 run 栈帧内完成。预检原操作数，Object、比较/抽象相等和坏槽仍走 canonical 路径；Add/Plus 的既有 conversion identity 协议不在此处重写。取出原语 owner 后结束短 RunSlots，保持原 FrameTransaction 和 run 栈帧；在借用外调用共享 primitive_output，提交后重开同一事务的 RunSlots 并继续分派。不得在 RunSlots 内解析、分配、释放最终 owner 或执行 JS；不复制另一套算术语义。
2. **N2 窄调用与精确观察边界。** 共享 helper 在原事务中按 RHS→LHS 消费，保留 postfix previous/value 输出顺序。仅跨小 helper 调用，不返回 ready、重新认证或重新构造 run prologue。解析/BigInt 分配及 Symbol/混合数值抛错前物化 canonical PC；错误在正确 realm 物化并交还正常异常处理，不重放已消费输入。必要 PC 发布不冒充可删除开销，Object ToPrimitive、GC/释放、异常行号、容量/域错误与恢复需反例测试。
3. **N3 共享 string_to_number ASCII 路径（额外收益）。** 对可证明 ASCII 的文本直接复用同一 Infinity/radix/decimal 语法解析器，避免 UTF16 collect 再 from_utf16 的双分配；Unicode 空白、非法 surrogate、非 ASCII 及所有 grammar 边界保留共享 fallback。这项 S0 也可获益的优化与退出协议回退的根因修复分别记录收益。
4. **N4 可证伪验证。** 小型语义/路径断言覆盖 Number/bool/String/BigInt/Symbol/Object、Sub/Mul/Div/Mod/Pow/位运算、负零/NaN/Infinity、Unicode/radix、postfix 输出顺序与异常恢复；bool/String subtraction、String BitOr 主体的 run_exit.Numeric 应归零，驻留命中次数与操作数一致，槽认证不再随这些迭代线性增长。正常错误/回调需要的退出不设零目标。完整新核心 workspace/oracle/boundary/Test262/Web/WASM 门禁后冻结源码。
5. **N5 最终单轮。** 仅最终新核心跑一次既有 358 项矩阵（58 fixed、67 compile、9 原始 V8、33 probe、14 memory、58×3 诊断、3 RSS），S0 复用旧三轮，直接前版 7dc70fbe 复用上一轮。新增事件仅做诊断，正式计时无插桩。报告全部差值与机制命中/认证/PC/Query/hardware/self，不能以源码结构或计数下降结案剩余吞吐回退；不重跑 S0、不做中途 benchmark/Profile、不补跑挑结果。最后更新本节状态、提交并推送。

当前状态：N1–N3 实现完成，N4 完整门禁通过（工作区 3558 通过、Test262 全向量不变及 Web/Node/WASM），N5 最终单轮 358/358 有效，58 项三种 legacy bridge 全零。string_to_float Numeric 退出 0、run 驻留完成 500,000、槽认证 445；相对 S0 耗时 -9.20%。所有剩余差值与硬件计数见[最终报告](performance/README.md)。G 未纳入，未追加第二轮或独立版本；**S09 仍未达到原退出条件**。

**执行与验收顺序：**先固定机制和可证伪指标，定向验证后合并候选，再对同一最终源码统一运行完整门禁；发现新失败才重新打开相关实现。阶段目标不是“所有计数归零”：必要的参数校验/初始化、实际创建捕获和最终释放仍按真实工作量计费；普通调用的额外记账必须与祖先数 D、整个环境宽度 C 无关。

**S09 退出条件：**补充 S09/本轮起点 `b36ad884`，并按共同口径同时给出 S09/独立 S08、S09/S07、S09/PR19。完整固定 50+8、原始八项/combined、67 项 compile、调用/内存/暂停证据齐全。原始旧 Earley-Boyer/combined 无有效基线，要求新核心默认预算持续成功，并与 S07 的有效分数比较。确认存在的吞吐/编译回退逐项修复或保持阶段未完成，不能靠 O(1) 理论、分配平台、局部 profile 百分比或可选实验的预期收益结案。#7 的分配/初始化/引用成本、#9 的最终码/分派和 #10 的独立 PC 结论同时交付；补齐 S08 收口记录保留的最终同源验证缺口，完整语义、零桥覆盖、有限/无限递归、小栈/宿主重入、native/Web/WASM 门禁通过后才进入 S10。

### S10 — `refactor(vm): finish validation and retire the previous execution path`

**改期（2026-09-15，用户决定）：本节顺延为 S13。**在其之前插入三个结构性修复阶段作为新的 S10（惰性帧协议三件套）、S11（Step/Resume 瘦身与两级状态机合并）、S12（属性读内联缓存驻留），病历记录、设计与验收见[惰性帧计划](primitive-vm-lazy-frames-plan.md)。本节全部前置条件、交付与验收内容不变，仅编号与顺序后移。

前置条件：S08/S09 的联合优化与回退修复达到上述退出条件。S10 不接收未归因回退作为默认切换后的待办。

按顺序完成同一提交的最终交付：

1. 新核心配置运行相关 QuickJS oracle、完整回归/Test262、native/Web/WASM、原始 V8 与固定 50+8，核对编译/内存/调用/暂停成本及全部能力清单。
2. 验收通过后切换全部默认入口，删除旧 VmHost、重复 activation/帧投影、旧驱动与迁移桥，清理通配导出和无用实验。
3. 在切换和删除后的最终源码上重新完成相应检查及正式发布验收，记录源码/构建身份。固定工作量交错至少 5 轮，敏感项 10 轮；诊断与正式计时分开。
4. 更新架构说明、源码契约、实际 commit 导览和 #1/#5/#7/#9/#10 的真实结果。完成新增 Number、转换顺序、异常/恢复、binding/eval 四种维护演练。

**验收：**默认预算原始 Earley-Boyer 独立及组合、小栈、有限/无限递归和重入通过；新核心完全覆盖且旧路径退出。不增加 skip，不按测试名 fallback，不改冻结预期掩盖回归，不用旧 receipt 替代本轮结果。#16 的其余优化与 #20 的 Fiber 调度不随此 PR 自动完成。

## 5. 审查规则

正式 PR 以这 **10 个完整提交单元**组织。文档、诊断、测试和局部结构调整并入其所属单元；开发过程中的临时提交在整理 PR 时归并，避免把每个 helper 或修正重新扩成独立的计划 commit。阶段验收可以发生在一个提交内部，验收顺序与证据仍需记录。

代码结构任务按[实施设计第 15 节](primitive-vm-implementation-plan.md#15-代码结构的独立改进清单)验收；接口、类型、算法和反例在同一提交可审查。较大提交按本文件的领域条目组织审查说明，保留逐调用点账本。文件移动与语义改变在 diff/说明中清楚标识，不引入只有占位抽象、没有消费者的提交。

每次增加恢复路径同时完成 roots、异常和释放责任；构建、测试、正式计时与诊断分阶段串行。架构、结构和 #16 问题验收同时满足后才完成本 PR。

## 新 S10–S12 执行账本（2026-09-15）

按[惰性帧计划](primitive-vm-lazy-frames-plan.md)分别提交 S10 惰性帧、S11 窄状态机、S12 属性位置 IC。用户最新授权覆盖旧阶段退出顺序与三轮测量：三阶段代码全部完成、额外覆盖 review 补齐后，最终代码统一单轮 benchmark/Profile，S0 复用旧数据。S10 认证缓存仅保留带发布失效机制的不可变事实；S12 已实现带布局/原型失效机制的位置事实缓存，当前实现读取实时属性值。S09 未达到性能退出条件的记录保留，不冒充已通过；旧退役阶段顺延 S13。

实施提交：S10 `77153244`；S11 `e0494120`（已并入额外 review 补漏）；S12 与本轮最终报告为第三个 commit。最终被测对象是包含 S12 的冻结工作树，基底 `e0494120` 不代表被测源码；逐文件哈希见报告 JSON，提交时核对相同源码。

| 阶段 | 实施与覆盖状态 | 最终验收结果 |
| --- | --- | --- |
| S10.1–S10.4 / AU-1–AU-4 | 已实现认证事实缓存、冷字段、惰性活动帧物化与可观察路径；普通 Call/Return 不做活动注册 | 普通调用计数与耗时明显改善；global_func_call / func_call / func_closure_call 相对 S0 分别 −16.38% / −21.59% / −33.05%。不能据此宣称所有 depth-native/getter/proxy/mixed 目标均达成 |
| S11.2 / 尺寸与冷布局 | 已实现单层冷分派、驻留中央 Step、窄 Resume/Frame 与全部领域请求包、冷诊断外提；131 项生产 domain Step 尺寸约束通过 | 主 RunExit 选择的机器码为一次跳表；不等于每条冷路径只有一次条件分支。local_destruct 降至相对 S0 +16.59%，array_for_of 为 −3.47%；Map/WeakMap 仍 +15.92%～+24.69%，性能目标未全面达成 |
| S12.2 / AU-5 | GetField/GetField2 定长位置侧表、own/prototype 失效守卫、两次 miss 退化、全 Value 保活与 B4 汇合均实现；当前缓存保存位置事实 | IC 命中路径实际驻留。Richards 为 −7.35%；Earley-Boyer / RayTrace 仍 +13.26% / +6.26%。读缓存实施完成不代表全部属性/V8 性能目标已满足 |
| 联合正确性 | 最后源码变更后库测试 2503/2503；最终工作区 3598 通过，独立 oracle 压力通过，源码布局 697 文件通过 | Test262 全向量一致：79982/80032 可运行变体通过，原有 50 失败不变（102037 总变体）；Web/Node/WASM 15 示例通过 |
| 额外覆盖 review | 找出并补齐“一次性交接宽包被误排除”“尺寸断言覆盖不全”两处遗漏；补齐后重新核对 | [逐项覆盖记录](performance/README.md)：本次范围内没有已知实施遗漏；不以此代替性能验收 |
| 唯一最终性能轮 | 403/403 有效，包含全部 fixed/compile/original/probe/memory/RSS、58 fixed 与 15 getter/proxy/mixed 探针的 stat/record/cost | 不重跑旧 S0，不进行中途候选性能测试，不补样替换。25 fixed 正差值中 21 项超过 +5%；单次对历史三轮不提供统计显著性或单阶段因果结论 |

直接前版 dce0b6ea 的 31 项 fixed 正差值，本轮有 7 项转为非正：array_for_in、array_for_of、func_call、global_func_call、regexp_replace、v8-deltablue、v8-richards；Math.min 从 −0.80% 变为 +9.70%，所以净减少至 25 项。Map 字符串/整数、WeakMap、int_to_string、BigInt64 等相对直接前版也有恶化，完整差值必须保留，不能只报改善项。JS 编译、原始 Score、调用探针与 RSS 各自使用对应表，不能与 fixed 同名行混用。

边界验收记录：完整反例矩阵运行到末尾，唯一失败为 `stage3b-function-realm-fallback` 的旧字段锚点。仅修正测试脚本两个 `self`→`self.0` 路径，定向负例按预期规则拒绝，当前源码扫描通过。聚合回执保留完整工具原 exit 1 与修正复验，不冒称第二次完整工具 exit 0；逐文件证明所有 Rust、语义测试、检查器规则未变，故不重跑已通过的语义门禁。

S10 证据：[认证/root 审计](performance/README.md)、[字段审计](performance/README.md)、[观察协议审计](performance/README.md)。发布代号复用不可变字节码 arena 的 index/generation；无可变重发布入口。无捕获事实来自同一发布快照，冷观察点按需物化。

S11 证据：[冷分派与转换审计](performance/README.md)、[协议布局与交接清单](performance/README.md)、[领域循环审计](performance/README.md)。Frame 56 B、Resume 32 B、ConversionTask 8 B、PendingIterator 8 B；中央 Step 168 B 驻留借用推进。无需等待的 Value 完成分支保留 inline，最低 40 B，不为凑 32 B 引入新的结果分配。

S12 证据：[布局失效审计](performance/README.md)。SetProperty own-data 写 IC 是原计划明确的二期，GetElement/Proxy 不进入本次 IC；G 与 S13 不在本轮范围。

最终数据与判断：[完整联合报告](performance/README.md)、[完整 JSON](performance/README.md)、[CSV](performance/README.md)。S10–S12 实施范围已交付，性能未达到的退出条件仍开放；不再追加本轮代码优化或第二轮性能测量。

## S14–S20 修复阶段（2026-09-15 立项）

针对最终报告全部超 5% 的正差值（fixed 21 项、调用探针 12 项、原始 Score 3 项、RSS 2 项、编译 5 项），按 [S14–S20 修复计划](primitive-vm-s14-s20-recovery-plan.md)追加七个提交单元。该计划已将全部残余差值下钻到实现层根因（R1 run 驻留谱系缺口、R2 字符串/转换内核、R3 属性 IC 覆盖缺口、R4 shape/atom/哈希 O(n²)、R5 native 调用外围、R6 heap/GC 结构、R7 regexp 外围、R8 属性驱动被调帧急切物化与 proxy get 陷阱外围、R9 RSS 映像差、R10 测量伪影），每条根因带 file:line 与机械计数或测量级证据。原 S13（退役旧路径）顺延到本序列之后执行，并**承接 RSS 2 项的验收**（R9：常数增量为旧/新 VM 双二进制 `.text` 映像差，非堆分配）；编译 5 项按 R10 判定为单轮测量伪影，不设提交单元，在 S14 验收轮以锁频三轮中位复测关闭。

| 提交 | 建议信息 | 主治根因 | 关键验收计数 |
| --- | --- | --- | --- |
| S14 | `perf(value): eliminate redundant conversion copies and enable in-place string append` | R2 | int_to_string/string_build 的 `try_from_utf8`/`from_validated_utf16` self 归零 |
| S15 | `perf(vm): complete run residency for add, equality and definition operations` | R1 | `run_exit.ConvertAdd`/`StrictEquality`/`LogicalNot` 降至启动量级；逐退出 `runtime_pc_publication` 消失 |
| S16 | `perf(vm): extend property caches to exotic receivers, writes and globals` | R3 | splay GetField 退出 808k→<5 万；crypto GetElement 1.5M/SetProperty 869k→启动量级 |
| S17 | `perf(object): add shape transition table, cheap hashing and pinned atoms` | R4 | prop_clone `copy_owner_clone.PropertyKey` 320k→0；SipHash self <3%；slice/解构 intern 归零 |
| S18 | `perf(vm): extend lazy frames to native, getter and proxy trap calls` | R5+R8 | math_min `native.prepare` 179ms→<40ms；depth-getter-0 每迭代槽认证 12→≤4；depth-proxy-0 每读 Box 分配 4→0（稳态）；探针 12 项相对 S0 转负或 ≤+2% |
| S19 | `perf(regexp): reuse input buffers and precompute result layout` | R7 | v8-regexp `Utf16Units::next` 28%→<5%；稳态 `validate_program` 归零 |
| S20 | `perf(heap): box context payloads and reduce per-value bookkeeping` | R6 | ArenaSlot 904B→约 100B；memcpy/`release_raw_no_drain` self 收敛（RSS 不在本单元，见 S13/R9） |

每个提交单元的逐工序步骤（文件/函数级改动、测试与验证命令、内部执行顺序）见[执行计划](primitive-vm-s14-s20-execution-plan.md)。每个提交单元沿用本文的验收纪律：机械计数先于耗时结论、三轮公平复测（S0 复用旧三轮）、单变量归因、全部语义门禁；实现期间不做中途 benchmark/Profile。新增测量口径约束（修复计划 §4）：对比一律三轮中位对三轮中位，正式轮要求 performance governor 且记录 loadavg/频率，超限样本作废——单轮对历史中位曾制造 v8-deltablue 编译 +95.89% 的假回退。设计文档先行修订项（owned-fusion 的 ConvertAdd 条款、写 IC 失效语义、shape 转移表字典化边界、S18 的属性驱动被调帧惰性观察边界）在对应阶段动代码前完成。第 5 节的"10 个完整提交单元"描述原 PR 组织，本序列作为后续提交单元延续同一审查规则。
