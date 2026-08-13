# MCS — 从 0 到 1 全自研系统路线图

> 目标:用 Rust 为主、C 做底层热路径,从引导扇区开始自研一个操作系统,并在这个系统上
> 完整复刻 Minecraft 服务器核心,最终打包成可引导 ISO,引导后自动拉起服务器。
> 首个可见里程碑:客户端能进一个**超平坦世界**并能走动。

参考范式:《操作系统从 0 到 1》(分层递进、每层有可见产出)。

---

## 总体分层

```
 L8  应用层    MCS 服务器核心(Rust,协议/世界/实体)
 L7  运行时    Rust no_std 运行时、allocator、FFI→C 热路径
 L6  系统服务  init、进程/线程模型、TCP/IP 用户态栈、文件系统服务
 L5  驱动      virtio-net、virtio-blk、串口、定时器(HPET/APIC)
 L4  内核      NUMA 感知内存管理、SMP 调度器、中断/异常、系统调用
 L3  引导装载  Multiboot2 → 长模式, 页表建立
 L2  固件       BIOS/UEFI 启动, 内存探测, 进入 64 位
 L1  工程基建  交叉编译、QEMU 验证台、每层冒烟测试
```

里程碑定义(每层必须有“看得见的东西”):
- M0 引导:QEMU 里看到内核串口输出
- M1 中断/时钟:能看到 tick 日志
- M2 内存+NUMA:能分配、能看 NUMA 拓扑
- M3 SMP 调度:多核各跑一个任务
- M4 网络:内核内 TCP 能回包
- M5 文件系统:能读写区块文件
- M6 宿主服务器:客户端进超平坦世界
- M7 服务器移植内核:内核上跑服务器
- M8 ISO:镜像引导即运行

---

## L1 工程基建

1. 工具链:
   - Rust nightly(需 `-Zbuild-std`)+ `x86_64-unknown-none` 目标。
   - `llvm-tools`/`cargo-binutils`(objcopy 生成内核二进制)。
   - WSL 内 gcc 用于 C 热路径与 Linux 交叉目标。
2. 验证台:QEMU `qemu-system-x86_64`(win 或 wsl 任选),统一 `scripts/run.sh`。
3. 仓库结构:见 `ARCHITECTURE.md`。
4. CI 思路:`cargo test`(宿主测服务器逻辑)+ `scripts/smoke.sh`(QEMU 跑内核冒烟)。

## L2/L3 引导装载

1. 编写 multiboot2 头(kernel 首部 magic + tag),GRUB/QEMU 直接装载。
2. 汇编入口 `_start`:
   - 检查 CPUID/长模式支持;
   - 加载 GDT;
   - 建立初始恒等映射页表;
   - `lgdt` → 跳入 64 位长模式;
   - 从 multiboot2 info 中解析内存布局、帧缓冲信息;
   - 跳转 Rust `kernel_main`。
3. 验证:串口输出启动 banner 与内存总量。

## L4 内核

### 4.1 中断与异常
1. IDT 表 + 各异常处理(stub 汇编 → Rust handler),断点/页错误有日志。
2. PIC→APIC 切换;Local APIC 定时器作为 tick 源(替代 PIT/HPET 或并存)。
3. 时钟:1000Hz tick,内核时钟计数,供调度与服务器 tick 使用。

### 4.2 内存管理(NUMA 感知)
1. 物理帧分配器:buddy allocator,按 frame 管理,支持多 NUMA 节点。
2. **NUMA 探测**:从 ACPI SRAT 表解析处理器/内存亲和域(node 与内存范围映射);
   - 每 node 独立 buddy 池;
   - 提供 `frame_alloc_node(node, flags)`(本地优先、可回退);
   - 内核初始化时按 node 着色/交错策略配置。
3. 虚拟内存:x86_64 四级页表封装(`PML4/PDPT/PD/PT`),懒映射/去映射。
4. 堆:内核 heap 分配器(可回收的 arena + 通用分配),`alloc` crate 对接。
5. 守护页/栈溢出检测。

### 4.3 SMP 调度
1. BSP 引导 AP:解析 MP 表/APIC、ACPI MADT,唤醒 AP。
2. 每 CPU 运行队列 + 全局队列负载均衡(work stealing)。
3. 抢占式调度器:优先级、时间片、`Condvar`/`Mutex` 内核原语。
4. 跨 CPU 通信:IPI(调度推拉、TLB shootdown)。
5. 服务器区块线程绑定:16×16 区域线程组映射到 CPU(node 亲和)。

### 4.4 系统调用与进程
1. syscall 入口(切换栈、寄存器保存)。
2. 用户态进程/线程模型:地址空间、线程栈、`spawn`/`exit`。
3. IPC:消息传递 + 共享内存,供网络栈服务与服务器进程通信。

