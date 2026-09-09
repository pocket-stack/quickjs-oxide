# 语言内置行为

实现 ECMAScript 内置对象与函数，以及 qjs 兼容辅助行为；拥有原生调用选择器与分派。

使用 value 转换、object 语义和 VM 调用；环境能力通过 host 契约注入。

## 文件与子目录

- [array/](array/README.md)：子模块职责与文件说明。
- [array.rs](array.rs)：Array constructor, prototype, iterator, and sorting intrinsics.。
- [array_buffer/](array_buffer/README.md)：子模块职责与文件说明。
- [array_buffer.rs](array_buffer.rs)：`%ArrayBuffer%` backing-store, constructor, resize, detach, and transfer.。
- [atomics/](atomics/README.md)：子模块职责与文件说明。
- [atomics.rs](atomics.rs)：Pinned QuickJS `%Atomics%` operations over integer TypedArrays.。
- [buffer_access.rs](buffer_access.rs)：Borrow-free access tokens for ArrayBuffer-family backing stores.。
- [date/](date/README.md)：子模块职责与文件说明。
- [dispatch.rs](dispatch.rs)：指令或原生调用分派。
- [error/](error/README.md)：Error 系列内置行为、错误对象构造与堆栈生成。
- [eval.rs](eval.rs)：eval 的类型和操作实现。
- [function.rs](function.rs)：function 的类型和操作实现。
- [iterator/](iterator/README.md)：子模块职责与文件说明。
- [json/](json/README.md)：子模块职责与文件说明。
- [map.rs](map.rs)：`%Map%`, Map Iterator, and strong ordered-record semantics.。
- [math/](math/README.md)：子模块职责与文件说明。
- [math.rs](math.rs)：Pinned QuickJS `Math` intrinsic algorithms.。
- [mod.rs](mod.rs)：模块入口、共享接口与子模块声明。
- [native.rs](native.rs)：原生内置函数选择器、调用协议与描述符。
- [object/](object/README.md)：子模块职责与文件说明。
- [object.rs](object.rs)：Object constructor and prototype intrinsics.。
- [primitive.rs](primitive.rs)：primitive 的类型和操作实现。
- [promise/](promise/README.md)：子模块职责与文件说明。
- [promise.rs](promise.rs)：`%Promise%`, resolving functions, and reaction semantics.。
- [proxy.rs](proxy.rs)：`%Proxy%` allocation and revocation lifecycle.。
- [qjs_host.rs](qjs_host.rs)：Optional qjs command-line host functions.。
- [qjs_value_printer.rs](qjs_value_printer.rs)：Side-effect-free value rendering used by the optional qjs host.。
- [reflect/](reflect/README.md)：子模块职责与文件说明。
- [reflect.rs](reflect.rs)：Pinned QuickJS `Reflect` intrinsic algorithms.。
- [regexp/](regexp/README.md)：子模块职责与文件说明。
- [replacement.rs](replacement.rs)：Shared replacement-template expansion for String and RegExp intrinsics.。
- [set.rs](set.rs)：`%Set%`, Set Iterator, and the proposal-era Set methods shipped by QuickJS.。
- [shared_array_buffer.rs](shared_array_buffer.rs)：`%SharedArrayBuffer%` constructor, grow, slice, and shared-backing bridge.。
- [string/](string/README.md)：子模块职责与文件说明。
- [string.rs](string.rs)：String prototype intrinsics beyond the shared primitive-wrapper substrate.。
- [uri.rs](uri.rs)：QuickJS-compatible URI and legacy escape codecs.。
- [weak_collection.rs](weak_collection.rs)：`%WeakMap%` / `%WeakSet%` and QuickJS-compatible weak-key behavior.。
- [weak_ref.rs](weak_ref.rs)：`%WeakRef%` / `%FinalizationRegistry%` and pinned QuickJS weak-target semantics.。
