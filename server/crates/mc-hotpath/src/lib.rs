//! C FFI 热路径绑定 + Rust 参考实现 + 随机交叉验证。
//! C 源码:server/native/{varint,bitpack}.c

#![no_std]
extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use core::ffi::c_char;

// ---------------- FFI 声明 ----------------

unsafe extern "C" {
    fn mcs_varint_encode(value: u32, out: *mut c_char) -> usize;
    fn mcs_varint_decode(buf: *const c_char, len: usize, consumed: *mut usize) -> i64;
    fn mcs_pack_section(blocks: *const u16, bits: u32, out: *mut u64, out_len: *mut usize);
    fn mcs_unpack_section(packed: *const u64, packed_len: usize, bits: u32, out: *mut u16);
}

// ---------------- Rust 包装 ----------------

/// C 实现:VarInt 编码。返回编码字节。
pub fn c_encode_varint(value: u32) -> Vec<u8> {
    let mut out = [0u8; 5];
    let n = unsafe { mcs_varint_encode(value, out.as_mut_ptr() as *mut c_char) };
    out[..n].to_vec()
}

/// C 实现:VarInt 解码。None 表示非法。
pub fn c_decode_varint(buf: &[u8]) -> Option<(u32, usize)> {
    let mut consumed = 0usize;
    let v = unsafe { mcs_varint_decode(buf.as_ptr() as *const c_char, buf.len(), &mut consumed) };
    if v < 0 {
        None
    } else {
        Some((v as u32, consumed))
    }
}

/// C 实现:区块 section 位打包。
pub fn c_pack_section(blocks: &[u16; 4096], bits: u32) -> Vec<u64> {
    let per_long = 64 / bits;
    let longs = (4096 + per_long - 1) / per_long;
    let mut out = vec![0u64; longs as usize];
    let mut out_len = 0usize;
    unsafe {
        mcs_pack_section(blocks.as_ptr(), bits, out.as_mut_ptr(), &mut out_len);
    }
    out.truncate(out_len);
    out
}

/// C 实现:解包。
pub fn c_unpack_section(packed: &[u64], bits: u32) -> Vec<u16> {
    let mut out = vec![0u16; 4096];
    unsafe {
        mcs_unpack_section(packed.as_ptr(), packed.len(), bits, out.as_mut_ptr());
    }
    out
}

// ---------------- Rust 参考实现 ----------------

/// Rust 参考:VarInt 编码。
pub fn r_encode_varint(mut value: u32) -> Vec<u8> {
    let mut out = vec![];
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
    out
}

/// Rust 参考:section 位打包。
pub fn r_pack_section(blocks: &[u16; 4096], bits: u32) -> Vec<u64> {
    let per_long = 64 / bits;
    let longs = (4096 + per_long - 1) / per_long;
    let mut out = vec![0u64; longs as usize];
    let mask = (1u64 << bits) - 1;
    for i in 0..4096usize {
        let (word, offset) = (i / per_long as usize, (i % per_long as usize) * bits as usize);
        out[word] |= (blocks[i] as u64 & mask) << offset;
    }
    out
}

// ---------------- 测试 ----------------

/// 确定性伪随机(XorShift32),保证可复现。
fn rng(seed: &mut u32) -> u32 {
    let mut x = *seed;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *seed = x;
    x
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate alloc;

    #[test]
    fn varint_matches_reference() {
        let mut seed = 0xC0FFEEu32;
        for _ in 0..2000 {
            let v = rng(&mut seed);
            let c = c_encode_varint(v);
            let r = r_encode_varint(v);
            assert_eq!(c, r, "encode mismatch for {v}");
            let (dv, n) = c_decode_varint(&c).unwrap();
            assert_eq!(dv, v);
            assert_eq!(n, c.len());
        }
        // 边界值
        for v in [0u32, 1, 127, 128, 255, 300, 16_384, 0x7FFF_FFFF, u32::MAX] {
            let c = c_encode_varint(v);
            assert_eq!(c, r_encode_varint(v), "boundary {v}");
        }
    }

    #[test]
    fn varint_truncated_returns_none() {
        assert!(c_decode_varint(&[0x80]).is_none());
        assert!(c_decode_varint(&[]).is_none());
        assert!(c_decode_varint(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01]).is_none());
    }

    #[test]
    fn pack_matches_reference_all_bits() {
        let mut seed = 0xABABABu32;
        for bits in [4u32, 5, 6, 8, 9, 12, 15, 16] {
            let max = 1u32 << bits;
            let mut blocks = [0u16; 4096];
            for i in 0..4096 {
                blocks[i] = (rng(&mut seed) % max) as u16;
            }
            let c = c_pack_section(&blocks, bits);
            let r = r_pack_section(&blocks, bits);
            assert_eq!(c, r, "pack mismatch bits={bits}");
            // C 解包还原
            let back = c_unpack_section(&c, bits);
            assert_eq!(back, blocks.to_vec(), "unpack mismatch bits={bits}");
        }
    }

    #[test]
    fn pack_single_values() {
        let mut blocks = [0u16; 4096];
        blocks[0] = 2;
        blocks[4095] = 7;
        blocks[1234] = 3;
        for bits in [4u32, 8, 12, 16] {
            let c = c_pack_section(&blocks, bits);
            let back = c_unpack_section(&c, bits);
            assert_eq!(back[0], 2);
            assert_eq!(back[4095], 7);
            assert_eq!(back[1234], 3);
        }
    }

    #[test]
    fn packed_lengths_correct() {
        let blocks = [0u16; 4096];
        assert_eq!(c_pack_section(&blocks, 4).len(), 256);
        assert_eq!(c_pack_section(&blocks, 8).len(), 512);
        assert_eq!(c_pack_section(&blocks, 12).len(), 820);
        assert_eq!(c_pack_section(&blocks, 16).len(), 1024);
    }

    #[test]
    fn ffi_pointers_are_stable() {
        // 确保 FFI 符号真的链接到了 C 实现(非空地址)
        let c = c_encode_varint(300);
        assert_eq!(c, [0xAC, 0x02]);
    }
}