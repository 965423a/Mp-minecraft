//! VarInt / VarLong 编解码。

use alloc::vec::Vec;

/// 将一个 u32 编码为 VarInt 写入 `out`,返回写入字节数。
pub fn write_varint(mut value: u32, out: &mut Vec<u8>) -> usize {
    let start = out.len();
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            out.push(byte | 0x80);
        } else {
            out.push(byte);
            break;
        }
    }
    out.len() - start
}

/// 将 i32 编码为 VarInt。
pub fn write_varint_i32(value: i32, out: &mut Vec<u8>) -> usize {
    write_varint(value as u32, out)
}

/// 将 u64 编码为 VarLong,返回写入字节数。
pub fn write_varlong(mut value: u64, out: &mut Vec<u8>) -> usize {
    let start = out.len();
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            out.push(byte | 0x80);
        } else {
            out.push(byte);
            break;
        }
    }
    out.len() - start
}

/// 解码一个 VarInt。返回 (值, 消耗字节数),非法或截断返回 None。
pub fn decode_varint(buf: &[u8], pos: &mut usize) -> Option<u32> {
    let mut result: u32 = 0;
    let mut shift = 0u32;
    loop {
        let byte = *buf.get(*pos)?;
        *pos += 1;
        if shift == 35 {
            // 最多 5 字节
            return None;
        }
        result |= ((byte & 0x7F) as u32) << shift;
        if byte & 0x80 == 0 {
            return Some(result);
        }
        shift += 7;
    }
}

/// 解码一个 VarInt(i32 视角)。
pub fn decode_varint_i32(buf: &[u8], pos: &mut usize) -> Option<i32> {
    decode_varint(buf, pos).map(|v| v as i32)
}

/// 解码一个 VarLong。返回 (值, 消耗字节数)。
pub fn decode_varlong(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let mut result: u64 = 0;
    let mut shift = 0u32;
    loop {
        let byte = *buf.get(*pos)?;
        *pos += 1;
        if shift >= 70 {
            return None;
        }
        result |= ((byte & 0x7F) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some(result);
        }
        shift += 7;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip_u32(v: u32) {
        let mut buf = Vec::new();
        write_varint(v, &mut buf);
        let mut pos = 0;
        assert_eq!(decode_varint(&buf, &mut pos), Some(v));
        assert_eq!(pos, buf.len());
    }

    #[test]
    fn varint_roundtrip() {
        roundtrip_u32(0);
        roundtrip_u32(1);
        roundtrip_u32(127);
        roundtrip_u32(128);
        roundtrip_u32(255);
        roundtrip_u32(300);
        roundtrip_u32(16384);
        roundtrip_u32(0x7FFF_FFFF);
        roundtrip_u32(u32::MAX);
        roundtrip_u32(0x7F7F_7F7F);
    }

    #[test]
    fn varint_known_encoding() {
        let mut buf = Vec::new();
        write_varint(300, &mut buf);
        assert_eq!(buf, vec![0xAC, 0x02]);
        buf.clear();
        write_varint(0, &mut buf);
        assert_eq!(buf, vec![0x00]);
    }

    #[test]
    fn varlong_roundtrip() {
        for v in [0u64, 1, 127, 128, 300, 1 << 32, u64::MAX] {
            let mut buf = Vec::new();
            write_varlong(v, &mut buf);
            let mut pos = 0;
            assert_eq!(decode_varlong(&buf, &mut pos), Some(v));
            assert_eq!(pos, buf.len());
        }
    }

    #[test]
    fn truncated_is_none() {
        let buf = [0x80u8];
        let mut pos = 0;
        assert_eq!(decode_varint(&buf, &mut pos), None);
    }
}
