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

## 决策记录(已定案)

- 语言:Rust 主体 + C 热路径(varint/位打包/NBT),内核入口汇编最小必要。
- 服务器核心整层 Rust,不引入 C++。
- 存档格式目标兼容原版 region。
- 系统目录:类 Linux FHS(见 FILESYSTEM.md)。