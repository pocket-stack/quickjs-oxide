# 语言内置行为

builtins 实现 ECMAScript 的内置对象与函数，以及显式启用的 qjs 辅助
行为。每个内置领域拥有自己的构造、原型方法和算法；原生调用选择器
将调用分派到这些实现。

一个内置方法通常会交错进行参数转换、属性访问、存储操作和用户回调。
转换使用 value，属性语义使用 object，执行 JS 使用 VM；这些步骤的
顺序和中途异常已经产生的效果由内置算法决定，不能由通用存储批处理
代替。Promise 反应与 jobs、async 驱动也各有自己的职责。

环境能力通过 host 契约取得，具体系统或浏览器实现位于 adapters。
内置对象的原始载荷与引用边由 heap 保存；语言行为仍留在这里。

[缓冲区与视图](array_buffer/README.md)有独立介绍，说明二进制内置的
共享存储边界。其余小型方法分组使用源码说明。[栈 VM 计划](../../../docs/primitive-vm-plan.md)
改变内部 JS 回调的推进机制，各内置领域继续拥有其算法与恢复状态。

迁移中的 `continuation` 是 native 算法的封闭登记表；它只选择领域
step，不保存属性或转换算法。各领域的 Step/Resume 保存阶段与必要 roots，
旧同步入口及 owned VM 共用同一算法。VM 的 request 模块按领域适配有类型的
请求和回复，回调由显式子帧推进；无回调叶函数仍直接执行。

S05 同步领域已接入，完整性仍须以[逐调用点账本](../../../docs/primitive-vm-sync-callbacks.md)
和统一验收为准。Promise/generator 归 S06，模块与真实 host/API 入口归 S07。
