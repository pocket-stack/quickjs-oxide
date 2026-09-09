# 存储与回收

拥有代际句柄、原始记录、存储变更、引用计数、roots 和循环回收。records 文件定义堆载荷；storage 文件维护记录变更和引用边。

记录可以保存 realm、模块、Promise 和挂起帧的数据；语言算法仍归对应模块。存储层不实现独立的语言内置行为。

## 文件与子目录

- [allocation.rs](allocation.rs)：分配及初始化。
- [arena.rs](arena.rs)：槽位、发布状态、计数与节点访问。
- [binding_records.rs](binding_records.rs)：binding 的原始堆记录、载荷与校验。
- [binding_storage.rs](binding_storage.rs)：binding 的存储变更和引用边维护。
- [buffer_records.rs](buffer_records.rs)：buffer 的原始堆记录、载荷与校验。
- [buffers.rs](buffers.rs)：Borrow-contained ArrayBuffer storage operations and SharedArrayBuffer backing handles.。
- [code_records.rs](code_records.rs)：code 的原始堆记录、载荷与校验。
- [collections.rs](collections.rs)：Insertion-ordered Map/Set storage, weak collection records, and collection iterator state.。
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
- [roots.rs](roots.rs)：变量引用及原始值的 roots。
- [runtime/](runtime/README.md)：子模块职责与文件说明。
- [runtime_gc.rs](runtime_gc.rs)：运行时 GC 入口与统计。
- [shared_memory.rs](shared_memory.rs)：Thread-safe backing storage for `SharedArrayBuffer` wrappers.。
- [suspension_records.rs](suspension_records.rs)：suspension 的原始堆记录、载荷与校验。
- [suspension_storage.rs](suspension_storage.rs)：suspension 的存储变更和引用边维护。
- [tests/](tests/README.md)：子模块职责与文件说明。
- [tests.rs](tests.rs)：模块回归测试。
