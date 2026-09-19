# 性能架构：8B 值表示 + quickening + 数据导向堆（无 JIT、默认无 unsafe）

> 状态：设计定稿，待评审与分阶段实施。本文档取代
> `performance-plan.md`「横向设计比较」之后悬置的 S3 定义；S0–S2 是常数因子
> 路线，S3 起升级为结构路线。目标不是「补齐常数」，而是在无 JIT 约束下对齐
> 并在特定轴上超过 QuickJS。

---

## 0. 输入与硬约束

输入文档：`performance-plan.md`（S0–S2 证据与横向比较）、`parity.md`、
`status.md`、`profiling.md`。外部证据见附录 A。

硬约束：

1. **无 JIT**（`performance-plan.md:670`）。上限是「极致解释器」。copy-and-patch、
   动态代码复制、wasm-as-format 均属 JIT 近亲，不采纳。
2. **GC 模型被契约钉死**：`parity.md:183-191`（§13）要求保持 QuickJS 式确定性
   RC + 循环回收，finalize 时机可观察。因此「换 tracing GC 消灭 RC 流量」
   **不在 S3 范围**；S3 内 RC 只能做便宜，不能消灭。契约修订情形下的
   tracing 迁移列为 **S4 候选（推迟，非否决）**，决策门禁见 §10。
3. **unsafe 政策**：`parity.md:23` 允许受审计 `unsafe`，但 workspace 现状是
   `unsafe_code = "forbid"`（`Cargo.toml:46-47`），且 `status.md:3` 对外宣称
   "unsafe-free"。S3 的默认路线**零 unsafe**；受审计 unsafe 只作保留席位
   （方案 F），引入是显性治理动作，不得在性能 PR 里悄悄发生。
4. **一致性门禁**：Test262 冻结向量（`pass=79982 / eligible=80032 /
   total=102037`）零回归；不得为性能改动修订一致性基线
   （`scripts/benchmark/README.md:175-176`）。
5. **测量纪律**：正式计时用非 profiling 的普通 release 构建、串行、独立输出
   目录、留 receipts（`profiling.md:288`；benchmark README）。S3 第一阶段
   必须先重定基线（方案 E）。
6. 单线程执行核心：`Rc`/`RefCell`/`Cell` 语义不变；不引入 `Send`/`Sync`
   共享（`parity.md:243` worker 模型不变）。

## 1. 关键架构事实（现状证据）

以下事实是 S3 选型的依据（全部已核实，含文件行号）：

1. **32B `Value` 的根因是「每个值背着 `Rc<Runtime>`」，不是 enum 本身。**
   `ObjectRef = Runtime(Rc, 8B) + ObjectId(8B)`（`src/engine/object/mod.rs:28-31`）；
   `SymbolRef = AtomOwner(Rc) + Atom(16B) = 24B`（`object/mod.rs:374,135-138`），
   它把 `Value`（`src/engine/value/mod.rs:13-24`）撑到 32B。该 Rc 的唯一作用是
   让值在没有 runtime 参数时也能 retain/release/drop 并做跨 runtime 防护——
   QuickJS 用显式 `JS_FreeValue(ctx, v)` 解决同一问题。**把 Rc 逐出内部值是
   8B 的入场券，这是所有权纪律问题，不是 unsafe 问题。**
2. **句柄天然是索引**：`ObjectId { index: u32, generation: u32 }`
   （`src/engine/heap/identity.rs:7-11`），解引用本来就是 arena 下标访问——
   安全的 bounds-checked load。把 u32 索引装进 NaN payload 只需纯整数位运算，
   **索引 NaN-box 不需要 unsafe**。最坏情况是 bug 变成 trusted-path panic 或
   leak，不是 UB。
3. **RC 单价已降、次数仍在**：S1 已把热路径 retain 改成 `&self` + `Cell`
   饱和加（`retain_raw_fast`，`src/engine/heap/gc.rs:1059`），但值每次复制/
   销毁仍各有一次计数操作，且 Object 克隆仍走 `try_borrow_mut` + 校验
   （`copy_reference`，`src/engine/vm/stack.rs:1419`）。RC 次数是语义性的，
   只能降单价（方案 A/D），不能归零（约束 2）。
4. **借检查是全局单点**：`RefCell<RuntimeState>` 全仓库 981 个借用站点
   （`runtime/mod.rs:31-62`；`vm/driver.rs` 122 处为最密）。热路径已尽量
   `Cell`/`try_borrow`，但「单线程可变性证明」在运行期付费。
5. **字节码不可变 + 侧表可变是既有先例**：`code: Rc<[Instruction]>`
   （`code/executable.rs:206`）发布即冻结，`FusionPlan` 用 per-PC 标志字节
   侧表做静态超指令（`code/fusion.rs:1-6`），IC 用 `Cell<State>` 挂在共享
   snapshot 上（`object/property_ic.rs:23-37`）。「不可变规范 + 可变执行态」
   的信任模型已经存在，quickening（方案 B）是它的自然延伸。
