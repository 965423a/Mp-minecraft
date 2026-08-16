#!/usr/bin/env python3
"""从 minecraft-data(pc/26.1)items.json 生成 items_pack.bin(物品注册表 ID 表)。"""
import json, os, struct, sys, urllib.request

SRC_URL = "https://raw.githubusercontent.com/PrismarineJS/minecraft-data/master/data/pc/26.1/items.json"
OUT = "server/crates/mc-world/items_pack.bin"

def main():
    if not os.path.exists("/tmp/md-items.json"):
        print(f"downloading {SRC_URL}")
        urllib.request.urlretrieve(SRC_URL, "/tmp/md-items.json")
    items = json.load(open("/tmp/md-items.json"))
    ids = {i["id"] for i in items}
    for i in range(len(items)):
        assert i in ids, f"item id {i} missing (registry must be dense)"

    out = bytearray()
    out += struct.pack("<4sII", b"ITMS", 1, len(items))
    for i in items:
        nb = i["name"].encode()
        out += struct.pack("<I", i["id"]) + struct.pack("<H", len(nb)) + nb

    with open(OUT, "wb") as f:
        f.write(bytes(out))
    print(f"{len(items)} items -> {OUT} ({len(out)/1024:.0f} KiB)")

if __name__ == "__main__":
    sys.exit(main())
