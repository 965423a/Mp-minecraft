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

pub fn generate_spawn(
    world_dir: &Path,
    seed: u64,
    wtype: WorldType,
    radius_chunks: i32,
) -> io::Result<WorldStats> {
    std::fs::create_dir_all(world_dir.join("region"))?;
    std::fs::create_dir_all(world_dir.join("entities"))?;
    std::fs::create_dir_all(world_dir.join("poi"))?;
    let lock = world_dir.join("session.lock");
    if !lock.exists() {
        let _ = std::fs::write(&lock, b"");
    }
    let coords: Vec<(i32, i32)> = (-radius_chunks..=radius_chunks)
        .flat_map(|cx| (-radius_chunks..=radius_chunks).map(move |cz| (cx, cz)))
        .collect();
    let threads = std::env::var("MCS_THREADS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1)
        })
        .min(coords.len());

    let chunks: Vec<mc_world::Chunk> = std::thread::scope(|s| {
        let handles: Vec<_> = coords
            .chunks(coords.len().div_ceil(threads))
            .map(|part| {
                let generator = WorldGenerator::new(seed, wtype);
                s.spawn(move || part.iter().map(|&(cx, cz)| generator.generate(cx, cz)).collect::<Vec<_>>())
            })
            .collect();
        let mut out = Vec::with_capacity(coords.len());
        for h in handles {
            out.extend(h.join().expect("chunk worker panicked"));
        }
        out
    });

    let mut total_blocks = 0usize;
    let mut total_surface = 0i64;
    for c in &chunks {
        total_blocks += c.block_count();
        total_surface += c.height_at(8, 8) as i64;
    }
    let mean_surface = (total_surface / chunks.len() as i64) as i32;

    let regions = {
        let mut m: std::collections::HashMap<(i32, i32), Vec<&mc_world::Chunk>> =
            std::collections::HashMap::new();
        for c in &chunks {
            m.entry((c.cx.div_euclid(32), c.cz.div_euclid(32))).or_default().push(c);
        }
        m
    };
    std::thread::scope(|s| {
        for cs in regions.into_values() {
            s.spawn(move || {
                for c in cs {
                    RegionFile::save(world_dir, c, seed, wtype)?;
                }
                Ok::<(), io::Error>(())
            });
        }
    });
    let files = RegionFile::count_files(world_dir)?;
    eprintln!("[parallel] {threads} worker threads, {} chunks", chunks.len());
    Ok(WorldStats {
        chunks: chunks.len(),
        blocks: total_blocks,
        mean_surface,
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

/// 共享世界:内存缓存的已加载区块,变更后写回 region 存档。
pub struct World {
    world_dir: std::path::PathBuf,
    seed: u64,
    wtype: WorldType,
    chunks: std::collections::HashMap<(i32, i32), mc_world::Chunk>,
}

impl World {
    pub fn open(world_dir: &Path, seed: u64, wtype: WorldType) -> Self {
        World {
            world_dir: world_dir.to_path_buf(),
            seed,
            wtype,
            chunks: std::collections::HashMap::new(),
        }
    }

    fn chunk(&mut self, cx: i32, cz: i32) -> &mut mc_world::Chunk {
        self.chunks.entry((cx, cz)).or_insert_with(|| {
            RegionFile::load(&self.world_dir, cx, cz)
                .unwrap_or_else(|_| WorldGenerator::new(self.seed, self.wtype).generate(cx, cz))
        })
    }

    pub fn get_block(&self, x: i32, y: i32, z: i32) -> u16 {
        let (cx, cz) = (x.div_euclid(16), z.div_euclid(16));
        self.chunks
            .get(&(cx, cz))
            .map(|c| c.get(x, y, z))
            .unwrap_or(mc_world::blocks::AIR)
    }

    pub fn set_block(&mut self, x: i32, y: i32, z: i32, id: u16) {
        let (cx, cz) = (x.div_euclid(16), z.div_euclid(16));
        let (dir, seed, wtype) = (self.world_dir.clone(), self.seed, self.wtype);
        let chunk = self.chunk(cx, cz);
        chunk.set(x, y, z, id);
        let _ = RegionFile::save(&dir, chunk, seed, wtype);
    }

    pub fn chunk_bytes(&mut self, cx: i32, cz: i32, biome: u16) -> (Vec<u8>, Vec<(u32, Vec<u64>)>) {
        let chunk = self.chunk(cx, cz);
        let heightmaps = mc_world::network::chunk_heightmaps(chunk);
        let mut data = Vec::new();
        mc_world::network::write_sections(chunk, biome, &mut data);
        (data, heightmaps)
    }
}