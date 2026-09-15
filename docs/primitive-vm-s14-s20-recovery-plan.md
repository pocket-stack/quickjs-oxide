# S14–S20 全量回退修复与进一步优化计划

**本次执行指令（覆盖旧验收流程）：**分阶段提交全部实现，代码全部完成后额外逐项覆盖 review，遗漏补齐后仅对最终新核心执行一轮 benchmark/Profile；S0 与 QuickJS 复用本地已有数据。实现期间不运行 benchmark、cost profile 或 CPU Profile，不按阶段重复测量。测量结果仅保留在本地 `target/performance-retained/`，不提交 Git。阶段提交已获授权，无需再次等待提交指示。

2026-09-15 立项，同日按全量回退清单扩容。目标：消除 S10–S12 验收后相对 S0 超过 5% 的全部正差值——fixed 21 项、调用探针 12 项、原始 Score 3 项、峰值 RSS 2 项，并对编译 5 项完成判定与处置（§1 R10：现有证据判定为测量伪影）。本计划**先设计后实现**；每阶段独立验收，沿用三轮公平复测与单变量归因纪律。S13（退役旧路径）的既有定位不变，且**新增承接 RSS 验收**（§1 R9）。

证据来源：`reports/primitive-vm-s10-s12-final.json` 的 cost/cpu Profile、probe_diagnostics 事件计数、raw_samples 环境记录，与两批共七线源码审计。所有 file:line 已逐条核对当前工作树。逐工序执行手册（文件/函数级步骤、测试与验证命令）见[执行计划](primitive-vm-s14-s20-execution-plan.md)。无符号热点 `0x18f413` 已确认为 glibc `__memcpy_avx512_unaligned_erms`（大块 memcpy），`0xa73e8` 为 malloc 内部例程。

## 1. 根因总表（十类，均有代码级或测量级确证）

### R1 — run 驻留谱系缺口：大量纯操作逐次退出解释循环

每次 RunExit 的固定税：`driver/ready.rs:36-38` 对**除 Call/Complete 外的所有退出**无条件 `frames.materialize`，其中已物化帧仍逐次 `publish_materialized_pc`（`vm/frame.rs:164-183`，`RefCell::borrow_mut` + 注册表写）；重入 run 时重建 transaction（每次 `slot_authentication`）。计数证据：bigint64_arith 每操作 4 次 `runtime_pc_publication`、4 次槽认证、15 次槽搬运。

无条件/结构性退出清单：

| 指令 | 位置 | 行为 | 受害用例（退出计数） |
|---|---|---|---|
| `Add`（String/BigInt 形态） | `vm/run.rs:1493-1499` | 提前 `return ConvertAdd`，**绕过** `run.rs:1683` 已存在的驻留数值分支；而 `Sub/Mul/BitAnd…` 的 BigInt 形态**已经驻留**（`run/numeric.rs`）。`run/numeric.rs` 的 `supported()` 并不排除 String/BigInt，`primitive_output` 本身支持 `NumericKind::Add`（`vm/numeric/operation.rs:123-125`）——Add 只是没接上 | string_build1/3/large1/large2（各 1.6M）、int_to_string（1.5M）、bigint64_arith（1.6M）、splay（517k）、v8-regexp（33k） |
| `StrictEq/StrictNeq`（非数字对） | `vm/run.rs:1557-1566` | 仅 `a.float()==b.float()` 驻留；字符串相等、对象/Symbol 引用相等、null/undefined/bool 全部退出 | earley-boyer（1,249,271） |
| `Not` | `vm/run.rs:1569` | **无条件**退出，连布尔取反都退出 | earley-boyer（469k）、splay（275k）、raytrace（103k） |
| `DefineField`（构造器 `this.x=v`） | `vm/run.rs:831-842` | 无条件退出 | splay（1,018,080）、earley-boyer（经 Construct） |
| `GetArrayEl2/3` | `vm/run.rs:441-446` | 无条件退出 | crypto/raytrace 元素读的保留接收者形态 |
| `delete` | `vm/run.rs:480` | 无条件退出为 Predicate | prop_delete（275,500） |
| Environment 族部分指令 | `vm/run.rs:549-709` | 多个变体无条件退出 | splay（767,626）、local_destruct（160,099） |

### R2 — 字符串/转换内核的冗余拷贝与分配

- **数字→字符串双转码**：`vm/numeric.rs:119-129` → `Value::to_js_string`（`value/mod.rs:134-166`）产生 `String → Vec<u16> → Vec<u8> → Rc` 共 3 遍拷贝/转码、4 次分配；反方向（string→number）已由 dce0b6ea 优化为 ASCII 借用，本方向对称的 `from_owned_latin1`（`value/primitive.rs:956-959`）存在但未被使用。
- **`try_concat` 缺左空串短路**（`primitive.rs:1680-1690`）：`"" + x` 仍走 `flat_concat` 全量拷贝——int_to_string 的 `n + ""` 形态每次命中。
- **无 in-place 追加**：`StringRepr::Latin1(Box<[u8]>)` 无容量字段，`flat_concat`（`primitive.rs:1217-1246`）必然重分配 + 全量拷贝；`r += "x"` 在 8192 字节内是 O(n²)。QuickJS 在 refcount==1 时原地 realloc 追加。
- **`complete_primitives` 冗余**（`vm/conversion_driver.rs:125-299`）：同两个槽被 peek 两遍（144-148 与 184-188）、同一局部被 `local_current` 3 次；`complete_local_add`（`conversion_driver/local_add.rs`）合计 7 次 `local_current`、3 次 `local_add_values`。
- **`MathStep::start` 无条件 `Box::new`**（`builtins/math/operation.rs:89`）：全原语 `Math.min` 也付一次分配——为满足 `MathResume ≤ 8B` 的尺寸断言把分配推进了热路径。

