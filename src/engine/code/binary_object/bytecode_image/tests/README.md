# 字节码镜像测试

保存所属模块的回归与协议验证，只在测试目标中使用。测试应验证可观察行为、失败路径和所有权收尾。

编译器生成草稿；发布过程验证身份并取得 roots；VM 消费经过验证的代码。binary_object 仅通过同目录发布入口被使用。

## 文件与子目录

- [atoms.rs](atoms.rs)：模块回归测试。
- [budgets.rs](budgets.rs)：模块回归测试。
- [modules.rs](modules.rs)：模块回归测试。
- [native_decoding.rs](native_decoding.rs)：模块回归测试。
- [reader.rs](reader.rs)：模块回归测试。
- [shared_buffers.rs](shared_buffers.rs)：模块回归测试。
- [topology.rs](topology.rs)：模块回归测试。
- [writer.rs](writer.rs)：模块回归测试。
