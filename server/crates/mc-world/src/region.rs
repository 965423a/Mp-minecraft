//! 简化 region 存档:魔数 "MCSR" + 版本 + 种子 + 世界类型 + 区块索引条目(cx, cz, offset, length) + 位打包 sections(空 section:bits=0+len=0)。

use crate::chunk::{Chunk, Section, SECTIONS};
use crate::generator::WorldType;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;

const MAGIC: &[u8; 4] = b"MCSR";
const VERSION: u32 = 3;
const ENTRY_SIZE: usize = 16;
const HEADER_SIZE: usize = 4 + 4 + 8 + 1 + 4;

pub struct RegionFile {
    #[allow(dead_code)]
    cx: i32,
    #[allow(dead_code)]
    cz: i32,
}

fn read_entries(buf: &[u8]) -> io::Result<Vec<(i32, i32, usize, usize)>> {
    if buf.len() < HEADER_SIZE || &buf[0..4] != MAGIC {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "bad region magic"));
    }
    let version = u32::from_be_bytes(buf[4..8].try_into().unwrap());
    if version != VERSION {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "region version"));
    }
    let count = u32::from_be_bytes(buf[17..21].try_into().unwrap()) as usize;
    let mut entries = Vec::with_capacity(count);
    let mut pos = HEADER_SIZE;
    for _ in 0..count {
        if pos + ENTRY_SIZE > buf.len() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "region entries truncated"));
        }
        let cx = i32::from_be_bytes(buf[pos..pos + 4].try_into().unwrap());
        let cz = i32::from_be_bytes(buf[pos + 4..pos + 8].try_into().unwrap());
        let off = u32::from_be_bytes(buf[pos + 8..pos + 12].try_into().unwrap()) as usize;
        let len = u32::from_be_bytes(buf[pos + 12..pos + 16].try_into().unwrap()) as usize;
        entries.push((cx, cz, off, len));
        pos += ENTRY_SIZE;
    }
    Ok(entries)
}

/// 覆盖最大 state ID 所需位深(全局 ID 最大约 29872,需 15 位)。
fn section_bits(s: &Section) -> u32 {
    let mut max = 0u16;
    for i in 0..crate::chunk::SECTION_VOLUME {
        let v = s.get(i % 16, i / 256, (i / 16) % 16);
        if v > max {
            max = v;
        }
    }
    if max == 0 {
        return 0;
    }
    let mut bits = 4;
    while (1u32 << bits) <= max as u32 && bits < 32 {
        bits += 1;
    }
    bits
}

fn chunk_bytes(chunk: &Chunk) -> Vec<u8> {
    let mut out = Vec::with_capacity(24 * 3077);
    for s in chunk.sections() {
        if s.is_empty() {
            out.push(0);
            out.extend_from_slice(&0u32.to_be_bytes());
            continue;
        }
        let bits = section_bits(s);
        out.push(bits as u8);
        let packed = s.pack(bits);
        out.extend_from_slice(&(packed.len() as u32).to_be_bytes());
        for l in &packed {
            out.extend_from_slice(&l.to_be_bytes());
        }
    }
    out
}

