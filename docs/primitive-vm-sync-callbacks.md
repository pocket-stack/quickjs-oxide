# S05 同步回调调用点账本

状态：2026-09-14 当前 S07 源码审计，**S05、S06、S07 各自统一正式验收通过**。
本表替代早期 129 个直接表达式的迁移状态快照；旧行号和已删除函数不再作为待办。
源码覆盖、定向测试和阶段验收分别记录，不能以注册成功或本表清单清空代替验收。

## 审计边界与分类

审计入口是 `builtins/native.rs::NativeFunctionId` 及其子 selector，逐项对照
`builtins/continuation.rs::NativeOperation::{for_target,start}`、领域 `Step/Resume`、
旧同步消费者和 `vm/proxy_get_driver` adapter。间接回调包含 Get/Set/Has、
ToPrimitive/Number/String/BigInt、constructor prototype/species、iterator next/close。
`NativeOperation::Pure` 仅表示该 selector 不调用 JS；不是把通用同步 dispatcher
当成可回调的 owned 实现。新旧入口共享同一领域算法，旧 `finish` 中的调用保留。

- **Owned：**领域状态拥有等待所需值，查询/调用由 driver 的显式请求恢复。
- **NoJs：**无 JS 回调的存储、品牌检查、纯计算或错误构造，保留普通函数。
- **S06：**generator/async/Promise/async iterator 的执行与恢复，即便执行同步前缀也不改归 S05。
- **S07：**module、真实 host、API 和 binary 入口；不能把内部 builtin 回调伪装为 host。
- **S05 起点缺口：**下文保留迁移前的直接调用快照；本轮已收口并验收。S07 的完整入口配置已通过统一验收。

## Native 领域与间接调用账本

以下路径均相对于 `src/engine/`；函数和类型名是稳定定位依据。

