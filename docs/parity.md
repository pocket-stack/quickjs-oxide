# QuickJS 语义 Feature Parity 契约

本文是 quickjs-oxide 的完成定义。它约束实现方向、测试证据和对外声明；阶段计划、当前测试数量或某个 demo 的可运行性都不能覆盖本文。

## 1. 目标与固定基线

目标是用 Rust 独立重写 QuickJS，并与 **QuickJS 2026-06-04** 达成语义和产品表面的 feature parity。这里的“parity”不是“能执行一部分 JavaScript”，而是同一输入在语言语义、宿主能力、嵌入 API、命令行、模块、序列化和生命周期上具有与该版本相同的可观察行为。

基线必须固定为官方 `quickjs-2026-06-04` 发布源码，而不是滚动的仓库 `master`、quickjs-ng、其他 fork 或系统里碰巧安装的 `qjs`。基线的关键锚点是：

- `VERSION` 为 `2026-06-04`；
- 字节码版本是 `quickjs.c` 中的 `BC_VERSION = 5`；
- Unicode 数据版本是 `libunicode.h` 中的 `17.0.0`；
- Test262 固定为 `Makefile` 中的提交 `5c8206929d81b2d3d727ca6aac56c18358c8d790`，并应用 `tests/test262.patch`；
- Test262 的启用/跳过范围以该发布包的 `test262.conf`、`test262o.conf` 为准，已知结果以 `test262_errors.txt`、`test262o_errors.txt` 为准。发布包当前分别记录 58 行和 0 行已知错误；这个行数只能做完整性校验，不能替代逐测试结果比对。

在 CI 接受任何 parity 结果前，仓库必须记录官方源码包的来源、SHA-256 和解包后文件清单。测试报告也必须带上基线 SHA-256、目标三元组、编译选项和 Test262 提交，避免同名版本漂移。

本契约复刻的是该版本实际提供的范围，不把“ES2025”当作一个会随时间变化的目标。上游文档明确未支持 proper tail calls、`Atomics.waitAsync`，也未实现 ECMA-402 Intl；`test262.conf` 中标记为 `skip` 的其他特性同样不在本基线的必达范围。新增这些能力可以另行立项，但不能用来抵消基线内的缺失或不兼容。

## 2. 纯 Rust 边界与 oracle 规则

所有交付给用户的产品实现都必须是 Rust：解释器、编译器、GC、RegExp、Unicode、BigInt、标准库、Worker、CLI、静态库/动态库和 C ABI 适配层均由 Rust 编译产生。允许经过审计的 `unsafe` 和 Rust 的系统调用绑定；“纯 Rust”不等于强制 `#![forbid(unsafe_code)]`。

以下行为禁止出现在产品构建或运行路径中：

- 编译、链接或 vendoring `quickjs.c`、`libquickjs.a`、其他 QuickJS fork，或通过 FFI 把它当作实际引擎；
- 运行时启动 `qjs`、Node.js、JavaScriptCore、V8 等外部解释器来完成解析或执行；
- 把外部引擎隐藏在 build script、动态库、可选 feature、WASM blob 或生成代码中；
- 在 Rust API 后面仅包一层现有 QuickJS binding。

官方 C 版 `qjs` 只允许存在于测试 oracle 路径：构建基线、生成 golden、执行差分和确认上游已知行为。测试用的 C embedder/native-module fixture，以及 `qjsc -c/-e` 按兼容契约生成给用户的 C 源文件，不属于产品引擎实现，因而可以使用 C；它们必须链接到 Rust 产出的库，而不能反向带入 C 引擎。

实现必须参考上游算法、状态机和不变量，而不是只参考语法示例。允许把上游逻辑重新表达为合适的 Rust 数据结构；不要求逐行翻译，也不要求内部地址、对象布局或优化完全相同，除非它们通过 C ABI、内存统计、终结时机或字节码格式变成可观察行为。每个核心模块应在模块文档或 source map 中注明对应的上游文件/符号，方便逐项审计。

## 3. 可运行骨架必须走最终路径

第一版解释器可以只覆盖很窄的语法，但必须是最终架构上的纵向切片。推荐的最小真实路径是：

