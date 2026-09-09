# 执行协议桥接

将 VM 请求桥接到 Runtime、对象内部操作和语言内置实现。

维护执行中的 realm 与异常语义；不实现另一套堆或值表示。

## 文件与子目录

- [dynamic_environment.rs](dynamic_environment.rs)：Authenticated object-environment operations used by `with` and sloppy eval.。
- [private_elements.rs](private_elements.rs)：VM adapter for authenticated class-private element instructions.。
- [super_property.rs](super_property.rs)：QuickJS HomeObject and `super` property bridges for one VM frame.。
