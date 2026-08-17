# ARCHITECTURE.md — 分层架构与语言决策

> 原则:每层选最合适的语言;同层内保持统一。

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
  Rust 在同一层已提供同等性能与更好的抽象,且保持整层统一。
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

## NUMA 子系统

> 目标:多路/多节点主机上按节点就近分配内存、调度 CPU。
> 现状:拓扑解析(SRAT/命令行/单节点兜底)+ 每节点帧链分配器 +
> 区间归属 + 距离矩阵 + 就近/交错分配策略,已在 QEMU 4GiB/2 节点验证;
> 任务调度器 CPU 亲和与内存迁移为后续。

### 拓扑来源(优先级)

| 来源 | 机制 | 说明 |
|---|---|---|
| ACPI SRAT | 内存亲和(proximity→base+len)+ 处理器亲和(proximity→LAPIC ID) | QEMU `-numa` 生成,实体多路主板亦生成 |
| 内核命令行 | `numa=<n>;<id>:<startMB>-<endMB>;...` | multiboot2 cmdline tag,无 ACPI 时兜底 |
| 单节点 | 全可用内存归节点 0 | 无 NUMA 的主板默认 |

### 数据结构

- `NumaNode{ id, pages, free_head, free_cnt, alloc_cnt, spans[8], span_cnt }` × 8(MAX_NODES)。
- 每节点一条**空闲帧链**:空闲帧内容存 next 物理地址,零额外内存开销;
  多个不连续区间按序入链(新区间接在旧链头之前,旧链保持可达)。
- `spans[]` 记录节点全部 usable 区间,`node_of(phys)` 据此精确归属 ——
  节点内存被 PCI hole 拆开(QEMU 4GiB/2 节点时 node1 = 2-3GiB + 4-5GiB 两段)也能正确判定。
- `SLIT[64][64]` 节点距离矩阵(ACPI SLIT;无 SLIT 时默认同节点 10/跨节点 20)。
- `CPU_NODES[(lapic_id, node)]`:SRAT 处理器亲和 → LAPIC ID 到节点索引。

### 接口

- `init(info) -> node_cnt`:扫描 multiboot2 mmap 的 usable 区间,排除低 1MiB
  (实模式 IVT/BDA/EBDA/multiboot info/trampoline 区)与内核镜像,按节点串帧链。
- **并发安全**:帧链分配/释放全程持自旋锁(`NODES_LOCK`),BSP 与 AP 可安全并发;
  内核现为无抢占轮询式,锁内无须关中断。
- **非法释放校验**:`free()` 的地址必须落在某节点 usable 区间,否则拒绝并告警
  (防低 1MiB/内核镜像/越界地址静默挂链毁链)。
- `alloc_local(node) -> Option<u64>`:本地节点优先,耗尽按距离就近 fallback。
- `alloc_interleave() -> Option<u64>`:跨节点轮转(interleave 策略)。
- `alloc() / free(phys) / node_of(phys)`、`node_mem(node) -> (MiB, free)`、
  `node_for_lapic(lapic) -> Option<node>`(SMP 唤醒时标注 AP 所属节点)。
- `node_distance(a, b) -> u8`:SLIT 距离;`node_allocs(node) -> u64` 分配计数。
- `selftest()`:启动时自检(各策略分配/归还、节点归属、距离),失败即 panic。
- 地址全部 u64,帧对齐 4KiB,物理上限 16GiB(`MAX_PHYS`,与页表一致)。
- shell `numa` 命令:节点拓扑/距离矩阵/分配策略演示。

### 后续(未实现)

- 节点间内存均衡分配器(按节点比例),任务调度器 CPU 亲和;
- 冷/热内存迁移、页面回收跨节点 fallback 统计。