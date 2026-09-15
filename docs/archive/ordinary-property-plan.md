# 普通对象属性访问内核改造计划

> 历史归档（2026-09-12）：下文状态、任务和契约属于文中记录的旧 PR / 源码基线，不作为当前实施指令。当前设计见[架构说明](../architecture.md)和[栈 VM 计划](../primitive-vm-plan.md)；本次归档没有更新性能或验证结果。

状态：P0–P5 已实现并完成验收。源码核对基线 `fec7519`（2026-09-11）。
面向实施者和评审者：按本文迁移普通属性读、写、定义及元数据查询，并验证语义、所有权和实际性能。
来源为 benchmark issue #16 的对象属性问题；历史热点仅用于选方向，不能作为当前收益预测。

## 1. 目标与范围

把普通属性操作组织成三层：存储定位与事务、普通属性算法、可观察的内部方法分派。
现有文件划分不是约束，可以移动、合并和删除旧实现；对象/heap/VM 的具体接口也可以调整。
需要保留的是语言行为、公开 API 契约、运行时域隔离、引用所有权及安全 Rust，而不是旧调用链。

本轮交付包括：

- 自有普通数据属性写入不预扫原型链，不为旧值生成完整描述符，不重复走完整 Define 算法。
- 普通 Get/Get-or-missing 按需取得数据值或 getter；Has、枚举筛选共享元数据定位。
- value-only Define 复用已有槽事务；完整 Define 和描述符查询有各自明确入口。
- 普通原型链按需逐层访问，命中即停；移除“先证明全链普通，再重新执行”的路线。
- 迁移后删除被替代的普通读写实现，避免永久保留两套 Set 规则。

首批直接优化对象为明确的 `ObjectPayload::Ordinary`，物理存储同时覆盖 shared shape 和 dictionary。
首批槽覆盖 Data/Accessor；VarRef/AutoInit 作为显式特殊状态处理，不能误当普通数据槽。
其他 payload 即使有普通命名属性，也必须逐类证明后才能复用；不以“不是 Proxy”推导普通。
Array、TypedArray、Arguments、String wrapper、模块命名空间和全局对象的语义入口保留并参与回归。
本轮不包含数值运算、跨操作 inline cache、shape 布局重写、批量冻结/删除或对象分配池。

## 2. 当前代码证据

这是本次决策的源码索引，实施时以符号及调用者为准：

| 现有位置 | 事实 | 需要改变的工作 |
| --- | --- | --- |
| `object/internal_methods.rs::ordinary_set_fast_path_available` | 检查 receiver 后遍历完整 prototype 链，排除 Proxy/TypedArray | 自有属性命中也付出全链分类成本；改为访问到哪层才分类哪层 |
| `internal_set` 与 `properties.rs::prepare_set_property_with_receiver_in_realm` | 同时存在完整通用 Set 路线和预扫后的普通路线 | 收敛普通语义，不新增第三套永久路线 |
| `prepare_set_property_with_receiver_in_realm` | 查询 target，然后再次查询 receiver，再调用 Define | receiver==target 且已有 Data 时复用本次定位 |
| `properties.rs::get_own_property` | shape/slot 快照后生成完整 rooted descriptor；accessor 同时 root get/set | 根据操作只提取需要的值、可调用对象或 flags |
| `internal_get` / `internal_get_or_missing` | 经完整 GetOwnProperty 获取读结果，多层重复对象分类 | 共用普通查找步骤，保留 missing 与 undefined 区分 |
| `define_ordinary_own_property`、`operations.rs` | current descriptor 和验证记录之间转换并 clone Value | value-only 定义避免无关字段往返；其余定义保留单一验证算法 |
| `storage.rs::store_property_slot` | 已经可以同 flags 单槽替换，但再次按 key 查槽 | 让合法的本次定位在事务内部直接用于替换 |
| `internal_has_own_property` / 两种 enumerable 查询 | 已有部分 metadata 路线；普通 recheck 和枚举快照的 AutoInit 行为不同 | 复用定位，但不可合并可观察语义 |
| `heap/object_storage.rs::replace_object_slot` | 验证新槽、保留新边、替换、释放旧边、drain，返回 cleanup | 作为事务基础，不绕过引用预检 |