| 入口/领域 | 共享算法与间接回调边界 | 当前分类 |
| --- | --- | --- |
| Object/Reflect 属性、描述符、keys/values/entries/assign/integrity/accessor | `builtins/object/{property,definitions,predicate,prototype,string}.rs::*Step::start`；Get/Set/Has/OwnKeys/Descriptor/Define、键和描述符转换、getter/setter、Proxy 不变量 | Owned；`Object.is/valueOf` 为 NoJs |
| Object constructor / create | `builtins/object/constructor.rs::ObjectConstructorStep::start`、`definitions.rs`；new.target prototype 与 descriptor 对象查询 | Owned |
| Proxy Get/Set/Has/Delete/Define/OwnKeys/Prototype/Call/Construct | `object/internal_methods/{get,set,define,boolean,own_keys,prototype,call,construct}.rs`；handler Get、trap Call、target 查询与约束 | Owned；Proxy 创建/revoke 为 NoJs |
| Array constructor/from/of | `builtins/array/{constructor,build}.rs::*Step::start`；prototype、iterator method/next/done/value、mapper、构造、定义与 close | Owned |
| Array callbacks/find/reduce/sort/flat/flatMap | `builtins/array/{callback,sort,flatten}.rs::*Step::start`；callback、比较结果转换、holes/length、species、递归展开 | Owned；flatten 进度在领域状态 |
| Array 索引、变更、slice/splice/concat/stringification | `builtins/array/{indexed,mutation,slice,concat,reverse,string,species}.rs`；ToObject、length/index/值转换、Has/Get/Set/Delete、species、toLocaleString | Owned；isArray/iterator creation/species getter为 NoJs |
| Iterator next/close、helper、consumer、wrap/from/concat | `builtins/iterator/{step,array,helper,consume,create,wrap,from,concat}.rs`；普通与 raw next 区分、Get done/value、callback、InstanceOf、close 和原错误优先级 | Owned；String/Map/Set iterator raw next 为 NoJs |
| Iterator constructor/accessors/tag | `builtins/iterator/{constructor,entry}.rs`；new.target prototype 与 tag setter 查询 | Owned；identity/getter及原 context-free accessor define 纯段为 NoJs |
| Object.fromEntries/Object.groupBy/Map.groupBy | `builtins/object/iteration.rs::IterationStep::start`；iterator、pair Get、callback/ToPropertyKey、group push、close | Owned；保持 fromEntries/groupBy 不同的 close 规则 |
| Map/Set/WeakMap/WeakSet constructors | `builtins/iterator/collection.rs::CollectionStep::start`；prototype/adder/iterator、pair Get、插入调用、close | Owned |
| Map forEach/getOrInsert(computed)、Set forEach | `builtins/map/callback.rs::CallbackStep::start`、`set/callback.rs::EachStep::start`；callback 与活动记录 guard | Owned；无 callback 的查询/存储/迭代器创建为 NoJs |
| Set 七项 set-like 操作 | `builtins/set/operations.rs::SetStep::start`；size 转换、has/keys 方法、iterator 与提前 close | Owned |
| WeakMap computed | `builtins/weak_collection/computed.rs::ComputedStep::start`；callback 前持有 key owner，返回后写入 | Owned；其他 WeakMap/WeakSet 品牌与存储为 NoJs |
| WeakRef/FinalizationRegistry | `builtins/weak_ref/constructor.rs::WeakConstructorStep::start`；先验证 target/callback，后请求 new.target prototype | 构造 Owned；deref/register/unregister 为 NoJs，不在 register 时执行 cleanup callback |
| String factories/raw | `builtins/string/factory.rs::StringFactoryStep::start`；fromCharCode/fromCodePoint/codePointRange 数值转换、raw 的 raw/length/索引 Get、chunk/substitution ToString | Owned；保留 latched 字符串长度错误顺序 |
| String prototype / RegExp protocol | `builtins/primitive/text.rs`、`builtins/string/{text,search,regexp,replace,split}.rs`；receiver/参数转换、symbol method Get/Call、replacement callback | Owned |
| RegExp constructor/compile/exec/test/presentation/协议 | `builtins/regexp/{constructor,compile,exec,prototype,search,match_protocol,match_all_protocol,split,replace,species}.rs`；pattern/flags、exec、lastIndex、species、replacement | Owned；escape只接受原始String，species为 NoJs |
| RegExp String Iterator next | `builtins/regexp/iterator_next.rs::RegExpIteratorStep::start`；Exec、Get(0)/ToString、空匹配 lastIndex Get/转换/Set | Owned；保留 raw result 和 normal materialization |
| ArrayBuffer/SharedArrayBuffer/DataView | `builtins/array_buffer/{constructor,mutation,slice}.rs`、`builtins/array_buffer/data_view.rs::{DataViewConstructorStep,DataViewAccessStep}`；options/length/index/value、new.target/species、resize/detach后重验 | Owned；各品牌 getter/isView 为 NoJs |
| TypedArray constructor/from/of | `builtins/array_buffer/typed_array/{create,collect}.rs::*Step::start`；iterator method/普通 next、done/value、length、mapper、prototype/construct、元素转换 | Owned；collect 保持原无 IteratorClose 规则 |
| TypedArray callback/sort/species/element/string/index/codec | `typed_array/{iteration,traversal,sort,species,element,search,mutation,slice,stringification,copying,set,uint8_codec}.rs`（位于 `builtins/array_buffer/`） | Owned；转换后重验 view/buffer；品牌 getter/reverse/toReversed 为 NoJs |
| Atomics | `builtins/atomics/operation.rs::AtomicsStep::start`；index、store/operation值、wait expected/timeout、notify count、isLockFree 转换 | Owned；validate→转换→按原模式重验；memory/waiter/pause 纯段保持原实现 |
| Function call/apply/Reflect apply/construct | `builtins/function/{invoke,arguments}.rs`；array-like length/索引、Call/Construct | Owned |
| Function bind/toString/hasInstance、instanceof | `builtins/function/{bind,text,instance}.rs`；prototype/length/name Get、ToString、@@hasInstance、普通prototype链 | Owned；debug getters/FunctionPrototype/ThrowTypeError为 NoJs |
| Dynamic Function constructors | `builtins/function/dynamic.rs::DynamicFunctionStep::start`；逐参数/body ToString、Eval请求、new.target prototype | 构造同步部分 Owned，包括生成器/async function 的源码构造；生成后执行/恢复归 S06 |
| Direct/global eval | `builtins/eval.rs::{prepare_direct_eval_original,prepare_indirect_string_eval}`、`vm/eval_driver.rs`；prepared callable以显式子帧执行 | Owned；host evalScript/module入口归 S07 |
| Primitive constructors | `builtins/primitive/constructor.rs::PrimitiveConstructorStep::start`；Number/String/BigInt转换、Symbol描述及new.target prototype | Owned；Boolean转换和已品牌化 valueOf为 NoJs |
| Number/BigInt 格式、全局 parse/predicate/URI、Symbol.for | `builtins/primitive/{numeric,globals}.rs::*Step::start`；radix/digits/width、BigInt input、string/number转换 | Owned；NumberPredicate、Symbol.keyFor/description为 NoJs |
| Math | `builtins/math/{operation,sum}.rs::*Step::start`；全部numeric参数，sumPrecise iterator/next/close | Owned；Random为 NoJs |
| Date constructor/parse/UTC | `builtins/date/constructor/operation.rs::DateConstructorStep::start`；Default ToPrimitive、ToString/ToNumber、new.target prototype | Owned；Now纯时间读取 |
| Date setter/toJSON/@@toPrimitive | `builtins/date/prototype/operation.rs::DatePrototypeStep::start`；setter每参数ToNumber、toJSON ToPrimitive/Get(toISOString)/Call；@@toPrimitive直接OrdinaryToPrimitive，不能再读自身symbol | Owned；timeValue/string/getField/timezoneOffset为品牌后NoJs |
| Error/AggregateError/toString | `builtins/error/operation.rs::ErrorStep::start`、`aggregate.rs::AggregateStep::start`；prototype、message ToString、cause Has/Get、errors iterator/close、name/message Get/ToString | Owned；Error.isError为 NoJs |
| JSON parse/stringify/raw | `builtins/json/reviver.rs::ParseStep::start`、`stringify/operation.rs::StringifyStep::start`、`raw.rs::JsonRawResume::string`；source、reviver、toJSON/replacer、gap/keys/value转换、Get/OwnKeys | Owned；isRawJSON为 NoJs |

