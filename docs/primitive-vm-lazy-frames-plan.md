# Primitive VM 新 S10–S12 计划：惰性帧协议、窄状态机与属性读内联缓存

**阶段指引的适用范围：**以下缓存种类、IC 覆盖范围、退化策略和代码布局手段是 S10–S12 的阶段方案及实现记录，不构成后续优化的项目级限制。后续方案依据语义、有效性、所有权和性能证据选择。

2026-09-15 用户决定：原 [S10（退役旧执行路径）](primitive-vm-commit-plan.md#s10--refactorvm-finish-validation-and-retire-the-previous-execution-path)顺延为 S13；在它之前插入三个结构性修复阶段，作为新的 S10、S11、S12。本阶段**先设计后实现，设计与实现期间不做 benchmark/Profile**；测量只发生在各阶段验收点，沿用 S09 的三轮公平复测与单变量归因纪律。S09 的未完成状态与退出条件不因本计划改变。

**本次执行授权（覆盖下文旧测量顺序）：**用户已要求分别用三个 commit 完成新 S10、S11、S12，允许并行独立工作，不要求向后兼容。S12 实施带精确失效机制的属性位置缓存。全部代码结束后额外逐项 review，仅核对 S10–S12 实现覆盖，发现遗漏必须补齐；之后只对最终新核心统一进行一轮 benchmark/Profile，复用旧 S0，包含 getter/proxy/mixed 探针级 Profile。期间不做 benchmark/Profile，不运行旧核心。阶段实现与语义检查按依赖推进，G 与 S13 不纳入。

设计参照：CPython 3.11 的 [inlined Python function calls](https://docs.python.org/3/whatsnew/3.11.html#inlined-python-function-calls) 与 [cheaper lazy Python frames](https://docs.python.org/3/whatsnew/3.11.html#cheaper-lazy-python-frames) 是成对改动——先把帧做便宜（lazy frames 贡献 3–7%），内联调用（1–3%）才能兑现。本 PR 的 stack-vm 迁移等价于前者的 inlined calls（递归原生栈 → 扁平 run 循环 + 显式帧栈；S0 在 depth 512/2048 探针全部栈溢出、新核心全部通过即其证据），但帧仍是"急切且宽"的。新 S10 补齐 lazy frames 对应物；S11、S12 分别处理状态机结构与属性读驻留。

## 1. 病历记录（诊断固化，2026-09-15）

数据口径：最新一轮 `docs/reports/primitive-vm-s09-numeric-resident.md`（当前单轮 vs S0 三轮中位）；硬件计数与热点来自三轮公平复测 JSON。此节固化诊断结论，后续阶段引用时不再重新推导。

### 1.1 两种病（总框架，见 commit-plan.md「三轮复测后的根因归纳」）

| 病型 | 硬件特征 | 纯样本 | 结构来源 |
|---|---|---|---|
| **病 1：协议往返超必要下限** | 指令数净增，IPC 持平 | bigint64_arith（已修复；曾 instr +34.5%，每 ConvertAdd 多次 fault-PC 写/槽认证/槽搬运） | 每逻辑操作重复认证、逐次 PC 发布、退出 run 再重入 |
| **病 2：驱动器结构破坏流水线** | 指令持平或更少，IPC 大跌、branch-miss 大涨 | local_destruct（instr +0.3%，cycles +20.1%，IPC −16.5%，branch-miss +30.1%） | `run_frames_with_state` 约 20 臂顺序扫描 + `frame_operations::step` 二级扫描 + 宽 `Step`/`Resume` 按值搬运（成功路两次 584 B memcpy）+ `.text` +46% 布局压力 |

### 1.2 分簇病历表

| 簇 | 代表用例（最新 vs S0） | 病型 | 机制诊断 | 状态 |
|---|---|---|---|---|
| C 属性读写驱动 | earley-boyer +29.40%、richards +19.03%、raytrace +15.87%、crypto +12.45%、splay +10.96%、deltablue +9.85%、prop_delete +12.56%、prop_clone +10.50% | 1+2 混合 | 静态键读退出 run 进 `property_driver::read_progress`（linked-read 选择、receiver 保留、publish_read_result 逐次 PC 发布、fusion 预检）；热点 `ordinary_read_probe_atom` 8–12% + `read_progress` 4–7% 取代 S0 单次 `internal_get_or_missing`。已排除 SipHash（两侧同在） | 未达标，S12 主标的 |
| B native 同步调用 | weak_map_set +15.49%、map_set_int +15.01%、map_delete +14.80%、map_set_string +10.25%、depth-native +7.75~9.03% | 2 为主 | B1–B4 后计数已降但 cycles 反升（IPC −9~11%）；分类/操作数搬运路径残余 `retain_object`；"计数降周期不降"无专门因果实验 | 未闭环 |
| D 迭代协议 | local_destruct +26.19%、array_for_of +5.59% | 2 纯样本 | 迭代 next 走 `start_array_next_without_pending` + 继续体；PendingIterator 前置 Box 已修（89.7 万→1.7 万），残余差额官方"尚未完成因果分解" | 未归因，S11 主标的 |
| E 正则外围 | regexp_utf16 +11.87%、regexp_ascii +10.10%、v8-regexp +9.99%、regexp_replace +8.00% | 1 为主 | 执行器两侧相同；回退在外围 Query dispatch/owner 克隆；E1 已修克隆，Query 仅降 36.26%，一个数量级目标未达；E2 被证伪 | 未达标 |
| A 转换完成 | int_to_string +7.14%、string_build_large2 +6.20% | 1 | `conversion_driver::complete_primitives` self 21–24% + `SlotStore::push_current`；string_to_float/int/bigint64 已修复 | 残余两项未闭合 |
| F 数组变异 | array_slice +20.16% | 1+2 | `SliceResume::collect` ~10.8% + 逐索引 `intern_property_key_js_string` ~8%；按值 SliceResume；轨迹反复（曾 −4.64%），最近两轮恶化无专门 Profile | 未归因反弹 |
| 调用/帧 | global_func_call +11.67%（instr −9.5%、branch-miss +62.8%）、depth-proxy +48~57%、depth-mixed +21~30%、depth-getter +9~19% | 2 纯样本 | `OrdinaryCall::install` 9.7% + `driver::ordinary::enter` 7.5% + `authenticate` 5.3%；深度曲线已水平（B2），残余为每层固定费用；depth-getter/proxy/mixed 零专门 Profile | 未归因，S10 主标的 |
| G RSS | func_call/func_closure_call +21.4~21.6%（约 +2.3 MiB） | — | "预分配"假设被源码审计推翻；`.text` 5.14→7.89 MB（+46%）是唯一记录事实，因果未建立 | 用户决定不实施 |

### 1.3 明确的"根因未确认"清单

- local_destruct 剩余差额的因果分解；array_for_of、array_slice 的近期反弹。
- Map/WeakMap 族"计数降、周期不降"的物理成因。
- depth-getter/proxy/mixed 探针（无任何探针级 Profile）。
- 病 2 的物理层归因（I-cache / 分支预测 / 宽结构 memcpy 的配对实验从未做过）。
- compile array_update +157.79% 单轮异常；compile 普遍正差值疑为单轮噪声（三轮复测时 9/67 且无范围分离）。
- 本轮 dce0b6ea 相对前版 7dc70fbe 的新增单轮差值（prop_update +12.4% 等），疑为 2200 行 `run` 插入分支后的布局扰动，单轮不定论。

## 2. 现行调用帧栈协议的逐项成本盘点

每次普通 JS→JS 调用（`driver/ready.rs:242 enter_call` → `driver/ordinary.rs:30 enter_selected`）付出：

**选择与认证（每调用重做，结果本可复用）**
1. `DirectSelection::select`（`vm/call/ordinary.rs:66`）：`state.borrow()` + `Ref::filter_map` + payload 匹配 + closure 计数校验 + `function_bytecode` 查表。
2. `OrdinarySelection::authenticate`（`vm/call/ordinary.rs:254`）：`Rc::clone(closure)` + `function.clone()` + `FunctionBytecodeRef::from_borrowed_handle`（含 Runtime Rc 克隆与 root 登记）+ `snapshot_function_bytecode_owned`。同一函数对象两次发布之间这些产物不变。

**安装（`vm/call/ordinary.rs:166 install`，急切填满全部状态）**
3. `call_storage.reserve_depth`：三个池各一次 `saturating_sub` + `try_reserve`（`vm/frame/storage.rs:47`）。
4. receiver 按值 copy（方法调用）。
5. `global_object_for_realm` 每调用查一次（绝大多数调用同 realm，且多数帧从不读全局）。
6. `push_ordinary_active_frame`（`vm/frames.rs:917`）：`borrow_mut` + token 分配 + `active_frames.push(ActiveFrameRecord)` + **guard 内 `runtime.clone()`（每调用一次 Runtime Rc 克隆，`frames.rs:950`）**。
7. `capture_flags`：无捕获局部量的函数也要取池/清零/resize。
8. `prepare_push` try_reserve + `push_ordinary_frame` 槽窗口 + cold 全字段填充 + `prepared.install(Frame)` 按值 move 进 `Vec<(FrameId, Frame)>`。

**运行期（每次 run 退出）**
9. `driver/ready.rs:42-46`：除 AddLocal 外每个 RunExit 都执行一次 `update_active_bytecode_pc`（`frames.rs:545`：`borrow_mut` + 栈顶 token 校验 + 写 `Option<BytecodePc>`）。math_min 曾录得约 13 次 fault-PC 写/调用。惰性 fault-PC 审计（commit-plan.md「独立消融候选」）已让 run **内部**持局部 PC，但**每退出一次的注册表发布仍在**。

**返回（`driver/ordinary.rs:140 finish`）**
10. pop（Frame 按值 move 出）+ `guard.finish()`（又一次 `borrow_mut`）+ `recycle` 逐字段清理。

**已有的惰性基建（S10 在其上扩展，不推倒）**：`FrameRare` 已经 `OnceCell<Box<_>>` 惰性（`vm/frame.rs:78`）；cold Box 已池化回收（`storage.rs:36 CallStorage`）；run 内局部 PC guard 已实现并通过观察边界审计；AddLocal 已有"豁免逐次发布"的先例（`ready.rs:42`）。

结论：**帧的"物化"目前发生在每次调用与每次 run 退出，而它的绝大多数消费者（backtrace、异常、调试、宿主回调）在绝大多数调用中从不出现。** 这正是 CPython 3.11 lazy frames 解决的问题形态。

## 3. 新 S10 — 惰性帧协议（lazy frames 三件套）

目标用例：global_func_call +11.67%、depth-getter/proxy/mixed 全族、func_call/closure 差值；同时为 S11/S12 的驻留化铺路（驻留操作要求"不发布也正确"）。

### S10.1 认证缓存（对应 CPython"不再每调用重建 code/frame 对象"）

**机制。** 在 `ObjectPayload::BytecodeFunction` 侧增加认证事实缓存：

- 缓存内容：`Rc<PublishedFunctionData>`（快照数据，无 Runtime 引用，避免 heap→Runtime 循环）+ 发布代号 `publish_generation: u64` + 已校验的 closure 计数事实。
- 命中路径：`DirectSelection::select` 时一并读缓存；`authenticate` 退化为一次 `u64` 代号比较 + `Rc::clone`。miss 或代号不符走现有完整认证并回填。
- **失效机制（必须精确枚举后实现）**：heap 为 `FunctionBytecodeId` 的每次重发布/替换递增代号。审计条目 AU-1：枚举所有使 `snapshot_function_bytecode_owned` 结果变化的写入点（函数字节码重发布、realm 卸载、debug 信息剥离等），逐点接入代号递增；漏一处即缓存不健全，审计不通过则本条目降级为"仅 per-execution 最近调用点 memo"。
- **root 问题**：`PublishedFunctionSnapshot.root: FunctionBytecodeRef` 持 Runtime 克隆，不能存进 heap 对象。设计为缓存只存 `Rc<PublishedFunctionData>` + `FunctionBytecodeId`；帧需要 root 的场景（`eval_environment`、活动帧物化时的 `root().unwrap()`，`frames.rs:945`）在 S10.3 的惰性物化点按需重建。审计条目 AU-2：枚举帧生命周期内所有 `root` 消费点，确认均可迁移到物化点或按需构造。

**缓存事实及有效性。**本阶段缓存发布期事实，以精确失效代号维护其运行时索引；属性位置和值缓存按各自依赖事实维护有效性。

**安全模型不变。** `OrdinaryCall` 见证类型仍是唯一进入 `install` 的通道；缓存只加速见证的构造，不提供绕过域检查/closure 校验的路径。缓存命中路径必须保留 `belongs_to` 检查与操作数域检查的原有顺序（`enter_selected` 中 select→validate 的错误顺序契约）。

### S10.2 cold 惰性填充（对应 CPython"帧字段惰性初始化"）

按"绝大多数帧从不消费"的原则给 `FrameCold`（`vm/frame.rs:75`）字段分级：

| 字段 | 现状 | 设计 |
|---|---|---|
| `input.callee_global` | 每调用 `global_object_for_realm` | 惰性：改为 `Resident<Option<ObjectRef>>` 或按需查询；全局读驱动第一次需要时经 `executable.realm` 现查并驻留。审计 AU-3：枚举 callee_global 全部读取点与其错误语义 |
| `reusable_captured_locals` | 每调用取池 + resize 清零 | 发布期事实 `has_captured_locals`（P2：进 `FunctionMetadata`）；无捕获函数直接空 Vec，零池操作 |
| `normalized_this`、`return_to.operation`、`rare` | rare 已惰性 | 保持；`normalized_this` 并入 rare（仅 NormalizeThis 路径使用） |
| `property_generation`/`iterator_generation`/`caller_realm`/`active_frame` | cold Box 内 | 上提为 `Frame` 内联热头（消除热路径一次指针追逐；`Frame` 尺寸增量需 size 断言控制，见 S11） |
| `reserve_depth` 三池 | 每调用三次 try_reserve | 合并为单水位比较：`depth <= prepared_depth` 时零操作 |

安装序（预算→副作用、错误顺序、rejected push 释放子 owner 的既有反例，`frame.rs` 测试组）逐条保留；每项惰性化都要有对应反例测试证明观察语义不变。

### S10.3 fault-PC 惰性发布 + 活动帧惰性物化（对应 CPython"frame object 仅在被观察时物化"）

这是三件套的核心，也是收益最大的一件。

**原则：VM 自己的 `FrameStore` 是唯一事实源；runtime 注册表 `active_frames` 变成按需物化的视图。**

- **取消每调用的注册**：`push_ordinary_active_frame` 不再在 install 时执行。`FrameStore` 维护 `materialized_watermark: usize`（已物化到注册表的帧深度）。
- **取消每退出的 PC 发布**：`ready.rs:42-46` 的逐退出 `update_active_bytecode_pc` 删除；PC 事实保持在 `frame.fault_pc/resume_pc`（run 已在做）。
- **物化点（flush 协议）**：在 VM→外界的每个边界，把 `watermark..depth` 的帧一次性补注册并写入当前 PC：
  1. 进入 native/host 调用前（`start_native_with_classification` 入口）；
  2. 异常物化前（错误构造、backtrace 采集，含 `binding_error`、`lexical_uninitialized_error` 等全部错误物化点）；
  3. suspend/detach（`OwnedSuspension::detach`）；
  4. 溢出恢复与 `bytecode_call_would_overflow` 相关路径；
  5. 宿主可重入回调（Proxy trap、getter/setter、valueOf 等——即所有离开 run/driver 进入 JS 或宿主代码的通道）；
  6. profiling/诊断读取。
  审计条目 AU-4：以 commit-plan.md:670 已完成的"run 借用期间 fault-PC 读取方"审计为基础，扩展枚举**注册表**（`active_frames`）的全部读取方（`active_script_or_module_name`、backtrace、strict 判定、`reserve_native_argument_depth` 的深度读取等），确认每个读取方要么在物化点之后，要么改读 `FrameStore`。审计不通过的读取方单独保留即时发布，不得以性能理由削弱可观察性（沿用 670 行纪律）。
- **guard 重构**：`ActiveFrameGuard` 的每调用 `runtime.clone()` 与 token 分配随注册一起移到物化点；未物化的帧退出时只动 watermark（`min(watermark, depth)`），零注册表操作。异常/panic 路径的清理从 guard Drop 改为 flush 协议的对账（物化深度 ≤ 实际深度恒成立，多余记录在 unwind 时截断）。
- **native 帧不变**：native 调用本身就是边界，其注册保持即时。
- **深度记账**：`reserve_native_argument_depth` 等读 `active_frames.len()` 的地方改读 `FrameStore::depth()` + 已物化 native 深度的增量记账（B2 的 `installed_wait_depth` 先例）。

**先例支撑**：AddLocal 已豁免逐次发布且通过验收；惰性 fault-PC 的 run 内局部 guard 已通过全部观察边界审计（commit-plan.md:698）。S10.3 是同一审计方法论向"注册表整体"的推广。

### S10.4 帧栈协议接口重设计

把上述散落在 `enter_selected`/`install`/`finish`/`ready` 的协议固化为一个显式契约（新模块 `vm/call/protocol.rs` 或扩展现有 `vm/protocol.rs`），审查时按不变量逐条对照：

- **I1（认证）**：进入 `install` 的唯一通道是见证类型；缓存命中与 miss 产生完全相同的见证。
- **I2（PC 一致性）**：任何可观察点（物化点清单 AU-4）上，注册表 PC = 对应 `Frame.fault_pc`；两个可观察点之间注册表内容不被读取。
- **I3（深度一致性）**：`materialized_watermark ≤ FrameStore::depth()` 恒成立；所有深度消费者读 FrameStore。
- **I4（owner 责任）**：惰性化不改变 owner 释放顺序（子先于父，rejected push 立即释放——现有 Drop 测试组全部保留并扩展到惰性路径）。
- **I5（错误顺序）**：域检查、TDZ、溢出等错误的产生顺序与现状逐字节一致（复用现有 layout_tests 风格反例组）。

### S10.5 验收（实现完成后统一测量，期间不 benchmark）

- 机械指标（既有 profiling 事件）：`runtime_pc_publication` 从每退出一次降为仅物化点；`ordinary_call_authenticated` 在稳态循环中≈0（全命中缓存）；global_func_call 每调用注册表操作 0（无异常路径）。
- 正确性门禁：同源完整门禁（工作区全量、oracle 压力、Test262 全向量一致、Web/Node/WASM）+ 新增惰性路径反例（异常中段物化、深栈溢出恢复、Proxy 重入观察 backtrace、suspend/resume 后的 PC 正确性）。
- 性能验收：三轮公平复测（S0 复用旧三轮），验收行 global_func_call、depth-getter/proxy/mixed/native 全族、func_call/func_closure_call；目标为差值显著收窄且方向稳定，不预设具体百分比结案线。

## 4. 新 S11 — Step/Resume 瘦身 + 两级状态机合并

目标用例：local_destruct +26.19%、array_for_of +5.59%、Map/WeakMap 族 +10~15%（IPC 病）。依赖 S10（帧协议稳定后再动分派结构，避免双变量）。

### S11.1 现状结构

- `RunExit`（`vm/run.rs:23`）52 个变体。
- 外层 `run_frames_with_state`（`vm/driver.rs:618`）：约 18 个**顺序** `if let` 臂（Call→Environment×2→DefineProperty→DefineClass→ClassInitializer→`frame_operations::step`→Construct/Apply→Convert×3→ApplyEval→Eval→Import→Predicate→SuperProperty→SetProperty→GetField→GetElement→unwind→Suspend）。
- 二级 `frame_operations::step`（`vm/frame_operations.rs:24`）：约 25 个顺序 `if let` 臂，且带 `exit` 变量重赋值的贯穿逻辑（`forwarded`/`exit = RunExit::Complete` 模式）。
- 一个冷臂 RunExit 最坏经历 ~40 次顺序判定；每层都是难预测分支。宽 `Step`/`Resume` 按值搬运（R2 残余行记录的成功路两次 584 B memcpy）叠加数据依赖。

### S11.2 设计

1. **合并为单层穷尽 `match`**。两级 if 链重写为一个对 `RunExit` 的穷尽 `match`（rustc 对无守卫穷尽 match 生成跳表），每臂调用一个 `#[inline(never)]` 处理函数；`forwarded`/`exit` 重赋值贯穿逻辑改为处理函数返回统一的窄 `Disposition` 枚举（Entered/Complete/Bridge/Rethrow），消除臂间状态穿透。热臂（Call/GetField/Numeric/Complete）已在 `ready::run` 内层处理的保持不动——本条只重排**冷边界**，不改 ready 热循环语义。
2. **宽状态 Box 化 + `&mut` 推进（P3 推广）**。定位所有 >64 B 按值搬运的协议结构（首要嫌疑：R2 残余行的 `Step`/`Resume`、`SliceResume`、`PendingIterator` 相邻家族、`Frame` 在 `prepare_push().install()`/`pop` 的整体 move）。逐个改为：驻留 + `&mut` 推进，或把宽 payload 装 Box 使枚举本体 ≤32 B。真实等待才构造宽结构的原则（P3）扩展为全家族规范。
3. **尺寸回归断言**。仿 `driver/ordinary.rs:231` 的 ABI 断言测试，为 `RunExit`、`Disposition`、各驱动 `Step`/`Resume`、`Frame` 增加 `size_of` 上限断言，防止后续提交无声退化。
4. **代码布局配套**（本阶段采用源码布局优化）：冷错误构造、诊断、profiling 分支全部 `#[cold]`/`#[inline(never)]` 外提出 `run` 与 ready 循环，压 `.text` 热区。

### S11.3 验收

- 机械：尺寸断言全部通过；单层 match 后冷臂判定次数上限 = 1 次跳表。
- 性能：三轮复测验收行 local_destruct、array_for_of、map_set_int/string、map_delete、weak_map_set；硬件计数要求 IPC 回升、branch-misses 下降为主要证据（这正是病 2 的配对因果实验——若合并后 IPC 不回升，则病 2 的"状态机结构"假设被削弱，须回到物理归因）。

## 5. 新 S12 — 属性读内联缓存驻留

目标用例：全部 6 个 v8-* 综合（earley-boyer/richards/raytrace/crypto/splay/deltablue）+ prop_* 族。对应 CPython 3.11 的另一半：PEP 659 自适应特化。依赖 S10.3（命中路径要求零 PC 发布）与 S11（驻留分派稳定）。

### 5.1 阶段方案与有效性

S12 采用属性位置缓存：

- 本阶段条目保存位置事实（对象布局代号 + 槽偏移 + 原型链深度），属性值在命中时读取；
- 每条缓存带精确失效代号（heap 侧对象布局/原型链变更递增 generation；`FrameCold.property_generation` 与既有 collection-records generation 机制为先例）；
- miss/失效沿用 `property_driver`，保持原有可观察语义。

后续可扩展缓存内容与覆盖范围；属性值缓存需要同时处理值变更依赖和持有值的生命周期。

### 5.2 设计

1. **调用点侧表**：`PublishedFunctionData` 已有 stack-vm 专属 `fusion` 侧表先例（`code/executable.rs:94`）。新增按 GetField 调用点索引的 IC 表（发布期分配定长槽，运行期可变内容与代码本体分离，保持快照数据不可变契约——IC 槽为 `Cell` 数组或独立 per-bytecode 运行态，设计时二选一并审计 GC/多 realm 语义）。
2. **IC 条目**：`{layout_generation: u64, holder_kind: Own | Proto(depth), slot: u32}`。命中判定 = 一次代号比较；own-data 命中在 run 借用内直接读槽 push 结果，不退出 run、不发布 PC（S10.3 保证）、不进 `read_progress`。
3. **失效**：heap 对象布局代号在属性增删/attribute 变更/原型替换/字典化时递增。审计条目 AU-5：枚举全部布局变更写入点；与 dictionary-objects/holey-array 既有机制对齐。
4. **覆盖顺序**：GetField（静态键读）→ 方法取用（与既有 GetField2 fusion span/B4 方法 span 汇合）→ SetProperty own-data 写（二期，需要 setter/只读检查事实进 IC）。S12 已实现覆盖 GetField/GetField2；属性写、GetElement 与 Proxy 当前沿用通用路径，作为后续扩展项。
5. **每条目单态起步**：S12 实现采用单槽 IC（monomorphic），两次 miss 后该条目退化至通用路径。这是当前策略的记录；后续可采用多态链、重新特化或其他有性能依据的策略。

### 5.3 验收

- 机械：稳态循环 `read_progress`/`ordinary_read_probe_atom` 调用数≈0（IC 命中路径零驱动进入）；IC 失效反例（属性删除、原型替换、defineProperty 改 attribute、字典化、跨 realm）全部命中回退路径。
- 性能：三轮复测验收行 6 个 v8-* fixed + 原始 V8 Score 六项 + prop_read/prop_write/prop_delete/prop_clone；E 簇（正则外围读）预期同步受益，复测一并记录。

## 6. 阶段顺序、依赖与纪律

```
新 S10（惰性帧协议）→ 新 S11（窄状态机）→ 新 S12（属性 IC 驻留）→ S13（原 S10：退役旧路径）
```

- 每阶段内部顺序：审计条目（AU-x）先行并留档 → 实现 → 同源正确性门禁 → 三轮公平复测（单阶段单变量）→ 回填 commit-plan 账本对应行。
- 本计划期间不做中途 benchmark/Profile（2026-09-15 用户指令）；所有测量集中在阶段验收点。
- S09 的 22 项 fixed 正差值、探针、RSS、compile 记录保持"未完成"状态；各阶段验收只允许关闭其证据支持的行，禁止以"结构已重写"为由批量结案。
- G（RSS）继续不实施；若 S11 的 `.text` 外提顺带改善 RSS，只记录不结案。
- 未归因清单（§1.3）随各阶段复测数据更新；depth-getter/proxy/mixed 在新 S10 验收轮补第一次探针级 Profile。

## 本次实施状态与最终结果

S10 `77153244`、S11 `e0494120` 已实施，S12 与最终报告为第三个 commit。三阶段代码及额外覆盖 review 均完成：review 找出的“一次性交接宽包遗漏”“尺寸断言不全”已补齐；131 项领域尺寸约束通过。内联 Value 完成枚举最低 40 B，不为凑 32 B 给无需等待的结果新增 Box。完整逐项记录见[覆盖核对](performance/README.md)。

最终同一 Rust 源码的库测试 2503/2503、工作区 3598、独立 oracle 压力、Test262 全结果向量与 Web/Node/WASM 15 示例均通过。边界验收为完整矩阵加唯一修正锚点的定向复验，原失败与补充证据均保留；具体口径见[执行账本](primitive-vm-commit-plan.md#新-s10s12-执行账本2026-09-15)。

全部实现与覆盖补漏完成后，唯一最终性能轮 403/403 有效，S0 只读取旧三轮；没有中途或第二轮 benchmark/Profile。普通调用、闭包调用、Richards 等已快于 S0；但 58 fixed 仍有 25 项正差值（直接前版 31 项），其中 21 项超过 +5%。Map/WeakMap 等相对直接前版还恶化，不能把结构实施完成写成性能目标全部达成。67 compile 中 51 项正差值，可比探针 12/20 更慢，RSS 3/3 更高。

S11 机器码的主 RunExit 选择为一次跳表，但冷路径仍含领域子操作与返回值条件分支；IPC/branch-miss 改善只在部分目标行兑现。S12 缓存命中路径驻留并复用 B4，全部调用点/输入的 lookup 总量并未因此归零。三个阶段联合单轮不支持独立阶段的因果归因。

结论：本次 S10–S12 计划内实施没有已知遗漏；性能退出条件未全部通过，S09 保持未完成。SetProperty 写 IC 仍是本节 5.2.4 明确的二期，G 与 S13 未实施。所有残余差值见[最终报告](performance/README.md)及其完整 JSON/CSV，不追加本轮测量或范围外优化。

## 后继计划（2026-09-15）

残余 25 项差值已全部下钻到实现层根因并立项修复，见 [S14–S20 修复计划](primitive-vm-s14-s20-recovery-plan.md)。该计划推翻或收窄了本文的三个诊断假设：Map/WeakMap 簇的病 2 假设被最终 IPC 数据证伪，真实根因是 IC 接收者白名单（`property_ic.rs:118-125`）与 native 调用外围；splay/raytrace 的 GetField 残留不是多态失配，而是 IC 的 GC 门控（`ic.rs:27-35`）与 2-miss 永久 megamorphic；`0x18f413` 已确认为 glibc memcpy，主源是 904 字节 ArenaSlot 与 regexp 缓冲的 `try_reserve_exact`。§1.3 未归因清单中除 compile 单轮噪声外的条目均已在新计划中归因。本文其余内容保留为 S10–S12 的历史设计与验收记录。