完整描述符是 Rust record，不能把它的每次构造都说成一次堆分配。实际成本包括查找、RawValue/Value
转换及 clone、对象/Atom retain-release、Runtime 借用和重复分类。具体占比需要当前基线采样。
`prepare_get_property_with_receiver_or_missing` 当前是测试专用入口，不应把它当生产 Get 的热点。

## 3. 建议的模块和接口

以下是目标组织，可因 Rust 借用实现细化；不要先大规模搬文件再寻找使用者。

### 3.1 存储层：一次定位、按需提取、原子替换

建议增加 `object/ordinary_storage.rs`，配合 `heap/object_storage.rs` 和 RuntimeState 的 Atom/cleanup 管理。
该模块不接收 realm，不调用 VM、Proxy trap、getter、setter 或用户转换。

私有定位结果表达：Missing，或 slot index + flags + slot kind。位置不是新的持久句柄。
通过持有 `&mut RuntimeState` 的短生命周期具体 session，或单个封闭方法，完成定位和消费。
优先采用具体类型和方法，不引入操作模式布尔集合、策略 trait 或通用闭包执行框架。

建议的操作职责（名称为设计草案，不是已存在 API）：

- `lookup_own(atom)`：在本次 session 内定位，返回内部视图；不生成 rooted descriptor。
- `read_data` / `root_getter` / `root_setter`：只保留将跨出借用范围的那个值。
- `replace_data`：消费本次定位，在同一独占访问中更新同 flags Data 槽。
- `materialize_descriptor`：完整描述符查询才读取并 root 全部所需字段。

slot index 和视图不得导出到公开/VM API，不实现可随意复制的“已验证 slot token”。
不能只靠 shape id 判断跨操作位置有效：dictionary 可以原地修改，swap-remove 可以移动槽。
定位后到提交前禁止布局变更；提交后视图立即失效，不能在 cleanup 后继续使用。

**root 生成必须解决借用问题。** 当前 `Runtime::root_raw_value` 会再次借用 Runtime；不能从
持有 RuntimeState 可变借用的 session 内直接调用它。增加或复用 state 内的 retain/promotion
原语，在同一状态访问中保留必要边，释放借用后用已有 owned-handle 构造器组装 Value。
跨层转移的是明确已拥有的引用，组装失败必须有回滚；不把裸 RawValue 误当 root。
另一种实现可以在禁止回调和修改的短读区间快照，然后立即 root，但必须证明旧边在 root 前仍存活，
且不得因此让写事务退化为导出裸 slot。P1 中用实际编译与所有权测试选择实现，不改变此契约。

**写事务顺序：**先验证 runtime 域及输入，准备 RawValue，保留新 Atom/heap edges，再发布新槽，
随后释放旧边并处理 cleanup。保留现有 retain 失败不修改对象的保证。
提交后的 cleanup/invariant 错误不冒充“完全未发生写入”；要明确区分发布前和发布后错误。
不得在持有 state 借用时随意 drop rooted ObjectRef/Value，避免隐式重入借用或改变延迟释放协议。
旧值与新值相同、重复边、符号、对象自引用和最终引用释放均需测试。

### 3.2 普通算法层：Set/Get/Define 各自明确，共享存储机制

建议 `object/ordinary.rs` 拥有普通算法，在代码量确有需要时再拆 read/set/define 子模块。
结果用具体枚举表达已完成、拒绝、需要调用、继续原型查找和进入特殊语义；不要把异常当普通 miss。

**Set：**

1. 查询当前对象自己的属性；命中 ordinary Data 且 receiver==target，直接检查 writable 并更新。
2. 自有只读数据属性直接拒绝；即便新旧 SameValue 也不能接受普通赋值。
3. 命中 accessor，只 root setter 和所需输入，结束借用后调用；receiver 保持原始值。
4. 缺失才进入 prototype；全链缺失相当于默认 writable Data 的 Set 行为，转到 receiver 自有属性处理。
5. receiver 不同，不能把 target 的槽复用于 receiver；receiver 为 Proxy 时必须保留其 GetOwnProperty/Define trap。
6. 新建属性、特殊 receiver、特殊槽进入所属语义，不从叶级路径重放已经执行的回调。