## VM 路径与阶段留项

| 入口 | 当前分类/精确源码 |
| --- | --- |
| 属性读写、super、in/delete、with/动态环境 | Owned：`vm/{property_driver,property_write_driver,super_property_driver,predicate_driver,with_driver,environment_driver}.rs`，共享object/value请求 |
| 同步 for-of、destructuring、spread/apply、IteratorClose | Owned：`vm/iterator_driver.rs::{drive,finish}`、`PendingIterator::{advance_query,next_query}`；Query消费action在同一循环推进，不递归再次进入query |
| for-in Proxy keys/descriptor/prototype | Owned：`vm/for_in/operation.rs::ForInStep::{start,next}`，`vm/frame_operations.rs`入口 |
| 算术/比较/BigInt/对象转换 | Owned：`vm/numeric/operation.rs::NumericKind::for_instruction`和`NumericStep::start`；非number热路径失败进入Numeric，不是整帧Bridge |
| literal definition、class heritage/public field/private initializer | Owned：`vm/{array_driver,construct_driver}.rs`、`object/class_fields.rs`；computed raw key的LiteralDefinition查询由当前工作区接入；authored常规键已覆盖 |
| PushAtomValueIndex/RegExp/ThrowDeleteSuper/InitializeVarRef/InitializeDerivedVarRef/SetProto/TypeOf/Is族/对象条件分支/CheckCtor | NoJs冷路径：`vm/pure_operations.rs::{step,perform}`；共享旧Runtime叶，错误留在owned unwind |
| 普通调用拒绝、默认派生构造器非法super | `vm/driver.rs::{enter_call,rejected_call}`、`construct_driver.rs::enter_default_derived`已改owned错误；非callable Proxy仍先由ProxyCall请求读apply再拒绝，不能提前纯TypeError |
| generator/async native selector | S06 Owned：GeneratorStep、AsyncStep、AsyncGeneratorStep，共用 ResumeFrame/完成/挂起回复 |
| Promise全部selector/resolve/capability/finally/聚合、job与thenable | S06 Owned：`promise/operation.rs` 及 capability/resolve/then/jobs/finally/convenience/aggregate；executor、species、thenable 与聚合内部 callback 都显式请求 |
| AsyncFromSyncIteratorResume/Unwrap/Close | S06 Owned：`vm/async_from_sync_iterator/operation.rs` 的读/调用/Resolve/Close 请求 |
| ForAwaitOfStart/Next、IteratorGetValueDone、IteratorStart/Next/Call/CheckObject、AsyncIteratorStart、InitialYield/Yield/YieldStar/AsyncYieldStar/Await/ThrowIteratorMissingThrow | S06 Owned：`vm/run.rs`、`iterator_driver/suspension.rs` 和 `suspend` 共用协议。`compiler/generator.rs`生成yield*协议；`compiler/parser/{loops,control}.rs`仅在async迭代/async generator close分支生成相应检查，不是同步destructuring缺口 |
| InitializeModuleImportCollision、Import、ModuleEvaluation、DynamicImportHandler | S07 已接入共享 binding 验证、Import/Link/Evaluation/Body/Callback 阶段；动态 load job 依次消费根操作，保留错误转换和 FIFO；统一验收通过 |
| Test262DetachArrayBuffer/EvalScript/CreateRealm/IsHtmlDda/Gc/Agent、QjsPrint/QjsConsoleLog | S07 已登记 host/API 入口；evalScript/Agent 的可观察转换使用 continuation。print/console.log 使用无 JS 回调的 qjs 诊断格式化，保留 WTF-8、实际 argc、换行/flush 与忽略 I/O 失败政策；输出调用是真实 host 边界。S07 统一验收通过 |
| cfg(test) ArgumentProbe/ConstructorProbe/ConstructorOrFunctionProbe/ActiveFrameProbe | 前三种为无 JS 回调测试叶；ActiveFrameProbe 共享 InvokeStep 保留 native frame 跨 JS 回调；非生产 ECMAScript intrinsic |

