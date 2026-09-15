# S14–S20 执行计划（工序级）

**本次执行指令（覆盖旧验收流程）：**分阶段提交全部实现，代码全部完成后额外逐项覆盖 review，遗漏补齐后仅对最终新核心执行一轮 benchmark/Profile；S0 与 QuickJS 复用本地已有数据。实现期间不运行 benchmark、cost profile 或 CPU Profile，不按阶段重复测量。测量结果仅保留在本地 `target/performance-retained/`，不提交 Git。阶段提交已获授权，无需再次等待提交指示。

2026-09-15 制定。本文件把 [S14–S20 修复计划](primitive-vm-s14-s20-recovery-plan.md)（根因、验收目标、覆盖矩阵的母文档）展开为**可直接执行的逐工序清单**：每步指明文件/函数/具体改动、语义红线、测试与验证命令。所有代码锚点已在当前工作树逐一核对；实现时若发现锚点漂移，以"先修订本文件再动代码"为序。

## 0. 每阶段通用流程

每个阶段（一个提交单元）严格按以下顺序执行，任何一步失败不进入下一步：

1. **前置设计修订**（如该阶段有列）：先改设计文档并自查一致性，再动代码。
2. **实现**：只做本阶段清单内的改动；实现期间**不做任何 benchmark/Profile**。
3. **语义门禁**（全部通过才算完成实现）：
   ```sh
   cargo test --locked --workspace --all-targets --features stack-vm
   cargo test --locked -p quickjs-oxide-cli --features stack-vm,profiling --test profiling --bin qjs
   cargo test --locked -p quickjs-oxide --features stack-vm,profiling --lib profiling_
   cargo test --locked --workspace --doc --features stack-vm
   cargo test --locked --workspace --features stack-vm,test262-host --lib --bins
   cargo clippy --locked --workspace --all-targets --features stack-vm -- -D warnings
   ```
   加 Test262 结果向量逐位对齐与边界矩阵（沿用 S10–S12 验收轮的既有流程与脚本，见 [profiling.md](profiling.md) 与 `scripts/checks/`）。
4. **覆盖 review**：全部阶段代码完成后，对照各工序逐项核对实现与测试，发现遗漏补齐。
5. **最终单轮测量**：只运行最终新核心一轮完整 benchmark/Profile（包含机械计数核对），S0 和 QuickJS 读取已有数据；原始环境、失败与差值如实保留，不补样或重跑旧版本。
6. **报告与提交**：实现及状态文档按阶段分别提交；性能报告与原始数据写入本地 `target/performance-retained/`，不提交 Git。

回滚策略：每阶段单 commit，回退即 `git revert` 单提交；阶段内不引入跨阶段共享的新公共接口（S17.4 pin atom 表是唯一例外，S18/S19 消费）。

## S14 — 字符串/转换内核

前置：无设计文档修订。纯 value/builtins 层，不触 VM 协议。

### 工序

**14.1 数字→字符串 ASCII 直通**（`src/engine/value/mod.rs:134-166`）

`Value::to_js_string` 的 `Int`/`Float`/`BigInt` 三个分支产物恒为 ASCII（十进制数字、`-`、`.`、`e`、`+`、`Infinity`、`NaN`；BigInt 为十进制数字与 `-`）。把出口 `JsString::try_from_utf8(&text)` 改为对这三个分支走 `JsString::from_owned_latin1(text.into_bytes())`（`value/primitive.rs:956-959`，已存在未被使用），`Undefined/Null/Bool` 字面量分支一并直通。加 `debug_assert!(text.is_ascii())`。消除 `String → Vec<u16> → Vec<u8> → Rc` 双转码（保留 `try_from_utf8` 给非 ASCII 调用方）。同法排查 `number_to_string` 的其他消费点（如 `Number.prototype.toString` 十进制路径）是否同样可直通，仅改证据确凿的。

**14.2 `try_concat` 左空串短路**（`src/engine/value/primitive.rs:1677-1698`）

