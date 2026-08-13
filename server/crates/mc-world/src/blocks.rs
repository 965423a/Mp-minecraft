//! 方块 ID 常量(对齐 MC 经典数据值,简化版)。

pub const AIR: u16 = 0;
pub const STONE: u16 = 1;
pub const GRASS_BLOCK: u16 = 2;
pub const DIRT: u16 = 3;
pub const BEDROCK: u16 = 7;
pub const WATER: u16 = 9;
pub const SAND: u16 = 12;
pub const GRAVEL: u16 = 13;
pub const COBBLESTONE: u16 = 4;
pub const OAK_LOG: u16 = 17;
pub const OAK_LEAVES: u16 = 18;
pub const SNOW_BLOCK: u16 = 80;
pub const ICE: u16 = 79;
pub const CLAY: u16 = 82;
pub const BROWN_MUSHROOM: u16 = 39;
pub const RED_MUSHROOM: u16 = 40;
pub const DANDELION: u16 = 37;
pub const POPPY: u16 = 38;
pub const OAK_SAPLING: u16 = 6;

/// 方块显示名(用于日志/统计)。
pub fn block_name(id: u16) -> &'static str {
    match id {
        AIR => "minecraft:air",
        STONE => "minecraft:stone",
        GRASS_BLOCK => "minecraft:grass_block",
        DIRT => "minecraft:dirt",
        BEDROCK => "minecraft:bedrock",
        WATER => "minecraft:water",
        SAND => "minecraft:sand",
        GRAVEL => "minecraft:gravel",
        COBBLESTONE => "minecraft:cobblestone",
        OAK_LOG => "minecraft:oak_log",
        OAK_LEAVES => "minecraft:oak_leaves",
        SNOW_BLOCK => "minecraft:snow_block",
        ICE => "minecraft:ice",
        CLAY => "minecraft:clay",
        _ => "minecraft:unknown",
    }
}