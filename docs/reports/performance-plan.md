# 性能改造计划（heap/value fast path）

本文件记录面向 QuickJS 的常数因子改造：计划、决策、实现与度量。各阶段独立
可验证、可回退，结果随实现更新。

- **S0**：属性读路径成本测量（证据见本文件 §2「热点路径与证据」；负载由 `scripts/benchmark/property_read_probe.py` 复现）
- **S1**：可信快路（trusted fast path）——已实现
- **S2**：可信路径收尾 + 快速释放——部分实现（S2.2；S2.1/S2.3 因契约冲突撤销）
- **S3 及之后**：横向设计比较（见文末「横向设计比较」）→ S3 已定稿为
  `docs/reports/performance-architecture.md`（8B 值表示 + quickening + 数据导向堆，无 JIT、默认无 unsafe）

---

# S1 计划：可信快路（trusted fast path）

> 状态：已实现。下方保留实施前的原始计划，实测结果见本文件「S1 第 9 节」。

S1 只改**热路径的开销**，不改任何 JS 可观察行为，也不换 GC 模型（继续
plain RC + 循环回收）。

## 0. 已决定的取舍

| 项 | 决定 |
| --- | --- |
| 可信快路遇到 stale handle / 内部哨兵 | **panic**（`debug_assert` + release panic），视为不变量被破坏的 bug |
| 可信快路的 refcount 溢出 | **饱和为 immortal**（`u32::MAX`），不返回错误 |
| 现有可失败 `retain_*` 的溢出 | **保持返回 `Err`**，签名与语义不变（不动现有溢出测试） |
| S1b（Symbol atom `Cell` 化、`VarRefData.value` `RefCell` 化） | **留到 S1 验证后再做** |

关键点：**快路与通用可失败路径的溢出行为不同**——快路饱和、通用路径报错。
快路只用于计数很小的已证明存活句柄，饱和在实践中不可达，因此
`src/engine/heap/tests.rs` 里 `retain_edges_transactionally` 的溢出用例无需改动。

## 1. 目标与非目标

**目标**：消除 S0 定位到的三大开销来源，即热路径上的

1. `Result` / `Option` 管道分支；
2. 全局 `RefCell<RuntimeState>` 的**可变**借用（`try_borrow_mut`）；
3. 可避免的 `validate_slot_identity` 校验。

**非目标**：

- 不删除 `Value` 的 runtime `Rc`（属于 S3 值瘦身）。
- 不改 GC 回收时机（保持即时 RC）。
- 不为 public API 或未可信输入放宽错误返回。
- 不改 Symbol 的 atom 表可变借用（S1 走回退路径，见 4.5）。
- 不改现有可失败接口的签名与失败语义。

## 2. 热点路径与证据（来自 S0）

S0 测得 `prop_read_int` 的一次循环迭代：

| 步骤 | 调用链 | 成本 |
| --- | --- | ---: |
| 读捕获对象 `o` | `read_run_cell` → `read_immediate_cell`(shared，decline) → `try_read_owned_var_ref`(`try_borrow_mut`) | 6.47%（其中 `Result::map` 6.29%） |
| 读属性 `o.a` | `stack::property_ic_read_current` → `try_property_ic_read_owned`(`try_borrow_mut`) | int 11.36% / obj 24.98%（`Result::branch` 12.54%） |
| 写立即数 `s`/`i` | `try_write_immediate_cell` → `try_replace_immediate_var_ref_value`(`try_borrow_mut`) | 6.92% |
| `run` 内联 | `Result::branch` + `copy_value` | 14.85% + 7.96% |

结论：主导成本是 **`Result`/`Option` 分支**，其次是可变借用与校验，而不是
引用计数本身。

## 3. 根因

热路径每一步都走可失败、需要 `&mut` 的通用接口：

- `Heap::live_node` / `live_node_mut` / `validate_slot_identity` 返回 `Result`；
- `Node.strong` 是裸 `u32`，retain 需要 `&mut`；
- `Runtime::retain_raw_root` 返回 `Result`（Object 走 heap retain，Symbol 走 atom retain）；
- `Runtime::take_owned_raw_value` 返回 `Result`（拒绝内部哨兵）；
- 于是 `try_*` 层层用 `Result<Option<_>>` 包裹，每次调用一个分支。

这些 `Result` 绝大多数是**防御性不变量**（stale handle / refcount overflow /
内部哨兵），在「句柄已由活对象持有」的热路径上不可能发生。

## 4. 设计：可信快路

核心原则：**内部已证明存活的句柄走不失败、可共享借用的快路；通用可失败
接口保留给边界与冷回退。**

### 4.1 refcount 改为 `Cell` + 新增快路 retain

- `Node { strong: u32, data }` → `Node { strong: Cell<u32>, data }`
  （`src/engine/heap/mod.rs`），`SlotState::strong()` 用 `.get()`。
