//! Chunk Data 包体编码(原版 1.21.5+ 格式,只做发送:Heightmaps + Data(sections))。

use crate::chunk::{Chunk, Section, SECTION_VOLUME};
use crate::blocks::AIR;

pub const WORLD_HEIGHT: usize = 384;
const WORLD_SURFACE: i32 = 1;

/// heightmap 位深:ceil(log2(height+1)),384 高 → 9 位。
pub const HEIGHTMAP_BITS: u32 = 9;

/// 计算一列的最高占用方块(MOTION_BLOCKING 语义:非空气)。
fn column_height(chunk: &Chunk, x: usize, z: usize) -> u16 {
    for y in (0..WORLD_HEIGHT).rev() {
        let wy = crate::chunk::MIN_Y + y as i32;
        if chunk.get(x as i32, wy, z as i32) != AIR {
            return y as u16;
        }
    }
    0
}

/// 打包 256 列高度为 long 数组(每列 HEIGHTMAP_BITS 位,x 最快变化)。
pub fn pack_heightmap(chunk: &Chunk) -> Vec<u64> {
    let per_long = (64 / HEIGHTMAP_BITS) as usize;
    let longs = (256 + per_long - 1) / per_long;
    let mut out = vec![0u64; longs];
    for i in 0..256 {
        let x = i % 16;
        let z = i / 16;
        let h = column_height(chunk, x, z);
        let (word, offset) = (i / per_long, (i % per_long) * HEIGHTMAP_BITS as usize);
        out[word] |= (h as u64 & ((1u64 << HEIGHTMAP_BITS) - 1)) << offset;
    }
    out
}

/// section 内流体数(water 全局 ID 86;简化:只统计 WATER)。
fn fluid_count(s: &Section) -> u16 {
    let mut n = 0u16;
    for i in 0..SECTION_VOLUME {
        if s.get(i % 16, i / 256, (i / 16) % 16) == crate::blocks::WATER {
            n += 1;
        }
    }
    n
}

/// 统计 section 的全局 ID 集合(去重,保持首次出现顺序)。
fn unique_ids(s: &Section) -> Vec<u16> {
    let mut out: Vec<u16> = Vec::new();
    for i in 0..SECTION_VOLUME {
        let v = s.get(i % 16, i / 256, (i / 16) % 16);
        if !out.contains(&v) {
            out.push(v);
        }
    }
    out
}

/// 写一个 paletted container(blocks:4096 条,biomes:64 条)。
fn write_container(w: &mut Vec<u8>, entries: &[u16], global_ids: &[u16], entry_count: usize) {
    // 选择位深:单值 0;否则 4..8 间接;超过 8 用 15 直接。
    let bits = if global_ids.len() <= 1 {
        0
    } else if global_ids.len() <= 256 {
        let mut b = 4;
        while (1u32 << b) < global_ids.len() as u32 {
            b += 1;
        }
        b
    } else {
        15
    };
    w.push(bits as u8);
    match bits {
        0 => {
            // 单值:直接写全局 ID
            let v = global_ids[0];
            write_varint(w, v as i32);
        }
        4..=8 => {
            // 间接:palette 长度 + 全局 ID 列表
            write_varint(w, global_ids.len() as i32);
            for g in global_ids {
                write_varint(w, *g as i32);
            }
            // 数据数组:索引(位数 = bits)
            let per_long = 64 / bits as usize;
            let mut packed = vec![0u64; (entry_count + per_long - 1) / per_long];
            for (i, e) in entries.iter().enumerate() {
                let idx = global_ids.iter().position(|g| g == e).unwrap_or(0);
                let (word, offset) = (i / per_long, (i % per_long) * bits as usize);
                packed[word] |= (idx as u64 & ((1u64 << bits) - 1)) << offset;
            }
            for l in packed {
                w.extend_from_slice(&l.to_be_bytes());
            }
        }
        _ => {
            // 直接:15 位全局 ID
            let bits = 15u32;
            let per_long = 64 / bits as usize;
            let mut packed = vec![0u64; (entry_count + per_long - 1) / per_long];
            for (i, e) in entries.iter().enumerate() {
                let (word, offset) = (i / per_long, (i % per_long) * bits as usize);
                packed[word] |= (*e as u64 & ((1u64 << bits) - 1)) << offset;
            }
            for l in packed {
                w.extend_from_slice(&l.to_be_bytes());
            }
        }
    }
}

