# 存储与回收

拥有代际句柄、原始记录、存储变更、引用计数、roots 和循环回收。records 文件定义堆载荷；storage 文件维护记录变更和引用边。

记录可以保存 realm、模块、Promise 和挂起帧的数据；语言算法仍归对应模块。存储层不实现独立的语言内置行为。

## 文件与子目录

- [dictionary_storage.rs](dictionary_storage.rs)：普通对象 dictionary 的单属性删除/替换事务；移动槽转移所有权，释放仅作用于被移除项。

- [collection_records.rs](collection_records.rs)：Map/Set 存活记录、键索引和递增记录 ID 的统一拥有者；删除回收记录，游标不依赖物理槽。

- [allocation.rs](allocation.rs)：分配及初始化。
- [arena.rs](arena.rs)：槽位、发布状态、计数与节点访问。
- [binding_records.rs](binding_records.rs)：binding 的原始堆记录、载荷与校验。
- [binding_storage.rs](binding_storage.rs)：binding 的存储变更和引用边维护。
- [buffer_records.rs](buffer_records.rs)：buffer 的原始堆记录、载荷与校验。
- [buffers.rs](buffers.rs)：Borrow-contained ArrayBuffer storage operations and SharedArrayBuffer backing handles.。
- [code_records.rs](code_records.rs)：code 的原始堆记录、载荷与校验。
- [collections.rs](collections.rs)：Insertion-ordered Map/Set storage, weak collection records, and collection iterator state.。
- [collection_index.rs](collection_index.rs)：强集合的非拥有型键索引；只保存哈希和稳定记录位置，由集合存储同步增删。查找不执行 JS；完整一致性扫描仅用于发布校验。
- [deferred.rs](deferred.rs)：延迟操作队列、待处理状态和清理重入守卫。
- [gc.rs](gc.rs)：Heap reference ownership, ordered weak-reference processing, and cycle collection.。
- [identity.rs](identity.rs)：identity 的类型和操作实现。
- [iteration_records.rs](iteration_records.rs)：iteration 的原始堆记录、载荷与校验。
- [iterator_records.rs](iterator_records.rs)：iterator 的原始堆记录、载荷与校验。
- [iterator_storage.rs](iterator_storage.rs)：iterator 的存储变更和引用边维护。
- [mod.rs](mod.rs)：模块入口、共享接口与子模块声明。
- [module_records.rs](module_records.rs)：module 的原始堆记录、载荷与校验。
- [module_storage.rs](module_storage.rs)：module 的存储变更和引用边维护。
- [object_records.rs](object_records.rs)：object 的原始堆记录、载荷与校验。
- [object_storage.rs](object_storage.rs)：object 的存储变更和引用边维护。
- [ownership.rs](ownership.rs)：运行时句柄保留、释放和延后引用清理。
- [private_validation.rs](private_validation.rs)：Authenticate private binding metadata and private callable bytecode before heap publication.。
- [promise_records.rs](promise_records.rs)：promise 的原始堆记录、载荷与校验。
- [promise_storage.rs](promise_storage.rs)：promise 的存储变更和引用边维护。
- [realm_records.rs](realm_records.rs)：realm 的原始堆记录、载荷与校验。
- [realm_storage.rs](realm_storage.rs)：realm 的存储变更和引用边维护。
- [release_cleanup_tests.rs](release_cleanup_tests.rs)：延迟清理优先级、重入、失败恢复和销毁生命周期测试。
- [roots.rs](roots.rs)：变量引用及原始值的 roots。
- [runtime/](runtime/README.md)：子模块职责与文件说明。
- [runtime_gc.rs](runtime_gc.rs)：运行时 GC 入口与统计。
- [shared_memory.rs](shared_memory.rs)：Thread-safe backing storage for `SharedArrayBuffer` wrappers.。
- [suspension_records.rs](suspension_records.rs)：suspension 的原始堆记录、载荷与校验。
- [suspension_storage.rs](suspension_storage.rs)：suspension 的存储变更和引用边维护。
- [tests/](tests/README.md)：子模块职责与文件说明。
- [tests.rs](tests.rs)：模块回归测试。

- [profiling.rs](profiling.rs)：可选 arena backing storage 跟踪和资源拥有者的内存统计。

Map/Set 的 `CollectionRecords` 独占存活记录、键索引、存活 ID 顺序和递增 ID
时钟。删除不复用 ID；clear 释放存储但保留时钟。游标只保存 ID，因此活迭代器
不要求保留墓碑。打印器将已删除的 current ID 合成为空项，不能把它当成存活记录。
记录 key 在发布后不可修改，value 替换通过专用入口，GC 边事务仍由 heap 拥有。

顺序暂用标准库 BTreeSet：键/记录定位平均 O(1)，增删与寻找下一项 O(log n)，
整表遍历 O(n)。这是为了在支持任意暂停游标时立即回收历史记录，避免引入游标
注册表或自定义链接回收协议；不声称全部操作 O(1)。哈希表按几何阈值收缩，容量
随存活规模变化；测试覆盖暂停游标、重插、clear、ID 耗尽与参考模型随机序列。

长字符串哈希缓存属于单个 CollectionIndex 的随机种子域。它只记录最多 8 个
长度至少 256 code units 的弱字符串身份，采用 FIFO 淘汰；每次 miss 仍按完整
内容哈希，命中后的键相等性仍需验证。缓存不会持有字符串载荷，不参与存储相等性，
clear 会释放它。短键不分配缓存；不能将缓存移交给使用不同种子的索引。