`other.is_flat()` 分支顶部（现有 `other.is_empty()` 短路之后）补：

```rust
if self.is_empty() && self.is_flat() {
    return Ok(other.clone());
}
```

限定 `is_flat` 以避免对空 rope 状态做未审计的假设。命中形态：int_to_string 的 `n + ""`。

**14.3 flat 串 in-place 追加**（`primitive.rs:137-141`、新增 API、接线 `vm/numeric/operation.rs`）

1. 表示变更：`StringRepr::Latin1(Box<[u8]>) → Latin1(Vec<u8>)`、`Utf16(Box<[u16]>) → Utf16(Vec<u16>)`（尺寸同为 3 usize；容量槽即 QuickJS refcount==1 原地 realloc 的等价物）。`grep -n 'StringRepr::Latin1\|StringRepr::Utf16' src/` 逐点修模式匹配（构造点 `into_boxed_slice()` 删除即可）。
2. 新增 `JsString::concat_owned(self, other: &JsString) -> Result<JsString, JsStringError>`：当 `Rc::strong_count(&self.0) == 1 && Rc::weak_count == 0` 且 self/other 均 flat 且宽窄兼容时，`Rc::get_mut` 取可变引用，`try_reserve`（**摊还增长，不用 `try_reserve_exact`**）+ `extend_from_slice`，长度守 `MAX_LEN`；否则回落 `try_concat`。窄 self + 宽 other 不做原地（需加宽重分配，走既有路径）。
3. 接线：`vm/numeric/operation.rs` 的 `add_primitives`（`:123-125` 引用处）字符串结果分支——左操作数以 `Value::String` **所有权**进入 `primitive_output`，满足 strong_count==1 判定的即走 `concat_owned`。`r += "x"` 循环由 O(n²) 变摊还 O(n)。
4. 语义红线：rope 阈值行为（`ROPE_SHORT_LEN/ROPE_SHORT2_LEN`）与 `linearize` 的 `Linearized` 状态迁移不变；`JsString` 相等/哈希不受容量槽影响（不比较 capacity）。

**14.4 `complete_primitives`/`complete_local_add` 冗余折叠**（`src/engine/vm/conversion_driver.rs:125-299`、`conversion_driver/local_add.rs`）

- 144-148 与 184-188 的两遍 peek 收敛为一遍（peek 结果传参）；同一局部的 3 次 `local_current` 收敛为 1 次绑定。
- `complete_local_add` 的 7 次 `local_current`、3 次 `local_add_values` 同法收敛。
- 纯等价重构，不改任何观察点/PC 语义。

**14.5 `MathStep::start` 的 Box 延迟**（`src/engine/builtins/math/operation.rs:89`）

全原语实参路径（`math_completed_without_argument_storage` 事件对应形态）不分配 `Box`；仅当确需跨回调挂起（有 Object 实参要 ToNumber 回调）才 `Box::new`。`MathResume ≤ 8B` 的尺寸断言保留。

### 测试与验收

- 新增单测：`to_js_string` 数字/BigInt 直通后表示为 Latin1（借 `is_wide()` 断言）；`"" + x`/`x + ""` 身份短路；`concat_owned` 的独占/共享/宽窄/超长四象限；`r += s` 循环结果与逐次 `try_concat` 一致（含 rope 边界跨越）。
- 计数验收（profiling cost profile）：int_to_string、string_build1/3/large1/large2 的 `try_from_utf8`/`from_validated_utf16` self 归零；math_min 分配计数每调用 −1。
- 三轮复测目标：string_build 系列与 int_to_string 相对 S0 显著收敛（终值目标在 S15 后判定）。
- 提交：`perf(value): eliminate redundant conversion copies and enable in-place string append`

## S15 — run 驻留谱系补全与退出税削减

前置：**先改 `docs/architecture/owned-fusion.md` 的 "Resident primitive arithmetic boundary" 章节**——按既定措辞把"ConvertAdd 不建立 run 驻留"改写为 S15 落地后的新边界（N1 模式：`drop(slots)` → 存活 FrameTransaction 内分配 → 重开短借用）。

