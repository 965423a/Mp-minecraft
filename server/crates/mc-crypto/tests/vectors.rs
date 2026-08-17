//! RFC 已知答案测试向量。
use mc_crypto::*;

fn hex(s: &str) -> Vec<u8> {
    let mut v = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i + 1 < b.len() {
        let hi = (b[i] as char).to_digit(16).unwrap();
        let lo = (b[i + 1] as char).to_digit(16).unwrap();
        v.push((hi * 16 + lo) as u8);
        i += 2;
    }
    v
}

#[test]
fn sha1_abc() {
    assert_eq!(sha1(b"abc"), hex("a9993e364706816aba3e25717850c26c9cd0d89d")[..]);
}

#[test]
fn sha256_abc() {
    assert_eq!(
        sha256(b"abc"),
        hex("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")[..]
    );
}

#[test]
fn sha512_abc() {
    assert_eq!(
        sha512(b"abc"),
        hex("ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f")[..]
    );
}

#[test]
fn hmac_sha256_vector() {
    let mac = hmac_sha256(b"key", b"The quick brown fox jumps over the lazy dog");
    assert_eq!(
        mac,
        hex("f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8")[..]
    );
}

#[test]
fn chacha20_rfc8439_a1() {
    let key: [u8; 32] = hex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f")
        .try_into()
        .unwrap();
    let nonce: [u8; 12] = hex("000000090000004a00000000").try_into().unwrap();
    let mut data = [0u8; 64];
    chacha20_xor(&key, &nonce, 1, &mut data);
    assert_eq!(
        data,
        hex("10f1e7e4d13b5915500fdd1fa32071c4c7d1f4c733c068030422aa9ac3d46c4ed2826446079faa0914c2d705d98b02a2b5129cd1de164eb9cbd083e8a2503c4e")[..]
    );
}

#[test]
fn chacha20_rfc8439_a2() {
    let key: [u8; 32] = hex("0000000000000000000000000000000000000000000000000000000000000000")
        .try_into()
        .unwrap();
    let nonce: [u8; 12] = hex("000000000000000000000000").try_into().unwrap();
    let mut data = [0u8; 64];
    chacha20_xor(&key, &nonce, 0, &mut data);
    assert_eq!(
        data,
        hex("76b8e0ada0f13d90405d6ae55386bd28bdd219b8a08ded1aa836efcc8b770dc7da41597c5157488d7724e03fb8d84a376a43b8f41518a11cc387b669b2ee6586")[..]
    );
}

#[test]
fn poly1305_rfc8439_a3() {
    let key: [u8; 32] =
        hex("85d6be7857556d337f4452fe42d506a80103808afb0db2fd4abff6af4149f51b")
            .try_into()
            .unwrap();
    let tag = poly1305(&key, b"Cryptographic Forum Research Group");
    assert_eq!(tag, hex("a8061dc1305136c6c22b8baf0c0127a9")[..]);
}

#[test]
fn chacha20_poly1305_rfc8439_a5() {
    let key: [u8; 32] = hex("808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f")
        .try_into()
        .unwrap();
    let nonce: [u8; 12] = hex("070000004041424344454647").try_into().unwrap();
    let pt = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
    let adata = hex("50515253c0c1c2c3c4c5c6c7");
    let mut ct = vec![0u8; pt.len() + 16];
    chacha20_poly1305_encrypt(&key, &nonce, &adata, pt, &mut ct);
    assert_eq!(
        ct,
        hex("d31a8d34648e60db7b86afbc53ef7ec2a4aded51296e08fea9e2b5a736ee62d63dbea45e8ca9671282fafb69da92728b1a71de0a9e060b2905d6a5b67ecd3b3692ddbd7f2d778b8c9803aee328091b58fab324e4fad675945585808b4831d7bc3ff4def08e4b7a9de576d26586cec64b61161ae10b594f09e26a7e902ecbd0600691")[..]
    );
}

#[test]
fn x25519_rfc7748_1() {
    let scalar: [u8; 32] =
        hex("a546e36bf0527c9d3b16154b82465edd62144c0ac1fc5a18506a2244ba449ac4")
            .try_into()
            .unwrap();
    let u: [u8; 32] = hex("e6db6867583030db3594c1a424b15f7c726624ec26b3353b10a903a6d0ab1c4c")
        .try_into()
        .unwrap();
    let out = x25519(&scalar, &u);
    assert_eq!(
        out,
        hex("c3da55379de9c6908e94ea4df28d084f32eccf03491c71f754b4075577a28552")[..]
    );
}

#[test]
fn x25519_rfc7748_2() {
    let scalar: [u8; 32] =
        hex("4b66e9d4d1b4673c5ad22691957d6af5c11b6421e0ea01d42ca4169e7918ba0d")
            .try_into()
            .unwrap();
    let u: [u8; 32] = hex("e5210f12786811d3f4b7959d0538ae2c31dbe7106fc03c3efc4cd549c715a493")
        .try_into()
        .unwrap();
    let out = x25519(&scalar, &u);
    assert_eq!(
        out,
        hex("95cbde9476e8907d7aade45cb4b873f88b595a68799fa152e6f8f7647aac7957")[..]
    );
}