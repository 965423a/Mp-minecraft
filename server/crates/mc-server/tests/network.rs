//! 网络集成测试:真实 TCP 起服,模拟客户端完成握手 + Status 全流程。

use mc_protocol::buf::{ReadBuf, WriteBuf};
use mc_protocol::consts::PROTOCOL_VERSION;
use std::io::{Read, Write};
use std::net::TcpStream;

fn write_frame(stream: &mut TcpStream, packet: &[u8]) {
    let mut frame = Vec::with_capacity(packet.len() + 5);
    mc_protocol::varint::write_varint(packet.len() as u32, &mut frame);
    frame.extend_from_slice(packet);
    stream.write_all(&frame).unwrap();
}

fn read_frame(stream: &mut TcpStream) -> Vec<u8> {
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let mut len_bytes = [0u8; 5];
    let mut pos = 0;
    loop {
        stream.read_exact(&mut len_bytes[pos..pos + 1]).unwrap();
        pos += 1;
        let mut used = 0;
        let len = match mc_protocol::varint::decode_varint_i32(&len_bytes[..pos], &mut used) {
            Some(v) => v,
            None => {
                if pos >= 5 {
                    panic!("bad length varint");
                }
                continue;
            }
        };
        let mut body = vec![0u8; len as usize];
        stream.read_exact(&mut body).unwrap();
        return body;
    }
}

fn start_server() -> (u16, std::process::Child) {
    static NEXT: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(28000);
    let port = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let home = std::env::temp_dir().join(format!("mcs-net-test-{port}"));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join("server.properties"),
        "view-distance=2\nlevel-name=world\n",
    )
    .unwrap();
    let child = std::process::Command::new(env!("CARGO_BIN_EXE_mc-server"))
        .env("MCS_HOME", &home)
        .env("MCS_THREADS", "2")
        .env("MCS_PORT", port.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    wait_port(port, std::time::Duration::from_secs(20));
    (port, child)
}

