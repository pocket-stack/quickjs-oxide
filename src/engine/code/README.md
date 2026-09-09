# 代码与发布

拥有指令、常量、函数/模块代码、位置与异常信息，以及代码校验、发布和私有字节码解码。

编译器生成草稿；发布过程验证身份并取得 roots；VM 消费经过验证的代码。binary_object 仅通过同目录发布入口被使用。

## 文件与子目录

- [binary_object/](binary_object/README.md)：子模块职责与文件说明。
- [binary_object_publish.rs](binary_object_publish.rs)：The sole executable bridge from the release-pinned BC5 archive reader.。
- [bytecode.rs](bytecode.rs)：bytecode 的类型和操作实现。
- [bytecode_publish/](bytecode_publish/README.md)：子模块职责与文件说明。
- [bytecode_publish.rs](bytecode_publish.rs)：Validation and iterative flattening for unlinked bytecode publication.。
- [bytecode_validation.rs](bytecode_validation.rs)：Validate compiler-authored frame, parameter, and eval bytecode layouts before publication.。
- [debug.rs](debug.rs)：Typed source locations and bytecode-to-source metadata.。
- [dynamic_import_policy.rs](dynamic_import_policy.rs)：动态导入代码授权状态。
- [dynamic_source.rs](dynamic_source.rs)：动态函数源码组装。
- [function/](function/README.md)：子模块职责与文件说明。
- [function.rs](function.rs)：Runtime-independent compilation products.。
- [mod.rs](mod.rs)：模块入口、共享接口与子模块声明。
- [module.rs](module.rs)：Runtime-independent ECMAScript module drafts.。
- [rooted.rs](rooted.rs)：Runtime-rooted immutable function bytecode and compiler drafts.。
- [runtime.rs](runtime.rs)：运行时操作或共享所有权入口（见本目录边界）。
