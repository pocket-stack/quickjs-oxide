# 函数元数据

负责函数元数据相关实现，是 engine/code 的内部子模块。拥有指令、常量、函数/模块代码、位置与异常信息，以及代码校验、发布和私有字节码解码。

`UnlinkedFunction::new` 要求生产者直接提供参数、局部变量和闭包定义，不按数量生成默认绑定。编译器提交作用域分析结果；BC5 发布桥根据已验证的受限 profile 构造普通绑定。

编译器生成草稿；发布过程验证身份并取得 roots；VM 消费经过验证的代码。binary_object 仅通过同目录发布入口被使用。

## 文件与子目录

- [fixtures.rs](fixtures.rs)：仅在 `cfg(test)` 下为手工字节码测试构造默认绑定及故意异常的定义；不参与生产构建。

- [metadata.rs](metadata.rs)：Function, parameter, closure, and eval descriptors shared by compilation and execution.。
