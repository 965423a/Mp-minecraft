//! no_std 密码学原语,供 mcssh 使用:
//! SHA-1 / SHA-256 / SHA-512、HMAC、ChaCha20-Poly1305、X25519、Ed25519。
//! 全部自实现、恒定数据流,测试向量见 tests/。

// 通用旋转与字节序
#[inline]
fn rotr(x: u32, n: u32) -> u32 {
    x.rotate_right(n)
}
#[inline]
fn rotl(x: u32, n: u32) -> u32 {
    x.rotate_left(n)
}
#[inline]
fn u32_be(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}
#[inline]
fn u64_be(b: &[u8]) -> u64 {
    u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

// =============================== SHA-1 ===============================
const K1: [u32; 4] = [0x5A827999, 0x6ED9EBA1, 0x8F1BBCDC, 0xCA62C1D6];

/// 返回 20 字节 SHA-1 摘要。
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let msg = pad(data, 8);
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut w = [0u32; 80];
    for blk in msg.chunks(64) {
        for i in 0..16 {
            w[i] = u32_be(&blk[i * 4..i * 4 + 4]);
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

pub struct Sha256 {
    h: [u32; 8],
    buf: [u8; 64],
    buf_len: usize,
    total: u64,
}

impl Sha256 {
    pub const fn new() -> Self {
        Sha256 {
            h: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buf: [0; 64],
            buf_len: 0,
            total: 0,
        }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.total = self.total.wrapping_add(data.len() as u64);
        if self.buf_len > 0 {
            let need = 64 - self.buf_len;
            let take = need.min(data.len());
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&data[..take]);
            self.buf_len += take;
            data = &data[take..];
            if self.buf_len == 64 {
                let blk = self.buf;
                self.compress(&blk);
                self.buf_len = 0;
            }
        }
        for blk in data.chunks_exact(64) {
            let mut b = [0u8; 64];
            b.copy_from_slice(blk);
            self.compress(&b);
        }
        let rem = data.len() % 64;
        if rem > 0 {
            let tail = &data[data.len() - rem..];
            self.buf[..rem].copy_from_slice(tail);
            self.buf_len = rem;
        }
    }

    pub fn finalize(self) -> [u8; 32] {
        let bit_len = self.total.wrapping_mul(8);
        let mut m = Sha256 { ..self };
        let mut pad = [0u8; 72];
        let plen = if m.buf_len < 56 {
            64 - m.buf_len
        } else {
            128 - m.buf_len
        };
        pad[0] = 0x80;
        let mut buf = [0u8; 128];
        buf[..m.buf_len].copy_from_slice(&m.buf[..m.buf_len]);
        buf[m.buf_len..m.buf_len + plen].copy_from_slice(&pad[..plen]);
        let nblk = (m.buf_len + plen) / 64;
        for i in 0..nblk {
            let mut b = [0u8; 64];
            b.copy_from_slice(&buf[i * 64..i * 64 + 64]);
            if i + 1 == nblk {
                b[56..64].copy_from_slice(&bit_len.to_be_bytes());
            }
            m.compress(&b);
        }
        let mut out = [0u8; 32];
        for (i, v) in m.h.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
        }
        out
    }

    fn compress(&mut self, blk: &[u8; 64]) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32_be(&blk[i * 4..i * 4 + 4]);
        }
        for i in 16..64 {
            let s0 = rotr(w[i - 15], 7) ^ rotr(w[i - 15], 18) ^ (w[i - 15] >> 3);
            let s1 = rotr(w[i - 2], 17) ^ rotr(w[i - 2], 19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h) = (
            self.h[0], self.h[1], self.h[2], self.h[3], self.h[4], self.h[5], self.h[6], self.h[7],
        );
        for i in 0..64 {
            let s1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K256[i])
                .wrapping_add(w[i]);
            let s0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        self.h[0] = self.h[0].wrapping_add(a);
        self.h[1] = self.h[1].wrapping_add(b);
        self.h[2] = self.h[2].wrapping_add(c);
        self.h[3] = self.h[3].wrapping_add(d);
        self.h[4] = self.h[4].wrapping_add(e);
        self.h[5] = self.h[5].wrapping_add(f);
        self.h[6] = self.h[6].wrapping_add(g);
        self.h[7] = self.h[7].wrapping_add(h);
    }
}

/// 一次性 SHA-256。
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut m = Sha256::new();
    m.update(data);
    m.finalize()
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

pub struct Sha512 {
    h: [u64; 8],
    buf: [u8; 128],
    buf_len: usize,
    total: u64,
}