## 本轮 S05 缺口收口与验收

- 本次发现并已修复的最后引用覆盖：`vm/run.rs::RunExit::ReplaceBinding` 与
  `vm/frame_operations.rs::step` 已接管 SetLocalUninitialized/InitializeLocal、
  PutLocal/SetLocal及checked变体、PutArg/SetArg的非Ready释放；先发布新值后在冷路径
  释放旧owner。主代理定向run组10项通过（含6个last-root场景），不再列为待实现缺口。
- method 定义已分为 `object/object_literal.rs::prepare_object_literal_method` 的
  无回调命名/HomeObject/描述符准备与 `LiteralDefinitionStep::Define` 请求。
  最终 DefineOrdinary 保留旧自有定义规则；Array length 的两次转换后拒绝、
  TypedArray 索引转换、Proxy target 不触发 defineProperty trap、native/bound 方法值
  均由 checkpoint 与旧消费者对照验证。computed key 仍遵守原 canonical 输入契约。
- 子帧安装通过 `FrameStore::prepare_push` 先验证身份/限额并预留，再安装 slot window；
  最后安装 frame 不再失败。拒绝请求不留下孤立窗口。`RunningExecution::drop` 和
  FrameStore 按内到外释放，先清 child slots，再释放 child 与父 native continuation；
  强制限额/身份失败及真实 owner 析构顺序的 5 项定向测试通过。
- 完整门禁已覆盖上述 trap/holes/length/sort/iterator/Unicode/buffer/shared/BigInt/GC：
  新库 2197、旧库 2032、新旧 oracle 各 912（含单独压力用例）、CLI 各 5 与 723 个
  边界反例通过。默认预算原始 Earley-Boyer 独立及组合均完整输出成绩；旧分派和
  整帧交接为零。同步桥分别为 3/10，每个事件均由可读的 QjsConsoleLog 目标实参、
  同次输出调用栈和 profile 计数对账归入 S07；没有按预期打印次数扣除。
  初次组合诊断的 600 秒外部超时仍记录为失败，后续完整归因未改变 VM 预算/脚本。
  原始日志、源码/二进制哈希与归因结论见 `target/primitive-vm-s05-acceptance/`。
  S07 后的正式 benchmark/profile 尚未开始。

## 诊断与已取得的定向证据

`legacy_dispatches`计旧指令；`owned_bridge_exits`计整帧交接；
`owned_sync_call_bridges`计owned调用进入剩余同步Runtime边界，包含无回调callee，
不是所有内部嵌套调用总数。计数归零仅证明对应测量fixture，不能推广到S06/S07。

- `rejected_calls_and_default_super_stay_owned`：定向1/1通过（9个fixture）；
  普通/尾调用非函数、非法live super、class无new和非callable Proxy apply getter异常
  均保留错误并可catch，三个旧路径计数均0。旧错误共享构造与caller realm保持。
