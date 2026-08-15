#!/usr/bin/env python3
import argparse
import os
import struct
import subprocess
import sys

MAGIC = b"MCSR"
SECTIONS = 24
BITS = 6
PER_LONG = 64 // BITS
MASK = (1 << BITS) - 1

CHARS = {
    0: " ",
    1: ".",
    2: "g",
    3: "d",
    4: "c",
    7: "b",
    9: "~",
    12: "s",
    13: "G",
    17: "T",
    18: "L",
    80: "S",
    79: "I",
    82: "C",
    37: "D",
    38: "P",
}


def load_chunks(path):
    with open(path, "rb") as f:
        buf = f.read()
    if len(buf) < 21 or buf[:4] != MAGIC:
        raise ValueError(f"bad region magic: {path}")
    version = struct.unpack_from(">I", buf, 4)[0]
    seed = struct.unpack_from(">Q", buf, 8)[0]
    wtype = buf[16]
    count = struct.unpack_from(">I", buf, 17)[0]
    if version == 1:
        pos = 21
        cx, cz = struct.unpack_from(">ii", buf, pos); pos += 8
        chunks = [read_chunk(buf, pos, cx, cz, seed, wtype)]
        return chunks
    if version != 2:
        raise ValueError(f"unsupported version {version}: {path}")
    pos = 21
    chunks = []
    for _ in range(count):
        cx, cz, off, ln = struct.unpack_from(">iiII", buf, pos); pos += 16
        chunks.append(read_chunk(buf, off, cx, cz, seed, wtype))
    return chunks


def read_chunk(buf, pos, cx, cz, seed, wtype):
    sections = []
    for _ in range(SECTIONS):
        bits = buf[pos]; pos += 1
        n = struct.unpack_from(">I", buf, pos)[0]; pos += 4
        if bits == 0:
            sections.append([0] * 4096)
            continue
        longs = struct.unpack_from(f">{n}Q", buf, pos); pos += n * 8
        blocks = [0] * 4096
        for i in range(4096):
            w, o = i // PER_LONG, (i % PER_LONG) * BITS
            blocks[i] = (longs[w] >> o) & MASK
        sections.append(blocks)
    return cx, cz, seed, wtype, sections


def heightmap(chunk):
    cx, cz, seed, wtype, sections = chunk
    hm = {}
    for sx, sec in enumerate(sections):
        if not any(sec):
            continue
        base_y = -64 + sx * 16
        for i, b in enumerate(sec):
            if b:
                x, z = i % 16, (i // 16) % 16
                y = base_y + i // 256
                hm[(x, z)] = max(hm.get((x, z), -10_000), y)
    return hm


def render(world_dir, cols=120):
    files = sorted(os.listdir(os.path.join(world_dir, "region")))
    if not files:
        print("no region files"); return
    chunks = {}
    seed = wtype = None
    for fn in files:
        if not fn.endswith(".mcr"):
            continue
        for c in load_chunks(os.path.join(world_dir, "region", fn)):
            chunks[(c[0], c[1])] = c
            seed, wtype = c[2], c[3]
    if not chunks:
        print("no valid chunks"); return
    xs = [c[0] for c in chunks]; zs = [c[1] for c in chunks]
    cx0, cx1 = min(xs), max(xs); cz0, cz1 = min(zs), max(zs)
    w, h = (cx1 - cx0 + 1) * 16, (cz1 - cz0 + 1) * 16
    scale = max(1, (max(w, h) + cols - 1) // cols)
    grid = [[" "] * (w // scale) for _ in range(h // scale)]
    water = 0
    for (cx, cz), c in chunks.items():
        hm = heightmap(c)
        for (x, z), y in hm.items():
            if y < -1000:
                continue
            px, pz = (cx - cx0) * 16 + x, (cz - cz0) * 16 + z
            gx, gz = px // scale, pz // scale
            if gx < len(grid[0]) and gz < len(grid):
                b = c[4][(y - -64) // 16][(x % 16) + (z % 16) * 16 + ((y + 64) % 16) * 256]
                if b == 9:
                    water += 1
                grid[gz][gx] = CHARS.get(b, "?")
    print(f"seed={seed} world_type={wtype} chunks={len(chunks)} water={water * 100 // (w * h)}%")
    for row in grid:
        print("".join(row))


def iso_check(path):
    with open(path, "rb") as f:
        data = f.read()
    def u32(o):
        return struct.unpack_from("<I", data, o)[0]
    pvd = 16 * 2048
    if data[pvd + 1:pvd + 6] != b"CD001":
        print("not an ISO9660 image"); return 1
    vol = data[pvd + 40:pvd + 72].split(b";")[0].decode("ascii", "replace").strip()
    boot = 17 * 2048
    ok = True
    if data[boot:boot + 6] != b"\x00CD001":
        print("no El Torito boot record"); ok = False
    cat = u32(boot + 71) * 2048
    entries = []
    for i in range(8):
        e = data[cat + i * 32:cat + (i + 1) * 32]
        if not e or len(e) < 32 or e[0] == 0x00:
            break
        entries.append(e)
    no_emul = [e for e in entries if e[0] == 0x88 and e[1] == 0x00]
    print(f"volume: {vol}")
    print(f"boot catalog: {len(entries)} entries, no-emulation={len(no_emul)}")
    if not no_emul:
        print("missing no-emulation boot entry"); ok = False
    if b"BOOTX64.EFI" not in data:
        print("missing EFI/BOOT/BOOTX64.EFI"); ok = False
    mbr_boot = data[510:512] == b"\x55\xaa"
    gpt = data[512:520] == b"EFI PART"
    apm = data[512:514] == b"PM"
    part_type = data[450] if len(data) > 450 else 0
    hybrid = mbr_boot and (gpt or apm)
    print(f"hybrid: MBR={mbr_boot} GPT={gpt} APM={apm} partition_type=0x{part_type:02x}")
    if not hybrid:
        print("not hybrid MBR/GPT"); ok = False
    print("ISO OK" if ok else "ISO CHECK FAILED")
    return 0 if ok else 1


def build():
    here = os.path.dirname(os.path.abspath(__file__))
    return subprocess.call([os.path.join(here, "build.sh")], cwd=os.path.join(here, ".."))


def main():
    ap = argparse.ArgumentParser(prog="mcs-admin")
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("preview", help="render world ascii heightmap")
    p.add_argument("world_dir", help="path to world/ directory")
    p.add_argument("-c", "--cols", type=int, default=120)
    sub.add_parser("iso", help="verify ISO boot structure").add_argument("iso")
    sub.add_parser("build", help="run full build + ISO")
    args = ap.parse_args()
    if args.cmd == "preview":
        render(args.world_dir, args.cols)
    elif args.cmd == "iso":
        sys.exit(iso_check(args.iso))
    elif args.cmd == "build":
        sys.exit(build())


if __name__ == "__main__":
    main()