1. 按 `qjs.c::eval_file` / `eval_buf` 建立参数、脚本/模块判定和错误出口；
2. 按 `JS_NewRuntime`、`JS_NewContext` 建立隔离的 heap/realm、atoms、intrinsics 和异常槽；
3. 按 `next_token`、`js_parse_*` 和 `JS_EvalInternal` 完成词法、作用域分析并直接生成 `quickjs-opcode.h` 所映射的栈式字节码；
4. 按 `JS_CallInternal` 的调用帧和栈效应执行字节码，正确传播 completion/exception；
5. 经 `JS_ExecutePendingJob` 排空 job，最后沿 `JS_FreeContext` / `JS_FreeRuntime` 释放资源。

骨架从一开始就应使用真实的 `Value`、atom、string、object/shape、property descriptor、environment/closure、bytecode function 和 exception 表示；尚未实现的语法应明确报错。临时 AST evaluator、把所有值都降成 JSON、跳过 property descriptor、用 Rust panic 表示 JS throw，或调用 oracle 得到答案，都不是可保留的解释器骨架。

## 4. 上游组件到 Rust 实现的审计映射

下表不是要求复制 C 的文件布局，而是要求 Rust 实现覆盖同一责任和关键不变量。

| 领域 | 上游锚点 | Rust 实现必须覆盖 |
| --- | --- | --- |
| runtime、realm、值和对象 | `quickjs.c` 的 `JSRuntime`、`JSContext`、`JSObject`、`JSShape`，`quickjs.h` 的 `JSValue`/tag 与 property flags | runtime/context 隔离、值转换、atoms/symbols、原型、shape/property descriptor、exotic object、异常槽和所有权 |
| lexer、compiler、VM | `next_token`、`js_parse_*`、`JS_EvalInternal`、`JSFunctionBytecode`、`quickjs-opcode.h`、`JS_CallInternal` | script/module grammar、作用域/闭包、栈效应、调用/构造、completion、debug/line info、优化后仍等价 |
| intrinsics | `JS_NewContext`、`JS_NewContextRaw`、`JS_AddIntrinsic*` | 基础对象以及 Date、eval、normalize、RegExp、JSON、Proxy、Map/Set、TypedArray、Promise、WeakRef 的可选择安装语义 |
| 数字和 BigInt | `dtoa.c` 的 `js_dtoa`，`quickjs.c` 的 `js_atof`、`js_bigint_*`、`JS_TAG_SHORT_BIG_INT` | IEEE-754 转换/格式化、`-0`/NaN/Infinity、任意精度二补数运算及小 BigInt 快路径的语义 |
| RegExp | `libregexp.c` 的 `lre_compile` / `lre_exec`、`libregexp-opcode.h` | ECMAScript RegExp parser、bytecode、显式回溯栈、captures、Unicode modes 和 API 行为 |
| Unicode | `libunicode.c` / `libunicode-table.h`、`unicode_normalize`、`unicode_script`、`unicode_prop` | Unicode 17.0.0 的 identifier、case、normalization、script/category/binary/string properties |
| modules | `js_resolve_module`、`JS_ResolveModule`、`JS_LoadModule`、module namespace exotic methods | parse/link/evaluate、live binding、cycles、TLA、dynamic import、`import.meta`、attributes 和 loader callbacks |
| GC/生命周期 | `JS_DupValue` / `JS_FreeValue`、`free_zero_refcount`、`gc_decref`、`gc_scan`、`gc_free_cycles`、`JS_RunGC` | 确定性引用计数、循环回收、weak objects、finalizer/mark、内存限制和公开所有权规则 |
| object serialization | `JS_WriteObject(2)`、`JS_ReadObject`、`JS_EvalFunction`、`BC_VERSION` | 函数/模块字节码、BJSON、byte swap、SAB、共享引用/循环图、source/debug stripping |
| host library/event loop | `quickjs-libc.c`、`js_std_loop`、`js_std_await`、`js_module_loader` | `std`/`os`、global helpers、文件/进程/信号/定时器/handlers、promise rejection 和事件循环 |
| Worker | `JSWorkerMessagePipe`、`worker_func`、`js_worker_postMessage` | 每线程独立 runtime/context、FIFO 消息、structured clone、SAB 共享、生命周期及上游限制 |
| CLI/compiler | `qjs.c`、`repl.js`、`qjsc.c` | `qjs`/REPL 的完整行为，以及 `qjsc` 的 C 源/可执行输出、feature stripping 和 module 收集 |

## 5. 全局完成门禁

只有以下条件同时成立，项目才可以声明“QuickJS 2026-06-04 feature parity”：

