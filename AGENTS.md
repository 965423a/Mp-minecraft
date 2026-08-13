# AGENTS.md — 项目协作规则

## 硬性规则(每次会话必须遵守)

1. **每完成一步工作,必须立即上传 GitHub**:
   - `git add -A`
   - `git commit -m "<描述性的英文提交信息>"`
   - `git push origin master:main`
   - 不允许攒多个步骤一次性提交,不允许遗漏上传。
2. 工作过程中如被用户打断,先上传当前已完成部分的成果,再继续下一步。
3. 提交信息用英文,描述这次改动的内容(如 `World generator: add noise/fbm terrain`)。

## 网络环境

- GitHub 走网关节点 `172.23.160.1:443`(hosts 已配置,SNI 反代,自签证书)。
- git 已关闭 SSL 校验(`http.sslVerify false`),凭证在 `~/.git-credentials`,push 免密。
- 默认远程仓库:`git@github.com:965423a/Mp-minecraft.git`(HTTPS remote,凭证自动注入)。

## 构建命令

- 服务器:`cargo build --manifest-path server/Cargo.toml --release`
- 内核:`cargo build --manifest-path boot/Cargo.toml --target x86_64-unknown-none --release`
- ISO:`scripts/build.sh`(一键全构建 + 打包 `dist/mcs.iso`)
- QEMU 测试:`scripts/run.sh`
- 测试:`cargo test --manifest-path server/Cargo.toml`

## 目录

- `server/` MC 服务器(Rust workspace):mc-protocol / mc-world / mc-hotpath / mc-server
- `boot/` 引导 + 最小内核(multiboot2 + 长模式 + 安装界面)
- `sysroot/` ISO 文件系统层(GRUB 配置等)
- `scripts/` 构建脚本
- `dist/` 产物(ISO,gitignore)

## 环境

- Ubuntu 26.04,root 用户。
- Rust nightly(x86_64-unknown-none target),gcc/clang/nasm,qemu-system-x86_64,grub-mkrescue,xorriso 已装。
- apt 走阿里云镜像;GitHub DNS 已被 hosts 修复。