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

    let succ_frame = read_frame(&mut stream);
    let mut succ = ReadBuf::new(&succ_frame);
    assert_eq!(succ.read_varint().unwrap(), 0x02);
    let uuid = succ.read_uuid().unwrap();
    let name = succ.read_string().unwrap();
    assert_eq!(name, "Tester");
    assert!(uuid.iter().any(|&b| b != 0), "offline uuid must be non-zero");

    let mut ack = WriteBuf::new();
    ack.write_varint(0x03);
    write_frame(&mut stream, &ack.into_bytes());

    let mut ci = WriteBuf::new();
    ci.write_varint(0x00);
    ci.write_string("en_us");
    ci.write_u8(0x7f);
    ci.write_varint(0);
    ci.write_bool(true);
    ci.write_varint(0);
    ci.write_varint(0);
    write_frame(&mut stream, &ci.into_bytes());

    let mut packs = WriteBuf::new();
    packs.write_varint(0x07);
    write_frame(&mut stream, &packs.into_bytes());

    for i in 0..5 {
        let body = read_frame(&mut stream);
        let mut r = ReadBuf::new(&body);
        let id = r.read_varint().unwrap();
        assert!(
            [0x07, 0x0C, 0x0D, 0x0E, 0x03].contains(&id),
            "unexpected configuration packet id 0x{id:02x}"
        );
        if id == 0x0E {
            assert_eq!(r.read_varint().unwrap(), 0, "known packs should be empty");
        }
        if i == 4 {
            assert_eq!(id, 0x03, "last packet must be finish configuration");
        }
    }
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