1. 本文各分区的门禁均有同一提交上的可复现报告，且没有未分类的失败、skip、timeout、crash 或差分；
2. 产品依赖审计证明运行和发布产物不包含任何外部 JS 引擎；在移除 oracle 工具和网络后，产品仍可构建并运行；
3. oracle 与 Rust 引擎的测试必须在独立进程、相同 fixture、固定环境下运行，测试不得根据被测引擎改变语义；
4. 所有允许差异都进入版本化 [deviation ledger](deviations.md)，记录最小复现、上游输出、Rust 输出、理由、兼容影响和批准者。没有记录的“行为更合理”仍是失败；
5. 测试报告必须保存逐用例结果和命令，而不只保存 pass rate；任何过滤、重试、更新 golden 或扩大 timeout 都必须在 diff 中可见；
6. 支持平台矩阵逐项通过同一套语义门禁。未验证平台可以标为 unsupported，但不能宣称该平台 parity；
7. release 构建在 OOM、stack limit、interrupt、恶意深层对象图、复杂 RegExp 和循环模块等压力用例中不 panic、abort、死锁或越界。超时本身是失败，除非 oracle 在相同限制下也产生同类结果并已登记。

性能和体积不是本文的主要等价判据，不要求复现内部地址或逐指令优化；但慢到使上游功能用例、Test262 或 worker/event-loop 门禁无法在约定预算内结束，仍属于功能不可用。

## 6. 语言与内建对象门禁

语言核心必须覆盖基线 `test262.conf` 启用的全部语法和语义，包括 strict/sloppy、Annex B、lexical/var environment、closures、direct/indirect eval、classes/private elements、`this`/`super`/`new.target`、destructuring、generators、async/await、iterators、exceptions、Proxy、Symbols、property order、Promises/jobs、WeakRef/FinalizationRegistry、ArrayBuffer/TypedArray/SharedArrayBuffer/Atomics 等。

门禁是：

- 官方 `Makefile::test` 中所有无修改 JS 文件都由 Rust `qjs` 通过：`test_closure.js`、`test_language.js`、`test_builtin.js`、`test_loop.js`、`test_bigint.js`、`test_cyclic_import.js`、`test_worker.js`、`test_std.js`、`test_rw_handler.js`；
- shared-library 配置下的 `test_bjson.js` 和 `examples/test_point.js` 也必须通过，并用于证明 native module/C API 路径，不得简单从测试清单移除；
- 每个 opcode 都有栈效应、异常边和控制流测试；parser/bytecode/VM 的覆盖报告能回溯到 `quickjs-opcode.h` 的 opcode 清单，未实现 opcode 不允许被当作 unreachable；
- 对 coercion、property descriptor、prototype/exotic methods、enumeration order、cross-realm、exception completion 和 job ordering 建立独立回归集，因为“程序最终打印相同”不足以证明中间语义。

允许 Rust 内部采用不同优化，但不能把 Proxy、eval、with、Annex B 或 WeakRef 等困难能力降级成 stub 或永久 feature flag。`qjsc -fno-*` 的裁剪模式是上游产品能力，不能被误解成默认引擎可缺少这些能力。

## 7. Test262 门禁

Test262 必须由仓库脚本一键从固定提交准备，应用上游 `tests/test262.patch`，并分别运行现代套件和旧 ES5.1 套件。门禁至少等价于上游的 `make test2-check`、`make test2` 和 `make test2o` 路径。

结果判定采用逐测试 outcome vector，而不是总通过率：

- Rust 不能比官方 oracle 多任何 fail、skip、timeout、crash 或 harness error；
- 对 baseline 的已知失败，Rust 若通过可以保留，但必须作为“修复的上游差异”进入 deviation ledger，确保不是 harness 没有真正执行；
- 对 oracle 和 Rust 都通过的测试，strict/noStrict、module、async 等运行变体必须一致；
- `test262.conf` 的 feature skip 是基线范围说明，不能自行新增 skip、按目录排除、忽略 stderr 或把 crash 记作预期失败；
- CI 保存配置文件哈希、Test262 提交、patch 哈希、逐用例结果和汇总。只展示“99.x%”不能作为证据。

Test262 证明规范覆盖，但不单独证明 QuickJS parity；CLI、C API、BJSON/bytecode、`std`/`os`、Worker 和 QuickJS 特有诊断仍由后续门禁证明。