### 工序

**15.1 Add 接入既有驻留分支**（`src/engine/vm/run.rs:1493-1499`）

把

```rust
Instruction::Add => {
    if binary(&mut slots, |a, b| value(a.add(b)))? { true } else { return Ok(RunExit::ConvertAdd); }
}
```

改为与 `Sub/Mul` 同形：`Instruction::Add => binary(&mut slots, |a, b| value(a.add(b)))?`。非 Number 对自动落入 `run.rs:1683` 的 `!handled` 通用分支——那里 `NumericKind::for_instruction` 已映射 Add（`numeric/operation.rs:52`）、`numeric::supported` 已放行（`primitive_arithmetic()` 含 Add，`operation.rs:85-87`）、`primitive_output` 已实现 Add 语义（`operation.rs:123-125` → `add_primitives`）。Object 操作数 `supported()` 拒绝 → `RunExit::Numeric(Add)` 走既有 Phase 机器（`operation.rs:195-260` 已支持 Add 的 ToPrimitive 回调）。BigInt/Symbol 先物化判定复用 `run.rs:1689-1695` 现成逻辑。

执行前必须核对：冷驱动器对 `RunExit::Numeric(Add)` 与原 `ConvertAdd` 的处理等价（转换顺序、hint、错误 PC）；`RunExit::ConvertAdd` 变体**保留**给 AddStore/LocalAdd 融合路径（本单元不动融合 span，留待计数证明后单独迁移）。

**15.2 StrictEq/StrictNeq 驻留**（`run.rs:1557-1566`）

else 分支不再立即退出：先按值域分类——

- Bool/Null/Undefined 任意组合、Int/Float×非数字混合：`Value::strict_equal`（`value/mod.rs:170` 既有内核）纯计算，直接驻留。
- Object/Symbol 引用相等：结果为 Bool，但**弹出比较对会释放两个 owner** → 套 `release_outside_slots!`（`run.rs:263-279` 宏，`run.rs:1483-1489` 有同形用例）。
- String×String：双 flat 时 `JsString` 相等内核驻留；任一为 rope（linearize 需分配/改共享状态）→ 仍 `return Ok(RunExit::StrictEquality(negate))`。
- 其余混合类型对：结果恒 `false != negate`，驻留 + `release_outside_slots!`。

**15.3 Not 驻留**（`run.rs:1569`）

`Instruction::Not` 改为：ToBoolean 是全值域纯函数（找 `Value::to_boolean`/等价内核，无则以 QuickJS 语义在 value 层补一个纯函数），结果入栈；Object/Symbol 操作数释放套 `release_outside_slots!`。删除 `RunExit::LogicalNot` 的产生点后，**该变体与其冷路径一并删除**（窄状态机原则：无到达路径的变体不保留）。

**15.4 DefineField 快路径**（`run.rs:831-842`，依赖注记）

fresh Ordinary 接收者 + 标量值 + 无同名既有属性时在 run 内直通 define。**标量版不依赖 S17**；引用值版与转移表联动，在 S17 落地后回补一小步。若实现中发现 define 必经的 shape 全表哈希使驻留收益被 R4 吞没，允许把本工序整体顺延到 S17 之后执行（在阶段报告记录顺延与理由，验收计数随之顺延）。

**15.5 delete 快路径**（`run.rs:480`）

own data 可配置属性的 delete 驻留。与 14.4 同理存在 R4 依赖（删除走全量 clone 路径），默认顺延到 S17 后回补；S15 仅在计数证明"退出税占比 > 存储成本"时先行。

**15.6 退出税削减**（`driver/ready.rs:36-38`、`vm/frame.rs:164-196`）

