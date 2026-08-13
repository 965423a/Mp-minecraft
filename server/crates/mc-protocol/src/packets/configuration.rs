//! Configuration state 包(1.20.2+ 强制经过)。

use crate::buf::{ReadBuf, Result, WriteBuf};

pub mod clientbound {
    use super::*;

    pub const ID_REGISTRY_DATA: i32 = 0x07;
    pub const ID_FEATURE_FLAGS: i32 = 0x0C;
    pub const ID_UPDATE_TAGS: i32 = 0x0D;
    pub const ID_KNOWN_PACKS: i32 = 0x0E;
    pub const ID_FINISH_CONFIGURATION: i32 = 0x03;
    pub const ID_PLUGIN_MESSAGE: i32 = 0x01;
    pub const ID_DISCONNECT: i32 = 0x02;

    /// Registry Data (0x07):registry id + 条目数组 {id, optional NBT}。
    /// `entries`: (entry_id, nbt_bytes)。nbt_bytes 为完整 unnamed NBT(含类型字节)。
    pub fn write_registry_data(w: &mut WriteBuf, registry_id: &str, entries: &[(&str, Option<&[u8]>)]) {
        w.write_varint(ID_REGISTRY_DATA);
        w.write_string(registry_id);
        w.write_varint(entries.len() as i32);
        for (id, nbt) in entries {
            w.write_string(id);
            match nbt {
                Some(bytes) => w.write_byte_array(bytes),
                None => w.write_varint(0), // 无 NBT
            }
        }
    }

    /// Feature Flags (0x0C)。
    pub fn write_feature_flags(w: &mut WriteBuf, flags: &[&str]) {
        w.write_varint(ID_FEATURE_FLAGS);
        w.write_varint(flags.len() as i32);
        for f in flags {
            w.write_string(f);
        }
    }

    /// Update Tags (0x0D)。空 tags 包即可满足大部分场景。
    pub fn write_update_tags_empty(w: &mut WriteBuf) {
        w.write_varint(ID_UPDATE_TAGS);
        w.write_varint(0);
    }

    /// Known Packs (0x0E)。
    pub fn write_known_packs(w: &mut WriteBuf, packs: &[(&str, &str, &str)]) {
        w.write_varint(ID_KNOWN_PACKS);
        w.write_varint(packs.len() as i32);
        for (ns, id, ver) in packs {
            w.write_string(ns);
            w.write_string(id);
            w.write_string(ver);
        }
    }

    /// Finish Configuration (0x03):无字段。
    pub fn write_finish_configuration(w: &mut WriteBuf) {
        w.write_varint(ID_FINISH_CONFIGURATION);
    }

    /// Plugin Message (0x01)。
    pub fn write_plugin_message(w: &mut WriteBuf, channel: &str, data: &[u8]) {
        w.write_varint(ID_PLUGIN_MESSAGE);
        w.write_string(channel);
        w.write_raw(data);
    }

    /// Disconnect (0x02)。
    pub fn write_disconnect(w: &mut WriteBuf, reason_json: &str) {
        w.write_varint(ID_DISCONNECT);
        w.write_string(reason_json);
    }
}

pub mod serverbound {
    use super::*;

    pub const ID_CLIENT_INFORMATION: i32 = 0x00;
    pub const ID_ACK_FINISH_CONFIGURATION: i32 = 0x03;
    pub const ID_KNOWN_PACKS: i32 = 0x07;
    pub const ID_PLUGIN_MESSAGE: i32 = 0x02;

    /// Client Information (0x00)。
    #[derive(Debug, Default)]
    pub struct ClientInformation {
        pub locale: String,
        pub view_distance: i8,
        pub chat_mode: i32,
        pub chat_colors: bool,
        pub displayed_skin_parts: u8,
        pub main_hand: i32,
        pub enable_text_filtering: bool,
        pub allow_server_listings: bool,
        pub particle_status: i32,
    }

    pub fn read_client_information(r: &mut ReadBuf) -> Result<ClientInformation> {
        Ok(ClientInformation {
            locale: r.read_string()?,
            view_distance: r.read_i16()? as i8,
            chat_mode: r.read_varint()?,
            chat_colors: r.read_bool()?,
            displayed_skin_parts: r.read_u8()?,
            main_hand: r.read_varint()?,
            enable_text_filtering: r.read_bool()?,
            allow_server_listings: r.read_bool()?,
            particle_status: r.read_varint()?,
        })
    }

    /// Known Packs 应答(0x07):客户端回执我们发的包列表。
    #[derive(Debug, Default)]
    pub struct KnownPacks {
        pub packs: Vec<(String, String, String)>,
    }

    pub fn read_known_packs(r: &mut ReadBuf) -> Result<KnownPacks> {
        let n = r.read_varint()?;
        let mut packs = Vec::with_capacity(n.max(0) as usize);
        for _ in 0..n {
            let ns = r.read_string()?;
            let id = r.read_string()?;
            let ver = r.read_string()?;
            packs.push((ns, id, ver));
        }
        Ok(KnownPacks { packs })
    }

    /// Plugin Message (0x02)。
    pub fn read_plugin_message(r: &mut ReadBuf) -> Result<(String, &[u8])> {
        let channel = r.read_string()?;
        let data = r.read_bytes(r.remaining())?;
        Ok((channel, data))
    }
}
