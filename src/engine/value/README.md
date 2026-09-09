# 值与转换

拥有完整 Value、原始常量、UTF-16 字符串、数值算法和 ECMAScript 值转换。

原始值算法不访问宿主；运行时转换可以调用对象方法，须保留抛出值和执行上下文。

## 文件与子目录

- [bigint.rs](bigint.rs)：`QuickJS`-compatible arbitrary-precision integer values.。
- [conversion.rs](conversion.rs)：ECMAScript 值转换。
- [mod.rs](mod.rs)：模块入口、共享接口与子模块声明。
- [number.rs](number.rs)：Exact numeric formatting primitives for the pinned QuickJS release.。
- [number_parse.rs](number_parse.rs)：Pure numeric-prefix parsing used by the global `parseInt` and `parseFloat`。
- [primitive.rs](primitive.rs)：primitive 的类型和操作实现。