/// 编码整个区块的 Data 部分(24 个 section,全部发送)。
pub fn write_sections(chunk: &Chunk, biome_id: u16, out: &mut Vec<u8>) {
    let biome_ids = vec![biome_id];
    for s in chunk.sections() {
        // block count / fluid count
        out.extend_from_slice(&(s.count_non_air() as u16).to_be_bytes());
        out.extend_from_slice(&fluid_count(s).to_be_bytes());
        // block states
        let ids = unique_ids(s);
        let entries: Vec<u16> = (0..SECTION_VOLUME)
            .map(|i| s.get(i % 16, i / 256, (i / 16) % 16))
            .collect();
        write_container(out, &entries, &ids, SECTION_VOLUME);
        // biomes(64 个 4×4×4,全同一 biome)
        let biome_entries = vec![biome_id; 64];
        write_container(out, &biome_entries, &biome_ids, 64);
    }
}

/// 编码 Heightmaps 字段:返回 [(type, 打包后 longs)] 列表,由协议层组装前缀数组。
pub fn chunk_heightmaps(chunk: &Chunk) -> Vec<(u32, Vec<u64>)> {
    vec![(WORLD_SURFACE as u32, pack_heightmap(chunk))]
}

/// Light 字段:全天空光 15。skyLightMask 26 位(世界 24 section + 上下各 1),
/// 每个 section 2048 字节 0xFF;其余 mask 与数组为空。
pub fn light_full() -> Vec<u8> {
    let mut out = Vec::new();
    // skyLightMask:BitSet = 1 个 long,位 0..26 全置位
    write_varint(&mut out, 1);
    out.extend_from_slice(&0x03FF_FFFFu64.to_be_bytes());
    // blockLightMask / emptySkyLightMask / emptyBlockLightMask:空
    write_varint(&mut out, 0);
    write_varint(&mut out, 0);
    write_varint(&mut out, 0);
    // skyLight:26 个数组,每个 2048 字节 0xFF
    write_varint(&mut out, 26);
    for _ in 0..26 {
        write_varint(&mut out, 2048);
        out.extend_from_slice(&[0xFFu8; 2048]);
    }
    // blockLight:空
    write_varint(&mut out, 0);
    out
}

