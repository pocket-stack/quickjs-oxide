# S3-A 计划：8B 值表示——融合实施

> 状态：待实施。起点为 pre-A 代码基线，无前置实现资产。本文档是
> `performance-architecture.md` §4（方案 A）的实施计划与验收规则；若两处
> 表述冲突，**以本文档为准**。约束与证据附录继承
> `performance-architecture.md` §0/§11/附录。

---

## 0. 设计决定（动工前钉死）

### D1：内部值类型与转换层先行——`Value` 只是公共 API

**事实**：`Value` 由 `src/engine/api/mod.rs:28` 公开导出（`pub use
crate::engine::value::{JsString, JsStringError, Value}`），内部数百个文件
直接使用它。方案 A 要求内部 8B，必须把「公共值」与「内部值」拆开，否则
改动面失控。

**决定**：

1. 公共 `Value`（`src/engine/value/mod.rs:13-24`，含 `ObjectRef` 等带
   `Rc<Runtime>` 的 root 类型）**保持不动**，只在 `engine::api` 与宿主回调
   适配层出现；公共签名一个不改。
2. 新增 crate 内部类型 **`JsValue`**（`src/engine/value/js_value.rs`）：
   - W1–W5 为 **16B 句柄 enum**：标量内联（`Int(i32)`/`Float(f64)` 等），
     堆类型为 `{index: u32, generation: u32}` 句柄；
   - A4 阶段再编码为 **u64 索引 NaN-box**（`performance-architecture.md`
     §4.1），若实测不划算则停在 16B（§4.3 退路，已比现状 32B 小一半）。
   - 不实现 `Copy`/`Drop`；显式 `dup`/`release` 纪律见 §1.2。
3. 转换层仅两个方向、只挂在 `Runtime` 上：
   - `unroot`（进引擎）：`&Value → JsValue`（dup 堆边）与
     `into_jsvalue(Value) → JsValue`（消费 root，省一次 retain/release 对）；
   - `root`（出引擎）：`JsValue → Value`（retain + 包装 Rc root）。
4. **边界规则**：`Value` 不得出现在 `engine::api` 与转换层之外——评审
   规则，若便宜则加进 `scripts/checks/check-source-layout.py`。
5. **穿越点清单**（公共 `Value` 进出引擎的全部位置，转换层只挂这些点）：
   - `Context::eval` / `eval_bytes` → `Value`（`api/context/script.rs`）；
   - `Context::execute` → `Value`（`api/context/calls.rs`）；
   - `Context::take_exception` → `Option<Value>`（`api/context/mod.rs`）；
   - `Context::new_array_from_values(Vec<Value>)`（`api/context/objects.rs`）；
   - native 调用参数缓冲：`&[Value]` / `Vec<Value>`（`builtins/dispatch.rs`
     等），内部以 `RawValue` 持有、边界再 root；
   - promise jobs / module loader / test262 agent 均经 `engine::api` 或内部
     `RawValue`，没有额外的公共 `Value` 签名；
   - `adapters/native` 仅转导出 `engine::api`，`adapters/web` 只用 wasm 侧
     `wasm_bindgen::JsValue`，不直接持有引擎 `Value`。

### D2：String/BigInt 存储层句柄化，公共 `JsString` 不动

**事实**：`RawValue`（`src/engine/heap/identity.rs:200-224`）对
Object/Symbol 已是句柄（`ObjectId`/`Atom`），但 `String(JsString)` /
`BigInt(JsBigInt)` 仍是 `Rc` 负载（`value/primitive.rs:20`、
`value/bigint.rs:104-115`）。`performance-architecture.md` §6 的 typed
arena 清单只列了 Object/VarRef/Shape/Context/FunctionBytecode——不堆化
String/BigInt，8B 无从谈起。

**公共 `JsString` 保持 runtime-free 纯计算类型**，约束证据：
`from_static` 全仓 1377 处、`try_from_utf*` 853 处均为无 runtime 的纯
构造；`impl JsString` 约 183 个方法全部 runtime-free；
`value/collection_key.rs` 是显式「no heap access」纯模块；任何带 runtime
的公共字符串表示都会破坏尺寸目标与公共表面。句柄化只发生在**值存储层**。

**决定**：

