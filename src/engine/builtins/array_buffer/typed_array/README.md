# TypedArray

负责TypedArray相关实现，是 engine/builtins 的内部子模块。实现 ECMAScript 内置对象与函数，以及 qjs 兼容辅助行为；拥有原生调用选择器与分派。

使用 value 转换、object 语义和 VM 调用；环境能力通过 host 契约注入。

## 文件与子目录

- [copying/](copying/README.md)：子模块职责与文件说明。
- [copying.rs](copying.rs)：Non-species copying `%TypedArray%.prototype` algorithms.。
- [find/](find/README.md)：子模块职责与文件说明。
- [find.rs](find.rs)：Callback-based `%TypedArray%.prototype` find algorithms.。
- [iteration/](iteration/README.md)：子模块职责与文件说明。
- [iteration.rs](iteration.rs)：Callback-based `%TypedArray%.prototype` iteration algorithms.。
- [mutation/](mutation/README.md)：子模块职责与文件说明。
- [mutation.rs](mutation.rs)：In-place `%TypedArray%.prototype` mutation algorithms.。
- [reduce/](reduce/README.md)：子模块职责与文件说明。
- [reduce.rs](reduce.rs)：Accumulator-based `%TypedArray%.prototype` reduction algorithms.。
- [search/](search/README.md)：子模块职责与文件说明。
- [search.rs](search.rs)：Indexed `%TypedArray%.prototype` lookup and search algorithms.。
- [slice/](slice/README.md)：子模块职责与文件说明。
- [slice.rs](slice.rs)：Copying and view-producing `%TypedArray%.prototype` algorithms.。
- [sort/](sort/README.md)：子模块职责与文件说明。
- [sort.rs](sort.rs)：`%TypedArray%.prototype.sort` and `toSorted`.。
- [species.rs](species.rs)：TypedArray species construction shared by copying prototype methods.。
- [stringification/](stringification/README.md)：子模块职责与文件说明。
- [stringification.rs](stringification.rs)：`%TypedArray%.prototype` stringification algorithms.。
- [tests.rs](tests.rs)：模块回归测试。
- [uint8_codec.rs](uint8_codec.rs)：Concrete `Uint8Array` base64 and hexadecimal codecs.。
