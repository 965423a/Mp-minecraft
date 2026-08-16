#!/usr/bin/env python3
"""从 minecraft-data(pc/26.1)blocks.json 生成 blocks_pack.bin(全局 block state ID 表,全局 ID 即 1.13+ 服务端注册表顺序 air=0)。"""
import json, os, struct, sys, urllib.request

SRC_URL = "https://raw.githubusercontent.com/PrismarineJS/minecraft-data/master/data/pc/26.1/blocks.json"
OUT = "server/crates/mc-world/blocks_pack.bin"

def main():
    if not os.path.exists("/tmp/md-blocks.json"):
        print(f"downloading {SRC_URL}")
        urllib.request.urlretrieve(SRC_URL, "/tmp/md-blocks.json")
    blocks = json.load(open("/tmp/md-blocks.json"))
    assert blocks[0]["name"] == "air" and blocks[0]["minStateId"] == 0

    out = bytearray()
    out += struct.pack("<4sII", b"BLKS", 1, len(blocks))
    for b in blocks:
        nb = b["name"].encode()
        out += struct.pack("<H", len(nb)) + nb
        out += struct.pack("<I", b["defaultState"])
        out += struct.pack("<I", b["maxStateId"] - b["minStateId"] + 1)
        for sid in range(b["minStateId"], b["maxStateId"] + 1):
            sb = b["name"].encode()
            out += struct.pack("<I", sid) + struct.pack("<H", len(sb)) + sb

    with open(OUT, "wb") as f:
        f.write(bytes(out))
    total_states = sum(b["maxStateId"] - b["minStateId"] + 1 for b in blocks)
    print(f"{len(blocks)} blocks, {total_states} states -> {OUT} ({len(out)/1024:.0f} KiB)")

if __name__ == "__main__":
    sys.exit(main())