- `iterator_and_collection_callbacks_stay_on_owned_driver`：最近定向运行1/1通过；
  fromEntries/groupBy、iterator/Array.from/collections、RegExp iterator三种消费模式、
  String factories、Atomics转换及CheckCtor用例均返回42，三个旧路径计数均0。
- `recursive_group_by_uses_default_logical_budget_and_recovers`、
  `recursive_from_entries_uses_default_logical_budget_closes_and_recovers`：2/2通过；
  保留65535默认逻辑帧预算，真正无限递归产生可catch的stack overflow并可恢复；
  默认VM旧native阈值断言仍由原cfg保留，未调整冻结oracle或预算。
- 最新合并领域筛查32项通过，唯一counter失败随后由Symbol DefineArrayEl修复并由上述
  独立counter测试复验通过；这不冒称该历史合并命令曾全通过。

## 直接调用表达式复核

下表保留 S05 收口时 `object/builtins/value` 中生产直接调用表达式的定位；
同一函数内不同调用仍分行。行号为本次审计快照，后续以函数名定位。
旧consumer中的`call_internal`是保留的同步消费者，不等于owned路径还调用该consumer。
VM另外保留 `vm/call_bridge.rs::PendingCall::invoke`、
`vm/conversion_driver.rs::invoke`、`vm/proxy_get_driver.rs::advance_inner`三处
未迁移callee回退；S05 验收时其剩余分类为 S06/S07 selector 和非 Normal bytecode；S06 已接入全部挂起族及间接回调；S07 已登记 module/host/API 余项，完整入口验收通过。
`vm/host_bridge.rs`及`host_bridge/private_elements.rs`是旧VM消费者，
owned 对应 property/private/eval/iterator 入口已分离；host/module 的 S07 当前归属见文末。

