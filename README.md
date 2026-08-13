# Mp-minecraft — 从 0 到 1:自研系统上的 Minecraft 服务器核心

一个由 Rust + C 语言编写的 Minecraft 系统:从硬件驱动出发尽力减少性能损耗,
并对多 CPU(Multiprocessor)场景做底层 NUMA 与各方面优化。
用 Rust(为主)+ C(底层热路径)从零实现一个操作系统,并在这个系统上完整复刻
Minecraft 服务器核心,最终打包成可引导 ISO,引导后自动拉起服务器。

目标版本:**Minecraft Java 26.1.2 / 协议 775 / data version 4790**。

## 结构

```
iso/
├── docs/ROADMAP.md   # 全量分层开发步骤(9 层,含 NUMA/SMP)
├── server/           # 服务器核心(Rust workspace)
│   ├── crates/
│   │   ├── mc-protocol/  # 协议层:varint、帧、包编解码
│   │   ├── mc-world/     # 世界:区块、section 位打包、超平坦生成
│   │   ├── mc-hotpath/   # C FFI 热路径 + Rust 参考实现
│   │   └── mc-server/    # 服务器二进制:握手/登录/配置/Play
│   └── native/           # C 热路径源码
├── kernel/           # 自研微内核(x86_64,no_std)
└── sysroot/          # ISO 系统层(buildroot overlay + 构建脚本)
```

## 当前进度

- [x] 工程骨架 + 协议层(varint / 帧 / 编解码)
- [ ] 世界:超平坦 + section 打包
- [ ] C 热路径:varint / chunk 位打包
- [ ] 服务器二进制:登录握手 → 配置 → Play
- [ ] 微内核引导(串口输出)
- [ ] ISO 打包

## 授权