### R3 — 属性 IC 的三重覆盖缺口

- **接收者白名单**（`object/property_ic.rs:118-125`）：只认 `Ordinary` 与 `Array`。Map/WeakMap/Set/Date/RegExp/TypedArray 等 exotic 接收者上的**方法取用**（方法本身在 Ordinary 的 prototype 上，完全可缓存）在第一次 locate 就失败 → 2 次 miss 永久 megamorphic。这是 weak_map_set 中 `read_progress_selected + ordinary_read_probe_atom` 合计 14% 的直接根因：每次循环全量重查 `m.set`（每操作 2 次 storage probe、1 次 `property_read_root_materialized`）。
- **两次 miss 永久 Megamorphic、无多态、无复活**（`property_ic.rs:103-108`）：splay 808,252 次 GetField 退出仅记 115 次 miss——站点第二次 miss 后永久放弃。
- **GC 门控饿死 IC**（`object/ordinary_storage/ic.rs:27-35`）：`deferred_references.has_pending()` 或 `has_pending_zero_cleanup()` 任一为真即整体旁路（不计 miss）。分配密集负载（splay/raytrace）常态处于该状态。
- **写侧无 IC**：`PutField` 快路径（`object/ordinary_storage.rs:992-1047`）要求**新值与旧值都是标量**，且每次仍 `locate`（SipHash）；写引用值（splay `node.left=child`）必退出走 `object/ordinary/set.rs` 全状态机。crypto 计数：`run_exit.SetProperty` 868,703、`property_storage_set_probe` 1,737,599。`PutArrayEl` 快路径只覆盖 typed array（`run.rs:356-364`），普通 dense 数组元素写必退出。
- **`GetArrayEl` 快路径覆盖不足**：crypto `run_exit.GetElement` 1,501,564。
- **全局读无缓存**（`heap/roots.rs:181-241`）：`Math` 这类未解析全局每次调用完整走 belongs_to/borrow/zero-queue 检查 + **全局大 shape 的 SipHash find** + retain/release 对；`GetVar` 不在 IC 站点表内。math_min 每调用 1 次（1.1M）。

### R4 — shape/atom/哈希的结构性 O(n²)

- **shape 缓存为内容寻址整表**：`ShapeFingerprint = {prototype, Box<[ShapeEntry]>}`（`object/operations.rs:16-20`），`get_or_create_shape`（`heap/runtime/mod.rs:248-285`）每次 layout 变更哈希**全部属性**（每 entry ≈7-8 次 `Hasher::write`）。没有 per-shape 转移表 `(parent, atom, flags) → ShapeId`。加第 k 个属性哈希 k 项 ⇒ 建 n 属性对象 O(n²) 哈希 + O(n²) 拷贝。
- **小对象强制全量路径**：`MIN_UNIQUE_SHAPE_APPEND_ENTRIES = 8`（`object/properties.rs:39`；`object/storage.rs:451-455`）——属性数 <8 的对象每次 define/delete 都是 `entries().to_vec() + slots.clone() + replace_layout`。prop_delete 分支 B（`properties.rs:1634-1680`）：删一个属性 = 2 次全表 SipHash + 2 次全量 Vec clone + 2 次 `Vec::remove` memmove。
- **全仓默认 SipHash**：无 fxhash/ahash 依赖。`Shape.lookup: HashMap<Atom,u32>`（`object/shape.rs:123`），`Atom` 16 字节、derive Hash 写 3 次（`atom/mod.rs:52-57`），无预计算哈希。prop_clone 的 `Hasher::write` 21.2% self、prop_delete 14.6% 即此。
- **Map/Set 双重哈希**（`heap/collection_index.rs:99-121`）：先 SipHash 键得 u64，再把该 u64 作为 `HashMap<u64,_>` 的键**又 SipHash 一次**；`set_map_record` 付 2 次查找哈希 + 3 次 `raw_property_value`（`builtins/map.rs:406-441`）。键哈希缓存仅覆盖 ≥256 单元长字符串。WeakMap 键（ObjectId）同为 SipHash 无缓存。
- **热路径字面量 intern**：全仓 80+ 处 `intern_property_key("字面量")`，每次 = 新建 JsString（2-3 次分配）+ 全串扫描 + SipHash 查表 + atom retain/release。重灾区：`object/properties.rs:879,1019,1108` 的 `"length"`（array_slice 每元素 ×2 ⇒ 每次 slice 2000 次 intern，8.5% self）；`vm/iterator_driver.rs:729-733,750-755` 的 `"next"/"return"`（local_destruct 每解构 ×2）；`iterator_driver.rs:891-894` 的 `intern_property_key(&position.to_string())` 整数→十进制→parse 往返。已有正确先例：`builtins/iterator/array/local.rs:47-67` 借用 shape 首 entry atom 校验拼写、`atom/runtime.rs:29-38` `property_key_for_index` 免 intern。

