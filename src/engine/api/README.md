# Rust 嵌入边界

导出 Runtime/Context、公共错误和受根保护的句柄；负责显式宿主注入及上下文操作。

公开入口为 `quickjs_oxide::engine::api`；不再提供根模块别名。共享资源所有权位于 heap/runtime，语言算法位于对应职责目录。

调用方从这里导入 Runtime、Context、Value、HostServices 等嵌入契约；源码坐标从 `quickjs_oxide::source` 导入。编译器、堆、VM 及字节码内部类型不作为嵌入 API 开放。

## 文件与子目录

- [context/](context/README.md)：子模块职责与文件说明。
- [error.rs](error.rs)：error 的类型和操作实现。
- [mod.rs](mod.rs)：模块入口、共享接口与子模块声明。
- [runtime.rs](runtime.rs)：运行时操作或共享所有权入口（见本目录边界）。
- [runtime_error.rs](runtime_error.rs)：运行时错误边界。
- [test262_agent.rs](test262_agent.rs)：QuickJS-shaped Test262 `$262.agent` host.。
- [test262_host.rs](test262_host.rs)：QuickJS-shaped Test262 realm host helpers.。

- [testing.rs](testing.rs)：仅由 test-support 特性开放的词法与数值差分测试观察入口，不属于生产嵌入 API。

## 模块加载契约

`ModuleLoader` 只有 `normalize`、`check_attributes` 和 `load` 三个回调，均接收发起请求的 `&mut Context`。`load` 同时接收模块名与导入属性，直接返回 `ModuleLoadResult`；不再提供旧的仅字符串加载回调或逐层适配方法。默认名称规范化和空属性校验仍提供实际默认行为。
