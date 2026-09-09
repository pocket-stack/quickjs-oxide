# CPU 热点定位报告 — PocketLab · 2026-09-09

已完成工作负载隔离、系统 CPU 栈采样，以及针对热点的计数/阶段计时。现在有证据定位到具体函数：**属性名被重复 intern、普通对象提前处理数组键、频繁引用释放/清理、整数操作经过通用桥接，以及 Context 初始化中的布局重建。** 本轮没有实施性能优化，下面的占比不能当作优化后的收益承诺。

所有构建和运行均在 `eric-83am` 的 Herdr PocketLab `quickjs-oxide` 终端执行。原始产品源码为 `9ed39e275d0f1b74ce82ed694ebaef797d6b79d7`。临时插桩仅进入 `target/cpu-investigation/instrumented-source` 的独立源码副本，产品源码没有加入这些探针。

[机器可读证据](cpu-hotspots.json) · [原有 benchmark 基线](profiler-baseline.md) · [诊断补丁与火焰图说明](cpu-hotspots/README.md)

## 最值得先修改的位置

| 顺序 | 具体问题 | 直接证据 | 修改候选 |
| --- | --- | --- | --- |
| 1 | 固定属性名反复处理；普通对象也先 intern `"length"` | 固定属性读取中 `constant_property_key` 包含子调用占 **33.45%**；Richards 中为 **24.44%**。Richards 的 `array_own_key` 占 **13.10%**，其中 **98.18%** 的调用接收非数组对象 | 优先检查非数组分支的顺序；评估在字节码发布/链接时持有固定键的 Atom，避免执行时反复从字符串 intern |
| 2 | 短命对象引用反复进入清理路径 | Richards 中 `release_object_handle` 自身占 **8.26%**，`drain_deferred_references` 自身占 **4.22%**；固定属性读取调用清理入口约 1.4 亿次，只处理 24 个 deferred 操作 | 检查普通数据属性读取中的临时 ObjectRef 克隆、引用未归零时的释放路径，以及空队列处理成本 |
| 3 | 已知数值/直接局部变量仍经过通用 VM 桥接 | 固定空循环中 `read_frame_binding` 自身 **17.69%**，`update_numeric` 自身 **16.14%**；1,000 万次迭代触发约 3,000 万次 `to_primitive`，输入全部是 Number | 评估数值更新/比较的 Number 快路径和直接局部变量访问，保留对象转换、BigInt、溢出及异常路径 |
| 4 | Context 初始化反复构造和替换布局 | 生命周期采样中 `replace_layout` 包含子调用占 **55.88%**；每个 Context 有 778 次布局替换、943 次 shape 查询 | 评估内建属性表批量构建、重复 shape/fingerprint 工作，以及少量边的事务保留路径 |

表中包含子调用的占比会重叠，不能相加。例如 `array_own_key` 和 `constant_property_key` 都会进入 Atom 处理；Atom 内部的字符串迭代成本又包含在上层占比里。Richards 的外层调用栈存在深度上限，详细限制见后文。

## 1. 属性读取：慢在取值之前，也慢在临时引用处理

固定工作量采用上游 `prop_read` 的原始函数体：循环 5,000,000 次，每次读取 `obj.a/b/c/d`，总计 **20,000,000 次读取**，最终校验和 `50,000,000`。原函数体未经修改，去除了外部自校准计时循环。

三次计数运行中，以下结构性计数完全一致；计数包括一次 CLI 初始化和收尾：

| 指标 | 调用/处理次数 |
| --- | ---: |
| `get_field` | 20,000,002 |
| `constant_property_key` | 20,000,006 |
| `AtomTable::intern_property_key_js_string` | 20,001,385 |
| `get_own_property` | 20,000,525 |
| 对象引用 retain / release | 40,000,710 / 40,000,881 |
| `drain_deferred_references` 入口 / 实际处理操作 | 140,008,893 / 24 |
| zero queue 清理入口 / 进入时为空 | 40,002,807 / 40,002,016 |

