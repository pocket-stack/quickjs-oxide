# 编译器

compiler 负责把完整 JavaScript 源码转换为尚未发布的函数和模块草稿，
包括词法与语法分析、声明和作用域处理、名字解析、闭包捕获及栈指令生成。
修改语法、绑定规则或编译期改写时，从这个模块开始。

入口只编排请求；options 管配置，parser 管语法/诊断与构造状态，model 管
阶段间共享产物。FunctionBuilder 的消费式 finish 检查临时控制状态后交出 IR；
resolution 解析名字，lowering 生成栈码，relocation 管目标重定位，flow 复用
必需验证，optimize 管有限局部改写与源码位置投影。

当前管线使用线性 FunctionIr/IrOp，解析和绑定解析完成后生成栈式代码。
声明顺序、稳定绑定身份、异常区域和源码位置贯穿这些阶段；名字索引是
查找工具，不能代替遮蔽、重复声明或 eval 的语义判断。

compiler 使用 source 的精确源码表示和 code 的代码契约。它不选择宿主
provider，也不拥有正在执行的帧；运行时链接、roots 和发布事务属于 code
及 heap。对象和 Symbol 的运行时身份不成为普通编译常量。

[栈 VM 实施设计](../../../docs/primitive-vm-implementation-plan.md)
描述待实施的内部重组；完整前端及其语义仍由本模块负责。
