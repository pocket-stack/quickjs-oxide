# 编码计划

负责编码计划相关实现，是 engine/code 的内部子模块。拥有指令、常量、函数/模块代码、位置与异常信息，以及代码校验、发布和私有字节码解码。

编译器生成草稿；发布过程验证身份并取得 roots；VM 消费经过验证的代码。binary_object 仅通过同目录发布入口被使用。

## 文件与子目录

- [function.rs](function.rs)：Function-record planning inside the shared whole-image writer state machine.。
- [mod.rs](mod.rs)：模块入口、共享接口与子模块声明。
- [module.rs](module.rs)：Module-record planning inside the shared whole-image writer state machine.。
