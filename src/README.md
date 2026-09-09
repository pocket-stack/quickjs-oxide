# 源码布局

顶层只保留 lib.rs 作为 Rust crate 入口，代码分别归 source、regexp 和 engine。

Rust 嵌入入口为 engine::api；engine 的其余职责模块仅在 crate 内可见。source、regexp 提供可复用组件，具体宿主实现位于仓库 adapters。旧模块路径不再保留。

## 文件与子目录

- [engine/](engine/README.md)：子模块职责与文件说明。
- [lib.rs](lib.rs)：A pure-Rust rewrite of `QuickJS` aiming at semantic feature parity with the。
- [regexp/](regexp/README.md)：子模块职责与文件说明。
- [source/](source/README.md)：子模块职责与文件说明。