这不是“创建了 4,000 万个 JS 对象”：retain/release 统计的是句柄操作。`drain_deferred_references` 的末尾空队列返回也不能直接当作“入口为空”；报告使用的是入口次数与实际处理操作数。zero queue 的空计数则是在入口直接检查。

源码路径是：

```text
get_field
  → constant_property_key
    → Runtime::intern_property_key_js_string
      → AtomTable::intern_js_string
        → 整数键判断、字符串哈希、相等性比较、Atom retain
  → get_property_with_key
    → internal_get → get_own_property
  → 临时 ObjectRef 释放 → 引用/队列清理
```

具体入口：[constant_property_key](https://github.com/Eric-Song-Nop/quickjs-oxide/blob/9ed39e275d0f1b74ce82ed694ebaef797d6b79d7/src/engine/vm/host_bridge.rs#L1547)、[get_field](https://github.com/Eric-Song-Nop/quickjs-oxide/blob/9ed39e275d0f1b74ce82ed694ebaef797d6b79d7/src/engine/vm/host_bridge.rs#L3762)、[AtomTable::intern_js_string](https://github.com/Eric-Song-Nop/quickjs-oxide/blob/9ed39e275d0f1b74ce82ed694ebaef797d6b79d7/src/engine/atom/mod.rs#L445)、[release_object_handle](https://github.com/Eric-Song-Nop/quickjs-oxide/blob/9ed39e275d0f1b74ce82ed694ebaef797d6b79d7/src/engine/heap/ownership.rs#L82)、[drain_deferred_references](https://github.com/Eric-Song-Nop/quickjs-oxide/blob/9ed39e275d0f1b74ce82ed694ebaef797d6b79d7/src/engine/heap/ownership.rs#L15)。

### 平坦字符串使用了通用 UTF-16 迭代器

上述固定读取三次分别构造 **80,006,596 / 80,006,610 / 80,006,610** 个 UTF-16 迭代器；rope 输入均为 **0**。相等性比较的少量计数波动不改变这些结构性结论；随机化哈希表是可能原因，未据此断言某一次具体碰撞。

[Utf16Units](https://github.com/Eric-Song-Nop/quickjs-oxide/blob/9ed39e275d0f1b74ce82ed694ebaef797d6b79d7/src/engine/value/primitive.rs#L152) 包含 61 个 `Option<JsString>` 栈槽；[Utf16Units::new](https://github.com/Eric-Song-Nop/quickjs-oxide/blob/9ed39e275d0f1b74ce82ed694ebaef797d6b79d7/src/engine/value/primitive.rs#L650) 对平坦字符串也建立这个通用结构。[content_hash](https://github.com/Eric-Song-Nop/quickjs-oxide/blob/9ed39e275d0f1b74ce82ed694ebaef797d6b79d7/src/engine/value/primitive.rs#L808) 每次调用都计算内容哈希，[JsString::eq](https://github.com/Eric-Song-Nop/quickjs-oxide/blob/9ed39e275d0f1b74ce82ed694ebaef797d6b79d7/src/engine/value/primitive.rs#L1881) 对不同存储的等长字符串使用两个 UTF-16 迭代器。

固定属性读取中，`drop_in_place<Utf16Units>` 自身占 **7.97%**，libc 内存复制/移动实现自身约 **12.03%**。复制/清零样本的近端调用者集中在 Atom intern、字符串哈希和整数键解析。这里可以评估平坦 Latin1/UTF-16 字符串的直接处理路径，但必须保留跨表示相等性、孤立 surrogate 和 rope 语义。

### Richards 中的 `"length"` 提前处理

[array_own_key](https://github.com/Eric-Song-Nop/quickjs-oxide/blob/9ed39e275d0f1b74ce82ed694ebaef797d6b79d7/src/engine/object/properties.rs#L829) 先调用 `intern_property_key("length")`，随后才检查对象是否为 Array。十次固定 `runRichards()` 中：

- `array_own_key` 共 **1,032,270** 次；
- 其中 **1,013,477** 次最终返回“非数组”，占 **98.18%**；
- 三次计数运行完全一致。

这是比笼统“优化属性查找”更小、更明确的修改候选。它的收益仍需实际修改后关闭探针重测，不能把整个 `array_own_key` 的占比当作可全部消除的时间。

## 2. 空循环：公共执行路径中的具体成本

固定空循环使用原始 `empty_loop` 函数体，执行 10,000,000 次迭代并校验返回值。三次补充计数完全一致：

| 操作 | 次数 |
| --- | ---: |
| `update_numeric` | 10,000,000 |
| `VmHost::to_primitive` | 30,000,002 |
| 上述调用中输入为 Int/Float | 30,000,002 |
| `read_frame_binding` | 30,000,004 |
| 上述读取中 Direct binding | 30,000,004 |

[update_numeric](https://github.com/Eric-Song-Nop/quickjs-oxide/blob/9ed39e275d0f1b74ce82ed694ebaef797d6b79d7/src/engine/vm/numeric_execution.rs#L47) 在匹配 `Value::Int` 之前先调用通用 `host.to_primitive`。桥接位于 [VmHost::to_primitive](https://github.com/Eric-Song-Nop/quickjs-oxide/blob/9ed39e275d0f1b74ce82ed694ebaef797d6b79d7/src/engine/vm/host_bridge.rs#L2752)；直接局部变量读取位于 [read_frame_binding](https://github.com/Eric-Song-Nop/quickjs-oxide/blob/9ed39e275d0f1b74ce82ed694ebaef797d6b79d7/src/engine/vm/host_bridge.rs#L135)。

固定循环采样中，通用 `to_primitive` 桥接自身占 **11.16%**。这证明这批纯数值操作确实反复经过通用边界，但没有证明整个桥接成本都能移除。`Vec::push_mut` 自身也占约 10.92%；此处是值栈 push 的采样归属，不能据此声称每次 push 都分配内存。

## 3. Context：定位到内建初始化与布局事务

每次生命周期工作负载包含初始 Context 和 1,000 次独立循环，共 **1,001 个 Context**。按这个数量归一化，三个计数运行完全一致的项目为：

| 每个 Context 的工作 | 数量 |
| --- | ---: |
| `replace_layout` | 778 |
| 替换布局累计 entry / slot 数 | 6,419 / 6,419 |
| `get_or_create_shape` / cache hit | 943 / 158 |
| shape fingerprint 累计 entry 数 | 6,421 |
| `retain_edges_transactionally` | 1,899 |
| 其中输入边数为 0 / 1 / 2 / 更多 | 37 / 1,214 / 222 / 426 |

源码入口：[Runtime::new_context](https://github.com/Eric-Song-Nop/quickjs-oxide/blob/9ed39e275d0f1b74ce82ed694ebaef797d6b79d7/src/engine/realm/construction.rs#L15)、[RuntimeState::replace_layout](https://github.com/Eric-Song-Nop/quickjs-oxide/blob/9ed39e275d0f1b74ce82ed694ebaef797d6b79d7/src/engine/heap/runtime/mod.rs#L337)、[get_or_create_shape](https://github.com/Eric-Song-Nop/quickjs-oxide/blob/9ed39e275d0f1b74ce82ed694ebaef797d6b79d7/src/engine/heap/runtime/mod.rs#L243)、[Heap::replace_object_layout](https://github.com/Eric-Song-Nop/quickjs-oxide/blob/9ed39e275d0f1b74ce82ed694ebaef797d6b79d7/src/engine/heap/object_storage.rs#L880)、[retain_edges_transactionally](https://github.com/Eric-Song-Nop/quickjs-oxide/blob/9ed39e275d0f1b74ce82ed694ebaef797d6b79d7/src/engine/heap/gc.rs#L975)。

`retain_edges_transactionally` 每次构造临时 HashMap 聚合边，即使输入只有一条边；该预检承担溢出和事务正确性，优化时要保留这些保证。布局替换又包含新边保留、旧边释放、shape 缓存维护等工作，因此不应把初始化成本简单归为“分配对象”。

仅打开粗粒度阶段计时后，每个 Context 的平均阶段时间取三轮中位数：

| 初始化阶段 | 每 Context 毫秒 |
| --- | ---: |
| TypedArray intrinsics | 0.382 |
| Date intrinsic | 0.292 |
| Array intrinsics | 0.220 |
| RegExp intrinsic | 0.123 |
| Error intrinsics | 0.114 |

这些阶段是新的定位线索，不能直接与 QuickJS 的 C `clock()` 最小值计算倍数。完整阶段与详细布局计时在 JSON/原始包中。

## 4. 编译和执行已分开计时

在原有 `Context::eval_compiling_with_options` 边界分别包围编译和执行，保持原有控制流及异常处理。下表为临时诊断构建的三次中位数，单位毫秒：

| 工作负载 | Context 创建累计 | 编译 | 执行 |
| --- | ---: | ---: | ---: |
| 固定空循环 | 2.831 | 0.189 | 1751.908 |
| 固定属性读取 | 2.874 | 0.218 | 11111.974 |
| 固定 Richards ×10 | 2.843 | 12.851 | 2385.996 |
| 原始 Richards harness | 2.842 | 13.426 | 9675.615 |

**Richards 的主要成本在执行，不在十几毫秒的编译或几毫秒的 Context 创建。** 执行计时包含脚本中的 warmup、harness 和输出，不是假称已经分离的纯 steady-state 时间；固定工作量版本用于减少自校准框架的干扰。编译计时包含解析、编译及发布，不是单独的 parser 时间。

## 5. 与 QuickJS 的固定工作量对照

这一轮额外比较相同固定函数体的完整进程墙钟时间，绕开微基准的毫秒自校准。使用未强制 frame pointer 的 Oxide 优化构建（带调试信息，profiling feature 已编译但诊断关闭）与原生 QuickJS。三次中位数：

| 工作负载 | Oxide 毫秒 | QuickJS 毫秒 | 完整进程耗时比 |
| --- | ---: | ---: | ---: |
| 空循环 1,000 万次 | 1893.948 | 42.181 | 44.9× |
| 属性读取 2,000 万次 | 10999.540 | 122.887 | 89.5× |

这是固定工作量、包括启动/编译/收尾的进程耗时比，不能冒充单次操作精确延迟，也不是之前自校准 microbench 指标的同一次实验。QuickJS 的短运行更容易受到启动成本影响；原始范围与全部样本已保留。

对原始自校准 `empty_loop` 整段程序采样时，`get_field` 包含子调用占 **35.41%**，但空循环函数体本身没有属性读取。因此直接采样整个微基准程序会把框架的属性读取和计时工作混进来。这就是本轮增加固定函数体工作负载的原因，不能把原始 harness 的热点归给空循环本身。

## 测量方法、扰动与质量

- 机器：AMD Ryzen 7 7840HS，16 logical CPUs，Linux 6.18.44-1-lts；Rust 1.94.1、perf 7.2.3-1。复用了外部 benchmark 提交 `2034d98fc8c5f8044e186267593f5d5ea5232caf` 和 QuickJS 2026-06-04。
- CPU 构建：release 优化，`CARGO_PROFILE_RELEASE_DEBUG=1`、`CARGO_PROFILE_RELEASE_STRIP=none`；最终栈采样构建另加 `RUSTFLAGS='-C force-frame-pointers=yes'`。采样与计数使用不同二进制，完整哈希见 JSON/原始包。 初始两个构建的 CLI commit 字段未嵌入，源码绑定使用采样前记录的 Git 提交及外部构建 receipt/ELF SHA-256。
- 每个工作负载三次独立采样，`perf record -e cycles:u -F 199 --call-graph fp`；普通运行与采样运行交替。工作负载串行执行，计时期间不并行构建或解析 perf 报告；未声称系统完全隔离或固定 CPU 频率。
- 百分比按每条样本的 cycle period 加权。自身占比只计叶函数；包含子调用占比在同一样本中对同一符号去重，避免递归重复累加。它是采样估计，不是每个函数的精确计时。
- 初始 DWARF 栈出现无效返回地址；增加到 64 KiB 的补充尝试仍未可靠恢复 Richards 外层栈。这两批调用关系没有用于本文热点占比。原始记录保留，并明确标注。
- 使用匹配 build ID `503200d7fda94a5dc6058d7e0694e5d1dcb2e372` 的 libc 调试符号，解析复制/移动和清零函数；通过用户目录 `symfs` 读取，没有修改系统符号或内核设置。

| 最终工作负载 | 三次样本数合计 | 未知叶函数的 cycle 权重 | 达到 127 帧上限的 cycle 权重 |
| --- | ---: | ---: | ---: |
| context | 1,610 | 0.0015% | 0.00% |
| empty_fixed | 1,276 | 0.0027% | 0.00% |
| prop_fixed | 6,253 | 0.0001% | 0.00% |
| richards | 5,487 | 0.0371% | 40.99% |

上述最终报告的 lost samples 均为 0。**Richards 有约 41% 的周期权重达到 127 帧上限**；71.33% 的权重仍包含 `qjs::main`。近端函数/调用者可用于本报告中的热点定位，外层累计调用关系不完整。火焰图保留真实采集结果，不补造缺失父帧。

采样运行相对同一个 FP 构建的普通运行，中位完整进程耗时比：Context 1.027×、固定空循环 1.041×、固定属性读取 0.997×、Richards 1.004×。小于 1 的观测不代表采样能加速程序。

临时探针模式 0=关闭、1=计数、2=计数及详细计时、3=粗阶段计时。V1 中计数模式相对同一诊断构建关闭探针的中位耗时比：Context 1.008×、固定属性读取 1.002×、固定 Richards 1.015×。V2 新增数值/数组分类计数后，空循环为 1.036×，固定 Richards 为 1.020×。

**插桩即使关闭也会改变代码生成，不能将诊断构建与原构建之间的速度差视为优化收益。** CPU 占比来自未加探针的构建；计数用于解释工作量；细粒度计时只作辅助。详细计时包含嵌套范围，不能相加；所有原始模式样本均保留。

## 火焰图与后续修改边界

[Context](cpu-hotspots/context.svg) · [固定属性读取](cpu-hotspots/prop_fixed.svg) · [固定空循环](cpu-hotspots/empty_fixed.svg) · [Richards（注意深度上限）](cpu-hotspots/richards.svg)

SVG 在浏览器单独打开后可点击放大、搜索函数；显示名称缩短，悬停保留完整符号。宽度表示采样 cycle 权重，横轴不表示时间顺序。使用 [FlameGraph](https://github.com/brendangregg/FlameGraph/tree/41fee1f99f9276008b7cd112fca19dc3ea84ac32) 生成。

本轮完成定位，尚未做第 4 步的算法修改与收益验证。建议从属性键处理开始，每次只改一个机制，再关闭所有探针重复固定工作量和原始 Richards，并验证属性/原型/Proxy、跨 realm Atom 所有权、数值转换及 GC 行为。原先 arena 体积的线索尚未被证明是主要 CPU 瓶颈；四个超时 V8 子测试也未在本轮采样，不能将 Richards 的结论直接推广到它们。

## 原始证据

完整包包含 `.data`、解析后的调用栈与报告、所有 stdout/stderr、构建日志、准确匹配的 Oxide ELF、libc 调试符号、两版临时诊断补丁、实验脚本和逐文件 SHA-256 清单。外部 benchmark 源码及生成 JS 不在包中。

文件：`quickjs-oxide-cpu-investigation-pocketlab-2026-09-09.tar.gz`（87,671,572 字节）。

SHA-256：`93ef6154096799dfeb8bd88159b73fd4e90fd3b9333d4d091ce411ff1d8e638a`。

本地与远端项目的 `target/` 均保留该包；远端原始目录是 `/home/eric/Documents/Sources/PocketLab/quickjs-oxide/target/cpu-investigation/`。