1. `RunExit` 上新增 `fn observes_activation(&self) -> bool`（默认 true），`ready.rs:36` 的 materialize 条件改为 `exit.observes_activation()`；首批豁免 = 现状 Call/Complete（**语义不变的重构**，为后续豁免名单提供单点）。
2. 已物化帧跳过重复发布：`FrameStore::materialize` 对 `is_materialized()` 帧比较"上次发布的 PC"（Frame 新增 `published_pc: Option<u32>` 或复用现有水位字段），`fault_pc` 未变则跳过 `publish_materialized_pc`。这是 depth 探针族每迭代 4-12 次 `runtime_pc_publication` 的直接削减项，**S18 依赖此步**。
3. 观察协议"拉"式改造只做设计评估（写入 S16 前置修订），本单元不实施。

### 测试与验收

- 新增/扩展单测（照抄 `run/numeric.rs:87-136` 的 `numeric_completed_in_run` 断言形式）：Add 的 String/BigInt/混合驻留 + `run_exit.ConvertAdd == 0`；StrictEq 全值域矩阵（含 `Object.is(-0,0)` 区分、NaN、rope 回退）；`!obj` 释放时序；错误 PC/栈锚点既有测试（`local_add.rs:129-242`）不动全过。
- 计数验收：string_build/int_to_string/bigint64 `run_exit.ConvertAdd` → 启动量级；earley-boyer `run_exit.StrictEquality` 1.25M → 启动量级、`run_exit.LogicalNot` → 0（变体已删则天然满足）；上述用例逐退出 `runtime_pc_publication` 消失；depth-js 探针无回退（豁免重构不伤普通调用）。
- 提交：`perf(vm): complete run residency for add, equality and definition operations`

## S16 — 属性协议第三期（IC 覆盖面与写路径）

前置：**补写设计章节**（新增于 `docs/architecture/` 或扩展现有属性文档）：IC 命中/失效语义——shape revision、prototype epoch、realm/domain 键、多态槽替换策略、写侧失效矩阵（delete/defineProperty/setPrototypeOf/字典化/跨 realm/Proxy revoke）。

### 工序

**16.1 放宽接收者白名单**（`object/property_ic.rs:120-126` `ordinary_receiver` 与 `:128` `locate`）

`ordinary_receiver` 从 `(Ordinary, Ordinary) | (Array, !numeric)` 扩展为"属性查找走 ordinary shape 存储的全部 kind"（Map/Set/WeakMap/Date/RegExp/Error/Arguments 等 payload；**排除** Proxy、以及 numeric key 时具索引 exotic 语义的 Array/TypedArray/String）。逐 kind 列清单并对照各 kind 的 `internal_get` 实现确认无 exotic 读拦截；`locate` 的原型链走查对新 kind 的 holder 同样成立（方法命中形态：exotic 接收者 + Ordinary prototype 上的 data 属性，`depth>0` 路径）。

**16.2 GC 门控收窄**（`object/ordinary_storage/ic.rs:27-35`）

现状：`deferred_references.has_pending()` 或 `has_pending_zero_cleanup()` 任一为真即整体旁路。改为：`keep_receiver == true` 的命中路径（只读 + 对结果 retain，不释放任何 owner）跳过这两个门控——`:53-57` 的 readiness 证明本就只服务 `!keep_receiver` 的接收者释放。需先写一段借用/时序论证（命中读期间 heap 借用独占，retain 不进 zero-queue），附在前置设计章节；论证不成立的分支保持门控。

**16.3 单态 → 2 路多态 + 复活**（`property_ic.rs:24-39,102-117`）

- `State` 增 `Polymorphic([Location; 2])`（保持 `Cell<State>` 的 Copy；2 路上限是尺寸取舍：`Location` ≈56B，4 路会把站点表推到 ~240B/站，先 2 路 + 溢出 Megamorphic，多态命中率计数决定是否加宽）。
- `misses >= 2 → Megamorphic` 改为：第二个不同 Location 升级 Polymorphic；第三个不同 → Megamorphic **但记 `revive_after: Cell<u16>` 计数**，衰减到 0 时复活为 Cold（阈值起点 1024 次读，计数证据调参）。
- `read()` 按序探测两个 Location；命中把该槽换到首位（近期优先）。

