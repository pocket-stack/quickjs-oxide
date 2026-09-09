# Realm 初始化

建立上下文内的全局绑定、原型和初始内置关系。

使用 heap 记录及 builtins 初始化入口；不决定应用的模块加载或输出策略。

## 文件与子目录

- [bindings.rs](bindings.rs)：全局词法和变量绑定。
- [construction.rs](construction.rs)：Allocate a realm and initialize its intrinsic roots in publication order.。
- [mod.rs](mod.rs)：模块入口、共享接口与子模块声明。
- [prototypes.rs](prototypes.rs)：realm 原型与全局对象解析。
