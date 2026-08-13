//! 简化版区域存档:将区块保存为二进制文件。
//! 结构:魔数 "MCSR" + 版本 + 种子 + 世界类型 + 区块数据(位打包 sections)。
//! 每区块 24 个 section,每 section:bit 数(1B)+ long 数组长度(VarLong)+ longs。
//! 后续可替换为原版 region 格式。

use crate::chunk::{Chunk, Section, SECTIONS};
use crate::generator::WorldType;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;

const MAGIC: &[u8; 4] = b"MCSR";
const VERSION: u32 = 1;
const BIT_DEPTH: u32 = 6; // 0..63 的方块 ID 足够

/// 世界存档目录(world/)下的元数据。
pub struct RegionFile {
    cx: i32,
    cz: i32,
}

impl RegionFile {
    pub fn path_for(world_dir: &Path, cx: i32, cz: i32) -> std::path::PathBuf {
        let region_x = cx.div_euclid(32);
        let region_z = cz.div_euclid(32);
        let region_dir = world_dir.join("region");
        region_dir.join(format!("r.{region_x}.{region_z}.mcr"))
    }

    pub fn save(
        world_dir: &Path,
        chunk: &Chunk,
        seed: u64,
        world_type: WorldType,
    ) -> io::Result<()> {
        let region_dir = world_dir.join("region");
        fs::create_dir_all(&region_dir)?;
        let path = Self::path_for(world_dir, chunk.cx, chunk.cz);
        let mut out = Vec::with_capacity(24 * 4096);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_be_bytes());
        out.extend_from_slice(&seed.to_be_bytes());
        out.extend_from_slice(&[world_type as u8]);
        out.extend_from_slice(&chunk.cx.to_be_bytes());
        out.extend_from_slice(&chunk.cz.to_be_bytes());

        for s in chunk.sections().iter() {
            out.push(BIT_DEPTH as u8);
            let packed = s.pack(BIT_DEPTH);
            out.extend_from_slice(&(packed.len() as u32).to_be_bytes());
            for l in &packed {
                out.extend_from_slice(&l.to_be_bytes());
            }
        }
        fs::write(&path, out)
    }

    pub fn load(world_dir: &Path, cx: i32, cz: i32) -> io::Result<Chunk> {
        let path = Self::path_for(world_dir, cx, cz);
        let mut f = File::open(path)?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        if buf.len() < 22 || &buf[0..4] != MAGIC {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "bad region magic"));
        }
        let mut pos = 4;
        let version = u32::from_be_bytes(buf[pos..pos + 4].try_into().unwrap());
        pos += 4;
        let _seed = u64::from_be_bytes(buf[pos..pos + 8].try_into().unwrap());
        pos += 8;
        let _wt = buf[pos];
        pos += 1;
        let scx = i32::from_be_bytes(buf[pos..pos + 4].try_into().unwrap());
        pos += 4;
        let scz = i32::from_be_bytes(buf[pos..pos + 4].try_into().unwrap());
        pos += 4;
        if version != VERSION || scx != cx || scz != cz {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "region mismatch"));
        }
        let mut chunk = Chunk::new(cx, cz);
        let mut sections = [Section::new(); SECTIONS];
        for s in sections.iter_mut() {
            let bits = buf[pos] as u32;
            pos += 1;
            let len = u32::from_be_bytes(buf[pos..pos + 4].try_into().unwrap()) as usize;
            pos += 4;
            let mut packed = Vec::with_capacity(len);
            for _ in 0..len {
                packed.push(u64::from_be_bytes(buf[pos..pos + 8].try_into().unwrap()));
                pos += 8;
            }
            s.unpack(bits, &packed);
        }
        chunk.replace_sections(sections);
        Ok(chunk)
    }

    /// 统计 world 目录已有区块文件数。
    pub fn count_files(world_dir: &Path) -> io::Result<usize> {
        let region_dir = world_dir.join("region");
        let mut n = 0;
        if let Ok(rd) = fs::read_dir(&region_dir) {
            for e in rd.flatten() {
                if e.file_name().to_string_lossy().ends_with(".mcr") {
                    n += 1;
                }
            }
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::SECTION_VOLUME;
    use crate::chunk::{MAX_Y, MIN_Y};
    use crate::generator::WorldGenerator;
    use crate::generator::WorldType::Normal;

    #[test]
    fn save_load_roundtrip() {
        let dir = std::env::temp_dir().join("mcs-region-test");
        let _ = fs::remove_dir_all(&dir);
        let g = WorldGenerator::new(12345, Normal);
        let c = g.generate(2, -3);
        RegionFile::save(&dir, &c, 12345, Normal).unwrap();
        let loaded = RegionFile::load(&dir, 2, -3).unwrap();
        assert_eq!(loaded.cx, 2);
        assert_eq!(loaded.cz, -3);
        for x in 0..16 {
            for z in 0..16 {
                for y in (MIN_Y..MAX_Y).step_by(2) {
                    assert_eq!(c.get(x, y, z), loaded.get(x, y, z), "({x},{y},{z})");
                }
            }
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_section_roundtrip() {
        let dir = std::env::temp_dir().join("mcs-region-test2");
        let _ = fs::remove_dir_all(&dir);
        let mut c = Chunk::new(0, 0);
        c.set(0, MIN_Y, 0, 7);
        c.set(7, 100, 9, 2);
        RegionFile::save(&dir, &c, 1, WorldType::Superflat).unwrap();
        let loaded = RegionFile::load(&dir, 0, 0).unwrap();
        assert_eq!(loaded.get(0, MIN_Y, 0), 7);
        assert_eq!(loaded.get(7, 100, 9), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn air_is_default() {
        let s = Section::new();
        assert!(s.get(0, 0, 0) == 0);
        assert_eq!(SECTION_VOLUME, 4096);
    }
}