**16.4 写 IC**（新增 `PropertyWriteCache`，站点=PutField；`object/ordinary_storage.rs:992-1047` 接线）

- `code/executable.rs:264-269` 的站点表构建扩展到 `PutField`（`indices` 合并一张表，`sites` 分读/写两段或统一 enum）。
- `Location{shape, revision, slot}` + `writable` 位；命中直通 `replace_data`（`ordinary_storage.rs:1046` 已有内核）；**去掉"新值与旧值都是标量"限制**：引用新值在命中路径 retain，被替换引用值走 zero-queue 既有释放协议（与 16.2 的论证共用）。
- miss/失效与读侧同一套 r/epoch 键。

**16.5 元素读写**（`run.rs:441-446`、`run.rs:356-364`）

- `GetArrayEl2/3`：复制既有 `GetArrayEl` 驻留形态（dense 界内标量读直通，保留接收者变体多推一次栈）。
- `PutArrayEl`：普通 dense 数组界内写快路径（标量直写；引用值写依赖 16.4 的 retain/释放协议）。

**16.6 全局读缓存**（`heap/roots.rs:181-241`）

`GetVar` 未解析全局：首读成功后回填"global shape revision → slot"缓存（站点侧或 global cell 侧二选一，取改动面小者：cell 侧回填不需要扩站点表）。失效键 = 全局对象 shape revision。消 math_min 每调用的全局大 shape SipHash find。

### 测试与验收

- 失效矩阵测试（每项一测）：命中后 delete / defineProperty 改 accessor / setPrototypeOf / 字典化阈值跨越 / 跨 realm 同 shape / Proxy revoke —— 均回退正确语义；写 IC 的 writable=false、setter 遮蔽、freeze。
- 计数验收：splay `run_exit.GetField` 808k → <5 万且 `property_ic.hit` 相应上升；crypto `run_exit.GetElement` 1.5M、`run_exit.SetProperty` 869k → 启动量级；weak_map_set `property_storage_read_probe` 1.28M → 启动量级；math_min `property_storage_read_probe` 1.1M → ~0。
- 提交：`perf(vm): extend property caches to exotic receivers, writes and globals`

## S17 — shape/atom/哈希结构

前置：**补写 shape 转移表的字典化边界设计**（转移表只服务 append 形态；delete/重配置/超阈值走 fingerprint 兜底并使源 shape 的转移边失效）。

### 工序

**17.1 per-shape 转移表**（`heap/runtime/mod.rs:248-285` `get_or_create_shape`、`object/operations.rs:16-20`）

Shape 侧挂 `transitions: HashMap<(Atom, PropertyFlags), ShapeId>`（或全局副表 keyed by parent ShapeId，取对 `ShapeId` 生命周期/回收更安全者——`unlink_finalized_shapes` 必须同步清边）。`get_or_create_shape` 的 append 形态先查转移边，miss 才走 `ShapeFingerprint` 全表哈希并回填。删除/重配置不建边。

**17.2 小对象路径**（`object/properties.rs:39,1618,1634-1680`）

`MIN_UNIQUE_SHAPE_APPEND_ENTRIES` 8 → 1（转移表使小 shape 复用免于全量拷贝）；delete 分支 B 的 2 次全量 `to_vec`/`slots.clone` 收敛为单次原地重建。**转移表先落地再降阈值**（同一提交内两小步，计数分别记录）。

**17.3 hasher 与键**