1. `HeapNodeKind`（`src/engine/heap/identity.rs:127-133`）新增 **`String`
   与 `BigInt`** 两个独立 kind（不合并），与 §6 typed arena 对齐；新增
   `StringId`/`BigIntId` 句柄（`heap/identity.rs`，
   `{index: u32, generation: u32}`）。**arena 节点持有现成的
   `JsString`/`JsBigInt`**（Rc 负载原样）；对象槽、常量池等对
   String/BigInt 的边纳入既有事务化 retain/release 与 `Edges` 遍历。
2. **cycle 处理：cascade-only，永不做 anchor**——string 节点零堆出边
   （rope 子节点留在 `Rc` 树内，不进 arena），BigInt 无出边，平凡成立。
3. `StringRepr`/rope 算法与 `impl JsString` 整体不动；arena 节点只是
   `Rc` 的一个持有者。同一节点多次读出克隆同一个内部 `Rc`，`ptr_eq`
   身份快路天然保留，`same_representation` 语义不变。
4. **公共 `JsString`/`Value` API 零改动**（`Rc<StringRepr>`，runtime-free
   构造全保留），`adapters`/oracle 不受影响；
   `RawValue::String(StringId)`/`BigInt(BigIntId)`，`JsValue` 同；堆→值
   边界经 arena 解引用后克隆 `Rc`。
5. `AtomTable` 基本不动：`strings` 仍按 `JsString` 键、`released_strings` /
   `WeakJsString` 保持；§6.4 的 hash 缓存（`StringRepr` 头部缓存 hash、
   atom 表换 FxHash）作为独立项照做。
6. **内容相等适配**：`RawValue` 的 `PartialEq` derive
   （`identity.rs:200`）必须移除——句柄 id 相等 ≠ 内容相等。
   `collection_key.rs`（`same_value_zero`/`hash`）、StrictEq、switch
   字符串匹配改为「id 相等快路 + arena 解引用内容兜底」；动工先盘点
   全部 `RawValue` 相等性使用点。
7. BigInt：`RawValue::BigInt(BigIntId)` 全 arena（`Short` 也进 arena，其
   分配成本列为测量点）；**A4 开放决定**——NaN-box 下 short 收缩为
   **±2⁴⁷ 内联**（48-bit payload + kind tag，超出晋升堆句柄，语义透明），
   默认取前者，A4 开工时按测量复核。
8. 内存语义注意：字符串/BigInt 从「`Rc` 独立分配」变为「arena 节点 +
   free-list 复用 + generation」——teardown 的 `live == 0` 断言与
   `GcStats`/`HeapCounts` 公共诊断（`api/mod.rs:15` 导出）口径需同步
   更新。
9. **测量点**：瞬态字符串（concat/slice/`number_to_string`）的 arena
   churn 会推高 zero_queue 水位，而 zero_queue 非空使 IC 快路 decline
   （`ordinary_storage/ic.rs:27-35`）；测量必须含 string-heavy 负载，
   确认不放倒 IC 快路。

### D3：`Atom` 内部 `u32`，品牌只留边界

**事实**：`Atom { raw: u32, generation: u32, table_id: u64 }`
（`src/engine/atom/mod.rs:52-57`）16B，相等比较逐 16B；shape 线性扫描
（≤8 项）与迁移表键全在吃这个体积。

**决定**：

1. 新增内部类型 **`AtomIdx(u32)`** newtype；保留 immediate-int 高位 tag
   （`ATOM_TAG_INT`，QuickJS parity 不动）。16B branded `Atom` 只留公共
   API（`PropertyKey` 等）与跨 runtime 进入点。
2. **存活不变量**（与 `live_node_fast` 同一论证）：内部 `AtomIdx` 只能由
   「已 retain 该 atom 的 owner」持有——shape entry、字节码
   `property_key_atoms`、pinned 集。可信路径免品牌校验（debug 构建全量
   校验），边界全量。
3. `AtomTable::Entry.ref_count` 改 `Cell<u32>`，retain/release 在共享借用
   下完成——`Symbol` 从所有快路 decline 名单移除。
4. 级联：`ShapeEntry`（`object/shape.rs:73-77`）24B→~8B（u32 atom +
   flags）；shape 迁移表键、shape fingerprint 同减；
   `RawValue::Symbol/Private` 负载 16B→4B，为 `RawValue` 8B 化扫清最后
   一个超标变体。
