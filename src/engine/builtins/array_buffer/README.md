# 缓冲区与视图

这个模块负责 ArrayBuffer 的语言行为，以及建立在缓冲区之上的 TypedArray
与 DataView：构造、长度与索引转换、数值读写、resize、detach 和 transfer。
不同视图共享背后的存储，而非把字节表示成普通对象的索引属性。

heap 拥有原始字节与共享存储，内置层决定品牌检查、转换时机、边界判断
和错误顺序。SharedArrayBuffer 与 Atomics 由相邻内置模块实现，共用
相应存储能力，但保留共享内存的专门规则。

对象参数转换可能执行 JS，并改变缓冲区或视图。读取元数据与实际字节
访问必须按该操作的语义重新确认有效性；一次无回调访问中的 token
不能跨转换回调继续使用。TypedArray 的 Number/BigInt 元素语义与
DataView 的访问方式也不能无条件合并。

具体方法按复制、排序、迭代等算法分组；这些小目录使用源码契约，
不各自维护模块介绍。上层职责见[builtins 介绍](../README.md)。
