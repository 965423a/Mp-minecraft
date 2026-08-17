# 系统根目录布局(类 Linux FHS 树状结构)

> 设计原则:目录树对齐 Linux FHS 习惯,便于按熟悉路径定位
> 配置、日志、数据与可执行文件。目录树为定案;服务器核心目录
> 已实现(与原版 jar 一致)。

## 1. 系统根目录(定案树)

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

## 2. 服务器核心目录(/srv/minecraft,与原版 jar 核心一致)

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

## 3. 目录职责划分

| 路径 | 内容 | 谁写 |
|---|---|---|
| /etc/mcs | 配置(只读惯例) | 管理员 |
| /srv/minecraft/world | 世界数据 | 服务器核心 |
| /srv/minecraft/logs | 日志 | 服务器核心 |
| /var/log/mcs | 系统侧日志聚合(链接到 logs) | 系统服务 |
| /var/run/mcs | PID、状态 | 服务管理器 |
| /dev /proc /sys | 内核接口 | 内核 |
| /tmp /var/tmp | 临时 | 任意进程 |