- **现有** `Heap::retain_raw(&mut self, id, additional) -> Result` 及其所有 caller
  **签名与错误语义不变**，只把字段读写改成 `.get()/.set()`。溢出仍返回 `Err`
  （保住 `retain_edges_transactionally` 的现有测试）。
- **新增** 快路：
  ```rust
  #[inline]
  pub(in crate::engine::heap) fn retain_raw_fast(&self, id: RawId) {
      let node = self.live_node_fast(id);
      node.strong.set(node.strong.get().saturating_add(1));
  }
  ```
  仅供快路使用，饱和后不可回退（immortal）。
- release 的「归零入队」仍需 `&mut`，`release_raw_no_drain(&mut self)` 保持不变；
  release 读/写 `Cell` 用 `.get()/.set()`，`u32::MAX` 视为 immortal（不递减）。
- 更新全部 `.strong` 直接字段访问（`gc.rs`、`arena.rs`、`slot_ownership.rs`、
  `roots.rs`、`mod.rs`、`tests.rs`，约 32 处）。

### 4.2 可信访问器（无 `Result`、省 generation 校验）

在 `Heap` 上新增仅供**内部已证明存活句柄**使用的访问器：

```rust
impl Heap {
    /// Trusted: caller holds a live owning edge. Bounds-checked index; the
    /// generation/state check runs in debug builds only.
    #[inline]
    pub(in crate::engine::heap) fn live_node_fast(&self, id: RawId) -> &Node {
        debug_assert!(self.validate_slot_identity(id).is_ok());
        match &self.slots[id.index() as usize].state {
            SlotState::Live(node) => node,
            _ => unreachable!("trusted handle reached a non-live slot"),
        }
    }
    #[inline]
    pub(in crate::engine::heap) fn var_ref_fast(&self, id: VarRefId) -> &VarRefData { ... }
    #[inline]
    pub(in crate::engine::heap) fn object_fast(&self, id: ObjectId) -> &ObjectData { ... }
}
```

保留现有 `live_node`/`validate_slot_identity` 作为边界与测试路径。注意 release
构建仍会做 `Vec` 越界检查（安全 Rust，不用 `get_unchecked`）。

### 4.3 无失败根转换

- 新增 `Runtime::take_owned_raw_value_fast(&self, raw: RawValue) -> Value`：
  仅接受公开变体（Undefined/Null/Bool/Int/Float/BigInt/String/Symbol/Object），
  内部哨兵走 `debug_assert` + panic。逻辑复制自 `src/engine/heap/roots.rs:345`。
- 新增 `Runtime::retain_object_root_fast(&self, id: ObjectId)`：调用于共享借用
  下的 `heap.retain_raw_fast(RawId::Object(id))`，不返回 `Result`。

### 4.4 快路函数与调用点改写

新增（原函数保留为冷回退）：

- `Runtime::read_owned_cell_fast(&self, root) -> Option<Value>`
  - `state.try_borrow()`（共享）；
  - `heap.var_ref_fast` 读单元；
  - 仅处理 Object/String/BigInt（String/BigInt 克隆自带所有权，Object 用
    `retain_object_root_fast`）；
  - Symbol 与标量 → `None`，回退 `try_read_owned_var_ref`。
- `bindings::read_run_cell_fast(runtime, root) -> Option<Value>`
  - `read_immediate_cell`（现有，shared）→ else `read_owned_cell_fast`。
- `Runtime::property_ic_read_fast(base, executable, pc, key, keep_receiver, native) -> Option<Value>`
  - 复制 `try_property_ic_read_owned`（`ic.rs:12`）的**数据属性命中**分支：
    - 共享借用；`heap.object_fast` / `cache.read` / `slot_object_release_readiness` 均 `&self`；
    - Object/String/BigInt 用可信 retain；
    - `keep_receiver` / native 选择 / accessor / 描述符等情形 → `None` 回退原函数。

调用点改写：

- `src/engine/vm/bindings.rs:64` `read_run_cell`：先 `read_immediate_cell`，再
  `read_owned_cell_fast`，最后才回退 `try_read_owned_var_ref`；不再包 `Result`。
- `src/engine/vm/run.rs` 的 `GetVarRef`/`GetArg` captured 分支改用
  `read_run_cell_fast`，去掉 `?`。
- `src/engine/vm/stack.rs:114` 与 `src/engine/object/ordinary_storage/ic.rs`
  的属性读调用点：先 `property_ic_read_fast`，`None` 再走原可失败路径。
- `try_replace_immediate_var_ref_value` 保持 `bool`；删除其中与前置判断重复的
  `validate_var_ref_value` 调用，并用 `var_ref_fast_mut`（新增，可信 `&mut`）。

### 4.5 Symbol / 立即写的原因与边界

- **Symbol**：`AtomTable::retain` 需要 `&mut`（atom 表可变），无法在共享借用
  下完成。S1 让 Symbol 读回退到 `try_read_owned_var_ref`（S1b 可把 atom 的
  `ref_count` 也 `Cell` 化）。
