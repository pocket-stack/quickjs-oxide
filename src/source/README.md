# 源码与文本基础

保存原始源码、范围和坐标，提供 Unicode 字符属性、大小写与规范化。编译策略和运行时对象不属于这里。

编译器、正则与诊断使用这里的文本规则；精确 UTF-16 文本目前共享 value 中的字符串载体。

## 文件与子目录

- [coordinates.rs](coordinates.rs)：Byte offsets and QuickJS source-coordinate mapping.。
- [mod.rs](mod.rs)：模块入口、共享接口与子模块声明。
- [text.rs](text.rs)：Byte-exact carrier text for dynamically supplied ECMAScript source.。
- [unicode/](unicode/README.md)：子模块职责与文件说明。
