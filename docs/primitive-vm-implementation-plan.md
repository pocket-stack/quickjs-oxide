# 栈 VM 实施设计：模块、数据、算法与维护契约

状态：2026-09-14，S01–S07 阶段验收通过，S08–S10 的优化及默认切换尚未实施。执行表示已确定为栈 VM；本轮使用线性栈 IR、轻量控制流分析与有限融合。目标见[架构计划](primitive-vm-plan.md)，实施顺序合并为[10 个 commit](primitive-vm-commit-plan.md)，完整能力见[迁移清单](primitive-vm-migration.md)。

S07 的完整性能测量已完成，回退及源码/机器码证据见[分析报告](performance/README.md)。S08/S09 的详细工作与退出条件以[更新后的第 4 节](primitive-vm-commit-plan.md#4-优化与最终交付)为准：执行/状态推进与融合先收口，调用存储、编译和布局后收口；本文的所有权与语义契约继续适用。

本文让实现者能够从一个指令或语义问题定位到唯一状态所有者，写出完整的调用/错误/恢复流程，并给审查者提供检查不变量。下面的类型与目录是目标设计，不是现有生产 API。

## 1. 模块划分与允许的依赖

```text
src/engine/
  compiler/
    mod.rs                  编译入口，组织各阶段
    parser/                 完整语法；context 管解析期状态，builder 管 IR 发射
    model/                  共享数据；ir、bindings、scope 分别拥有自己的类型
    resolution/             名字、hoist、capture、eval 与绑定布局
    lowering.rs             resolved IR → 逻辑栈指令及控制区域
    flow.rs                 块索引、正常/异常栈状态、初始化事实
    optimize.rs             有界局部融合；统一重定位请求
  code/
    instruction.rs          Instruction、栈效果、潜在效果和操作数契约
    function/               CodeDraft/CodeImage、FrameLayout、binding 与发布输入
    verify/                 指令/栈、参数/绑定、函数树、模块/私有状态的验证
    publish/                名称链接、heap 发布及失败回滚；消费验证结果
    executable.rs           一份运行时绑定的不可变执行描述
    debug.rs                源码位置与融合阶段映射
    encoding.rs             内部布局；仅在编码实验进入时拆出
    binary_object/          外来格式解码/翻译，保留其独立边界
  vm/
    mod.rs                  启动、恢复、状态观察的内部入口
    protocol.rs             小型数据协议：完成、恢复、调用与边界
    execution.rs            RunningExecution、帧/槽/操作容器和资源限额
    frame.rs                Frame、FrameLayout 实例化、身份/返回目标
    stack.rs                栈窗口、局部/实参槽、move/copy/clear
    bindings.rs             TDZ、cell、arguments/private 与动态绑定适配
    run.rs                  唯一热分派循环；局部 pc/sp
    driver.rs               运行帧、推进操作、调用宿主的唯一调度入口
    call.rs                 callable 解析、参数窗口、建帧和返回
    operation.rs            操作登记、结果交付及类型分派，无领域算法
    unwind.rs               completion、handler/cleanup、finally 展开
    suspend.rs              运行/挂起状态转换、所有权交接
    observe.rs              PC、backtrace、debug、interrupt 与重入快照
    native_stack.rs         真实宿主栈保护
  value/
    number/                 operations、integer、format、float16；纯 Number 规则
    conversion/             ToPrimitive/ToNumeric 等状态与 step
  object/                   属性、Proxy 等算法及可回调阶段
  builtins/                 内置算法及各自的可回调阶段
  heap/                     原始记录、引用事务、roots、循环回收
  api/compile.rs            编译请求编排；调用 compiler 后交 code 验证/发布
  api/、host/、jobs/          其余现有入口、宿主服务与规范 jobs
```

`parser` 和 `resolution` 内按既有完整语言职责拆分，避免再出现一个承载类型、语法和所有后处理的 mod.rs。`code` 保留外部二进制所需的细分验证；表中名字表示目标责任，不要求把所有验证重新合并成一个文件。

| 调用方 | 允许依赖的能力 | 不允许形成的依赖 |
| --- | --- | --- |
| compiler | source、纯值/数值、code 草稿和 metadata | 活动 Execution、宿主 provider、运行中 heap 对象 |
| code 的表示/验证 | 草稿、只读布局、显式发布输入 | compiler 的 Parser/编译上下文、执行中的帧 |
| 编译请求入口 | compiler、code 发布与既有 Runtime 服务 | 复制解析、验证或发布算法 |
| run | 已验证 code、当前 frame/window、纯 Number、受限槽引用事务、边界数据 | 直接运行 JS、分配新 frame、执行属性/内置算法 |
| driver/call | execution、code、操作分派、host 入口 | 自己实现 ToPrimitive/Proxy/Array 的规则 |
| 语义模块 | heap/object 等原责任能力，以及小型 step 数据协议 | 调用 run/driver，直接修改 frame Vec 或 pc |
| stack/frame | 槽生命周期、绑定布局与静态 code metadata | 解析源码、控制 Promise job 顺序 |
| observe/suspend | 已物化执行状态、明确的根交接接口 | 读取未发布窗口或保留跨回调的 slice |

protocol 是可以由语义模块使用的叶子数据契约，不含 Runtime 方法或 driver 实现。操作类型使用 Value 和身份等数据，不借此反向调用 VM。不新增泛化的“所有能力都可注入”的 trait；生产只需要一套栈执行器。

跨责任模块使用显式导入与窄可见性。`use super::*` 不再承担生产依赖协议，mod.rs 不转发整层内部方法。跨模块职责集中在架构说明，局部算法与所有权契约写在对应类型和函数旁；不要求为目录创建 README。

## 2. 编译管线与中间表示

```rust
struct FunctionIr {
    operations: Vec<SpannedIrOp>,
    bindings: BindingModel,
    children: Vec<FunctionId>,
    control: ControlMetadata,
}
struct StackDraft {
    instructions: Vec<StackInstruction>,
    labels: LabelTable,
    layout: FrameLayout,
    handlers: HandlerTable,
    sites: SourceSites,
}
```

保留线性操作和完整求值顺序。解析期未决 binding/reference 由名字解析统一解决；lowering 只消费已解析身份，不再次猜测名字。`StackDraft` 是发布前同一数据的阶段形态，不同时复制保留 AST、HIR、CFG、SSA 与成品码。

具体算法：

1. 完成 parser、作用域/声明验证、child-first capture/eval 解析，确定普通 local、cell、环境和 private 访问方式。
2. lowering 把 scope/私有 Reference、异常/恢复标记变成认证的栈操作和 metadata；不提前丢掉错误/源码位置。
3. 建立块边界：入口、目标标签、终结后继、handler 入口、恢复入口、需要独立错误状态的点。块可以只是指令区间与边表。
4. worklist 验证正常/异常/恢复的栈高度、私有状态和控制区域；TDZ 的初始化事实在合流取交集。
5. 在满足契约的块内应用有限短模式，记录标签、源码与 handler 的映射；不跨可进入标签或改变受保护区域。
6. 一次重定位并验证最终码，再绑定当前 Runtime 的 atoms/constants/realm 发布。

语义必需的验证不得因预算耗尽跳过。可选分析达到预算时使用未优化码或保守事实，丢弃依赖未收敛结果的改写；不能用不完整初始化或活跃信息删除检查/roots。

本轮采用局部分析方案。全函数值版本、phi、去 SSA、寄存器分配及全堆 MemorySSA 等方案按后续跨块优化需求评估，不作为本次调用和数值优化的前置工作。

## 3. 指令的唯一契约

`Instruction` 是一套逻辑栈 ISA，配套一个穷尽的描述入口。普通 Rust enum/match 足以起步，不先开发复杂生成器或 DSL。

```rust
struct InstructionInfo {
    stack: StackEffect,
    effects: PotentialEffects,
    control: ControlEffect,
    operands: OperandContract,
}
```

栈效果不只是一对 pop/push 数字：call、分支、异常入口、私有 Reference 和 resume 可以有依赖操作数或边类型的状态变换。verifier 统一使用这个模型，优化器检查改写前后的契约，disassembler 输出同一份指令含义。

`PotentialEffects` 描述通用语义可能 throw/call/allocate 等；数值快路通过标签、目标写入与拥有状态检查后，可以证明本次命中没有这些效果。不能因 Add 的通用描述可能回调，就每次数字相加都退出；也不能把通用 Add 当纯操作删除。

源位置用 `(指令位置, 语义阶段)` 关联固定 site。融合后的 read/convert/write 可能对应不同原位置，操作恢复时保存阶段。调试模式无法忠实单步某个融合时，编译器对该模式不融合；继续使用同一个执行器。

## 4. code、frame 与 stack 的状态所有权

```rust
struct Executable {
    image: Rc<CodeImage>,
    bindings: RuntimeCodeBindings,
    realm: ContextId,
}
struct Frame {
    executable: ExecutableRef,
    slots: FrameSlotRange,
    operand_base: SlotIndex,
    sp: StackDepth,
    fault_pc: Pc,
    resume_pc: Pc,
    return_to: ReturnTarget,
    control_base: ControlIndex,
    extra: Option<FrameExtraId>,
}
struct RunningExecution {
    frames: FrameStore,
    slots: SlotStore,
    operations: OperationStore,
    pending: PendingState,
    limits: ExecutionLimits,
}
```

这不是新增“不可变共享代码”的收益：现有 code/constants 已经 Rc 共享。目标是每帧只取得一份拥有正确生命周期的 Executable，不再重新投影多个 metadata 数组和额外 root。RuntimeCodeBindings 保存 Runtime-specific atoms/对象/realm；共享 CodeImage 不允许跨 Runtime 使用这些身份。

FrameId/OperationId 带所属执行身份与防复用信息；保存索引或 ID，不保存 Vec 元素地址。this、new.target、callee、环境和 constructor return 状态有明确槽/冷数据所有者，不能因缩小帧而遗漏错误 realm 或 root。

FrameLayout 区分：原始实参来源、可写形参、普通局部、cell/private 状态、操作数最大范围。TDZ/private 不是任意 JS Value；可采用位图和专用侧槽，但第一次实现允许集中管理的 typed binding，不能为消掉一个 enum 引入未验证的状态组合。

SlotStore 的操作限定为当前帧的 push/pop、peek、local read/write、move span、clear range；外部不直接操作 Vec。push 的存储在进入帧前按 max_stack 预留。连续 arena 容量可跨同步调用复用；首次增长及初始化发生在 driver 边界。

首版允许为父帧保留完整已验证栈窗口，为子帧分配独立区间。之后可以比较独占尾部参数区的转移，但不能让两个 frame 都拥有同一段可写值。记录预留槽数、实际活跃槽数、初始化写入和高水位，避免仅减少 Vec 数却扩大常驻内存。

## 5. 运行/挂起与 RC 的交接

运行中的 Value 是 owning handle，可能间接拥有 Runtime。长期 heap 状态必须保存受 heap 引用事务管理的原始边，不能永久保留 Runtime-owning wrapper。现有 [RawValue](../src/engine/heap/identity.rs) 的 Clone 不保活 Object 边，[挂起记录](../src/engine/heap/suspension_records.rs)也明确禁止这种 wrapper；本轮必须保持这条边界。

目标采用以下生命周期：

- RunningExecution 由一个作用域内的运行所有者持有；active registry 只登记身份与可观察视图，或者由严格作用域 guard 管理临时登记。不能形成永久的 Runtime→Execution<Value>→Runtime 拥有环。
- 进入宿主前执行状态仍有运行所有者，但所有 RefCell/mutable slice 借用已结束。重入建立独立入口身份和 delimiter，观察表能读到已发布的父帧。
- 挂起时，用统一转换器把有效 frames/slots/operation/pending 状态变为 heap 记录。先事务性保留目标的全部边，再解除运行 roots；失败时保留原所有者并清理未发布目标。
- 恢复时验证 code/frame/binding/region/resume 身份和形状，在原 heap owner 仍保活时建立运行 roots；全部成功后转移状态，避免半恢复或丢根。
- 完成、异常、关闭生成器、运行作用域退出均只有一个释放责任。挂起记录随可达 generator/async 对象存活，不能永久注册为全局 root。

同一个逻辑 FrameLayout/StackState 服务运行和挂起；拥有载荷不同不等于维护两套解释语义。转换只在挂起边界发生，不在普通同步调用上编码/解码整个 activation。是否进一步让存储直接成为 heap-managed record，需要独立内存证据；本轮不强制更换 GC。

对每个槽操作规定：move 转移已有引用，copy 增加必要引用，clear 放弃一次拥有；减少 sp 后被消费值必须移走或释放。release/drain 若可能触发 runtime 观察或回收，先将结果和 pending 根安装到有效所有者，再结束窗口处理。纯 Number 输入不保证旧目标值的析构也无成本。

## 6. 主循环与热/冷分工

```rust
fn run_stack(window: &mut StackWindow, budget: &mut Budget) -> RunExit;
// RunExit 只描述边界；大 payload 已由执行状态中的 pending 拥有。
```

主循环直接 match 已验证指令。字面量、简单栈移动、直接局部访问、确切 Number 运算和静态分支在这里完成，不再先分 numeric/hot 再重复匹配全部 opcode。

Number/Immediate 的读取与复制不进入 Runtime。对 owning heap 值的 copy/replace，通过同一 SlotStore 明确的生命周期操作完成。可证明不会分配、回收或调用 JS 的引用记账可以在循环内通过窄接口完成，借用不越过该操作；只有需要扩容、drain、观察或处理错误时才退出。不能把所有 heap 值复制一律变成冷出口，也不能偷偷调用任意 `Value::clone/drop` 绕过契约。

该窄接口只提供拥有权操作，不扩成 Runtime 服务集合。释放到零时需要特别检查：当前 `release_raw_no_drain` 仍可能向 zero queue 入队，名称不保证无分配。快路必须在修改引用前证明队列容量和状态转换条件，否则先退出；失败后不重做已经提交的 retain/release。第一版以正确槽操作为基础，再依据 #7 的计数缩小开销。

分派入口不返回巨型带所有语义状态的 enum。`RunExit` 仅保存种类、ID、pc/小字段；转换、构造器和内置操作的大状态进入 OperationStore/冷帧数据。测量 release 下实际 sizeof、生成函数栈帧及 .text；源码拆文件不能保证机器码栈帧变小。

首版先使用规范内存栈，建立正确数值/调用路径。栈顶缓存是后续独立实验：缓存值有唯一 owning 位置，sp 记录逻辑深度，控制流入口约定一致；调用、异常、GC/release、观察和挂起前必须物化。无收益或维护成本过高就不保留缓存。

## 7. Number 与局部更新的算法

唯一 Number 实现返回数值结果，不拥有 VM 栈或 Runtime；运行指令与编译常量折叠共用语义，编译优化仍需自己的效果证明。

| 操作 | 快路 | 验证重点 |
| --- | --- | --- |
| Int 加/减 | checked i32，溢出转 f64 输入计算 | JS Number 不做 Lua 整数回绕 |
| Int 比较 | 精确 i32 比较；混合 Number 走正确浮点比较 | NaN、±0、否定关系操作 |
| Number 位运算 | Int 直接 i32/u32，其他 Number 走 JS 转换 | ToInt32/Uint32、移位量 &31、`>>>` 大正数 |
| 乘法 | 先共用 f64；Int 专化独立验证 | `0 * -1` 的 -0、溢出和舍入 |
| 除法/余数/幂 | 共用 Number 算法 | ±0、Infinity、NaN、边界指数 |
| ++/-- | Int checked update，Float 正确更新 | prefix/postfix/discard、保留既有表示契约 |
| 加法 | 两输入确为 Number 才用数值加法 | String/BigInt/对象转换不能误入 |

普通二元栈操作：读取/取得两个输入 → 标签快路 → 成功结果取代两个已消费值；未命中则把拥有输入交给 PendingInstruction，慢路成功后压入一次结果。错误时按声明的异常栈状态展开，不能重新执行原指令。

`UpdateLocal(slot, delta, mode)` 对未捕获可写局部直接操作；mode 为 discard、prefix、postfix。顺序是取得原引用并读取（含 TDZ 检查）→ ToNumeric → 计算新值 → PutValue → 产生所需结果。可写性若由静态契约证明，可以省去动态检查；一般路径的 const 写入错误必须留在 PutValue，不能提前到转换前。postfix 的结果是转换后的旧 Numeric。回调改 x 后若转换抛错，不能恢复旧 x 抹掉它的修改。

示例反例：

```js
let x = { valueOf() { x = 40; throw 'stop'; } };
try { x++; } catch (_) {}
// x 为 40；不能恢复成原对象。

let calls = 0;
const c = { valueOf() { calls++; return 1; } };
try { c++; } catch (_) {}
// calls 为 1：先转换，再因写入 const 抛 TypeError。
```

## 8. 融合算法及限制

优化器对每个块做有界扫描，以模式规则描述读取/写入时刻、栈状态、效果、site 和可能入口。成功改写后由统一 relocation 处理所有跳转/handler/debug 表；不能各规则手算不同 PC 偏移。

优先规则是 UpdateLocal、比较紧接条件跳转、已证明无回调的局部 AddLocal。规则集保持小而明确；每条记录实际频率和独立收益。调用、eval、对象属性和任意转换不能作为可随意跨越的无副作用节点。

```js
x += g();             // 在调用 g 之前已经读取 x
let y = x + (x = 2);  // 左输入不能变成赋值之后的 x
```

因此第一例不能一般地变成 `Call g; AddLocal x`；第二例仍需保存旧 x。栈表示本身保留了这些快照，融合不能将其删除。

`CompareBranch` 消费两个输入并直接选择后继，不必生成 Bool 再弹出。慢路也只完成一次比较。`!(a < b)` 不等于 `a >= b`，NaN 是必要用例。未证明 n 不变且无转换副作用时，JS for 条件不能转换为预计算次数。

不需要的结果可省去，但有副作用的计算不能省去。TDZ/cell/private 读取、const 写入与其错误顺序由不同访问契约控制，不能仅凭最后有 Pop 就删除。

## 9. 调用、返回和参数窗口

Call 指令读已求值的 callee/receiver/实参区域，建立一个拥有输入的调用请求。driver 解析 callable、realm 和 executable，处理 bound/Proxy/constructor，并建立新的显式 frame。所有分配失败与错误都由这个请求继续保活参数，不能先清空 caller 再丢失最后引用。

```text
caller 栈上的已求值参数
→ 请求取得所有权，保存 caller 的正常/异常恢复点
→ 检查 native/VM 限额，分配或复用 callee 区
→ 初始化 frame，移动或必要复制参数
→ driver 切换 frame
→ callee 完成并经过 finally/清理
→ 关闭依赖帧槽的捕获，把最终结果交给 caller/operation
→ 清理 callee 有效槽，恢复 caller 的规范栈高度
```

返回值、throw 值在清槽前有明确拥有者。每帧的实际参数数量独立于 formal_count；多余实参、默认参数环境与 arguments 不能被布局优化丢弃。

原始实参与形参存储有以下要求：

```js
function f(a) { 'use strict'; a = 2; return arguments[0]; }
f(1); // 1
```

严格/非简单参数的 unmapped arguments 需要原始来源；可用入口快照、独立来源区或经证明的写时分离。sloppy 简单参数的映射则按重复参数、缺失实参与 cell 规则建立。证明没有 arguments/eval 等观察者时才省去不需要的来源保留。

容量复用首先消除每调用 arguments/locals/capture Vec，不先承诺零复制。尾部参数区转移必须证明唯一所有权、caller 恢复形状和挂起/捕获的独立性；特殊情况走同一布局算法的必要复制形式。

VM 帧/槽限额与真实宿主栈限额分别计量。普通 JS 深度不再线性消耗 Rust 栈；无限递归仍受可捕获资源限制，不能变成无限堆增长，也不靠扩大原生预算通过 Earley-Boyer。

## 10. 可回调操作的协议与领域归属

```rust
enum OperationStep {
    Complete(Completion),
    InvokeJs(CallRequest),
    InvokeHost(HostRequest),
    StartChild(ChildOperation),
}
```

示例类型表达数据契约，具体大 payload 由 OperationStore 拥有。每个请求关联 operation ID、恢复阶段和单次回复状态；拿到错误 owner、陈旧 ID 或第二次回复必须拒绝。同步纯计算不创建 operation。

`value/conversion` 保存 ToPrimitive 的 hint、receiver、已获取 method、当前阶段；`object` 保存 Get/Set/Proxy 的进度；`builtins/array` 保存 sort/map 等跨 callback 状态。`vm/operation` 仅做有类型的登记和 step 分派，不复制这些算法。本阶段登记采用封闭 enum 的领域状态及冷存储。

Add 慢路的流程是：保留两输入 → 左 ToPrimitive → 右 ToPrimitive → String/Number/BigInt 决策 → 计算 → 提交。子操作需要 JS 时返回 driver；子帧返回后恢复下一阶段。getter 已取到、valueOf 已调用等事实只发生一次。

引擎内部语义模块不得同步调用 driver/run 来等待 JS 完成；否则 getter→JS→getter 仍会堆积 Rust 栈。真实外部 host 回调保留同步 ABI，使用下节的特殊边界。不能把未迁移的内部调用简单改名为 host 回调绕过要求。

## 11. Completion、异常与重入

语言 Return/Throw 与内部故障保持区分。unwind 沿 frame/operation/control 区域推进，处理 catch、finally、break/continue、IteratorClose 和 constructor return。静态 handler 目标保存在 code；运行状态只保留当前有效控制区域和动态 cleanup 状态，不复制另一份可独立变化的 PC 真相。

```js
let x = 1;
try {
  const y = x + (x = obj.value);
  consume(y);
} catch (e) {
  consume(x);
}
```

getter 抛错时外层赋值未发生；getter 成功后赋值已发生，之后的转换抛错不能撤销。getter 自己可修改的 captured/eval cell 仍按实际 cell 观察。异常恢复不能只使用整个 try 块末尾的一份状态。

进入 finally 前保存旧 completion；finally 产生新的 abrupt completion 时按规范覆盖。IteratorClose 等清理错误按原优先级处理，不统一成“最后错误获胜”。被放弃的 pending 和操作输入只清理一次；关闭帧捕获后才回收其存储。

外部同步回调的协议是：发布父入口状态 → 结束所有 Execution/Runtime 借用 → 调用 host → host 若重入则建立新入口 delimiter → 子执行到达该 delimiter → 将回复交给原 pending。父运行所有者保持有效；登记 guard 在正常、错误和 Rust unwinding 下都解除临时观察记录。

保留真实 native 栈保护。不承诺跨任意同步 Rust 回调挂起；async 返回 Promise 与宿主可挂起 continuation 是不同协议。本轮不引入新的 host continuation ABI。

## 12. PC、预算与可观察状态

热循环保存下一条 cursor；Frame 保存 fault_pc 与 resume_pc，回调/错误发生时使用准确语义阶段。更新直接定位已知帧，不重复借用 Runtime、找 last frame 并检查同一 token。

| 观察点 | 要发布的状态 |
| --- | --- |
| 纯 Number/简单栈操作连续成功 | pc/sp 可局部保留，无外部读取未发布状态 |
| JS 转换、属性/内置操作、普通调用 | fault/resume、当前栈深、pending 输入和返回目标 |
| 经证明无分配/回收/回调的槽引用记账 | 可留在循环内；唯一拥有位置和引用计数始终一致 |
| 槽事务需要扩容、release drain、GC/分配或报错 | 全部有效拥有值和执行阶段；结束窗口借用 |
| throw、backtrace、debug/hook、interrupt | 精确源位置和 frame identity；调试按其观察密度执行 |
| host 重入、yield/await、恢复 | 全入口规范状态、操作进度、resume 输入及其 owner |

每 N 条更新 PC 不能替代观察契约。异步采样是否需要精确到当前 JS 指令是单独接口问题；不能把普通边界快照冒充任意时刻的精确 PC。

fuel/poll 按现有可观察契约计量；融合后如原预算按原逻辑操作数计费，应保存等价权重。减少 dispatch 不应无意放宽资源限额。内部 poll 不排空微任务、不增加用户可见抢占。

## 13. 挂起与完整入口

generator、async、async generator 共用 frame/stack/control 布局和 freeze/thaw；保留各自状态机：初始 yield、next/throw/return、yield*、async 同步前缀、thenable assimilation、async generator 请求队列。暂停只分离规范允许挂起的 activation/控制边界，不冻结整个宿主调用链。

模块 link/evaluate、live imports、TLA、dynamic import 与 host loader 按现有语义接入 driver。外来二进制转换成所选栈码后进入统一验证；不要求外部格式跟随内部融合编码改变。

所有入口最终使用一套栈执行器。临时桥可以服务迁移，但必须列出实际未完成能力；不得按测试名/benchmark 名切旧 VM。完整 parser 和可复用语义算法不属于必须删除的旧路径。

## 14. 维护演练和测量顺序

| 修改任务 | 预期主改动面 | 结构失败信号 |
| --- | --- | --- |
| 新增 Number 操作 | number、一个 opcode 契约、run 分支、相关测试；需要语法时增加 lowering | 修改 VmHost 及多个重复 numeric dispatcher |
| 修复转换顺序 | conversion 的状态/step 与一个边界反例 | 在 compiler、driver、builtin 各写一遍转换 |
| 增加异常/恢复阶段 | 领域操作状态、统一 resume/unwind/验证中的明确入口 | 把恢复藏在递归 Rust 局部或任意闭包里 |
| 修改 captured/eval 规则 | resolution/binding layout、bindings 与验证 | 所有普通 local 指令都重新查询名字/全局帧 |

演练以完整语义路径为单位，不按文件越少越好评分。复杂算法可拆成 helper，重复状态、无消费者的抽象和过宽公共接口必须删除。

S07 后的测量顺序调整为：冻结 PR19/S07 → S08 小型状态传递与无回调完成路径 → 认证运行窗口/immediate 槽操作 → PC、UpdateLocal、CompareBranch 各自 A/B 及组合 → S09 参数/帧/continuation/metadata 复用 → 编译回退与冷热布局 → 有证据时再做栈顶/编码实验。阶段内每次隔离一个变量，并验证集成后的交互。

分别记录编译阶段时间与临时内存、成品码、动态分派、run 出口、领域状态/parent 转移、槽 move/copy/clear、调用分配、retain/release、初始化、高水位与 native frame 消耗。Frame PC 写入、Runtime 观察发布、逻辑 owner 转移、编译器生成的 payload memcpy 分开计量。`size_of`、prologue 栈预留、整个调用链实测高水位也不能互相替代。

原始 Earley-Boyer 默认预算和小栈/重入是 #1 的硬门槛。S08/S09 阶段验收采用固定 50+8 全项交错 10 轮、原始 V8 八项与 combined 至少 5 轮、独立 compile API 的 67 项各 10 轮；正式计时与诊断开关分开，构建/测试/采样分阶段串行。没有独立 PC 归因就不宣称其加速；编译时间、内存和暂停不因热循环获益而省略。旧版失败的原始 Earley-Boyer/combined 不提供新旧速度或 RSS 比值；新核心持续验证默认预算成功，并与 S07 有效结果比较。

最后运行相关 QuickJS oracle、完整回归/Test262、native/Web/WASM，检查每个调用点的最终归属。验证入口见 [Test262 文档](test262.md)。不增加 skip、不改冻结预期掩盖回归、不恢复 Tachyon benchmark 工具或历史成绩文件。

## 15. 代码结构的独立改进清单

以下是当前源码核对后的结构任务，属于本 PR 的交付。它们说明文件、类型、函数与依赖如何改变；运行架构见前文。行数用于定位阅读负担，不作为拆分阈值，也不代表已测得性能损失。

### 15.1 编译入口、共享模型与解析状态分别归属

下述行数和混合职责描述是实施前基线；当前归属和验证进度见 architecture.md 与逐 commit 计划的实施记录。

当前 [compiler/mod.rs](../src/engine/compiler/mod.rs) 共 9,481 行，包含编译入口、Binding/Scope/IR 类型、FunctionIr、Parser、语句/表达式解析及 IR 片段重定位。resolution、lowering、scope_validation 已有独立实现，应复用。

目标结构中，mod.rs 仅组织入口与导出。model/ir、model/bindings、model/scope 按共同使用的数据分组；parser 中按语法职责组织函数，context/builder 集中管理解析游标和发射状态。已有 class、destructuring 等算法继续使用，不另写子集 parser，也不把所有类型搬成新的巨型 model.rs。

FunctionIr 目前同时含 `last_member_reference`、`last_identifier_reference`、`break_controls`、`stack_depth` 等解析期状态，以及闭包、eval、参数布局等后续产物。需要区分“正在构造函数的状态”和“供解析后阶段消费的数据”。先以组合结构和消费式 finish/resolve 入口表达阶段完成；复用同一份 owned 存储，以减少完整 IR 复制；本阶段采用具体的阶段类型表达责任。阶段标记只在责任明确后消除，不简单删除验证它们的检查。

**提交：S01。验收：**新增融合规则定位到 optimize/指令契约，词法错误顺序变化定位到 parser，绑定规则变化定位到 resolution/model；不会三者都经由父模块共享任意可变状态。完整语法、捕获与 eval 回归继续通过。

### 15.2 host_bridge 按绑定、构帧、挂起和语义归属拆开

当前 [host_bridge.rs](../src/engine/vm/host_bridge.rs) 共 5,358 行，内联 tests 从 4,678 行附近开始；主体还包括少量测试构造器。一个文件同时定义 FrameBinding，执行读写/捕获/关闭，编解码挂起状态，构造普通帧，准备 eval，并实现大量 VmHost 方法。

迁移归属必须逐项明确：

| 现有责任 | 目标所有者 |
| --- | --- |
| FrameBinding 与 read/write/capture/close | vm/bindings，槽拥有由 stack 承接 |
| 普通建帧、实参初始化、callee 元数据 | vm/call 与 frame |
| encode/decode、临时 roots 与恢复验证 | vm/suspend，长期 raw 记录仍在 heap |
| eval 的实际槽和 cell 检查 | vm/bindings 的 eval 边界 |
| 属性、转换、迭代器等语言步骤 | object/value/builtins 对应算法 |
| 真实外部调用与重新进入 VM | 明确 host 边界与 driver |

已有 `read_frame_binding` / `write_frame_binding` 已共享普通槽操作，不能把它们重新实现并记成新去重。创建帧、恢复帧和测试构造器的输入保证不同；共享布局与拥有权操作，保留各入口需要的验证，不合成一个由大量 bool 控制的万能构造器。过渡阶段可以拆开旧类型的实现，但最终不能只得到一组继续任意访问巨大 RuntimeVmHost 的子文件。

**提交：S03–S07。验收：**修复一个绑定写入不需阅读挂起序列化与迭代算法；挂起清理有唯一入口；删除旧 host 后没有第二个同职责容器。

### 15.3 发布验证的大函数改成显式流程与命名状态

实施前的 verify_unlinked_tree_with_root 从约 2,138 行延续至 4,757 行，混合根角色、参数、闭包来源、eval/super、指令布局和子函数遍历等检查。该文件总计 11,793 行，其中约 6,872 行位于内联 tests 模块；应分别看待生产流程和测试导航负担。

保留一个明确组织验证顺序的入口，按参数/绑定、控制流、私有状态、模块和函数树规则分出有具体输入输出的检查。树遍历仍迭代推进；工作项的八元素 tuple 改为有字段名的 PublicationWorkItem，闭包来源数组组合为命名状态。跨函数检查共享必要索引和上下文，不能每拆一个规则就重扫整棵树或复制全部分析状态。

`VerifiedFunction` 已经拥有验证后的原始草稿，要保留这一保证。纯验证归 code/verify，Atom 链接和 heap 回滚归 code/publish。原来的字节码表示检查、可达栈状态、参数布局、来源权限和动态恢复检查证明不同事实；不能仅因都叫 verify 就合并或删掉。输入尚未验证时不提前索引，外来畸形码仍被拒绝，既有首个错误行为保持。

**提交：S02。验收：**顶层验证流程能看出检查顺序；新增一项参数约束有明确归属；原外部格式反例和 mutation 检查仍有效；不会新增一个容纳所有算法的 verify.rs。

### 15.4 解除 code 对编译器配置的反向依赖

实施前 VerifiedFunction 直接使用 compiler::EvalCompileContext，code/runtime.rs 同时有编译入口调用和发布事务。当前验证所有者见 [verify/verified.rs](../src/engine/code/verify/verified.rs)，请求编排见 [api/compile.rs](../src/engine/api/compile.rs)。compiler 又依赖 code 的指令与布局，这使共享数据和上层请求编排的位置不清楚。

在 code/function 中定义发布验证所需的只读输入视图，由编译请求入口从现有 eval 上下文提供；不把解析选项或 Parser 类型传给验证器。请求编排移到 api 的编译入口，code 接受草稿/验证结果。移动原方法的责任归属，不新增转发 Runtime，也不复制验证上下文和完整绑定数组。

**提交：S01–S02；S07 收口入口。验收：**生产 code 的表示/验证不再导入 compiler；验证所需的 caller 权限、名字及 profile 不减少。测试可以调用 compiler 生成真实草稿，不能将这种测试依赖误报为生产依赖。

### 15.5 明确依赖与数值代码命名

VM 的 protocol、activation、frame_execution、dispatch、numeric 等文件通过 `use super::*` 获取父模块 namespace；vm/mod.rs 又汇集并通配导出多个子模块。compiler/class/fields 等使用 `super::super::*`。Rust 的父子可见性允许这种写法，但读者难以从一个文件的入口识别依赖。

生产模块从实际所有者显式导入，相关共享类型放进小型 model/protocol；模块入口只显式导出消费者需要的名称。保留一个 crate 与当前公有 API 边界，本阶段按实际消费者组织接口与可见性。单元测试中的 `use super::*` 和局部 enum variant 导入不做机械禁止。

[value/number](../src/engine/value/number/mod.rs) 的既有 pow、ToInt32、float16 与完整格式化算法已在 S03 按 operations/integer/format/float16 拆分，原入口和测试保留；vm/numeric 继续调用同一纯算法。普通槽和 Number 主循环已在非默认 `stack-vm` 配置接入，S03 已验收；显式调用和其余领域协议仍按 S04–S07 迁移。对象 ToPrimitive/ToNumeric 属于 value/conversion，栈 pop/push 属于 run/stack。源码中的 numeric、numeric_execution、dispatch 不再让调用者猜测同一操作究竟在哪层完成；必要的冷函数仍保留，最终机器码帧大小另行测量。

**提交：S01–S03；S10 清理。验收：**从 imports 和类型归属能识别依赖；新增 Number 运算不增加一层转发或在格式化文件中混入 VM 状态。

### 15.6 测试、结构检查与维护文档同步迁移

大段内联测试可按 stack/control、parameters/eval、closures、private、modules、malformed 分到验证模块的 tests 子目录；保持单元测试所需私有可见性和真实发布入口覆盖。编译器现有 tests/ 已按语义分类，可继续使用。文件移动复用既有行为测试，不添加仅断言函数被拆开的镜像测试。

[binary_object/layout.py](../scripts/checks/binary_object/layout.py) 登记物理文件归属，[runtime_protocols.py](../scripts/checks/binary_object/rules/runtime_protocols.py) 还对指定函数做源码形状/哈希检查。结构迁移必须同步到真实新调用路径与所有者；每次更新保留其防止断开生产入口、绕过验证等检查目的，并以原畸形码和 mutation 反例核验。不能只替换哈希就宣称新结构正确，也不能因旧文件删除而跳过整个检查。

[架构说明](architecture.md)维护跨模块入口与职责；局部不变量由所属 Rust 类型、函数和测试共同说明。源码中的过期导航（如 code/bytecode_validation 注释仍称规则在 heap）随迁移修正。源码布局检查只检查源码归属、模块可达性与公有边界，不检查 README 的存在、内容或覆盖率。新文件和方法按责任命名，避免使用旧迁移阶段编号作为生产概念。

**提交：随 S01–S09 的对应改动完成；S10 最终核验。验收：**从架构说明和源码契约到生产入口、反例测试可以连续定位；没有过期路径、未编译的替代实现或因改名静默失效的检查。

这些结构项合入一个 PR 的 10 个完整提交单元；文件移动、接口、算法、测试和对应文档随其责任一起交付。较大的提交按上述领域条目组织审查，明确标识结构移动与语义变化；开发中的临时提交归并回所属单元。提交数减少后，阶段依赖、逐调用点清单和完整检查仍保留，不追求固定文件行数。