- **立即写**（`i`/`s`）：要修改 `VarRefData.value`（`RawValue`，非 `Copy`），
  仍需要 `&mut`。S1 只降低其校验与 `Result` 开销，不消除可变借用；彻底消除
  需要把单元值改成 `RefCell<RawValue>`（S1b）。

## 5. Scope 清单

| 文件 | 改动 |
| --- | --- |
| `src/engine/heap/mod.rs` | `Node.strong: Cell<u32>`；`SlotState::strong` |
| `src/engine/heap/gc.rs` | `.strong.get()/.set()`；新增 `retain_raw_fast` |
| `src/engine/heap/arena.rs` | 新增 `live_node_fast` / `var_ref_fast_mut` 等可信访问器 |
| `src/engine/heap/slot_ownership.rs` | `.strong.get()` |
| `src/engine/heap/roots.rs` | 新增 `take_owned_raw_value_fast`、`read_owned_cell_fast` |
| `src/engine/heap/binding_storage.rs` | 精简 `try_replace_immediate_var_ref_value`；`var_ref_fast_mut` |
| `src/engine/heap/runtime/mod.rs` | 新增 `retain_object_root_fast` |
| `src/engine/vm/bindings.rs` | 新增 `read_run_cell_fast` |
| `src/engine/vm/run.rs` | captured 读调用点改快路 |
| `src/engine/vm/stack.rs` | 属性读调用点改快路 |
| `src/engine/object/ordinary_storage/ic.rs` | 新增 `property_ic_read_fast` |
| `src/engine/heap/tests.rs` | `.strong` 测试适配 |

预计 12 个源文件，不新增模块，不改 public API。

## 6. 算法草图

```rust
// heap/gc.rs
#[inline]
pub(in crate::engine::heap) fn retain_raw_fast(&self, id: RawId) {
    let node = self.live_node_fast(id);
    node.strong.set(node.strong.get().saturating_add(1));
}

// heap/roots.rs
#[inline]
pub(crate) fn read_owned_cell_fast(&self, root: &VarRefRoot) -> Option<Value> {
    if !root.belongs_to(self) || self.0.deferred_references.has_pending() {
        return None;
    }
    let state = self.0.state.try_borrow().ok()?;      // shared, 非 mut
    if !state.heap.zero_queue.is_empty() { return None; }
    let cell = state.heap.var_ref_fast(root.id());
    match &cell.value {
        RawValue::Object(object) => {
            state.heap.retain_raw_fast(RawId::Object(*object));
            Some(Value::Object(ObjectRef::from_owned_handle(self.clone(), *object)))
        }
        RawValue::String(s) => Some(Value::String(s.clone())),
        RawValue::BigInt(b) => Some(Value::BigInt(b.clone())),
        _ => None,                                     // Symbol/标量回退
    }
}

// object/ordinary_storage/ic.rs（数据属性命中分支）
#[inline]
pub(crate) fn property_ic_read_fast(
    &self, base: &Value, executable: &PublishedFunctionSnapshot,
    pc: usize, key: u32, native: &mut Option<LinkedNativeSelection>,
) -> Option<Value> {
    let atom = linked_field_atom(self, executable, key)?;
    let Value::Object(object) = base else { return None };
    if !object.belongs_to(self) { return None; }
    let cache = executable.property_read_ic.site(pc)?;
    let state = self.0.state.try_borrow().ok()?;
    if state.heap.has_pending_zero_cleanup() { return None; }
    let receiver = object.object_id();
    let raw = cache.read(&state.heap, self.domain_id(), executable.realm, receiver)?.clone();
    match raw {
        RawValue::Object(id) => {
            state.heap.retain_raw_fast(RawId::Object(id));
            Some(Value::Object(ObjectRef::from_owned_handle(self.clone(), id)))
        }
        RawValue::String(s) => Some(Value::String(s)), // Rc clone 已在 clone() 中
        RawValue::BigInt(b) => Some(Value::BigInt(b)),
        _ => None,
    }
}
```

## 7. 分步提交计划

1. `perf(heap): make refcounts Cell-backed` —— 仅 4.1 的字段机械化改造 +
   新增 `retain_raw_fast`，行为不变，测试通过。
2. `perf(heap): add trusted non-fallible accessors` —— 4.2/4.3，未被调用，
   行为不变。
3. `perf(vm): add shared-borrow fast paths for captured reads` ——
   `read_owned_cell_fast` / `read_run_cell_fast` + `run.rs` 调用点。
4. `perf(object): add property-read fast path` —— `property_ic_read_fast` +
   `stack.rs`/`ic.rs` 调用点。
5. `perf(heap): slim immediate var-ref write validation` ——
   `try_replace_immediate_var_ref_value` 精简 + `var_ref_fast_mut`。
6. `docs(perf): record S1 measurements` —— 更新 S1 报告数据。

每步独立可编译、可测；若某步无收益可单独回退。

## 8. 验证与度量