fn parse_chunk(buf: &[u8], cx: i32, cz: i32) -> io::Result<Chunk> {
    let mut chunk = Chunk::new(cx, cz);
    let mut sections = [Section::new(); SECTIONS];
    let mut pos = 0usize;
    for s in sections.iter_mut() {
        if pos + 5 > buf.len() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "chunk data truncated"));
        }
        let bits = buf[pos] as u32;
        let len = u32::from_be_bytes(buf[pos + 1..pos + 5].try_into().unwrap()) as usize;
        pos += 5;
        if bits == 0 {
            continue;
        }
        if pos + len * 8 > buf.len() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "chunk longs truncated"));
        }
        let mut packed = Vec::with_capacity(len);
        for _ in 0..len {
            packed.push(u64::from_be_bytes(buf[pos..pos + 8].try_into().unwrap()));
            pos += 8;
        }
        s.unpack(bits, &packed);
    }
    for (i, sec) in sections.into_iter().enumerate() {
        chunk.sections_mut()[i] = sec;
    }
    Ok(chunk)
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

        let mut entries: Vec<(i32, i32, usize, usize)> = Vec::new();
        let mut data_map: Vec<(i32, i32, Vec<u8>)> = Vec::new();
        if let Ok(buf) = fs::read(&path) {
            if let Ok(es) = read_entries(&buf) {
                for (ex, ez, off, len) in es {
                    let end = off + len;
                    if end <= buf.len() && len > 0 {
                        data_map.push((ex, ez, buf[off..end].to_vec()));
                        entries.push((ex, ez, 0, 0));
                    }
                }
            }
        }
        entries.retain(|(ex, ez, _, _)| *ex != chunk.cx || *ez != chunk.cz);
        data_map.retain(|(ex, ez, _)| *ex != chunk.cx || *ez != chunk.cz);

        let new_data = chunk_bytes(chunk);
        entries.push((chunk.cx, chunk.cz, 0, 0));
        data_map.push((chunk.cx, chunk.cz, new_data));

        let mut out = Vec::with_capacity(HEADER_SIZE + entries.len() * ENTRY_SIZE);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_be_bytes());
        out.extend_from_slice(&seed.to_be_bytes());
        out.extend_from_slice(&[world_type as u8]);
        out.extend_from_slice(&(entries.len() as u32).to_be_bytes());

        let mut payload = Vec::new();
        let payload_base = HEADER_SIZE + entries.len() * ENTRY_SIZE;
        for (entry, (_, _, data)) in entries.iter_mut().zip(data_map.iter()) {
            entry.2 = payload_base + payload.len();
            entry.3 = data.len();
            payload.extend_from_slice(data);
        }
        for (ex, ez, off, len) in &entries {
            out.extend_from_slice(&ex.to_be_bytes());
            out.extend_from_slice(&ez.to_be_bytes());
            out.extend_from_slice(&(*off as u32).to_be_bytes());
            out.extend_from_slice(&(*len as u32).to_be_bytes());
        }
        out.extend_from_slice(&payload);
        fs::write(&path, out)
    }

    pub fn load(world_dir: &Path, cx: i32, cz: i32) -> io::Result<Chunk> {
        let path = Self::path_for(world_dir, cx, cz);
        let mut f = File::open(path)?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        let entries = read_entries(&buf)?;
        let (_, _, off, len) = entries
            .iter()
            .find(|(ex, ez, _, _)| *ex == cx && *ez == cz)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "chunk not in region"))?;
        let end = off + len;
        if end > buf.len() || *len == 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "chunk offset out of range"));
        }
        parse_chunk(&buf[*off..end], cx, cz)
    }

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
        c.set(0, MIN_Y, 0, crate::blocks::BEDROCK);
        c.set(7, 100, 9, crate::blocks::GRASS_BLOCK);
        RegionFile::save(&dir, &c, 1, WorldType::Superflat).unwrap();
        let loaded = RegionFile::load(&dir, 0, 0).unwrap();
        assert_eq!(loaded.get(0, MIN_Y, 0), crate::blocks::BEDROCK);
        assert_eq!(loaded.get(7, 100, 9), crate::blocks::GRASS_BLOCK);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn air_is_default() {
        let s = Section::new();
        assert!(s.get(0, 0, 0) == 0);
        assert_eq!(SECTION_VOLUME, 4096);
    }

    #[test]
    fn one_file_many_chunks() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let dir = std::env::temp_dir().join("mcs-region-test3");
                let _ = fs::remove_dir_all(&dir);
                let g = WorldGenerator::new(42, Normal);
                let coords = [(0, 0), (0, 1), (1, 0), (1, 1), (2, 2)];
                let generated: Vec<_> = coords.iter().map(|&(cx, cz)| g.generate(cx, cz)).collect();
                for c in &generated {
                    RegionFile::save(&dir, c, 42, Normal).unwrap();
                }
                let files = RegionFile::count_files(&dir).unwrap();
                assert_eq!(files, 1, "同区域应只有一个文件");
                for ((cx, cz), orig) in coords.iter().zip(generated.iter()) {
                    let c = RegionFile::load(&dir, *cx, *cz).unwrap();
                    for x in 0..16 {
                        for z in 0..16 {
                            for y in (MIN_Y..MAX_Y).step_by(2) {
                                assert_eq!(c.get(x, y, z), orig.get(x, y, z), "({cx},{y},{z})");
                            }
                        }
                    }
                }
                RegionFile::save(&dir, &g.generate(1, 1), 42, Normal).unwrap();
                let c = RegionFile::load(&dir, 1, 1).unwrap();
                assert_eq!(c.get(8, 100, 8), g.generate(1, 1).get(8, 100, 8));
                let _ = fs::remove_dir_all(&dir);
            })
            .unwrap()
            .join()
            .unwrap();
    }
}