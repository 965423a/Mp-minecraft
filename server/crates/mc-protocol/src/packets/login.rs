//! Login state 包(协议 775:Login Success 仅含 Game Profile,无 Session ID)。

use crate::buf::{ReadBuf, Result, WriteBuf};

pub mod clientbound {
    use super::*;

    /// Disconnect (0x00)。
    pub fn write_disconnect(w: &mut WriteBuf, reason_json: &str) {
        w.write_varint(0x00);
        w.write_string(reason_json);
    }

    /// Set Compression (0x03)。须在 Login Success 之前发送。
    pub fn write_set_compression(w: &mut WriteBuf, threshold: i32) {
        w.write_varint(0x03);
        w.write_varint(threshold);
    }

    /// Login Success (0x02)。775:Game Profile(UUID + 用户名 + properties)。
    pub fn write_success(w: &mut WriteBuf, uuid: [u8; 16], username: &str, properties: &[([u8; 16], &str)]) {
        w.write_varint(0x02);
        w.write_uuid(uuid);
        w.write_string(username);
        w.write_varint(properties.len() as i32);
        for (prop_uuid, value) in properties {
            w.write_string("textures");
            w.write_string(value);
            w.write_bool(false); // 无签名
            let _ = prop_uuid;
        }
    }
}

pub mod serverbound {
    use super::*;

    /// Login Start (0x00):用户名 + Player UUID。
    pub struct LoginStart {
        pub name: String,
        pub uuid: [u8; 16],
    }

    pub fn read_login_start(r: &mut ReadBuf) -> Result<LoginStart> {
        Ok(LoginStart {
            name: r.read_string()?,
            uuid: r.read_uuid()?,
        })
    }

    /// Login Acknowledged (0x03):无字段,客户端已进入 Configuration。
    pub fn read_login_acknowledged(r: &mut ReadBuf) -> Result<()> {
        Ok(())
    }
}