## 8. 差分门禁

差分 runner 必须把完全相同的输入分别交给官方 `qjs` 和 Rust `qjs`，比较：

- parse/early-error 的接受或拒绝、错误种类和发生阶段；
- 正常值、property descriptors、own-key 顺序、原型和可观察 side-effect trace；
- stdout、stderr、exit status、uncaught exception 类型/消息/stack，以及 Promise rejection；
- module link/evaluate 顺序、dynamic import/TLA、job 和 timer/message 顺序；
- compile/serialize/read/evaluate 的跨引擎结果。

路径、地址、wall clock、随机数、线程调度等非确定字段只能通过版本化的最小 normalization 规则处理，不能整段丢弃 stderr 或 stack。测试环境固定 locale、timezone、cwd、环境变量和资源限制；需要时间/随机性的用例使用显式 fixture，无法稳定化的用例单独分类。

语料至少包含：全部上游 tests、Test262 的代表性缩减集、每个公开 API 的 probe、每个 opcode/builtin 的定向用例、历史回归、以及可复现 seed 的 grammar/property/bytecode/object-graph fuzz。fuzzer 发现的差异必须自动缩减并落入回归集。固定跑若干 happy-path 示例不构成差分门禁。

## 9. CLI 与 REPL 门禁

Rust `qjs` 必须兼容 `qjs.c` 的实际参数解析，而不仅是文档里最常用的选项。需覆盖 `-e/--eval`、`-i/--interactive`、`-m/--module`、`--script`、`-I/--include`、`--std`、`--strict`、`-d/--dump`、`-T/--trace`、`-q/--quit`、`--memory-limit`、`--stack-size`、`-s`、`--strip-source`、`--no-unhandled-rejection`、`--` 和脚本参数。

门禁逐项比较：

- option grouping、缺参/未知参数、help/version 文本、stdout/stderr 和 exit code；
- `.mjs` 与 `JS_DetectModule` 自动判定、显式 script/module、include 顺序、`scriptArgs`；
- `print`、`console.log`、`performance.now`、`__loadScript`、`std`/`os` 注入；
- REPL 的 multiline、completion/printing、exception recovery、top-level await 和 EOF/TTY 行为，对照 `repl.js`；
- memory/stack/interrupt/unhandled-rejection 行为及 `-d/-T` 中稳定可比较的字段。

Rust `qjsc` 必须覆盖 `qjsc.c` 的 `-c`、`-e`、`-o`、`-N`、`-m`、`-D`、`-M`、`-x`、`-p`、`-S`、`-s`、`--keep-source`、`-flto` 和所有 `-fno-*` 模式。生成的 C 不要求文本逐字相同，但必须遵守相同的 symbol/initialization 契约并能链接 Rust runtime；默认生成的可执行文件及 module/native-module/worker 收集行为必须与基线等价。

## 10. 嵌入 API 与 C ABI 门禁

Rust-first API 可以更符合 Rust 习惯，但它不能代替 QuickJS 的嵌入表面。必须由 Rust `staticlib`/`cdylib` 提供对 `quickjs.h` 和 `quickjs-libc.h` 的 source/ABI-compatible facade，并建立从头文件自动抽取的公开符号清单。每个函数、类型、常量、flag、callback 和 inline/macro 行为都必须标为 implemented、platform-specific 或 baseline-unsupported；“没有被当前 demo 调用”不是跳过理由。

门禁包括：

- `JSRuntime`/`JSContext` 生命周期和多 context 同 runtime 的对象共享边界；
- `JSValue`、tag、常量与调用约定，`JS_DupValue`/`JS_FreeValue` 的 ownership，以及 exception sentinel/`JS_GetException`；
- atoms/strings/UTF-8-CESU-8 转换、numeric/BigInt coercion、objects/arrays/property descriptors/prototypes/enumeration；
- `JS_Call*`、`JS_Eval*`、JSON、ArrayBuffer/TypedArray/SAB、Promise/job、interrupt、memory allocator/limit、class/exotic/finalizer/gc_mark；
- module loader/C module、`JS_WriteObject*`/`JS_ReadObject`、print/memory usage，以及 `quickjs-libc.h` 的 helpers/event loop；
- upstream `examples/fib.c`、`examples/point.c`、`tests/bjson.c` 及额外 ownership/error fixture 在不改业务逻辑的情况下，针对 Rust header/library 编译、链接并通过；
- C ABI sanitizer/valgrind 类测试和 Rust Miri/并发模型能覆盖跨边界 use-after-free、double-free、panic unwind、callback re-entry 与 allocator failure。