1. `cargo fmt --check`。
2. `cargo clippy --locked --all-targets -- -D warnings`。
3. `cargo test --locked --workspace`。
4. Test262 冻结向量：`pass=79982 / eligible=80032 / total=102037` **不得倒退**。
5. `python3 scripts/checks/check-source-layout.py` 及 rust-only 门禁。
6. 重跑 `scripts/benchmark/property_read_probe.py`，与 S0 对比。
7. profiling 构建对比 `heap_root_copies`，确认复制次数未异常变化。

## 9. 实施结果

已按本计划实现（`Cell` refcount、可信访问器、无失败根转换、captured/属性读快路、
立即写校验精简）。行为不变。

### 计时对比（`property_read_probe.py`，N=5,000,000，median，同机）

| case | engine | S0 ns/op | S1 ns/op | 变化 |
| --- | --- | ---: | ---: | ---: |
| prop_read_int | plain | 222.55 | 189.95 | −14.6% |
| prop_read_obj | plain | 270.94 | 243.62 | −10.1% |
| prop_read_string | plain | 299.68 | 273.83 | −8.6% |
| prop_read_int | profiling | 343.98 | 309.44 | −10.0% |
| prop_read_obj | profiling | 409.00 | 355.74 | −13.0% |
| prop_read_string | profiling | 437.11 | 406.46 | −7.0% |

### perf 归属变化

- `run::run` 自耗时从 42.50% 降到 36.29%（int）/ 33.99% 降到 29.13%（obj）。
- 原 `try_property_ic_read_owned` 的 `Result::branch` 主导项消失；快路内联后
  归属到 `RunSlots::property_ic_read`，不再看到成片的 `Result::branch` 子项。
- `try_replace_immediate_var_ref_value` 去掉重复 `validate_var_ref_value`。

### 完整性能对比（pre-S1 `34db437c` vs S1 `fe537951`）

基线在独立 worktree 构建；两者使用**相同 flags 的普通 release**（无 debug、
无 profiling），串行运行，无并发构建/测试。

**属性读探针（`property_read_probe.py`，N=5,000,000，repeat 7，median）：**

| case | before ns/op | after ns/op | 变化 |
| --- | ---: | ---: | ---: |
| prop_read_int | 222.87 | 185.38 | −16.8% |
| prop_read_obj | 279.78 | 234.58 | −16.2% |
| prop_read_string | 306.63 | 284.84 | −7.1% |

**`scaling.py`（整进程 wall，16 case × 3 size = 48 单元，operations=32768，
repeat 3）：** 中位 `after/before = 97.0%`（**−3.0%**），区间 −15% ~ +23%
（小负载启动噪声大）。稳定收益集中在 S1 直接命中的路径：`long-key`
−8~−13%、`map-churn` −4~−14%、`arguments` −7~−15%、`array-holey`
−3.5~−7.5%、`array-index` −10~−12%、`typed-index` −3~−10.5%、
`mapped-arguments` −1~−9.5%、`scope` 多为 −6~−12%。`map-*`/`set*` 基本中性。

**QuickJS 官方 `microbench`（毫秒分辨率）：** `prop_read`/`array_read`/
`int_arith` 前后均为定值、无法分辨 ~10% 变化；仅用于标定与 QuickJS 的差距
（对应项 QuickJS 约 10–25 倍快）。

**V8-v7（Score，越高越好，repeat 3）：**

| case | before | after | quickjs | after 变化 |
| --- | ---: | ---: | ---: | ---: |
| richards | 48.7 | 48.6 | 1378 | −0.2% |
| deltablue | 63.4 | 62.2 | 1259 | −1.9% |
| crypto | 60.9 | 61.8 | 1522 | +1.5% |
| raytrace | 94.2 | 93.6 | 2832 | −0.6% |
| earley-boyer | 117 | 117 | 3504 | 0% |
| regexp | 87.6 | 86.9 | 639 | −0.8% |
| splay | 316 | 313 | 5118 | −0.9% |
| navier-stokes | 254 | 262 | 3224 | +3.1% |

V8-v7 整体**中性（±2%，噪声内）**：计算密集型套件中属性读/binding 不是主导。

**小结：** S1 是定向优化——目标路径提升 7–17%，混合整进程负载中位 −3%，
计算密集型套件中性；不是全局加速，符合计划定位。

### 验证

- `cargo test --locked --workspace --all-targets`：通过（lib 2278、oracle 907、
  CLI 32、rust-only 4、unsupported-diagnostics 6 等，0 失败）。
- `cargo fmt`：通过。
- clippy：本次改动未新增 lint；1.88/1.94 下报出的均为仓库既有 lint
  （`collapsible_if`、`manual_is_multiple_of`、测试 cfg 的 unused/dead_code）。
