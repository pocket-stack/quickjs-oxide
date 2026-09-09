# 对象语义

拥有对象句柄、描述符、shape，以及属性、调用能力、类、私有成员、对象分配和初始化的语义。

通过 heap 保存原始记录与引用边；调用和用户代码执行交给 VM 协调；内置方法归 builtins。

## 文件与子目录

- [access.rs](access.rs)：access 的类型和操作实现。
- [allocation.rs](allocation.rs)：分配及初始化。
- [arguments.rs](arguments.rs)：QuickJS-compatible mapped and unmapped Arguments exotic objects.。
- [class.rs](class.rs)：Class constructor/prototype publication.。
- [class_fields.rs](class_fields.rs)：Public class-field property definition primitives.。
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
