//! 大端字节缓冲:协议读写的基础设施。
//! `WriteBuf` 用于编码发送的包,`ReadBuf` 用于解码收到的包。

use crate::varint;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    EndOfBuffer,
    InvalidVarInt,
    InvalidStringUtf8,
    StringTooLong,
}

pub type Result<T> = core::result::Result<T, Error>;

pub const MAX_STRING_CHARS: usize = 32767;
pub const MAX_BYTES: usize = 2 * 1024 * 1024;

/// 写入缓冲。
#[derive(Debug, Default)]
pub struct WriteBuf {
    pub data: Vec<u8>,
}

impl WriteBuf {
    pub fn new() -> Self {
        WriteBuf { data: Vec::new() }
    }

    pub fn with_capacity(cap: usize) -> Self {
        WriteBuf {
            data: Vec::with_capacity(cap),
        }
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.data
    }

    pub fn write_u8(&mut self, v: u8) {
        self.data.push(v);
    }

    pub fn write_bool(&mut self, v: bool) {
        self.data.push(v as u8);
    }

    pub fn write_i8(&mut self, v: i8) {
        self.data.push(v as u8);
    }

    pub fn write_u16(&mut self, v: u16) {
        self.data.extend_from_slice(&v.to_be_bytes());
    }

    pub fn write_i16(&mut self, v: i16) {
        self.data.extend_from_slice(&v.to_be_bytes());
    }

    pub fn write_i32(&mut self, v: i32) {
        self.data.extend_from_slice(&v.to_be_bytes());
    }

    pub fn write_i64(&mut self, v: i64) {
        self.data.extend_from_slice(&v.to_be_bytes());
    }

    pub fn write_u64(&mut self, v: u64) {
        self.data.extend_from_slice(&v.to_be_bytes());
    }

    pub fn write_f32(&mut self, v: f32) {
        self.data.extend_from_slice(&v.to_be_bytes());
    }

    /// Position:x/z 各 26 位、y 12 位打包为 i64。
    pub fn write_position(&mut self, x: i32, y: i32, z: i32) {
        let v = ((x & 0x3FFFFFF) as i64) << 38 | ((z & 0x3FFFFFF) as i64) << 12 | (y & 0xFFF) as i64;
        self.write_i64(v);
    }

    pub fn write_f64(&mut self, v: f64) {
        self.data.extend_from_slice(&v.to_be_bytes());
    }

    pub fn write_varint_u32(&mut self, v: u32) {
        varint::write_varint(v, &mut self.data);
    }

    pub fn write_varint(&mut self, v: i32) {
        varint::write_varint_i32(v, &mut self.data);
    }

    pub fn write_varlong(&mut self, v: u64) {
        varint::write_varlong(v, &mut self.data);
    }

    /// 字符串:VarInt 长度 + UTF-8 字节。
    pub fn write_string(&mut self, s: &str) {
        self.write_varint(s.len() as i32);
        self.data.extend_from_slice(s.as_bytes());
    }

    /// 原始字节(不带长度前缀)。
    pub fn write_raw(&mut self, bytes: &[u8]) {
        self.data.extend_from_slice(bytes);
    }

    /// Prefixed Array of Byte:VarInt 长度 + 字节。
    pub fn write_byte_array(&mut self, bytes: &[u8]) {
        self.write_varint(bytes.len() as i32);
        self.data.extend_from_slice(bytes);
    }

    /// VarInt 长度前缀的 byte 数组(压缩场景用)。
    pub fn write_var_bytes(&mut self, bytes: &[u8]) {
        self.write_varint(bytes.len() as i32);
        self.data.extend_from_slice(bytes);
    }

    /// Long 数组:VarInt 长度 + 每个 VarLong。
    pub fn write_long_array(&mut self, longs: &[u64]) {
        self.write_varint(longs.len() as i32);
        for l in longs {
            self.write_varlong(*l);
        }
    }

    /// Long 数组(裸写,不带长度前缀)。
    pub fn write_long_array_raw(&mut self, longs: &[u64]) {
        for l in longs {
            self.write_varlong(*l);
        }
    }

    /// UUID:两个大端 u64。
    pub fn write_uuid(&mut self, uuid: [u8; 16]) {
        self.data.extend_from_slice(&uuid);
    }
}

