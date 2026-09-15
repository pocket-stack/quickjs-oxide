# Workspace architecture

This document describes the current implementation and its responsibility
boundaries. The [stack VM plan](primitive-vm-plan.md) describes the pending
redesign, including the structures that will replace the current execution
path. As of 2026-09-12, that redesign is still a plan; it has no new engine
implementation or measured optimization result.

## Packages and module owners

One root package, `quickjs-oxide`, contains the complete interpreter.
`src/lib.rs` is its only top-level Rust source file.

```text
src/
  lib.rs
  source/                 exact source bytes, positions and Unicode support
  regexp/                 pattern compilation, programs, matching and interruption
  engine/
    compiler/             lexing, parsing, scopes, resolution and lowering
    code/                 instructions, drafts, verification and publication
    value/                JS values, strings, numbers and runtime conversions
    object/               properties, shapes and object internal methods
    atom/                 interned names, symbols and property keys
    heap/                 raw records, storage, retention, tracing and collection
    vm/                   frames, calls, dispatch, exceptions and suspension
    realm/                global bindings, prototypes and initialization
    builtins/             language builtin algorithms and native call dispatch
    modules/              module instances, loading, linking and evaluation
    jobs/                 queued computations, retained roots and cleanup
    host/                 environment capability contracts
    api/                  Runtime/Context and embedding operations
```

This is a map of current owners, not a requirement to retain every internal
file or interface. The VM plan specifies the new compiler, code and execution
structure. Shared use alone does not require a separate crate, service trait
or forwarding layer.

| Package directory | Production workspace dependencies |
| --- | --- |
| Repository root: quickjs-oxide | None |
| adapters/native: quickjs-oxide-host | quickjs-oxide |
| adapters/web: quickjs-oxide-web-host | quickjs-oxide |
| apps/cli | quickjs-oxide, native adapter |
| apps/web | quickjs-oxide, web adapter |
| conformance/test262 | quickjs-oxide with test262-host, native adapter |

Applications construct `Runtime::new_with_host_services(provider)`. Adapters
implement environment capabilities such as clocks, timezone, random seed and
output; they do not implement JS coercion or execution policy. Applications
own inputs, module-loading policy, diagnostics and job driving. The engine
has no production dependency on its adapters, applications or conformance
runner.

## Current compilation and execution

The compiler produces linear `FunctionIr/IrOp` and then stack instructions.
Shared operation and source-site data now lives in `compiler/model/ir.rs`,
lexical identities and declaration-order indexes in `model/scope.rs`, and
binding storage/declaration records in `model/bindings.rs`. Resolution and
lowering consume those owners explicitly. Completed function artifacts live
in `model/ir/function.rs`. `parser/context.rs` owns the lexer cursor, grammar
context and temporary per-function reference/control state. During parsing,
`parser/builder.rs` owns each function's single IR allocation and emission
state; its consuming `finish` rejects unclosed scopes or controls before
moving the artifact into `FunctionTree`. Temporary state is then dropped,
while the completed body-boundary fact remains available to suspension
metadata validation. Grammar methods live in the corresponding parser modules;
the existing class, destructuring and resolution algorithms remain in use.
`compiler/relocation.rs` owns fragment insertion, prefix relocation and the
lowered instruction offset map. `flow.rs` exposes structural block entries
for local rewrites and delegates required normal/exception/resume stack facts
to the existing code verifier. `optimize.rs` owns the bounded constant-branch
rewrite and its ordered QuickJS late-throw source-site projection. It runs the
projection before rewriting and preserves physical instruction slots.
Profiling builds expose scoped compile/legacy-dispatch counters through
`api/profiling/cost.rs`; default builds contain no instrumentation hooks.

Expression intermediates live on an operand stack; numbered locals do not
make this a register VM. The current execution path splits arguments and
locals in `RuntimeVmHost` from the operand stack in `VmActivation`, with
additional active-frame tracking. Ordinary JS calls recursively enter the
Rust interpreter. These are the principal ownership and driving boundaries
that the pending plan replaces.

Code verification and transactional publication already exist. Published
instructions and constant storage already share immutable arrays; the plan
must account for remaining per-call projections and roots rather than treat
sharing as a missing feature. Code representation and publication belong in
`code`; active pc, stack position and call state belong in `vm`.

The redesign keeps stack instructions and the complete language frontend.
It introduces explicit JS frames, one execution driver, domain-owned callback
state and concentrated slot ownership. Algorithms, proposed files and
migration order live in the [implementation design](primitive-vm-implementation-plan.md),
[10-commit plan](primitive-vm-commit-plan.md) and
[migration checklist](primitive-vm-migration.md). Those documents are the
implementation plan; this overview does not duplicate their future file tree.

## State and semantic boundaries