### R5 — native 调用外围：S10 惰性化未覆盖 native 帧

- **native 帧每次调用急切发布**：`vm/call/native.rs:316` 无条件 `publish` → `publish_validated_active_frame`（`vm/frames.rs:406-441`：borrow_mut + token++ + Vec push + weight charge + Runtime Rc clone + function root retain）；且 `driver/ordinary.rs:107` 在 native 分支**强制物化全部惰性 JS 帧**。math_min 计数：`native.prepare` 阶段 179ms（占总耗时 21.9%）、每调用 3 次 `runtime_pc_publication`、17 次槽搬运。
- **纯验证 retain 对**：`promote_selected/promote_linked` 各 2 次（`frames.rs:34-35,56-57`）、`publish` 第 3 次（`frames.rs:259`）、Pure 族 `invocation.clone()` 的 receiver retain（`builtins/continuation.rs:39`）——被调函数与 receiver 在调用期间已被 slot/argv owner 钉住，这些全部可省。
- **argv 双缓冲**：`call.native_argv` → `native.readable` 两次 values_moved（weak_map_set 各 641k）；`reserve_native_argument_depth` 每调用 2 次；take 时逐槽 peek 重认证（`vm/stack.rs:204-232`）。
- **通用读路径不产出 `LinkedNativeSelection`**（`object/access.rs:122-127`）：IC miss 的方法调用每次 `DirectSelection::select` 现场重分类。
- **witness 每调用重验**：`NativePublicationWitness::from_classification/validate`（`vm/frames.rs:147-250`）每次 native 调用重新 borrow state、重取 heap 对象、重查 `data.target/min_readable_args/realm`；argv 侧 `try_reserve_exact` + `resize(available, Undefined)` padding（`call/native.rs:256-286`）每调用重写。探针证据：depth-native-0 相对 S0 +21.1%（dce0b6ea 时已 +18.6%，非 S10–S12 引入），每次调用固定付 3 组 retain/release 对 + 双缓冲搬运 + active frame push/pop。

### R6 — heap/GC 结构：904 字节 ArenaSlot 与逐值 Vec

- **`ArenaSlot` = 904 B**（`heap/mod.rs:171-240`；`NodeData::Context(ContextData)` inline，`ContextData` 43+ 字段；profiler-baseline.json 实测 1171584/1296=904）。每次 `release_raw_no_drain`/`drain_zero_queue` 的 `mem::replace` 都搬 904B，终结一个节点 ≈6 次；`Vec<ArenaSlot>` 增长按 904B 步长 memmove。这是 splay/raytrace/earley-boyer 的 memcpy（0x18f413，4-6.5% self）与 RSS 差值的主嫌疑。
- **逐值 Vec 分配**：`object_edges`/`property_slot_edges` 每次返回新 `Vec<RawId>`（`heap/gc.rs:1318,1744-1764`）；`retain_raw_value_atoms` 无条件 2 个 Vec（`heap/runtime/mod.rs:329-348`）。
- **每 root drop 全套通道**：`release_or_defer`（`heap/ownership.rs:42-58`）每次 try_borrow_mut + match + drain 检查——即使立即分支成功。

### R7 — regexp 外围：per-exec 重复工作

- **每次 exec 重新物化整条输入**：`builtins/regexp/exec.rs:65` `input.utf16_units().collect::<Vec<u16>>()` 是函数局部变量，无跨调用缓存；`Utf16Units::next`（`primitive.rs:683-724`）逐单元三分支 match + 边界检查 + Latin1 加宽。v8-regexp 843,950 次 exec × O(串长) ⇒ 28.07% self。`Utf16Units` 构造本身零填充 488B。
- **每次 exec 重验程序**：`regexp/executor.rs:186` 无条件 `validate_program`（两遍全指令扫描 + 分配，`executor.rs:211-365`）；`CompiledRegExp` 无 validated 标记。
- **每次 exec ≥6-7 次字面量 intern**（`"lastIndex"×2、"length"、"index"/"input"/"groups"、"exec"`）+ 结果对象**整层布局替换**（`regexp/result.rs:206-222`：entries to_vec + slots clone + fingerprint 全表哈希 + 2 次 edges Vec + HeapCleanup）。结果 shape 恒定，可 realm 级预建。
- **替换缓冲二次方**：`ReplacementStringBuffer` 的 `reserve_additional` 用 `try_reserve_exact`（`primitive.rs:434-444`）取消摊还增长 ⇒ k 次追加最坏 k 次全量 memcpy；`append_range` 的 `skip(start)` 因 `Utf16Units` 无 `nth/advance_by` 退化为 O(start) 逐单元丢弃，`$&``$'` 每匹配重扫全串前后缀（`builtins/replacement.rs:78-172`）。
- **每候选起点 2 次分配**：`AttemptState::new`（`executor.rs:453-475`）在 `run_attempt` 内。

### R8 — 属性驱动发起的被调帧"出生即物化"与 proxy get 陷阱外围（探针族 getter/proxy/mixed 主根因）

探针语义（生成产物 `target/primitive-vm-s09-implementation/measurements/depth-*.js`）：depth-getter 每迭代读一次 accessor 属性（getter 内调 `Math.abs`）；depth-proxy 是 **Proxy `get` 陷阱**（非 apply）；depth-mixed 是普通调用 + `Math.min` + `valueOf` 重入。回退在 dce0b6ea 时已存在（proxy +67% → 当前 +55%），是新 VM 的每调用固定开销。

