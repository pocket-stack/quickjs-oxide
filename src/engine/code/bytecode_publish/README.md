# 事务化代码发布

校验代码布局、闭包、环境及元数据，并将合法草稿转换成运行时持有的代码。

解码/编译只提供草稿；发布必须保留失败回滚与名称、代码 roots 的一致性。

## 文件与子目录

- [module_initializer_flow.rs](module_initializer_flow.rs)：Context-sensitive module-initializer flow validation.。
- [private_elements.rs](private_elements.rs)：Publication-time authentication for class-private bytecode.。
