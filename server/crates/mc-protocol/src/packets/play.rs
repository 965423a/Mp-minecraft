//! Play state 包(仅实现服务器需要的最小集合)。

use crate::buf::{ReadBuf, Result, WriteBuf};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

pub mod clientbound {
    use super::*;
    use alloc::vec;

    pub const ID_JOIN_GAME: i32 = 0x31;
    pub const ID_CHUNK_DATA: i32 = 0x2D;
    pub const ID_KEEP_ALIVE: i32 = 0x2C;
    pub const ID_SYNC_PLAYER_POSITION: i32 = 0x48;
    pub const ID_PLAYER_INFO_UPDATE: i32 = 0x46;

    #[derive(Debug, Clone)]
    pub struct JoinGame {
        pub entity_id: i32,
        pub hardcore: bool,
        pub dimension_names: Vec<String>,
        pub max_players: i32,
        pub view_distance: i32,
        pub simulation_distance: i32,
        pub reduced_debug_info: bool,
        pub enable_respawn_screen: bool,
        pub do_limited_crafting: bool,
        pub dimension_type: i32,
        pub dimension_name: String,
        pub hashed_seed: i64,
        pub gamemode: u8,
        pub previous_gamemode: i8,
        pub is_debug: bool,
        pub is_flat: bool,
        pub portal_cooldown: i32,
        pub sea_level: i32,
        pub enforces_secure_chat: bool,
    }

    /// 编码 Login(play)/Join Game (0x31)。
    /// 注意:775 无 online_mode 字段(776+ 才有)。
    pub fn write_join_game(w: &mut WriteBuf, p: &JoinGame) {
        w.write_varint(ID_JOIN_GAME);
        w.write_i32(p.entity_id);
        w.write_bool(p.hardcore);
        w.write_varint(p.dimension_names.len() as i32);
        for d in &p.dimension_names {
            w.write_string(d);
        }
        w.write_varint(p.max_players);
        w.write_varint(p.view_distance);
        w.write_varint(p.simulation_distance);
        w.write_bool(p.reduced_debug_info);
        w.write_bool(p.enable_respawn_screen);
        w.write_bool(p.do_limited_crafting);
        w.write_varint(p.dimension_type);
        w.write_string(&p.dimension_name);
        w.write_i64(p.hashed_seed);
        w.write_u8(p.gamemode);
        w.write_i8(p.previous_gamemode);
        w.write_bool(p.is_debug);
        w.write_bool(p.is_flat);
        w.write_bool(false); // has_death_location
        w.write_varint(p.portal_cooldown);
        w.write_varint(p.sea_level);
        w.write_bool(p.enforces_secure_chat);
    }

    /// 编码 Synchronize Player Position (0x48)。
    #[allow(clippy::too_many_arguments)]
    pub fn write_sync_player_position(
        w: &mut WriteBuf,
        teleport_id: i32,
        x: f64,
        y: f64,
        z: f64,
        vx: f64,
        vy: f64,
        vz: f64,
        yaw: f32,
        pitch: f32,
        flags: i32,
    ) {
        w.write_varint(ID_SYNC_PLAYER_POSITION);
        w.write_varint(teleport_id);
        w.write_f64(x);
        w.write_f64(y);
        w.write_f64(z);
        w.write_f64(vx);
        w.write_f64(vy);
        w.write_f64(vz);
        w.write_f32(yaw);
        w.write_f32(pitch);
        w.write_i32(flags);
    }

    /// 编码 Keep Alive (0x2C)。
    pub fn write_keep_alive(w: &mut WriteBuf, id: i64) {
        w.write_varint(ID_KEEP_ALIVE);
        w.write_i64(id);
    }

    /// Player Info Update (0x46),仅 Add Player 动作(0x01)。
    pub fn write_player_info_add(w: &mut WriteBuf, uuid: [u8; 16], name: &str, properties: &[(&str, &str)]) {
        w.write_varint(ID_PLAYER_INFO_UPDATE);
        w.write_u8(0x01);
        w.write_varint(1);
        w.write_uuid(uuid);
        w.write_string(name);
        w.write_varint(properties.len() as i32);
        for (name_p, value) in properties {
            w.write_string(name_p);
            w.write_string(value);
            w.write_bool(false);
        }
    }

