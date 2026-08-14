# ARCHITECTURE.md — 分层架构与语言决策

> 原则:**各司其职,不搞大一统王朝**。每层选最合适的语言;
> 同层内保持统一,降低开发上下文切换与跨语言 bug。

## 分层

```
 L4  上层管理    Python:构建、ISO 检查、世界工具、安装器、服务管理
 L3  服务器核心  Rust(统一):协议 / 世界 / 实体 / 网络状态机
 L2  底层热路径  C:varint、位打包、NBT、压缩(FFI 供 L3)
 L1  内核核心    Rust(主体,no_std)+ C(驱动热路径)+ 汇编(最小入口)
 L0  引导        汇编(boot.S)+ GRUB(multiboot2)
```

## 语言决策(已定案)

| 层 | 语言 | 理由 |
|---|---|---|
| L0 引导 | 汇编 + GRUB | multiboot2 规范,GRUB 开源组件,最小汇编入口 |
| L1 内核核心 | Rust + C | 用户要求:内核核心保持 C/Rust。Rust 所有权保证无 GC 安全,驱动热路径可落 C |
| L2 热路径 | C | 位操作密集,直接映射指令集;Rust 参考实现交叉验证防回归 |
| L3 服务器核心 | **Rust 统一** | 同层统一原则:整层 Rust,性能≈C++,trait/枚举抽象更强;引入 C++ 会制造 FFI 边界与所有权 bug,收益为零 |
| L4 上层管理 | **Python** | 构建/安装/服务管理是胶水逻辑,Python 开发效率最高,替代 bash 脚本 |

### 决策记录

- 中层**不采用 C++**:项目无既有 C++ 代码库,无 C++ 生态需求;
  Rust 在同一层已提供同等性能与更好的抽象,且保持整层统一(用户原则:同层统一 > 跨层炫技)。
- 上层**采用 Python**:管理工具(构建、ISO 检查、世界预览、服务管理)
  以胶水逻辑为主,Python 表达力与标准库足以胜任;宿主与 rootfs 通用。
- 内核保持 **Rust 主体 + C 热路径**:与 L2 共用同一套 C 库(双后端),
  内核与宿主服务器共享 varint/bitpack 实现,减少行为差异。

## 数据流

```
Python (L4) ── 调用 ──> mc-server (L3, Rust)
                            │  FFI
                            └──> mc-hotpath (L2, C) ──> 内核 C 库 (L1)
```

## 目录

```
iso/
├── server/            L2+L3:Rust workspace + native/ C 热路径
├── boot/              L0+L1:引导 + 内核
├── sysroot/           L0:ISO 文件系统层(GRUB 配置)
├── scripts/           L4:Python/bash 管理工具
└── dist/              ISO 产物
```