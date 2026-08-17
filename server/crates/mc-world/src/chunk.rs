//! 区块模型:16×16×384 世界柱,24 个 section,索引 x + z*16 + y*256。

pub const CHUNK_SIZE: usize = 16;
pub const SECTION_HEIGHT: usize = 16;
pub const SECTIONS: usize = 24; // 384 / 16
pub const SECTION_VOLUME: usize = 4096;
pub const CHUNK_VOLUME: usize = CHUNK_SIZE * CHUNK_SIZE * SECTIONS * SECTION_HEIGHT;
pub const MIN_Y: i32 = -64;
pub const MAX_Y: i32 = 320;
pub const SEA_LEVEL: i32 = 63;

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use crate::blocks::AIR;

/// 一个 section(16×16×16),平面数组存储,索引 x + z*16 + y*256。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Section {
    blocks: [u16; SECTION_VOLUME],
}

impl Default for Section {
    fn default() -> Self {
        Section {
            blocks: [AIR; SECTION_VOLUME],
        }
    }
}

impl Section {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn get(&self, x: usize, y: usize, z: usize) -> u16 {
        self.blocks[x + z * 16 + y * 256]
    }

    #[inline]
    pub fn set(&mut self, x: usize, y: usize, z: usize, id: u16) {
        self.blocks[x + z * 16 + y * 256] = id;
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.blocks.iter().all(|&b| b == AIR)
    }

    pub fn count_non_air(&self) -> usize {
        self.blocks.iter().filter(|&&b| b != AIR).count()
    }

    /// 位打包为 compact long 数组(与协议一致):4096 格,每格 bits 位。
    pub fn pack(&self, bits: u32) -> Vec<u64> {
        let per_long = (64 / bits) as usize;
        let longs_needed = (SECTION_VOLUME + per_long - 1) / per_long;
        let mut out = vec![0u64; longs_needed];
        for i in 0..SECTION_VOLUME {
            let (word, offset) = (i / per_long, (i % per_long) * bits as usize);
            out[word] |= (self.blocks[i] as u64 & ((1u64 << bits) - 1)) << offset;
        }
        out
    }

    /// 从位打包数组解包(用于验证打包正确性)。
    pub fn unpack(&mut self, bits: u32, packed: &[u64]) {
        let per_long = (64 / bits) as usize;
        for i in 0..SECTION_VOLUME {
            let (word, offset) = (i / per_long, (i % per_long) * bits as usize);
            let v = (packed[word] >> offset) & ((1u64 << bits) - 1);
            self.blocks[i] = v as u16;
        }
    }
}

#[derive(Debug, Clone)]
pub struct Chunk {
    pub cx: i32,
    pub cz: i32,
    sections: Box<[Section; SECTIONS]>,
}

impl Chunk {
    pub fn new(cx: i32, cz: i32) -> Self {
        Chunk {
            cx,
            cz,
            sections: Box::new([Section::new(); SECTIONS]),
        }
    }

    #[inline]
    pub fn section_index(y: i32) -> usize {
        ((y - MIN_Y) / SECTION_HEIGHT as i32) as usize
    }

    #[inline]
    pub fn local(x: i32, z: i32) -> (usize, usize) {
        (x.rem_euclid(16) as usize, z.rem_euclid(16) as usize)
    }

    #[inline]
    pub fn get(&self, x: i32, y: i32, z: i32) -> u16 {
        if y < MIN_Y || y >= MAX_Y {
            return AIR;
        }
        let (lx, lz) = Self::local(x, z);
        self.sections[Self::section_index(y)].get(lx, (y - MIN_Y) as usize % SECTION_HEIGHT, lz)
    }

    #[inline]
    pub fn set(&mut self, x: i32, y: i32, z: i32, id: u16) {
        if y < MIN_Y || y >= MAX_Y {
            return;
        }
        let (lx, lz) = Self::local(x, z);
        self.sections[Self::section_index(y)].set(lx, (y - MIN_Y) as usize % SECTION_HEIGHT, lz, id);
    }

    pub fn sections(&self) -> &[Section; SECTIONS] {
        &self.sections
    }

    pub fn sections_mut(&mut self) -> &mut [Section; SECTIONS] {
        &mut self.sections
    }

    pub fn section(&self, i: usize) -> &Section {
        &self.sections[i]
    }

    pub fn non_empty_sections(&self) -> usize {
        self.sections.iter().filter(|s| !s.is_empty()).count()
    }

    pub fn block_count(&self) -> usize {
        self.sections.iter().map(|s| s.count_non_air()).sum()
    }

    /// 最高非空气方块(表面高度),全空返回 MIN_Y-1。
    pub fn height_at(&self, x: i32, z: i32) -> i32 {
        for y in (MIN_Y..MAX_Y).rev() {
            if self.get(x, y, z) != AIR {
                return y;
            }
        }
        MIN_Y - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::{GRASS_BLOCK, STONE};

    #[test]
    fn set_get_roundtrip() {
        let mut c = Chunk::new(0, 0);
        c.set(3, 70, 5, STONE);
        assert_eq!(c.get(3, 70, 5), STONE);
        assert_eq!(c.get(3, 70, 4), AIR);
        assert_eq!(c.get(3, 71, 5), AIR);
    }

    #[test]
    fn negative_world_coords() {
        let mut c = Chunk::new(-2, -3);
        // 区块 -2,-3 覆盖 x∈[-32,-17], z∈[-48,-33]
        c.set(-32, 80, -48, GRASS_BLOCK);
        assert_eq!(c.get(-32, 80, -48), GRASS_BLOCK);
        assert_eq!(c.get(-17, 80, -33), AIR);
    }

    #[test]
    fn out_of_bounds_y() {
        let mut c = Chunk::new(0, 0);
        c.set(0, -65, 0, STONE);
        c.set(0, 320, 0, STONE);
        assert_eq!(c.get(0, -65, 0), AIR);
        assert_eq!(c.get(0, 320, 0), AIR);
    }

    #[test]
    fn pack_unpack_roundtrip() {
        let mut s = Section::new();
        s.set(0, 0, 0, 2);
        s.set(15, 15, 15, 7);
        s.set(8, 8, 8, 3);
        for bits in [4u32, 5, 6, 8, 12] {
            let packed = s.pack(bits);
            let mut back = Section::new();
            back.unpack(bits, &packed);
            assert_eq!(s, back, "bits={bits}");
        }
    }

    #[test]
    fn packed_size_expected() {
        let s = Section::new();
        // 4096 格 / 每 long 16 格(4bit)= 256 longs
        assert_eq!(s.pack(4).len(), 256);
        assert_eq!(s.pack(8).len(), 512);
    }

    #[test]
    fn section_is_empty() {
        assert!(Section::new().is_empty());
        let mut s = Section::new();
        s.set(1, 1, 1, STONE);
        assert!(!s.is_empty());
    }
}