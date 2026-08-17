// no_std 密码学原语,供 mcssh(SSH-2 服务器)使用:
// SHA-1 / SHA-256 / SHA-512、HMAC、ChaCha20、Poly1305、
// X25519(ECDH)、Ed25519(验签)。全部自实现,测试向量见 tests/。
#![no_std]
extern crate alloc;

// ============================== SHA-1 ===============================
const K1: [u32; 4] = [0x5A827999, 0x6ED9EBA1, 0x8F1BBCDC, 0xCA62C1D6];

/// 返回 20 字节 SHA-1 摘要。
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut w = [0u32; 80];
    let bitlen = (data.len() as u64).wrapping_mul(8);
    let padlen = {
        let rem = (data.len() + 9) % 64;
        if rem == 0 { 0 } else { 64 - rem }
    };
    let mut msg = alloc::vec::Vec::new();
    msg.extend_from_slice(data);
    msg.push(0x80);
    msg.extend_from_slice(&[0u8; 64][..padlen]);
    msg.extend_from_slice(&bitlen.to_be_bytes());
    for blk in msg.chunks(64) {
        for i in 0..16 {
            w[i] = u32::from_be_bytes([blk[i * 4], blk[i * 4 + 1], blk[i * 4 + 2], blk[i * 4 + 3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i / 20 {
                0 => ((b & c) | ((!b) & d), K1[0]),
                1 => (b ^ c ^ d, K1[1]),
                2 => ((b & c) | (b & d) | (c & d), K1[2]),
                _ => (b ^ c ^ d, K1[3]),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    let mut out = [0u8; 20];
    for (i, v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

// ============================== SHA-256 ==============================
const K256: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// 一次性 SHA-256,返回 32 字节。
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut w = [0u32; 64];
    let bitlen = (data.len() as u64).wrapping_mul(8);
    let padlen = {
        let rem = (data.len() + 9) % 64;
        if rem == 0 { 0 } else { 64 - rem }
    };
    let mut msg = alloc::vec::Vec::new();
    msg.extend_from_slice(data);
    msg.push(0x80);
    msg.extend_from_slice(&[0u8; 64][..padlen]);
    msg.extend_from_slice(&bitlen.to_be_bytes());
    for blk in msg.chunks(64) {
        for i in 0..16 {
            w[i] = u32::from_be_bytes([blk[i * 4], blk[i * 4 + 1], blk[i * 4 + 2], blk[i * 4 + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K256[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut out = [0u8; 32];
    for (i, v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

// ============================== SHA-512 ==============================
const K512: [u64; 80] = [
    0x428a2f98d728ae22, 0x7137449123ef65cd, 0xb5c0fbcfec4d3b2f, 0xe9b5dba58189dbbc,
    0x3956c25bf348b538, 0x59f111f1b605d019, 0x923f82a4af194f9b, 0xab1c5ed5da6d8118,
    0xd807aa98a3030242, 0x12835b0145706fbe, 0x243185be4ee4b28c, 0x550c7dc3d5ffb4e2,
    0x72be5d74f27b896f, 0x80deb1fe3b1696b1, 0x9bdc06a725c71235, 0xc19bf174cf692694,
    0xe49b69c19ef14ad2, 0xefbe4786384f25e3, 0x0fc19dc68b8cd5b5, 0x240ca1cc77ac9c65,
    0x2de92c6f592b0275, 0x4a7484aa6ea6e483, 0x5cb0a9dcbd41fbd4, 0x76f988da831153b5,
    0x983e5152ee66dfab, 0xa831c66d2db43210, 0xb00327c898fb213f, 0xbf597fc7beef0ee4,
    0xc6e00bf33da88fc2, 0xd5a79147930aa725, 0x06ca6351e003826f, 0x142929670a0e6e70,
    0x27b70a8546d22ffc, 0x2e1b21385c26c926, 0x4d2c6dfc5ac42aed, 0x53380d139d95b3df,
    0x650a73548baf63de, 0x766a0abb3c77b2a8, 0x81c2c92e47edaee6, 0x92722c851482353b,
    0xa2bfe8a14cf10364, 0xa81a664bbc423001, 0xc24b8b70d0f89791, 0xc76c51a30654be30,
    0xd192e819d6ef5218, 0xd69906245565a910, 0xf40e35855771202a, 0x106aa07032bbd1b8,
    0x19a4c116b8d2d0c8, 0x1e376c085141ab53, 0x2748774cdf8eeb99, 0x34b0bcb5e19b48a8,
    0x391c0cb3c5c95a63, 0x4ed8aa4ae3418acb, 0x5b9cca4f7763e373, 0x682e6ff3d6b2b8a3,
    0x748f82ee5defb2fc, 0x78a5636f43172f60, 0x84c87814a1f0ab72, 0x8cc702081a6439ec,
    0x90befffa23631e28, 0xa4506cebde82bde9, 0xbef9a3f7b2c67915, 0xc67178f2e372532b,
    0xca273eceea26619c, 0xd186b8c721c0c207, 0xeada7dd6cde0eb1e, 0xf57d4f7fee6ed178,
    0x06f067aa72176fba, 0x0a637dc5a2c898a6, 0x113f9804bef90dae, 0x1b710b35131c471b,
    0x28db77f523047d84, 0x32caab7b40c72493, 0x3c9ebe0a15c9bebc, 0x431d67c49c100d4c,
    0x4cc5d4becb3e42b6, 0x597f299cfc657e2a, 0x5fcb6fab3ad6faec, 0x6c44198c4a475817,
];

/// 一次性 SHA-512,返回 64 字节。
pub fn sha512(data: &[u8]) -> [u8; 64] {
    let mut h: [u64; 8] = [
        0x6a09e667f3bcc908, 0xbb67ae8584caa73b, 0x3c6ef372fe94f82b, 0xa54ff53a5f1d36f1,
        0x510e527fade682d1, 0x9b05688c2b3e6c1f, 0x1f83d9abfb41bd6b, 0x5be0cd19137e2179,
    ];
    let mut w = [0u64; 80];
    let bitlen = (data.len() as u64).wrapping_mul(8);
    let padlen = {
        let rem = (data.len() + 17) % 128;
        if rem == 0 { 0 } else { 128 - rem }
    };
    let mut msg = alloc::vec::Vec::new();
    msg.extend_from_slice(data);
    msg.push(0x80);
    msg.extend_from_slice(&[0u8; 128][..padlen]);
    msg.extend_from_slice(&(bitlen >> 32).to_be_bytes());
    msg.extend_from_slice(&(bitlen & 0xffff_ffff).to_be_bytes());
    for blk in msg.chunks(128) {
        for i in 0..16 {
            let mut b = [0u8; 8];
            b.copy_from_slice(&blk[i * 8..i * 8 + 8]);
            w[i] = u64::from_be_bytes(b);
        }
        for i in 16..80 {
            let s0 = w[i - 15].rotate_right(1) ^ w[i - 15].rotate_right(8) ^ (w[i - 15] >> 7);
            let s1 = w[i - 2].rotate_right(19) ^ w[i - 2].rotate_right(61) ^ (w[i - 2] >> 6);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..80 {
            let s1 = e.rotate_right(14) ^ e.rotate_right(18) ^ e.rotate_right(41);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K512[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(28) ^ a.rotate_right(34) ^ a.rotate_right(39);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut out = [0u8; 64];
    for (i, v) in h.iter().enumerate() {
        out[i * 8..i * 8 + 8].copy_from_slice(&v.to_be_bytes());
    }
    out
}

// =============================== HMAC ================================
/// HMAC-SHA256,返回 32 字节。
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&sha256(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0u8; 64];
    let mut opad = [0u8; 64];
    for i in 0..64 {
        ipad[i] = k[i] ^ 0x36;
        opad[i] = k[i] ^ 0x5c;
    }
    let mut inner = alloc::vec::Vec::with_capacity(64 + data.len());
    inner.extend_from_slice(&ipad);
    inner.extend_from_slice(data);
    let ih = sha256(&inner);
    let mut outer = alloc::vec::Vec::with_capacity(64 + 32);
    outer.extend_from_slice(&opad);
    outer.extend_from_slice(&ih);
    sha256(&outer)
}

/// HMAC-SHA512,返回 64 字节。
pub fn hmac_sha512(key: &[u8], data: &[u8]) -> [u8; 64] {
    let mut k = [0u8; 128];
    if key.len() > 128 {
        k[..64].copy_from_slice(&sha512(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0u8; 128];
    let mut opad = [0u8; 128];
    for i in 0..128 {
        ipad[i] = k[i] ^ 0x36;
        opad[i] = k[i] ^ 0x5c;
    }
    let mut inner = alloc::vec::Vec::with_capacity(128 + data.len());
    inner.extend_from_slice(&ipad);
    inner.extend_from_slice(data);
    let ih = sha512(&inner);
    let mut outer = alloc::vec::Vec::with_capacity(128 + 64);
    outer.extend_from_slice(&opad);
    outer.extend_from_slice(&ih);
    sha512(&outer)
}

// ============================= ChaCha20 ==============================
#[inline]
fn qr(s: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    s[a] = s[a].wrapping_add(s[b]);
    s[d] ^= s[a];
    s[d] = s[d].rotate_left(16);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] ^= s[c];
    s[b] = s[b].rotate_left(12);
    s[a] = s[a].wrapping_add(s[b]);
    s[d] ^= s[a];
    s[d] = s[d].rotate_left(8);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] ^= s[c];
    s[b] = s[b].rotate_left(7);
}

/// 生成 counter 起的一个 64 字节密钥流块。
fn chacha_block(key: &[u8; 32], nonce: &[u8; 12], counter: u32) -> [u8; 64] {
    let mut state = [0u32; 16];
    state[0] = 0x61707865;
    state[1] = 0x3320646e;
    state[2] = 0x79622d32;
    state[3] = 0x6b206574;
    for i in 0..8 {
        state[4 + i] =
            u32::from_le_bytes([key[i * 4], key[i * 4 + 1], key[i * 4 + 2], key[i * 4 + 3]]);
    }
    state[12] = counter;
    state[13] = u32::from_le_bytes([nonce[0], nonce[1], nonce[2], nonce[3]]);
    state[14] = u32::from_le_bytes([nonce[4], nonce[5], nonce[6], nonce[7]]);
    state[15] = u32::from_le_bytes([nonce[8], nonce[9], nonce[10], nonce[11]]);
    let mut q = state;
    for _ in 0..10 {
        qr(&mut q, 0, 4, 8, 12);
        qr(&mut q, 1, 5, 9, 13);
        qr(&mut q, 2, 6, 10, 14);
        qr(&mut q, 3, 7, 11, 15);
        qr(&mut q, 0, 5, 10, 15);
        qr(&mut q, 1, 6, 11, 12);
        qr(&mut q, 2, 7, 8, 13);
        qr(&mut q, 3, 4, 9, 14);
    }
    let mut out = [0u8; 64];
    for i in 0..16 {
        let v = q[i].wrapping_add(state[i]);
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}

/// ChaCha20 流加密(原地异或)。
pub fn chacha20_xor(key: &[u8; 32], nonce: &[u8; 12], counter: u32, data: &mut [u8]) {
    let mut ctr = counter;
    let mut off = 0usize;
    while off < data.len() {
        let block = chacha_block(key, nonce, ctr);
        let n = (data.len() - off).min(64);
        for i in 0..n {
            data[off + i] ^= block[i];
        }
        off += n;
        ctr = ctr.wrapping_add(1);
    }
}

// ============================== Poly1305 ==============================
#[inline]
fn load32_le(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// Poly1305 一次性认证(5×26-bit limbs mod 2^130-5),输出 16 字节 tag。
pub fn poly1305(key: &[u8; 32], msg: &[u8]) -> [u8; 16] {
    let r0 = load32_le(key, 0) & 0x3ff_ffff;
    let r1 = (load32_le(key, 3) >> 2) & 0x3ff_ff03;
    let r2 = (load32_le(key, 6) >> 4) & 0x3ff_c0ff;
    let r3 = (load32_le(key, 9) >> 6) & 0x3f0_3fff;
    let r4 = (load32_le(key, 12) >> 8) & 0x00f_ffff;
    let s1 = r1 * 5;
    let s2 = r2 * 5;
    let s3 = r3 * 5;
    let s4 = r4 * 5;
    let (mut h0, mut h1, mut h2, mut h3, mut h4) = (0u32, 0u32, 0u32, 0u32, 0u32);
    let mut h = [0u32; 5];
    let mut b = [0u8; 17];
    let mut i = 0;
    while i < msg.len() {
        let rem = (msg.len() - i).min(16);
        b = [0u8; 17];
        b[..rem].copy_from_slice(&msg[i..i + rem]);
        if rem < 16 {
            b[rem] = 1;
        } else {
            b[16] = 1;
        }
        h[0] = h[0].wrapping_add(load32_le(&b, 0) & 0x3ff_ffff);
        h[1] = h[1].wrapping_add((load32_le(&b, 3) >> 2) & 0x3ff_ffff);
        h[2] = h[2].wrapping_add((load32_le(&b, 6) >> 4) & 0x3ff_ffff);
        h[3] = h[3].wrapping_add((load32_le(&b, 9) >> 6) & 0x3ff_ffff);
        h[4] = h[4].wrapping_add(((load32_le(&b, 12) >> 8) & 0x3ff_ffff) | ((b[16] as u32) << 24));
        let h0 = h[0] as u64;
        let h1 = h[1] as u64;
        let h2 = h[2] as u64;
        let h3 = h[3] as u64;
        let h4 = h[4] as u64;
        let d0 = h0 * r0 as u64 + h1 * s4 as u64 + h2 * s3 as u64 + h3 * s2 as u64 + h4 * s1 as u64;
        let d1 = h0 * r1 as u64 + h1 * r0 as u64 + h2 * s4 as u64 + h3 * s3 as u64 + h4 * s2 as u64;
        let d2 = h0 * r2 as u64 + h1 * r1 as u64 + h2 * r0 as u64 + h3 * s4 as u64 + h4 * s3 as u64;
        let d3 = h0 * r3 as u64 + h1 * r2 as u64 + h2 * r1 as u64 + h3 * r0 as u64 + h4 * s4 as u64;
        let d4 = h0 * r4 as u64 + h1 * r3 as u64 + h2 * r2 as u64 + h3 * r1 as u64 + h4 * r0 as u64;
        let mut c = (d0 >> 26) as u32;
        h[0] = (d0 as u32) & 0x3ff_ffff;
        let d1 = d1 + c as u64;
        c = (d1 >> 26) as u32;
        h[1] = (d1 as u32) & 0x3ff_ffff;
        let d2 = d2 + c as u64;
        c = (d2 >> 26) as u32;
        h[2] = (d2 as u32) & 0x3ff_ffff;
        let d3 = d3 + c as u64;
        c = (d3 >> 26) as u32;
        h[3] = (d3 as u32) & 0x3ff_ffff;
        let d4 = d4 + c as u64;
        c = (d4 >> 26) as u32;
        h[4] = (d4 as u32) & 0x3ff_ffff;
        h[0] = h[0].wrapping_add(c.wrapping_mul(5));
        c = h[0] >> 26;
        h[0] &= 0x3ff_ffff;
        h[1] = h[1].wrapping_add(c);
        i += rem;
    }
    let (mut h0, mut h1, mut h2, mut h3, mut h4) = (h[0], h[1], h[2], h[3], h[4]);
    // 最终规约(与 donna 相同:进位 + 条件减 p)
    let mut c = h1 >> 26;
    h1 &= 0x3ff_ffff;
    h2 += c;
    c = h2 >> 26;
    h2 &= 0x3ff_ffff;
    h3 += c;
    c = h3 >> 26;
    h3 &= 0x3ff_ffff;
    h4 += c;
    c = h4 >> 26;
    h4 &= 0x3ff_ffff;
    h0 += c * 5;
    c = h0 >> 26;
    h0 &= 0x3ff_ffff;
    h1 += c;
    // g = h + 5(判断是否 >= p),再按符号选择
    let mut g0 = h0.wrapping_add(5);
    c = g0 >> 26;
    g0 &= 0x3ff_ffff;
    let mut g1 = h1.wrapping_add(c);
    c = g1 >> 26;
    g1 &= 0x3ff_ffff;
    let mut g2 = h2.wrapping_add(c);
    c = g2 >> 26;
    g2 &= 0x3ff_ffff;
    let mut g3 = h3.wrapping_add(c);
    c = g3 >> 26;
    g3 &= 0x3ff_ffff;
    let mut g4 = h4.wrapping_add(c);
    c = g4 >> 26;
    g4 = (g4 & 0x3ff_ffff).wrapping_sub(1 << 26);
    let mask = (g4 >> 31).wrapping_sub(1);
    g4 &= mask;
    g3 &= mask;
    g2 &= mask;
    g1 &= mask;
    g0 &= mask;
    h4 = (h4 & !mask) | g4;
    h3 = (h3 & !mask) | g3;
    h2 = (h2 & !mask) | g2;
    h1 = (h1 & !mask) | g1;
    h0 = (h0 & !mask) | g0;
    // 序列化 128 位(小端)+ 加 pad
    let lo = h0 as u64 | (h1 as u64) << 26 | (h2 as u64) << 52;
    let hi = (h2 >> 12) as u64 | (h3 as u64) << 14 | (h4 as u64) << 40;
    let mut f0 = lo as u32;
    let mut f1 = (lo >> 32) as u32;
    let mut f2 = hi as u32;
    let mut f3 = (hi >> 32) as u32;
    let mut carry = 0u64;
    let p0 = load32_le(key, 16) as u64;
    let p1 = load32_le(key, 20) as u64;
    let p2 = load32_le(key, 24) as u64;
    let p3 = load32_le(key, 28) as u64;
    let s = f0 as u64 + p0 + carry;
    f0 = s as u32;
    carry = s >> 32;
    let s = f1 as u64 + p1 + carry;
    f1 = s as u32;
    carry = s >> 32;
    let s = f2 as u64 + p2 + carry;
    f2 = s as u32;
    carry = s >> 32;
    let s = f3 as u64 + p3 + carry;
    f3 = s as u32;
    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&f0.to_le_bytes());
    out[4..8].copy_from_slice(&f1.to_le_bytes());
    out[8..12].copy_from_slice(&f2.to_le_bytes());
    out[12..16].copy_from_slice(&f3.to_le_bytes());
    out
}

// ====================== ChaCha20-Poly1305 AEAD =======================
/// 计算 poly1305 的 MAC 输入数据(ada 填充 + ct 填充 + 长度),返回 tag。
fn poly_mac(pkey: &[u8; 32], adata: &[u8], ct: &[u8]) -> [u8; 16] {
    let mut mac = alloc::vec::Vec::with_capacity(adata.len() + ct.len() + 48);
    mac.extend_from_slice(adata);
    let apad = (16 - (adata.len() % 16)) % 16;
    mac.extend_from_slice(&[0u8; 16][..apad]);
    mac.extend_from_slice(ct);
    let cpad = (16 - (ct.len() % 16)) % 16;
    mac.extend_from_slice(&[0u8; 16][..cpad]);
    mac.extend_from_slice(&(adata.len() as u64).to_le_bytes());
    mac.extend_from_slice(&(ct.len() as u64).to_le_bytes());
    poly1305(pkey, &mac)
}

/// ChaCha20-Poly1305 加密(RFC 8439):
/// 输出 ct = 密文 ++ tag(16),写入 ct;输入为明文。
pub fn chacha20_poly1305_encrypt(
    key: &[u8; 32],
    nonce: &[u8; 12],
    adata: &[u8],
    pt: &[u8],
    ct: &mut [u8],
) {
    let mut buf = alloc::vec::Vec::from(pt);
    chacha20_xor(key, nonce, 1, &mut buf);
    ct[..pt.len()].copy_from_slice(&buf);
    let block0 = chacha_block(key, nonce, 0);
    let mut pkey = [0u8; 32];
    pkey.copy_from_slice(&block0[..32]);
    let tag = poly_mac(&pkey, adata, &ct[..pt.len()]);
    ct[pt.len()..pt.len() + 16].copy_from_slice(&tag);
}

/// 解密 + 验证 tag。成功返回 true,失败返回 false(不覆盖输出)。
pub fn chacha20_poly1305_decrypt(
    key: &[u8; 32],
    nonce: &[u8; 12],
    adata: &[u8],
    ct: &[u8],
    out: &mut [u8],
) -> bool {
    if ct.len() < 16 {
        return false;
    }
    let block0 = chacha_block(key, nonce, 0);
    let mut pkey = [0u8; 32];
    pkey.copy_from_slice(&block0[..32]);
    let tag = poly_mac(&pkey, adata, &ct[..ct.len() - 16]);
    if tag != ct[ct.len() - 16..] {
        return false;
    }
    out[..ct.len() - 16].copy_from_slice(&ct[..ct.len() - 16]);
    chacha20_xor(key, nonce, 1, &mut out[..ct.len() - 16]);
    true
}
// ============================== X25519 ================================
// 字段元素:5×51-bit limbs(u64,小端位序),2^255-19。
type Fe = [u64; 5];

#[inline]
fn limb_from(b: &[u8; 32], start_bit: usize) -> u64 {
    let byte = start_bit / 8;
    let shift = start_bit % 8;
    let mut x = [0u8; 8];
    let n = (32 - byte).min(8);
    x[..n].copy_from_slice(&b[byte..byte + n]);
    let v = u64::from_le_bytes(x);
    (v >> shift) & ((1u64 << 51) - 1)
}

pub fn fe_frombytes_pub(b: &[u8; 32]) -> Fe { fe_frombytes(b) }
pub fn fe_tobytes_pub(f: &Fe) -> [u8; 32] { fe_tobytes(f) }
pub fn fe_mul_pub(f: &Fe, g: &Fe) -> Fe { fe_mul(f, g) }
pub fn fe_add_pub(f: &Fe, g: &Fe) -> Fe { fe_add(f, g) }
pub fn fe_sub_pub(f: &Fe, g: &Fe) -> Fe { fe_sub(f, g) }
pub fn fe_cswap_pub(f: &mut Fe, g: &mut Fe, b: u64) { fe_cswap(f, g, b) }

#[inline]
fn fe_frombytes(b: &[u8; 32]) -> Fe {
    [
        limb_from(b, 0),
        limb_from(b, 51),
        limb_from(b, 102),
        limb_from(b, 153),
        limb_from(b, 204),
    ]
}

#[inline]
fn fe_tobytes(f: &Fe) -> [u8; 32] {
    let mut out = [0u8; 32];
    let m = (1u64 << 51) - 1;
    // 进位规约(输入 limb 可略超 51 位)
    let mut l = *f;
    for i in 0..4 {
        l[i + 1] += l[i] >> 51;
        l[i] &= m;
    }
    let c = l[4] >> 51;
    l[4] &= m;
    l[0] += c * 19;
    l[1] += l[0] >> 51;
    l[0] &= m;
    for i in 1..4 {
        let c = l[i] >> 51;
        l[i] &= m;
        l[i + 1] += c;
    }
    let c = l[4] >> 51;
    l[4] &= m;
    l[0] += c * 19;
    // 条件减 p:h+19 溢出 → h >= p → 用 q = h+19-2^255 = h-p
    let t0 = l[0] + 19;
    let c0 = t0 >> 51;
    let q0 = t0 & m;
    let t1 = l[1] + c0;
    let c1 = t1 >> 51;
    let q1 = t1 & m;
    let t2 = l[2] + c1;
    let c2 = t2 >> 51;
    let q2 = t2 & m;
    let t3 = l[3] + c2;
    let c3 = t3 >> 51;
    let q3 = t3 & m;
    let t4 = l[4] + c3;
    let c4 = t4 >> 51;
    let q4 = t4 & m;
    let mask = !c4.wrapping_sub(1);
    l[0] = (q0 & mask) | (l[0] & !mask);
    l[1] = (q1 & mask) | (l[1] & !mask);
    l[2] = (q2 & mask) | (l[2] & !mask);
    l[3] = (q3 & mask) | (l[3] & !mask);
    l[4] = (q4 & mask) | (l[4] & !mask);
    for i in 0..5 {
        let bit = i * 51;
        let byte = bit / 8;
        let shift = bit % 8;
        let v = l[i] << shift;
        for j in 0..8 {
            let idx = byte + j;
            if idx < 32 {
                out[idx] |= (v >> (8 * j)) as u8;
            }
        }
    }
    out
}

fn fe_mul(f: &Fe, g: &Fe) -> Fe {
    let f0 = f[0];
    let f1 = f[1];
    let f2 = f[2];
    let f3 = f[3];
    let f4 = f[4];
    let g0 = g[0];
    let g1 = g[1];
    let g2 = g[2];
    let g3 = g[3];
    let g4 = g[4];
    let m = (1u64 << 51) - 1;
    // 完全展开的 schoolbook(乘积项 51+51=102 位,u128 承载)
    let p00 = f0 as u128 * g0 as u128;
    let p01 = f0 as u128 * g1 as u128;
    let p02 = f0 as u128 * g2 as u128;
    let p03 = f0 as u128 * g3 as u128;
    let p04 = f0 as u128 * g4 as u128;
    let p11 = f1 as u128 * g1 as u128;
    let p12 = f1 as u128 * g2 as u128;
    let p13 = f1 as u128 * g3 as u128;
    let p14 = f1 as u128 * g4 as u128;
    let p22 = f2 as u128 * g2 as u128;
    let p23 = f2 as u128 * g3 as u128;
    let p24 = f2 as u128 * g4 as u128;
    let p33 = f3 as u128 * g3 as u128;
    let p34 = f3 as u128 * g4 as u128;
    let p44 = f4 as u128 * g4 as u128;
    let p10 = f1 as u128 * g0 as u128;
    let p20 = f2 as u128 * g0 as u128;
    let p21 = f2 as u128 * g1 as u128;
    let p30 = f3 as u128 * g0 as u128;
    let p31 = f3 as u128 * g1 as u128;
    let p32 = f3 as u128 * g2 as u128;
    let p40 = f4 as u128 * g0 as u128;
    let p41 = f4 as u128 * g1 as u128;
    let p42 = f4 as u128 * g2 as u128;
    let p43 = f4 as u128 * g3 as u128;
    let d0 = p00;
    let d1 = p01 + p10;
    let d2 = p02 + p11 + p20;
    let d3 = p03 + p12 + p21 + p30;
    let d4 = p04 + p13 + p22 + p31 + p40;
    let d5 = p14 + p23 + p32 + p41;
    let d6 = p24 + p33 + p42;
    let d7 = p34 + p43;
    let d8 = p44;
    // 折叠 2^255 ≡ 19:2^(51(5+i)) ≡ 19·2^(51i) → d[5+i]·19 加到 d[i]
    let d0 = d0 + d5 * 19;
    let d1 = d1 + d6 * 19;
    let d2 = d2 + d7 * 19;
    let d3 = d3 + d8 * 19;
    let m = (1u64 << 51) - 1;
    let mm = m as u128;
    // 进位链(保留 5 个 limb)
    let c0 = d0 >> 51;
    let mut r0 = (d0 & mm) as u64;
    let d1 = d1 + c0;
    let c1 = d1 >> 51;
    let mut r1 = (d1 & mm) as u64;
    let d2 = d2 + c1;
    let c2 = d2 >> 51;
    let mut r2 = (d2 & mm) as u64;
    let d3 = d3 + c2;
    let c3 = d3 >> 51;
    let mut r3 = (d3 & mm) as u64;
    let d4 = d4 + c3;
    let c4 = d4 >> 51;
    let mut r4 = (d4 & mm) as u64;
    // c4 = 位 255..306 溢出 → ×19 折叠回 r0
    let v0 = r0 as u128 + c4 * 19;
    r0 = (v0 & mm) as u64;
    let c = v0 >> 51;
    r1 += c as u64;
    let c = r1 >> 51;
    r1 &= m;
    r2 += c;
    let c = r2 >> 51;
    r2 &= m;
    r3 += c;
    let c = r3 >> 51;
    r3 &= m;
    r4 += c;
    let c = r4 >> 51;
    r4 &= m;
    if c > 0 {
        let v0 = r0 as u128 + (c as u128) * 19;
        r0 = (v0 & mm) as u64;
        r1 += (v0 >> 51) as u64;
        let c1 = r1 >> 51;
        r1 &= m;
        r2 += c1;
        let c2 = r2 >> 51;
        r2 &= m;
        r3 += c2;
        let c3 = r3 >> 51;
        r3 &= m;
        r4 += c3;
    }
    [r0, r1, r2, r3, r4]
}

#[inline]
fn fe_sq(f: &Fe) -> Fe {
    fe_mul(f, f)
}

#[inline]
fn fe_add(f: &Fe, g: &Fe) -> Fe {
    [
        f[0] + g[0],
        f[1] + g[1],
        f[2] + g[2],
        f[3] + g[3],
        f[4] + g[4],
    ]
}

#[inline]
fn fe_sub(f: &Fe, g: &Fe) -> Fe {
    // 偏置 = 2p = [2^52-38, 2^52-2, 2^52-2, 2^52-2, 2^52-2](模 p 为 0)
    [
        f[0] + (1 << 52) - 38 - g[0],
        f[1] + (1 << 52) - 2 - g[1],
        f[2] + (1 << 52) - 2 - g[2],
        f[3] + (1 << 52) - 2 - g[3],
        f[4] + (1 << 52) - 2 - g[4],
    ]
}

#[inline]
fn fe_cswap(f: &mut Fe, g: &mut Fe, b: u64) {
    let mask = 0u64.wrapping_sub(b & 1);
    for i in 0..5 {
        let t = mask & (f[i] ^ g[i]);
        f[i] ^= t;
        g[i] ^= t;
    }
}

fn fe_invert(z: &Fe) -> Fe {
    // z^(2^255-21) 二进制平方-乘:e = 1<<255 - 21
    // 二进制:位 254=1,位 253..5 全 1,位 4..0 = 01011
    let mut r = *z;
    for _ in 0..249 {
        r = fe_sq(&r);
        r = fe_mul(&r, z);
    }
    r = fe_sq(&r);
    r = fe_sq(&r);
    r = fe_mul(&r, z);
    r = fe_sq(&r);
    r = fe_sq(&r);
    r = fe_mul(&r, z);
    r = fe_sq(&r);
    r = fe_mul(&r, z);
    r
}

/// X25519 标量乘:scalar(32B LE,已夹位)与基点 u=9 或给定 u。
pub fn x25519(scalar: &[u8; 32], u: &[u8; 32]) -> [u8; 32] {
    let mut e = *scalar;
    e[0] &= 248;
    e[31] &= 127;
    e[31] |= 64;
    x25519_raw(&e, u)
}

/// 不夹位版本(调试用)
pub fn x25519_raw_debug(e: &[u8; 32], u: &[u8; 32]) -> ([u8; 32], [u8; 32], [u8; 32], [u8; 32]) {
    let mut e2 = *e;
    let mut x1 = fe_frombytes(u);
    let mut x2 = [1u64, 0, 0, 0, 0];
    let mut z2 = [0u64, 0, 0, 0, 0];
    let mut x3 = x1;
    let mut z3 = [1u64, 0, 0, 0, 0];
    let mut swap = 0u64;
    for t in (0..255).rev() {
        let k = (e2[t / 8] >> (t % 8)) & 1;
        swap ^= k as u64;
        fe_cswap(&mut x2, &mut x3, swap);
        fe_cswap(&mut z2, &mut z3, swap);
        swap = k as u64;
        let a = fe_add(&x2, &z2);
        let b = fe_sub(&x2, &z2);
        let c = fe_add(&x3, &z3);
        let d = fe_sub(&x3, &z3);
        let da = fe_mul(&d, &a);
        let cb = fe_mul(&c, &b);
        let nx3 = fe_add(&da, &cb);
        let nz3 = fe_sub(&da, &cb);
        let nx3 = fe_sq(&nx3);
        let nz3 = fe_mul(&nz3, &nz3);
        let nz3 = fe_mul(&nz3, &x1);
        let aa = fe_sq(&a);
        let bb = fe_sq(&b);
        let e0 = fe_sub(&aa, &bb);
        let e1 = fe_mul(&e0, &[121665, 0, 0, 0, 0]);
        let e2 = fe_add(&aa, &e1);
        x3 = nx3;
        z3 = nz3;
        x2 = fe_mul(&aa, &bb);
        z2 = fe_mul(&e0, &e2);
    }
    fe_cswap(&mut x2, &mut x3, swap);
    fe_cswap(&mut z2, &mut z3, swap);
    let inv = fe_invert(&z2);
    let out = fe_mul(&x2, &inv);
    (fe_tobytes(&x2), fe_tobytes(&z2), fe_tobytes(&inv), fe_tobytes(&out))
}

pub fn x25519_pub(e: &[u8; 32], u: &[u8; 32]) -> [u8; 32] { x25519(e, u) }

pub fn x25519_raw(e: &[u8; 32], u: &[u8; 32]) -> [u8; 32] {
    let mut x1 = fe_frombytes(u);
    let mut x2 = [1u64, 0, 0, 0, 0];
    let mut z2 = [0u64, 0, 0, 0, 0];
    let mut x3 = x1;
    let mut z3 = [1u64, 0, 0, 0, 0];
    let mut swap = 0u64;
    for t in (0..255).rev() {
        let k = (e[t / 8] >> (t % 8)) & 1;
        swap ^= k as u64;
        fe_cswap(&mut x2, &mut x3, swap);
        fe_cswap(&mut z2, &mut z3, swap);
        swap = k as u64;
        let a = fe_add(&x2, &z2);
        let b = fe_sub(&x2, &z2);
        let c = fe_add(&x3, &z3);
        let d = fe_sub(&x3, &z3);
        let da = fe_mul(&d, &a);
        let cb = fe_mul(&c, &b);
        let nx3 = fe_add(&da, &cb);
        let nz3 = fe_sub(&da, &cb);
        let nx3 = fe_sq(&nx3);
        let nz3 = fe_mul(&nz3, &nz3);
        let nz3 = fe_mul(&nz3, &x1);
        let aa = fe_sq(&a);
        let bb = fe_sq(&b);
        let e0 = fe_sub(&aa, &bb);
        let e1 = fe_mul(&e0, &[121665, 0, 0, 0, 0]);
        let e2 = fe_add(&aa, &e1);
        x3 = nx3;
        z3 = nz3;
        x2 = fe_mul(&aa, &bb);
        z2 = fe_mul(&e0, &e2);
    }
    fe_cswap(&mut x2, &mut x3, swap);
    fe_cswap(&mut z2, &mut z3, swap);
    let inv = fe_invert(&z2);
    let out = fe_mul(&x2, &inv);
    fe_tobytes(&out)
}