6. **存量浪费清单**（与安全无关，纯自残税）：
   - `ArenaSlot = 440B`：全 kinds 混排单一 `Vec<ArenaSlot>`
     （`src/engine/heap/mod.rs:232-237`；测试断言 `src/engine/heap/edges.rs:124`），
     由最大的 `ObjectData` 撑起；
   - 对象无内联槽：`ObjectData.slots: Vec<PropertySlot>`
     （`src/engine/heap/object_records.rs:461-484`），每个对象至少一次独立分配；
   - 属性键 16B：`Atom { raw, generation, table_id }`（`src/engine/atom/mod.rs:52-57`），
     相等比较逐 16B（shape ≤8 项线性扫描全命中）；
   - atom 字符串表用 SipHash（`atom/mod.rs:280-294`），且 `JsString` 不缓存
     hash，每次 probe 全串重算（`src/engine/value/primitive.rs:1249-1256`）；
   - 任一 prototype 被写即 bump 堆全局 `property_layout_epoch`
     （`src/engine/heap/object_storage.rs:10-18`），**全堆 depth>0 的 IC 条目
     集体失效**——V8 用 per-prototype validity cell，粒度天差地别；
   - shape 迁移存 runtime 全局嵌套 HashMap、键是 24B `ShapeEntry`
     （`runtime/mod.rs:112-113`）；
   - 派发循环每步付 bounds-checked fetch + `checked_add` + `Result` 管道
     （`src/engine/vm/run.rs:316-328`），指令是 ~16B/条的 198-variant enum
     （`code/bytecode.rs:123`）。
7. **`[profile.release]` 从未调过**：`Cargo.toml:51-57` 只有 web profile；
   native release 是 cargo 默认（无 LTO、16 CGU）。PGO/LTO 是未领取的
   免费收益。

## 2. 方案总览

按依赖与收益排序（E 先行，A 是地基，B 是差异化武器，D/C 与 A 复利）：

| 方案 | 内容 | unsafe | 预期量级 | 依据 |
| --- | --- | --- | --- | --- |
| **E** | 构建基线：LTO + CGU=1 + PGO（BOLT 可选） | 无 | 8–20% | 附录 A.7 |
| **A** | 8B 值表示：索引 NaN-box，Rc 逐出内部值 | 无 | ~1.5–2×（值流量密集路径） | §1.1–1.3 |
| **B** | quickening + 可变执行 IR（QuickJS 没有） | 无 | +10–25% | 附录 A.1–A.3 |
| **D** | 数据导向堆：typed arena、内联槽、validity cell、atom/string 便宜化 | 无 | +10–30%（对象/数组密集） | §1.6 |
| **C** | 派发与栈流量：TOS/accumulator 缓存、扩展静态超指令、可选 fn-pointer threading | 无 | +5–15% | 附录 A.4–A.6 |
| **F** | 受审计 unsafe 保留席位：仅在测量点名后逐点引入 | 受审计 | 视点名位置 | §8 |

不采纳：寄存器式 VM 全面重写、nightly `become`、copy-and-patch /
动态复制（理由见 §9）。**S4 候选：RC → tracing GC——推迟而非否决**，
双门禁见 §10。

## 3. E：构建基线（第一阶段，先于一切测量）

- `Cargo.toml` 增加 `[profile.release] lto = "fat", codegen-units = 1`；
  建立 PGO 流程（`-Cprofile-generate` / `-Cprofile-use`），训练负载用
  `scripts/benchmark/scaling.py` + v8-v7 套件。BOLT 作为可选后续。
- 用同一 flags 重建 pre-S3 基线二进制并留 receipts；S3 之后所有对比都以
  **PGO 后的基线**为分母（否则把编译器布局噪声当设计收益）。
- 提交粒度：profile 改动与 PGO 流程脚本各一个 commit；无语义变更，门禁走
  `cargo test --locked --workspace --all-targets` + test262 `--check`（源码
  哈希会变，需重跑 `--full` 出 current-source receipt，不改 `current.conf`）。
- 生效范围：`lto = "fat"` + `codegen-units = 1` 对**任何 `cargo build --release`
  自动生效**（含 CLI、`build.py`）；**PGO 不会**——它需要 `pgo.py` 的两阶段
  `RUSTFLAGS=-Cprofile-use`，默认 release 构建没有 profile，需显式跑 PGO 流程
  才能得到完整方案 E 的二进制。

### E 实测（相对 pre-S3 HEAD `06386457`）

在独立 worktree、同机串行构建并测量三个普通 release 二进制，无并发构建/测试：

| 标签 | 配置 |
| --- | --- |
| `baseline` | pre-S3 HEAD 默认 release（无 LTO、16 CGU） |
| `lto` | `lto = "fat"` + `codegen-units = 1` |
| `pgo` | LTO/CGU=1 + PGO；训练负载 = `scaling.py` 全 22 case×{64,128} + v8-v7 全 8 suite |

`pgo.py` 逐进程设置唯一 `LLVM_PROFILE_FILE=<dir>/%m_%p.profraw`，再
`llvm-profdata merge` 合并（否则每进程覆盖同一个 `default_%m_%c.profraw`，只剩
最后一次训练负载）；产物带 `.build.json` receipt，记录 profdata 哈希与训练负载。

