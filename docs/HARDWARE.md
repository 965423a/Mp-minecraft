# 硬件适配:科脑双路 X99 + E5-2680v4

目标平台:双路 X99 主板,2× E5-2680v4(28 核 56 线程),2× 16GB DDR4 ECC。
内核相关子系统:NUMA(SRAT/SLIT)、多核唤醒(AP trampoline)、IOAPIC/8259
路由、LAPIC 定时器。

## 启动参数(QEMU 等价验证)

QEMU 4 核双节点拓扑与真机一致:

```
-smp 4,sockets=2,cores=2,threads=1
-numa node,nodeid=0,cpus=0-1,memdev=mem0
-numa node,nodeid=1,cpus=2-3,memdev=mem1
-object memory-backend-ram,size=2G,id=mem0
-object memory-backend-ram,size=2G,id=mem1
```

## 真机启动流程

1. UEFI 引导:主板启动项选择 U 盘(GRUB2,multiboot2)。
2. 内核自检顺序:
   - 串口 COM1(115200 8N1)输出启动日志;
   - 8259 重映射 → IOAPIC 枚举(MADT)→ 键盘/定时器路由;
   - APIC 定时器校准(tsc_per_us);
   - SRAT 内存亲和 → NUMA 节点帧链;
   - AP 唤醒(IPI INIT/SIPI,node-local 栈);
   - 调度器启动,demo 任务 + shell。
3. shell 验证命令:`numa`(拓扑/分配)、`mem`(每节点内存)、`tasks`(任务表)、
   `genworld`(多核世界生成)。

## 真机验证清单

- [ ] COM1 日志完整(无乱码/撕裂,锁生效)
- [ ] 56 线程全部 online(AP 唤醒,node-local 栈)
- [ ] `numa` 显示 2 节点,内存与真机容量一致
- [ ] 键盘输入可用(IOAPIC → LAPIC 路由,IRQ1)
- [ ] `genworld` 4/56 核并行,结果与 QEMU 一致(seed 确定性)
- [ ] 长时间压测无 #UD/#GP/BAD FRAME
- [ ] ECC/内存错误无异常(真机 BIOS 内存报告)

## 已知 QEMU 与真机差异

- QEMU TCG 下 IOAPIC 外部中断投递不可靠,键盘走轮询兜底;
  真机 IOAPIC 投递正常。
- QEMU 虚拟 LAPIC 定时器与 TSC 关系理想;真机需校准正确(已实现)。
- 真机启动早期有 A20/PCI/内存初始化延迟,AP 唤醒需容忍。

## 排障

- AP 唤醒失败:检查 tramp 复制区(0x7000)与 PARAM(gdt/cr3/stack/ready)。
- #UD:浮点指令需 CR4.OSFXSR(AP 侧 enable_sse_ap 已处理)。
- 中断风暴:检查 EOI 顺序(tick 先 EOI 再切栈)。
- 内存错误:先跑 `mem`/`numa`,确认帧链与 E820/SRAT 一致。