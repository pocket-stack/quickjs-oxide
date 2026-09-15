# 值与转换

拥有完整 Value、原始常量、UTF-16 字符串、数值算法和 ECMAScript 值转换。

原始值算法不访问宿主；运行时转换可以调用对象方法，须保留抛出值和执行上下文。

## 文件与子目录

- [bigint.rs](bigint.rs)：`QuickJS`-compatible arbitrary-precision integer values.。
- [collection_key.rs](collection_key.rs)：已验证 raw key 的 SameValueZero 与哈希；不访问堆、不执行转换或 JS，调用方负责 Runtime 域和内部哨兵检查。
- [conversion.rs](conversion.rs)：ECMAScript 值转换。
- [mod.rs](mod.rs)：模块入口、共享接口与子模块声明。
- [number.rs](number.rs)：Exact numeric formatting primitives for the pinned QuickJS release.。
- [number_parse.rs](number_parse.rs)：Pure numeric-prefix parsing used by the global `parseInt` and `parseFloat`。
- [primitive.rs](primitive.rs)：primitive 的类型和操作实现。

字符串比较、QuickJS 内容指纹及集合键哈希对平坦 Latin-1/UTF-16 直接遍历
切片；只有 rope 使用带所有权保护的遍历状态。混合宽度必须按 UTF-16 code
unit 比较和哈希，不按 UTF-8 字节比较。集合哈希继续由集合索引提供随机种子，
不能以 32 位 QuickJS 内容指纹替代。