**属性读探针（`property_read_probe.py`，N=5,000,000，repeat 7，median ns/op）：**

| case | baseline | lto | pgo | lto 变化 | pgo 变化 |
| --- | ---: | ---: | ---: | ---: | ---: |
| prop_read_int | 176.23 | 160.64 | 112.34 | −8.8% | −36.3% |
| prop_read_obj | 215.68 | 203.83 | 135.83 | −5.5% | −37.0% |
| prop_read_string | 271.43 | 249.57 | 198.37 | −8.1% | −26.9% |

**`scaling.py`（22 case × 2 size = 44 cell，operations=32768，repeat 3，整进程 wall）：**
`lto` geomean `0.898×`；`pgo` geomean `0.696×`、中位 `0.685×`（**−31.5%**），
区间 0.555–0.996（最好 cell `array-holey`、`array-index`、`set`，最差 `scope`）。

**V8-v7（Score，越高越好，repeat 3）：**

| case | baseline | lto | pgo | lto 变化 | pgo 变化 |
| --- | ---: | ---: | ---: | ---: | ---: |
| richards | 50.7 | 55.7 | 87.6 | +9.9% | +72.8% |
| deltablue | 64.1 | 72.2 | 110.0 | +12.6% | +71.6% |
| crypto | 62.9 | 65.6 | 101.0 | +4.3% | +60.6% |
| raytrace | 95.7 | 105.0 | 143.0 | +9.7% | +49.4% |
| earley-boyer | 117 | 127 | 181 | +8.5% | +54.7% |
| regexp | 88.5 | 90.5 | 138 | +2.3% | +55.9% |
| splay | 320 | 349 | 483 | +9.1% | +50.9% |
| navier-stokes | 255 | 299 | 422 | +17.3% | +65.5% |
| **geomean** | | | | **+9.1%** | **+60.0%** |

**`microbench`（ms 分辨率，仅定性，min ns/op）：** `empty_loop` 100→50→40、
`prop_read` 250→125→100、`array_read` 200→200→100、`func_call` 500→500→250；
`int_arith` 三档均为 200、未分辨。

**结论：** 方案 E 的实测收益超出 §2 预计的 8–20%——LTO+CGU=1 约 5–9%，
PGO 把属性读延迟压低 27–37%、V8-v7 Score 整体抬高 **1.60×**、混合整进程负载
geomean **0.696×**（≈1.44× 吞吐）。这是纯构建层收益、无语义改动，成功为
后续 A/B/D 建立更高的比较基线。

**门禁：** `cargo fmt --check`、`check-source-layout.py`、workspace
`cargo test --locked --workspace --all-targets`、benchmark 单测（22）全部通过；
Test262 `--check` 如预期报 baseline 源码过期（`Cargo.toml` 在
`engine_semantics_files` 内），`--full` 重跑得到 current-source receipt：
`complete Test262 vector matches: 79982 pass of 80032 eligible (102037 total)`，
零回归，`current.conf` 未改。

## 4. A：8B 值表示——索引 NaN-box（零 unsafe）

> 实施拆分（A0–A4）与三个设计点（内部 `JsValue`/API 边界、String/BigInt
> 堆化、`Atom` 瘦身）已钉死于 **`docs/reports/s3-a-plan.md`**；本节为设计
> 概要，冲突处以 s3-a-plan.md 为准。

### 4.1 编码

```rust
pub struct JsValue(u64);  // 内部执行值；不实现 Copy/Drop
```

- `Float(f64)`：按位原样存储（与 QuickJS 同等待遇，浮点不装箱）；
- `Int(i32)` / `Bool` / `Null` / `Undefined`：tag 空间内联；
- Object / String / Symbol / BigInt：NaN payload 52-bit 内装 `kind | u32 index`，
  解引用 = typed arena 的安全下标访问（保留 bounds check；generation 校验维持
  现状的「可信路径 debug-only、边界全量」分层）；
- 全部纯整数位运算，**无 union、无 transmute、无裸指针**。

### 4.2 所有权纪律（核心改动）

- `JsValue` 不实现 `Copy`/`Drop`：VM 在覆盖槽、弹栈、拆帧时显式 release
  （此刻调用点本就有 `&RuntimeState`/`&Heap`）；复制即显式 dup（trusted
  `Cell` 递增，S1 已就位）。这是 QuickJS 的 C 纪律搬进安全 Rust：Rust 不
  强制，出错形态是 leak 或 trusted-path panic，不是 UB。
- 公共 API 保留现有带 Rc 的 root 类型（`ObjectRef` 等），只在 API 边界做
  root/unroot 转换；root 类型继续提供 Drop 语义与跨 runtime 防护。
- `Atom` 内部瘦身为 `u32` newtype；16B branded 形式只留不可信边界。论证与
  `live_node_fast` 相同：活 shape/字节码/IC 持有的 atom 必然被其 owner retain，
  可信路径免品牌校验。atom refcount 顺带 `Cell` 化（收尾 S1b，`Symbol` 不再
  从快路 decline）。

