# 对象图

负责对象图相关实现，是 engine/code 的内部子模块。拥有指令、常量、函数/模块代码、位置与异常信息，以及代码校验、发布和私有字节码解码。

编译器生成草稿；发布过程验证身份并取得 roots；VM 消费经过验证的代码。binary_object 仅通过同目录发布入口被使用。

## 文件与子目录

- [arena.rs](arena.rs)：槽位、发布状态、计数与节点访问。
- [decode.rs](decode.rs)：Bounded, heap-independent decoder for the first BC5 data-object slice.。
- [encode.rs](encode.rs)：Canonical BC5 writer for a validated, heap-independent [`WireGraph`].。
- [mod.rs](mod.rs)：模块入口、共享接口与子模块声明。
- [model.rs](model.rs)：Heap-independent data model for decoded BC5 object graphs.。
- [sab_transport.rs](sab_transport.rs)：Pointer-free archive identities for BC5 SharedArrayBuffer records.。
- [write_state.rs](write_state.rs)：Shared traversal accounting for canonical BC5 data writers.。
