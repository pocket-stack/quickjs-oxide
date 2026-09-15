# 对象语义

拥有对象句柄、描述符、shape，以及属性、调用能力、类、私有成员、对象分配和初始化的语义。

通过 heap 保存原始记录与引用边；调用和用户代码执行交给 VM 协调；内置方法归 builtins。

Dictionary 模式用于普通对象和已转为慢表示的 Array。共享 shape 首次分离，独占 shape
原地转换并脱离 weak cache。物理槽用 swap-remove，插入顺序由独立双向链接维护；
`Shape::entries()` 只表示槽顺序，可观察遍历必须使用 `ordered_indices()` 或
`ordered_own_keys()`。不允许跨可能修改对象的调用缓存槽编号。没有永久墓碑，
容量按几何阈值收缩。prototype 等整布局替换必须经过统一入口恢复语义顺序。
Array 的中间删除/特殊描述符只在首次转换时移动 dense values；以后增删复用
索引和紧凑槽。`dense: None` 保留 QuickJS 的表示敏感行为，length/原型 setter/
索引上界仍由 Array 算法处理。length 不可配置，删除不会移动它的物理首槽。

## 文件与子目录

- [dictionary.rs](dictionary.rs)：dictionary 转换、共享布局分离和整布局替换的顺序恢复。
- [dictionary_order.rs](dictionary_order.rs)：紧凑槽的插入顺序链接和 swap-remove 修复。

- [array_storage.rs](array_storage.rs)：真正 Array 的稀疏索引批量截断；不执行 JS，描述符和 length 回滚由 properties 负责，引用事务复用布局发布入口。

- [access.rs](access.rs)：access 的类型和操作实现。
- [allocation.rs](allocation.rs)：分配及初始化。
- [arguments.rs](arguments.rs)：QuickJS-compatible mapped and unmapped Arguments exotic objects.。
- [class.rs](class.rs)：Class constructor/prototype publication.。
- [class_fields.rs](class_fields.rs)：Public class-field property definition primitives.。
- [builtin_properties.rs](builtin_properties.rs)：在明确的初始化边界批量安装内建懒方法属性。
- [function_initialization.rs](function_initialization.rs)：function_initialization 的类型和操作实现。
- [home_object.rs](home_object.rs)：Bytecode-function HomeObject installation.。
- [internal_methods.rs](internal_methods.rs)：Completion-aware ECMAScript internal-method dispatch.。
- [mod.rs](mod.rs)：模块入口、共享接口与子模块声明。
- [object_literal.rs](object_literal.rs)：Object-literal method publication.。
- [operations.rs](operations.rs)：对象操作的中间状态与描述符转换。
- [private_elements.rs](private_elements.rs)：Runtime substrate for class private data fields.。
- [properties.rs](properties.rs)：Runtime property lookup, definition, and object-layout operations.。
- [property.rs](property.rs)：Ordinary ECMAScript property descriptors.。
- [shape.rs](shape.rs)：Ordinary-object shape metadata with shared-shape copy-on-write.。
- [storage.rs](storage.rs)：storage 的类型和操作实现。
- [template_object.rs](template_object.rs)：Realm-local materialization of QuickJS tagged-template constants.。