5. `Atom` 的 `Hash` 现为 `generation<<32|raw`（`atom/mod.rs:59-65`）；
   内部 `AtomIdx` 直接以 raw 作 hash（Fx），不再移位拼装。

---

## 1. 终态设计

### 1.1 类型格局

- **`JsValue`**（crate 内部，16B enum）：标量内联 +
  `String(StringId)`/`BigInt(BigIntId)`/`Symbol(AtomIdx)`/`Object(ObjectId)`；
  无 `Copy`/`Drop`。
- **`RawValue`**（堆存储形态）：同一套句柄 + `Private`/哨兵；与 `JsValue`
  互转是无分配的同 id 拷贝。
- **`Value`**（公共）：只在 `engine::api` 边界与宿主回调适配层出现（D1）。

### 1.2 所有权纪律（一条规则）

每个存储位置（堆槽、帧、操作数栈、记录、常量池、pending_exception）持有
其句柄的一条边：

- **store**：拷贝入库 → retain；**move 入库 → 交接生产者边，不产生计数对**；
- **读出**：dup（retain）交出 owned `JsValue`；纯读取可原地借用；
- **overwrite / pop / finalize**：release；
- **String/BigInt 节点只在真创建点分配**：字符串/大整数产生运算、字面量
  publish、api/host 输入转换；**store 永不分配**；
- dup/release 走既有快路纪律（可信 `Cell` retain；release 经
  `release_or_defer`）；
- **无 RAII 包装**：`JsValue`/`RawValue` 均无 `Drop`，所有权全靠上述显式
  纪律——任何「自动释放」包装都会把堆访问需求带进值类型的 drop 路径，
  与「值的 drop 不需要堆」的既有纪律冲突。

## 2. 借用与分配放置规则

1. **转换提出借用区**：值→`RawValue` 的转换必须发生在任何 `state` 借用
   之外。值刚从持有借用的结构读出的路径，先结束借用、转换、再重新借用
   ——单线程引擎、两次借用之间无 JS/native 回调，拆分借用语义不可见。
2. **物化沉进事务**：批量/事务性存储路径（`retain_edges_transactionally`、
   publish、dense 写）把 String/BigInt 节点分配放在事务内部（本来就持
   `&mut`、本来就走边），同 shape 分配先例（`get_or_create_shape`/
   `append_transition` 在持有 `&mut RuntimeState` 的 store 事务内分配堆
   节点）。
3. **借用拓扑审计先于编码**：`RefCell` 借用次序是运行期行为，编译器抓
   不到。每个工作流动笔前，先列出它触到的热路径调用点当前的借用持有
   情况（谁持 `state` 借用、转换/分配放在哪一层），按规则 1/2 放置后
   再写代码。重点审计：`property_ic_write_scalar` 及 IC 写路径、dense
   写、bytecode publish、挂起/恢复、`raw_property_value` 全部调用点。
4. **升级条款**：某条路径疑似无法提出借用区时，举证标准 = 两次借用之间
   存在 JS 可观察行为；成立则对该点用规则 2。两条都走不通才允许复审
   「独立 `RefCell` 侧 arena」方案（拆锁式治标，与 §6 typed arena「物理
   拆分、纪律统一」方向冲突），**不允许静默采用**。

## 3. 执行序列（compiler-driven）

回退单位 = 整个大阶段（分支级）；中间 commit 不要求可编译；WIP commit
只留本地，推送以绿为准。

| 工作流 | 内容 |
| --- | --- |
| **W1** | 句柄与转换层地基：`HeapNodeKind::String/BigInt` + typed arena（allocate/retain/release/finalize/counts + trusted 访问器）；`StringId`/`BigIntId`/`AtomIdx` 句柄；`AtomTable::Entry.ref_count` `Cell` 化；`JsValue` 与四转换函数（unroot/dup/release/root）完整实现；deferred release 通路 |
| **W2** | 堆存储层：`RawValue` 句柄化（String/BigInt/Symbol/Private 全句柄，移除 `PartialEq` derive）+ `raw_value_edges` + 事务 retain-release + collection_key/index heap 化；`raw_property_value` 退役为纯 strip，仅供边界 |
| **W3** | VM 核心：`FrameBinding`/`SlotStore`/`run.rs` → `JsValue`；帧建立/拆除、挂起编解码、调用约定的显式 dup/release/move |
| **W4** | builtins 与 drivers 签名 `Value`→`JsValue` |
| **W5** | api 边界：`eval`/call/host 回调/promise jobs/module loader 的唯一 `Value`↔`JsValue` 转换层 |
| **W6** | 测试适配 + 全门禁 |

