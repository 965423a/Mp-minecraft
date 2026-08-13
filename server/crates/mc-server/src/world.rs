//! 世界:预生成出生点区块 + 统计。

use mc_world::chunk::{MAX_Y, MIN_Y};
use mc_world::generator::{WorldGenerator, WorldType};
use mc_world::region::RegionFile;
use std::io;
use std::path::Path;

pub struct WorldStats {
    pub chunks: usize,
    pub blocks: usize,
    pub mean_surface: i32,
    pub files: usize,
}

/// 预生成出生点周围 (2r+1)² 个区块,保存到 world/region/。
pub fn generate_spawn(
    world_dir: &Path,
    seed: u64,
    wtype: WorldType,
    radius_chunks: i32,
) -> io::Result<WorldStats> {
    std::fs::create_dir_all(world_dir)?;
    let generator = mc_world::generator::WorldGenerator::new(seed, wtype);
    let mut total_blocks = 0usize;
    let mut total_surface = 0i64;
    let mut samples = 0i64;

    for cx in -radius_chunks..=radius_chunks {
        for cz in -radius_chunks..=radius_chunks {
            let chunk = generator.generate(cx, cz);
            total_blocks += chunk.block_count();
            // 表面高度采样(每区块中心一列)
            let h = chunk.height_at(8, 8);
            total_surface += h as i64;
            samples += 1;
            RegionFile::save(world_dir, &chunk, seed, wtype)?;
        }
    }
    let files = RegionFile::count_files(world_dir)?;
    Ok(WorldStats {
        chunks: (2 * radius_chunks as usize + 1).pow(2),
        blocks: total_blocks,
        mean_surface: if samples > 0 {
            (total_surface / samples) as i32
        } else {
            MIN_Y
        },
        files,
    })
}

/// 从存档重新加载一个区块(验证持久化)。
#[allow(dead_code)]
pub fn load_chunk(world_dir: &Path, cx: i32, cz: i32) -> io::Result<mc_world::Chunk> {
    let c = RegionFile::load(world_dir, cx, cz)?;
    debug_assert!(c.height_at(8, 8) >= MIN_Y && c.height_at(8, 8) < MAX_Y);
    Ok(c)
}