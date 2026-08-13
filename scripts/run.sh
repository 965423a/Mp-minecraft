#!/usr/bin/env bash
# 在 QEMU 中运行 MCS ISO
set -euo pipefail
cd "$(dirname "$0")/.."
ISO="${1:-dist/mcs.iso}"
ACCEL=()
if [ -e /dev/kvm ] && qemu-system-x86_64 -accel kvm -machine none 2>/dev/null; then
    ACCEL=(-enable-kvm -cpu host)
    echo "[run] KVM hardware acceleration"
else
    ACCEL=(-cpu max)
    echo "[run] KVM unavailable, falling back to TCG (software emulation)"
fi
exec qemu-system-x86_64 \
    -m 512M \
    -machine q35 \
    "${ACCEL[@]}" \
    -cdrom "$ISO" \
    -boot d \
    -display default,show-cursor=on \
    -serial stdio \
    -no-reboot \
    -nic user,model=virtio-net-pci