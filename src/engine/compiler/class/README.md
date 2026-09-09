# 类编译

负责类定义中字段、私有名称和静态初始化块的编译，供父模块的类解析与字节码生成流程使用。

- [fields.rs](fields.rs)：类字段初始化的编译。
- [private.rs](private.rs)：私有名称及相关访问的编译。
- [static_block.rs](static_block.rs)：类静态初始化块的编译。

此处处理源码与编译期语义；运行时类对象、字段与私有元素操作由 engine/object 负责。