- **S10 惰性 token 只在 `OrdinaryCall::install` 设置**（`call/ordinary.rs:250` `ActiveFrameToken::unmaterialized()`）。经属性驱动发起的 getter/proxy 陷阱回调帧走通用 `BytecodeCallRequest::prepare` → `call/prepare.rs:123` **急切 `push_bytecode_active_frame`**，且 `initialize_bindings: true` + `original_arguments: Vec`（`call/request.rs:74-82`）→ `push_initialized_frame`，完全绕过惰性协议与 `OrdinaryCall::authenticate` 的认证缓存。
- **GetField 非豁免退出**（R1 的 `ready.rs:36-38`）叠加：accessor/proxy 读每次先付整栈 materialize，再付被调帧急切物化。计数证据（每迭代）：depth-getter-0 槽认证 12 次、PC 发布 4 次；depth-proxy-0 槽认证 12 次、PC 发布 5 次、完整读分派 5 次。
- **getter 专属重复**（`vm/property_driver.rs:602-659`）：每次调用 `normalize_callback` 全量重分类 + 2 次额外 heap borrow（function_kind 检查、native_operation 探测）；PropertyKey 每次读从 atom 重建（`property_driver.rs:224-241`，Rc clone + atom retain）。
- **proxy get 陷阱专属**：每次读 4 个 `Box` 分配（`MethodResumeState`、`ProxyGetResumeState`×2 个阶段、`PendingProxyGet`，`object/internal_methods/method.rs:101-106`、`get.rs:112-130,157-168`、`proxy_get_driver.rs:1550-1576`）+ 每次 `intern_property_key("get")`（`method.rs:59`）+ 3 元素实参 Vec + key→JsString 物化（`get.rs:97-109`）；陷阱帧带 `operation: Some(PropertyGet)` 与 `property_wait`，**返回被 `frame.rs:765-784` 的 `ordinary_return` 拒绝**，走完整冷退出（`frame_exit.rs:71-161`）+ 二次 materialize（`driver.rs:802`）。每迭代合计 3 次 run 退出、4 次重入。
- **waitable native**（depth-mixed 的 `valueOf` 重入路径）：每次 `Math.min` 走 `start_waitable_native_call`（`proxy_get_driver.rs:669-800`）——operation identity 递增、`query_storage.acquire`、`query.natives/spare_parents` 双 reserve、`install_waiting`。计数：depth-mixed-0 每迭代槽认证 26 次、PC 发布 12 次。

### R9 — RSS 常数增量：双二进制映像差，非堆分配（func_call +15.7%、func_closure_call +18.6%）

三个 RSS 用例的绝对增量近似常数（1584–1988 KiB），与工作负载规模无关。逐项排除与坐实：

- **启动堆分配 S0 与当前逐字节相同**（两个二进制 `qjs -q -d` 实测：objects 164 / shapes 61 / arena_slots 278、inline capacity 462,848 B 完全一致）；arena 无初始 chunk（`heap/arena.rs:10` `Vec::new()`），全仓无 MB 级 `with_capacity`。`ArenaSlot` 904B 自 S0 起未变，其 452 KiB 容量两版相同——**R6 的 904B 是 memcpy/吞吐问题，不是本项 RSS 来源**。
- **映像差坐实**：S0（`features=[]` 旧 VM）`.text` 5,138,269 B → 当前（`stack-vm`）8,084,685 B（**+2.95 MB**），可加载节合计 +3.79 MB；空脚本（`qjs -e '1'`）RSS 已差 ~1150 KiB，负载期再多触碰 ~550 KiB 代码页。每函数新表（FusionPlan 1B/PC、IC indices 4B/PC）在微基准合计 <1 KiB，且增量的规模无关性本身排除堆侧。
- **驱动量是"被触碰代码页数"而非字节总数**：S09→当前 `.text` 只增 192 KB，空脚本 RSS 反降 ~1.9 MB（S11 `#[cold]` 外提把冷码挤出热页）。
- **归属修正**：RSS 验收从 S20 移交 **S13（退役旧路径）**——删除旧 VmHost/桥后 `.text` 收缩是唯一能收回映像差的杠杆；S20 仅保留 memcpy/吞吐目标。S13 前 RSS 无法完全恢复，属两套路径并存的结构性成本。

### R10 — 测量伪影：编译 5 项与个别 fixed 单轮噪声（判定为非代码回退，处置为复测）