若某个平台的 C ABI 尚未验证，只能声明该平台的 Rust API 可用，不能声明完整 QuickJS API parity。

## 11. 字节码、BJSON 与 `qjsc` 门禁

QuickJS 自己声明字节码与具体版本绑定且不应加载不可信输入。本项目只承诺与 **2026-06-04 / BC_VERSION 5** 互操作，不把该格式宣传为长期稳定或安全的持久化格式。

门禁是双向的：

- Rust 能读取、resolve 并执行官方 `JS_WriteObject`/`qjsc` 生成的 scripts、functions、closures 和 modules；
- 官方基线能读取并执行 Rust 生成的等价对象；若输出字节不相同，语义、metadata 和再序列化必须等价；
- `JS_WRITE/READ_OBJ_BYTECODE`、`BSWAP`、`SAB`、`REFERENCE`、ROM-data 读取、循环/共享对象图均有正反向 fixture；
- source/debug stripping、filename/line table、stack trace、atom table、closure var refs、module imports/attributes 和 JSON module 数据跨引擎保持；
- malformed/truncated/wrong-version 输入与 oracle 产生相同类别的失败，且 Rust 不 panic 或越界；这不改变“仅信任字节码”的产品警告；
- `tests/test_bjson.js` 覆盖的 Date、boxed primitives、TypedArray/ArrayBuffer、共享引用和 cycle 必须交叉读取验证，而不只是 Rust 自己 round-trip。

只做到“本引擎能读取自己生成的私有格式”，或让 `qjsc` 退化成打包源码，都不算 bytecode parity。

## 12. `std`、`os` 与事件循环门禁

`quickjs-libc.c` 是基线的一部分。`std` 模块的环境变量、扩展 JSON、文件/stdio、printf、popen、load/eval、error constants 等，以及 `os` 模块的 fd I/O、TTY、filesystem、signals、process、pipe、time/timer、async handlers 和平台常量，都必须按 `js_std_funcs`、`js_std_file_proto_funcs`、`js_os_funcs` 的导出清单逐项盘点。

门禁包括：

- 导出名、arity、property flags、常量、错误码和 platform conditional 与 oracle 一致；
- `js_module_loader` 的相对路径、system module、JSON import attribute、native `.so` module 行为一致；
- `js_std_loop` 同时驱动 Promise jobs、fd handlers、timers、worker ports，并保持 queue ordering 和退出条件；
- `js_std_await`、unhandled rejection tracking、异常打印和 async 错误出口一致；
- `tests/test_std.js`、`test_rw_handler.js`、进程/信号/文件/定时器的隔离 integration tests 在每个支持平台通过，无权限的 CI 能力必须有明确的平台 runner，而不是 skip 后仍宣布 parity。

只实现 `print` 和文件读取，或把 `std`/`os` 永久定义为“宿主自行提供”，不符合 QuickJS CLI feature parity。

## 13. GC、WeakRef 与资源门禁

为了保持 C API ownership 和可观察的 finalization，默认生命周期模型应以 QuickJS 的引用计数加 cycle removal 为设计基准，而不是依赖 Rust 所有权自动“碰巧”释放。Rust 可以改变内部容器，但必须保留：

- `Dup/Free` 的确定性引用计数效果，零引用对象释放和递归释放队列；
- `gc_decref -> gc_scan -> gc_free_cycles` 所表达的 cycle 判定与 `JS_RunGC` 行为；
- class finalizer、`gc_mark`、opaque resource、weak refs/finalization jobs 和 SAB backing store 的关系；
- finalizer 中不能执行 JS 等上游约束、context/runtime teardown 次序，以及 allocator failure/limit/threshold；
- 共享 shape、closures/var refs、async frames、modules、Promises 和 worker message 中的 cycle 不泄漏也不提前释放。

门禁使用确定性 drop probes、带 finalizer 的 C/Rust classes、随机循环对象图、WeakRef/FinalizationRegistry、反复创建销毁 runtime/context、OOM fault injection 和长时间 leak test。测试必须同时检查“最终释放”和“释放时机/次数”；只看进程退出后 OS 回收内存不能证明 GC parity。

## 14. Unicode 门禁

