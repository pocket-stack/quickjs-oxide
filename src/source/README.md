# 源码与文本基础

保存原始源码、范围和坐标，提供 Unicode 字符属性、大小写与规范化。编译策略和运行时对象不属于这里。

编译器、正则与诊断使用这里的文本规则；精确 UTF-16 文本目前共享 value 中的字符串载体。

## 文件与子目录

- [coordinates.rs](coordinates.rs)：Byte offsets and QuickJS source-coordinate mapping.。
- [mod.rs](mod.rs)：模块入口、共享接口与子模块声明。
- [text.rs](text.rs)：Byte-exact carrier text for dynamically supplied ECMAScript source.。
- [unicode/](unicode/README.md)：子模块职责与文件说明。

`QuickJsSourceLocator` 服务一次性诊断，保持无分配；大量位置查询使用其
`index()` 生成的 `QuickJsSourceIndex`。索引按每 256 个原始字节记录行列，
跨所有子函数共享，查询最多扫描 255 字节，单个超长行也不会退化。
两者使用相同 LF/UTF-8 continuation 计数规则，不把解析字节改成标准 UTF-8。
