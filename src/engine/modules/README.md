# 模块实例与执行

负责模块加载接口、实例、连接、命名空间、求值、动态导入和顶层 await。

应用提供来源与加载策略；code 拥有模块草稿，heap 保存记录，jobs 驱动延后完成。

## 文件与子目录

- [mod.rs](mod.rs)：模块入口、共享接口与子模块声明。
- [namespace.rs](namespace.rs)：ECMAScript Module Namespace Exotic Object storage helpers.。
- [tests/](tests/README.md)：子模块职责与文件说明。
- [tests.rs](tests.rs)：模块回归测试。
