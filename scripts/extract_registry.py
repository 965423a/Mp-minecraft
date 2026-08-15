#!/usr/bin/env python3
"""从 26.1.2 client jar 提取注册表 JSON,打包为 registry_pack.bin(zlib 压缩)。"""
import sys, zipfile, zlib, struct

JAR = "/tmp/client-26.1.2.jar"
OUT = "server/crates/mc-server/registry_pack.bin"

REGS = [
    "worldgen/biome",
    "dimension_type",
    "chat_type",
    "damage_type",
    "banner_pattern",
    "trim_pattern",
    "trim_material",
    "painting_variant",
    "wolf_variant",
    "cat_variant",
    "instrument",
    "jukebox_song",
    "enchantment",
]

def main():
    z = zipfile.ZipFile(JAR)
    entries = []
    for name in sorted(z.namelist()):
        parts = name.split("/")
        if len(parts) < 4 or parts[0] != "data" or parts[1] != "minecraft":
            continue
        reg = f"{parts[2]}/{parts[3]}" if len(parts) >= 5 else parts[2]
        if reg not in REGS:
            continue
        entry = parts[3] if len(parts) < 5 else parts[4]
        if not entry.endswith(".json"):
            continue
        entry = entry[:-5]
        data = z.read(name)
        regname = f"minecraft:{reg}"
        key = f"minecraft:{entry}"
        entries.append((regname, key, data))

    out = bytearray()
    out += struct.pack("<4sII", b"MREG", 1, len(entries))
    for regname, key, data in entries:
        nb = regname.encode()
        kb = key.encode()
        zb = zlib.compress(data, 9)
        out += struct.pack("<H", len(nb)) + nb
        out += struct.pack("<H", len(kb)) + kb
        out += struct.pack("<I", len(zb)) + zb

    with open(OUT, "wb") as f:
        f.write(bytes(out))
    print(f"{len(entries)} entries -> {OUT} ({len(out)/1024:.0f} KiB)")

if __name__ == "__main__":
    sys.exit(main())