| 当前调用位置/函数 | 边界 | 分类 |
| --- | --- | --- |
| [src/engine/builtins/array/build.rs:540](../src/engine/builtins/array/build.rs#L540) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/array/callback.rs:442](../src/engine/builtins/array/callback.rs#L442) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/array/flatten.rs:434](../src/engine/builtins/array/flatten.rs#L434) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/array/sort.rs:454](../src/engine/builtins/array/sort.rs#L454) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/array/string.rs:295](../src/engine/builtins/array/string.rs#L295) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/array_buffer/typed_array/collect.rs:275](../src/engine/builtins/array_buffer/typed_array/collect.rs#L275) `finish_collect` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/array_buffer/typed_array/create.rs:738](../src/engine/builtins/array_buffer/typed_array/create.rs#L738) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/array_buffer/typed_array/element.rs:48](../src/engine/builtins/array_buffer/typed_array/element.rs#L48) `finish_sync` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/array_buffer/typed_array/traversal.rs:270](../src/engine/builtins/array_buffer/typed_array/traversal.rs#L270) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/date/prototype/operation.rs:291](../src/engine/builtins/date/prototype/operation.rs#L291) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/error/aggregate.rs:222](../src/engine/builtins/error/aggregate.rs#L222) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/eval.rs:107](../src/engine/builtins/eval.rs#L107) `call_direct_eval_original` | `call_internal` | 旧eval consumer；owned使用prepare+Call |
| [src/engine/builtins/eval.rs:461](../src/engine/builtins/eval.rs#L461) `execute_indirect_string_eval` | `call_internal` | 旧eval consumer；owned使用prepare+Call |
| [src/engine/builtins/function/instance.rs:251](../src/engine/builtins/function/instance.rs#L251) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/function/invoke.rs:241](../src/engine/builtins/function/invoke.rs#L241) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/iterator/collection.rs:388](../src/engine/builtins/iterator/collection.rs#L388) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/iterator/concat.rs:586](../src/engine/builtins/iterator/concat.rs#L586) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/iterator/consume.rs:301](../src/engine/builtins/iterator/consume.rs#L301) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/iterator/from.rs:150](../src/engine/builtins/iterator/from.rs#L150) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/iterator/helper.rs:43](../src/engine/builtins/iterator/helper.rs#L43) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/iterator/step.rs:142](../src/engine/builtins/iterator/step.rs#L142) `finish_next` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/iterator/step.rs:276](../src/engine/builtins/iterator/step.rs#L276) `finish_close` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/iterator/wrap.rs:191](../src/engine/builtins/iterator/wrap.rs#L191) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/json/reviver.rs:558](../src/engine/builtins/json/reviver.rs#L558) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/json/stringify/operation.rs:866](../src/engine/builtins/json/stringify/operation.rs#L866) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/map/callback.rs:201](../src/engine/builtins/map/callback.rs#L201) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/math/sum.rs:210](../src/engine/builtins/math/sum.rs#L210) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/object/iteration.rs:522](../src/engine/builtins/object/iteration.rs#L522) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/object/string.rs:170](../src/engine/builtins/object/string.rs#L170) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/promise/all.rs:20](../src/engine/builtins/promise/all.rs#L20) `call_promise_aggregate` | `call_internal` | S06 |
| [src/engine/builtins/promise/all.rs:20](../src/engine/builtins/promise/all.rs#L20) `call_promise_aggregate` | `call_internal` | S06 |
| [src/engine/builtins/promise/all.rs:453](../src/engine/builtins/promise/all.rs#L453) `finish_promise_aggregate_element` | `call_internal` | S06 |
| [src/engine/builtins/promise/convenience.rs:70](../src/engine/builtins/promise/convenience.rs#L70) `call_promise_try` | `call_internal` | S06 |
| [src/engine/builtins/promise/convenience.rs:70](../src/engine/builtins/promise/convenience.rs#L70) `call_promise_try` | `call_internal` | S06 |
| [src/engine/builtins/promise/convenience.rs:138](../src/engine/builtins/promise/convenience.rs#L138) `call_promise_race` | `call_internal` | S06 |
| [src/engine/builtins/promise/convenience.rs:256](../src/engine/builtins/promise/convenience.rs#L256) `promise_iterator_record` | `call_internal` | S06 |
| [src/engine/builtins/promise/convenience.rs:296](../src/engine/builtins/promise/convenience.rs#L296) `invoke_promise_then` | `call_internal` | S06 |
| [src/engine/builtins/promise/convenience.rs:314](../src/engine/builtins/promise/convenience.rs#L314) `reject_promise_capability` | `call_internal` | S06 |
| [src/engine/builtins/promise/finally.rs:86](../src/engine/builtins/promise/finally.rs#L86) `call_promise_finally_handler` | `call_internal` | S06 |
| [src/engine/builtins/promise.rs:604](../src/engine/builtins/promise.rs#L604) `call_promise_constructor` | `call_internal` | S06 |
| [src/engine/builtins/promise.rs:604](../src/engine/builtins/promise.rs#L604) `call_promise_constructor` | `call_internal` | S06 |
| [src/engine/builtins/promise.rs:878](../src/engine/builtins/promise.rs#L878) `execute_promise_resolve_thenable_job` | `call_internal` | S06 |
| [src/engine/builtins/promise.rs:878](../src/engine/builtins/promise.rs#L878) `execute_promise_resolve_thenable_job` | `call_internal` | S06 |
| [src/engine/builtins/promise.rs:911](../src/engine/builtins/promise.rs#L911) `execute_promise_reaction_job` | `call_internal` | S06 |
| [src/engine/builtins/promise.rs:911](../src/engine/builtins/promise.rs#L911) `execute_promise_reaction_job` | `call_internal` | S06 |
| [src/engine/builtins/promise.rs:1079](../src/engine/builtins/promise.rs#L1079) `call_promise_catch` | `call_internal` | S06 |
| [src/engine/builtins/promise.rs:1146](../src/engine/builtins/promise.rs#L1146) `promise_static_resolve_core` | `call_internal` | S06 |
| [src/engine/builtins/regexp/exec.rs:404](../src/engine/builtins/regexp/exec.rs#L404) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/regexp/replace.rs:1185](../src/engine/builtins/regexp/replace.rs#L1185) `finish_replace` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/set/callback.rs:100](../src/engine/builtins/set/callback.rs#L100) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/set/operations.rs:511](../src/engine/builtins/set/operations.rs#L511) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/string/regexp.rs:423](../src/engine/builtins/string/regexp.rs#L423) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/string/replace.rs:502](../src/engine/builtins/string/replace.rs#L502) `call_string_prototype_replace` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/string/split.rs:251](../src/engine/builtins/string/split.rs#L251) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/builtins/weak_collection/computed.rs:99](../src/engine/builtins/weak_collection/computed.rs#L99) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/object/class_fields.rs:221](../src/engine/object/class_fields.rs#L221) `call_class_instance_initializer` | `call_internal` | 旧class consumer；owned construct_driver已替代调用 |
| [src/engine/object/class_fields.rs:288](../src/engine/object/class_fields.rs#L288) `run_class_static_initializer` | `call_internal` | 旧class consumer；owned construct_driver已替代调用 |
| [src/engine/object/class_fields.rs:340](../src/engine/object/class_fields.rs#L340) `call_class_static_block` | `call_internal` | 旧class consumer；owned construct_driver已替代调用 |
| [src/engine/object/internal_methods/boolean.rs:357](../src/engine/object/internal_methods/boolean.rs#L357) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/object/internal_methods/construct.rs:213](../src/engine/object/internal_methods/construct.rs#L213) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/object/internal_methods/own_keys.rs:398](../src/engine/object/internal_methods/own_keys.rs#L398) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/object/internal_methods/prototype.rs:311](../src/engine/object/internal_methods/prototype.rs#L311) `finish` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/object/internal_methods.rs:155](../src/engine/object/internal_methods.rs#L155) `call_value_internal` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/object/internal_methods.rs:777](../src/engine/object/internal_methods.rs#L777) `proxy_get` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/object/internal_methods.rs:820](../src/engine/object/internal_methods.rs#L820) `internal_set` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/object/internal_methods.rs:923](../src/engine/object/internal_methods.rs#L923) `proxy_set` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/object/internal_methods.rs:1038](../src/engine/object/internal_methods.rs#L1038) `proxy_get_own_property` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/object/internal_methods.rs:1109](../src/engine/object/internal_methods.rs#L1109) `proxy_define_own_property` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/object/internal_methods.rs:1202](../src/engine/object/internal_methods.rs#L1202) `call_proxy` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/object/ordinary.rs:97](../src/engine/object/ordinary.rs#L97) `finish_prepared_read` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/value/conversion/primitive.rs:177](../src/engine/value/conversion/primitive.rs#L177) `finish_primitive_steps` | `call_internal` | 旧同步consumer；对应领域已Owned |
| [src/engine/value/conversion.rs:150](../src/engine/value/conversion.rs#L150) `native_to_number` | `call_internal` | 旧同步consumer；对应领域已Owned |

### S07 当前入口审计（统一验收通过）

- Context 的 call/construct/get/own/define/set 以根请求进入 owned driver；参数及
  wrong-runtime 验证保留，descriptor 结果保持原类型。execute 与 binary 翻译后的
  callable 复用同一入口，外部 code 的验证和发布未旁路。
- 模块 link/evaluate、TLA body、private completion handler、Import 参数转换拥有
  领域 continuation。dynamic load job 在原有宿主调度点依次驱动根操作；无内部 drain。
- FinalizationRegistry cleanup 与动态导入 settler 从作业入口进入根请求。
- 真实 loader/rejection tracker 重入登记使用 delimiter；正常/异常/panic 均由 guard
  清除登记。禁止没有 delimiter 的同 Runtime 嵌套根，防止内部同步调用伪装成新执行。
- 时钟和时区 HostServices 仍为原有禁止重入的 infallible 值服务。未新增 host ABI。
- 默认配置及旧消费者保留到 S10。`stack-vm` 已显式转发至 native/web/Test262；
  CLI、Test262 和 WASM 的完整两配置验收通过，证据见逐 commit 计划及迁移清单。

S07 正式门禁发现并修复的调用点：tagged-template 的 Object 常量曾令
`PushConst` 进入旧整帧交接；现通过 `pure_operations::load_value_constant`
共用原 host 的类型检查与持根规则，再由 owned 冷操作压栈。模板身份、raw/cooked、
GC 与 optional-chain member tag 的原 oracle 预期保留，完整门禁复核通过。

同次 Test262 门禁发现 ReadValue 的 nullish 准备错误越过 Promise continuation；
现与原同步读取共用 TypeError → Throw 回复转换。四种组合方法的 16 项原失败
均通过，两配置完整向量逐字节对齐冻结基线。最终 owned/default 库 2212/2035、
常规 oracle 各 911 加压力各 1、完整 Test262 各 79982 pass、两配置 Node/WASM
及 726 个边界反例均通过。本轮失败、修复和最终证据统一保存在
`target/primitive-vm-s07-acceptance/`；完整 benchmark/profile 留待本阶段 commit 后。
