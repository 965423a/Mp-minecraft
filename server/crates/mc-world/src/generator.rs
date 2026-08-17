//! 地形生成器:超平坦与正常地形(确定性:同种子同区块 → 完全一致)。

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::blocks::*;
use crate::chunk::{Chunk, MAX_Y, MIN_Y, SEA_LEVEL};
use crate::noise::Noise;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldType {
    Superflat,
    Normal,
}

impl WorldType {
    pub fn name(&self) -> &'static str {
        match self {
            WorldType::Superflat => "superflat",
            WorldType::Normal => "normal",
        }
    }
}

/// 超平坦固定分层(从下往上)。
pub const SUPERFLAT_LAYERS: &[(u16, i32)] = &[
    (BEDROCK, 1),
    (DIRT, 2),
    (GRASS_BLOCK, 1),
];

/// 生成器:持有噪声状态,线程安全(不可变)。
pub struct WorldGenerator {
    pub world_type: WorldType,
    seed: u64,
    continent: Noise, // 大陆/海洋形状
    hills: Noise,     // 山丘高度
    detail: Noise,    // 细节起伏
    cave3d: Noise,    // 洞穴(3D)
}

impl WorldGenerator {
    pub fn new(seed: u64, world_type: WorldType) -> Self {
        WorldGenerator {
            world_type,
            seed,
            continent: Noise::new(seed.wrapping_mul(0x9E37_79B1) ^ 0x11),
            hills: Noise::new(seed.wrapping_mul(0x85EB_CA77) ^ 0x22),
            detail: Noise::new(seed.wrapping_mul(0xC2B2_AE35) ^ 0x33),
            cave3d: Noise::new(seed.wrapping_mul(0x27D4_EB2F) ^ 0x44),
        }
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    pub fn generate(&self, cx: i32, cz: i32) -> Chunk {
        let mut chunk = Chunk::new(cx, cz);
        match self.world_type {
            WorldType::Superflat => self.fill_superflat(&mut chunk),
            WorldType::Normal => self.fill_normal(&mut chunk),
        }
        chunk
    }

    fn fill_superflat(&self, chunk: &mut Chunk) {
        let base_x = cx0(chunk.cx);
        let base_z = cz0(chunk.cz);
        for lx in 0..16 {
            for lz in 0..16 {
                let wx = base_x + lx as i32;
                let wz = base_z + lz as i32;
                let mut y = MIN_Y;
                for (id, count) in SUPERFLAT_LAYERS {
                    for _ in 0..*count {
                        chunk.set(wx, y, wz, *id);
                        y += 1;
                    }
                }
            }
        }
    }

    fn fill_normal(&self, chunk: &mut Chunk) {
        let base_x = cx0(chunk.cx);
        let base_z = cz0(chunk.cz);
        // 表面高度缓存:填充列、洞穴雕刻、树生成共用,避免重复 fbm。
        let mut hts = [0i32; 256];
        for lx in 0..16 {
            for lz in 0..16 {
                let wx = base_x + lx as i32;
                let wz = base_z + lz as i32;
                let height = self.surface_height(wx, wz);
                hts[lz * 16 + lx] = height;
                self.fill_column(chunk, wx, wz, height);
            }
        }
        self.carve_caves(chunk, &hts);
        let n = &self.detail;
        if n.noise2(base_x as f64 * 0.01, base_z as f64 * 0.01) > 0.82 {
            let tx = 8 + (n.noise2(base_x as f64, base_z as f64) * 4.0) as i32;
            let tz = 8 + (n.noise2(base_z as f64, base_x as f64) * 4.0) as i32;
            if tx >= 0 && tx < 16 && tz >= 0 && tz < 16 {
                let wx = base_x + tx;
                let wz = base_z + tz;
                let top = hts[tz as usize * 16 + tx as usize];
                if Self::is_solid_surface(chunk.get(wx, top, wz)) && top > SEA_LEVEL {
                    self.grow_tree(chunk, wx, top + 1, wz);
                }
            }
        }
    }

    fn surface_height(&self, wx: i32, wz: i32) -> i32 {
        let (x, z) = (wx as f64, wz as f64);
        // 大陆:低频,决定海洋/陆地
        let continent = self.continent.fbm2(x * 0.0015, z * 0.0015, 3, 2.0, 0.5);
        // 山丘:中频
        let hills = self.hills.fbm2(x * 0.008, z * 0.008, 4, 2.0, 0.5);
        // 细节:高频,让表面自然
        let detail = self.detail.fbm2(x * 0.05, z * 0.05, 2, 2.0, 0.5);

        let land_factor = (continent + 1.0) / 2.0; // [0,1]
        let base = if land_factor < 0.40 {
            (SEA_LEVEL - 20) as f64 + continent * 12.0
        } else {
            SEA_LEVEL as f64 + (land_factor - 0.40) * 260.0 + hills * 60.0
        };
        (libm::round(base + detail * 3.0) as i32).clamp(MIN_Y + 2, MAX_Y - 30)
    }

    fn is_solid_surface(id: u16) -> bool {
        id == GRASS_BLOCK || id == DIRT || id == STONE || id == SAND || id == SNOW_BLOCK
    }

    fn fill_column(&self, chunk: &mut Chunk, wx: i32, wz: i32, surface: i32) {
        // 基岩:底层 1~2 层
        chunk.set(wx, MIN_Y, wz, BEDROCK);
        if self.detail.noise2(wx as f64 * 0.9, wz as f64 * 0.9) > 0.3 {
            chunk.set(wx, MIN_Y + 1, wz, BEDROCK);
        }

        let underwater = surface < SEA_LEVEL;
        let beach = !underwater && surface <= SEA_LEVEL + 2 && surface >= SEA_LEVEL - 1;

        for y in (MIN_Y + 1)..=surface {
            let depth = surface - y;
            let id = if beach && depth <= 3 {
                SAND
            } else if underwater && depth <= 4 && surface >= SEA_LEVEL - 8 {
                // 浅海床:沙/砾石/黏土混合
                let r = self.detail.noise2(wx as f64 * 0.3 + y as f64, wz as f64 * 0.3);
                if r > 0.55 {
                    GRAVEL
                } else if r < -0.6 {
                    CLAY
                } else {
                    SAND
                }
            } else if depth == 0 && !underwater && !beach {
                GRASS_BLOCK
            } else if depth <= 3 && !underwater {
                DIRT
            } else {
                STONE
            };
            chunk.set(wx, y, wz, id);
        }
        // 水:海平面下填充到水面
        if underwater {
            for y in (surface + 1)..=SEA_LEVEL {
                chunk.set(wx, y, wz, WATER);
            }
        }
    }

    /// 洞穴:3D 噪声走廊,削掉石头/泥土(不挖水)。
    /// 只雕到表面高度以下(hts 为 16×16 表面高度缓存),减少 20%+ 噪声计算。
    fn carve_caves(&self, chunk: &mut Chunk, hts: &[i32; 256]) {
        let (bx, bz) = (cx0(chunk.cx), cz0(chunk.cz));
        let mut n1 = [0f64; 160];
        let mut n2 = [0f64; 160];
        for lx in 0..16 {
            for lz in 0..16 {
                let wx = bx + lx as i32;
                let wz = bz + lz as i32;
                let surf = hts[lz * 16 + lx].min(SEA_LEVEL + 24);
                let mut y0 = MIN_Y + 4;
                if surf <= y0 {
                    continue;
                }
                // 列级噪声:同 (wx,wz) 前缀复用,沿 y 扫描。
                let m = (surf - y0) as usize;
                self.cave3d.noise3_yline(
                    wx as f64 * 0.09,
                    wz as f64 * 0.09,
                    y0 as f64 * 0.12,
                    0.12,
                    &mut n1[..m],
                );
                self.cave3d.noise3_yline(
                    wx as f64 * 0.03 + 100.0,
                    wz as f64 * 0.03,
                    y0 as f64 * 0.05,
                    0.05,
                    &mut n2[..m],
                );
                for i in 0..m {
                    let y = y0 + i as i32;
                    if n1[i] > 0.92 && n2[i] > 0.0 {
                        let id = chunk.get(wx, y, wz);
                        if id == STONE || id == DIRT || id == GRAVEL {
                            chunk.set(wx, y, wz, AIR);
                        }
                    }
                }
            }
        }
    }

    /// 简单橡树:高 4~6,树冠 5×5×3。
    fn grow_tree(&self, chunk: &mut Chunk, wx: i32, y0: i32, wz: i32) {
        let r = self.detail.noise2(wx as f64 * 0.7, wz as f64 * 0.7);
        let trunk_h = 4 + ((r * 2.0).abs() as i32) % 3;
        for i in 0..trunk_h {
            chunk.set(wx, y0 + i, wz, OAK_LOG);
        }
        let canopy_top = y0 + trunk_h;
        for dy in 0..3 {
            let radius: i32 = if dy == 2 { 1 } else { 2 };
            for dx in -radius..=radius {
                for dz in -radius..=radius {
                    if dx == 0 && dz == 0 && dy == 0 {
                        continue; // 树干位置
                    }
                    let y = canopy_top + dy;
                    if dx.abs() == radius && dz.abs() == radius && dy != 0 {
                        continue; // 圆角
                    }
                    if chunk.get(wx + dx, y, wz + dz) == AIR {
                        chunk.set(wx + dx, y, wz + dz, OAK_LEAVES);
                    }
                }
            }
        }
    }
}

#[inline]
fn cx0(cx: i32) -> i32 {
    cx * 16
}
#[inline]
fn cz0(cz: i32) -> i32 {
    cz * 16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn superflat_layers_correct() {
        let g = WorldGenerator::new(42, WorldType::Superflat);
        let c = g.generate(0, 0);
        // -64 基岩
        assert_eq!(c.get(0, -64, 0), BEDROCK);
        assert_eq!(c.get(15, -64, 15), BEDROCK);
        // -63..-62 泥土
        assert_eq!(c.get(5, -63, 5), DIRT);
        // -61 草方块
        assert_eq!(c.get(5, -61, 5), GRASS_BLOCK);
        // -60 以上是空气
        assert_eq!(c.get(5, -60, 5), AIR);
        assert_eq!(c.height_at(3, 7), -61);
        // 全区块一致
        for lx in 0..16 {
            for lz in 0..16 {
                assert_eq!(c.height_at(lx, lz), -61);
            }
        }
    }

    #[test]
    fn normal_has_terrain_and_air() {
        let g = WorldGenerator::new(2026, WorldType::Normal);
        let c = g.generate(0, 0);
        assert!(c.block_count() > 1000, "too few blocks: {}", c.block_count());
        // 底部基岩
        assert_eq!(c.get(8, MIN_Y, 8), BEDROCK);
        // 顶部存在空气
        assert_eq!(c.get(8, MAX_Y - 1, 8), AIR);
    }

    #[test]
    fn normal_deterministic() {
        let a = WorldGenerator::new(99, WorldType::Normal).generate(3, -5);
        let b = WorldGenerator::new(99, WorldType::Normal).generate(3, -5);
        for x in 0..16 {
            for z in 0..16 {
                for y in (MIN_Y..MAX_Y).step_by(4) {
                    assert_eq!(a.get(x, y, z), b.get(x, y, z), "({x},{y},{z})");
                }
            }
        }
    }

    #[test]
    fn different_seed_different_world() {
        let a = WorldGenerator::new(1, WorldType::Normal).generate(0, 0);
        let b = WorldGenerator::new(2, WorldType::Normal).generate(0, 0);
        let mut diff = 0;
        for x in 0..16 {
            for z in 0..16 {
                if a.height_at(x, z) != b.height_at(x, z) {
                    diff += 1;
                }
            }
        }
        assert!(diff > 20, "worlds too similar: {diff}/256");
    }

    #[test]
    fn ocean_has_water_to_sea_level() {
        let g = WorldGenerator::new(7, WorldType::Normal);
        let c = g.generate(100, 100); // 大概率海洋
        let h = c.height_at(8, 8);
        if h < SEA_LEVEL {
            assert_eq!(c.get(8, SEA_LEVEL, 8), WATER);
            assert_eq!(c.get(8, SEA_LEVEL + 1, 8), AIR);
        }
    }

    #[test]
    fn heights_within_world() {
        let g = WorldGenerator::new(555, WorldType::Normal);
        for cx in -2..=2 {
            for cz in -2..=2 {
                let c = g.generate(cx, cz);
                for x in 0..16 {
                    for z in 0..16 {
                        let h = c.height_at(x, z);
                        assert!(h >= MIN_Y && h < MAX_Y);
                    }
                }
            }
        }
    }
}