- `Shape.lookup`（`object/shape.rs:123`）：entries ≤8 直接线性扫 atom（不建 HashMap）；>8 换 FxHash（无第三方依赖政策则手写 fx 乘法 hasher 于 `engine/hash.rs`，~40 行）。
- `Atom` 的 `derive(Hash)` 3 次 write（`atom/mod.rs:52-57`）收敛为单次 `write_u64`（打包 index+generation）。
- `CollectionIndex`（`heap/collection_index.rs:99-121`）：外层 `HashMap<u64,_>` 换 nohash（键已是哈希值）；`set_map_record` 的 3 次 `raw_property_value`（`builtins/map.rs:406-441`）收敛为 1 次。
- 短字符串/Int/Object 键的哈希缓存进 record（现仅 ≥256 单元长串）。

**17.4 常量 atom 预驻留**（`atom/mod.rs:496` `intern_static` 已支持）

RuntimeState 建 `PinnedAtoms` 表：`length/next/return/done/value/index/input/groups/lastIndex/exec/get/set/has/apply/deleteProperty/toString/valueOf`（S18/S19 消费 `get`、`lastIndex` 等）。调用点改引用：`properties.rs:879,1019,1108`、`iterator_driver.rs:729-733,750-755`、`object/internal_methods/method.rs:59` 等（`grep -rn 'intern_property_key("' src/` 全量 80+ 处逐一改，无一处遗留字面量热调用）；`iterator_driver.rs:891-894` 的整数 key 改 `property_key_for_index`（`atom/runtime.rs:29-38` 免 intern 先例）。

**17.5 array_slice 批量旁路 + prop_clone 瘦身**

- slice：自有 dense source + fresh fast array + 无 species/hole/索引原型时整段拷贝（照抄 `SliceKind::ToSpliced` 的 `values[] + new_array_from_values` 形态）；dense 快路绕过 `define_array_index` 的 descriptor 三次结构转换。
- prop_clone：`own_property_keys` 预留容量；`clone_copy_key/clone_copy_object` 每属性 retain 对改借用游标。

附带项（R10）：`code/fusion.rs:29/90` 预筛与主循环合并为单遍。

### 测试与验收

- 语义红线测试：属性枚举序、字典化阈值行为、delete 后 shape 复用正确性、跨 realm shape 不串边、`Map/Set` 迭代序与 NaN/±0 键语义。
- 计数验收：prop_clone `copy_owner_clone.PropertyKey` 320k → 0、SipHash self 21.2% → <3%；prop_delete SipHash 14.6% → <3%；array_slice `intern_property_key_js_string` self 8.5% → ~0、`property_storage_read_probe` 880k → 启动量级；local_destruct intern self 3.1% → ~0。
- 提交：`perf(object): add shape transition table, cheap hashing and pinned atoms`

## S18 — 调用外围：native 帧与属性驱动被调帧惰性化

前置：**补写属性驱动被调帧的惰性观察协议边界**（哪些 operation/property_wait 状态允许惰性 token；proxy 陷阱返回快速回执的语义与 backtrace 契约），并入 `vm/call/protocol.rs:1-15` 的 I1–I5 注释与设计文档。依赖：S15.6 的"已物化帧跳过重复发布"。

### 工序（按内部验证顺序，仍一个提交单元）

**18a getter 回调帧惰性化**（最小闭环先行）

1. `property_driver.rs:645-659` 的 `BytecodeCallRequest`（`operation: None`）改走 `OrdinaryCall` 等价 install：惰性 token（`call/ordinary.rs:250` `ActiveFrameToken::unmaterialized()`）、`push_ordinary_frame`、免 `original_arguments` Vec（getter 零实参，thisValue 单独携带）；`call/prepare.rs:106-146` 增 lazy 构造变体或参数化 `prepare_bytecode_header` 的 activation 策略，**不改既有急切调用方的行为**。
2. `property_driver.rs:602-632` 的 `normalize_callback` 重分类 + function_kind/native_operation 两次 heap borrow 折叠为一次分类结果复用。
3. PropertyKey 每读重建（`property_driver.rs:224-241`）改借用/缓存于站点。

**18b proxy get 陷阱外围**