### 4.3 退路

若位编码需分阶段：先落 `enum { Int(i32), Float(f64), Object(u32), … }`
（16B，f64 内联、u32 句柄），零编码风险，已比现状小一半；NaN-box 作为
第二步。注意 Nova 式 8B enum（句柄 + boxed f64）**不采纳**：浮点上堆会在
算术密集路径引入分配，比 NaN-box 差。

### 4.4 级联收益

- `RawValue`（`identity.rs:200-224`）24B→8B；`PropertySlot::Data` 同减，
  属性内存减半；
- `FrameBinding`（`vm/bindings.rs:17-23`）40B→~12B，操作数栈槽同减；
- `copy_value`（`vm/stack.rs:1402`，S0 实测 ~8%）标量臂从 73B outlined
  变为一条 `mov`；值搬运总量降 4×；
- 每条 64B 缓存行放 8 个值（现状 2 个）。

### 4.5 分步提交

下列草单已被 `s3-a-plan.md` 的 A0–A4 拆分取代（新增 A0 地基与 A1
String/BigInt 堆化两个阶段，原子瘦身提前至 A0-a），保留仅供追溯：

1. `refactor(value): introduce handle-based internal value type` —— 新类型 +
   转换层，先不接线；
2. `refactor(atom): slim internal atom to u32 index` —— 边界保留 branded；
3. `refactor(vm): operate on internal values in slots and frames` —— FrameBinding /
   SlotStore / run.rs 切换，显式 dup/release；
4. `refactor(heap): store 8-byte raw values` —— RawValue / PropertySlot 切换；
5. `perf(value): nan-box encoding` ——（或先停在 16B enum 退路）；
6. `docs(perf): record S3-A measurements`。

### 4.6 风险

- 手动 RC 纪律扩大 invariant-panic 面：debug 构建维持全量 generation 校验 +
  冻结向量兜底；trusted 访问器遇 stale 即 panic 的政策不变（S1 已确立）。
- 触及面最大（值类型是所有模块的公共依赖）；必须与 B/D 分阶段，不可一锅端。

## 5. B：quickening + 可变执行 IR（差异化武器）

### 5.1 设计

把「规范指令」与「执行指令」分离：

- `code: Rc<[Instruction]>` 保持不可变、已验证、BC5 契约不变
  （`code/bytecode.rs:111-114` 的单一指令契约注释、验证器
  `code/bytecode_validation.rs`、二进制对象链路均不动）；
- 每个 `PublishedFunctionSnapshot` 旁挂一条**可变执行 IR**：
  `quick: Box<[QuickOp]>`，8B/word（opcode u8 + 操作数位 + IC 槽号），
  发布时从规范指令译出；
- 运行时按 feedback 把通用 QuickOp 重写为特化形（`Add → AddInt`、
  `GetField → GetFieldIC`、比较/分支/调用同理），guard 失败 deopt 回通用形；
  **发布时由验证器认证每个 PC 允许的重写集合**——与 `FusionPlan` 同一信任
  模型，BC5 只含规范 opcode，quickening 是纯运行期层；
- 特化状态 per-FunctionBytecode 共享（与现有 IC 站点一致，
  `code/executable.rs:267-269`），CPython 亦然。

### 5.2 为什么这是「超过 QuickJS」的点

- CPython（PEP 659）、JSC LLInt、V8 Ignition、Deegen 全部收敛到
  quickening + IC；**QuickJS 完全没有 type feedback**——算术、比较每次走
  完整通用路径。给算术/比较/属性/调用装上带 guard 的特化 + deopt，是在
  QuickJS 的盲区建立结构优势（证据量级见附录 A.1–A.3：特化归属 10–25%+）。
- 纯安全 Rust 可达：`Cell`/侧表模式已有先例，无需改 GC、无需 unsafe。

### 5.3 顺带解决的存量问题

- IC 槽号直接编码进指令字，干掉 GetField 的 bitmap + block-rank 站点查找
  （`object/property_ic.rs:364-416`）；
- fusion 跨度直接译成单个 QuickOp（`code/fusion.rs` 的 span 模式平移）；
- 指令 ~16B enum → 8B word，icache 占用减半；
- 198-variant 大 match（`run.rs:328-1832`）分为热集 + generic 两档。

### 5.4 分步提交

1. `feat(code): decode canonical instructions into quick ops at publish` ——
   执行 IR 只读译码 + 派发切到 QuickOp（语义不变， fusion 平移）；
2. `feat(vm): quicken arithmetic and comparison ops with guards and deopt`；
3. `feat(vm): quicken property access with embedded ic slots`；
4. `feat(vm): quicken call sites`；
5. `docs(perf): record S3-B measurements`。

### 5.5 风险

- 验证面扩大：重写集合必须在发布时认证，deopt 必须回规范语义；pc2line /
  `Ret`/`Gosub` 的 pc-as-`Value::Int` ABI（`run.rs:1773-1796`）以规范指令
  索引为准，QuickOp 侧维护映射；
- 冻结向量是最强兜底；每个 quickened opcode 需配 guard-fail 单测。

