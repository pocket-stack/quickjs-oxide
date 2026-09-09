# 环境能力契约

定义引擎请求的时钟、时区、随机种子与输出能力。

生产代码不依赖具体 adapter。原生和浏览器实现位于 adapters/native 与 adapters/web。

## 文件与子目录

- [mod.rs](mod.rs)：模块入口、共享接口与子模块声明。
