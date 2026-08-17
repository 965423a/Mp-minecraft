//! 全局 block state ID 表:从 blocks_pack.bin(原版注册表顺序)加载。

use alloc::string::String;
use alloc::string::ToString;
use alloc::vec;
use alloc::vec::Vec;

pub const PACK_BYTES: &[u8] = include_bytes!("../blocks_pack.bin");

pub fn name_of(id: u16) -> Option<String> {
    let mut pos = 12;
    let blocks = read_u32(PACK_BYTES, 8) as usize;
    for _ in 0..blocks {
        let (_, p) = read_string(PACK_BYTES, pos);
        pos = p;
        read_u32(PACK_BYTES, pos);
        pos += 4;
        let count = read_u32(PACK_BYTES, pos) as usize;
        pos += 4;
        for _ in 0..count {
            let sid = read_u32(PACK_BYTES, pos) as u16;
            pos += 4;
            let (sname, p) = read_string(PACK_BYTES, pos);
            pos = p;
            if sid == id {
                return Some(sname);
            }
        }
    }
    None
}

pub fn default_id(name: &str) -> Option<u16> {
    let mut pos = 12;
    let blocks = read_u32(PACK_BYTES, 8) as usize;
    for _ in 0..blocks {
        let (bname, p) = read_string(PACK_BYTES, pos);
        pos = p;
        let default = read_u32(PACK_BYTES, pos) as u16;
        pos += 4;
        let count = read_u32(PACK_BYTES, pos) as usize;
        pos += 4;
        for _ in 0..count {
            read_u32(PACK_BYTES, pos);
            pos += 4;
            let (_, p) = read_string(PACK_BYTES, pos);
            pos = p;
        }
        if bname == name {
            return Some(default);
        }
    }
    None
}

pub fn all_states(name: &str) -> Vec<u16> {
    let mut out = Vec::new();
    let mut pos = 12;
    let blocks = read_u32(PACK_BYTES, 8) as usize;
    for _ in 0..blocks {
        let (bname, p) = read_string(PACK_BYTES, pos);
        pos = p;
        read_u32(PACK_BYTES, pos);
        pos += 4;
        let count = read_u32(PACK_BYTES, pos) as usize;
        pos += 4;
        for _ in 0..count {
            let sid = read_u32(PACK_BYTES, pos) as u16;
            pos += 4;
            let (_, p) = read_string(PACK_BYTES, pos);
            pos = p;
            if bname == name {
                out.push(sid);
            }
        }
    }
    out
}

fn read_u32(b: &[u8], pos: usize) -> u32 {
    u32::from_le_bytes([b[pos], b[pos + 1], b[pos + 2], b[pos + 3]])
}

fn read_string(b: &[u8], pos: usize) -> (String, usize) {
    let len = u16::from_le_bytes([b[pos], b[pos + 1]]) as usize;
    let s = core::str::from_utf8(&b[pos + 2..pos + 2 + len])
        .unwrap_or("")
        .to_string();
    (s, pos + 2 + len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_ids() {
        assert_eq!(default_id("air"), Some(0));
        assert_eq!(default_id("stone"), Some(1));
        assert_eq!(default_id("grass_block"), Some(9));
        assert_eq!(default_id("bedrock"), Some(85));
        assert_eq!(default_id("water"), Some(86));
        assert_eq!(default_id("red_mushroom"), Some(2337));
    }

    #[test]
    fn id_name_roundtrip() {
        for id in [0u16, 1, 9, 85, 86, 6946, 29872] {
            let name = name_of(id).expect("name_of");
            let back = default_id(name.trim_start_matches("minecraft:")).expect("default_id");
            assert_eq!(back, id, "name={name} id={id}");
        }
    }
}