- **v8-deltablue 编译 +95.89% 为单样本离群**：r1 样本环境 `loadavg 4.08`、`governor powersave`、频率在 1.91–4.79 GHz 间跳动（raw_samples 记录）；同源用例 `deltablue` 仅 +1.49%；dce0b6ea 三轮 3.79–3.84 ms 与 S0 中位 3.89 ms 持平；当前工作树 profiling 相位归因合计 3.87 ms（parse 2.16 / publish 0.59 / lowering 0.45 / verify 0.33 / resolution 0.30 / fusion 0.025）与 7.63 ms 矛盾。
- **其余 4 项（v8-earley-boyer +12.0%、earley-boyer +7.1%、v8-crypto +5.5%、all +5.2%）为本轮 +2~5% 整体水平漂移**：单轮值系统性高于 dce0b6ea 三轮全部样本，而相位和与旧值一致（v8-earley-boyer 相位和 38.2 ms vs 单轮 43.1 ms）。
- **S0 之后编译期新增工作已排除**：唯一新增 `FusionPlan::build`（`heap/allocation.rs:1307` → `code/fusion.rs:23-214`）实测 24.6 µs = deltablue 编译的 0.64%，O(n) 无回溯，ns/指令在方法调用最密的 deltablue 与 richards 相同；S12 IC 表在首次调用快照期构建（`code/executable.rs:267`），**不在编译探针计时内**（`compile_probe.rs:18-22` 只含 `compile_with_filename`）；verify/S11 无每函数编译期新表；编译探针二进制不含 profiling feature，计数开销不进正式计时。
- **fixed 侧同类噪声成分**：string_build1（单轮 +14.8% vs dce0b6ea 三轮中位 +3.9%）、string_build3（+11.5% vs +3.8%）、array_slice（+11.8% vs +0.6%）——真实回退存在但幅度以三轮中位为准，S14/S17 验收按三轮判定。
- **处置**：不立编译工程项。S14 验收轮起，编译 67 项与全部对比一律三轮中位对三轮中位，并记录/约束环境（见 §4）；若锁频三轮后 v8-deltablue 编译仍 >S0 +5% 再回开专项。低优先级附带项（真实但微小，挂 S17/S20 顺带）：`fusion.rs:29/90` 预筛与主循环合并（每 PC 少分类一遍）、`property_ic.rs:177-193` 的 `indices` 恒定 4B/指令改稀疏表示 + `sites` 预留容量。

## 2. 阶段计划

依赖与归因顺序：S14/S15 互不依赖但都动 run/转换（先 S14 后 S15，避免双变量）；S16 依赖 S15 的退出税豁免语义稳定；S17、S18、S19、S20 相互独立，可与前序并行开发但**按阶段顺序单独测量**。每阶段完成后：全量工作区测试 + Test262 结果向量 + 边界矩阵 + 三轮公平复测（S0 复用旧三轮），机械计数逐项核对。

### S14 — 字符串/转换内核（纯 value 层，无 VM 协议改动，最低风险先行）

改动：
1. `Value::to_js_string` 数字分支改走 `from_owned_latin1`（ASCII 直通，消双转码），对称复刻 dce0b6ea 的 string→number 借用路径。
2. `try_concat` 补 `self.is_empty()` 短路（`primitive.rs:1680`）。
3. flat 串 in-place 追加：`Rc` 强计数==1 且 Latin1+Latin1 时复用容量（需把 `Box<[u8]>` 改为带容量的表示或引入 builder 态），消 string_build 的 O(n²)。
4. `complete_primitives`/`complete_local_add` 冗余折叠（peek/local 各收敛到一遍）。
5. `MathStep::start` 的 Box 延迟到确需挂起时分配。

验收：int_to_string、string_build×4 的 `try_from_utf8/from_validated_utf16` self 归零；string_build 系列耗时相对 S0 转负；math_min 分配计数每调用 -1。

### S15 — run 驻留谱系补全与退出税削减（病 1 终结）

改动：
1. **Add 接入既有驻留分支**：`run.rs:1493-1499` 不再提前 return，落入 `run.rs:1683` 的 `!handled` 通用分支；`numeric::supported` 接住 String/BigInt（Object 形态照旧退 `RunExit::Numeric(Add)` 走既有 driver）。BigInt/Symbol 沿用 `run.rs:1689-1695` 的先物化判定。`add_store` 融合与 `observable_release` 语义随迁（`release_outside_slots!` 先例，`run.rs:263-279`）。前置：修订 `docs/architecture/owned-fusion.md:50-52,99-114` 的"ConvertAdd 不建立 run 驻留"条款——dce0b6ea 先例已证明"分配不进 RunSlots 借用"≠"不在 run 函数内"（`drop(slots)` 后在 FrameTransaction 存活窗口内分配）。
2. **StrictEq 驻留**：对象/Symbol 引用相等、bool/null/undefined 混合形态、flat×flat 字符串相等直接在 run 内完成；rope 需展平时仍退出。
3. **Not 驻留**：ToBoolean 对全部值形态是纯函数；操作数释放沿用 `release_outside_slots!`。
4. **DefineField 快路径**：fresh Ordinary 接收者 + 标量值时在 run 内直通 define（配合 S17 转移表后扩展到引用值）。
5. **delete 快路径**：own data 可配置属性的 delete 在 run 内完成（复用 S17 后的存储改动）。
6. **退出税豁免**：`ready.rs:36-38` 对"驻留完成型"退出（AddLocal、Numeric、conversion Completed 等不可能观察注册表的路径）跳过 `materialize`/`publish_materialized_pc`；已物化且 PC 未变的帧跳过发布。远期方向：观察协议改"拉"式（注册表按需读运行态 PC），供 S16+ 评估。

验收：string_build/int_to_string/bigint64 的 `run_exit.ConvertAdd` → 启动量级；earley-boyer `run_exit.StrictEquality` 1.25M → 启动量级、`run_exit.LogicalNot` → 0；splay `run_exit.DefineProperty` 1.02M 大幅收敛；上述用例 `runtime_pc_publication` 相应消失。锚定 PC/栈的既有测试（`local_add.rs:129-242`、`run/numeric.rs:87-136`）全部保持；驻留验收断言照抄 `numeric_completed_in_run` 形式。