fn wait_port(port: u16, timeout: std::time::Duration) -> TcpStream {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(s) => return s,
            Err(_) => {
                if std::time::Instant::now() > deadline {
                    panic!("server did not start on port {port}");
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
}

#[test]
fn status_ping_roundtrip() {
    let (port, mut child) = start_server();
    let mut stream = wait_port(port, std::time::Duration::from_secs(20));

    let mut hs = WriteBuf::new();
    hs.write_varint(0x00);
    hs.write_varint(PROTOCOL_VERSION);
    hs.write_string("127.0.0.1");
    hs.write_u16(port);
    hs.write_varint(1);
    write_frame(&mut stream, &hs.into_bytes());

    let mut req = WriteBuf::new();
    req.write_varint(0x00);
    write_frame(&mut stream, &req.into_bytes());

    let resp = read_frame(&mut stream);
    let mut r = ReadBuf::new(&resp);
    assert_eq!(r.read_varint().unwrap(), 0x00);
    let json = r.read_string().unwrap();
    assert!(json.contains("\"protocol\":") && json.contains("A Mp-minecraft Server"), "bad status json: {json}");
    assert!(json.contains(PROTOCOL_VERSION.to_string().as_str()));

    let now = 1234567890i64;
    let mut ping = WriteBuf::new();
    ping.write_varint(0x01);
    ping.write_i64(now);
    write_frame(&mut stream, &ping.into_bytes());

    let pong = read_frame(&mut stream);
    let mut r = ReadBuf::new(&pong);
    assert_eq!(r.read_varint().unwrap(), 0x01);
    assert_eq!(r.read_i64().unwrap(), now);
    let _ = child.kill();
}

#[test]
fn login_to_configuration() {
    let (port, mut child) = start_server();
    let mut stream = wait_port(port, std::time::Duration::from_secs(20));
    let mut hs = WriteBuf::new();
    hs.write_varint(0x00);
    hs.write_varint(PROTOCOL_VERSION);
    hs.write_string("127.0.0.1");
    hs.write_u16(port);
    hs.write_varint(2);
    write_frame(&mut stream, &hs.into_bytes());

    let mut start = WriteBuf::new();
    start.write_varint(0x00);
    start.write_string("Tester");
    start.write_uuid([0u8; 16]);
    write_frame(&mut stream, &start.into_bytes());

    let comp_frame = read_frame(&mut stream);
    let mut set_comp = ReadBuf::new(&comp_frame);
    assert_eq!(set_comp.read_varint().unwrap(), 0x03);
    assert_eq!(set_comp.read_varint().unwrap(), 256);

    // 压缩已启用,后续收发走压缩格式
    let succ_frame = read_frame_compressed(&mut stream);
    let mut succ = ReadBuf::new(&succ_frame);
    assert_eq!(succ.read_varint().unwrap(), 0x02);
    let uuid = succ.read_uuid().unwrap();
    let name = succ.read_string().unwrap();
    assert_eq!(name, "Tester");
    assert!(uuid.iter().any(|&b| b != 0), "offline uuid must be non-zero");

    let mut ack = WriteBuf::new();
    ack.write_varint(0x03);
    write_frame_compressed(&mut stream, &ack.into_bytes());

    let mut ci = WriteBuf::new();
    ci.write_varint(0x00);
    ci.write_string("en_us");
    ci.write_u8(0x7f);
    ci.write_varint(0);
    ci.write_bool(true);
    ci.write_varint(0);
    ci.write_varint(0);
    write_frame_compressed(&mut stream, &ci.into_bytes());

    let mut packs = WriteBuf::new();
    packs.write_varint(0x07);
    write_frame_compressed(&mut stream, &packs.into_bytes());

    for _ in 0..30 {
        let body = read_frame_compressed(&mut stream);
        let mut r = ReadBuf::new(&body);
        let id = r.read_varint().unwrap();
        assert!(
            [0x07, 0x0C, 0x0D, 0x0E, 0x03].contains(&id),
            "unexpected configuration packet id 0x{id:02x}"
        );
        if id == 0x0E {
            assert_eq!(r.read_varint().unwrap(), 0, "known packs should be empty");
        }
        if id == 0x03 {
            let _ = child.kill();
            return;
        }
    }
    panic!("finish configuration never received");
}

/// 压缩模式下发帧:长度前缀 + [未压缩长度 varint + 数据]。
fn write_frame_compressed(stream: &mut TcpStream, packet: &[u8]) {
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    enc.write_all(packet).unwrap();
    let z = enc.finish().unwrap();
    let mut body = Vec::with_capacity(z.len() + 5);
    mc_protocol::varint::write_varint(packet.len() as u32, &mut body);
    body.extend_from_slice(&z);
    write_frame(stream, &body);
}

/// 压缩模式下收帧:长度前缀 + [未压缩长度 varint + zlib 数据]。
fn read_frame_compressed(stream: &mut TcpStream) -> Vec<u8> {
    use std::io::Read;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let mut len_bytes = [0u8; 5];
    let mut pos = 0;
    let len = loop {
        stream.read_exact(&mut len_bytes[pos..pos + 1]).unwrap();
        pos += 1;
        let mut used = 0;
        if let Some(v) = mc_protocol::varint::decode_varint_i32(&len_bytes[..pos], &mut used) {
            break v;
        }
        if pos >= 5 {
            panic!("bad length varint");
        }
    };
    let mut body = vec![0u8; len as usize];
    stream.read_exact(&mut body).unwrap();
    let mut rp = 0usize;
    let uncompressed_len = mc_protocol::varint::decode_varint_i32(&body, &mut rp).unwrap();
    if uncompressed_len == 0 {
        return body[rp..].to_vec();
    }
    let mut dec = flate2::read::ZlibDecoder::new(&body[rp..]);
    let mut out = Vec::new();
    dec.read_to_end(&mut out).unwrap();
    assert_eq!(out.len() as i32, uncompressed_len);
    out
}

#[test]
fn play_full_flow() {
    let (port, mut child) = start_server();
    let mut stream = wait_port(port, std::time::Duration::from_secs(20));
    let mut hs = WriteBuf::new();
    hs.write_varint(0x00);
    hs.write_varint(PROTOCOL_VERSION);
    hs.write_string("127.0.0.1");
    hs.write_u16(port);
    hs.write_varint(2);
    write_frame(&mut stream, &hs.into_bytes());

    let mut start = WriteBuf::new();
    start.write_varint(0x00);
    start.write_string("Player1");
    start.write_uuid([0u8; 16]);
    write_frame(&mut stream, &start.into_bytes());

    // Set Compression 在压缩开启前明文发送
    let comp_frame = read_frame(&mut stream);
    let mut set_comp = ReadBuf::new(&comp_frame);
    assert_eq!(set_comp.read_varint().unwrap(), 0x03);
    assert_eq!(set_comp.read_varint().unwrap(), 256);

    // 此后服务器启用压缩,Login Success 走压缩格式
    let succ_frame = read_frame_compressed(&mut stream);
    let mut succ = ReadBuf::new(&succ_frame);
    assert_eq!(succ.read_varint().unwrap(), 0x02);

    // Login Acknowledged(此后所有发送走压缩格式)
    let mut ack = WriteBuf::new();
    ack.write_varint(0x03);
    write_frame_compressed(&mut stream, &ack.into_bytes());

    // Client Information
    let mut ci = WriteBuf::new();
    ci.write_varint(0x00);
    ci.write_string("en_us");
    ci.write_u8(0x7f);
    ci.write_varint(0);
    ci.write_bool(true);
    ci.write_varint(0);
    ci.write_varint(0);
    write_frame_compressed(&mut stream, &ci.into_bytes());

    // Known Packs
    let mut packs = WriteBuf::new();
    packs.write_varint(0x07);
    write_frame_compressed(&mut stream, &packs.into_bytes());

    let mut finished = false;
    for _ in 0..30 {
        let body = read_frame_compressed(&mut stream);
        let mut r = ReadBuf::new(&body);
        let id = r.read_varint().unwrap();
        assert!(
            [0x07, 0x0C, 0x0D, 0x0E, 0x03].contains(&id),
            "unexpected configuration packet id 0x{id:02x}"
        );
        if id == 0x03 {
            finished = true;
            break;
        }
    }
    assert!(finished, "finish configuration never received");

    // Acknowledge Finish Configuration → 进入 Play
    let mut fin = WriteBuf::new();
    fin.write_varint(0x03);
    write_frame_compressed(&mut stream, &fin.into_bytes());

    // 服务器应发送:Join Game + Player Info + Sync Position + Keep Alive 节拍
    let join = read_frame_compressed(&mut stream);
    let mut r = ReadBuf::new(&join);
    assert_eq!(r.read_varint().unwrap(), 0x31);
    let entity_id = r.read_i32().unwrap();
    assert_eq!(entity_id, 1);
    let hardcore = r.read_bool().unwrap();
    assert!(!hardcore);
    let dims_len = r.read_varint().unwrap();
    assert_eq!(dims_len, 3);
    for _ in 0..3 {
        r.read_string().unwrap();
    }
    r.read_varint().unwrap(); // max players
    r.read_varint().unwrap(); // view distance
    r.read_varint().unwrap(); // sim distance
    r.read_bool().unwrap(); // reduced debug info
    r.read_bool().unwrap(); // enable respawn screen
    r.read_bool().unwrap(); // limited crafting
    r.read_varint().unwrap(); // dimension type
    assert_eq!(r.read_string().unwrap(), "minecraft:overworld");

    // Player Info Update
    let pi = read_frame_compressed(&mut stream);
    let mut r = ReadBuf::new(&pi);
    assert_eq!(r.read_varint().unwrap(), 0x46);
    assert_eq!(r.read_u8().unwrap(), 0x01);
    assert_eq!(r.read_varint().unwrap(), 1);

    // 区块序列:Set Center Chunk(0x5E)+ Spawn Position(0x61)+ Batch Start(0x0C)
    // + 9 × Chunk Data(0x2D)+ Batch Finished(0x0B),然后才到 Sync Position(0x48)
    let mut chunk_count = 0;
    let mut pos_pkt: Option<Vec<u8>> = None;
    for _ in 0..20 {
        let body = read_frame_compressed(&mut stream);
        let mut r = ReadBuf::new(&body);
        let id = r.read_varint().unwrap();
        match id {
            0x2C => continue,
            0x5E => {
                let cx = r.read_i32().unwrap();
                let cz = r.read_i32().unwrap();
                assert_eq!((cx, cz), (0, 0));
            }
            0x61 => {
                assert!(r.finished() || r.remaining() >= 12, "spawn pos fields");
            }
            0x0C => {}
            0x2D => {
                chunk_count += 1;
                let cx = r.read_i32().unwrap();
                let cz = r.read_i32().unwrap();
                assert!((-1..=1).contains(&cx) && (-1..=1).contains(&cz));
            }
            0x0B => {
                assert_eq!(r.read_varint().unwrap(), 9, "batch size");
            }
            0x48 => {
                pos_pkt = Some(body);
                break;
            }
            other => panic!("unexpected play packet id 0x{other:02x}"),
        }
    }
    assert_eq!(chunk_count, 9, "expected 9 chunk data packets");
    let pos = pos_pkt.expect("sync player position packet");
    let mut r = ReadBuf::new(&pos);
    assert_eq!(r.read_varint().unwrap(), 0x48);
    let teleport_id = r.read_varint().unwrap();
    assert_eq!(r.read_f64().unwrap(), 0.5);
    assert_eq!(r.read_f64().unwrap(), 65.0);

    // 回 Confirm Teleportation
    let mut ct = WriteBuf::new();
    ct.write_varint(0x00);
    ct.write_varint(teleport_id);
    write_frame_compressed(&mut stream, &ct.into_bytes());

    // Keep Alive 节拍(压缩格式收)
    let ka = read_frame_compressed(&mut stream);
    let mut r = ReadBuf::new(&ka);
    assert_eq!(r.read_varint().unwrap(), 0x2C);
    let ka_id = r.read_i64().unwrap();
    assert!(ka_id > 0);

    // 回 Keep Alive 应答
    let mut ka_ack = WriteBuf::new();
    ka_ack.write_varint(0x1C);
    ka_ack.write_i64(ka_id);
    write_frame_compressed(&mut stream, &ka_ack.into_bytes());

    // 再等一个 Keep Alive,证明循环继续
    let ka2 = read_frame_compressed(&mut stream);
    let mut r = ReadBuf::new(&ka2);
    assert_eq!(r.read_varint().unwrap(), 0x2C);
    assert_eq!(r.read_i64().unwrap(), ka_id + 1);
    let _ = child.kill();
}

#[test]
fn login_wrong_protocol_disconnected() {
    let (port, mut child) = start_server();
    let mut stream = wait_port(port, std::time::Duration::from_secs(20));
    let mut hs = WriteBuf::new();
    hs.write_varint(0x00);
    hs.write_varint(9999);
    hs.write_string("127.0.0.1");
    hs.write_u16(port);
    hs.write_varint(2);
    write_frame(&mut stream, &hs.into_bytes());

    let mut start = WriteBuf::new();
    start.write_varint(0x00);
    start.write_string("Tester");
    start.write_uuid([0u8; 16]);
    write_frame(&mut stream, &start.into_bytes());

    let disc_frame = read_frame(&mut stream);
    let mut disc = ReadBuf::new(&disc_frame);
    assert_eq!(disc.read_varint().unwrap(), 0x00);
    assert!(disc.read_string().unwrap().contains("Unsupported protocol"));
    let _ = child.kill();
}