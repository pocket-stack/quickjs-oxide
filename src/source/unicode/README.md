# Unicode 文本规则

实现标识符字符判断、大小写、字符属性和规范化。

只处理文本；生成表来自固定的上游版本，生成工具位于 scripts/unicode。

## 文件与子目录

- [case.rs](case.rs)：Checksum-pinned Unicode 17 case conversion used by String intrinsics.。
- [generated/](generated/README.md)：子模块职责与文件说明。
- [mod.rs](mod.rs)：模块入口、共享接口与子模块声明。
- [normalize.rs](normalize.rs)：Checksum-pinned Unicode 17 normalization used by String intrinsics.。
- [property.rs](property.rs)：Checksum-pinned Unicode 17 property sets used by RegExp property escapes.。