### S16 — 属性协议第三期（IC 覆盖面与写路径）

改动：
1. **放宽接收者白名单**（`property_ic.rs:118-125`）：receiver 只需"参与 shape 查找的 kind"即可作为缓存键起点；exotic 接收者上的原型方法读全面可缓存。
2. **门控解耦**（`ic.rs:27-35`）：命中路径只读缓存槽 + retain，证明与 deferred/zero-queue 互不干扰后收窄 guard 粒度；或在进入 run 前主动 drain 小队列。
3. **2-miss 永久 mega → 4 路多态 + 计数复活**（`property_ic.rs:103-108`）。
4. **写 IC**：站点表纳入 `PutField`，`Location{shape,revision,depth:0,slot}` + writable 位，命中直通 `replace_data`（`ordinary_storage.rs:1046` 已有）；引用值写在命中路径内做 retain/release（旧值释放走 zero-queue 既有协议）。
5. **元素读写**：`GetArrayEl2/3` 补驻留；`PutArrayEl` 增加普通 dense 数组标量/引用写快路径。
6. **全局读缓存**：`GetVar` 站点缓存 `(global shape revision → slot)`，或未解析全局 cell 在首读后回填；消 math_min 的 `try_read_unresolved_global` 每调用全查。

验收：splay `run_exit.GetField` 808k → <5 万且 `property_ic.hit` 相应上升；crypto `run_exit.GetElement` 1.5M、`run_exit.SetProperty` 869k → 启动量级；weak_map_set `property_storage_read_probe` 1.28M → 启动量级；math_min `property_storage_read_probe` 1.1M → ~0。失效语义反例（delete/defineProperty/setPrototypeOf/字典化/跨 realm/Proxy）在写侧与多态侧补齐等价矩阵。

### S17 — shape/atom/哈希结构（结构性 O(n²) 清除）

改动：
1. **per-shape 转移表**：`(parent ShapeId, Atom, PropertyFlags) → ShapeId` 单步查找替代 `ShapeFingerprint` 全表哈希（保留 fingerprint 仅作字典化/去重兜底）；`unlink_finalized_shapes` 消第二次全表哈希。
2. **小对象路径**：`MIN_UNIQUE_SHAPE_APPEND_ENTRIES` 降为 1-2 或独占 shape 一律走 `append_selected_unique_layout`；delete 分支 B 的双全量 clone 消除。
3. **hasher 与键**：`Shape.lookup` ≤8 项走 entries 线性扫描（不建 HashMap）；大 shape 换 FxHash 或以 `atom.raw()` 为 nohash 键（同 runtime 内 generation/table_id 恒定）；`Atom` 的 Hash 收敛为单次 write；`CollectionIndex` 消第二重哈希（u64 键 nohash）；短字符串/Int/Object 键哈希缓存进 record。
4. **常量 atom 预驻留**：`length/next/return/done/value/index/input/groups/lastIndex/exec` 等 pin 进 RuntimeState（`intern_static` 已支持）；`properties.rs:879,1019,1108`、`iterator_driver.rs:729-733` 等 80+ 调用点改引用；`iterator_driver.rs:893` 改 `property_key_for_index`。
5. **array_slice 批量旁路**：自有 dense source + fresh fast array + 无 species/hole/索引原型时整段拷贝（照抄 `SliceKind::ToSpliced` 的 `values[] + new_array_from_values`）；`define_array_index` 的 descriptor 三次结构转换在 dense 快路绕过。
6. **prop_clone 协议瘦身**：`own_property_keys` 预留容量、`clone_copy_key`/`clone_copy_object` 的每属性 retain 对改借用式游标。

验收：prop_clone `copy_owner_clone.PropertyKey` 320k → 0、SipHash self 21.2% → <3%；prop_delete SipHash 14.6% → <3%；array_slice `property_storage_read_probe` 880k → 启动量级、`intern_property_key_js_string` self 8.5% → ~0；local_destruct intern self 3.1% → ~0；splay/raytrace/earley-boyer 的 `Hasher::write` self 显著收敛。语义红线：属性序、字典化阈值行为、Test262 向量不变。

### S18 — 调用外围：native 帧与属性驱动被调帧惰性化（承接旧簇 B 与 R8）

