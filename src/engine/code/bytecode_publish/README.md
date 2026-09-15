# 事务化代码发布

校验代码布局、闭包、环境及元数据，并将合法草稿转换成运行时持有的代码。

解码/编译只提供草稿；发布必须保留失败回滚与名称、代码 roots 的一致性。

## 文件与子目录

- [module_initializer_flow.rs](module_initializer_flow.rs)：Context-sensitive module-initializer flow validation.。
- [private_elements.rs](private_elements.rs)：Publication-time authentication for class-private bytecode.。

## 验证结果的所有权

`VerifiedFunction` 拥有通过角色验证的原始草稿，只能由 Script、受限
ordinary leaf、eval 或 module 构造入口产生。发布消费该值；验证和发布之间
不暴露可变草稿。模块 parts 的类型参数仅表示 function 从草稿变为验证结果，
其余表保持与该函数同一次验证的内容。该结果不拥有运行时 roots；后续发布的
名称链接、heap 保留和失败回滚仍由 runtime 发布事务负责。