Unicode 行为固定在 17.0.0。门禁覆盖：

- UTF-8 输入、UTF-16 code units、lone surrogates、C API 的 UTF-8/CESU-8 转换和字符串索引；
- identifier start/continue、case conversion/folding、locale-independent casing；
- NFC/NFD/NFKC/NFKD normalization；
- script/script extensions、general category、binary properties、Unicode sequence properties；
- RegExp `u`/`v` 模式、property escapes、word boundary/canonicalization 与 astral code points。

除 Test262 外，表驱动测试应遍历 Unicode 17 数据的所有 code point/range，并与 `libunicode.c` oracle 比较；生成表必须记录数据来源、版本和校验和。使用宿主 OS/ICU 的“最新 Unicode”而不锁版本，即使大多数测试通过，也不是 parity。

## 15. RegExp 门禁

RegExp 必须实现 `libregexp.c` 对应的完整 parser/compiler/executor，而不是转发给 Rust `regex` crate，因为后者通常不具备 ECMAScript backreference、lookaround、lastIndex 和回溯语义。

门禁覆盖基线接受的全部 flags/grammar、named/numbered captures、backreferences、lookahead/lookbehind、greedy/lazy quantifiers、sticky/global `lastIndex`、indices、Unicode properties/sets、replace/split/search/match/matchAll 和空匹配推进。生成式测试同时比较 compile error、capture spans、结果数组的 properties 和状态变化。

回溯使用显式栈并受 runtime stack/memory/interrupt 控制，参照 `lre_exec` 和 `libregexp-opcode.h`；复杂或灾难性输入不能造成 Rust stack overflow、panic 或无法中断。只通过简单字面量、或依赖 Test262 没覆盖到某个回溯顺序，不能算 RegExp parity。

## 16. Number 与 BigInt 门禁

Number 门禁对照 `dtoa.c`/`js_atof`，覆盖二进制 IEEE-754 边界、十进制最短表示、radix、舍入、subnormal、`-0`、NaN/Infinity、numeric literal 和 String/JSON 转换。不得把 Rust 标准格式化输出未经证明地视为等价。

BigInt 门禁对照 `js_bigint_*` 的任意精度二补数语义和 short-BigInt normalization，覆盖：

- 各 radix 解析/格式化、正负边界和超大输入/OOM；
- `+ - * / % ** << >> & | ^ ~`，除零、负指数和 shift 边界；
- 与 Number/String/Boolean 的 coercion、abstract/strict equality 和 relational comparison；
- `BigInt.asIntN/asUintN`、BigInt typed arrays、DataView、Atomics；
- C API `JS_NewBigInt64`/`JS_NewBigUint64`/`JS_ToBigInt64` 及 bytecode/BJSON round-trip。

除 upstream tests/Test262 外，使用独立任意精度模型生成可复现随机向量，并对 oracle、Rust 和模型三方比较。固定 64/128 位整数实现不可能满足此门禁。

## 17. Module 门禁

module parser/linker/evaluator 必须覆盖 static import/export、live bindings、namespace exotic object、re-export/star ambiguity、cycles、dynamic `import()`、top-level await、`import.meta`、JSON modules/import attributes 和 host normalize/load/check-attributes callbacks。

门禁需要比较 parse、resolve、instantiate、evaluate 四个阶段的错误类型和顺序，并覆盖：

- 同一 module identity/cache、relative/system/native module resolution；
- sync/async module graph、diamond/cycle、TLA fulfillment/rejection 与 job ordering；
- namespace own keys/descriptors/immutability 和 live binding 更新；
- `JS_EVAL_FLAG_COMPILE_ONLY`、`JS_ResolveModule`、`JS_EvalFunction`、`JS_GetImportMeta/ModuleName/ModuleNamespace`；
- CLI autodetection、`qjsc -D/-M`、bytecode module、worker 相对路径。

把每个文件当独立 script 拼接执行，或仅支持无环 static import，不是 module parity。

## 18. Worker 门禁

`os.Worker` 必须保持上游模型：worker 使用独立线程、独立 `JSRuntime`/`JSContext` 和 module entry；父子通过 FIFO pipe/port 通信。`postMessage` 按 `JS_WRITE_OBJ_SAB | JS_WRITE_OBJ_REFERENCE` structured-clone 普通对象图，同时保持 SharedArrayBuffer backing store 共享，而不是共享普通 JS object。