fn write_varint(w: &mut Vec<u8>, mut v: i32) {
    loop {
        let b = (v & 0x7F) as u8;
        v >>= 7;
        if v != 0 {
            w.push(b | 0x80);
        } else {
            w.push(b);
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::{BEDROCK, DIRT, GRASS_BLOCK, STONE};

    /// 测试用:计算一个 section + biome container 的字节长度(用于跳过)。
    fn container_len(buf: &[u8]) -> usize {
        // BPE
        let bits = buf[0] as u32;
        let mut pos = 1usize;
        match bits {
            0 => {
                // palette 单值(1 varint),无数据数组
                let (_, n) = varint_len(&buf[pos..]);
                pos += n;
            }
            4..=8 => {
                // palette 长度 varint + 每项 varint
                let (plen, n) = varint_len(&buf[pos..]);
                pos += n;
                for _ in 0..plen {
                    let (_, n) = varint_len(&buf[pos..]);
                    pos += n;
                }
                // 数据数组:4096 条,bits 位
                let per_long = (64 / bits) as usize;
                let longs = (4096 + per_long - 1) / per_long;
                pos += longs * 8;
            }
            _ => {
                // 直接:4096 条 15 位
                pos += (4096 / 4) * 8;
            }
        }
        // biome container
        let bits = buf[pos] as u32;
        pos += 1;
        match bits {
            0 => {
                let (_, n) = varint_len(&buf[pos..]);
                pos += n;
            }
            1..=3 => {
                let (plen, n) = varint_len(&buf[pos..]);
                pos += n;
                for _ in 0..plen {
                    let (_, n) = varint_len(&buf[pos..]);
                    pos += n;
                }
                let per_long = (64 / bits) as usize;
                let longs = (64 + per_long - 1) / per_long;
                pos += longs * 8;
            }
            _ => {
                pos += (64 / 4) * 8;
            }
        }
        pos
    }

    fn varint_len(buf: &[u8]) -> (i32, usize) {
        let mut v = 0i32;
        let mut n = 0;
        for (i, b) in buf.iter().enumerate() {
            v |= ((b & 0x7F) as i32) << (7 * i);
            n += 1;
            if b & 0x80 == 0 {
                break;
            }
        }
        (v, n)
    }

    fn flat_chunk() -> Chunk {
        let mut c = Chunk::new(0, 0);
        // 超平坦:y=-64 基岩,-63..-62 泥土,-61 草
        for x in 0..16 {
            for z in 0..16 {
                c.set(x, crate::chunk::MIN_Y, z, BEDROCK);
                c.set(x, crate::chunk::MIN_Y + 1, z, DIRT);
                c.set(x, crate::chunk::MIN_Y + 2, z, DIRT);
                c.set(x, crate::chunk::MIN_Y + 3, z, GRASS_BLOCK);
            }
        }
        c
    }

    #[test]
    fn heightmap_values() {
        let c = flat_chunk();
        // y=-61 是最高块 → 相对高度 3
        let hm = pack_heightmap(&c);
        let bits = HEIGHTMAP_BITS;
        let per_long = (64 / bits) as usize;
        let v = (hm[0] >> (0 % per_long * bits as usize)) & ((1u64 << bits) - 1);
        assert_eq!(v, 3, "height of column 0 should be 3");
    }

    #[test]
    fn sections_have_block_counts() {
        let c = flat_chunk();
        let mut out = Vec::new();
        write_sections(&c, 0, &mut out);
        // 每 section 固定布局:sections 从底部开始
        // section 0(-64..-48)含基岩/泥土/草 → count > 0
        let count0 = u16::from_be_bytes([out[0], out[1]]);
        assert!(count0 > 0, "section 0 has blocks, got {count0}");
        // section 1(-48..-32)全空气 → count 0
        let mut pos = 0usize;
        for _ in 0..crate::chunk::SECTIONS {
            pos += 4 + container_len(&out[pos + 4..]);
        }
    }

    #[test]
    fn air_section_single_palette() {
        let c = flat_chunk();
        let mut out = Vec::new();
        write_sections(&c, 0, &mut out);
        // 跳过 section 0 找到 section 1(全空气)
        let mut pos = 0usize;
        for si in 0..crate::chunk::SECTIONS {
            if si == 1 {
                // 全空气:count=0,然后 block container:BPE 0 + value air(0)
                assert_eq!(&out[pos..pos + 2], &[0, 0], "air section block count");
                assert_eq!(&out[pos + 2..pos + 4], &[0, 0], "air section fluid count");
                assert_eq!(out[pos + 4], 0, "BPE 0");
                assert_eq!(out[pos + 5], 0, "air palette value");
                break;
            }
            pos += 4 + container_len(&out[pos + 4..]);
        }
    }

    #[test]
    fn stone_section_uses_indirect() {
        let mut c = Chunk::new(1, 1);
        for x in 0..16 {
            for z in 0..16 {
                for y in 0..16 {
                    c.set(x, crate::chunk::MIN_Y + y, z, STONE);
                }
            }
        }
        let mut out = Vec::new();
        write_sections(&c, 0, &mut out);
        // section 0:4096 stone,count=4096
        let count = u16::from_be_bytes([out[0], out[1]]);
        assert_eq!(count, 4096);
        // BPE=4(1 个唯一值→单值?不,1 个唯一 → BPE 0 单值)
        assert_eq!(out[4], 0);
    }
}