- **Test262 全量通过、零回归**：`TEST262_WORKERS=2 ./scripts/test262/test-test262.sh
  --full` 得到
  `total=102037 pass=79982 fail=3580 unsupported=3530 skipped=18475`，
  `runnable=80032`，门禁判定 `complete Test262 vector matches:
  79982 pass of 80032 eligible (102037 total) variants`，与冻结基线逐字节一致。
  该次运行只产出 current-source receipt，**未修改 `current.conf`**（符合 README
  的“不得为性能改动修改基线”）。注：`prepare-test262.sh` 会拒绝任何 `GIT_*`
  环境变量，运行前需清理。

## 10. 预期与风险

- **预期**：消除读路径的 `Result` 分支与可变借用。S0 显示 `Result` 管道合计
  >30%，其中读路径占大部分；S1 目标是把其中可移除的部分拿掉，期望整体有
  可测的正收益（量级待测，不承诺具体倍数）。
- **风险**：
  - 可信快路的 panic 可能暴露此前被静默返回错误的真实不变量 bug —— 由
    Test262 与 workspace 测试兜底。
  - 共享借用与活动 `borrow_mut` 冲突时快路返回 `None` 回退，行为与现状一致。
  - `property_ic_read_fast` 只覆盖数据属性命中；其余走原路径，正确性不受影响。

---

# S2 计划：可信路径收尾 + 快速释放

## S2.0 背景与重定义

原 S2 目标是「热路径去 generation 校验」，但 S1 之后该目标**已基本达成**：
release 构建下 `live_node_fast`/`object_fast`/`var_ref_fast` 已省掉 generation
校验，重采样里 `slot_ownership`（ready 校验）仅约 **0.2%**。

S1 后 `prop_read_int` 热点（debug 构建，`perf report --no-children`）：

| 符号 | 占比 |
| --- | ---: |
| `vm::run::run` | 36.26% |
| `RunSlots::property_ic_read` | 14.97% |
| `SlotStore::push_current` | 10.48% |
| `try_replace_immediate_var_ref_value` | 9.36% |
| `bindings::read_run_cell` | 7.75% |
| `run::binary` | 4.94% |
| `RunSlots::insert_copy` | 4.20% |
| `apply_deferred_operation` + `release_raw_no_drain` + `release_or_defer` | ~4.1% |
| `slot_ownership`（generation ready） | 0.18% |

因此把 S2 重定义为**三个仍可执行、可度量的子项**，继续不动 GC 模型、不动值大小：

- **S2.1 快速释放**：释放路径在共享借用下用 `Cell` 递减，避免大部分独占借用与
  generation 校验。
- **S2.2 可信 IC 读**：`PropertyReadCache::read_location` 与 readiness 证明改用
  可信访问器，去掉 IC 读里的 generation 校验。
- **S2.3 非失败 operand push**：`push_current` 的不变量检查改为非失败，去掉
  `Result` 分支。

共享原则与 S1 一致：**可信路径对不变量破坏 panic；通用可失败路径保留。**

## S2.1 快速释放

### 现状

`ObjectRef::drop` → `Runtime::release_object_handle`（`ownership.rs:68`）→
`release_or_defer(DeferredRefOp::Object(id))`（`ownership.rs:44`）：

```
try_borrow_mut(state)                      // 独占借用整个 runtime
→ apply_deferred_operation
  → release_heap_reference
    → heap.release_object
      → release_raw_no_drain               // validate_slot_identity（generation）
→ drain_deferred_references()
```

即每次释放都占独占借用、做 generation 校验；共享对象的递减本不需要这些。

### 设计

- 新增 `Heap::release_raw_fast(&self, id: RawId) -> bool`（`gc.rs`）：
  ```rust
  /// Trusted: only decrements while another owner remains. Returns false when
  /// the count is 1, leaving the zero transition to the ordinary fallible path.
  #[inline]
  pub(in crate::engine::heap) fn release_raw_fast(&self, id: RawId) -> bool {
      let node = self.live_node_fast(id);
      let current = node.strong.get();
      if current > 1 {
          node.strong.set(current - 1);
          true
      } else {
          false
      }
  }
  ```
  （`live_node_fast` 在 release 下不校验 generation；`current == 0` 不会出现，
  Live 蕴含 `strong >= 1`。）
- `Runtime::release_object_handle`（`ownership.rs`）：
  ```rust
  pub(crate) fn release_object_handle(&self, id: ObjectId) {
      if let Ok(state) = self.0.state.try_borrow() {
          if state.heap.release_raw_fast(RawId::Object(id)) {
              return; // count>1, 无归零清理
          }
      }
      self.release_or_defer(DeferredRefOp::Object(id)); // 只能 count==1 或借用失败
  }
  ```
  `live_node_fast` 若遇到非 Live 会 panic（可信路径语义）。count==1 时不递减，
  交由 `release_or_defer` 正常递减归零入队，**不会重复递减**。
- 对 `release_var_ref_handle` / `release_context_handle` /
  `release_function_bytecode_handle` 同样处理（各自 `release_raw_fast(RawId::X)`）。
- `Atom` 保持原路径（需要 atom 表可变借用）。

### Scope

