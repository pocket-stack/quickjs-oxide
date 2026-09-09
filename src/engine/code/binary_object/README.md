# 私有字节码解码

校验受信任的 QuickJS wire 数据，产生脱离运行时的解码结果。

只能由 code/binary_object_publish 消费；中间表示不穿过公开 API，不在解码阶段发布堆对象。

## 文件与子目录

- [atoms.rs](atoms.rs)：Checked atom-index translation for QuickJS 2026-06-04 binary objects.。
- [bytecode_image/](bytecode_image/README.md)：子模块职责与文件说明。
- [code.rs](code.rs)：Bounded, heap-independent scanner for QuickJS 2026-06-04 function code.。
- [function_envelope/](function_envelope/README.md)：子模块职责与文件说明。
- [function_translate/](function_translate/README.md)：子模块职责与文件说明。
- [graph/](graph/README.md)：子模块职责与文件说明。
- [mod.rs](mod.rs)：模块入口、共享接口与子模块声明。
- [ordinary_leaf/](ordinary_leaf/README.md)：子模块职责与文件说明。
- [ordinary_leaf.rs](ordinary_leaf.rs)：Archive-side admission and lowering for an ordinary synchronous leaf.。
- [pinned_atoms.rs](pinned_atoms.rs)：Release-pinned QuickJS atom manifest used by the binary-object format.。
- [pinned_opcodes.rs](pinned_opcodes.rs)：Release-pinned QuickJS final-bytecode opcode catalog.。
- [read_cursor.rs](read_cursor.rs)：Shared checked-read surface for complete BC5 inputs.。
- [scalar_script.rs](scalar_script.rs)：Narrow semantic admission for trusted scalar-script BC5 objects.。
- [wire.rs](wire.rs)：Pure wire primitives for QuickJS 2026-06-04 binary objects.。