/// 读取缓冲。
#[derive(Debug)]
pub struct ReadBuf<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> ReadBuf<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        ReadBuf { buf, pos: 0 }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn set_pos(&mut self, p: usize) {
        self.pos = p;
    }

    pub fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    pub fn finished(&self) -> bool {
        self.pos >= self.buf.len()
    }

    pub fn read_u8(&mut self) -> Result<u8> {
        let b = self.buf.get(self.pos).ok_or(Error::EndOfBuffer)?;
        self.pos += 1;
        Ok(*b)
    }

    pub fn read_i8(&mut self) -> Result<i8> {
        Ok(self.read_u8()? as i8)
    }

    pub fn read_bool(&mut self) -> Result<bool> {
        Ok(self.read_u8()? != 0)
    }

    pub fn read_i16(&mut self) -> Result<i16> {
        let b = self.read_bytes(2)?;
        Ok(i16::from_be_bytes([b[0], b[1]]))
    }

    pub fn read_u16(&mut self) -> Result<u16> {
        let b = self.read_bytes(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    pub fn read_i32(&mut self) -> Result<i32> {
        let b = self.read_bytes(4)?;
        Ok(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn read_i64(&mut self) -> Result<i64> {
        let b = self.read_bytes(8)?;
        Ok(i64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }

    pub fn read_u64(&mut self) -> Result<u64> {
        let b = self.read_bytes(8)?;
        Ok(u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }

    pub fn read_f32(&mut self) -> Result<f32> {
        Ok(f32::from_be_bytes(self.read_bytes(4)?.try_into().unwrap()))
    }

    pub fn read_f64(&mut self) -> Result<f64> {
        Ok(f64::from_be_bytes(self.read_bytes(8)?.try_into().unwrap()))
    }

    /// Position:i64 解出 (x, y, z)。
    pub fn read_position(&mut self) -> Result<(i32, i32, i32)> {
        let v = self.read_i64()?;
        let x = (v >> 38) as i32;
        let y = (v << 52 >> 52) as i32;
        let z = (v << 26 >> 38) as i32;
        Ok((x, y, z))
    }

    pub fn read_varint(&mut self) -> Result<i32> {
        varint::decode_varint_i32(self.buf, &mut self.pos).ok_or(Error::InvalidVarInt)
    }

    pub fn read_varlong(&mut self) -> Result<u64> {
        varint::decode_varlong(self.buf, &mut self.pos).ok_or(Error::InvalidVarInt)
    }

    pub fn read_string(&mut self) -> Result<String> {
        let len = self.read_varint()?;
        if len < 0 || len as usize > MAX_STRING_CHARS {
            return Err(Error::StringTooLong);
        }
        let bytes = self.read_bytes(len as usize)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| Error::InvalidStringUtf8)
    }

    /// 读取任意长度的原始字节切片。
    pub fn read_bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        if n > MAX_BYTES {
            return Err(Error::EndOfBuffer);
        }
        let end = self.pos.checked_add(n).ok_or(Error::EndOfBuffer)?;
        if end > self.buf.len() {
            return Err(Error::EndOfBuffer);
        }
        let slice = &self.buf[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    /// Prefixed Array of Byte:VarInt 长度 + 字节。
    pub fn read_byte_array(&mut self) -> Result<&'a [u8]> {
        let len = self.read_varint()?;
        if len < 0 {
            return Err(Error::EndOfBuffer);
        }
        self.read_bytes(len as usize)
    }

    pub fn read_uuid(&mut self) -> Result<[u8; 16]> {
        let b = self.read_bytes(16)?;
        let mut out = [0u8; 16];
        out.copy_from_slice(b);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_read_roundtrip() {
        let mut w = WriteBuf::new();
        w.write_u8(0xAB);
        w.write_i16(-1234);
        w.write_i32(0x1234_5678);
        w.write_i64(-1);
        w.write_f32(1.5);
        w.write_f64(-2.25);
        w.write_varint(300);
        w.write_string("hello");
        w.write_bool(true);
        w.write_byte_array(&[1, 2, 3]);

        let mut r = ReadBuf::new(&w.data);
        assert_eq!(r.read_u8().unwrap(), 0xAB);
        assert_eq!(r.read_i16().unwrap(), -1234);
        assert_eq!(r.read_i32().unwrap(), 0x1234_5678);
        assert_eq!(r.read_i64().unwrap(), -1);
        assert_eq!(r.read_f32().unwrap(), 1.5);
        assert_eq!(r.read_f64().unwrap(), -2.25);
        assert_eq!(r.read_varint().unwrap(), 300);
        assert_eq!(r.read_string().unwrap(), "hello");
        assert_eq!(r.read_bool().unwrap(), true);
        assert_eq!(r.read_byte_array().unwrap(), &[1, 2, 3]);
        assert!(r.finished());
    }

    #[test]
    fn truncated_fails() {
        let mut r = ReadBuf::new(&[0x01]);
        assert!(r.read_i32().is_err());
    }
}
