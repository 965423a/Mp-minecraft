//! 方块常量:全局 block state ID(原版注册表顺序,air=0;与 26.1.2 BuiltInRegistries.BLOCK 一致)。

pub const AIR: u16 = 0;
pub const STONE: u16 = 1;
pub const GRASS_BLOCK: u16 = 9;
pub const DIRT: u16 = 10;
pub const COBBLESTONE: u16 = 14;
pub const OAK_SAPLING: u16 = 29;
pub const BEDROCK: u16 = 85;
pub const WATER: u16 = 86;
pub const SAND: u16 = 118;
pub const GRAVEL: u16 = 124;
pub const OAK_LOG: u16 = 137;
pub const OAK_LEAVES: u16 = 279;
pub const DANDELION: u16 = 2321;
pub const POPPY: u16 = 2324;
pub const BROWN_MUSHROOM: u16 = 2336;
pub const RED_MUSHROOM: u16 = 2337;
pub const ICE: u16 = 6927;
pub const SNOW_BLOCK: u16 = 6928;
pub const CLAY: u16 = 6946;

pub fn block_name(id: u16) -> &'static str {
    match id {
        AIR => "minecraft:air",
        STONE => "minecraft:stone",
        GRASS_BLOCK => "minecraft:grass_block",
        DIRT => "minecraft:dirt",
        COBBLESTONE => "minecraft:cobblestone",
        OAK_SAPLING => "minecraft:oak_sapling",
        BEDROCK => "minecraft:bedrock",
        WATER => "minecraft:water",
        SAND => "minecraft:sand",
        GRAVEL => "minecraft:gravel",
        OAK_LOG => "minecraft:oak_log",
        OAK_LEAVES => "minecraft:oak_leaves",
        DANDELION => "minecraft:dandelion",
        POPPY => "minecraft:poppy",
        BROWN_MUSHROOM => "minecraft:brown_mushroom",
        RED_MUSHROOM => "minecraft:red_mushroom",
        ICE => "minecraft:ice",
        SNOW_BLOCK => "minecraft:snow_block",
        CLAY => "minecraft:clay",
        _ => "minecraft:unknown",
    }
}