## L5 驱动

1. 串口(COM1):早期日志与调试台。
2. 定时器:HPET/APIC 校准。
3. virtio-blk:MMIO 队列,支持 read/write;用于世界存档与 rootfs。
4. virtio-net:收发队列、中断→poll 混合;提供 `net_rx/tx` 接口。
5. 键盘/帧缓冲:后期控制台输出到屏幕(可选,串口优先)。

## L6 系统服务

1. `init` 进程:开机自动拉起服务器进程。
2. TCP/IP 栈(内核内,协议全在 TCP 上):
   - IP/ICMP/ARP;
   - TCP:状态机、重传、窗口、慢启动;
   - 以太网驱动对接 vnet;
   - 提供 socket API(sock 层 + syscall)。
3. 文件系统:
   - 块层 + 简单 FS(ext2 简化版或自定义),支持目录、追加写;
   - 世界目录、日志、配置。
4. 服务器多线程模型:网络线程(收包队列)+ tick 线程 + 区块线程组 + 存档 I/O 线程。

## L7 运行时与语言层

1. Rust no_std:自建 `GlobalAlloc`、栈切换、协程/任务抽象。
2. 平台抽象层(`kernel-backend` trait):宿主(Windows)实现与内核实现双后端,
   服务器核心逻辑与平台解耦。
3. C 热路径库(独立于内核,双方共用):
   - varint/varlong 编解码;
   - chunk section 位打包/解包(4096 状态 → compacted long[]);
   - NBT 编解码;
   - zlib 压缩(对接内核内/宿主 zlib 或自实现 DEFLATE)。
   - 全部有 Rust 参考实现交叉验证(随机属性测试)。

## L8 服务器核心(核心主线)

> 目标版本:MC Java 26.1.2,协议 **775**,data version **4790**。
> 首个里程碑:超平坦世界 + 客户端可进。

### 8.1 协议常量与注册表
1. 固化 v775 包 ID 表(status/login/config/play 各方向)。
2. 注册表数据(block/item/biome 等)来源:内置生成或从 vanilla 提取。

### 8.2 网络与握手
1. 帧层:包长度(变长)+ 压缩(zlib,threshold=256)+ 解压。
2. Handshake → Status(响应 JSON)→ Login(加密/离线)→ Configuration
   → Play,全流程原版客户端可过。
3. 配置阶段:注册表同步、功能开关、资源包协商。

### 8.3 世界与超平坦
1. 世界模型:区块坐标、区块 = 24 个 section(26.x 世界高度)。
2. 区块生成管线:超平坦生成器(固定分层),→ palette → 位打包。
3. 玩家初始生成点、出生点区块预生成。

### 8.4 Play 数据流(最小可玩集)
1. Join Game / Player Info / Position / Chunk Data(bundle)/ 保持 alive。
2. 区块数据包编码(bitmask + sections + biome)经 C 热路径打包 + zlib。
3. 移动同步:PositionLook、客户端确认、服务器端校验(反作弊降级)。

### 8.5 后续玩法(按依赖排序)
1. 物理/重力/碰撞箱,方块放置/破坏,物品栏。
2. 流体、TNT/重力方块。
3. 实体系统与简单 AI(猪/牛/僵尸)、路径搜索。
4. 红石、活塞、矿车;昼夜/天气。
5. 存档 region + NBT 读写(与 M5 对接)。
6. 命令、聊天。

## L9 ISO 打包与部署

1. 可引导介质:GRUB(或自写 bootloader)装载内核二进制。
2. rootfs:initramfs(内核 + init + 服务器二进制 + 世界数据 + 配置)。
3. `genisoimage -b` El Torito 引导扇区 → `dist/mcs.iso`。
4. 验收:QEMU 引导 ISO → 自动拉起服务器 → 原版客户端进世界。

---

## 执行顺序(当前向后)

1. 宿主侧先把「服务器核心 + 超平坦世界 + v775 协议」跑通(M6 优先,最快看到真东西),
   以 `mc-server` crate 形式存在于 workspace,平台抽象层先行。
2. 并行推进内核 L2-L5(自研引导/内存/SMP/驱动)。
3. M6 达成后移植:服务器核心逻辑(纯 Rust,平台无关)跑在微内核上。
4. 网络栈就绪后内核内 socket 直连 → 在 QEMU 里从内核服务器进世界。
5. 打包 ISO 收尾。

## 决策记录

- 语言:主 Rust;热点(位打包/压缩/NBT)走 C FFI;内核入口汇编仅最小必要。
- 内核路线:全自研微内核(x86_64,no_std),兼容多核与 NUMA。
- 版本:MC 26.1.2 / 协议 775 / data 4790。
- 存档格式:目标兼容原版 region(后期)。