    /// Chunk Data and Update Light (0x2D)。
    /// 字段:ChunkX, ChunkZ, Heightmaps(前缀数组), Data(前缀字节数组),
    ///      Block Entities(前缀数组), Light(变长,读到包尾)。
    pub fn write_chunk_data(
        w: &mut WriteBuf,
        chunk_x: i32,
        chunk_z: i32,
        heightmaps: &[(&[u64], u32)], // (packed longs, type ordinal)
        data: &[u8],
        block_entities: &[u8],
        light: &[u8],
    ) {
        w.write_varint(ID_CHUNK_DATA);
        w.write_i32(chunk_x);
        w.write_i32(chunk_z);
        w.write_varint(heightmaps.len() as i32);
        for (longs, ty) in heightmaps {
            w.write_varint(*ty as i32);
            w.write_long_array(longs);
        }
        w.write_var_bytes(data);
        w.write_raw(block_entities);
        w.write_raw(light);
    }
}

pub mod serverbound {
    use super::*;
    use alloc::vec;

    pub const ID_CONFIRM_TELEPORTATION: i32 = 0x00;
    pub const ID_KEEP_ALIVE: i32 = 0x1C;
    pub const ID_SET_POSITION: i32 = 0x1E;
    pub const ID_SET_POSITION_ROTATION: i32 = 0x1F;
    pub const ID_PLAYER_LOADED: i32 = 0x2C;

    /// Keep Alive (0x1C):i64 id。
    pub fn read_keep_alive(r: &mut ReadBuf) -> Result<i64> {
        r.read_i64()
    }

    /// Confirm Teleportation (0x00):VarInt teleport id。
    pub fn read_confirm_teleportation(r: &mut ReadBuf) -> Result<i32> {
        r.read_varint()
    }

    /// Set Player Position (0x1E) / Set Position+Rotation (0x1F)。
    #[derive(Debug)]
    pub struct PlayerPosition {
        pub x: f64,
        pub y: f64,
        pub z: f64,
        pub yaw: Option<f32>,
        pub pitch: Option<f32>,
        pub flags: i8,
    }

    pub fn read_player_position(r: &mut ReadBuf, with_rotation: bool) -> Result<PlayerPosition> {
        let x = r.read_f64()?;
        let y = r.read_f64()?;
        let z = r.read_f64()?;
        let (yaw, pitch) = if with_rotation {
            (Some(r.read_f32()?), Some(r.read_f32()?))
        } else {
            (None, None)
        };
        let flags = r.read_i8()?;
        Ok(PlayerPosition {
            x,
            y,
            z,
            yaw,
            pitch,
            flags,
        })
    }

    /// Player Loaded (0x2C):无字段。
    pub fn read_player_loaded(r: &mut ReadBuf) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::clientbound::*;
    use crate::buf::{ReadBuf, WriteBuf};
    use alloc::vec;

    #[test]
    fn join_game_encodes() {
        let mut w = WriteBuf::new();
        write_join_game(
            &mut w,
            &JoinGame {
                entity_id: 1,
                hardcore: false,
                dimension_names: vec!["minecraft:overworld".into()],
                max_players: 20,
                view_distance: 8,
                simulation_distance: 8,
                reduced_debug_info: false,
                enable_respawn_screen: true,
                do_limited_crafting: false,
                dimension_type: 0,
                dimension_name: "minecraft:overworld".into(),
                hashed_seed: 42,
                gamemode: 0,
                previous_gamemode: -1,
                is_debug: false,
                is_flat: true,
                portal_cooldown: 0,
                sea_level: 63,
                enforces_secure_chat: false,
            },
        );
        // 包 ID 之后应可正常读到字段。
        let mut r = ReadBuf::new(&w.data);
        assert_eq!(r.read_varint().unwrap(), ID_JOIN_GAME);
        assert_eq!(r.read_i32().unwrap(), 1);
        assert_eq!(r.read_bool().unwrap(), false);
    }
}