**Get / Get-or-missing：**普通节点共用一个查询步骤；数据只返回 value，accessor 只返回 getter。
缺失才取原型，优先用迭代推进普通节点；遇到 Proxy 等特殊节点转交可观察入口。
Proxy Get 是 terminal observable boundary，不能把其 undefined 再解释成 missing；保留全局引用错误行为。
若调整递归形态，核对现有 Proxy 深度及栈溢出契约，不能意外绕过其预算。

**Define：**只针对“仅 value 字段存在、普通对象、已有 Data”使用明确捷径。
可写属性接受；不可写但 configurable 的属性也可更新；不可配置且不可写的属性只接受 SameValue。
NaN、+0/-0、对象身份按 SameValue 而非 `==`。该判定放在现有纯描述符验证所属模块，
共享已有规则，并用属性组合测试对照完整验证算法；不能在多个调用者复制条件。
空 descriptor、属性变更、Data/Accessor 转换、缺失属性和特殊槽走完整算法。
完整验证可以继续输出完整记录；首轮不为消除所有转换而改写整个 descriptor 类型系统。

**Has / enumerable / descriptor：**共用存储定位，语义入口分别命名。
HasOwn 和 own-key snapshot 可以按现有约定不物化 AutoInit；普通 enumerable recheck / descriptor 查询
仍按现有约定物化；模块 VarRef 的 TDZ 及 Proxy trap 不可被纯 flags 查询绕过。

### 3.3 分派层：对象类别、可观察调用与错误映射

`internal_methods` 保留或拆出 Proxy/internal-property 分派文件，负责特殊对象、realm、receiver、
JavaScript 调用、Throw 和 API 层拒绝映射。不再拥有第二份普通 Set 算法。
允许移动现有 Array/Arguments 定义代码到所属模块，但只随对应迁移进行，不扩大成目录重排项目。
分类必须针对操作及 key：TypedArray 的整数索引、Array length、String 虚拟索引不能用一枚全局
“ordinary=true”覆盖。P1 采用 Ordinary payload 白名单，使首批适用条件可审查。
PropertyKey 转换和 ToPropertyDescriptor 在外部语义入口完成，内核不能再执行一次转换。

## 4. 迁移切片与完成条件

所有步骤当前均为未开始。每步提交后单独测量；先跑目标语义，再扩大回归。

| 步骤 | 交付与依赖 | 完成条件 |
| --- | --- | --- |
| P0 当前基线与行为矩阵 | 捕获生产 Get/Set/Define/Has 调用者，冻结测试负载和基线 ELF；收集属性相关 profile | 能区分链遍历、descriptor/root 和存储成本；无改动收益声明 |
| P1 自有数据写入贯穿三层 | 建立最小存储 session、共享引用事务、ordinary Set 入口；先在旧预扫之前命中 Ordinary Data 自有写入 | 无原型预扫、无旧值 descriptor、单次定位；shared/dictionary 和引用失败验证通过 |
| P2 普通链与 accessor Set 收敛 | 扩展同一算法处理缺失、继承、异 receiver 和 accessor；特殊对象仍分派 | 删除 `ordinary_set_fast_path_available` 和替代掉的普通 Set 分支；保留一个普通算法和特殊对象算法 |
| P3 Get 与元数据消费者 | Get/Get-or-missing、Has、enumerable、完整 descriptor 接入共同定位与按需提取 | Getter 只 root getter；missing/undefined、AutoInit、Proxy/TDZ 语义矩阵通过 |
| P4 value-only Define | 依赖 P1/P3；纯规则判定接入已定位槽更新 | flags 全组合及 SameValue 对照通过，不把只读 Set 与 Define 混同 |
| P5 收口 | 删除迁移桥接/死实现，更新源码契约和架构规则；完整回归与最终性能矩阵 | 无第二套普通规则、无裸 slot 跨回调、性能结论包含退化与控制组 |

