# 系统根目录布局(类 Linux FHS 树状结构)

> 设计原则:让有 Linux 维护经验的用户「回到 Linux 的感觉」,
> 用熟悉的路径习惯快速定位配置、日志、数据与可执行文件。
> 本文覆盖:目录树 + 硬盘分配 + 挂载表 + 服务器核心目录。

## 1. 存储介质模型

系统由三种介质组成,挂载关系清晰:

| 介质 | 内容 | 读写 | 挂载点 |
|---|---|---|---|
| ISO(启动介质) | 内核、引导、基础 rootfs | 只读 | `/`(初始) |
| 系统盘(第一块磁盘) | 系统软件、配置、日志 | 读写 | `/etc /usr /var /home` |
| 数据盘(第二块及以上) | 服务器世界数据、大块数据 | 读写 | `/srv` |

- 启动流程:ISO 引导 → 内核挂载只读 rootfs(内存盘)→
  探测磁盘 → 按挂载表把读写目录挂到系统盘/数据盘。
- 无磁盘时降级:全部在内存盘运行(数据不持久,仅演示)。

## 2. 系统根目录(完整树)

```
/
├── boot/            引导与内核镜像(ISO 承载)
│   ├── grub/        GRUB 配置与模块
│   ├── mcs-kernel   自研内核(ELF)
│   └── mc-server    服务器核心可执行文件
├── bin/             系统基础命令(内核 shell 内建)
├── sbin/            系统管理命令(需特权)
├── etc/             系统级配置(挂系统盘)
│   ├── mcs/         服务器核心配置
│   │   └── server.properties
│   ├── fstab        挂载表(见第 3 节)
│   ├── hosts        主机名/IP 映射
│   └── mcsctl.conf  服务管理器配置
├── srv/             服务数据(挂数据盘)
│   └── minecraft/   MC 服务器核心目录(与原版 jar 一致,见第 4 节)
├── var/             运行时可变数据(挂系统盘)
│   ├── log/         系统与服务器日志
│   │   └── mcs/     latest.log
│   ├── run/         PID/运行状态
│   ├── tmp/         重启后清除的临时数据
│   └── cache/       缓存
├── tmp/             全局临时目录(重启清除)
├── dev/             设备接口(串口/磁盘/网络)
├── proc/            内核运行时信息(CPU/内存/任务)
├── sys/             内核设备树(类比 /sys)
├── usr/             用户态程序与库(挂系统盘)
│   ├── bin/         用户程序
│   └── lib/         共享库
├── home/            用户目录(挂系统盘)
│   └── admin/       管理员用户
├── root/            特权用户目录
├── opt/             可选应用(预留)
├── mnt/             临时手动挂载点
└── media/           可移除介质(USB 等,预留)
```

## 3. 硬盘分配与挂载表

分区策略(类比 Linux,自研格式 MCSFS):

| 分区 | 用途 | 典型大小 | 挂载点 |
|---|---|---|---|
| 系统分区 | 系统软件+配置+日志 | 16 GiB | `/etc /usr /var /home` |
| 数据分区 | 服务器世界数据 | 剩余全部 | `/srv` |
| 交换分区 | 内存压力兜底 | 2 GiB(可选) | 无挂载点 |

`/etc/fstab` 格式(对齐 Linux 习惯):

```
# <device>        <mountpoint> <fs>     <options>      <dump> <pass>
mc0p1             /etc         mcsfs    rw             0      1
mc0p1             /usr         mcsfs    rw             0      1
mc0p1             /var         mcsfs    rw             0      1
mc0p1             /home        mcsfs    rw             0      1
mc1p1             /srv         mcsfs    rw             0      2
```

- `mc0` = 第一块磁盘,`mc1` = 第二块磁盘(多盘自动编号)。
- 挂载顺序:根 → 系统分区 → 数据分区;失败跳过并告警,不阻塞启动。
- 多盘场景:第二块盘整个给 `/srv`;更多盘可配给 `/srv/minecraft/world`
  等子目录(见第 4 节)。
- 管理员用 `mount`/`umount`/`df`(内核 shell 提供)查看与操作,
  与 Linux 体验一致。

## 4. 服务器核心目录(/srv/minecraft,与原版 jar 核心一致)

```
/srv/minecraft/
├── server.jar        核心本体(自研 mc-server,兼容包装名)
├── start.sh          启动脚本(选择 JVM/直启参数)
├── server.properties 配置(权威副本在 /etc/mcs/)
├── eula.txt          EULA 接受标记
├── ops.json          管理员 UUID 列表
├── whitelist.json    白名单
├── banned-players.json
├── banned-ips.json
├── bin/              运行时二进制与库
├── libraries/        依赖库目录(结构预留,自研核心暂不使用)
├── config/           服务器核心附加配置
├── world/            主世界
│   ├── level.dat     世界元数据(种子/时间/难度/玩家)
│   ├── region/       r.{x}.{z}.mcr 区块文件
│   ├── entities/     实体数据(预留)
│   └── poi/          兴趣点数据(预留)
├── logs/
│   ├── latest.log    本次运行日志
│   └── *.log.gz      历史日志(轮转)
└── crash-reports/    崩溃报告(预留)
```

所有文件名、格式与原版 jar 服务器对齐;管理员可沿用既有
运维脚本与习惯(改 properties、查 logs、编辑 ops.json)。

## 5. 目录职责划分

| 路径 | 内容 | 谁写 |
|---|---|---|
| /etc/mcs | 配置(只读惯例) | 管理员 |
| /srv/minecraft/world | 世界数据 | 服务器核心 |
| /srv/minecraft/logs | 日志 | 服务器核心 |
| /var/log/mcs | 系统侧日志聚合(链接到 logs) | 系统服务 |
| /var/run/mcs | PID、状态 | 服务管理器 |
| /dev /proc /sys | 内核接口 | 内核 |
| /tmp /var/tmp | 临时 | 任意进程 |

## 6. 落地步骤

1. ISO(sysroot)建立 boot/ + etc/mcs/ 骨架;内核启动后按 /etc/fstab
   挂载磁盘,无盘则内存盘降级。
2. 内核 shell 提供 `mount`/`ls`/`cat`/`df` 等基础命令。
3. mc-server 初始化时:创建 /srv/minecraft 全套目录与
   server.properties/eula.txt/ops.json 等文件(与原版首启行为一致)。
4. 服务管理器(mcsctl,Python 上层)负责 start/stop/status,
   读取 /etc/mcs,写 /var/run。