1. 陷阱名 atom：`object/internal_methods/method.rs:59` 用 S17.4 的 pin 表（若 S17 未先行，本步独立 pin `get`）。
2. 4 Box 收敛：`MethodResumeState`（`method.rs:101-106`）、`ProxyGetResumeState` ×2（`get.rs:112-130,157-168`）、`PendingProxyGet`（`proxy_get_driver.rs:1550-1576`）——resume 状态内联进 `Query` 存储或走 `query_storage` 池化（既有 acquire/recycle 先例）；3 元素实参 Vec 改复用缓冲。
3. 陷阱帧（`operation: Some(PropertyGet)`）返回快速回执：`frame.rs:765-784` `ordinary_return` 对 `operation.is_some()` 的拒绝，改为对 PropertyGet 操作提供"直达 reply"路径（弹帧 + 交值 + `proxy_get_driver::reply` 短接），消 `frame_exit.rs:71-161` 完整冷退出与 `driver.rs:802` 的二次 materialize。invariant 阶段（`get.rs:157-214`）语义不变。
4. 陷阱帧本身接 18a 的惰性 install。

**18c native 调用外围**

1. publish 延迟：`call/native.rs:315-316` 的 `publish_validated_active_frame` 挂惰性 token（复用 S10 机制），确有观察者（backtrace/异常/宿主回调/挂起）才发布；`driver/ordinary.rs:113` 的强制物化改按 `observes_activation` 判定。
2. witness 缓存：`NativePublicationWitness::validate`（`vm/frames.rs:147-205`）的重验证结果随 `NativeClassification` 缓存，classification 复用时跳过（失效键 = 函数对象身份 + realm）。
3. argv 单搬运：`call.native_argv → native.readable` 双缓冲合并（`vm/stack.rs:204-240` 的 take 直接产出 readable 视图）；`reserve_native_argument_depth` 每调用 2 次 → 1 次；`try_reserve_exact` → `try_reserve`；`resize(available, Undefined)` padding 只在 `actual < min_readable` 时执行。
4. retain 削减：`promote_selected/linked` 双 retain（`frames.rs:34-35,56-57`）、publish 的第 3 次 function root（`frames.rs:259`）、Pure 族 `invocation.clone()` 的 receiver retain（`builtins/continuation.rs:39`）改借用证明——调用期间 slot/argv owner 已钉住被调者。
5. waitable native（`proxy_get_driver.rs:669-800`）：`query.natives/spare_parents` 双 reserve 与 operation identity 簿记按稳态复用。

**18d IC 联动与 Map 内部表**

1. 通用读路径补 `LinkedNativeSelection` 产出（`object/access.rs:122-127`）；exotic 方法读 IC 命中（S16.1）后直达 selection，消 `DirectSelection::select` 现场重分类。
2. `set_map_record` 收敛（与 17.3 协同，此处只接线）。

### 测试与验收

- 语义红线测试：getter/陷阱内 `new Error().stack` 帧序、陷阱内抛错的 PC/栈、`Reflect` invariant 违例仍抛 TypeError、getter 内再入 backtrace、native 内异常/中断的帧可见性、depth-mixed 的 `valueOf` 重入顺序。
- 计数验收（probe_diagnostics）：depth-getter-0 每迭代槽认证 12 → ≤4、PC 发布 4 → ≤1；depth-proxy-0 槽认证 12 → ≤6、PC 发布 5 → ≤2、每读 Box 4 → 0（稳态）、`run_exit.Complete` 走快速回执；depth-mixed-0 槽认证 26 → ≤10；math_min `vm_phases.native.prepare` 179ms → <40ms、`runtime_pc_publication` 3.3M → 启动量级；weak_map_set `slot_moves` 9.5/op → ≤4/op。
- 耗时验收：探针 12 项相对 S0 转负或 ≤+2%；map 族与 math_min 相对 S0 转负。
- 提交：`perf(vm): extend lazy frames to native, getter and proxy trap calls`

## S19 — regexp 外围

前置：无新增设计章节；S17.4 pin atom 就绪（`lastIndex/index/input/groups/exec`）。

### 工序

