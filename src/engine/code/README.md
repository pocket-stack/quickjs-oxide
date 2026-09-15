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
- [executable.rs](executable.rs)：发布快照、只读数据投影与常量访问。
- [dynamic_source.rs](dynamic_source.rs)：动态函数源码组装。
- [function/](function/README.md)：子模块职责与文件说明。
- [function.rs](function.rs)：Runtime-independent compilation products.。
- [mod.rs](mod.rs)：模块入口、共享接口与子模块声明。
- [module.rs](module.rs)：Runtime-independent ECMAScript module drafts.。
- [rooted.rs](rooted.rs)：Runtime-rooted immutable function bytecode and compiler drafts.。
- [runtime.rs](runtime.rs)：运行时操作或共享所有权入口（见本目录边界）。

`executable` 提供不可变的已发布执行快照。快照在 Runtime 身份与 realm
验证后取得字节码 root，共享代码和常量等只读 backing storage，不复制内容。
读取投影不会调用 JS；只有合成测试可以修改无 root 的 fixture，已发布快照
在测试中也保持不可变。运行时值的 rooting 和释放仍遵循原 heap 契约。

快照构造只增加固定数量的 Rc/root 引用，不遍历或复制指令、常量与绑定数组；
销毁释放这些引用。`constant` 共享下标转换与边界处理，调用者保留明确的
Value/Function/RegExp 种类 match；属性名字继续使用原有链接 Atom 表。
错误 Runtime 或 realm 在借出执行视图之前被拒绝。root 的生命周期与不可变性
反例见 `executable::tests`；发布失败的事务回滚仍由 `runtime` 拥有。

`PublishedEvalEnvironment` 通过受检索引共享已发布环境数组，并持有字节码 root。
准备、编译和物化只复制 Rc/root 引用；同一环境按数组身份和索引核对，
不再深拷贝或比较静态 scopes/bindings。动态名字解析与捕获不在此视图缓存。
