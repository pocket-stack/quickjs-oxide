# 固定版本 Unicode 生成表

保存上游固定版本对应的 Unicode 数据及生成说明。正常构建直接读取这些文件，不执行生成器。

编译器、正则与诊断使用这里的文本规则；精确 UTF-16 文本目前共享 value 中的字符串载体。

## 文件与子目录

- [unicode/](unicode/README.md)：子模块职责与文件说明。

## 补充实现说明

# Generated production data

`unicode/` contains the four checksum-pinned Rust Unicode tables. Handwritten
algorithms remain in `src/unicode*.rs`, which include these files within their
existing private table modules. No generated file expands the public API.

Regenerate with the corresponding `scripts/generate-unicode-*-tables` command
and the pinned QuickJS source, following the source and license information in
each header. Generators default to this directory. Ordinary builds consume the
checked-in tables and do not run the generators or require a reference engine.

To verify an update, generate into a temporary output file and compare it with
the checked-in file; run the Unicode library tests and the normalization
fingerprint check. Test262 metadata belongs in `dev-support/test262/generated/`.
