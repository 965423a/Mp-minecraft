#!/usr/bin/env python3
"""从 minecraft-data(pc/26.1)blocks.json 提取全局 block state ID 表。

生成 blocks_pack.bin(供 mc-world 与 mc-server 使用):
  [u32 magic 0x424C4B53 "BLKS"] [u32 version=1]
  [u32 block_count]
  每 block: [u16 namelen][name utf8][u32 default_state][u32 state_count]
  每 state: [u32 global_id][u16 namelen][name utf8]
全局 ID 即 1.13+ 服务端注册表顺序(air=0, stone=1, ...)。
"""
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
        # 每个 block 的 state 列表(按全局 ID 升序,原版注册表顺序)
        states = []
        for sid in range(b["minStateId"], b["maxStateId"] + 1):
            states.append((sid, b["name"]))
        out += struct.pack("<I", len(states))
        for sid, name in states:
            sb = name.encode()
            out += struct.pack("<I", sid) + struct.pack("<H", len(sb)) + sb

    with open(OUT, "wb") as f:
        f.write(bytes(out))
    total_states = sum(b["maxStateId"] - b["minStateId"] + 1 for b in blocks)
    print(f"{len(blocks)} blocks, {total_states} states -> {OUT} ({len(out)/1024:.0f} KiB)")

if __name__ == "__main__":
    sys.exit(main())