## 6. D：数据导向堆布局（与 A 复利）

按收益密度排序，各项独立成 PR：

1. **Typed arenas**：440B 统一 `ArenaSlot` 拆 per-kind arena（Object / VarRef /
   Shape / Context / FunctionBytecode 各自 `Vec` + 各自 free list）。句柄格式
   不变，trusted 访问器平移。VarRef/Shape 槽从 440B 降到几十 B；局部性与
   RSS 一起改善。
2. **对象内联属性槽**：`slots` 改 SmallVec 模式（前 2–4 槽内联，溢出再堆
   分配），消灭每个对象一次独立分配。
3. **Per-prototype validity cell 取代全局 `property_layout_epoch`**：
   `used_as_prototype` 对象各自携带 epoch/cell；IC 条目记录具体 cell。
   现状是任一 proto 写全堆杀 depth>0 IC，原型链密集负载（deltablue /
   richards 类）白丢缓存。
4. **atom / string 便宜化**：`StringRepr` 头部缓存 hash；atom 字符串表
   SipHash → FxHash（`src/engine/hash.rs` 已有）；shape 迁移改 per-shape 小
   Vec（1–2 项内联）替代 runtime 全局嵌套 HashMap。
5. **S2.1 快速释放单独立项**：A 落地后重新评估「任何借用都 defer」的
   契约松弛（`src/engine/heap/slot_ownership.rs:208` 测试钉死），不作为
   性能 PR 的附带改动。

## 7. C：派发与栈流量——收割，不重写

- **不做寄存器式重写**：fat-opcode JS 的现代证据只有 ~7%（附录 A.5），
  对不起 198 variant + 验证器 + 挂起 ABI（`vm/suspend.rs`）的全面翻新。
  locals/args 本就是直寻址槽（等价寄存器），目标只剩临时栈流量。
- **TOS/accumulator 缓存**：栈顶 1 项驻留；二元运算/比较/分支原地 peek
  （`binary_number_current` 已是先例，`run.rs:1603-1663`），推及比较/存储。
- **扩展静态超指令**：按 profile 选热对，在 QuickOp 层融合（B 的 IR 令其
  成本骤降）。现代 ITTAGE 下期望 5–15%，不是 2003 年的 3×。
- **派发机制**：保留单点 match 为基线（附录 A.6：现代预测器下单点 switch
  并不差）；handler 抽到宏背后以便 A/B fn-pointer threading。stable Rust
  **不保证** tail call（wasmi 赌 LLVM TCO + 逐版本审汇编）；Rust 1.92 的
  DestinationPropagation 曾合并条件分支站点致同类解释器 −30~50%——
  **每次升 rustc 必须数二进制里的间接跳转**。此项是选项不是地基，最后做。

## 8. F：受审计 unsafe 保留席位

A–E 全部不需要 unsafe。仅当落地后 profile 点名具体位置（trusted 访问器的
bounds check、PC 裸指针化、侵入式链表）时，按 `parity.md:23` 逐点引入。
引入即治理动作：翻 `unsafe_code = "forbid"`（`Cargo.toml:46-47`）、收窄
codec 自测门禁（`status.md:319-320`）、修订 `status.md:3` 的 "unsafe-free"
声明——三件事必须显性完成，不得在性能 PR 里夹带。

## 9. 不采纳的路线与理由

| 路线 | 理由 |
| --- | --- |
| 寄存器式 VM 全面重写 | fat-opcode JS 实测 ~1.067×（附录 A.5）；重写成本巨大 |
| tracing GC 替代 RC | **推迟至 S4（见 §10），非否决**：§13 契约当前排除；S3 内只降 RC 单价（A/D） |
| nightly `become` / musttail | MSRV 1.88 stable；x86 codegen 仍不稳定（附录 A.4） |
| copy-and-patch / 动态复制 | 运行期机器码生成，属 JIT（约束 1） |
| wasm-as-interpreter-format | 是换产品形态，不是解释器技术 |
| Nova 式 8B enum（boxed f64） | 浮点上堆，算术密集路径引入分配；索引 NaN-box 更优 |

## 10. S4 候选：RC → tracing GC（推迟，非否决）

### 10.1 定位

- **不是当前差距的约束项**：QuickJS 同为 RC + 循环回收，仍快本项目
  10–25×（`performance-plan.md:319-320`）——RC 不是天花板，值表示 / 派发 /
  IC / 对象布局才是。S0–S2 实测 RC 直接成本 ≈ `copy_value` ~8% + release
  路径 ~4%（S1 后更低）。
- **是 S3 之后的最大单项杠杆**：S3 削掉其他项后，「每次值复制/销毁各一次
  计数」的语义性开销将浮为头部项——RC 是一块地板，tracing 是拆地板的
  唯一手段。
- **与方案 A 不冲突**：A 的 8B 句柄值 / 索引 NaN-box / typed arena / 边界
  root 层在 tracing 下约 80% 原样复用；被废弃的只有「显式 dup/release
  纪律」一层。先做 A 在任何未来路径上都不亏。

### 10.2 若实施的设计形态

