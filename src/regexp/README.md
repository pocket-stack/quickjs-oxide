# 正则程序

负责模式解析、正则指令生成、匹配和受限中断回调。

JS RegExp 对象、属性、强制转换和用户回调由 engine/builtins/regexp 负责。

## 文件与子目录

- [compiler.rs](compiler.rs)：Typed RegExp parser/compiler foundation.。
- [executor.rs](executor.rs)：Explicit-stack execution for the typed RegExp instruction program.。
- [flags.rs](flags.rs)：Typed flags from pinned QuickJS `libregexp.h` lines 30-38.。
- [group_name.rs](group_name.rs)：QuickJS-shaped RegExp group-name parsing and lexical capture scans.。
- [mod.rs](mod.rs)：模块入口、共享接口与子模块声明。
- [opcode.rs](opcode.rs)：Runtime-independent RegExp instruction IR.。