改动：
1. **native 帧纳入惰性观察协议**：`call/native.rs:316` 的 publish 延迟到确有观察者（backtrace/异常/宿主回调）；`driver/ordinary.rs:107/113` 的强制物化随 S15 豁免收窄；`NativePublicationWitness` 重验与 argv `resize` padding 收敛为一次性/按需。
2. **retain 削减**：`promote_selected/linked` 双 retain、publish 的 function root、Pure 族 `invocation.clone()` 改借用证明（调用期间 slot/argv owner 已钉住）。
3. **argv 单次搬运**：合并 `call.native_argv → native.readable` 双缓冲；`reserve_native_argument_depth` 收敛为一次；take 的逐槽重认证消除。
4. **getter/proxy 陷阱回调帧走惰性 install**（R8）：属性驱动发起的字节码调用改用 `OrdinaryCall::authenticate/install` 等价路径——`ActiveFrameToken::unmaterialized()`、复用认证缓存，消 `call/prepare.rs:123` 的急切 `push_bytecode_active_frame` 与 `original_arguments` Vec；getter 侧 `normalize_callback` 重分类 + 2 次 heap borrow 折叠进一次分类结果，PropertyKey 改借用（消 `property_driver.rs:224-241` 每读重建）。
5. **proxy get 陷阱外围瘦身**（R8）：`"get"` 等陷阱名 pin atom（并入 S17.4 清单：`get/set/has/apply/deleteProperty` 等）；`MethodResumeState/ProxyGetResumeState/PendingProxyGet` 的 4 Box 收敛为 query 存储内联/池化（`query_storage` 已有池化先例）；陷阱返回补 operation 感知的快速回执，避免 `ordinary_return` 拒绝后的完整冷退出 + 二次 materialize。
6. **waitable native 收敛**（depth-mixed）：`start_waitable_native_call` 的双 reserve 与 operation identity 簿记按稳态复用。
7. **IC 联动**（依赖 S16.1）：exotic 接收者方法读命中后 `LinkedNativeSelection` 直达，消 `DirectSelection::select` 现场重分类；通用读路径补 native selection 产出（`object/access.rs:122-127`）。
8. **Map/WeakMap 内部表**（与 S17.3 协同）：消双哈希、`set_map_record` 收敛为 1 次查找哈希 + 1 次 `raw_property_value`。

验收：math_min `vm_phases.native.prepare` 179ms → <40ms、`runtime_pc_publication` 3.3M → 启动量级；weak_map_set `slot_moves` 9.5/op → ≤4/op、heap retain/release 对 ≥4/call → ≤1/call；探针机械计数（probe_diagnostics）——depth-getter-0 每迭代槽认证 12 → ≤4、PC 发布 4 → ≤1，depth-proxy-0 槽认证 12 → ≤6、PC 发布 5 → ≤2、Box 分配 4/读 → 0（稳态）、`run_exit.Complete` 冷退出 → 快速回执，depth-mixed-0 槽认证 26 → ≤10；depth-native/getter/proxy/mixed 全族 12 项相对 S0 转负或 ≤+2%；map_set_int/map_delete/weak_map_set 耗时相对 S0 转负。语义红线：陷阱/getter 的可观察调用序、backtrace 帧序、`Reflect` 不变量检查（proxy invariant 阶段）逐项保持。

### S19 — regexp 外围（承接旧簇 E）

改动：
1. **输入缓冲跨 exec 复用**：JsString 上缓存扁平 UTF-16 视图，或 flat Latin1/Utf16 提供零拷贝借用 + 批量加宽（`extend_from_slice` 替代逐 unit 迭代），`exec.rs:65` 不再每次 collect。
2. **`validate_program` 一次性**：编译出口验证 + `ValidatedProgram` 新类型（`compiler.rs:50` 唯一构造点）。
3. **结果构造预制**：结果数组/groups 的恒定 shape realm 级预建（消 `replace_object_layout` 整层替换）；`lastIndex/index/input/groups` 用 S17.4 的 pin atom。
4. **替换缓冲**：`try_reserve_exact` → `try_reserve`（恢复摊还增长）；`Utf16Units` 实现 `nth/advance_by`；`append_range` 对 flat 源直接 `extend_from_slice`。
5. `AttemptState` 缓冲跨候选起点复用。

验收：v8-regexp `Utf16Units::next` self 28.07% → <5%、`validate_program` → 0 稳态样本；regexp_ascii/utf16 memcpy self（9.2%/13.0%）减半以上；regexp 族 fixed 相对 S0 全部转负。

### S20 — heap/GC 结构（memcpy 尾部；RSS 归属已修正为 S13，见 R9）

改动：
1. **`NodeData::Context` 改 `Box<ContextData>`**（`heap/mod.rs:171-177`）：ArenaSlot 904B → ~100B；release/finalize 的 memcpy 与 `Vec<ArenaSlot>` 增长 memmove 等比缩小；Context 分配次数 O(realm)，与 O(object) 的搬运不可比。
2. `drain_zero_queue` 四次 `mem::replace` 合并为单次 take（`gc.rs:1119-1152`）。
3. `object_edges/property_slot_edges` 改迭代器/SmallVec（`gc.rs:1967` 的 `property_slot_atoms` 已是迭代器先例）。
4. `retain_raw_value_atoms` 消无条件双 Vec；`release_or_defer` 立即分支去重复 drain 检查。

验收：splay/raytrace/earley-boyer 的 memcpy(`0x18f413`) 与 malloc self 收敛；`release_raw_no_drain` self（多例 2-2.7%）减半。（RSS 目标不在本阶段：R9 已判定常数增量来自映像差，由 S13 的 `.text` 收缩承接，验收目标 func_call/closure RSS 相对 S0 回到 ±5%。）

## 3. 用例覆盖矩阵（全部超 5% 差值 → 阶段）