P1 允许短暂保留旧路线作为迁移 fallback；P2/P5 必须清理。测试专用入口若仍需要，应委托同一算法，
不能保留只供测试通过的旧内核。现有架构 hash/canary 可随职责变化调整，但必须更新能命中新入口的反例，
不能把旧结构断言当作禁止重构的理由，也不能仅改 hash 放行。

删除、新建属性、批量 define/freeze 和原型变更参与控制测试，本轮不承诺对它们加速。
若 P0 发现某个相邻路径是主要热点，可写明新证据再扩展切片，不能隐式增加实施范围。

## 5. 必须覆盖的行为与所有权

- 普通 Data：自有/继承/缺失，writable/configurable/enumerable 组合，非 extensible，shared/dictionary，字符串及 Symbol key。
- Set：严格赋值、非严格赋值、Reflect.set 返回值，异 receiver、primitive receiver、receiver 自有 accessor；错误类型与已冻结诊断保持一致。
- 原型链：自有命中下方为深链或 revoked Proxy 时不访问下方；缺失遇 Proxy 时 trap 次数、顺序、receiver 正确。
- Accessor：getter/setter 修改目标布局、删除/重建属性、递归访问、抛错；Get 不触及 setter，Set 不调用 getter。
- Define：value 字段缺失与显式 undefined，空 descriptor，混合 descriptor 拒绝，NaN、±0、相同/不同对象；所有 flags 组合对照完整验证。
- 特殊语义：AutoInit 物化/拒绝顺序、模块 TDZ/global VarRef、mapped Arguments 解除映射、Array length/索引、String 虚拟索引、TypedArray detached/resizable 和转换重入。
- 生命周期：跨 Runtime object/key/value/descriptor 拒绝；重复引用、引用溢出/保留失败、发布前回滚、发布后 cleanup；drop/GC 无泄漏、双释放或借用 panic。
- 组合操作：`obj.x++`、`obj.x += value` 的转换期间修改布局，确认 Get 与 Set 不共享失效定位。
- 架构反例：绕过域检查、retain 前发布、错误复用 receiver 槽、把特殊对象纳入白名单、跳过 AutoInit、让 slot 逃离 session、重放 trap。

纯判定测试归 `object/property`，新内核契约测试归新模块；低级引用事务测试复用 heap/runtime 测试设施。
JS 可观察行为用现有 CLI/QuickJS differential 和 Test262；新用例进入现有 registry，不改冻结行为向量。

## 6. 测量与回归流程

固定工作量，不采用 Date.now 最小窗口成绩衡量小幅收益。每个优化提交与直接前序做普通 release
交错 A/B；机器、CPU、工具链、flags、工作负载哈希和输出固定，构建/测试与 timing 串行。
首轮至少五轮；疑似退化追加十轮，并报告分布、instructions/cycles 和整进程时间。

新增诊断矩阵：

- 同一自有属性固定写入次数，原型深度 0/1/8/64/256；另有同样 setup、无写循环控制，单独报告不伪装成纯阶段耗时。
- 对象宽度 4/32/256/2048，同时测试共享 shape 与删除触发的 dictionary；控制变量固定总操作数。
- 属性值分别为 Int、Float、String、Symbol、对象；区分重复同一值和两个预建值轮换，不把分配成本混入所有场景。
- Get、自有 Set、继承 Data Set、accessor、异 receiver、value-only Define、HasOwn 和枚举查询分别测量。
- Proxy、特殊对象、缺失属性为语义与性能控制组；首次写入和 steady repeated writes 分开配置。

诊断 instrumentation 只证明工作数量，不用于正式 timing；如能稳定计数，验证自有 Set 的 prototype visits=0、
旧值 descriptor materializations=0，且定位次数不随链深度增长。不要把源码函数调用次数等同于机器指令成本。
正式性能判断还包括 `prop_read/write/update/create/delete`、array/arguments/global 控制和完整 50+8 固定矩阵。
无预设加速倍数；新增复杂度若没有可复现耗时收益，应调整或撤回，不能仅以指令数下降验收。

回归执行入口以当前 CI 和测试文档为准，包括：

```sh
cargo fmt --all -- --check
cargo test --locked --workspace --all-targets
python3 -m unittest discover -s scripts/benchmark -p 'test_*.py'
./scripts/checks/check-binary-object-boundary.sh
PYTHONPATH=scripts/checks python3 -m unittest discover -s scripts/checks/binary_object/tests
python3 scripts/checks/check-source-layout.py
```