`Runtime` is defined in `api/runtime.rs`. `RuntimeInner` and `RuntimeState`
belong to `heap/runtime` and own the shared heap/atom domain and cleanup.
Runtime methods live with their behavior: properties in `object`, conversions
in `value`, bindings in `realm`, publication in `code`, execution in `vm`
and builtin algorithms in `builtins`. These are inherent implementations of
one Runtime type, not parallel runtimes.

Heap records retain raw data and reference edges; storage operations maintain
those edges. Language algorithms remain with their semantic owner. Active
rooted values and long-lived heap records must preserve the same retention
and collection model. The VM plan details the ownership transfer required
when execution suspends or resumes.

The following boundaries apply across internal reorganizations:

- Source and module inputs preserve exact bytes and source positions.
  Compiler constants use the restricted `PrimitiveValue` representation;
  compilation does not create live Object or Symbol roots. Full `Value`
  preserves Object and Symbol identity.
- Verification authenticates the exact code that will be published.
  Publication links names, retains roots and rolls back failures. Published
  handles and runtime-owned identities cannot be reused across runtimes.
- Pure Number and string helpers remain separate from runtime coercion.
  ToPrimitive, property operations and builtin callbacks may execute JS.
  Borrowed slots, heap views and buffer access must obey their callback and
  mutation boundaries; their validity cannot be inferred from a directory.
- Strings preserve exact UTF-16 code-unit semantics, including lone
  surrogates. Latin1 and UTF-16 storage forms must agree for equality and
  hashing. Unicode algorithms and checked-in generated tables belong to
  `source/unicode`.
- The `regexp` module owns pattern programs and matching. The JS RegExp
  object, property access, coercion and replacement callbacks belong to
  `engine/builtins/regexp`. Source and regexp are engine siblings sharing
  the existing string carrier, not independent Cargo packages.
- Module-host callbacks can reenter through their originating Context.
  JS-valued failures preserve the exact thrown value, including Object and
  Symbol identity. They are not converted to text diagnostics.
- Jobs own pending computation and its retained roots. Applications decide
  when to drain the queue; internal VM restructuring must preserve Promise,
  async and module ordering.

## Public API and internal visibility

Embedders use `engine::api`, the only public engine module.
`src/lib.rs` has no legacy root reexports. Test-support and Test262 hooks
remain explicit opt-in surfaces; detached VM fixtures are unit-test-only.

Use explicit imports from the actual owner and the narrowest visibility
needed by callers. A shared Runtime type does not justify wildcard imports
through its implementation module. New public capabilities belong in
`api`; helpers must not expose a second value system or execution path.

The BC5 decoder remains private under `engine/code`; only
`code/binary_object_publish` consumes its archive models. Decoder
intermediates retain restricted visibility and `ConstructorRef` remains
opaque. Internal compiler and code types do not promise a stable external
bytecode format or an independent compiler product.

## Documentation and verification

Cross-module responsibilities are maintained here; active redesign decisions
belong in the VM plan. Local algorithm and ownership contracts belong beside
their Rust types and functions. Add a directory guide only when it provides
useful navigation or operating instructions; there is no per-directory
README requirement.

`scripts/checks/check-source-layout.py` checks source ownership, Rust module
reachability and the public API boundary. It does not inspect documentation.
When source owners change, update the relevant boundary checks and their
negative cases to follow the real production route, not just new filenames
or fingerprints.

