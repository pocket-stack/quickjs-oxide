# 共享运行时所有权

维护 RuntimeInner/RuntimeState、堆和 atom 域、延后释放及清理。公开 Runtime 句柄定义在 api/runtime.rs。

其他模块在各自目录为 Runtime 实现操作；这里仅维护自身拥有的状态类型和运行时集成测试，不再转导出其他职责模块的类型。

## 文件与子目录

- [mod.rs](mod.rs)：模块入口、共享接口与子模块声明。
- [tests/](tests/README.md)：子模块职责与文件说明。
- [tests.rs](tests.rs)：模块回归测试。