最终还须执行 CI 固定 Rust 的全部 Clippy/feature 组合、profiling/test262-host/doc、QuickJS differential、
focused/full Test262、架构变异、Node/WASM 验收。使用当前 CI 的确切命令，不把上述短清单当完整门禁。
Test262 fingerprint 变化与实际行为变化分别报告，不能改冻结 receipts 让新结果通过。

## 7. 交付记录

每个切片记录源码提交、修改拥有者、已运行验证、未运行验证、基线/新 ELF 及工作负载哈希、A/B 分布、
退化和最终处置。原始实验保留在忽略的 target；计划和维护契约入库，不预填结果。
涉及借用/引用事务及 Set 语义收敛的变更需要独立评审，实施者自查不得称为独立评审。
实现记录：P1 `88a39d1`，P2 `5ad6436`，P3 `68ce00f`，P4 `eaea103`。P5 `bc16d97`，特殊分类/初始借用收口 `41ba53a`，清理与验证边界收口 `9c91663`。
存储实现采用私有 `OwnSlot` 与短借用 probe，不导出 session/slot；白名单同时检查 kind 和 payload。
引用事务由 RuntimeState 统一管理，发布前失败回滚新引用，发布后清理失败不撤销已发布引用。


### P5 首次验收（9c91663）

- 自有 Data Set：同一次借用中定位、检查 writable、替换已有槽；不预扫原型、不构造旧值 descriptor、不往返 Define。
- Get/Get-or-missing：共同定位后只提取 value 或 getter；完整 descriptor 只在其消费者需要时物化。
- value-only Define：共享槽事务和纯权限判定，保留 SameValue、字段缺失、flags 和跨 Runtime 规则。
- Has/HasOwn/enumerable：共享定位和 flags；snapshot 与实际 GetOwnProperty 对 AutoInit/VarRef 的不同要求保持。
- 普通原型链：逐层定位，命中即停；特殊分类在进入回调前复用，异 receiver 单独查找。Array/TypedArray 等特殊对象及普通 AutoInit/VarRef 仍保留相应回退，不把剩余成本称为已经全部消失。

P5 首次验收的存储提交为 `9c91663`。独立评审覆盖引用事务、Set 收敛、特殊分类与共享 operation 边界；最后一轮没有阻塞问题。
Rust 1.88.0 的 workspace 3068 项、实际 QuickJS differential 3072 项、全部 Clippy/feature/doc 组合和 Node/WASM 验收通过；各有原有 ignored 项的两组不计入通过数。
701 个原有架构反例被拒绝，最终源码扫描和新增 8 个内核反例通过。Test262 focused 6844 项通过，full 102037 项行为向量未变（79982 pass、原有 50 个可运行失败）。
严格 receipt gate 因源码 fingerprint 变化报告 checksum drift；仅规范化该字段后，TSV/JSONL 完整 SHA-256 与冻结基线相同，未修改冻结 receipts。

基线为 #18 `fec7519`（ELF 构建提交 `c1dfce3` 仅增加设计文档）；普通 release 使用相同 Rust 1.94.1 和构建 flags、CPU 2、Ryzen 7 7840HS。
已运行 50+8 固定矩阵和 38 项对象诊断，各五轮交错；超过 3% 的疑似退化追加十轮。各优化提交也与直接前序交错对照。
首次验收关键耗时变化：prop_read -11.4%，prop_write -67.0%，prop_update -56.0%，value-only Define -25.6%，enumerable -15.1%。
自有写入原型深度 0–256 的新耗时约 66–68 ms，基线约 149–1221 ms（包含 setup；另有 setup-only 控制）。八项固定真实程序耗时几何均值 -10.8%。
首次验收发现的退化：array_read +6.5%，typed_array_read +7.4%，typed_array_write +2.8%（十轮）。这些退化已在后续修复中消除，见下节。