| 文件 | 改动 |
| --- | --- |
| `src/engine/heap/gc.rs` | 新增 `release_raw_fast` |
| `src/engine/heap/ownership.rs` | 4 个 `release_*_handle` 加快速分支 |

估计 ~40 行。

### 预期

削掉共享对象释放的独占借用 + generation 校验 + `apply_cleanup`。收益取决于负载中
「引用计数 >1 的对象」比例（对象/数组共享多的负载更高）。`count==1` 的临时值仍
走慢路，因此 **S2.1 不是普适加速**。

## S2.2 可信 IC 读

### 现状

`PropertyReadCache::read_location`（`object/property_ic.rs:74`）用可失败、带
generation 校验的访问器：

```rust
let object = heap.object(receiver).ok()?;                 // validate
let shape = heap.shape(object.shape).ok()?;               // validate
// depth 循环里同样 object()/shape()
```

`property_ic_read_fast`（`ordinary_storage/ic.rs`）调用
`slot_object_release_readiness`（`slot_ownership.rs:22`）→ `validate_slot_identity`。

### 设计

- 新增 `Heap::shape_fast(&self, id: ShapeId) -> &Shape`（`object_storage.rs`，紧邻
  `shape`），与 `object_fast` 同型：`debug_assert` 存活，非 Live panic。
- `read_location` 改用 `object_fast`/`shape_fast`。命中路径的 `receiver` 与原型链
  `holder` 都是活对象（由活 receiver 可达，且 shape/revision/epoch 已判定匹配），
  可信。
- 新增 `Heap::slot_release_readiness_fast(&self, id: RawId) -> SlotReleaseReadiness`
  （`slot_ownership.rs`）：去掉 `validate_slot_identity`，其余逻辑不变
  （zero_queue 检查 + strong 分支）。
- `property_ic_read_fast` 改调 `slot_release_readiness_fast`。

### Scope

| 文件 | 改动 |
| --- | --- |
| `src/engine/heap/object_storage.rs` | 新增 `shape_fast` |
| `src/engine/heap/slot_ownership.rs` | 新增 `slot_release_readiness_fast` |
| `src/engine/object/property_ic.rs` | `read_location` 改可信访问器 |
| `src/engine/object/ordinary_storage/ic.rs` | `property_ic_read_fast` 改调 fast readiness |

估计 ~50 行。

### 风险

`read_location` 也被 Proxy trap 缓存等复用；这些调用点的 receiver 同样来自活值，
可信。若非可信调用者存在，保留原 `read` 走可失败路径即可（S2 只改热路径调用）。

## S2.3 非失败 operand push

### 现状

`SlotStore::operand_push_index`（`vm/stack.rs:925`）返回 `Result<usize, Error>`，
两个检查都是 VM 已验证的不变量（操作数容量、目标槽为空）；`push_current`/
`push_pending_current` 因此返回 `Result`，调用点用 `?`。

### 设计

- `operand_push_index` 改非失败：
  ```rust
  #[inline]
  fn operand_push_index(&self, window: &FrameWindow) -> usize {
      debug_assert!(window.depth < window.operands().len());
      debug_assert!(self.slots[index].is_none());
      ...
      index
  }
  ```
  越界/占位按不变量破坏 panic。
- `push_current`/`push_pending_current` 去掉 `Result`；`insert_copy_current` 内部
  调用相应调整；对外的 `push`（`vm/stack/window.rs:288/296`）保留 `Result` 或同步
  改非失败（按调用者需要）。
- 全仓库仅 5 个调用点，改动可控。

### Scope

| 文件 | 改动 |
| --- | --- |
| `src/engine/vm/stack.rs` | `operand_push_index`/`push_current`/`push_pending_current` |
| `src/engine/vm/stack/window.rs` | `push`/`push_pending` 包装 |

估计 ~60 行。

## S2.4 提交与验证

- 分 3 个 commit：`perf(heap): add fast shared release`、`perf(object): use trusted
  accessors in the property-read cache`、`perf(vm): make operand pushes infallible`。
  每步独立可编译/可测。
- 验证：`cargo fmt`、workspace `--all-targets`、`TEST262_WORKERS=2 ... --full`
  零回归、`property_read_probe.py` + `scaling.py` 前后对比（pre-S2 vs S2）。
- 老规矩：`prepare-test262.sh` 会拒绝 `GIT_*` 环境变量，需先清理。

## S2.5 风险与预期

- **失败模式**：与 S1 一致，可信路径把不变量破坏当 bug（panic）。
- **预期量级**：S2.1 释放路径约 4%，S2.2 削 `property_ic_read` 的 15% 中的
  generation 部分，S2.3 削 `push_current` 的分支部分；合计**个位数百分比**。
  诚实地说，剩余大头（`run` 36% + 值/槽表示）要靠 S3。
- **S3（下一步）**：`Value` 从 32B 瘦身（去掉 runtime `Rc`、thin 句柄 + 显式
  retain，保持计数以免改 GC 根），目标 `push_current`/`insert_copy`/`copy_value`/
  drop 合计约 20% 与所有值搬运。

