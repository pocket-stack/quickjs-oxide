# 编译

负责词法、语法、作用域解析与字节码生成。输出尚未发布的代码草稿。

使用 source、code、atom 和 value 的共享表示；不选择宿主 provider，不创建执行中的堆对象。

## 文件与子目录

- [arrow.rs](arrow.rs)：arrow 的类型和操作实现。
- [class/](class/README.md)：子模块职责与文件说明。
- [class.rs](class.rs)：Class parsing and lowering.。
- [destructuring.rs](destructuring.rs)：destructuring 的类型和操作实现。
- [function.rs](function.rs)：function 的类型和操作实现。
- [generator.rs](generator.rs)：generator 的类型和操作实现。
- [lexer.rs](lexer.rs)：ECMAScript lexical analysis.。
- [lowering.rs](lowering.rs)：Lower resolved IR to verified bytecode and source debug information.。
- [mod.rs](mod.rs)：模块入口、共享接口与子模块声明。
- [module.rs](module.rs)：Static ECMAScript module parsing and root-binding lowering.。
- [object_literal.rs](object_literal.rs)：object_literal 的类型和操作实现。
- [optional_chain.rs](optional_chain.rs)：QuickJS-shaped optional-chain parsing and control-flow rewrites.。
- [private_reference.rs](private_reference.rs)：Class-private data-field references and late lexical resolution.。
- [pseudo_binding.rs](pseudo_binding.rs)：pseudo_binding 的类型和操作实现。
- [resolution.rs](resolution.rs)：Resolve lexical names, declaration hoists, eval environments, and closure captures in the IR.。
- [scope_validation.rs](scope_validation.rs)：Validate the completed scope and binding graph before identifier resolution.。
- [template.rs](template.rs)：QuickJS-shaped template literal and tagged-template lowering.。
- [tests/](tests/README.md)：子模块职责与文件说明。
- [tests.rs](tests.rs)：模块回归测试。

作用域的 `bindings` 保留声明顺序和稳定 BindingId；名称索引仅用于选择该
顺序中最后一个同名绑定。正常声明通过 `FunctionIr::add_binding` 同步索引，
晚插入的函数私有名称在调整顺序后重建索引。遮蔽、重复声明是否合法以及
参数环境边界仍由解析和验证逻辑决定，不能依赖 HashMap 的覆盖行为判定。

常量池仍按追加顺序决定字节码编号。所有追加通过
`FunctionIr::append_constant`；字符串索引记录第一个 primitive string，
仅供 `ensure_string_constant` 复用名称。普通字面量追加不会去重，AtomString
和模板常量也不会混入这个索引。模板后处理只修改模板载荷。
