# Mp-minecraft

用 Rust(为主)+ C(底层热路径)自研操作系统与 Minecraft 服务器核心。
目标版本:**Minecraft Java 26.1.2 / 协议 775 / data version 4790**。

## 结构

```
iso/
├── docs/            # 架构、目录布局、进度记录
├── server/          # 服务器核心(Rust workspace)
│   ├── crates/
│   │   ├── mc-protocol/  # 协议层:varint、帧、包编解码
│   │   ├── mc-world/     # 世界:区块、section 位打包
│   │   ├── mc-hotpath/   # C FFI 热路径 + Rust 参考实现
│   │   └── mc-server/    # 服务器二进制:握手/登录/配置/Play
│   └── native/           # C 热路径源码
├── boot/            # 引导 + 最小内核(multiboot2 + 长模式)
├── sysroot/         # ISO 文件系统层(GRUB 配置)
├── scripts/         # 构建与工具
└── dist/            # ISO 产物
```

## 进度

已完成:`docs/PROGRESS.md`。