门禁覆盖：

- `new os.Worker(relative_module)` 的调用方基准路径、module loader 和启动错误；
- `Worker.parent`、`postMessage`、`onmessage` set/get/null、event `{ data }` shape 和消息顺序；
- primitives、BigInt、ArrayBuffer/TypedArray、循环/共享引用图、SAB+Atomics 的跨线程行为；
- worker 异常/unhandled rejection、父子提前退出、队列 drain、资源释放和大量并发消息；
- 上游明确禁止 worker 内再创建 worker 的行为及错误；
- `tests/test_worker.js`/`test_worker_module.js` 不修改通过，并有 stress/GC/teardown/TSAN 类测试证明无丢消息、死锁和 use-after-free。

用同线程 task 模拟而改变隔离/阻塞语义，或只支持 JSON message，不是 Worker parity。

## 19. 实现顺序

以下顺序用于降低返工，不代表某一阶段完成后已经 parity：

1. **锁定证据**：引入官方基线 metadata、oracle builder、差分协议、逐测试结果格式和 source-map 清单。
2. **真实核心骨架**：实现 runtime/context、Value、atoms/strings、object/shape/property、异常，以及 lexer -> bytecode compiler -> stack VM 的最小纵向路径；让 Rust `qjs -e` 走最终管线。
3. **语言基础闭环**：作用域/closure、call/construct、control flow、function/class、eval、exceptions，逐步覆盖 `test_language`、`test_closure`、`test_loop`，同步补 opcode tests。
4. **对象模型与 intrinsics**：完成 coercion、descriptor/prototype/exotic、Array、Symbol、Date、JSON、Map/Set、Proxy、TypedArray 等，再扩展到 generators、Promises/jobs、async 和 WeakRef。
5. **精确子系统**：按上游逻辑实现 dtoa/number、BigInt、Unicode 17 和 RegExp，并各自建立生成式差分门禁。
6. **modules 与序列化**：实现完整 module graph/TLA/import attributes，再完成 BC_VERSION 5、BJSON 和双向 cross-read。
7. **生命周期与嵌入**：闭合 refcount/cycle GC、limits/interrupts、完整 Rust API 和由 Rust 导出的 QuickJS C ABI，跑通 upstream C examples/native modules。
8. **宿主产品面**：实现 `std`、`os`、event loop、native loader 和 Worker，再补齐 `qjs`/REPL/`qjsc` 的全部选项与输出模式。
9. **收口而非改名**：跑完整 upstream tests、Test262/ES5.1、差分/fuzz、平台/ABI/压力矩阵，清零或批准所有 deviation 后才发布 parity 声明。

每一阶段只能报告具体 coverage，例如“`test_closure.js` 通过”或“已实现 37/NN 个 opcode”；禁止把“可运行”“MVP”“核心语法完成”“Test262 某通过率”改写成 feature parity。

## 20. 明确不能算完成的情况

出现以下任一情况，都不能宣布 parity：

- 只能运行 hello world、算术、JSON 或某个 benchmark；
- 解析器覆盖大部分语法，但 VM、异常、descriptor、module phase 或 job ordering 是近似实现；
- Test262 通过率很高，但提交/patch/config 不固定，新增了 skip，或没有逐测试结果；
- upstream JS tests 通过，却跳过 native module、C API、`std`/`os`、Worker、BJSON/bytecode、REPL 或 `qjsc`；
- 只提供 idiomatic Rust API，不提供 `quickjs.h`/`quickjs-libc.h` 兼容面；
- bytecode 只是自有格式、自身 round-trip，不能与 2026-06-04 双向互读；
- RegExp、Unicode、BigInt、GC 或 Worker 由 stub、固定宽度近似、宿主库的漂移版本或外部进程代办；
- 产品包仍含 QuickJS C 实现，或在运行时需要 oracle；
- 为了让 CI 变绿修改上游测试含义、只比较 stdout 的一部分、吞掉异常、无限重试或扩大排除集；
- 已知差异没有最小复现和 deviation 记录；
- 某个平台、feature flag 或 release profile 没跑门禁，却对其做无条件 parity 宣称；
- 阶段性架构是一次性 toy interpreter，后续仍计划“换成真正的 QuickJS 模型”。

最终判定只看可复现证据是否覆盖本契约全部门禁，不看实现投入、代码行数、计划完成比例或主观相似度。
