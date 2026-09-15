# 执行协议桥接

将 VM 请求桥接到 Runtime、对象内部操作和语言内置实现。

维护执行中的 realm 与异常语义；不实现另一套堆或值表示。

## 文件与子目录

- [dynamic_environment.rs](dynamic_environment.rs)：Authenticated object-environment operations used by `with` and sloppy eval.。
- [eval_validation.rs](eval_validation.rs)：eval 的实际帧槽检查；发布拓扑不在执行时重复验证。
- [private_elements.rs](private_elements.rs)：VM adapter for authenticated class-private element instructions.。
- [super_property.rs](super_property.rs)：QuickJS HomeObject and `super` property bridges for one VM frame.。

`prepare_direct_eval_environment` 只接受已发布 snapshot 的环境。发布器验证
拓扑、名字、flags、变量目标和 super 能力；执行前仍检查 caller strictness、
实际槽与 closure cell 元数据。校验不执行用户代码，不跨编译或捕获保留
Runtime 借用；本地/参数槽的实际捕获由 `capture_frame_binding` 完成并验证。
