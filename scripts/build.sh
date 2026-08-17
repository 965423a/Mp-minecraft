#!/usr/bin/env bash
# 一键构建:MCS 服务器 + 内核 + ISO
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$(pwd)"
source "$HOME/.cargo/env"

# 版本化产物:ISO 名 = mcs-<版本>.iso(版本取 VERSION 环境变量,否则取 git 描述)。
# 本地打包与 GitHub Release 资产统一用同名文件。
VERSION="${VERSION:-$(git describe --tags --always 2>/dev/null || echo dev)}"
ISO_NAME="mcs-${VERSION}.iso"
ISO="$ROOT/dist/$ISO_NAME"
DEST_DIR="${DEST_DIR:-/mnt/d}"

echo "==> [1/4] cargo build (server)"
cargo build --manifest-path "$ROOT/server/Cargo.toml" --release

echo "==> [2/4] cargo build (kernel)"
cargo build --manifest-path "$ROOT/boot/Cargo.toml" --target x86_64-unknown-none --release

echo "==> [3/4] stage ISO root"
ISO_ROOT="$ROOT/dist/iso-root"
rm -rf "$ISO_ROOT"
mkdir -p "$ISO_ROOT/boot/grub" "$ISO_ROOT/EFI/BOOT"
cp "$ROOT/boot/target/x86_64-unknown-none/release/mcs-kernel" "$ISO_ROOT/boot/mcs-kernel"
cp "$ROOT/sysroot/boot/grub/grub.cfg" "$ISO_ROOT/boot/grub/grub.cfg"
cp "$ROOT/server/target/release/mc-server" "$ISO_ROOT/boot/mc-server" 2>/dev/null || true

# UEFI 可执行文件:精简模块集即可 —— 驱动是运行时按需加载的,
# grub.cfg 里 insmod all_video/efi_gop 显式加载(完整模块集并非必需)。
grub-mkimage -O x86_64-efi \
    -o "$ISO_ROOT/EFI/BOOT/BOOTX64.EFI" \
    -p /boot/grub \
    iso9660 serial terminal normal configfile search search_fs_file \
    test echo ls cat multiboot2 videoinfo \
    video video_fb gfxterm font all_video efi_gop \
    part_msdos part_gpt fat ext2
mkdir -p "$ISO_ROOT/boot/grub/fonts"
cp /usr/share/grub/unicode.pf2 "$ISO_ROOT/boot/grub/fonts/unicode.pf2" 2>/dev/null || true

echo "==> [4/4] mkisofs (grub-mkrescue)"
grub-mkrescue -o "$ISO" "$ISO_ROOT" -- -volid MCS -padding 0 2>&1 | tail -2
echo "==> ISO ready: $ISO (BIOS + UEFI hybrid)"
ls -lh "$ISO"

# 拷贝到 Windows D 盘(与本地 dist 同名)
if [ -d "$DEST_DIR" ] && [ -w "$DEST_DIR" ]; then
    cp -f "$ISO" "$DEST_DIR/$ISO_NAME"
    echo "==> copied to $DEST_DIR/$ISO_NAME"
else
    echo "==> WARNING: $DEST_DIR not writable, skip D: copy"
fi