- **保留**：typed arena；`{index, generation}` 句柄；`Edges` 出边遍历
  （循环回收器已有，mark 阶段直接复用）；边界 root 类型。
- **删除**：`strong` 计数、zero_queue、deferred 队列、trial-deletion 循环
  收集器、全部 retain/release、所有「drop 改堆」逻辑。
- **新增**：
  - per-slot mark 位（或侧 bitmap）+ mark stack + sweep 重建 free list；
  - **Root registry**：边界 root 类型从「Rc + 计数」改为「registry 条目 +
    Drop 注销」，仅跨 GC 点持有的代码创建——tracing 的结构性优势即在此：
    RC 在每条内部路径按次计费（操作数栈、帧、对象字段、常量池），tracing
    只在 native 边界收注册费，内部流量全免；
  - **safepoint 纪律**：GC 仅发生在分配点与 operation 边界；「两个分配点
    之间持有的裸句柄必须活在 VM 可扫描状态（操作数栈/帧）」成为不变量；
  - 分代化时的 write barrier：老→新字段写 push remembered set（仅堆字段
    写收费）；
  - **debug GC stress 模式**：每次分配即回收，配合 generation 校验把
    rooting 错误从「事后悬垂」变成「即时 panic」。
- **架构简化红利**：drop-defer 契约网整体蒸发（S2.1 类冲突不复存在）、
  Rc + heap 双计数消失、retain-during-borrow panic 类别消失、`Value` 变
  `Copy`、操作数栈操作退化为数组搬移。

### 10.3 安全 Rust 下 rooting 纪律的三条路线

| 路线 | 结论 |
| --- | --- |
| gc-arena（不变量生命周期） | 完全安全证明，但堆访问闭包域化 ≈ 强制 stackless 解释器，等于全引擎重写——否决 |
| Nova 式 reborrow | 编译期强制 rooting，但 ~800 个 bind/unbind 站点、人体工学差，作者自述 soundness 仍在研究——不选 |
| **显式 root registry**（SpiderMonkey `Rooted<T>` / V8 `HandleScope` 同族） | **选中**：纪律性风险用 generation 校验 + stress 模式兜底；是「运行时抓」而非「编译期消灭」，接受这一点 |

### 10.4 成本与风险（诚实清单）

- rooting 纪律是全 codebase 级人因风险（忘 root = 提前回收）；
- **契约谈判是真正门槛**：§13 钉的是可观察的确定性回收——临时对象立即
  释放、FinalizationRegistry/WeakRef 时机、内存 footprint 行为；换 tracing
  后对象死于 GC 时刻，冻结向量需逐案重审、`parity.md` 需修订——这是产品
  主人的决定，不是性能 PR；
- 内存 headroom：tracing 不能像 RC 在接近满堆时运行；分代 minor GC 控制
  暂停；
- write barrier 常驻税约几个百分点；
- 工期估计与方案 A 同级或更大。

### 10.5 中间路线（不动 §13，先拿走 RC 税的大块）

1. pinned atom / interned string 全体 immortal 化（`u32::MAX` 饱和先例已有）；
2. consume 点 move-not-copy 纪律 + 直线代码 retain/release 配对消除；
3. S2.1 快速释放单独立项（§6 第 5 项）。

中间路线落地后**必须重新量化** tracing 的边际收益——届时残余 RC 成本可能
只剩个位数百分比，S4 未必仍值得。

### 10.6 决策门禁（两个同时满足才启动 S4）

1. **量化门禁**：S3（E/A/B/D）落地且 §10.5 中间路线榨尽后，profile 归因于
   RC 机制的总成本（copy/retain/release/deferred/zero-queue/atom 计数）在
   目标负载上仍 **> 15–20%**；
2. **契约门禁**：产品主人书面接受修订 `parity.md` §13；
   FinalizationRegistry/WeakRef 时机语义重新审计；冻结向量重基线。

### 10.7 预期与证据

- S3 之上再 **1.3–2×**；分配/GC 密集负载（splay 类）数倍。
- 证据：RC Immix（OOPSLA 2013，附录 A.8）显示优化后的 RC 可追平分代
  tracing——「RC vs tracing」的差距大半在实现质量而非原理；CPython 以
  PEP 683（immortal objects）/ PEP 703（biased refcounting）给 RC 本身做
  手术，证明 RC 成本真实且值得重构。

## 11. 路线、预期与验证门禁

### 路线

E（重定基线）→ A（地基）→ B（差异化）→ D/C 按测量交替推进。每阶段
独立可回退，严禁跨阶段混合提交。S4（RC → tracing GC）不在此路线内，按
§10.6 双门禁另行决策。

### 预期（诚实口径）

- 累积约 **2.5–4×** vs PGO 后基线。对 QuickJS 的 10–25× microbench 差距
  （`performance-plan.md:319-320`）大概率收敛到**同数量级、仍差 2–5×**——
  见附录 B 的税种分析。
- 「超过 QuickJS」按轴成立：**算术/特化密集**（QuickJS 无 quickening）与
  **原型链/多态属性**（validity cell + 更深 IC vs QuickJS 静态快路径）可
  反超；纯字符串/正则轴不奢望（表示层同源）。