## S2.6 实施结果

实施中发现 **S2.1 与 S2.3 与既有契约冲突，已撤销**；只落地 **S2.2**。

- **S2.1 快速释放（撤销）**：既有测试
  `heap::slot_ownership::blocked_borrow_and_deferred_release_do_not_commit_or_drain`
  把「**任意**借用（含共享）都使释放 defer、不立即提交」固定为契约。快速释放用
  共享借用递减，会在已持有共享借用时成功提交，改变该契约与 deferred 出队顺序。
  需要单独立项评估该松弛是否安全，故本次不做。
- **S2.3 非失败 push（撤销）**：`operand_push_index` 的容量/占位失败是**被显式
  测试覆盖的可恢复事务路径**（`failed_capacity_and_shape_checks_do_not_change_live_windows`、
  多个 `primitive_transaction_*` 测试）。改为 panic 会破坏该契约，故保持可失败。
- **S2.2 可信 IC 读（已落地）**：
  - 新增 `Heap::shape_fast`（`object_storage.rs`）。
  - `PropertyReadCache::read_location`（`object/property_ic.rs`）改用
    `object_fast`/`shape_fast`。
  - 新增 `Heap::slot_object_release_readiness_fast`（`slot_ownership.rs`），
    `property_ic_read_fast` 改用，去掉读路径上的 `validate_slot_identity`。

### S2.2 计时（`property_read_probe.py`，N=5,000,000，repeat 7，median）

| case | before ns/op | S1 ns/op | S2 ns/op | S1→S2 | before→S2 |
| --- | ---: | ---: | ---: | ---: | ---: |
| prop_read_int | 222.73 | 185.97 | 184.35 | −0.9% | −17.2% |
| prop_read_obj | 281.70 | 236.71 | 220.22 | **−7.0%** | **−21.8%** |
| prop_read_string | 300.87 | 280.10 | 280.86 | +0.3% | −6.7% |

S2.2 的收益集中在**对象结果的属性读**（IC 命中里 object/shape 的可信访问 +
readiness 无校验）；int/string 基本不变（噪声）。

### 验证

- `cargo test --locked -p quickjs-oxide --lib`：2259 通过。
- `cargo test --locked --workspace --all-targets`：全部通过（lib 2278、oracle 907、
  CLI 32 等，0 失败）。
- `cargo fmt`：通过。
- Test262 全量零回归：`TEST262_WORKERS=2 ./scripts/test262/test-test262.sh --full`
  得 `total=102037 pass=79982 runnable=80032`，门禁判定
  `complete Test262 vector matches`，与冻结基线逐字节一致（仅产出 current-source
  receipt，未改 `current.conf`）。

### 结论

S2.2 是安全的增量（对象属性读 S1→S2 −7%）；S2.1/S2.3 的正确做法需要改动既有
事务/延迟释放契约，应作为独立设计项，而不是塞进性能 PR。下一阶段（值表示 /
派发 / 自适应特化）见文末「横向设计比较」。

---

# 横向设计比较：参考引擎 vs quickjs-oxide

> 原 S3「值表示瘦身」计划已移除，改为本横向比较，作为重新设计 S3 的依据。目标
> 不是「补齐常数因子」，而是对齐/超过参考引擎的关键设计。

## 1. 总表

| 引擎 | 值表示 | 属性键 | GC / 回收 | 分配器 | 派发 | IC / 自适应特化 | JIT |
| --- | --- | --- | --- | --- | --- | --- | --- |
| **QuickJS** | **8B NaN-box** `JSValue` | `JSAtom` = 裸 `u32` | 侵入式 RC + 循环回收 | 自研 `js_malloc` | 栈式 + switch/threading | 少量静态快路径，**无 quickening** | 无 |
| **Lua 5.4** | 16B `TValue`（union+tag） | interned `TString*` 指针 | 增量/分代标记清除 | size-class | **寄存器式** | 无 | 无（LuaJIT 另立） |
| **LuaJIT** | 8B NaN-box | interned 指针 | tracing GC + RC | — | 解释器 + 汇编桩 | — | **tracing JIT** |
| **V8** | 8B / 32-bit 压缩 `Tagged` | `Name` 指针 | 分代 tracing（并发/增量） | bump nursery + size-class | **寄存器式（Ignition）** | feedback vector + 多态 IC | Sparkplug / Maglev / TurboFan |
| **JSC** | 指针 tagging | `Identifier`/`Name` | 分代 tracing | — | 手写汇编 LLInt + Baseline | 多态 IC | LLInt / Baseline / DFG / FTL |
| **SpiderMonkey** | 指针 tagging | `Name` | 分代 tracing | — | Baseline 解释器 | CacheIR | Warp |
| **CPython** | `PyObject*` 8B 指针 | 任意对象（interned `str`） | RC + 分代循环 GC | pymalloc / size-class | 栈式 | **PEP 659 特化 + IC** | 无（3.13 实验副本补丁，默认关） |
| **Boa** | Rust enum `JsValue`（~16B） | interner | `Gc<T>` 标记清除 | — | 栈式 | 无 | 无 |
| **quickjs-oxide（现状）** | **32B enum**（实测） | `Atom` = raw+generation+table_id（16B） | arena RC + 循环回收 | arena `Vec<ArenaSlot>` | 栈式 | mono/poly-2 IC + fusion，**无 quickening** | 无 |