1. **输入缓冲跨 exec 复用**（`builtins/regexp/exec.rs:65`）：flat Latin1 提供零拷贝借用 + 批量加宽（`extend_from_slice` 整段，替代 `Utf16Units` 逐单元）；rope 输入先 linearize 一次后借用。不再每次 `collect::<Vec<u16>>()`。
2. **`validate_program` 一次性**（`regexp/executor.rs:186,211-365`）：`ValidatedProgram` newtype，唯一构造点在编译出口（`compiler.rs:50`），executor 只接受已验证类型；每 exec 的两遍全指令扫描归零。
3. **结果构造预制**（`regexp/result.rs:206-222`）：结果数组/groups 的恒定 shape realm 级预建（消整层 `replace_object_layout`）；属性名用 pin atom。
4. **替换缓冲**（`value/primitive.rs:434-444`、`builtins/replacement.rs:78-172`）：`try_reserve_exact` → `try_reserve`；`Utf16Units` 补 `nth/advance_by`；`append_range` 对 flat 源 `extend_from_slice`；`$&`/`$'` 前后缀不再逐单元重扫。
5. `AttemptState` 缓冲跨候选起点复用（`executor.rs:453-475`）。

### 测试与验收

- 语义红线：`lastIndex` 读写序、sticky/global 组合、命名组、`Symbol.replace` 自定义、超长串与 OOM 路径的错误等价。
- 计数验收：v8-regexp `Utf16Units::next` self 28.07% → <5%、稳态 `validate_program` 归零；regexp_ascii/utf16 memcpy self（9.2%/13.0%）减半以上。
- 提交：`perf(regexp): reuse input buffers and precompute result layout`

## S20 — heap/GC 结构

前置：无。RSS 不在本单元（R9 → S13）。

### 工序

1. **`NodeData::Context` 改 `Box<ContextData>`**（`heap/mod.rs:171-177`）：ArenaSlot 904B → ~100B；`grep -n 'NodeData::Context' src/` 逐点补一层解引用；Context 分配 O(realm) 次，Box 间接开销可忽略。
2. `drain_zero_queue` 四次 `mem::replace` 合并单次 take（`heap/gc.rs:1119-1152`）。
3. `object_edges/property_slot_edges` 改迭代器/SmallVec（`gc.rs:1318,1744-1764`；`gc.rs:1967` `property_slot_atoms` 已是迭代器先例）。
4. `retain_raw_value_atoms` 消无条件双 Vec（`heap/runtime/mod.rs:329-348`）；`release_or_defer` 立即分支去重复 drain 检查（`heap/ownership.rs:42-58`）。
5. 附带项（R10）：`property_ic.rs:177-193` 的 `indices` 恒定 4B/指令改稀疏表示、`sites` 预留容量。

### 测试与验收

- 语义红线：GC oracle 压力、循环回收、弱引用/终结时序既有测试全过；`size_of` 断言更新（ArenaSlot 新尺寸钉死为断言）。
- 计数验收：splay/raytrace/earley-boyer 的 memcpy(`0x18f413`)/malloc self 收敛；`release_raw_no_drain` self（2-2.7%）减半。
- 提交：`perf(heap): box context payloads and reduce per-value bookkeeping`

## 收尾

1. **S14 验收轮附带**：编译 67 项锁频三轮复测（R10 关闭判定：v8-deltablue 若仍 >S0 +5% 回开专项）。
2. **全阶段完成后**：全量 403 项终测 + 与修复计划 §3 矩阵逐项对账（fixed 21、探针 12、Score 3 全部相对 S0 转负或 ≤+2%），更新 README/plan/commit-plan 状态行。
3. **S13（本序列之后）**：退役旧 VM 路径，验收含 RSS 2 项相对 S0 ±5%（R9）。
4. 落地时同步重写 `docs/architecture/owned-fusion.md` 驻留边界章节（S15）、IC 失效语义（S16）、转移表边界（S17）、惰性观察边界（S18）。
