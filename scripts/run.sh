#!/usr/bin/env bash
# 在 QEMU 中运行 MCS ISO
set -euo pipefail
cd "$(dirname "$0")/.."
ISO="${1:-dist/mcs.iso}"
exec qemu-system-x86_64 \
    -m 512M \
    -machine q35 \
    -cdrom "$ISO" \
    -boot d \
    -display default,show-cursor=on \
    -serial stdio \
    -no-reboot \
    -nic user,model=virtio-net-pci