# Threaded Regions 设计(吸收 Folia 调度逻辑)

> 参考 Folia(Paper 多线程分叉)的 region 线程模型。
> 目的:世界按空间分区并行 tick,消除单主线程瓶颈;
> 与 SMP-NUMA.md 的 per-CPU 队列 + 工作窃取配合落地。

## 1. 核心模型

- 世界按 **region section 网格**分区(默认 2^4 = 16×16 chunks 一格)。
- 每个独立 region 有专属 tick 循环,20 TPS,并行跑在线程池上。
- **没有主线程**。每个 region 相当于自己世界的"主线程"。
- region 之间**并行但非并发**:不共享数据,不允许跨 region 访问;
  违反即数据损坏。只有少数全局点(连接管理/控制台)例外。

## 2. 四不变式(regionizer 维护)

1. ticking 中的 region 不得扩张(chunk 加载期间不能抢地盘)。
2. 任意 region section 的 merge radius 内邻居,必须属于同一 region
   或是待合并目标(保证 region 有缓冲,可安全 tick 产生新 chunk)。
3. region 不得拥有相邻的 ticking region(相邻必须合并)。
4. 多段不相连的 region 最终拆分为独立 region(保并行度)。

状态机:transient(不可 tick)→ ready → ticking → dead。
- `tryMarkTicking`:ready→ticking,失败返回 false(可能已被降级 transient)。
- `markNotTicking`:ticking→ready;先处理 pending merges,再判断
  是否要转入 transient(待并入他人),否则尝试 split 后回到 ready。
- merge 只允许 dead region 并入 transient/ready 的 region;
  "merge later" 处理 ticking 中的邻居:ticking 结束后再合并。

## 3. 调度:EDF(earliest start time)

- 每 region 一个 repeating task,按 start time 排序调度。
- deadline = start + 50ms → 行为等同 EDF;线程池不饱和时,
  tick 耗时 ≤ 50ms 的 region 都稳定 20 TPS。
- 线程池**不做 NUMA/核亲和**——这正是我们内核层的增强点:
  region tick 任务入 node 本地队列(按 region 所在位置 hash 分 node),
  空闲核从同 node 窃取,失衡才跨 node(见 SMP-NUMA.md)。
- 配置项(借鉴 Folia):threads(线程数,-1 auto)、
  gridExponent(region 边长 2^n chunks,默认 4)、scheduler(EDF/FIFO)。

## 4. 时间语义(红石时序正确性的关键)

- **current tick / redstone tick 每 region 独立维护**。
- global game time、daylight time 由 **global region**(单例,20 TPS)
  维护;每 region 在 tick 开始时拷贝一份,整个 tick 内读拷贝值,
  保证同 tick 内多次读取一致。
- 计划任务/区块事件按绝对 tick 存 deadline:
  - **merge 时**:两 region tick 数不同,deadline 平移
    `offset = to_tick - from_tick`,redstone tick 与 current tick
    分别算(redstone 可被 level.tickTime 关闭)。
  - **split 时**:子 region 继承父 tick 数,相对 deadline 不变。
- 红石/计划 tick 用相对 deadline 存储,天然免疫 merge/split。

## 5. RegionizedData(区域本地数据)

- 每 region 一份:entities、chunks、block/fluid tick lists、tick count。
- 只允许 owner 线程访问;其他线程访问 = 硬错误。
- merge/split 回调迁移数据:
  - `merge(from, into, fromTickOffset)`:from 并入 into,deadline 平移。
  - `split(from, regionToData, dataSet)`:按 section 坐标分给新 region,
    无 tick offset(子 region 继承父 tick)。
- 我们在 Rust 中落地:per-region `RegionData` 结构体 + 线程所有权检查
  (类似 TickThread.isTickThreadFor:当前线程 = region owner 才允许访问)。

## 6. 跨 region 任务路由

- `schedule_chunk_task(x, z, f)`:路由到拥有该 chunk 的 region,
  下个 tick 执行;用于跨 region 改方块/实体。
- `schedule_chunk_task_eventually`(防死锁):调用方持锁时
  不立即路由,先入全局队列,由 global tick 稍后路由。
  避免 ticket lock ↔ schedule lock 环形等待。
- EntityScheduler:任务跟实体走,实体跨 region 时任务随之转移。
- 执行前一律校验线程所有权,错误访问 fail fast。

## 7. 与 SMP-NUMA.md 的接合

| Folia 概念 | 我们的落地 |
|---|---|
| SchedulerThreadPool(EDF) | 内核 per-CPU 队列 + 工作窃取(node 本地优先) |
| ThreadedRegion | 每 region 一个 tick 任务,hash 到 node |
| GlobalRegion(单例 20 TPS) | 全局 tick 线程:连接管理/控制台/天气/时间 |
| RegionizedWorldData | Rust `RegionData`,owner 线程独占 |
| RegionizedTaskQueue | 跨 region 消息队列(无锁 ring 或 CAS 队列) |
| TickThread.isTickThreadFor | 线程局部 owner id 校验 |

## 8. 落地顺序

1. `mc-sched`(或 mc-server 内)先做单 node 原型:
   regionizer + EDF 调度 + RegionizedData + 任务路由。
2. 验证:多个 region 并行 tick,红石机器跨 region 行为与
   单线程参考实现一致(0-tick 依赖 update 顺序,须逐 tick 对齐)。
3. 内核层:AP 启动 + per-CPU 队列就绪后,把 EDF 调度迁到内核线程。
4. 真机(双路)验证 NUMA 亲和与负载均衡。

## 9. 风险与对策

- **红石跨 region 时序**:Folia 已证明 region 隔离 + deadline 平移
  可保正确性;我们额外要求 update 队列顺序与原版一致(见 ROADMAP B 阶段)。
- **0-tick**:是单 region 内更新顺序的自然产物,region 模型不影响;
  跨 region 的 0-tick 机器需按原版顺序排队(计划 tick + BlockEvent)。
- **插件生态**:Bukkit/Folia 插件 API 兼容暂缓,调度模型先行
  (API 语义可日后按同样 region 语义设计 Rust 版)。