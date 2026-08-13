#!/usr/bin/env bash
# 一键构建:MCS 服务器 + 内核 + ISO
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$(pwd)"
source "$HOME/.cargo/env"

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

# UEFI 可执行文件:让 Rufus/固件能直接识别 ISO 的 UEFI 支持
grub-mkimage -O x86_64-efi \
    -o "$ISO_ROOT/EFI/BOOT/BOOTX64.EFI" \
    -p /boot/grub \
    iso9660 serial terminal normal configfile search search_fs_file \
    test echo ls cat multiboot2 efi_gop efi_uga efi_console \
    part_msdos part_gpt fat ext2 2>/dev/null || true

echo "==> [4/4] mkisofs (grub-mkrescue)"
grub-mkrescue -o "$ROOT/dist/mcs.iso" "$ISO_ROOT" -- -volid MCS -padding 0 2>&1 | tail -2
echo "==> ISO ready: $ROOT/dist/mcs.iso (BIOS + UEFI hybrid)"
ls -lh "$ROOT/dist/mcs.iso"