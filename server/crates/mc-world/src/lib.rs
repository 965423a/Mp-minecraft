//! mc-world:世界模型与地形生成。
//! 简化实现,确定性生成:同种子同坐标 → 同一世界。

pub mod blocks;
pub mod chunk;
pub mod generator;
pub mod noise;
pub mod region;

pub use chunk::{Chunk, Section, CHUNK_SIZE, MAX_Y, MIN_Y, SEA_LEVEL, SECTIONS};
pub use generator::{WorldGenerator, WorldType};
pub use noise::Noise;