- 所有数字以方案 E 后的基线为分母；无 PGO 基线之前的对比数字不得引用。

### 验证门禁（每阶段）

1. `cargo fmt --check`；
2. `cargo clippy --locked --all-targets -- -D warnings`（1.88）；
3. `cargo test --locked --workspace --all-targets`；
4. Test262：`--check` → `--focused` → `--full` 零回归（运行前清理 `GIT_*`
   环境变量）；只产出 current-source receipt，**不改 `current.conf`**；
5. `python3 scripts/checks/check-source-layout.py` + rust-only 门禁；
6. 基准：`property_read_probe.py` + `scaling.py` + `run.py`（v8-v7 /
   microbench），串行、独立输出目录、receipts 齐全。**比较协议**：开工前
   保存一份无 PGO、无 LTO 的固定基线；每阶段只与上一阶段和该基线比较
   （两侧同 flags、无 PGO/LTO）；大阶段收尾建议（非强制）一次 LTO+PGO
   双方复核；跨协议对比须标注双方构建协议（细则见
   `scripts/benchmark/README.md`「Profile-guided optimization」与
   `s3-a-plan.md` §2）；
7. profiling 构建核对计数器无异常漂移；
8. 每阶段结束更新本文档对应章节的实测结果。

---

## 附录 A：外部证据

### A.1 CPython 特化解释器（PEP 659，3.11–3.14）

- 机制：字节码可变，通用指令执行 ~8 次后自重写为特化形并内嵌 IC；guard
  失败经饱和计数 deopt 回自适应形。<https://peps.python.org/pep-0659/>
- 速度：3.11 官方口径 geomean 1.25× vs 3.10（捆绑了零成本异常与调用改造，
  **无干净的 PEP 659 单独归属**）；PEP 自估特化贡献 10–60% 区间、
  super-instruction 另有一小部分。<https://docs.python.org/3/whatsnew/3.11.html>
- 重要旁证：3.13 的 copy-and-patch JIT 初期只有 2–9%——**特化解释器已经
  拿走了可达收益的大头**。<https://tonybaloney.github.io/posts/python-gets-a-jit.html>
- 经验：单态缓存优先于多态（简单且交替类型回通用形可接受）；deopt 必须
  是单指令重写，不做区域 bailout。

### A.2 Deegen / LuaJIT Remake

- 元编译器：从 C++ 语义描述自动生成 CPS tail-call 解释器 + 自动 quickening
  变体 + 自动 IC。其生成的解释器 geomean 比 LuaJIT 手写汇编解释器快
  ~28–31%（34 项赢 31；**注意当时 GC 未实现、对比关了 LuaJIT 的 GC**）。
  <https://sillycross.github.io/2022/11/22/2022-11-22/>；论文版 OOPSLA 2024
  <https://arxiv.org/abs/2411.11469>
- 可借鉴（无需元编译器）：IC 与 opcode 融合以去掉命中路径的间接跳转；
  hot/cold 变体分裂；「缓存幂等步、重放效果步」。
- 不可借鉴：GHC calling convention、IR 级原型统一——需要身在 LLVM 内。

### A.3 学术脉络

- Brunthaler quickening 论文称至 5.5×，但屡被拒稿、口径是 microbenchmark
  上限，只作方向参考。<https://arxiv.org/abs/2109.02958>
- Shannon  thesis（Glasgow 2011）是 PEP 659 的设计基础。
  <https://theses.gla.ac.uk/2975/1/2011shannonphd.pdf>

### A.4 tail-call / CPS 派发

- Wasm3 Massey 模型 + Steven Johnson 的 musttail 系统化：
  <https://github.com/wasm3/wasm3/blob/main/docs/Interpreter.md>
  <https://blog.reverberate.org/2021/04/21/musttail-efficient-interpreters.html>
- 现状：Clang/GCC 已有 musttail；**stable Rust 无 tail-call 保证**，`become`
  仅 nightly 且 x86 codegen 有已知问题（ARM64 上安全 Rust tail-call 解释器
  已能胜手写汇编）。<https://www.mattkeeter.com/blog/2026-04-05-tailcall/>
- CPython 3.14 tail-call 解释器 headline 9–15%，经 Nelson Elhage 复查大部分
  是绕过 LLVM 19 回归；**对好基线的真实收益 1–5%**，复制派发本身在现代
  核上 ~2%。<https://blog.nelhage.com/post/cpython-tail-call/>
- Wasmtime Pulley A/B：真实负载上 giant match 与 tail-call 互有胜负
  （bz2 上 match 快 13–19%，spidermonkey 上 tail-call 快 3–4%）。
  <https://github.com/bytecodealliance/wasmtime/issues/9995>
- wasmi 2.0：四种派发模式并存（direct-threaded 最快，indirect-threaded 慢
  10–15% 但 IR 小），整体 2.2×；教训含 Rust 1.92 DestinationPropagation
  合并分支站点事件。<https://wasmi-labs.github.io/blog/posts/wasmi-v2.0/>

### A.5 寄存器 vs 栈式