| 用例（相对 S0） | 主治阶段 | 辅助 |
|---|---|---|
| weak_map_set +24.69% / map_set_int +22.12% / map_set_string +15.92% / map_delete +16.58% | S16(方法 IC)、S18(native 外围) | S17(双哈希) |
| int_to_string +20.97% | S14(双转码/空串短路)、S15(Add 驻留) | — |
| string_build1 +14.76% / large2 +14.24% / build3 +11.52% / large1 +9.82% | S14(in-place 追加)、S15(Add 驻留) | R10：build1/build3 单轮含噪声成分（三轮中位 +3.9%/+3.8%），验收以三轮判定 |
| bigint64_arith +9.11% | S15(Add 驻留) | S14 |
| local_destruct +16.59% | S17(pin atom)、S15(Environment/退出税) | S18(iterator native) |
| array_slice +11.82% | S17(批量旁路 + length atom) | R10：三轮中位仅 +0.6%，验收以三轮判定 |
| prop_delete +11.32% | S17(小对象路径/转移表/hasher)、S15(delete 驻留) | — |
| prop_clone +2.35% | S17(转移表/copy 瘦身) | — |
| array_length_decr +1.68% | S17(length atom + locate) | S16(写 IC) |
| math_min +9.70% | S16(全局读缓存/方法 IC)、S18(native 外围)、S14(Box) | — |
| regexp_ascii +11.94% / regexp_utf16 +12.66% / v8-regexp +6.19% | S19 | S16、S20 |
| v8-crypto +11.18% | S16(GetElement/写 IC) | S15、S20 |
| v8-splay +6.79% | S15(DefineField/Not)、S16(IC 门控/多态/写 IC)、S17(转移表) | S20 |
| v8-earley-boyer +13.26% | S15(StrictEq/Not)、S16(IC 门控) | S17、S20 |
| v8-raytrace +6.26% | S16(IC 门控/多态) | S15、S17、S20 |
| float_arith +1.02% / float_to_string +0.24% | 噪声带观察项：S15 退出税与 S14 后三轮复测判定，不单列专项 | — |

**调用探针 12 项 → 阶段：**

| 探针（相对 S0，深度 0/32/128） | 主治阶段 | 说明 |
|---|---|---|
| depth-proxy +54.77%/+52.72%/+47.03% | S18.4/18.5(陷阱帧惰性化 + 4 Box/intern/冷退出收敛)、S15.6(GetField 退出税) | dce0b6ea 时 +67%，S10–S12 已收窄；R8 全清单 |
| depth-getter +24.71%/+23.86%/+13.31% | S18.4(getter 帧惰性 install + 重分类折叠)、S15.6 | R8 |
| depth-native +21.11%/+15.88%/+6.01% | S18.1-3(native publish/witness/argv) | R5 |
| depth-mixed +18.02%/+17.43%/+14.83% | S18.1-6 全项（native ×2 + waitable + 转换） | R5+R8 |

**原始 Score 3 项 → 阶段**（与同名 fixed 用例同根因，随其回收）：crypto −12.21% → S16(GetElement/写 IC)+S15+S20；splay −9.64% → S15+S16+S17+S20；raytrace −9.10% → S16+S15+S17+S20。验收以 Score 相对 S0 转正为准。

**RSS 2 项 → S13**（R9）：func_call +15.74%、func_closure_call +18.64% 判定为映像差，由 S13 退役旧路径的 `.text` 收缩承接，验收目标相对 S0 ±5%；S20 不再背负 RSS 目标。

**编译 5 项 → R10 处置**：判定为测量伪影，不立工程项；S14 验收轮以锁频三轮中位复测关闭（若 v8-deltablue 仍 >S0 +5% 回开专项）。fusion 单遍化与 IC `indices` 稀疏化作为 S17/S20 附带项。

## 4. 测量与纪律

- 每阶段单独 commit、单变量归因；实现期间不做 benchmark/Profile，验收点三轮公平复测（S0 复用旧三轮），cost/cpu Profile 全量 403 项。
- 机械计数验收先于耗时验收：上文各阶段的计数目标不达标即不进入耗时结论。
- **对比口径**：一律三轮中位对三轮中位，单轮对历史中位不得作为回退立项或修复关闭的依据（R10 教训：单轮口径制造了 v8-deltablue 编译 +95.89% 的假回退，也放大了 string_build/array_slice）。
- **环境约束**：正式对比轮要求 `governor=performance`（或记录并锁定频率）、采样前 loadavg 低于核数的 1/2；raw_samples 继续记录 loadavg/governor/scaling_cur_freq，超限样本作废重测（R10 的 r1 样本 loadavg 4.08、powersave、1.91–4.79 GHz 跳频即为反例）。
- 语义门禁不变：全量工作区测试、Test262 结果向量逐位对齐、边界矩阵、oracle 压力。驻留化改动必须保留错误 PC/栈锚点与可重入观察点的既有断言。
- 设计文档先行修订：S15 前更新 `docs/architecture/owned-fusion.md` 的 ConvertAdd 条款；S16 前补写 IC 与多态的失效语义章节；S17 前补 shape 转移表的字典化边界；S18 前补属性驱动被调帧的惰性观察协议边界（哪些 operation/property_wait 状态允许惰性、陷阱返回的快速回执语义）。

## 5. 状态

- [ ] S14 字符串/转换内核
- [ ] S15 run 驻留谱系与退出税
- [ ] S16 属性协议第三期
- [ ] S17 shape/atom/哈希结构
- [ ] S18 调用外围（native + getter/proxy 陷阱帧）
- [ ] S19 regexp 外围
- [ ] S20 heap/GC 结构
- [ ] 编译 5 项伪影：S14 验收轮锁频三轮复测关闭（R10）
- [ ] RSS 2 项：S13 退役旧路径后验收（R9）
