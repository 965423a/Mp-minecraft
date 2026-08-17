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

# UEFI 可执行文件:用完整模块集(与 grub-mkrescue 模板/发行版 ISO 行为一致)。
# 之前最小模块集导致 efi_gop 驱动在 Hyper-V 的 GOP 上切换模式失败
# ("no suitable video mode found"),Ubuntu/Kali 用完整 GRUB 却能正常显示。
EFI_MODS=$(for m in /usr/lib/grub/x86_64-efi/*.mod; do basename "$m" .mod; done | tr '\n' ' ')
grub-mkimage -O x86_64-efi \
    -o "$ISO_ROOT/EFI/BOOT/BOOTX64.EFI" \
    -p /boot/grub \
    $EFI_MODS
mkdir -p "$ISO_ROOT/boot/grub/fonts"
cp /usr/share/grub/unicode.pf2 "$ISO_ROOT/boot/grub/fonts/unicode.pf2" 2>/dev/null || true

echo "==> [4/4] mkisofs (grub-mkrescue)"
grub-mkrescue -o "$ROOT/dist/mcs.iso" "$ISO_ROOT" -- -volid MCS -padding 0 2>&1 | tail -2
echo "==> ISO ready: $ROOT/dist/mcs.iso (BIOS + UEFI hybrid)"
ls -lh "$ROOT/dist/mcs.iso"