普通 ELF、构建 receipts、工作负载哈希、完整分布、instructions/cycles 和匹配 ELF 的叶采样保留在忽略的 `target/property-delivery-*` / `target/property-final3-*`；当轮本地数据保留用于前后对照。
首次 pilot、与架构检查重叠的早期 P3 计时，以及 Node 彩色参考输出导致 admission 失败的诊断轮次均不用于结论。差分测试曾遇到并行 feature 构建覆盖 CLI 的测试竞争，已在构建结束后串行重跑通过。
Rust 1.94.1 的两项 debug native-stack-budget 失败已在未改动 #18 上独立复现；本次完整验收使用仓库固定的 Rust 1.88.0。

诊断生成器可复现实际测量的 38 个程序字节、参数及无颜色 Node 参考输出：

```sh
python3 scripts/benchmark/ordinary_workloads.py --output target/ordinary-diagnostics
python3 scripts/benchmark/fixed.py --manifest target/ordinary-diagnostics/manifest.json \
  --engine before=target/property-baseline/release/qjs \
  --engine after=target/property-regfix3/release/qjs --repeat 5 --cpu 2 \
  --output target/ordinary-replay
```


### Array / TypedArray 退化修复（1cc51bb）

Array 的直接编码稠密索引和真正 Array 的已有自有槽共享 value/getter 选择，不构造完整 descriptor；Array 未命中、需要回退的非直接编码稠密索引、AutoInit/VarRef 仍走原算法。读取扩展不放宽普通 Set/Define 白名单。命名属性不预先解析数组索引，避免 `length` 读取额外开销。
TypedArray 的边界计算和字节访问复用同一次访问中的 owned buffer token，边界计算抽成纯函数供原有状态查询共享。写入仍在可能执行用户代码的值转换之后重新获取 token，不跨回调缓存状态；所有元素类型共用该机制。

最终 engine 为 `1cc51bb`，仍与 #18 的普通 release 在相同 Rust 1.94.1、flags 和 CPU 2 上比较。重点七项同时与旧 #19 `9c91663` 做十轮轮换顺序三方对照；完整 58 项矩阵及 38 项对象诊断另各做五轮交错，所有样本输出校验通过，计时没有与编译/测试重叠。

| 操作 | 相对 #18 耗时变化（十轮中位数） |
| --- | ---: |
| 数组元素读取 | -18.2% |
| 数组 length 读取 | -28.8% |
| TypedArray 元素读取 | -12.9% |
| TypedArray 元素写入 | -18.9% |
| 普通属性读取 | -13.5% |
| 普通属性写入 | -65.4% |
| 普通属性更新 | -54.7% |

完整 50 项微基准耗时几何均值 -8.6%，8 项真实程序 -12.7% 且全部改善。最终 58+38 项没有超过 3% 的退化；不把这一阈值表述为每个负载都更快。五轮 instructions/cycles 对照也确认四项 Array/TypedArray 重点负载的指令数和 cycles 均减少。

最终源码的 Rust 1.88.0 workspace/all-targets + test262-host + pinned QuickJS 对照为 3074 passed、0 failed、1 原有 ignored；包含 14 项属性回归，覆盖稠密/慢 Array、空洞和原型 getter、receiver、命名/Symbol/大索引，以及转换期间 resize/detach 与共享 buffer。13 组 QuickJS fixture、19 组 C oracle、三种 Clippy 配置、15 个 Web/WASM 示例、格式/布局、架构扫描和 16 项架构测试通过。
全量 Test262 102037 个结果逐项未变（79982 pass、80032 runnable、同样 50 个 runnable failures）。仅规范化源码指纹 `e4f461c60faf9116313fcbcc6fc062d691f308ad4042e7f98d9a9154d21deea3` 后，完整 TSV/JSONL SHA-256 与冻结基线相同；严格 receipt gate 的源码指纹差异和语义结果差异分别处理，没有修改冻结 receipts。

最终普通 ELF、构建 receipts、完整样本/分布、stdout/stderr、计数器与验证记录保留在忽略的 `target/property-regfix3-*`；完整性能表见 [PR #19](https://github.com/pocket-stack/quickjs-oxide/pull/19)。仅内联的尝试已撤回，中间候选的命名属性退化在最终版本中修复；中间版本不混入最终性能表。