- 经典：Shi/Casey/Ertl/Gregg（VEE'05 + TACO 2008）寄存器机静态指令少
  43%、时间少 ~26.5%——但那是 thin-opcode JVM。
  <https://www.usenix.org/events/vee05/full_papers/p153-yunhe.pdf>
- 现代再评估：RegCPython（ACM TACO 2022）平均仅 **1.067×**（最好 1.287，
  最差 0.977）——fat-opcode 高级语言上派发占比小，省派发买不到多少。
  <https://dl.acm.org/doi/10.1145/3568973>
- 折中甜点位：Ignition 的「寄存器文件 + 隐式 accumulator」
  <https://v8.dev/blog/ignition-interpreter> 与 wasmi 的 `ireg`/`freg`
  累加器——TOS 缓存拿到大部分收益而无需格式革命。stack caching 原始文献
  Ertl PLDI 1995「栈顶 1 项驻留通常最优」。

### A.6 分支预测时代

- Ertl & Gregg PLDI 2003：老 BTB 上 switch 派发误预测 81–98%，复制 +
  动态超指令至 3.17×——硬件已变。
  <https://www.eecg.utoronto.ca/~steffan/carg/readings/optimizing-indirect-branch-prediction.pdf>
- Rohou/Swamy/Seznec CGO 2015「Don't Trust Folklore」：TAGE/ITTAGE 级
  预测器下，单点 switch 与复制 threaded 几乎打平。
  <https://inria.hal.science/hal-01100647/document>
- 结论：派发机制微调的期望是个位数百分比；减少派发次数（quickening、
  超指令、TOS 缓存）比优化单次派发更值。

### A.7 构建层

- BOLT：FDO+LTO 之上再 ~8%（无 FDO/LTO 时至 20.4%），HHVM 实测 ~7%。
  <https://arxiv.org/abs/1807.06735>
- V8 pointer compression 博客（32-bit 句柄/偏移的内存与速度收益：堆至
  −43%，CPU/GC 5–10%）——arena 索引是同一思想的安全版本。
  <https://v8.dev/blog/pointer-compression>

### A.8 值表示与 GC 的 Rust 先例

- Nova（安全 Rust ECMAScript 引擎）：值 = 内联标量或 u32 句柄的 enum，
  堆 = per-type arena，GC 时 compact 提局部性。<https://github.com/trynova/nova>
- Kiesel（Zig）：16B→8B NaN-boxing。
  <https://codeberg.org/kiesel-js/kiesel/pulls/37>
- **未发现任何生产引擎 NaN-box arena 索引**（均为 box 指针）：本方案的
  8B 编码是合理但无生产先例的设计，收益/成本（每次 deref 多 base load +
  bounds check）须自行测量——这正是方案 A 要求先建基线的原因。
- RC vs tracing：RC Immix（OOPSLA 2013）在 JVM/MMTk 上追平分代 Immix，
  但解释器里每次计数操作都是软件开销，无公开数据覆盖「索引 arena + 安全
  Rust」场景；本项目 S3 内由契约排除 tracing（S4 候选见 §10），此比较
  仅作背景。
  <https://www.cs.utexas.edu/users/mckinley/papers/rcix-oopsla-2013.pdf>

## 附录 B：为什么 C 的常数与安全 Rust 的税不会全消失

把差距拆成三类税，结论各异：

1. **自残税（可全消）**：32B 值、SipHash、全局 epoch、440B 混排 slot、
   `Result` 管道——这些与安全无关，是历史实现的债务。S0–S2 已证明这类可
   逐项消除，S3 的 A/D 继续。
2. **安全税（可摊薄，不可归零）**：
   - 每次堆解引用 = base load + bounds check（arena 下标），C 的
     `ptr->field` 是单条 load；解释器的下标来自字节码操作数，编译器基本
     无法证明范围，消除率低；
   - `RefCell`/借检查把「单线程可变性」从 C 的编译期（程序员纪律）挪到
     运行期（flag 读改写）；
   - 不可用的工具：computed goto、musttail、手写汇编派发（rust-only 门禁）、
     `get_unchecked`（政策保留）——各值个位数百分比，累加即税。
3. **契约税（结构性）**：
   - RC 次数：§13 钉死确定性 RC + 循环回收，值每次复制/销毁各一次计数。
     对 V8（tracing，复制纯 mov）是天花板；对 QuickJS 是同税——但 QuickJS
     的计数是对象头内 `ptr->ref_count++`，本项目是 arena 查槽 + `Cell`
     读改写 + 状态机分支，**同税不同价**，方案 A/D 在缩价差；
   - 健壮性门禁（`parity.md:77`：OOM/栈限/中断不 panic）要求边界保留
     可失败路径与回退分支，QuickJS 在同位置直接返回 NULL/longjmp，路径
     更短。
   
   量级：同设计下典型残留 **10–30%** 常数差。
4. **反向项**：安全换来的是激进重构不穿帮——quickening、IR 重写、typed
   arena 在 C 里是高危手术，在这里由类型系统兜底。税是常数项，设计收益
   是结构项；这正是「分轴反超」（§11 预期）的根据。