## 2. 逐维度要点与启示

### 值表示
- 主流是 **8B**：QuickJS/LuaJIT 用 NaN-box；V8 用指针 tagging + 指针压缩；CPython 是 8B `PyObject*` 指针。**没有任何主流引擎用 32B 带标签枚举**（那是 Boa 与 quickjs-oxide 这类安全 Rust 实现的产物）。
- 8B 的意义：一条 64B 缓存行放 8 个值（而非 2 个）、复制是一条 `mov`、可进 CPU 寄存器。
- **quickjs-oxide 的 32B** 是最大差距；要追平 QuickJS，值表示必须降到 8B（NaN-box 或 thin 指针）。这需要 `unsafe`，与当前 `forbid(unsafe_code)` 冲突（`parity.md` 已允许受审计 `unsafe`）。

### 属性键 / atom
- QuickJS：裸 `u32`；Lua：interned 指针；V8/CPython：指针。**都无 per-handle 品牌**；有效性靠构造（per-runtime 表、不跨域）或 tracing（活对象不复用）。
- **quickjs-oxide 的 16B `Atom`（raw + generation + table_id）** 是 Rust arena 安全的额外产物，比 QuickJS 每个键多 12 字节；shape entry、属性键内存同受其累。
- 启示：去品牌 + 裸句柄能缩小键/shape，但只覆盖「值表示」的一个子维度；参考引擎靠「指针 + GC/构造」而非句柄品牌。

### GC / 分配
- 回收：侵入式 RC + 循环回收（QuickJS）vs 分代 tracing（V8/JSC/SpiderMonkey）vs RC + 分代循环（CPython）。
- 分配：bump nursery（V8）或 size-class（CPython）vs 你们的分代 arena。
- **quickjs-oxide**：arena + `Cell` strong + 全局 `RefCell` + generation 校验，是 Rust 安全的产物，也是常数因子的主要来源。方向是 bump + 侵入式（需 `unsafe`），或至少去掉全局借用/校验。

### 派发
- 栈式：QuickJS、CPython、Boa、**quickjs-oxide**。
- 寄存器式：Lua 5.4、V8 Ignition——指令更少、栈流量更少。
- 栈式优化：**stack caching**（Ertl：栈顶若干槽放寄存器）。
- **quickjs-oxide**：`run` 自耗时约 **36%**，派发是最大单点；寄存器式或 stack caching 是主要杠杆。

### IC / 自适应特化
- V8/JSC/SpiderMonkey：feedback vector + 多态/megamorphic IC。
- CPython：**PEP 659 specializing adaptive interpreter**（quickening）。
- QuickJS / Lua：基本没有。
- **quickjs-oxide**：mono/poly-2 IC + `fusion`（superinstruction） + 编译期常量折叠 + resident 手写快分支；**没有 type feedback、专用 opcode、deopt**，因此**没有 quickening**。
- 启示：这是**纯安全 Rust 可做**、且 QuickJS 没有的少数优势点，优先级应高。

### JIT
- 有：V8、JSC、SpiderMonkey、LuaJIT。无：QuickJS、Lua、CPython（默认）。
- 本项目的 JIT 被排除，因此**上限是「极致解释器」**：可追平/略超 QuickJS，但拿不到 V8/LuaJIT 那种数量级。

## 3. 结论：2× 目标（无 JIT）需要什么

按杠杆排序：

1. **值表示 8B + 侵入式 RC**（需受审计 `unsafe`）—— 追平 QuickJS 的入场券。
2. **quickening + 更深 IC**（纯安全 Rust，QuickJS 没有）—— 确定的优势点。
3. **派发改造**：stack caching 先行，评估后再决定是否寄存器式 VM；配 superinstruction。
4. **分配器**：bump + 内联属性 + 去 arena/`RefCell`/`Result` 间接。
5. **JIT 排除**：2× 是极限目标；且安全 Rust 相对 C 仍有税，需靠 2/3/4 补回。

这份比较取代原 S3 计划。S3 已据此定稿为 **`docs/reports/performance-architecture.md`**：
「8B 值表示 + quickening + 数据导向堆」的组合，而不是单纯的 16B 瘦身。
相对上面第 1 条有一处关键修正：8B 值表示**不需要 unsafe**——本项目句柄
本就是 arena 索引（`ObjectId{index,generation}`），把 u32 索引装进 NaN
payload 是纯位运算，「索引 NaN-box」在安全 Rust 内成立；受审计 unsafe
降级为保留席位（performance-architecture.md 方案 F）。
