# MCS 进度记录(已完成)

> 只记录已完成、可验证的产出。

## 宿主服务器核心(L3,Rust)

- 协议 775 / MC 26.1.2 目标版本固化。
- 帧层:包长度 varint + zlib 压缩(threshold 256)收发。
- Handshake → Status → Login → Configuration → Play 全流程;
  原版客户端 26.1.2 目录布局对齐(server.properties 原版键、
  logs gzip 轮转、world/region+entities+poi、usercache 等)。
- Login:协议版本校验、Set Compression、Login Success(offline uuid)。
- Configuration:Known Packs / Registry Data / Feature Flags /
  Update Tags / Finish 五包下发。
- Play 最小集:Join Game、Player Info Update、Sync Player Position、
  Keep Alive 20 TPS 节拍。
- 区块下发:Set Center Chunk + Chunk Batch Start + Chunk Data +
  Batch Finished + Set Default Spawn Position;出生点 3x3 网格,
  minecraft-data 26.1 全局 block state ID(1168 blocks/29873 states,
  `blocks_pack.bin`)+ 原生调色板编码(BPE 0/4-8/15)+ WORLD_SURFACE
  高度图 + 全天空光 15。客户端可用 26.1 协议加载区块。
- 方块交互:creative 模式;共享 World(Arc<Mutex>,缓存 + region 持久化),
  Use Item On 放置(手持物品 → 方块)、Player Action 破坏,
  回 Block Update + Acknowledge Block Change;Set Held Item /
  Set Creative Mode Slot 维护虚拟背包(45 格)。
- 注册表管线:`scripts/extract_registry.py` 从原版 client jar 提取
  13 个注册表 → `registry_pack.bin`(MREG 格式,341 entries)→
  `registry.rs` include_bytes! + JSON→NBT 转换 → Registry Data 包。
- 集成测试:tests/network.rs(全流程/压缩/错误协议),registry.rs 单测。

## 内核(L0/L1)

- multiboot2 引导 + 长模式 + GDT + 页表 + 串口输出 + 内存探测,
  QEMU 验证。
- 安装界面:语言/时区/键盘选择、EULA 第一步,Y/N 交互,QEMU 验证。
- 硬浮点:消除 soft-float,SSE2 硬件浮点(验证 mulsd 出现在反汇编,
  软浮点符号为 0)。
- 系统命令行:默认进入,行编辑(退格/Shift/大写),命令
  help/mem/ver/eula/install/reboot/ctrls,QEMU 验证。
- rootfs:内存只读文件系统(Linux FHS 目录树,57 节点),目录
  /bin /boot /dev /etc /home /lib /media /mnt /opt /proc /root /run
  /sbin /srv /sys /tmp /usr /var,含 /etc/os-release、/proc/meminfo
  等演示文件;命令 pwd / cd(相对/绝对路径,支持 / . ..)/
  ls(权限位+大小)/ cat,QEMU 验证。
- systemctl:start/stop/restart/status/list-units,省略
  ".service" 后缀自动匹配;mc-server.service 运行状态与服务器
  控制台共享,QEMU 验证。
- 汉字显示:VGA 文本模式字库重载(plane 2 上传 0x80-0xFF 槽位,
  汉字 16x16 = 左右两个 8x16 槽位),GB2312 16x16 点阵
  (HZK16 262KB)+ 逻辑屏幕缓冲 + 滚动,一屏最多 64 汉字。
- 拼音输入法:Ctrl+Space 中/英切换,前缀查表(206 拼音条目/
  782 常用字),候选黑底窗框(光标下方,1-9 选字,空格/Esc 操作),
  上屏 GB2312 双字节;内核数据包:boot/data/hzk16.bin +
  pinyin_pack.bin(scripts/gen_pinyin.py 生成)。

- 4 核调度器竞态修复:sp 锁内写 + 空闲标记入队锁内,4 分钟压测
  BAD=0/intr=0;提交 e5e2fa7(iso)/a8997f6(nolog)。
- mc-world no_std 化(libm 替代 core 浮点)、内核 GlobalAlloc(kalloc,
  分配/释放包 preempt 保护)、genworld 命令(每核一个生成任务)。
- 偶发 #UD 根因:Chunk 192KB 栈对象溢出 16KB 任务栈(覆盖相邻任务
  栈与 .bss);修复:Chunk.sections 改 Box 堆分配,server 测试全过;
  验证 4 核 genworld 128 区块/4 核/66s 无 #UD、alloc=free 无泄漏。
- 链接布局:relocation-model=static 消除 GOT(ELF 变 EXEC)。
- AP SSE:enable_sse_ap(GAS)在 ap_entry 开头调用。
- 管理命令:tasks(任务表/队列/核数)。
- 部署文档:docs/HARDWARE.md(双路 X99 真机流程)。

- mc-server.service 真服务化:systemctl start 真 spawn 世界生成任务
  (emb::mc_server_task,持续生成区块),stop 置标志停止,status 显示
  running + 已生成区块数;QEMU 验证:running=true chunks=12,ud=0。
- 管理命令:tasks(任务表/就绪队列/核数),systemctl 绑定真实任务。

- 服务器核心嵌入:mc-hotpath(C 热路径 varint/位打包,内核侧编译 native C)
  + mc-protocol(协议编解码)链入内核;pkt 命令验证全链路:
  varint C=R 交叉 1008/1008、Status 帧 roundtrip、真实区块位打包
  → chunk-data 帧;服务器任务持续打包(生成 → 打包计数)。
- 系统完善:任务退出机制(gen_task 完成 exit,槽位/栈内存回收,
  连续 genworld 验证 allocs 256 frees 256);uptime/stats 命令
  (切换计数/每核 tick);help 完整;无 klog 构建 kerr fallback。
- switch 版本切换:注册表 1.0..26.2(真实协议号/数据版本/世界高度/
  打包位数),switch 列版本/切换;status 响应与区块打包按当前版本
  参数化;验证 1.12.2(proto=340,maxY=128)↔ 26.2(proto=776,maxY=384)。

## 决策记录(已定案)

- 语言:Rust 主体 + C 热路径(varint/位打包/NBT),内核入口汇编最小必要。
- 服务器核心整层 Rust,不引入 C++。
- 存档格式目标兼容原版 region。
- 系统目录:类 Linux FHS(见 FILESYSTEM.md)。