//! 物品注册表 ID 表:从 items_pack.bin(原版注册表顺序)加载。
//! 提供 item ID → 名称 与 名称 → item ID 双向查找。

use std::string::String;

pub const PACK_BYTES: &[u8] = include_bytes!("../items_pack.bin");

/// item ID → 物品名(如 28 → "dirt")。
pub fn name_of(id: u32) -> Option<String> {
    let mut pos = 12;
    let count = read_u32(PACK_BYTES, 8) as usize;
    for _ in 0..count {
        let iid = read_u32(PACK_BYTES, pos);
        pos += 4;
        let (name, p) = read_string(PACK_BYTES, pos);
        pos = p;
        if iid == id {
            return Some(name);
        }
    }
    None
}

/// 物品名 → item ID(如 "dirt" → 28)。
pub fn id_of(name: &str) -> Option<u32> {
    let mut pos = 12;
    let count = read_u32(PACK_BYTES, 8) as usize;
    for _ in 0..count {
        let iid = read_u32(PACK_BYTES, pos);
        pos += 4;
        let (bname, p) = read_string(PACK_BYTES, pos);
        pos = p;
        if bname == name {
            return Some(iid);
        }
    }
    None
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
    fn known_ids() {
        assert_eq!(name_of(0).as_deref(), Some("air"));
        assert_eq!(name_of(28).as_deref(), Some("dirt"));
        assert_eq!(name_of(134).as_deref(), Some("oak_log"));
        assert_eq!(id_of("stone"), Some(1));
    }

    #[test]
    fn dense_registry() {
        for id in [0u32, 1, 27, 28, 58, 134, 1505] {
            assert!(name_of(id).is_some(), "item id {id} must exist");
        }
    }
}