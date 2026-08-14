//! Status state 包。

use crate::buf::{ReadBuf, Result, WriteBuf};

pub mod clientbound {
    use super::*;

    /// Status Response (0x00)。
    pub fn write_response(w: &mut WriteBuf, json: &str) {
        w.write_varint(0x00);
        w.write_string(json);
    }

    /// Pong Response (0x01)。
    pub fn write_pong(w: &mut WriteBuf, timestamp: i64) {
        w.write_varint(0x01);
        w.write_i64(timestamp);
    }
}

pub mod serverbound {
    use super::*;

    /// Status Request (0x00)。无字段。
    pub fn write_request(w: &mut WriteBuf) {
        w.write_varint(0x00);
    }

    /// Ping Request (0x01):i64 时间戳。
    pub struct PingRequest {
        pub timestamp: i64,
    }

    pub fn read_ping(r: &mut ReadBuf) -> Result<PingRequest> {
        Ok(PingRequest {
            timestamp: r.read_i64()?,
        })
    }
}
