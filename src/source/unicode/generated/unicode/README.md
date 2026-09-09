# 固定版本 Unicode 生成表

保存上游固定版本对应的 Unicode 数据及生成说明。正常构建直接读取这些文件，不执行生成器。

编译器、正则与诊断使用这里的文本规则；精确 UTF-16 文本目前共享 value 中的字符串载体。

## 文件与子目录

- [unicode_case_tables.rs](unicode_case_tables.rs)：unicode_case_tables 的类型和操作实现。
- [unicode_ident_tables.rs](unicode_ident_tables.rs)：unicode_ident_tables 的类型和操作实现。
- [unicode_normalize_tables.rs](unicode_normalize_tables.rs)：unicode_normalize_tables 的类型和操作实现。
- [unicode_property_tables.rs](unicode_property_tables.rs)：unicode_property_tables 的类型和操作实现。
