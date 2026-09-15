# 本地性能测量产物

Benchmark/Profile 结果保留在本地忽略目录，不提交到 Git。

- S0：`target/performance-retained/reports/s0/`
- QuickJS：`target/performance-retained/reports/quickjs/`
- 最新实现：`target/performance-retained/reports/latest/`
- 原始证据：`target/performance-retained/artifacts/`

各套数据的 `evidence-files.json` 保存原路径、SHA-256 与本地证据位置。`target/` 已由仓库 `.gitignore` 忽略；这些文件不会随克隆自动下载，需要单独保存和传递。

Git 保留测量工具、工作负载、方法及实现说明。上述报告中的历史提交身份仍指测量时版本；源码文件哈希用于核验，历史重写不改变实际测量数据。