A4（NaN-box 编码）为独立的测量门禁后续阶段，不在本序列内。

## 4. 验收规则

**设计一致性（评审第一顺位）**：

1. 每个引入的类型/函数/构造必须属于 §1 终态设计；仅用于让中间态编译
   通过的临时构造一律不接受——编译器报错要求的改动，要么按终态设计
   改到底，要么不改。
2. 借用与分配放置符合 §2；任何新增分配点必须能指出它属于「真创建点」
   或「事务内部」。
3. 公共表面不变：`tests/checked_string_construction.rs` 零改动通过；
   `engine::api` 签名、`adapters/*` 零改动；任何需要改公共测试的迹象
   即警报。

**尺寸断言（编译期钉死）**：`JsValue` = 16B；`AtomIdx` = 4B；
`ShapeEntry` = 8B；`RawValue` ≤ 16B。

**语义门禁**：`RawValue` 相等性使用点全部改为「id 快路 + 内容兜底」，
SameValueZero 语义逐点核对（`collection_key`、StrictEq、switch 字符串
匹配）；teardown `live == 0` 断言与 `GcStats`/`HeapCounts` 口径适配。

**全门禁（大阶段末一次）**：`cargo fmt --check` → clippy 1.88
`-D warnings` → `cargo test --locked --workspace --all-targets` →
Test262 `--check`/`--focused`/`--full` 零回归（清理 `GIT_*`，只产
current-source receipt，不改 `current.conf`）→
`check-source-layout.py` + rust-only 门禁 → benchmark receipts
（`property_read_probe.py` + `scaling.py` + `run.py`，串行、独立输出
目录，协议见 §5）→ profiling 计数器无漂移 → 实测结果记入本文档。

**委托执行交底**：任务拆分委托时，提示词必须包含 §1 终态设计、§1.2
所有权纪律、§2 放置规则与「临时构造不接受」条款；评审先看设计一致性，
再看编译。

## 5. 阶段性能比较协议

1. **固定基线**：S3-A 开工前保存一份基线——无 PGO、无 LTO 的 release
   构建（`CARGO_PROFILE_RELEASE_LTO=off
   CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16`），全量基准数字记入报告，
   作为所有阶段的固定分母之一；
2. **每阶段只比两次**：本阶段 vs 上一阶段、本阶段 vs 保存的基线；两侧
   都是无 PGO、无 LTO 的同 flags 构建。**不做每阶段 PGO 重训**；
3. **例外与复核**：E 阶段测的就是构建配置本身，按
   `performance-architecture.md` §3 已有口径；每个大阶段（A/B/D）收尾时
   **建议**（非强制）做一次 LTO+PGO 双方复核——LTO 会改变内联与代码
   布局，无-LTO 下的阶段胜率偶尔会在最终构建配置下翻转，复核只为确认
   符号不变；
4. 跨协议对比允许用于累计/用户口径的报告，须标注双方构建协议。

## 6. 风险

- **手工 RC 纪律扩大 panic 面**（VM 接线起）：debug 构建维持全量
  generation 校验 + 冻结向量兜底；trusted 访问器遇 stale 即 panic 的
  政策不变。
- **触及面全仓最大**：值类型是所有模块的公共依赖；融合大扫除期间允许
  长时间不绿，回退单位是整个大阶段（分支级）。
- **8B 索引 NaN-box 无生产先例**（附录 A.8）：每次 deref 多一次 base
  load + bounds check；A4 必须以测量定去留，退路（16B enum）不是失败
  而是默认值。
- **行为敏感点**：集合键 SameValueZero 与内容 hash（`collection_key.rs`）、
  StrictEq/switch 的字符串路径、teardown `live == 0` 断言、
  `GcStats`/`HeapCounts` 口径；`same_representation` 与
  `released_strings` 在 D2 下**不变**（公共 `JsString` 不动）。
