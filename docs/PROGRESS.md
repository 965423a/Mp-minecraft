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
- 汉字显示:VGA 文本模式字库重载(plane 2 上传 0x80-0xFF 槽位,
  汉字 16x16 = 左右两个 8x16 槽位),GB2312 16x16 点阵
  (HZK16 262KB)+ 逻辑屏幕缓冲 + 滚动,一屏最多 64 汉字。
- 拼音输入法:Ctrl+Space 中/英切换,前缀查表(206 拼音条目/
  782 常用字),候选黑底窗框(光标下方,1-9 选字,空格/Esc 操作),
  上屏 GB2312 双字节;内核数据包:boot/data/hzk16.bin +
  pinyin_pack.bin(scripts/gen_pinyin.py 生成)。
- 服务器控制台:`ctrls` 进入,help/start/stop/list/say(支持中文
  广播)/version/exit,exit 返回系统命令行,QEMU 验证。

## 决策记录(已定案)

- 语言:Rust 主体 + C 热路径(varint/位打包/NBT),内核入口汇编最小必要。
- 服务器核心整层 Rust,不引入 C++。
- 存档格式目标兼容原版 region。
- 系统目录:类 Linux FHS(见 FILESYSTEM.md)。