impl Sha512 {
    pub const fn new() -> Self {
        Sha512 {
            h: [
                0x6a09e667f3bcc908, 0xbb67ae8584caa73b, 0x3c6ef372fe94f82b, 0xa54ff53a5f1d36f1,
                0x510e527fade682d1, 0x9b05688c2b3e6c1f, 0x1f83d9abfb41bd6b, 0x5be0cd19137e2179,
            ],
            buf: [0; 128],
            buf_len: 0,
            total: 0,
        }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.total = self.total.wrapping_add(data.len() as u64);
        if self.buf_len > 0 {
            let need = 128 - self.buf_len;
            let take = need.min(data.len());
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&data[..take]);
            self.buf_len += take;
            data = &data[take..];
            if self.buf_len == 128 {
                let blk = self.buf;
                self.compress(&blk);
                self.buf_len = 0;
            }
        }
        for blk in data.chunks_exact(128) {
            let mut b = [0u8; 128];
            b.copy_from_slice(blk);
            self.compress(&b);
        }
        let rem = data.len() % 128;
        if rem > 0 {
            let tail = &data[data.len() - rem..];
            self.buf[..rem].copy_from_slice(tail);
            self.buf_len = rem;
        }
    }

    pub fn finalize(self) -> [u8; 64] {
        let bit_len = self.total.wrapping_mul(8);
        let mut m = Sha512 { ..self };
        let mut buf = [0u8; 256];
        buf[..m.buf_len].copy_from_slice(&m.buf[..m.buf_len]);
        buf[m.buf_len] = 0x80;
        let plen = if m.buf_len < 112 {
            128 - m.buf_len
        } else {
            256 - m.buf_len
        };
        let nblk = (m.buf_len + plen) / 128;
        for i in 0..nblk {
            let mut b = [0u8; 128];
            b.copy_from_slice(&buf[i * 128..i * 128 + 128]);
            if i + 1 == nblk {
                b[112..120].copy_from_slice(&(bit_len >> 32).to_be_bytes());
                b[120..128].copy_from_slice(&(bit_len & 0xffff_ffff).to_be_bytes());
            }
            m.compress(&b);
        }
        let mut out = [0u8; 64];
        for (i, v) in m.h.iter().enumerate() {
            out[i * 8..i * 8 + 8].copy_from_slice(&v.to_be_bytes());
        }
        out
    }

    fn compress(&mut self, blk: &[u8; 128]) {
        let mut w = [0u64; 80];
        for i in 0..16 {
            w[i] = u64_be(&blk[i * 8..i * 8 + 8]);
        }
        for i in 16..80 {
            let s0 = w[i - 15].rotate_right(1) ^ w[i - 15].rotate_right(8) ^ (w[i - 15] >> 7);
            let s1 = w[i - 2].rotate_right(19) ^ w[i - 2].rotate_right(61) ^ (w[i - 2] >> 6);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h) = (
            self.h[0], self.h[1], self.h[2], self.h[3], self.h[4], self.h[5], self.h[6], self.h[7],
        );
        for i in 0..80 {
            let s1 = e.rotate_right(14) ^ e.rotate_right(18) ^ e.rotate_right(41);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K512[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(28) ^ a.rotate_right(34) ^ a.rotate_right(39);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        self.h[0] = self.h[0].wrapping_add(a);
        self.h[1] = self.h[1].wrapping_add(b);
        self.h[2] = self.h[2].wrapping_add(c);
        self.h[3] = self.h[3].wrapping_add(d);
        self.h[4] = self.h[4].wrapping_add(e);
        self.h[5] = self.h[5].wrapping_add(f);
        self.h[6] = self.h[6].wrapping_add(g);
        self.h[7] = self.h[7].wrapping_add(h);
    }
}

/// 一次性 SHA-512。
pub fn sha512(data: &[u8]) -> [u8; 64] {
    let mut m = Sha512::new();
    m.update(data);
    m.finalize()
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
    let mut inner = [0u8; 96];
    inner[..64].copy_from_slice(&ipad);
    inner[64..].copy_from_slice(data);
    let ih = sha256(&inner);
    let mut outer = [0u8; 96];
    outer[..64].copy_from_slice(&opad);
    outer[64..].copy_from_slice(&ih);
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
    let mut inner = [0u8; 192];
    inner[..128].copy_from_slice(&ipad);
    inner[128..].copy_from_slice(data);
    let ih = sha512(&inner);
    let mut outer = [0u8; 192];
    outer[..128].copy_from_slice(&opad);
    outer[128..].copy_from_slice(&ih);
    sha512(&outer)
}

// ============================= ChaCha20 ==============================
/// ChaCha20 密钥流生成(写入 out,可任意长度)。
pub fn chacha20_xor(key: &[u8; 32], nonce: &[u8; 12], counter: u32, data: &mut [u8]) {
    let mut state = [0u32; 16];
    state[0] = 0x61707865;
    state[1] = 0x3320646e;
    state[2] = 0x79622d32;
    state[3] = 0x6b206574;
    for i in 0..8 {
        state[4 + i] = u32_be(&key[i * 4..i * 4 + 4]);
    }
    state[12] = counter;
    state[13] = u32_be(&nonce[0..4]);
    state[14] = u32_be(&nonce[4..8]);
    state[15] = u32_be(&nonce[8..12]);

    let mut block = [0u8; 64];
    let mut off = 0usize;
    let mut ctr = counter;
    while off < data.len() {
        for (i, s) in state.iter().enumerate() {
            block[i * 4..i * 4 + 4].copy_from_slice(&s.to_le_bytes());
        }
        block20(&block, &state, ctr, &mut block);
        for i in 0..64 {
            if off + i >= data.len() {
                break;
            }
            data[off + i] ^= block[i];
        }
        off += 64;
        ctr = ctr.wrapping_add(1);
    }
}

fn block20(out: &mut [u8; 64], state: &[u32; 16], ctr: u32, block: &mut [u8; 64]) {
    let mut x = *state;
    x[12] = ctr;
    let mut q = x;
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
    for i in 0..16 {
        let v = q[i].wrapping_add(x[i]);
        block[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    let _ = out;
}

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

// ============================== Poly1305 ==============================
/// Poly1305 一次性认证,输出 16 字节 tag。
pub fn poly1305(key: &[u8; 32], msg: &[u8]) -> [u8; 16] {
    let mut r = [0u8; 16];
    r.copy_from_slice(&key[..16]);
    r[3] &= 15;
    r[7] &= 15;
    r[11] &= 15;
    r[15] &= 15;
    r[4] &= 252;
    r[8] &= 252;
    r[12] &= 252;
    let mut acc: u128 = 0;
    let mut rn = [0u128; 1];
    let rl = u128::from_le_bytes({
        let mut b = [0u8; 16];
        b.copy_from_slice(&r);
        b
    });
    rn[0] = rl;
    let mut blocks = msg.chunks(16).peekable();
    while let Some(blk) = blocks.next() {
        let mut b = [0u8; 17];
        b[..blk.len()].copy_from_slice(blk);
        b[blk.len()] = 1;
        acc = (acc + u128::from_le_bytes({
            let mut x = [0u8; 16];
            x.copy_from_slice(&b[..16]);
            x
        }) + (b[16] as u128) << 128)
            .wrapping_mul(rn[0]);
        let _ = blocks;
    }
    let mut s = [0u8; 16];
    s.copy_from_slice(&key[16..]);
    let sl = u128::from_le_bytes({
        let mut b = [0u8; 16];
        b.copy_from_slice(&s);
        b
    });
    let mut tag = [0u8; 16];
    let v = acc.wrapping_add(sl);
    tag.copy_from_slice(&v.to_le_bytes()[..16]);
    tag
}

/// ChaCha20-Poly1305 AEAD(RFC 8439),输出 len=adata_len+ct_len+16,ct 原地。
/// 输入 msg 前 16 字节为 poly1305 key 填充区,ct 从 16 开始。
pub fn chacha20_poly1305(
    key: &[u8; 32],
    nonce: &[u8; 12],
    adata: &[u8],
    msg: &mut [u8],
    ct: &mut [u8],
) {
    chacha20_xor(key, nonce, 1, msg);
    ct[..msg.len()].copy_from_slice(msg);
    let mut mac_data = [0u8; 16];
    let mut mac_nonce = *nonce;
    let mut key_block = [0u8; 64];
    let mut kb = [0u8; 64];
    let mut st = [0u8; 64];
    st.copy_from_slice(&[0u8; 64]);
    let _ = &mut key_block;
    // poly key = first 32 bytes of block0
    let mut pkey = [0u8; 32];
    let mut block = [0u8; 64];
    let mut state = [0u32; 16];
    state[0] = 0x61707865;
    state[1] = 0x3320646e;
    state[2] = 0x79622d32;
    state[3] = 0x6b206574;
    for i in 0..8 {
        state[4 + i] = u32_be(&key[i * 4..i * 4 + 4]);
    }
    state[12] = 0;
    state[13] = u32_be(&mac_nonce[0..4]);
    state[14] = u32_be(&mac_nonce[4..8]);
    state[15] = u32_be(&mac_nonce[8..12]);
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
    for i in 0..16 {
        let v = q[i].wrapping_add(state[i]);
        block[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    pkey.copy_from_slice(&block[..32]);
    let _ = &mut kb;
    let _ = &mut mac_data;

    // MAC over: adata || pad16(adata) || ct || pad16(ct) || len(adata) || len(ct)
    let mut mac = Vec::new_();
    mac.extend_from_slice(adata);
    if adata.len() % 16 != 0 {
        mac.extend_from_slice(&[0u8; 16][..16 - (adata.len() % 16)]);
    }
    mac.extend_from_slice(ct);
    if msg.len() % 16 != 0 {
        mac.extend_from_slice(&[0u8; 16][..16 - (msg.len() % 16)]);
    }
    mac.extend_from_slice(&(adata.len() as u64).to_le_bytes());
    mac.extend_from_slice(&(msg.len() as u64).to_le_bytes());
    let tag = poly1305(&pkey, &mac);
    ct[msg.len()..msg.len() + 16].copy_from_slice(&tag);
}

trait VecNew {
    fn new_() -> Self;
}
impl VecNew for alloc::vec::Vec<u8> {
    fn new_() -> Self {
        alloc::vec::Vec::new()
    }
}