Use the [verification entry point](../README.md#verify) and the affected
owners' tests. Frozen oracle and Test262 receipts refer to their recorded
source; a refactor or documentation edit does not renew them. The
[archived architecture and completed plans](archive/README.md) preserve
previous decisions and results without prescribing the current VM design.

S02 已验收：源代码请求及编译错误边界归 `api/compile.rs`；`code/runtime.rs` 负责发布事务。eval 验证接收 `code/function/publication.rs` 的只读权限视图，借用调用方绑定与 profile，不再依赖编译器上下文。函数树验证以命名工作项携带闭包来源状态，保持迭代顺序；入口权限、参数布局、绑定和模块表验证分别归 `verify/{roles,parameters,bindings,modules}.rs`，主流程按原先顺序消费参数分析产物。只读验证与原有测试归 code/verify；Atom 链接、展平和私有绑定发布归 code/bytecode_publish，发布事务仍归 code/runtime。私有绑定名称与配对规则复用 verify/private_elements 中的只读辅助函数。控制流/闭包树拆分、指令契约和帧布局已接入，S02 的 709 项完整边界反例已拒绝；S03 非默认原语栈核心已验收，完整迁移仍按 S04–S10 推进。

S02 的 eval 环境、来源权限和拓扑检查归 `verify/eval.rs`；子函数捕获与入队归 `verify/children.rs`。后者借用父函数已经建立的参数分析和闭包来源，只向同一迭代队列追加工作项，不重扫函数树。

S02 主验证流程依次消费 `verify/closures.rs` 的声明索引、`verify/flow.rs` 的入口/控制流结果和 `verify/operands.rs` 的操作数权限检查。class 边界继续使用既有可达栈验证之后的分支目标与子闭包位置；不同验证层的事实和错误顺序保留。原测试按 modules/private/parameters/eval/bindings 分组，共用原有草稿构造 helper。

`code/function/layout.rs` 的 FrameLayout 是 rooted executable 的只读视图，借用同一份参数、局部、闭包定义与 metadata。构帧保留额外实参容量，恢复形状检查复用这些定义；RootedVmActivation 不再另存 bytecode/code/metadata。静态布局不拥有调用的原始实参数组，动态恢复身份与 region 检查仍属于 VM。

`code/instruction.rs` 集中 nominal 栈效果、控制流与可捕获 JS 异常分类，供栈验证、module initializer flow 和编译器块边界共用。该描述不代替依赖活动 region、private Reference 和 resume 身份的动态检查；通用回调/分配效果采用保守上界，操作数以槽位、常量角色、目标和隐藏来源等类型描述；可选 profiling 反汇编输出同一契约。

S03 已验收：`value/number/{operations,integer,format,float16}.rs` 分别拥有纯数值算法，原有 Number 公共入口通过显式导出保持兼容。`vm/bindings.rs` 拥有运行期 typed binding 的共享读写、捕获、关闭和闭包视图规则，既有 host 与 heap cell 验证使用同一实现。挂起编码仍在原 VM 适配层，非默认 `stack-vm` 配置已接入帧/槽容器和主循环原型；完整调用/挂起/入口迁移仍属后续阶段。

`vm/frame.rs` 的 Frame 持有 executable、窗口索引和 boxed cold state；`vm/stack.rs` 管理互斥的原始实参、形参、局部及 operand 区间。`vm/execution.rs` 的作用域登记只保存 domain/identity，不把运行 Value 放进 Runtime。`vm/run.rs` 直接执行已覆盖的普通槽与纯 Number 操作。`host_bridge/owned.rs` 是临时单向交接适配器：遇到未覆盖指令时，输入和 PC 尚未消费，现有 owner 移回旧路径；默认配置保持原执行器。引用预检、完整成本与后续语义迁移仍待完成。

S08/S09 当前 PC 选择（覆盖上述早期迁移记录的表示方式）：`run` 每次实际 dispatch/span
入口直接写 Frame fault PC，仅把 resume PC 保留为局部值；Drop guard 在正常、错误、
冷出口、挂起和 Rust unwind 时物化 resume。Runtime 活跃帧发布仍在既有 driver
观察出口执行，不随 Frame fault 写入变成逐指令发布。有限融合与 AddStore 的错误
位置不变，异步 CPU 采样不承诺任意时刻的精确 JS PC。该选择来自普通 A/B 的逐项
权衡，不以少写次数代替吞吐证据；完整阶段验收仍在进行，详见
[有限融合与观察点](architecture/owned-fusion.md) 和
[开发测量记录](performance/README.md)。

S03 的 `heap/slot_ownership.rs` 提供受限引用预检和提交：Runtime 域、借用、
deferred references、zero queue 与 primitive 共享存储共同决定是否能热释放。
不能热释放时保留原操作数交接；`SlotStore` 负责逻辑 owner 的移动、复制、重排
和帧清理，原始实参快照及交接缓冲先完成可失败预留。运行根仍由作用域拥有，
登记表只保留身份。`profiling::OwnedStorageCost` 从实际操作收集逐存储区容量、
活跃槽和逻辑转移；排除完整调用分配与回收级联，详细口径见 profiling.md。

S04 开始迁移构帧责任：`vm/call/prepare.rs` 返回拥有 executable、参数/局部、
CallInput 和 ActiveFrameGuard 的 PreparedBytecodeFrame；host_bridge 只消费
准备结果并选择现有普通/挂起入口。此准备阶段保持参数补齐、词法初始化与
函数名绑定的原顺序。`vm/driver.rs` 已驱动普通字节码子帧，共享同一执行的
FrameStore/SlotStore；子帧的旧路径交接只转移该帧，父帧仍由 driver 持有。
正常结果恢复父帧，尾调用结果向上传递；控制记录和 capture 创建指令仍在
执行前交接，因此当前直接传播 Throw 只适用于没有这些记录的 owned 父帧。
启用对应指令前必须完成统一展开和 capture close；完整 S04 尚未验收。

S04 的 `PushThis` 直接处理无需装箱的 receiver；原语装箱返回 driver，
发布 PC 后调用无 JS 回调的对象分配。FrameCold 持有 normalized_this，
交接时随帧转移，因此同一调用中的包装对象身份保持稳定。

S04 的 `call/request.rs` 拥有已分类字节码调用的完整输入，供普通调用与静态
属性 getter 共用构帧。driver 消费普通查找结果后安装 getter 子帧；GetField2
保留原 receiver，结果只恢复一次。完整转换操作状态和统一展开仍待迁移。
